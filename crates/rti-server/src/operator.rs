//! The operator agent: the user's conversational interface to RTI. It can
//! inspect the archive, run experiments, queue tasks and steer the loop.

use std::sync::Arc;

use rti_core::ExperimentSpec;
use rti_llm::{ChatOptions, Message, ToolDef};
use serde_json::{json, Value};

use crate::state::AppState;

const MAX_TURNS: usize = 12;
const MAX_HISTORY: usize = 40;

fn system_prompt(state: &Arc<AppState>) -> String {
    let directives = state.directives.lock().unwrap().join("\n- ");
    format!(
        r#"You are the RTI operator: the human's interface to Recursive Trackmania Intelligence, an automated research system that studies a replicated Trackmania physics simulator, verifies results against an oracle, searches for fast lines with many methods, trains models and improves its own tooling.

You can inspect the archive (status, context, SQL over the DuckDB schema, tracks, trajectories, findings, experiments), run experiments synchronously, queue coding/verify/calibrate tasks, start/stop the autonomous research loop, and record research directives that the researcher brain will see in its context.

Be concise and concrete: report numbers, ids and hashes. When the user asks to "optimize track X", a good plan is: check the track's best times, run a beam or CEM search (20-100M ticks), then local_opt warm-started from the best hash, then verify. Prefer asking the loop to do long campaigns (loop_start) rather than running many experiments yourself. Explain what you did after using tools.

{}

Current user directives:
- {}"#,
        ExperimentSpec::schema_doc(),
        if directives.is_empty() {
            "(none)".to_string()
        } else {
            directives
        }
    )
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required})
}

pub fn tools() -> Vec<ToolDef> {
    let t = |name: &str, desc: &str, params: Value| ToolDef {
        name: name.into(),
        description: desc.into(),
        parameters: params,
    };
    vec![
        t("get_status", "Archive counts, loop state, LLM spend, oracle, throughput.", obj(json!({}), &[])),
        t("get_context", "The full research context summary the researcher brain reads.", obj(json!({}), &[])),
        t("query_archive", "Run a read-only SQL query (SELECT/WITH) against the DuckDB archive. Tables: tracks, physics, experiments, trajectories, verifications, models, findings, hypotheses, ledger, llm_calls, tasks, cycles.", obj(json!({"sql": {"type": "string"}}), &["sql"])),
        t("list_tracks", "Tracks with best sim and verified times.", obj(json!({}), &[])),
        t("get_track", "Track definition and its best trajectories.", obj(json!({"name": {"type": "string"}}), &["name"])),
        t("run_experiment", "Run an ExperimentSpec synchronously (seconds to minutes). Returns the summary.", obj(json!({"spec": {"type": "object"}}), &["spec"])),
        t("queue_task", "Queue a task: kind coding|verify|calibrate. For verify, description must be JSON {\"trajectory\": hash}.", obj(json!({"kind": {"type": "string"}, "title": {"type": "string"}, "description": {"type": "string"}, "priority": {"type": "integer"}}), &["kind", "title", "description"])),
        t("run_pending_tasks", "Execute up to `max` pending tasks now.", obj(json!({"max": {"type": "integer"}}), &[])),
        t("loop_start", "Start the autonomous research loop for `cycles` cycles (0 = until stopped or budget).", obj(json!({"cycles": {"type": "integer"}}), &[])),
        t("loop_stop", "Request the research loop to stop after the current cycle.", obj(json!({}), &[])),
        t("loop_status", "Loop state and last cycle report.", obj(json!({}), &[])),
        t("recent_findings", "Most recent findings.", obj(json!({"n": {"type": "integer"}}), &[])),
        t("recent_experiments", "Most recent experiments.", obj(json!({"n": {"type": "integer"}}), &[])),
        t("set_research_note", "Record a directive for the researcher brain (persisted; shown in its context every cycle).", obj(json!({"text": {"type": "string"}}), &["text"])),
        t("tmx_search", "Search Trackmania Exchange for maps. Params pass through to the TMX API: name, author, tag (comma-separated tag ids, e.g. 3=Tech, 25=Mini, 14=Ice, 15=Dirt), vehicle (1=CarSport), order1 (8=newest, 4=most awarded), count (≤50).", obj(json!({"name": {"type": "string"}, "author": {"type": "string"}, "tag": {"type": "string"}, "vehicle": {"type": "string"}, "order1": {"type": "string"}, "count": {"type": "integer"}}), &[])),
        t("import_map", "Download a TM2020 map from Trackmania Exchange (id or URL) or read a local .Map.Gbx, compile it into a simulator track and register it. Returns the compile report (coverage). The track can then be used in search experiments.", obj(json!({"source": {"type": "string"}, "name": {"type": "string"}}), &["source"])),
        t("list_maps", "Imported real-game maps with their track names, author times, TMX WRs and compile coverage.", obj(json!({"n": {"type": "integer"}}), &[])),
    ]
}

