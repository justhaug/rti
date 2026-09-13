//! Built-in coding agent: an LLM tool loop with file and shell tools,
//! confined to the task worktree. Cheap models are expected here; the
//! gates, not the model, decide whether a change lands.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use rti_core::config::AgentConfig;
use rti_llm::{ChatOptions, LlmClient, Message, ToolDef};

use crate::gates::run_shell;
use crate::task::{task_prompt, CodingTask};
use crate::workspace::Workspace;

pub const SYSTEM_PROMPT: &str = r#"You are RTI's coding worker, modifying the RTI Rust workspace inside an isolated git worktree.
Work autonomously: read the relevant code first, make focused changes, run `cargo build`/`cargo test` with the bash tool, fix failures, then call `finish` with a summary.
Rules:
- Only touch what the task needs. Keep the public APIs other crates rely on unless the task says otherwise.
- Determinism is sacred: the simulator must remain a pure function of (state, action, params, track). Never add randomness or wall-clock dependence to rti-sim.
- Every change must pass: cargo fmt --check, cargo build, cargo test --workspace, and the sim throughput benchmark must not regress more than the configured tolerance.
- Prefer small, verifiable steps. Add or update tests for behaviour you change.
- Do not run git commands; the harness commits and merges.
- If the task is impossible or ill-specified, call `finish` and explain why.
"#;

fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "read_file".into(),
            description: "Read a UTF-8 file (path relative to the repo root). Optional line range.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer"},"end_line":{"type":"integer"}},"required":["path"]}),
        },
        ToolDef {
            name: "write_file".into(),
            description: "Create or overwrite a file with the given content.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
        },
        ToolDef {
            name: "edit_file".into(),
            description: "Replace exactly one occurrence of `old` with `new` in a file. Fails if `old` is absent or ambiguous.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"old":{"type":"string"},"new":{"type":"string"}},"required":["path","old","new"]}),
        },
        ToolDef {
            name: "list_files".into(),
            description: "List files under a directory (recursive, skips target/ and .git/).".into(),
            parameters: serde_json::json!({"type":"object","properties":{"dir":{"type":"string","default":"."}}}),
        },
        ToolDef {
            name: "search".into(),
            description: "Grep the repo for a pattern (fixed string or regex via grep -E). Returns matching lines with file:line.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"pattern":{"type":"string"},"dir":{"type":"string","default":"crates"}},"required":["pattern"]}),
        },
        ToolDef {
            name: "bash".into(),
            description: "Run a shell command in the worktree (cargo build/test/run, etc). Output is truncated to the tail.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"command":{"type":"string"},"timeout_secs":{"type":"integer","default":300}},"required":["command"]}),
        },
        ToolDef {
            name: "finish".into(),
            description: "Signal that the task is complete (or cannot be done). Provide a concise summary of what changed and how it was verified.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"summary":{"type":"string"},"success":{"type":"boolean"}},"required":["summary","success"]}),
        },
    ]
}

fn safe_path(root: &Path, rel: &str) -> anyhow::Result<PathBuf> {
    let p = Path::new(rel);
    if p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
        anyhow::bail!("path must be relative to the repo and must not contain '..': {rel}");
    }
    Ok(root.join(p))
}

