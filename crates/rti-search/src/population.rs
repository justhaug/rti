//! Random shooting, cross-entropy method and policy-noise search.

use rand::Rng;
use rand_distr::{Distribution, Normal};
use rti_nn::policy_rollout_batch;

use crate::common::{Evaluator, SearchInput, SearchOutcome};
use crate::genome::Genome;

fn init_mean(input: &SearchInput, k: usize) -> Genome {
    input.warm_genome(k).unwrap_or_else(|| Genome::straight(k))
}

fn sample_around(mean: &Genome, std: &[f32], rng: &mut impl Rng) -> Genome {
    let mut g = Genome(
        mean.0
            .iter()
            .zip(std)
            .map(|(m, s)| {
                let n = Normal::new(0.0f32, s.max(1e-4)).unwrap();
                m + n.sample(rng)
            })
            .collect(),
    );
    g.clamp();
    g
}

pub fn random_shooting(input: &SearchInput) -> SearchOutcome {
    let k = input.control_points();
    let pop = input.params.population.unwrap_or(256);
    let sigma = input.params.sigma.unwrap_or(0.5);
    let max_ticks = input.max_ticks();
    let mut rng = input.rng();
    let mut ev = Evaluator::new(input.sim, max_ticks, input.budget);
    let mean = init_mean(input, k);
    let std = vec![sigma; 2 * k];
    if let Some(w) = input.warm_start {
        ev.eval_one(w);
    }
    let mut gens = 0u64;
    while !ev.exhausted() {
        let genomes: Vec<Genome> = (0..pop)
            .map(|_| sample_around(&mean, &std, &mut rng))
            .collect();
        ev.eval_genomes(&genomes);
        gens += 1;
    }
    ev.finish(
        "random_shooting",
        serde_json::json!({"batches": gens, "control_points": k, "sigma": sigma}),
    )
}

pub fn cem(input: &SearchInput) -> SearchOutcome {
    let k = input.control_points();
    let pop = input.params.population.unwrap_or(128);
    let elite_frac = input.params.elite_frac.unwrap_or(0.1);
    let n_elite = ((pop as f32 * elite_frac).round() as usize).clamp(2, pop);
    let sigma0 = input.params.sigma.unwrap_or(0.5);
    let max_iters = input.params.iters.unwrap_or(usize::MAX);
    let max_ticks = input.max_ticks();
    let mut rng = input.rng();
    let mut ev = Evaluator::new(input.sim, max_ticks, input.budget);
    let mut mean = init_mean(input, k);
    let mut std = vec![sigma0; 2 * k];
    let std_floor = 0.02;
    let mut gen = 0usize;
    let mut stale = 0usize;
    let mut last_best = f64::INFINITY;
    while !ev.exhausted() && gen < max_iters {
        let genomes: Vec<Genome> = (0..pop)
            .map(|_| sample_around(&mean, &std, &mut rng))
            .collect();
        let (objs, _) = ev.eval_genomes(&genomes);
        let mut idx: Vec<usize> = (0..pop).collect();
        idx.sort_by(|&a, &b| objs[a].partial_cmp(&objs[b]).unwrap());
        let elites: Vec<&Genome> = idx[..n_elite].iter().map(|&i| &genomes[i]).collect();
        // smoothed update
        let alpha = 0.7;
        for d in 0..2 * k {
            let m: f32 = elites.iter().map(|g| g.0[d]).sum::<f32>() / n_elite as f32;
            let v: f32 = elites.iter().map(|g| (g.0[d] - m).powi(2)).sum::<f32>() / n_elite as f32;
            mean.0[d] = alpha * m + (1.0 - alpha) * mean.0[d];
            std[d] = (alpha * v.sqrt() + (1.0 - alpha) * std[d]).max(std_floor);
        }
        if ev.best_objective < last_best {
            last_best = ev.best_objective;
            stale = 0;
        } else {
            stale += 1;
            if stale.is_multiple_of(8) {
                // restart exploration around the best when stuck
                for s in &mut std {
                    *s = (*s * 2.0).min(sigma0);
                }
            }
        }
        gen += 1;
    }
    ev.finish("cem", serde_json::json!({"generations": gen, "control_points": k, "population": pop, "elites": n_elite}))
}

/// Roll out a trained policy with exploration noise; keeps the best sample.
pub fn policy_search(input: &SearchInput) -> anyhow::Result<SearchOutcome> {
    let bundle = input
        .policy
        .ok_or_else(|| anyhow::anyhow!("policy search requires params.model (a policy bundle)"))?;
    let pop = input.params.population.unwrap_or(64);
    let noise = input.params.noise.unwrap_or(0.1);
    let max_ticks = input.max_ticks();
    let mut ev = Evaluator::new(input.sim, max_ticks, input.budget);
    let mut rng = input.rng();
    let mut batches = 0u64;
    // deterministic rollout first
    let det = policy_rollout_batch(input.sim, bundle, 0.0, max_ticks, &[0]);
    for (acts, ro) in &det {
        ev.ticks += ro.ticks;
        ev.evaluations += 1;
        let o = ev.objective(&ro.result);
        if o < ev.best_objective {
            ev.best_objective = o;
            ev.best_actions = acts.clone();
            ev.best_result = ro.result.clone();
        }
    }
    ev.record();
    while !ev.exhausted() {
        let seeds: Vec<u64> = (0..pop).map(|_| rng.random()).collect();
        let outs = policy_rollout_batch(input.sim, bundle, noise, max_ticks, &seeds);
        for (acts, ro) in outs {
            ev.ticks += ro.ticks;
            ev.evaluations += 1;
            let o = ev.objective(&ro.result);
            if o < ev.best_objective {
                ev.best_objective = o;
                ev.best_actions = acts;
                ev.best_result = ro.result;
            }
        }
        ev.record();
        batches += 1;
    }
    Ok(ev.finish(
        "policy",
        serde_json::json!({"batches": batches, "noise": noise}),
    ))
}
