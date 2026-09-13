//! Beam search over discrete macro-actions from savestates. The heuristic
//! is track progress plus a speed bonus, or a value net's predicted
//! remaining time when a bundle with a value head is supplied.

use rayon::prelude::*;
use rti_core::{Action, CarState, RunResult};
use rti_sim::{observe, rollout};

use crate::common::{Evaluator, SearchInput, SearchOutcome};

#[derive(Clone)]
struct Node {
    state: CarState,
    actions: Vec<Action>,
    score: f64,
}

pub fn beam(input: &SearchInput) -> SearchOutcome {
    let width = input.params.beam_width.unwrap_or(64);
    let macro_ticks = input.params.macro_ticks.unwrap_or(20);
    let steer_levels = input.params.steer_levels.unwrap_or(5);
    let max_ticks = input.max_ticks();
    let sim = input.sim;
    let value = input.policy.and_then(|b| b.value.as_ref());
    let action_set = Action::discrete_set(steer_levels);
    let mut ev = Evaluator::new(sim, max_ticks, input.budget);
    let track_len = sim.geom.total_len as f64;

    let mut beam: Vec<Node> = vec![Node {
        state: sim.initial_state(),
        actions: Vec::new(),
        score: 0.0,
    }];
    let mut best_finished: Option<(RunResult, Vec<Action>)> = None;
    let mut depth = 0u64;

    while !beam.is_empty() && !ev.exhausted() {
        // expand
        let expanded: Vec<(Node, RunResult, u64)> = beam
            .par_iter()
            .flat_map_iter(|node| {
                action_set.iter().map(move |&a| {
                    let seq = vec![a; macro_ticks as usize];
                    let ro = rollout(sim, &node.state, &seq, max_ticks, false);
                    let mut actions = node.actions.clone();
                    actions.extend(seq.iter().take(ro.ticks as usize));
                    let s = ro.final_state;
                    let score = if let Some(v) = value {
                        let obs = observe(&sim.geom, &s);
                        -(v.net.forward(&obs)[0] as f64) * 6000.0 - s.tick as f64
                    } else {
                        // progress + speed, penalised when faster than the speed
                        // from which the upcoming corners can still be braked for
                        let loc = sim.geom.locate(s.x, s.y, s.seg as usize);
                        let g = sim.params.surface_grip[loc.surface.index()];
                        let vsafe = sim.geom.safe_speed(
                            s.progress,
                            150.0,
                            sim.params.grip * g * 0.85,
                            sim.params.brake_decel * g * 0.8,
                        );
                        let over = (s.forward_speed() - vsafe).max(0.0) as f64;
                        let stuck_penalty = if s.speed() < 2.0 && s.tick > 100 {
                            -50.0
                        } else {
                            0.0
                        };
                        s.progress as f64 + 0.4 * s.forward_speed() as f64
                            - 0.05 * s.tick as f64
                            - 8.0 * (s.wall_hits - node.state.wall_hits) as f64
                            - 1.5 * over * over
                            + stuck_penalty
                    };
                    (
                        Node {
                            state: s,
                            actions,
                            score,
                        },
                        ro.result,
                        ro.ticks,
                    )
                })
            })
            .collect();
        for (_, _, t) in &expanded {
            ev.ticks += *t;
        }
        ev.evaluations += expanded.len() as u64;

        let mut next: Vec<Node> = Vec::with_capacity(expanded.len());
        for (node, _r, _) in expanded {
            if node.state.finished {
                let full = rollout(sim, &sim.initial_state(), &node.actions, max_ticks, false);
                ev.ticks += full.ticks;
                let o = full.result.objective(track_len as f32);
                if o < ev.best_objective {
                    ev.best_objective = o;
                    ev.best_actions = node.actions.clone();
                    ev.best_result = full.result.clone();
                }
                if best_finished
                    .as_ref()
                    .map(|(r, _)| full.result.time_ms < r.time_ms)
                    .unwrap_or(true)
                {
                    best_finished = Some((full.result, node.actions));
                }
            } else if node.state.tick < max_ticks
                && node.state.stuck_ticks < rti_sim::physics::STUCK_TICKS
                && _r.ticks > 0
            {
                next.push(node);
            }
        }
        // prune: if we already have a finish, drop nodes slower than it
        if let Some((r, _)) = &best_finished {
            next.retain(|n| n.state.tick < r.time_ms / rti_core::TICK_MS);
        }
        next.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        // diversity: cap near-duplicate states (same rounded position bucket)
        let mut kept: Vec<Node> = Vec::with_capacity(width);
        let mut buckets: std::collections::HashSet<(i32, i32, i32)> = Default::default();
        for n in next {
            let key = (
                (n.state.x / 1.5) as i32,
                (n.state.y / 1.5) as i32,
                (n.state.forward_speed() / 3.0) as i32,
            );
            if buckets.insert(key) || kept.len() < width / 4 {
                kept.push(n);
            }
            if kept.len() >= width {
                break;
            }
        }
        beam = kept;
        // record progress of leading node as "best" for unfinished runs
        if best_finished.is_none() {
            if let Some(lead) = beam.first() {
                let partial = RunResult {
                    finished: false,
                    time_ms: lead.state.tick * rti_core::TICK_MS,
                    ticks: lead.state.tick,
                    checkpoints_hit: lead.state.next_checkpoint,
                    progress: lead.state.progress,
                    max_speed: 0.0,
                    mean_speed: 0.0,
                    offtrack_ticks: lead.state.offtrack_ticks,
                    wall_hits: lead.state.wall_hits,
                };
                let o = partial.objective(track_len as f32);
                if o < ev.best_objective {
                    ev.best_objective = o;
                    ev.best_actions = lead.actions.clone();
                    ev.best_result = partial;
                }
            }
        }
        ev.record();
        depth += 1;
    }
    ev.finish(
        "beam",
        serde_json::json!({"depth": depth, "beam_width": width, "macro_ticks": macro_ticks, "actions": action_set.len(), "value_guided": value.is_some()}),
    )
}
