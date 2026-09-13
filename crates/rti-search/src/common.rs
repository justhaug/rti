use rand::SeedableRng;
use rayon::prelude::*;
use rti_core::experiment::{SearchMethod, SearchParams};
use rti_core::{Action, RunResult};
use rti_nn::PolicyBundle;
use rti_sim::{rollout, rollout_batch, rollout_with, Rollout, Sim};

use crate::genome::{action_at, decode, Genome};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Stop conditions for a search.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub max_ticks: u64,
    pub max_wall_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryPoint {
    pub ticks: u64,
    pub evaluations: u64,
    pub best_objective: f64,
    pub best_time_ms: Option<u32>,
    pub best_progress: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchOutcome {
    pub method: String,
    pub best_actions: Vec<Action>,
    pub best_result: RunResult,
    pub best_objective: f64,
    pub evaluations: u64,
    pub ticks: u64,
    pub wall_ms: u64,
    pub history: Vec<HistoryPoint>,
    /// Method-specific extras (archive coverage, generations, ...).
    pub extra: serde_json::Value,
}

/// Shared evaluation bookkeeping: tracks ticks, evaluations, best-so-far
/// and the improvement history.
pub struct Evaluator<'a> {
    pub sim: &'a Sim,
    pub max_ticks_per_run: u32,
    pub budget: Budget,
    pub start: Instant,
    pub ticks: u64,
    pub evaluations: u64,
    pub best_objective: f64,
    pub best_actions: Vec<Action>,
    pub best_result: RunResult,
    pub history: Vec<HistoryPoint>,
    track_len: f32,
}

impl<'a> Evaluator<'a> {
    pub fn new(sim: &'a Sim, max_ticks_per_run: u32, budget: Budget) -> Self {
        Evaluator {
            sim,
            max_ticks_per_run,
            budget,
            start: Instant::now(),
            ticks: 0,
            evaluations: 0,
            best_objective: f64::INFINITY,
            best_actions: Vec::new(),
            best_result: RunResult::default(),
            history: Vec::new(),
            track_len: sim.geom.total_len,
        }
    }

    pub fn objective(&self, r: &RunResult) -> f64 {
        r.objective(self.track_len)
    }

    pub fn exhausted(&self) -> bool {
        if self.ticks >= self.budget.max_ticks {
            return true;
        }
        if let Some(ms) = self.budget.max_wall_ms {
            if self.start.elapsed().as_millis() as u64 >= ms {
                return true;
            }
        }
        false
    }

    /// Evaluate a batch of action sequences; returns objectives (lower is better).
    pub fn eval_batch(&mut self, seqs: &[Vec<Action>]) -> (Vec<f64>, Vec<Rollout>) {
        let (ros, ticks) = rollout_batch(
            self.sim,
            &self.sim.initial_state(),
            seqs,
            self.max_ticks_per_run,
        );
        self.ticks += ticks;
        self.evaluations += seqs.len() as u64;
        let mut objs = Vec::with_capacity(ros.len());
        for (i, ro) in ros.iter().enumerate() {
            let o = self.objective(&ro.result);
            objs.push(o);
            if o < self.best_objective {
                self.best_objective = o;
                self.best_actions = seqs[i].clone();
                self.best_result = ro.result.clone();
            }
        }
        self.record();
        (objs, ros)
    }

    /// Evaluate genomes without materialising action vectors; only the best
    /// genome is decoded (and truncated to the ticks actually used).
    pub fn eval_genomes(&mut self, genomes: &[Genome]) -> (Vec<f64>, Vec<Rollout>) {
        let init = self.sim.initial_state();
        let n = self.max_ticks_per_run;
        let sim = self.sim;
        let len = sim.geom.total_len;
        let ros: Vec<Rollout> = genomes
            .par_iter()
            .map(|g| {
                rollout_with(sim, &init, n, n, false, |_, s| {
                    action_at(g, len, s.progress)
                })
            })
            .collect();
        self.ticks += ros.iter().map(|r| r.ticks).sum::<u64>();
        self.evaluations += genomes.len() as u64;
        let mut objs = Vec::with_capacity(ros.len());
        let mut best_i = None;
        for (i, ro) in ros.iter().enumerate() {
            let o = self.objective(&ro.result);
            objs.push(o);
            if o < self.best_objective {
                self.best_objective = o;
                self.best_result = ro.result.clone();
                best_i = Some(i);
            }
        }
        if let Some(i) = best_i {
            let (acts, ro) = decode(&genomes[i], sim, n);
            self.ticks += ro.ticks;
            self.best_actions = acts;
        }
        self.record();
        (objs, ros)
    }

