//! Executes queued tasks: verifications, calibrations and coding tasks.

use rti_agent::{run_task, CodingTask, TaskStatus};
use rti_archive::rows::TaskRow;
use rti_core::{ExperimentSpec, Provenance};

use crate::runner::run_experiment;
use crate::session::Session;

/// Run one pending task (highest priority first). Returns the task id, or
/// None if the queue is empty.
pub fn run_next_task(s: &Session) -> anyhow::Result<Option<(String, String)>> {
    let Some(task) = s.archive.pending_tasks()?.into_iter().next() else {
        return Ok(None);
    };
    s.archive
        .update_task(&task.id, "running", None, None, None)?;
    let res = match task.kind.as_str() {
        "coding" => run_coding_task(s, &task),
        "verify" | "calibrate" => run_experiment_task(s, &task),
        other => Err(anyhow::anyhow!("unknown task kind {other}")),
    };
    match res {
        Ok((status, summary)) => {
            s.archive.update_task(
                &task.id,
                &status,
                Some(&serde_json::json!({"summary": summary})),
                None,
                None,
            )?;
            Ok(Some((task.id, format!("{status}: {summary}"))))
        }
        Err(e) => {
            s.archive.update_task(
                &task.id,
                "failed",
                Some(&serde_json::json!({"error": e.to_string()})),
                None,
                None,
            )?;
            Ok(Some((task.id, format!("failed: {e}"))))
        }
    }
}

fn run_experiment_task(s: &Session, task: &TaskRow) -> anyhow::Result<(String, String)> {
    let spec = match task.kind.as_str() {
        "verify" => {
            let v: serde_json::Value = serde_json::from_str(&task.description)?;
            ExperimentSpec::Verify {
                trajectory: v["trajectory"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
                    .into(),
            }
        }
        _ => ExperimentSpec::Calibrate {
            tracks: vec![],
            trajectories: vec![],
            probe_runs: 6,
            budget_ticks: s.cfg.budget.default_ticks * 2,
            seed: 1,
            physics: None,
        },
    };
    let exp_id = s
        .archive
        .begin_experiment(&spec, None, task.cycle, &Provenance::now())?;
    let report = run_experiment(s, &spec, &exp_id)?;
    let cost = report.cost.clone();
    s.archive.finish_experiment(
        &exp_id,
        "done",
        &report.result,
        &cost,
        None,
        None,
        &report.summary,
    )?;
    s.archive
        .ledger_add(&format!("task:{}", task.kind), &exp_id, &cost, &task.title)?;
    Ok(("done".into(), report.summary))
}

fn run_coding_task(s: &Session, task: &TaskRow) -> anyhow::Result<(String, String)> {
    let ct = CodingTask {
        id: task.id.clone(),
        title: task.title.clone(),
        description: task.description.clone(),
        acceptance: vec![],
        files_hint: vec![],
        bench_guard: true,
    };
    let baseline = s
        .archive
        .recent_experiments(50)?
        .iter()
        .filter(|e| e.kind == "benchmark")
        .filter_map(|e| e.result.as_ref().and_then(|r| r["mticks_per_s"].as_f64()))
        .next();
    s.llm.reset_spent();
    s.llm.set_budget(Some(s.cfg.agent.max_usd_per_task));
    let llm = if s.cfg.agent.backend == "builtin" {
        Some(&s.llm)
    } else {
        None
    };
    let out = run_task(&s.cfg.agent, llm, &s.root, &ct, baseline, &s.repo_docs())?;
    let llm_cost = s.flush_usage(task.cycle, None)?;
    let mut cost = out.cost.clone();
    cost.llm_calls = llm_cost.llm_calls;
    cost.llm_prompt_tokens = llm_cost.llm_prompt_tokens;
    cost.llm_completion_tokens = llm_cost.llm_completion_tokens;
    cost.llm_usd = llm_cost.llm_usd.max(out.cost.llm_usd);
    s.archive
        .ledger_add("task:coding", &task.id, &cost, &task.title)?;
    let status = match out.status {
        TaskStatus::Merged => "merged",
        TaskStatus::Passed => "done",
        TaskStatus::Failed => "rejected",
        TaskStatus::NoChange => "rejected",
        TaskStatus::Errored => "failed",
    };
    s.archive.update_task(
        &task.id,
        status,
        Some(&serde_json::to_value(&out)?),
        Some(&cost),
        Some(&out.branch),
    )?;
    let gates = out
        .gates
        .iter()
        .map(|g| format!("{} {}", if g.ok { "✓" } else { "✗" }, g.command))
        .collect::<Vec<_>>()
        .join("; ");
    Ok((
        status.into(),
        format!(
            "{:?} after {} turns (${:.3}); gates: {}; {}\n{}",
            out.status, out.turns, cost.llm_usd, gates, out.diff_stat, out.summary
        ),
    ))
}
