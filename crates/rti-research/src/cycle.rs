//! One turn of the central loop.

use rti_archive::rows::{Finding, TaskRow};
use rti_core::{CostRecord, ExperimentSpec, Provenance};
use serde::{Deserialize, Serialize};

use crate::brain::{brain_for, Brain, Conclusion, Proposal};
use crate::context::build_context;
use crate::runner::{run_experiment, ExperimentReport};
use crate::session::Session;
use crate::value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CycleReport {
    pub cycle: i64,
    pub brain: String,
    pub experiment_id: String,
    pub proposal: Proposal,
    pub report: ExperimentReport,
    pub conclusion: Conclusion,
    pub value_total: f64,
    pub cost: CostRecord,
    pub follow_ups: Vec<String>,
}

/// Validate and normalise a spec against config limits and archive state.
/// Validate and normalise a spec against config limits and archive state.
/// Abbreviated hashes (as shown in the context) are expanded.
pub fn validate_spec(s: &Session, spec: &mut ExperimentSpec) -> anyhow::Result<()> {
    let cap = s.cfg.budget.max_ticks_per_experiment;
    let fix_traj = |h: &mut rti_core::ContentHash| -> anyhow::Result<()> {
        if s.archive.trajectory_row(h)?.is_some() {
            return Ok(());
        }
        match s.archive.resolve_trajectory_hash(h.as_str())? {
            Some(full) => {
                *h = full;
                Ok(())
            }
            None => anyhow::bail!("trajectory {h} does not exist"),
        }
    };
    let fix_phys = |h: &mut rti_core::ContentHash| -> anyhow::Result<()> {
        if s.archive.physics_by_hash(h)?.is_some() {
            return Ok(());
        }
        match s.archive.resolve_physics_hash(h.as_str())? {
            Some(full) => {
                *h = full;
                Ok(())
            }
            None => anyhow::bail!("physics {h} does not exist"),
        }
    };
    match spec {
        ExperimentSpec::Search {
            track,
            budget_ticks,
            warm_start,
            physics,
            params,
            ..
        } => {
            s.track(track)?;
            *budget_ticks = (*budget_ticks).clamp(100_000, cap);
            if let Some(h) = warm_start {
                fix_traj(h)?;
            }
            if let Some(h) = physics {
                fix_phys(h)?;
            }
            if let Some(m) = &mut params.model {
                if !s.archive.cas.exists(m) {
                    let full = s
                        .archive
                        .models("policy", 1000)?
                        .into_iter()
                        .find(|r| r.hash.starts_with(m.as_str()))
                        .map(|r| rti_core::ContentHash(r.hash));
                    match full {
                        Some(f) => *m = f,
                        None => anyhow::bail!("model {m} does not exist"),
                    }
                }
            }
        }
        ExperimentSpec::Verify { trajectory } => fix_traj(trajectory)?,
        ExperimentSpec::Calibrate {
            tracks,
            budget_ticks,
            probe_runs,
            trajectories,
            physics,
            ..
        } => {
            for t in tracks.iter() {
                s.track(t)?;
            }
            for h in trajectories.iter_mut() {
                fix_traj(h)?;
            }
            if let Some(h) = physics {
                fix_phys(h)?;
            }
            *budget_ticks = (*budget_ticks).clamp(1_000_000, cap);
            *probe_runs = (*probe_runs).clamp(1, 64);
        }
        ExperimentSpec::ReplayBench { tracks } => {
            for t in tracks.iter() {
                s.track(t)?;
            }
        }
        ExperimentSpec::CalibrateReplays {
            tracks,
            generations,
            ..
        } => {
            for t in tracks.iter() {
                s.track(t)?;
            }
            *generations = (*generations).clamp(1, 200);
        }
        ExperimentSpec::TrainBc {
            tracks,
            epochs,
            hidden,
            ..
        } => {
            for t in tracks.iter() {
                s.track(t)?;
            }
            *epochs = (*epochs).clamp(1, 200);
            *hidden = (*hidden).clamp(8, 512);
        }
        ExperimentSpec::Benchmark { ticks } => *ticks = (*ticks).clamp(1_000_000, cap),
        ExperimentSpec::GenerateTrack {
            segments,
            half_width,
            ..
        } => {
            *segments = (*segments).clamp(2, 40);
            *half_width = half_width.clamp(4.0, 16.0);
        }
        ExperimentSpec::ImportMap { source, .. } => {
            anyhow::ensure!(!source.trim().is_empty(), "import_map needs a source");
        }
        ExperimentSpec::ImportReplay { source, .. } => {
            anyhow::ensure!(
                source.starts_with("tmx:") || std::path::Path::new(source).is_file(),
                "import_replay: {source:?} is neither a file nor tmx:<replay id>"
            );
        }
    }
    Ok(())
}