fn exec(state: &Arc<AppState>, name: &str, args: &Value) -> String {
    let res: anyhow::Result<Value> = (|| {
        Ok(match name {
            "get_status" => crate::api::status(state)?,
            "get_context" => {
                let s = state.session.lock().unwrap();
                json!(rti_research::context::build_context(&s)?)
            }
            "tmx_search" => {
                let mut params: Vec<(String, String)> = vec![];
                for k in ["name", "author", "tag", "vehicle", "order1"] {
                    if let Some(v) = args[k].as_str() {
                        params.push((k.to_string(), v.to_string()));
                    }
                }
                let count = args["count"].as_u64().unwrap_or(10).min(50);
                params.push(("count".into(), count.to_string()));
                let refs: Vec<(&str, &str)> = params
                    .iter()
                    .map(|(a, b)| (a.as_str(), b.as_str()))
                    .collect();
                let r = rti_maps::TmxClient::new().search(&refs)?;
                json!({"more": r.more, "results": r.results.iter().map(|m| json!({"map_id": m.map_id, "name": m.name, "authors": m.author_names(), "author_ms": m.length, "tags": m.tag_names(), "awards": m.award_count, "wr_ms": m.wr_ms(), "vehicle": m.vehicle_name, "map_type": m.map_type, "uploaded": m.uploaded_at})).collect::<Vec<_>>()})
            }
            "import_map" => {
                let spec = rti_core::ExperimentSpec::ImportMap {
                    source: args["source"].as_str().unwrap_or_default().to_string(),
                    name: args["name"].as_str().map(|s| s.to_string()),
                };
                let (id, rep) = state.run_spec(spec)?;
                state.push_event("import", &rep.summary);
                json!({"experiment_id": id, "summary": rep.summary, "result": rep.result})
            }
            "list_maps" => {
                let s = state.session.lock().unwrap();
                json!(s.archive.maps(args["n"].as_u64().unwrap_or(20) as usize)?)
            }
            "query_archive" => {
                let s = state.session.lock().unwrap();
                let rows = s
                    .archive
                    .query_json(args["sql"].as_str().unwrap_or_default())?;
                let n = rows.len();
                json!({"rows": rows.into_iter().take(200).collect::<Vec<_>>(), "total": n})
            }
            "list_tracks" => {
                let s = state.session.lock().unwrap();
                let mut out = vec![];
                for t in s.archive.tracks()? {
                    out.push(json!({"name": t.name, "length_m": t.length_m, "best_sim": s.archive.best_time(&t.name, "sim")?, "best_oracle": s.archive.best_time(&t.name, "oracle")?}));
                }
                Value::Array(out)
            }
            "get_track" => {
                let s = state.session.lock().unwrap();
                let name = args["name"].as_str().unwrap_or_default();
                let t = s.track(name)?;
                json!({"name": t.name, "description": t.description, "length_m": t.length(), "nodes": t.nodes.len(), "checkpoints": t.checkpoints, "walls": t.walls, "max_ticks": t.max_ticks,
                       "best_sim": s.archive.best_trajectories(name, "sim", 5)?, "best_oracle": s.archive.best_trajectories(name, "oracle", 5)?})
            }
            "run_experiment" => {
                let spec: ExperimentSpec = serde_json::from_value(args["spec"].clone())?;
                let (id, r) = state.run_spec(spec)?;
                json!({"id": id, "summary": r.summary, "trajectories": r.trajectories, "models": r.models, "cost": r.cost})
            }
            "queue_task" => {
                let t = rti_archive::rows::TaskRow::new(
                    args["kind"].as_str().unwrap_or("coding"),
                    args["title"].as_str().unwrap_or("untitled"),
                    args["description"].as_str().unwrap_or(""),
                    args["priority"].as_i64().unwrap_or(2) as i32,
                    None,
                );
                let s = state.session.lock().unwrap();
                let id = s.archive.insert_task(&t)?;
                state.push_event(
                    "task",
                    &format!("operator queued {} task {id}: {}", t.kind, t.title),
                );
                json!({"id": id})
            }
            "run_pending_tasks" => {
                let max = args["max"].as_u64().unwrap_or(1).min(10);
                let s = state.session.lock().unwrap();
                let mut out = vec![];
                for _ in 0..max {
                    match rti_research::tasks::run_next_task(&s)? {
                        Some((id, msg)) => {
                            state.push_event("task", &format!("{id}: {msg}"));
                            out.push(json!({"id": id, "result": msg}));
                        }
                        None => break,
                    }
                }
                json!(out)
            }
            "loop_start" => {
                json!({"started": state.loop_start(args["cycles"].as_u64().unwrap_or(0)), "loop": state.loop_status()})
            }
            "loop_stop" => {
                state.loop_stop();
                state.loop_status()
            }
            "loop_status" => state.loop_status(),
            "recent_findings" => {
                let s = state.session.lock().unwrap();
                serde_json::to_value(
                    s.archive
                        .findings(args["n"].as_u64().unwrap_or(10) as usize)?,
                )?
            }
            "recent_experiments" => {
                let s = state.session.lock().unwrap();
                let rows = s
                    .archive
                    .recent_experiments(args["n"].as_u64().unwrap_or(10) as usize)?;
                json!(rows.iter().map(|e| json!({"id": e.id, "cycle": e.cycle, "kind": e.kind, "method": e.method, "track": e.track, "status": e.status, "value_total": e.value_total, "summary": e.summary})).collect::<Vec<_>>())
            }
            "set_research_note" => {
                state.add_directive(args["text"].as_str().unwrap_or_default())?;
                json!({"ok": true})
            }
            other => anyhow::bail!("unknown tool {other}"),
        })
    })();
    match res {
        Ok(v) => {
            let s = v.to_string();
            if s.len() > 20_000 {
                format!(
                    "{}...[truncated]",
                    &s[..s
                        .char_indices()
                        .map(|(i, _)| i)
                        .find(|&i| i >= 20_000)
                        .unwrap_or(20_000)]
                )
            } else {
                s
            }
        }
        Err(e) => format!("ERROR: {e:#}"),
    }
}

