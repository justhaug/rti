use rand::SeedableRng;
use rayon::prelude::*;
use rti_core::{Action, RunResult};
use rti_sim::{observe, Rollout, Sim};

use crate::policy::PolicyBundle;

/// Drive the sim with the policy for up to `max_ticks`, returning the
/// actions taken and a `Rollout` (with recorded states).
pub fn policy_rollout(
    sim: &Sim,
    bundle: &PolicyBundle,
    noise: f32,
    max_ticks: u32,
    seed: u64,
) -> (Vec<Action>, Rollout) {
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let init = sim.initial_state();
    let mut s = init;
    let mut actions = Vec::with_capacity(max_ticks as usize);
    let mut states = Vec::with_capacity(max_ticks as usize + 1);
    states.push(s);
    let mut max_speed = 0.0f32;
    let mut speed_sum = 0.0f64;
    let mut ticks = 0u64;
    while !sim.is_terminal(&s, max_ticks) {
        let obs = observe(&sim.geom, &s);
        let a = bundle.policy.act(&obs, noise, &mut rng);
        sim.step(&mut s, a);
        actions.push(a);
        states.push(s);
        ticks += 1;
        let sp = s.speed();
        max_speed = max_speed.max(sp);
        speed_sum += sp as f64;
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
        offtrack_ticks: s.offtrack_ticks,
        wall_hits: s.wall_hits,
    };
    (
        actions,
        Rollout {
            result,
            final_state: s,
            states,
            ticks,
        },
    )
}

/// Parallel policy rollouts, one per seed.
pub fn policy_rollout_batch(
    sim: &Sim,
    bundle: &PolicyBundle,
    noise: f32,
    max_ticks: u32,
    seeds: &[u64],
) -> Vec<(Vec<Action>, Rollout)> {
    seeds
        .par_iter()
        .map(|&seed| policy_rollout(sim, bundle, noise, max_ticks, seed))
        .collect()
}
