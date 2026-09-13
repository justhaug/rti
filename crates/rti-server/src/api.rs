//! JSON API routing on top of tiny_http.

use std::sync::Arc;

use rti_core::action::compress_actions;
use rti_core::{ContentHash, ExperimentSpec};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response};

use crate::state::AppState;

fn json_response(status: u16, v: &Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(v).unwrap_or_default();
    Response::from_data(body)
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
}

fn err(e: impl std::fmt::Display) -> Value {
    json!({"error": e.to_string()})
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (k == key).then(|| v.to_string())
    })
}

fn n_param(url: &str, default: usize) -> usize {
    query_param(url, "n")
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
        .min(1000)
}

pub fn handle(state: &Arc<AppState>, mut req: Request) {
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("").to_string();
    let method = req.method().clone();
    let mut body = String::new();
    if method == Method::Post {
        let _ = req.as_reader().read_to_string(&mut body);
    }
    if path == "/" || path == "/index.html" {
        let r = Response::from_string(crate::ui::INDEX_HTML)
            .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap());
        let _ = req.respond(r);
        return;
    }
    let (status, v) = match route(state, &method, &path, &url, &body) {
        Ok(v) => (200, v),
        Err(e) => (400, err(format!("{e:#}"))),
    };
    let _ = req.respond(json_response(status, &v));
}

fn body_json(body: &str) -> anyhow::Result<Value> {
    if body.trim().is_empty() {
        return Ok(json!({}));
    }
    Ok(serde_json::from_str(body)?)
}