/// Messages for the UI (role, content, tool calls), skipping the system prompt.
pub fn history_view(history: &[Message]) -> Vec<Value> {
    history
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            json!({
                "role": m.role,
                "content": m.content,
                "tool_calls": m.tool_calls.iter().map(|t| json!({"name": t.name, "arguments": t.arguments})).collect::<Vec<_>>(),
                "name": m.name,
            })
        })
        .collect()
}

pub fn chat_turn(state: &Arc<AppState>, user_message: &str) -> anyhow::Result<Value> {
    {
        let s = state.session.lock().unwrap();
        if !s.llm.is_configured() {
            anyhow::bail!("LLM not configured: set the {} environment variable (OpenRouter API key) and restart `rti serve`. The rest of the UI works without it.", s.cfg.llm.api_key_env);
        }
        let mut c = state.chat.lock().unwrap();
        if c.busy {
            anyhow::bail!("operator is busy with the previous message");
        }
        c.busy = true;
        c.history.push(Message::user(user_message));
    }
    let result = run_turn(state);
    let mut c = state.chat.lock().unwrap();
    c.busy = false;
    while c.history.len() > MAX_HISTORY {
        c.history.remove(0);
    }
    result
}

fn run_turn(state: &Arc<AppState>) -> anyhow::Result<Value> {
    let tools = tools();
    let opts = ChatOptions {
        temperature: Some(0.3),
        ..Default::default()
    };
    let mut calls_made = vec![];
    let mut reply = String::new();
    for _ in 0..MAX_TURNS {
        let mut messages = vec![Message::system(system_prompt(state))];
        messages.extend(state.chat.lock().unwrap().history.iter().cloned());
        let resp = {
            let s = state.session.lock().unwrap();
            let r = s.llm.chat("operator", &messages, &tools, &opts);
            let _ = s.flush_usage(None, None);
            r?
        };
        let msg = resp.message.clone();
        state.chat.lock().unwrap().history.push(msg.clone());
        if msg.tool_calls.is_empty() {
            reply = msg.content.clone();
            break;
        }
        for call in &msg.tool_calls {
            state.push_event(
                "operator",
                &format!(
                    "tool {}({})",
                    call.name,
                    rti_research::context::truncate(&call.arguments.to_string(), 160)
                ),
            );
            let out = exec(state, &call.name, &call.arguments);
            calls_made.push(json!({"name": call.name, "arguments": call.arguments, "result": rti_research::context::truncate(&out, 600)}));
            state
                .chat
                .lock()
                .unwrap()
                .history
                .push(Message::tool_result(&call.id, &call.name, out));
        }
    }
    if reply.is_empty() {
        reply = "(operator stopped after using tools; ask for a summary)".into();
    }
    Ok(json!({"reply": reply, "tool_calls": calls_made}))
}
