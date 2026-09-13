use rayon::prelude::*;
use rti_core::{Action, CarState, RunResult, Trajectory};

use crate::physics::Sim;

/// Output of one rollout.
#[derive(Clone, Debug)]
pub struct Rollout {
    pub result: RunResult,
    pub final_state: CarState,
    pub states: Vec<CarState>,
    pub ticks: u64,
}

/// Run `actions` from `init`. Stops at finish / DNF / `max_ticks`. If the
/// action list is shorter than the run, the last action is held. When
/// `record` is set, every post-step state is kept (savestates for search
/// and telemetry for oracle comparison).
pub fn rollout(
    sim: &Sim,
    init: &CarState,
    actions: &[Action],
    max_ticks: u32,
    record: bool,
) -> Rollout {
    let mut s = *init;
    let mut states = if record {
        Vec::with_capacity(actions.len().min(max_ticks as usize) + 1)
    } else {
        Vec::new()
    };
    if record {
        states.push(s);
    }
    let hold = actions.last().copied().unwrap_or(Action::coast());
    let mut i = 0usize;
    let mut ticks = 0u64;
    let mut max_speed = 0.0f32;
    let mut speed_sum = 0.0f64;
    let mut cp_hit = 0u32;
    while !sim.is_terminal(&s, max_ticks) {
        let a = if i < actions.len() { actions[i] } else { hold };
        sim.step(&mut s, a);
        i += 1;
        ticks += 1;
        let sp = s.speed();
        if sp > max_speed {
            max_speed = sp;
        }
        speed_sum += sp as f64;
        if record {
            states.push(s);
        }
        if actions.is_empty() && i > max_ticks as usize {
            break;
        }
    }
    cp_hit = cp_hit.max(s.next_checkpoint);
    let result = RunResult {
        finished: s.finished,
        time_ms: if s.finished {
            s.finish_tick * rti_core::TICK_MS
        } else {
            s.tick * rti_core::TICK_MS
        },
        ticks: s.tick - init.tick,
        checkpoints_hit: cp_hit,
        progress: s.progress,
        max_speed,
        mean_speed: if ticks > 0 {
            (speed_sum / ticks as f64) as f32
        } else {
            0.0
        },
        offtrack_ticks: s.offtrack_ticks - init.offtrack_ticks,
        wall_hits: s.wall_hits - init.wall_hits,
    };
    Rollout {
        result,
        final_state: s,
        states,
        ticks,
    }
}

/// Evaluate many action sequences in parallel from the same initial state.
/// Returns results and the total number of simulated ticks.
pub fn rollout_batch(
    sim: &Sim,
    init: &CarState,
    seqs: &[Vec<Action>],
    max_ticks: u32,
) -> (Vec<Rollout>, u64) {
    let outs: Vec<Rollout> = seqs
        .par_iter()
        .map(|a| rollout(sim, init, a, max_ticks, false))
        .collect();
    let ticks = outs.iter().map(|r| r.ticks).sum();
    (outs, ticks)
}

/// Build a `Trajectory` record from a rollout of `actions` on this sim.
pub fn to_trajectory(
    sim: &Sim,
    actions: Vec<Action>,
    ro: &Rollout,
    method: &str,
    record_states: bool,
) -> Trajectory {
    Trajectory {
        track_hash: sim.geom.track.hash(),
        track_name: sim.geom.track.name.clone(),
        world: format!("sim:{}", sim.params.hash().short()),
        actions,
        states: if record_states {
            ro.states.clone()
        } else {
            Vec::new()
        },
        result: ro.result.clone(),
        method: method.to_string(),
        parent: None,
    }
}
