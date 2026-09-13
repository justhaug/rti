use std::path::Path;
use std::time::Duration;

use rti_core::config::AgentConfig;
use rti_core::CostRecord;
use rti_llm::LlmClient;
use serde::{Deserialize, Serialize};

use crate::gates::{bench_gate, run_gates, run_shell, GateResult};
use crate::workspace::Workspace;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodingTask {
    pub id: String,
    pub title: String,
    /// Full description: motivation, what to change, how to verify.
    pub description: String,
    /// Acceptance criteria the agent must satisfy (checked by gates + review).
    #[serde(default)]
    pub acceptance: Vec<String>,
    /// Files/crates the task most likely touches (hint only).
    #[serde(default)]
    pub files_hint: Vec<String>,
    /// Whether the sim throughput benchmark must not regress.
    #[serde(default = "default_true")]
    pub bench_guard: bool,
}

fn default_true() -> bool {
    true
}

impl CodingTask {
    pub fn new(title: &str, description: &str) -> CodingTask {
        CodingTask {
            id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
            title: title.to_string(),
            description: description.to_string(),
            acceptance: vec![],
            files_hint: vec![],
            bench_guard: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Changes merged into main.
    Merged,
    /// Gates passed but auto-merge disabled or main dirty; branch kept.
    Passed,
    /// Agent produced changes but gates failed; branch kept for inspection.
    Failed,
    /// Agent produced no changes.
    NoChange,
    /// Agent errored (LLM/budget/tooling).
    Errored,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskOutcome {
    pub task_id: String,
    pub status: TaskStatus,
    pub branch: String,
    pub commit: Option<String>,
    pub merge_commit: Option<String>,
    pub gates: Vec<GateResult>,
    pub bench_mticks: Option<f64>,
    pub summary: String,
    pub diff_stat: String,
    pub turns: u32,
    pub cost: CostRecord,
}

/// Execute a coding task end to end: worktree → agent → gates → merge.
pub fn run_task(
    cfg: &AgentConfig,
    llm: Option<&LlmClient>,
    repo_root: &Path,
    task: &CodingTask,
    baseline_mticks: Option<f64>,
    context_docs: &str,
) -> anyhow::Result<TaskOutcome> {
    let ws = Workspace::create(repo_root, &task.id)?;
    let timer = rti_core::ledger::CostTimer::start();
    let mut outcome = TaskOutcome {
        task_id: task.id.clone(),
        status: TaskStatus::Errored,
        branch: ws.branch.clone(),
        commit: None,
        merge_commit: None,
        gates: vec![],
        bench_mticks: None,
        summary: String::new(),
        diff_stat: String::new(),
        turns: 0,
        cost: CostRecord::default(),
    };

    // 1. run the agent
    let agent_result = match cfg.backend.as_str() {
        "external" => run_external(cfg, &ws, task, context_docs),
        _ => {
            let llm =
                llm.ok_or_else(|| anyhow::anyhow!("builtin coding agent needs an LLM client"))?;
            crate::builtin::run_builtin(cfg, llm, &ws, task, context_docs)
        }
    };
    let (summary, turns, llm_cost) = match agent_result {
        Ok(r) => r,
        Err(e) => {
            outcome.summary = format!("agent error: {e}");
            outcome.cost = timer.finish(0);
            ws.remove(true);
            return Ok(outcome);
        }
    };
    outcome.summary = summary;
    outcome.turns = turns;
    outcome.diff_stat = ws.diff_stat();

    if !ws.has_changes() {
        outcome.status = TaskStatus::NoChange;
        outcome.cost = timer.finish(0);
        outcome.cost.llm_usd = llm_cost;
        ws.remove(true);
        return Ok(outcome);
    }

    // 2. gates
    outcome.gates = run_gates(&ws.path, &cfg.gates, cfg.command_timeout_secs);
    let gates_ok = outcome.gates.iter().all(|g| g.ok);
    let mut bench_ok = true;
    if gates_ok && task.bench_guard {
        let (ok, measured, out) = bench_gate(
            &ws.path,
            baseline_mticks,
            cfg.max_bench_regression,
            cfg.command_timeout_secs,
        );
        outcome.bench_mticks = measured;
        outcome.gates.push(GateResult {
            command: "bench (sim throughput)".into(),
            ok,
            duration_ms: 0,
            output_tail: out,
        });
        bench_ok = ok;
    }
    outcome.cost = timer.finish(0);
    outcome.cost.llm_usd = llm_cost;

    if !(gates_ok && bench_ok) {
        outcome.status = TaskStatus::Failed;
        // keep the branch with a WIP commit for inspection
        outcome.commit = ws
            .commit(&format!("wip(rti): {} [gates failed]", task.title))
            .ok();
        ws.remove(false);
        return Ok(outcome);
    }

    // 3. commit + merge
    let commit = ws.commit(&format!(
        "rti: {}\n\nTask {}\n\n{}",
        task.title, task.id, outcome.summary
    ))?;
    outcome.commit = Some(commit);
    if cfg.auto_merge {
        match ws.merge_into_main(&format!("merge rti/{}: {}", task.id, task.title)) {
            Ok(mc) => {
                outcome.merge_commit = Some(mc);
                outcome.status = TaskStatus::Merged;
                ws.remove(true);
            }
            Err(e) => {
                outcome.summary.push_str(&format!("\n(merge skipped: {e})"));
                outcome.status = TaskStatus::Passed;
                ws.remove(false);
            }
        }
    } else {
        outcome.status = TaskStatus::Passed;
        ws.remove(false);
    }
    Ok(outcome)
}

/// Compose the prompt file handed to agents.
pub fn task_prompt(task: &CodingTask, context_docs: &str) -> String {
    let mut p = String::new();
    p.push_str(&format!(
        "# Task: {}\n\n{}\n\n",
        task.title, task.description
    ));
    if !task.acceptance.is_empty() {
        p.push_str("## Acceptance criteria\n");
        for a in &task.acceptance {
            p.push_str(&format!("- {a}\n"));
        }
        p.push('\n');
    }
    if !task.files_hint.is_empty() {
        p.push_str(&format!(
            "Likely relevant files: {}\n\n",
            task.files_hint.join(", ")
        ));
    }
    p.push_str("## Repository guide\n\n");
    p.push_str(context_docs);
    p
}

fn run_external(
    cfg: &AgentConfig,
    ws: &Workspace,
    task: &CodingTask,
    context_docs: &str,
) -> anyhow::Result<(String, u32, f64)> {
    let prompt_path = ws.path.join(".rti-task.md");
    std::fs::write(&prompt_path, task_prompt(task, context_docs))?;
    let cmd = cfg
        .external_command
        .replace("{prompt_file}", prompt_path.to_str().unwrap())
        .replace("{workdir}", ws.path.to_str().unwrap());
    let (ok, out, _) = run_shell(
        &ws.path,
        &cmd,
        Duration::from_secs(cfg.command_timeout_secs * 6),
        6000,
    );
    let _ = std::fs::remove_file(&prompt_path);
    if !ok {
        anyhow::bail!("external agent failed:\n{out}");
    }
    Ok((format!("external agent output (tail):\n{out}"), 1, 0.0))
}
