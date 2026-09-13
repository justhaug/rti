//! Fit PhysicsParams to human replays: CEM over the parameter vector,
//! minimising finish-time / checkpoint-split errors of the human inputs
//! replayed in our simulator on the compiled maps.
//! `cargo run --release -p rti-maps --example replayfit -- <replays dir> <out.json> [gens]`
use rayon::prelude::*;
use rti_core::{Action, PhysicsParams, Track, TICK_MS};
use rti_maps::catalog::Catalog;
use rti_maps::replay::inputs_to_actions;
use rti_sim::{rollout, Sim, TrackGeom};

struct Case {
    id: String,
    track: Track,
    world: std::sync::Arc<rti_sim::World>,
    actions: Vec<Action>,
    real_ms: u32,
    cps_ms: Vec<u32>,
    full: bool,
}

fn load(dir: &str) -> anyhow::Result<Vec<Case>> {
    let cat = Catalog::default_catalog();
    let mut out = vec![];
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".Replay.Gbx"))
        .collect();
    files.sort();
    for rf in files {
        let Ok(r) = rti_maps::parse_replay(&std::fs::read(&rf)?) else {
            continue;
        };
        let Some(g) = r.ghosts.first() else { continue };
        let Some(m) = &r.map else { continue };
        if g.inputs.is_empty() || g.race_time_ms == 0 {
            continue;
        }
        let Ok((track, rep, world)) = rti_maps::compile::compile_track3(m, &cat, "x") else {
            continue;
        };
        let full = rep.finish_found && rep.start_found;
        let avg_speed = track.length() / (g.race_time_ms as f32 / 1000.0);
        // only geometry we trust: start-to-finish chains with a plausible average speed
        if !full || !(8.0..=110.0).contains(&avg_speed) {
            continue;
        }
        out.push(Case {
            world: std::sync::Arc::new(world),
            id: rf
                .file_name()
                .unwrap()
                .to_string_lossy()
                .trim_end_matches(".Replay.Gbx")
                .to_string(),
            track,
            actions: inputs_to_actions(&g.inputs, g.ticks + 300),
            real_ms: g.race_time_ms,
            cps_ms: g.checkpoint_times_ms.clone(),
            full,
        });
    }
    Ok(out)
}

/// Loss for one case (ms-equivalent). DNF: big penalty scaled by remaining track.
fn case_loss(c: &Case, params: &PhysicsParams) -> (f64, bool, u32, f32) {
    let mut geom = TrackGeom::new(c.track.clone());
    geom.world = Some(c.world.clone());
    let sim = Sim::new(params.clone(), geom);
    let ro = rollout(
        &sim,
        &sim.initial_state(),
        &c.actions,
        c.actions.len() as u32,
        true,
    );
    let len = sim.geom.total_len;
    if !ro.result.finished {
        return (
            20_000.0 + (len - ro.result.progress).max(0.0) as f64 * 20.0,
            false,
            ro.result.time_ms,
            ro.result.progress,
        );
    }
    let mut l = (ro.result.time_ms as f64 - c.real_ms as f64).abs();
    for (k, &d) in sim.geom.checkpoint_dist.iter().enumerate() {
        if let (Some(&real), Some(st)) =
            (c.cps_ms.get(k), ro.states.iter().find(|s| s.progress >= d))
        {
            l += ((st.tick * TICK_MS) as f64 - real as f64).abs() * 0.5;
        }
    }
    (l, true, ro.result.time_ms, ro.result.progress)
}

