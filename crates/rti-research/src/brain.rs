//! The researcher/critic/analyst roles. `LlmBrain` uses OpenRouter through
//! `rti-llm`; `ScriptedBrain` is a deterministic fallback so the whole loop
//! runs (and is testable) without any API key.

use rti_core::experiment::{SearchMethod, SearchParams};
use rti_core::ExperimentSpec;
use rti_llm::{ChatOptions, Message};
use serde::{Deserialize, Serialize};

use crate::runner::ExperimentReport;
use crate::session::Session;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ValueEstimate {
    #[serde(default)]
    pub race_improvement: f64,
    #[serde(default)]
    pub information_gain: f64,
    #[serde(default)]
    pub novelty: f64,
    #[serde(default)]
    pub sim_accuracy: f64,
    #[serde(default)]
    pub efficiency: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodingTaskProposal {
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub acceptance: Vec<String>,
    #[serde(default)]
    pub files_hint: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proposal {
    pub hypothesis: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub prediction: String,
    pub experiment: ExperimentSpec,
    #[serde(default)]
    pub value_estimate: ValueEstimate,
    #[serde(default)]
    pub coding_task: Option<CodingTaskProposal>,
    /// Filled in by the critic pass.
    #[serde(default)]
    pub critique: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conclusion {
    pub title: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    pub body: String,
    #[serde(default = "default_conf")]
    pub confidence: f32,
    #[serde(default)]
    pub hypothesis_status: String,
    #[serde(default)]
    pub information_gain: f64,
    #[serde(default)]
    pub follow_ups: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_kind() -> String {
    "result".into()
}
fn default_conf() -> f32 {
    0.5
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Critique {
    pub verdict: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub critique: String,
    #[serde(default)]
    pub revised_experiment: Option<ExperimentSpec>,
}

pub trait Brain {
    fn name(&self) -> String;
    fn propose(&self, s: &Session, context: &str) -> anyhow::Result<Proposal>;
    fn critique(
        &self,
        s: &Session,
        context: &str,
        proposal: &Proposal,
    ) -> anyhow::Result<Option<Critique>>;
    fn conclude(
        &self,
        s: &Session,
        context: &str,
        proposal: &Proposal,
        report: &ExperimentReport,
    ) -> anyhow::Result<Conclusion>;
}

// ---------------------------------------------------------------- LLM brain

pub struct LlmBrain;

impl Brain for LlmBrain {
    fn name(&self) -> String {
        "llm".into()
    }

    fn propose(&self, s: &Session, context: &str) -> anyhow::Result<Proposal> {
        let system = format!("{}\n\n{}\n\nAvailable search methods: {}.\nBudget rules: budget_ticks ≤ {}; a typical exploratory search is 20-50M ticks (~0.3-1 s of compute); verification and calibration use the oracle and are slower.",
            s.prompt("researcher"), ExperimentSpec::schema_doc(), SearchMethod::ALL.iter().map(|m| m.name()).collect::<Vec<_>>().join(", "), s.cfg.budget.max_ticks_per_experiment);
        let msgs = vec![
            Message::system(system),
            Message::user(format!("{context}\n\nPropose the next experiment.")),
        ];
        let (p, _): (Proposal, _) =
            s.llm
                .chat_json("researcher", &msgs, &ChatOptions::default())?;
        Ok(p)
    }

    fn critique(
        &self,
        s: &Session,
        context: &str,
        proposal: &Proposal,
    ) -> anyhow::Result<Option<Critique>> {
        if !s.cfg.research.use_critic {
            return Ok(None);
        }
        let system = format!("{}\n\n{}", s.prompt("critic"), ExperimentSpec::schema_doc());
        let user = format!(
            "{context}\n\n## Proposal\n{}",
            serde_json::to_string_pretty(proposal)?
        );
        let msgs = vec![Message::system(system), Message::user(user)];
        let (c, _): (Critique, _) = s.llm.chat_json(
            "critic",
            &msgs,
            &ChatOptions {
                temperature: Some(0.2),
                ..Default::default()
            },
        )?;
        Ok(Some(c))
    }

    fn conclude(
        &self,
        s: &Session,
        context: &str,
        proposal: &Proposal,
        report: &ExperimentReport,
    ) -> anyhow::Result<Conclusion> {
        let system = s.prompt("analyst");
        let mut result = report.result.clone();
        // keep the payload small: drop long histories
        if let Some(obj) = result.as_object_mut() {
            for k in ["history", "cells", "param_log_deltas"] {
                if let Some(v) = obj.get_mut(k) {
                    if let Some(arr) = v.as_array_mut() {
                        if arr.len() > 12 {
                            let keep: Vec<_> = arr
                                .iter()
                                .step_by((arr.len() / 12).max(1))
                                .cloned()
                                .collect();
                            *arr = keep;
                        }
                    }
                }
            }
            if let Some(extra) = obj.get_mut("extra").and_then(|e| e.as_object_mut()) {
                extra.remove("cells");
            }
        }
        let user = format!(
            "## Archive context (abridged)\n{}\n\n## Hypothesis\n{}\n\n## Prediction\n{}\n\n## Experiment\n{}\n\n## Result summary\n{}\n\n## Result data\n{}\n\n## Cost\n{}",
            crate::context::truncate(context, 6000),
            proposal.hypothesis,
            proposal.prediction,
            serde_json::to_string(&proposal.experiment)?,
            report.summary,
            crate::context::truncate(&result.to_string(), 6000),
            serde_json::to_string(&report.cost)?
        );
        let msgs = vec![Message::system(system), Message::user(user)];
        let (c, _): (Conclusion, _) = s.llm.chat_json(
            "researcher",
            &msgs,
            &ChatOptions {
                temperature: Some(0.3),
                ..Default::default()
            },
        )?;
        Ok(c)
    }
}

// ------------------------------------------------------------ scripted brain

/// Deterministic research policy: a sensible bootstrap curriculum that
/// exercises every subsystem. Used when no API key is configured, in tests,
/// and as a baseline the LLM brain must beat.
pub struct ScriptedBrain;

impl ScriptedBrain {
    fn pick(&self, s: &Session) -> anyhow::Result<(String, String, ExperimentSpec)> {
        let a = &s.archive;
        let n_exp = a.experiment_count()?;
        let tracks = a.tracks()?;
        let ticks = s.cfg.budget.default_ticks;
        if n_exp == 0 {
            return Ok((
                "Throughput baseline".into(),
                "Every later cost estimate depends on it.".into(),
                ExperimentSpec::Benchmark { ticks: 20_000_000 },
            ));
        }
        // 1. verify any sim best that beats the verified best
        for t in &tracks {
            let bs = a.best_time(&t.name, "sim")?;
            let bo = a.best_time(&t.name, "oracle")?;
            if let Some((h, ms)) = bs {
                let needs = match bo {
                    None => true,
                    Some((_, oms)) => ms + s.cfg.research.verify_improvement_ms <= oms,
                };
                let already = a
                    .recent_verifications(50)?
                    .iter()
                    .any(|v| v.trajectory_hash == h.0);
                if needs && !already {
                    return Ok((
                        format!(
                            "Sim best on {} ({ms} ms) will hold up in the oracle within 5%",
                            t.name
                        ),
                        "Unverified sim results are hypotheses, not knowledge.".into(),
                        ExperimentSpec::Verify { trajectory: h },
                    ));
                }
            }
        }
        // 2. calibrate when disagreement is large and we have not calibrated recently
        let vs = a.verification_stats()?;
        let recent_cal = a
            .recent_experiments(6)?
            .iter()
            .any(|e| e.kind == "calibrate");
        if vs.n >= 2
            && vs.mean_pos_err > s.cfg.oracle.disagreement_threshold_m as f64
            && !recent_cal
        {
            return Ok((
                format!("Refitting PhysicsParams to oracle probes cuts mean divergence ({:.2} m) by at least 30%", vs.mean_pos_err),
                "Divergence above threshold; calibration is cheaper than sim redesign.".into(),
                ExperimentSpec::Calibrate { tracks: vec![], trajectories: vec![], probe_runs: 6, budget_ticks: ticks * 2, seed: n_exp, physics: None },
            ));
        }
        // 3. methods × tracks coverage: pick the (track, method) pair with the fewest runs
        let stats = a.method_stats()?;
        let mut best: Option<(u64, String, SearchMethod)> = None;
        for t in &tracks {
            for m in [
                SearchMethod::Beam,
                SearchMethod::Cem,
                SearchMethod::LocalOpt,
                SearchMethod::CmaEs,
                SearchMethod::MapElites,
                SearchMethod::RandomShooting,
            ] {
                let n = stats
                    .iter()
                    .find(|x| x.track == t.name && x.method == m.name())
                    .map(|x| x.n_runs)
                    .unwrap_or(0);
                if best.as_ref().map(|b| n < b.0).unwrap_or(true) {
                    best = Some((n, t.name.clone(), m));
                }
            }
        }
        // 4. once every track has several finished runs, train a policy and use it
        let has_policy = a.latest_model("policy")?.is_some();
        let finished_tracks = tracks
            .iter()
            .filter(|t| a.best_time(&t.name, "sim").ok().flatten().is_some())
            .count();
        if finished_tracks == tracks.len() && !has_policy && n_exp > 6 {
            return Ok((
                "A behaviour-cloned policy from the best sim trajectories finishes every track deterministically".into(),
                "Nets give intuition; we need one before value-guided beam search makes sense.".into(),
                ExperimentSpec::TrainBc { tracks: vec![], top_k: 5, epochs: 20, hidden: 64, seed: n_exp, value_head: true },
            ));
        }
        if let Some((n, track, method)) = best {
            if n == 0 || n_exp % 3 != 0 {
                let warm = if matches!(method, SearchMethod::LocalOpt) {
                    a.best_time(&track, "sim")?.map(|(h, _)| h)
                } else {
                    None
                };
                return Ok((
                    format!(
                        "{} finds a finishing line on {} within {} Mticks",
                        method.name(),
                        track,
                        ticks / 1_000_000
                    ),
                    format!("Coverage: {} has {} runs on {}.", method.name(), n, track),
                    ExperimentSpec::Search {
                        track,
                        method,
                        budget_ticks: ticks,
                        seed: n_exp,
                        params: SearchParams::default(),
                        warm_start: warm,
                        physics: None,
                    },
                ));
            }
        }
        // 5. otherwise: polish the best track with local opt from the best trajectory, or policy search
        let t = &tracks[(n_exp as usize) % tracks.len()];
        let warm = a.best_time(&t.name, "sim")?.map(|(h, _)| h);
        let method = if has_policy && n_exp % 2 == 0 {
            SearchMethod::Policy
        } else {
            SearchMethod::LocalOpt
        };
        Ok((
            format!(
                "{} improves the sim best on {} by ≥ 20 ms",
                method.name(),
                t.name
            ),
            "Exploit the current best before exploring further.".into(),
            ExperimentSpec::Search {
                track: t.name.clone(),
                method,
                budget_ticks: ticks,
                seed: n_exp,
                params: SearchParams::default(),
                warm_start: warm,
                physics: None,
            },
        ))
    }
}

impl Brain for ScriptedBrain {
    fn name(&self) -> String {
        "scripted".into()
    }

    fn propose(&self, s: &Session, _context: &str) -> anyhow::Result<Proposal> {
        let (hypothesis, rationale, experiment) = self.pick(s)?;
        Ok(Proposal {
            hypothesis,
            rationale,
            prediction: String::new(),
            experiment,
            value_estimate: ValueEstimate::default(),
            coding_task: None,
            critique: None,
        })
    }

    fn critique(&self, _: &Session, _: &str, _: &Proposal) -> anyhow::Result<Option<Critique>> {
        Ok(None)
    }

    fn conclude(
        &self,
        _s: &Session,
        _context: &str,
        proposal: &Proposal,
        report: &ExperimentReport,
    ) -> anyhow::Result<Conclusion> {
        let kind = match &proposal.experiment {
            ExperimentSpec::Verify { .. }
                if report.divergence_after.map(|d| d > 2.0).unwrap_or(false) =>
            {
                "sim_gap"
            }
            ExperimentSpec::Calibrate { .. } => "sim_gap",
            ExperimentSpec::Search { .. } => "result",
            _ => "result",
        };
        let status = match &proposal.experiment {
            ExperimentSpec::Search { .. } => {
                if report.result["finished"].as_bool().unwrap_or(false) {
                    "supported"
                } else {
                    "refuted"
                }
            }
            ExperimentSpec::Calibrate { .. } => {
                if report.result["improved"].as_bool().unwrap_or(false) {
                    "supported"
                } else {
                    "refuted"
                }
            }
            _ => "inconclusive",
        };
        Ok(Conclusion {
            title: crate::context::truncate(&report.summary, 80),
            kind: kind.into(),
            body: format!(
                "{}\n\nHypothesis: {} — {}",
                report.summary, proposal.hypothesis, status
            ),
            confidence: 0.6,
            hypothesis_status: status.into(),
            information_gain: if status == "refuted" { 0.6 } else { 0.3 },
            follow_ups: vec![],
            tags: vec![proposal.experiment.kind().into()],
        })
    }
}

pub fn brain_for(s: &Session) -> Box<dyn Brain> {
    if s.cfg.research.brain == "llm" && s.llm.is_configured() {
        Box::new(LlmBrain)
    } else {
        Box::new(ScriptedBrain)
    }
}