fn exec_tool(ws: &Workspace, cfg: &AgentConfig, name: &str, args: &serde_json::Value) -> String {
    let root = &ws.path;
    let res: anyhow::Result<String> = (|| match name {
        "read_file" => {
            let path = safe_path(root, args["path"].as_str().unwrap_or_default())?;
            let text = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
            let lines: Vec<&str> = text.lines().collect();
            let start = args["start_line"]
                .as_u64()
                .map(|v| v.max(1) as usize)
                .unwrap_or(1);
            let end = args["end_line"]
                .as_u64()
                .map(|v| v as usize)
                .unwrap_or(lines.len())
                .min(lines.len());
            let mut out = String::new();
            for (i, l) in lines
                .iter()
                .enumerate()
                .take(end)
                .skip(start.saturating_sub(1))
            {
                out.push_str(&format!("{:5}| {l}\n", i + 1));
            }
            if out.len() > 60_000 {
                out.truncate(60_000);
                out.push_str("\n...[truncated; request a line range]");
            }
            Ok(out)
        }
        "write_file" => {
            let path = safe_path(root, args["path"].as_str().unwrap_or_default())?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let content = args["content"].as_str().unwrap_or_default();
            std::fs::write(&path, content)?;
            Ok(format!("wrote {} bytes to {}", content.len(), args["path"]))
        }
        "edit_file" => {
            let path = safe_path(root, args["path"].as_str().unwrap_or_default())?;
            let text = std::fs::read_to_string(&path)?;
            let old = args["old"].as_str().unwrap_or_default();
            let new = args["new"].as_str().unwrap_or_default();
            let n = text.matches(old).count();
            if old.is_empty() || n == 0 {
                anyhow::bail!("`old` not found in {}", args["path"]);
            }
            if n > 1 {
                anyhow::bail!("`old` occurs {n} times in {}; make it unique", args["path"]);
            }
            std::fs::write(&path, text.replacen(old, new, 1))?;
            Ok(format!("edited {}", args["path"]))
        }
        "list_files" => {
            let dir = safe_path(root, args["dir"].as_str().unwrap_or("."))?;
            let mut out = vec![];
            for e in walkdir::WalkDir::new(&dir).into_iter().filter_entry(|e| {
                let n = e.file_name().to_string_lossy();
                n != "target" && n != ".git" && n != ".rti" && n != "data"
            }) {
                let e = e?;
                if e.file_type().is_file() {
                    out.push(
                        e.path()
                            .strip_prefix(root)
                            .unwrap_or(e.path())
                            .display()
                            .to_string(),
                    );
                }
                if out.len() > 2000 {
                    out.push("...[truncated]".into());
                    break;
                }
            }
            Ok(out.join("\n"))
        }
        "search" => {
            let pattern = args["pattern"].as_str().unwrap_or_default();
            let dir = args["dir"].as_str().unwrap_or("crates");
            let cmd = format!(
                    "grep -rnE --include='*.rs' --include='*.toml' --include='*.md' -e {} {} | head -200",
                    shell_quote(pattern),
                    shell_quote(dir)
                );
            let (_, out, _) = run_shell(root, &cmd, Duration::from_secs(30), 20_000);
            Ok(if out.trim().is_empty() {
                "(no matches)".into()
            } else {
                out
            })
        }
        "bash" => {
            let command = args["command"].as_str().unwrap_or_default();
            if command.trim_start().starts_with("git ")
                || command.contains("&& git ")
                || command.contains("| git ")
            {
                anyhow::bail!("git commands are not allowed; the harness handles version control");
            }
            let t = args["timeout_secs"]
                .as_u64()
                .unwrap_or(300)
                .min(cfg.command_timeout_secs);
            let (ok, out, ms) = run_shell(root, command, Duration::from_secs(t), 12_000);
            Ok(format!(
                "[exit {}] ({} ms)\n{}",
                if ok { "0" } else { "non-zero" },
                ms,
                out
            ))
        }
        other => anyhow::bail!("unknown tool {other}"),
    })();
    match res {
        Ok(s) => s,
        Err(e) => format!("ERROR: {e}"),
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Run the built-in agent loop. Returns (summary, turns, llm_usd).
pub fn run_builtin(
    cfg: &AgentConfig,
    llm: &LlmClient,
    ws: &Workspace,
    task: &CodingTask,
    context_docs: &str,
) -> anyhow::Result<(String, u32, f64)> {
    let tools = tools();
    let mut messages = vec![
        Message::system(SYSTEM_PROMPT),
        Message::user(task_prompt(task, context_docs)),
    ];
    let spent_before = llm.spent();
    let opts = ChatOptions {
        temperature: Some(0.2),
        ..Default::default()
    };
    let mut turns = 0u32;
    let mut summary = String::new();
    while turns < cfg.max_turns {
        if llm.spent() - spent_before > cfg.max_usd_per_task {
            summary = format!(
                "stopped: task LLM budget ${:.2} exceeded",
                cfg.max_usd_per_task
            );
            break;
        }
        turns += 1;
        let resp = llm.chat("coder", &messages, &tools, &opts)?;
        let msg = resp.message.clone();
        messages.push(msg.clone());
        if msg.tool_calls.is_empty() {
            // Model replied in prose; nudge it to use tools or finish.
            if turns >= cfg.max_turns {
                summary = msg.content.clone();
                break;
            }
            messages.push(Message::user(
                "Use the tools to make progress, or call `finish` when done.",
            ));
            continue;
        }
        let mut finished = false;
        for call in &msg.tool_calls {
            if call.name == "finish" {
                summary = call.arguments["summary"]
                    .as_str()
                    .unwrap_or("finished")
                    .to_string();
                finished = true;
                messages.push(Message::tool_result(&call.id, &call.name, "ok"));
                continue;
            }
            tracing::info!(tool = %call.name, args = %truncate(&call.arguments.to_string(), 200), "agent tool");
            let out = exec_tool(ws, cfg, &call.name, &call.arguments);
            messages.push(Message::tool_result(&call.id, &call.name, out));
        }
        if finished {
            break;
        }
    }
    if summary.is_empty() {
        summary = format!("stopped after {turns} turns without calling finish");
    }
    Ok((summary, turns, llm.spent() - spent_before))
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!(
            "{}...",
            &s[..s
                .char_indices()
                .map(|(i, _)| i)
                .find(|&i| i >= n)
                .unwrap_or(n)]
        )
    }
}