pub fn run_cycle(s: &Session) -> anyhow::Result<CycleReport> {
    let brain = brain_for(s);
    run_cycle_with(s, brain.as_ref())
}

pub fn run_cycle_with(s: &Session, brain: &dyn Brain) -> anyhow::Result<CycleReport> {
    // budget guard
    let spent = s.archive.llm_spend_total()?;
    if spent >= s.cfg.budget.max_total_usd {
        anyhow::bail!(
            "lifetime LLM budget exhausted (${spent:.2} ≥ ${:.2}); raise [budget].max_total_usd",
            s.cfg.budget.max_total_usd
        );
    }
    s.llm.reset_spent();
    s.llm.set_budget(Some(s.cfg.budget.max_usd_per_cycle));
    let model = s.llm.model_for("researcher");
    let cycle = s.archive.begin_cycle(&brain.name(), &model)?;
    tracing::info!(cycle, brain = %brain.name(), "cycle start");

    // 1. read accumulated knowledge
    let context = build_context(s)?;

    // 2-4. select uncertainty, hypothesis, experiment design (+ critic)
    let mut proposal = brain.propose(s, &context)?;
    if let Some(c) = brain.critique(s, &context, &proposal)? {
        tracing::info!(verdict = %c.verdict, score = c.score, "critic");
        proposal.critique = Some(format!("[{} {:.2}] {}", c.verdict, c.score, c.critique));
        if c.verdict == "revise" {
            if let Some(rev) = c.revised_experiment {
                proposal.experiment = rev;
            }
        } else if c.verdict == "reject" {
            // one more attempt with the critique in context
            let ctx2 = format!(
                "{context}\n\n## Critic rejected the previous proposal\n{}\n{}",
                serde_json::to_string(&proposal.experiment)?,
                c.critique
            );
            proposal = brain.propose(s, &ctx2)?;
            proposal.critique = Some(format!("[rejected once] {}", c.critique));
        }
    }
    validate_spec(s, &mut proposal.experiment)?;
    let hyp_id =
        s.archive
            .insert_hypothesis(&proposal.hypothesis, &proposal.rationale, Some(cycle))?;

    // 5. run
    let prov = Provenance::now();
    let exp_id =
        s.archive
            .begin_experiment(&proposal.experiment, Some(&hyp_id), Some(cycle), &prov)?;
    tracing::info!(experiment = %exp_id, kind = proposal.experiment.kind(), "running: {}", proposal.hypothesis);
    let report = match run_experiment(s, &proposal.experiment, &exp_id) {
        Ok(r) => r,
        Err(e) => {
            let cost = s.flush_usage(Some(cycle), Some(&exp_id))?;
            s.archive.finish_experiment(
                &exp_id,
                "failed",
                &serde_json::json!({"error": e.to_string()}),
                &cost,
                None,
                None,
                &format!("failed: {e}"),
            )?;
            s.archive.set_hypothesis_status(&hyp_id, "inconclusive")?;
            s.archive
                .finish_cycle(cycle, &format!("experiment failed: {e}"), &cost)?;
            return Err(e);
        }
    };

    // 6. analyse + archive
    let conclusion = brain
        .conclude(s, &context, &proposal, &report)
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "analyst failed; using summary");
            Conclusion {
                title: crate::context::truncate(&report.summary, 80),
                kind: "result".into(),
                body: report.summary.clone(),
                confidence: 0.4,
                hypothesis_status: "inconclusive".into(),
                information_gain: 0.2,
                follow_ups: vec![],
                tags: vec![],
            }
        });
    let llm_cost = s.flush_usage(Some(cycle), Some(&exp_id))?;
    let mut cost = report.cost.clone();
    cost += &llm_cost;
    let vs = value::assemble(
        value::race_component(report.improvement_ms, report.improvement_verified),
        conclusion.information_gain.clamp(0.0, 1.0),
        report.novelty.clamp(0.0, 1.0),
        value::sim_accuracy_component(report.divergence_before, report.divergence_after),
        cost.mticks(),
        cost.llm_usd,
    );
    let value_total = vs.total(&s.cfg.value);
    s.archive.finish_experiment(
        &exp_id,
        "done",
        &report.result,
        &cost,
        Some(&vs),
        Some(value_total),
        &report.summary,
    )?;
    s.archive.ledger_add(
        proposal.experiment.kind(),
        &exp_id,
        &cost,
        &format!("cycle {cycle}"),
    )?;
    s.archive.set_hypothesis_status(
        &hyp_id,
        if conclusion.hypothesis_status.is_empty() {
            "inconclusive"
        } else {
            &conclusion.hypothesis_status
        },
    )?;
    let finding = Finding {
        id: String::new(),
        cycle: Some(cycle),
        kind: conclusion.kind.clone(),
        title: conclusion.title.clone(),
        body: conclusion.body.clone(),
        confidence: conclusion.confidence,
        experiment_ids: vec![exp_id.clone()],
        tags: conclusion.tags.clone(),
        created_at: String::new(),
    };
    s.archive.insert_finding(&finding)?;

    // 7. follow-ups: verification, sim-improvement, coding tasks
    let mut follow_ups = vec![];
    if report.result["beats_verified"].as_bool().unwrap_or(false) {
        if let Some(h) = report.trajectories.first() {
            let t = TaskRow::new(
                "verify",
                &format!("verify {} (beats verified best)", h.short()),
                &format!("{{\"trajectory\":\"{}\"}}", h),
                5,
                Some(cycle),
            );
            s.archive.insert_task(&t)?;
            follow_ups.push(format!("queued verification of {}", h.short()));
        }
    }
    if report.result["disagreement"].as_bool().unwrap_or(false) {
        let recent_cal = s
            .archive
            .recent_experiments(5)?
            .iter()
            .any(|e| e.kind == "calibrate");
        if !recent_cal {
            let t = TaskRow::new(
                "calibrate",
                "calibrate physics after sim/oracle disagreement",
                "{}",
                4,
                Some(cycle),
            );
            s.archive.insert_task(&t)?;
            follow_ups.push("queued calibration (disagreement above threshold)".into());
        }
    }
    if let ExperimentSpec::Calibrate { .. } = proposal.experiment {
        if !report.result["improved"].as_bool().unwrap_or(true) {
            let pending = s
                .archive
                .pending_tasks()?
                .iter()
                .any(|t| t.kind == "coding");
            if !pending {
                let desc = format!(
                    "Calibration cannot reduce sim/oracle divergence any further (mean position error {:.2} m). The public model in crates/rti-sim/src/physics.rs lacks an effect the oracle has. Investigate the archived verifications (tables verifications, trajectories; oracle trajectories carry per-tick states) and find where and how divergence grows (speed-dependent? during braking? on a particular surface? in yaw?). Add the missing physics term to PhysicsParams (keeping determinism and throughput), with a test, so that a subsequent calibration can fit it.\n\nEvidence:\n{}",
                    report.divergence_after.unwrap_or(0.0),
                    report.summary
                );
                let t = TaskRow::new(
                    "coding",
                    "sim improvement: model the unexplained divergence",
                    &desc,
                    3,
                    Some(cycle),
                );
                s.archive.insert_task(&t)?;
                follow_ups.push("queued sim-improvement coding task".into());
            }
        }
    }
    if let Some(ct) = &proposal.coding_task {
        let n_pending = s
            .archive
            .pending_tasks()?
            .iter()
            .filter(|t| t.kind == "coding")
            .count() as u32;
        if n_pending < s.cfg.research.max_coding_tasks_per_cycle {
            let desc = format!(
                "{}\n\nAcceptance:\n{}\n\nFiles: {}",
                ct.description,
                ct.acceptance
                    .iter()
                    .map(|a| format!("- {a}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                ct.files_hint.join(", ")
            );
            let t = TaskRow::new("coding", &ct.title, &desc, 2, Some(cycle));
            s.archive.insert_task(&t)?;
            follow_ups.push(format!("queued coding task: {}", ct.title));
        }
    }
    for f in &conclusion.follow_ups {
        follow_ups.push(format!("suggested: {f}"));
    }

    s.archive.finish_cycle(
        cycle,
        &format!(
            "{} | V={value_total:.2} | {}",
            conclusion.title, report.summary
        ),
        &cost,
    )?;
    tracing::info!(cycle, value = value_total, "cycle done: {}", report.summary);
    if let Err(e) = s.oracle.maintenance() {
        tracing::warn!(error = %e, "oracle maintenance failed");
    }
    Ok(CycleReport {
        cycle,
        brain: brain.name(),
        experiment_id: exp_id,
        proposal,
        report,
        conclusion,
        value_total,
        cost,
        follow_ups,
    })
}