    pub fn eval_one(&mut self, seq: &[Action]) -> (f64, Rollout) {
        let ro = rollout(
            self.sim,
            &self.sim.initial_state(),
            seq,
            self.max_ticks_per_run,
            false,
        );
        self.ticks += ro.ticks;
        self.evaluations += 1;
        let o = self.objective(&ro.result);
        if o < self.best_objective {
            self.best_objective = o;
            self.best_actions = seq.to_vec();
            self.best_result = ro.result.clone();
        }
        (o, ro)
    }

    pub fn record(&mut self) {
        let improved = self
            .history
            .last()
            .map(|h| h.best_objective > self.best_objective)
            .unwrap_or(true);
        let sparse = self
            .history
            .last()
            .map(|h| self.ticks - h.ticks > self.budget.max_ticks / 50)
            .unwrap_or(true);
        if improved || sparse {
            self.history.push(HistoryPoint {
                ticks: self.ticks,
                evaluations: self.evaluations,
                best_objective: self.best_objective,
                best_time_ms: if self.best_result.finished {
                    Some(self.best_result.time_ms)
                } else {
                    None
                },
                best_progress: self.best_result.progress,
            });
        }
    }

    pub fn finish(mut self, method: &str, extra: serde_json::Value) -> SearchOutcome {
        self.record();
        SearchOutcome {
            method: method.to_string(),
            best_actions: self.best_actions,
            best_result: self.best_result,
            best_objective: self.best_objective,
            evaluations: self.evaluations,
            ticks: self.ticks,
            wall_ms: self.start.elapsed().as_millis() as u64,
            history: self.history,
            extra,
        }
    }
}

/// Inputs every method receives.
pub struct SearchInput<'a> {
    pub sim: &'a Sim,
    pub params: &'a SearchParams,
    pub budget: Budget,
    pub seed: u64,
    pub warm_start: Option<&'a [Action]>,
    pub policy: Option<&'a PolicyBundle>,
}

impl<'a> SearchInput<'a> {
    pub fn max_ticks(&self) -> u32 {
        self.params
            .max_ticks
            .unwrap_or(self.sim.geom.track.max_ticks)
    }
    pub fn rng(&self) -> rand::rngs::StdRng {
        rand::rngs::StdRng::seed_from_u64(self.seed)
    }
    /// Default number of control points: about one per 8 m of track, bounded.
    pub fn control_points(&self) -> usize {
        self.params
            .control_points
            .unwrap_or_else(|| ((self.sim.geom.total_len / 8.0) as usize).clamp(8, 160))
    }
    /// Warm-start genome from the input trajectory, if any.
    pub fn warm_genome(&self, k: usize) -> Option<Genome> {
        match self.warm_start {
            Some(a) if !a.is_empty() => Some(crate::genome::encode(a, self.sim, k)),
            _ => None,
        }
    }
}

/// Dispatch a search method.
pub fn run(method: SearchMethod, input: &SearchInput) -> anyhow::Result<SearchOutcome> {
    match method {
        SearchMethod::RandomShooting => Ok(population::random_shooting(input)),
        SearchMethod::Cem => Ok(population::cem(input)),
        SearchMethod::CmaEs => Ok(cma::cma_es(input)),
        SearchMethod::Beam => Ok(beam::beam(input)),
        SearchMethod::LocalOpt => local::local_opt(input),
        SearchMethod::MapElites => Ok(map_elites::map_elites(input)),
        SearchMethod::Policy => population::policy_search(input),
    }
}

use crate::{beam, cma, local, map_elites, population};
