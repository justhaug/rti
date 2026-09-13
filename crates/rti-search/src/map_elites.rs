//! MAP-Elites quality-diversity search. Behaviour descriptors:
//! d1 = mean speed / 80, d2 = fraction of ticks braking. Each grid cell keeps
//! its best genome; the archive as a whole is the "novel behaviour" store.

use rand::seq::IndexedRandom;
use rand::Rng;
use rand_distr::{Distribution, Normal};

use crate::common::{Evaluator, SearchInput, SearchOutcome};
use crate::genome::{action_at, Genome};

#[derive(Clone)]
struct Elite {
    genome: Genome,
    objective: f64,
    time_ms: u32,
    finished: bool,
}

pub fn map_elites(input: &SearchInput) -> SearchOutcome {
    let k = input.control_points();
    let grid = input.params.grid.unwrap_or(12);
    let pop = input.params.population.unwrap_or(128);
    let sigma = input.params.sigma.unwrap_or(0.15);
    let max_ticks = input.max_ticks();
    let mut rng = input.rng();
    let mut ev = Evaluator::new(input.sim, max_ticks, input.budget);
    let mut cells: Vec<Option<Elite>> = vec![None; grid * grid];
    let warm = input.warm_genome(k);
    let track_len = input.sim.geom.total_len;
    let mut batches = 0u64;

    let descriptor = |r: &rti_core::RunResult, g: &Genome| -> usize {
        let d1 = (r.mean_speed / 80.0).clamp(0.0, 0.999);
        let n_probe = 40;
        let brake = (0..n_probe)
            .filter(|&i| action_at(g, track_len, r.progress * i as f32 / n_probe as f32).brake)
            .count() as f32
            / n_probe as f32;
        let d2 = brake.clamp(0.0, 0.999);
        (d1 * grid as f32) as usize * grid + (d2 * grid as f32) as usize
    };

    while !ev.exhausted() {
        let filled: Vec<&Elite> = cells.iter().flatten().collect();
        let genomes: Vec<Genome> = (0..pop)
            .map(|_| {
                let mut g = if filled.is_empty() || rng.random_bool(0.1) {
                    if rng.random_bool(0.5) {
                        warm.clone().unwrap_or_else(|| Genome::straight(k))
                    } else {
                        Genome::straight(k)
                    }
                } else {
                    filled.choose(&mut rng).unwrap().genome.clone()
                };
                let s = if filled.is_empty() { 0.5 } else { sigma };
                let n = Normal::new(0.0f32, s).unwrap();
                for x in &mut g.0 {
                    *x += n.sample(&mut rng);
                }
                g.clamp();
                g
            })
            .collect();
        let (objs, ros) = ev.eval_genomes(&genomes);
        for i in 0..pop {
            let c = descriptor(&ros[i].result, &genomes[i]);
            let better = cells[c]
                .as_ref()
                .map(|e| objs[i] < e.objective)
                .unwrap_or(true);
            if better {
                cells[c] = Some(Elite {
                    genome: genomes[i].clone(),
                    objective: objs[i],
                    time_ms: ros[i].result.time_ms,
                    finished: ros[i].result.finished,
                });
            }
        }
        batches += 1;
    }
    let coverage = cells.iter().filter(|c| c.is_some()).count();
    let finished_cells = cells.iter().flatten().filter(|e| e.finished).count();
    let cell_summary: Vec<serde_json::Value> = cells
        .iter()
        .enumerate()
        .filter_map(|(i, c)| c.as_ref().map(|e| serde_json::json!({"cell": i, "speed_bin": i / grid, "brake_bin": i % grid, "time_ms": e.time_ms, "finished": e.finished})))
        .collect();
    ev.finish(
        "map_elites",
        serde_json::json!({"batches": batches, "grid": grid, "coverage": coverage, "finished_cells": finished_cells, "cells": cell_summary}),
    )
}
