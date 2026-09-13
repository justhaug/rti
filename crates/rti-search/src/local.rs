//! Local trajectory optimisation: hill-climb on a per-tick action sequence
//! with windowed perturbations, batched proposals, accept-if-better.

use rand::Rng;
use rti_core::Action;

use crate::common::{Evaluator, SearchInput, SearchOutcome};
use crate::population;

fn perturb(base: &[Action], rng: &mut impl Rng, scale: f32) -> Vec<Action> {
    let n = base.len();
    let mut out = base.to_vec();
    if n == 0 {
        return out;
    }
    let win = rng.random_range(1..=(n / 8).max(2)).min(n);
    let start = rng.random_range(0..n - win + 1);
    match rng.random_range(0..6) {
        0 | 1 => {
            // steer offset over window
            let d: f32 = rng.random_range(-scale..scale);
            for a in &mut out[start..start + win] {
                a.steer = (a.steer + d).clamp(-1.0, 1.0);
            }
        }
        2 => {
            // brake pulse
            let b = rng.random_bool(0.5);
            for a in &mut out[start..start + win] {
                a.brake = b;
                if b {
                    a.gas = false;
                }
            }
        }
        3 => {
            // gas toggle
            let g = rng.random_bool(0.8);
            for a in &mut out[start..start + win] {
                a.gas = g;
                if g {
                    a.brake = false;
                }
            }
        }
        4 => {
            // time-shift: move the window earlier/later by a few ticks
            let shift = rng.random_range(1..=5usize).min(start.max(1));
            if start >= shift {
                let seg: Vec<Action> = out[start..start + win].to_vec();
                out.copy_within(start - shift..start, start - shift + win);
                out[start - shift..start - shift + win].copy_from_slice(&seg);
            }
        }
        _ => {
            // smooth steer ramp
            let d: f32 = rng.random_range(-scale..scale);
            for (i, a) in out[start..start + win].iter_mut().enumerate() {
                let f = i as f32 / win as f32;
                a.steer = (a.steer + d * f).clamp(-1.0, 1.0);
            }
        }
    }
    out
}

pub fn local_opt(input: &SearchInput) -> anyhow::Result<SearchOutcome> {
    let max_ticks = input.max_ticks();
    let pop = input.params.population.unwrap_or(64);
    let max_iters = input.params.iters.unwrap_or(usize::MAX);
    let mut rng = input.rng();

    // Obtain a starting trajectory: warm start, or a short random shooting phase.
    let mut base: Vec<Action> = match input.warm_start {
        Some(a) if !a.is_empty() => a.to_vec(),
        _ => {
            let sub = SearchInput {
                sim: input.sim,
                params: input.params,
                budget: crate::common::Budget {
                    max_ticks: input.budget.max_ticks / 5,
                    max_wall_ms: input.budget.max_wall_ms,
                },
                seed: input.seed ^ 0x9e37_79b9,
                warm_start: None,
                policy: None,
            };
            population::random_shooting(&sub).best_actions
        }
    };
    if base.len() < max_ticks as usize {
        let hold = base.last().copied().unwrap_or(Action::full_gas());
        base.resize(max_ticks as usize, hold);
    }
    let mut ev = Evaluator::new(input.sim, max_ticks, input.budget);
    let (mut best_obj, _) = ev.eval_one(&base);
    let mut scale = 0.5f32;
    let mut iters = 0usize;
    let mut accepted = 0usize;
    while !ev.exhausted() && iters < max_iters {
        let props: Vec<Vec<Action>> = (0..pop).map(|_| perturb(&base, &mut rng, scale)).collect();
        let (objs, _) = ev.eval_batch(&props);
        let (bi, bo) =
            objs.iter().enumerate().fold(
                (0, f64::INFINITY),
                |acc, (i, &o)| if o < acc.1 { (i, o) } else { acc },
            );
        if bo < best_obj {
            best_obj = bo;
            base = props[bi].clone();
            accepted += 1;
            scale = (scale * 1.1).min(1.0);
        } else {
            scale = (scale * 0.9).max(0.05);
        }
        iters += 1;
    }
    Ok(ev.finish(
        "local_opt",
        serde_json::json!({"iters": iters, "accepted": accepted, "population": pop}),
    ))
}
