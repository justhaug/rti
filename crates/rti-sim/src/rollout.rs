use rayon::prelude::*;
use rti_core::{Action, CarState, RunResult, Trajectory};

use crate::physics::Sim;

/// Why a rollout stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Finished,
    /// The action sequence ran out (the sim was slower than the driver).
    InputsExhausted,
    /// Near-stationary for `STUCK_TICKS` (wall grind, wrong way, no drive).
    Stuck,
    /// Fell far below the last surface it was grounded on.
    Fell,
    /// Hit the track's tick budget.
    TickLimit,
}

/// Output of one rollout.
#[derive(Clone, Debug)]
pub struct Rollout {
    pub result: RunResult,
    pub final_state: CarState,
    pub states: Vec<CarState>,
    pub ticks: u64,
    pub stop: StopReason,
}

/// Run `actions` from `init`. Stops at finish / DNF / stuck / `max_ticks`,
/// or when the action list is exhausted. When `record` is set, every
/// post-step state is kept (savestates for search and telemetry for oracle
/// comparison).
pub fn rollout(
    sim: &Sim,
    init: &CarState,
    actions: &[Action],
    max_ticks: u32,
    record: bool,
) -> Rollout {
    rollout_with(
        sim,
        init,
        actions.len() as u32,
        max_ticks,
        record,
        |i, _| actions[i as usize],
    )
}

/// Like `rollout` but actions come from a closure of the step index and the
/// current state (closed-loop controllers, distance-indexed genomes), which
/// avoids materialising long action vectors for every candidate. Runs at
/// most `n_actions` steps.
pub fn rollout_with(
    sim: &Sim,
    init: &CarState,
    n_actions: u32,
    max_ticks: u32,
    record: bool,
    action_at: impl Fn(u32, &CarState) -> Action,
) -> Rollout {
    let mut s = *init;
    let mut states = if record {
        Vec::with_capacity((n_actions.min(max_ticks) + 1) as usize)
    } else {
        Vec::new()
    };
    if record {
        states.push(s);
    }
    let mut i = 0u32;
    let mut ticks = 0u64;
    let mut max_speed = 0.0f32;
    let mut speed_sum = 0.0f64;
    while i < n_actions && !sim.is_terminal(&s, max_ticks) {
        let a = action_at(i, &s);
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
    }
    let result = RunResult {
        finished: s.finished,
        time_ms: if s.finished {
            s.finish_tick * rti_core::TICK_MS
        } else {
            s.tick * rti_core::TICK_MS
        },
        ticks: s.tick - init.tick,
        checkpoints_hit: s.next_checkpoint,
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
    let stop = if s.finished {
        StopReason::Finished
    } else if s.stuck_ticks >= crate::physics::STUCK_TICKS {
        StopReason::Stuck
    } else if sim.geom.world.is_some()
        && s.air_ticks > 0
        && s.h < s.ground_h - crate::physics::FALL_LIMIT_M
    {
        StopReason::Fell
    } else if s.tick >= max_ticks {
        StopReason::TickLimit
    } else {
        StopReason::InputsExhausted
    };
    Rollout {
        result,
        final_state: s,
        states,
        ticks,
        stop,
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
/// Actions are truncated to the ticks actually simulated.
pub fn to_trajectory(
    sim: &Sim,
    mut actions: Vec<Action>,
    ro: &Rollout,
    method: &str,
    record_states: bool,
) -> Trajectory {
    actions.truncate(ro.ticks as usize);
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