fn route(
    state: &Arc<AppState>,
    method: &Method,
    path: &str,
    url: &str,
    body: &str,
) -> anyhow::Result<Value> {
    let get = *method == Method::Get;
    let post = *method == Method::Post;
    let seg: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match (seg.as_slice(), get, post) {
        (["api", "status"], true, _) => status(state),
        (["api", "context"], true, _) => {
            let s = state.session.lock().unwrap();
            Ok(json!({"text": rti_research::context::build_context(&s)?}))
        }
        (["api", "experiments"], true, _) => {
            let s = state.session.lock().unwrap();
            Ok(serde_json::to_value(
                s.archive.recent_experiments(n_param(url, 50))?,
            )?)
        }
        (["api", "experiment", id], true, _) => {
            let s = state.session.lock().unwrap();
            let row = s
                .archive
                .experiment(id)?
                .ok_or_else(|| anyhow::anyhow!("no experiment {id}"))?;
            let trajs = s.archive.trajectories_for_experiment(id)?;
            Ok(json!({"experiment": row, "trajectories": trajs}))
        }
        (["api", "findings"], true, _) => {
            let s = state.session.lock().unwrap();
            Ok(serde_json::to_value(s.archive.findings(n_param(url, 50))?)?)
        }
        (["api", "hypotheses"], true, _) => {
            let s = state.session.lock().unwrap();
            Ok(serde_json::to_value(
                s.archive.hypotheses(None, n_param(url, 50))?,
            )?)
        }
        (["api", "tasks"], true, _) => {
            let s = state.session.lock().unwrap();
            Ok(serde_json::to_value(s.archive.tasks(n_param(url, 100))?)?)
        }
        (["api", "tasks"], _, true) => {
            let b = body_json(body)?;
            let t = rti_archive::rows::TaskRow::new(
                b["kind"].as_str().unwrap_or("coding"),
                b["title"].as_str().unwrap_or("untitled"),
                b["description"].as_str().unwrap_or(""),
                b["priority"].as_i64().unwrap_or(2) as i32,
                None,
            );
            let s = state.session.lock().unwrap();
            let id = s.archive.insert_task(&t)?;
            state.push_event("task", &format!("queued {} task {id}: {}", t.kind, t.title));
            Ok(json!({"id": id}))
        }
        (["api", "tasks", "run"], _, true) => {
            let s = state.session.lock().unwrap();
            match rti_research::tasks::run_next_task(&s)? {
                Some((id, msg)) => {
                    state.push_event("task", &format!("{id}: {msg}"));
                    Ok(json!({"id": id, "result": msg}))
                }
                None => Ok(json!({"id": null, "result": "no pending tasks"})),
            }
        }
        (["api", "maps"], true, _) => {
            let s = state.session.lock().unwrap();
            Ok(serde_json::to_value(s.archive.maps(n_param(url, 100))?)?)
        }
        (["api", "import"], _, true) => {
            let b = body_json(body)?;
            let spec = rti_core::ExperimentSpec::ImportMap {
                source: b["source"].as_str().unwrap_or_default().to_string(),
                name: b["name"]
                    .as_str()
                    .filter(|n| !n.trim().is_empty())
                    .map(|n| n.to_string()),
            };
            let (id, rep) = state.run_spec(spec)?;
            state.push_event("import", &rep.summary);
            Ok(json!({"experiment_id": id, "summary": rep.summary, "result": rep.result}))
        }
        (["api", "tmx", "search"], true, _) => {
            let q = url.split_once('?').map(|(_, q)| q).unwrap_or("");
            let mut params: Vec<(String, String)> = vec![];
            for kv in q.split('&').filter(|s| !s.is_empty()) {
                let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                let v = v.replace('+', " ");
                let v = percent_decode(&v);
                if [
                    "name", "author", "tag", "vehicle", "order1", "count", "after",
                ]
                .contains(&k)
                    && !v.is_empty()
                {
                    params.push((k.to_string(), v));
                }
            }
            let refs: Vec<(&str, &str)> = params
                .iter()
                .map(|(a, b)| (a.as_str(), b.as_str()))
                .collect();
            let r = rti_maps::TmxClient::new().search(&refs)?;
            Ok(
                json!({"more": r.more, "results": r.results.iter().map(|m| json!({"map_id": m.map_id, "name": m.name, "authors": m.author_names(), "author_ms": m.length, "tags": m.tag_names(), "awards": m.award_count, "wr_ms": m.wr_ms(), "vehicle": m.vehicle_name, "uploaded": m.uploaded_at})).collect::<Vec<_>>()}),
            )
        }
        (["api", "cycles"], true, _) => {
            let s = state.session.lock().unwrap();
            Ok(serde_json::to_value(
                s.archive.recent_cycles(n_param(url, 20))?,
            )?)
        }
        (["api", "ledger"], true, _) => {
            let s = state.session.lock().unwrap();
            Ok(json!({
                "totals": s.archive.ledger_totals()?,
                "by_category": s.archive.ledger_by_category()?.into_iter().map(|(c, r)| json!({"category": c, "cost": r})).collect::<Vec<_>>(),
                "models": s.archive.llm_call_stats()?,
                "llm_spend_total": s.archive.llm_spend_total()?,
            }))
        }
        (["api", "methods"], true, _) => {
            let s = state.session.lock().unwrap();
            Ok(serde_json::to_value(s.archive.method_stats()?)?)
        }
        (["api", "verifications"], true, _) => {
            let s = state.session.lock().unwrap();
            Ok(
                json!({"recent": s.archive.recent_verifications(n_param(url, 30))?, "stats": s.archive.verification_stats()?}),
            )
        }
        (["api", "tracks"], true, _) => {
            let s = state.session.lock().unwrap();
            let mut out = vec![];
            for t in s.archive.tracks()? {
                let bs = s.archive.best_time(&t.name, "sim")?;
                let bo = s.archive.best_time(&t.name, "oracle")?;
                out.push(json!({
                    "name": t.name, "hash": t.hash, "length_m": t.length_m, "n_nodes": t.n_nodes,
                    "best_sim": bs.map(|(h, ms)| json!({"hash": h, "time_ms": ms})),
                    "best_oracle": bo.map(|(h, ms)| json!({"hash": h, "time_ms": ms})),
                }));
            }
            Ok(Value::Array(out))
        }
        (["api", "track", name], true, _) => {
            let s = state.session.lock().unwrap();
            let t = s.track(name)?;
            Ok(json!({
                "track": t,
                "best_sim": s.archive.best_trajectories(name, "sim", 5)?,
                "best_oracle": s.archive.best_trajectories(name, "oracle", 5)?,
            }))
        }
        (["api", "trajectory", hash], true, _) => {
            let s = state.session.lock().unwrap();
            let t = s
                .archive
                .trajectory(&ContentHash(hash.to_string()))?
                .ok_or_else(|| anyhow::anyhow!("no trajectory {hash}"))?;
            let step = (t.states.len() / 600).max(1);
            let states: Vec<Value> = t
                .states
                .iter()
                .enumerate()
                .filter(|(i, _)| i % step == 0 || *i + 1 == t.states.len())
                .map(|(_, st)| json!({"tick": st.tick, "x": st.x, "y": st.y, "heading": st.heading, "speed": st.speed()}))
                .collect();
            Ok(json!({
                "hash": hash, "track_name": t.track_name, "track_hash": t.track_hash, "world": t.world, "method": t.method,
                "parent": t.parent, "result": t.result, "n_actions": t.actions.len(), "n_states": t.states.len(),
                "actions": compress_actions(&t.actions), "states": states,
            }))
        }
        (["api", "query"], _, true) => {
            let b = body_json(body)?;
            let sql = b["sql"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing sql"))?;
            let s = state.session.lock().unwrap();
            Ok(json!({"rows": s.archive.query_json(sql)?}))
        }
        (["api", "experiment"], _, true) => {
            let b = body_json(body)?;
            let spec: ExperimentSpec = serde_json::from_value(b["spec"].clone())?;
            let (id, r) = state.run_spec(spec)?;
            Ok(
                json!({"id": id, "summary": r.summary, "result": r.result, "cost": r.cost, "trajectories": r.trajectories, "models": r.models}),
            )
        }
        (["api", "loop", "start"], _, true) => {
            let b = body_json(body)?;
            let started = state.loop_start(b["cycles"].as_u64().unwrap_or(0));
            Ok(json!({"started": started, "loop": state.loop_status()}))
        }
        (["api", "loop", "stop"], _, true) => {
            state.loop_stop();
            Ok(json!({"loop": state.loop_status()}))
        }
        (["api", "loop", "once"], _, true) => {
            if state
                .loop_ctl
                .running
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                anyhow::bail!("loop already running");
            }
            let v = state.cycle_once()?;
            Ok(json!({"report": v, "loop": state.loop_status()}))
        }
        (["api", "loop"], true, _) => Ok(state.loop_status()),
        (["api", "events"], true, _) => {
            let since = query_param(url, "since")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            Ok(
                json!({"events": state.events_since(since), "seq": state.event_seq.load(std::sync::atomic::Ordering::SeqCst)}),
            )
        }
        (["api", "directives"], true, _) => {
            Ok(json!({"directives": *state.directives.lock().unwrap()}))
        }
        (["api", "directives"], _, true) => {
            let b = body_json(body)?;
            let text = b["text"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing text"))?;
            state.add_directive(text)?;
            Ok(json!({"ok": true}))
        }
        (["api", "chat"], true, _) => {
            let c = state.chat.lock().unwrap();
            Ok(json!({"history": crate::operator::history_view(&c.history), "busy": c.busy}))
        }
        (["api", "chat"], _, true) => {
            let b = body_json(body)?;
            let msg = b["message"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing message"))?;
            crate::operator::chat_turn(state, msg)
        }
        (["api", "chat", "clear"], _, true) => {
            state.chat.lock().unwrap().history.clear();
            Ok(json!({"ok": true}))
        }
        _ => anyhow::bail!("not found: {method} {path}"),
    }
}

pub fn status(state: &Arc<AppState>) -> anyhow::Result<Value> {
    let s = state.session.lock().unwrap();
    let mut v = s.archive.status()?;
    let totals = s.archive.ledger_totals()?;
    let bench = s
        .archive
        .recent_experiments(100)?
        .iter()
        .filter(|e| e.kind == "benchmark")
        .filter_map(|e| e.result.as_ref().and_then(|r| r["mticks_per_s"].as_f64()))
        .next();
    v["loop"] = state.loop_status();
    v["llm"] = json!({
        "configured": s.llm.is_configured(),
        "roles": s.cfg.llm.roles,
        "spend_total_usd": s.archive.llm_spend_total()?,
        "budget_total_usd": s.cfg.budget.max_total_usd,
        "budget_cycle_usd": s.cfg.budget.max_usd_per_cycle,
    });
    v["oracle"] = json!(s.oracle.name());
    v["brain"] = json!(s.cfg.research.brain);
    v["mticks_per_s"] = json!(bench.unwrap_or(totals.ticks_per_second() / 1e6));
    v["directives"] = json!(state.directives.lock().unwrap().len());
    v["data_dir"] = json!(s.cfg.data_dir.display().to_string());
    Ok(v)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