fn total_loss(cases: &[Case], params: &PhysicsParams) -> f64 {
    cases
        .par_iter()
        .map(|c| case_loss(c, params).0)
        .sum::<f64>()
        / cases.len().max(1) as f64
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cases = load(&args[1])?;
    let out = &args[2];
    let gens: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(40);
    eprintln!(
        "{} cases ({} compiled start-to-finish)",
        cases.len(),
        cases.iter().filter(|c| c.full).count()
    );
    let base = PhysicsParams::default();
    let names = PhysicsParams::names();
    let b = base.to_vec();
    let dim = b.len();
    let mut mean = vec![0.0f32; dim];
    let mut std = vec![0.5f32; dim];
    // downforce starts at zero in the defaults; search it additively (see below)
    let df_idx = names.iter().position(|n| *n == "downforce").unwrap();
    let mut best = (total_loss(&cases, &base), base.clone());
    eprintln!("baseline loss {:.0}", best.0);
    let mut rng_state = 0x9E3779B97F4A7C15u64;
    let mut rnd = || {
        rng_state ^= rng_state << 13;
        rng_state ^= rng_state >> 7;
        rng_state ^= rng_state << 17;
        (rng_state >> 11) as f64 / (1u64 << 53) as f64
    };
    let pop = 48;
    let elite = 8;
    for gen in 0..gens {
        let cands: Vec<Vec<f32>> = (0..pop)
            .map(|_| {
                (0..dim)
                    .map(|d| {
                        // box-muller
                        let u1 = rnd().max(1e-12);
                        let u2 = rnd();
                        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                        mean[d] + std[d] * z as f32
                    })
                    .collect()
            })
            .collect();
        let scored: Vec<(f64, usize)> = cands
            .par_iter()
            .enumerate()
            .map(|(i, c)| {
                let mut v: Vec<f32> = b
                    .iter()
                    .zip(c)
                    .map(|(bb, m)| bb * m.clamp(-2.5, 2.5).exp())
                    .collect();
                v[df_idx] = (c[df_idx].clamp(-2.5, 2.5) * 0.01).max(0.0);
                let p = PhysicsParams::from_vec(&v).unwrap();
                (total_loss(&cases, &p), i)
            })
            .collect();
        let mut sc = scored.clone();
        sc.sort_by(|a, c| a.0.partial_cmp(&c.0).unwrap());
        for &(l, i) in &sc[..elite] {
            if l < best.0 {
                let mut v: Vec<f32> = b
                    .iter()
                    .zip(&cands[i])
                    .map(|(bb, m)| bb * m.clamp(-2.5, 2.5).exp())
                    .collect();
                v[df_idx] = (cands[i][df_idx].clamp(-2.5, 2.5) * 0.01).max(0.0);
                best = (l, PhysicsParams::from_vec(&v).unwrap());
            }
        }
        for d in 0..dim {
            let m: f32 = sc[..elite].iter().map(|(_, i)| cands[*i][d]).sum::<f32>() / elite as f32;
            let v: f32 = sc[..elite]
                .iter()
                .map(|(_, i)| (cands[*i][d] - m).powi(2))
                .sum::<f32>()
                / elite as f32;
            mean[d] = 0.7 * m + 0.3 * mean[d];
            std[d] = (0.7 * v.sqrt() + 0.3 * std[d]).max(0.02);
        }
        eprintln!(
            "gen {gen}: best {:.0} (elite mean {:.0})",
            best.0,
            sc[..elite].iter().map(|x| x.0).sum::<f64>() / elite as f64
        );
    }
    std::fs::write(out, serde_json::to_string_pretty(&best.1)?)?;
    println!("best loss {:.0}; params written to {out}", best.0);
    let bv = best.1.to_vec();
    for (n, (a, c)) in names.iter().zip(b.iter().zip(bv.iter())) {
        println!(
            "  {:<24} {:>10.4} -> {:>10.4} ({:+.0}%)",
            n,
            a,
            c,
            (c / a - 1.0) * 100.0
        );
    }
    println!("\nper case with fitted params:");
    for c in &cases {
        let (l, fin, t, prog) = case_loss(c, &best.1);
        println!(
            "  {:<8} full={} real {:>6} ms sim {:>6} {} prog {:>5.0}/{:.0} loss {:.0}",
            c.id,
            c.full as u8,
            c.real_ms,
            t,
            if fin { "FIN" } else { "DNF" },
            prog,
            c.track.length(),
            l
        );
    }
    Ok(())
}
