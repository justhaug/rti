//! Replay benchmark: replay human inputs from TMX/autosave replays in our
//! simulator on the compiled map and compare with the real times.
//! `cargo run --release -p rti-maps --example replaybench -- <maps dir> <replays dir> [--physics file.json] [--verbose]`
use rti_core::{PhysicsParams, TICK_MS};
use rti_maps::catalog::Catalog;
use rti_maps::replay::inputs_to_actions;
use rti_sim::{rollout, Sim, TrackGeom};
use std::collections::BTreeMap;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let maps_dir = &args[1];
    let replays_dir = &args[2];
    let verbose = args.iter().any(|a| a == "--verbose");
    let params = match args.iter().position(|a| a == "--physics") {
        Some(i) => serde_json::from_str(&std::fs::read_to_string(&args[i + 1])?)?,
        None => PhysicsParams::default(),
    };
    let cat = Catalog::default_catalog();
    let mut rows = vec![];
    let mut files: Vec<_> = std::fs::read_dir(replays_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".Replay.Gbx"))
        .collect();
    files.sort();
    for rf in files {
        let id = rf
            .file_name()
            .unwrap()
            .to_string_lossy()
            .trim_end_matches(".Replay.Gbx")
            .to_string();
        let replay = match rti_maps::parse_replay(&std::fs::read(&rf)?) {
            Ok(r) => r,
            Err(e) => {
                if verbose {
                    eprintln!("{id}: replay parse failed: {e}");
                }
                continue;
            }
        };
        let Some(ghost) = replay.ghosts.first() else {
            continue;
        };
        if ghost.inputs.is_empty() || ghost.race_time_ms == 0 {
            continue;
        }
        // prefer the map embedded in the replay (same version the run was driven on)
        let map = match &replay.map {
            Some(m) => m.clone(),
            None => {
                let mf = std::path::Path::new(maps_dir).join(format!("{id}.Map.Gbx"));
                match std::fs::read(&mf)
                    .ok()
                    .and_then(|d| rti_maps::parse_map(&d).ok())
                {
                    Some(m) => m,
                    None => continue,
                }
            }
        };
        let (track, report, world) = match rti_maps::compile::compile_track3(&map, &cat, &id) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let flat = args.iter().any(|a| a == "--flat");
        let geom = if flat {
            TrackGeom::new(track.clone())
        } else {
            TrackGeom::new(track.clone()).with_world(world)
        };
        let sim = Sim::new(params.clone(), geom);
        let acts = inputs_to_actions(&ghost.inputs, ghost.ticks + 500);
        let ro = rollout(&sim, &sim.initial_state(), &acts, ghost.ticks + 500, true);
        if let Some(i) = args.iter().position(|a| a == "--trace") {
            if args[i + 1] == id {
                println!(
                    "trace {id}: start heading {:.2}, first nodes {:?}",
                    sim.initial_state().heading,
                    &track.nodes[..3.min(track.nodes.len())]
                );
                for st in ro.states.iter().step_by(10).take(60) {
                    let loc = sim.geom.locate(st.x, st.y, st.seg as usize);
                    let a = acts[(st.tick as usize).min(acts.len() - 1)];
                    println!("  t={:4} prog {:6.1} lat {:6.2}/{:.1} v {:5.1} h {:6.1} air {:3} hdg {:5.2} dir {:5.2} walls {:3} in: steer {:+.2} gas {} brake {}", st.tick, st.progress, loc.lateral, loc.half_width, st.speed(), st.h, st.air_ticks, st.heading, loc.dir, st.wall_hits, a.steer, a.gas as u8, a.brake as u8);
                }
            }
        }
        // split comparison: sim tick at which each track checkpoint distance is reached vs real cp times
        let geom = &sim.geom;
        let mut split_errs = vec![];
        for (k, &cpd) in geom.checkpoint_dist.iter().enumerate() {
            if let (Some(real), Some(st)) = (
                ghost.checkpoint_times_ms.get(k),
                ro.states.iter().find(|s| s.progress >= cpd),
            ) {
                split_errs.push((st.tick * TICK_MS) as i64 - *real as i64);
            }
        }
        let real = ghost.race_time_ms;
        let simt = ro.result.time_ms;
        let err = if ro.result.finished {
            Some(simt as i64 - real as i64)
        } else {
            None
        };
        rows.push((
            id.clone(),
            map.info.name.clone(),
            report.finish_found,
            report.blocks_chained,
            track.length(),
            real,
            ro.result.finished,
            simt,
            ro.result.progress,
            ro.result.wall_hits,
            split_errs,
            err,
        ));
    }
    rows.sort_by(|a, b| b.2.cmp(&a.2).then(b.3.cmp(&a.3)));
    let mut finished = 0;
    let mut within10 = 0;
    let mut compiled_full = 0;
    let mut abs_errs = vec![];
    println!(
        "{:<8} {:<28} {:>5} {:>6} {:>7} {:>8} {:>8} {:>8} {:>6} {}",
        "map",
        "name",
        "chain",
        "finish",
        "len m",
        "real ms",
        "sim",
        "prog m",
        "walls",
        "split errs ms"
    );
    for r in &rows {
        if r.2 {
            compiled_full += 1;
        }
        if r.6 {
            finished += 1;
            let e = r.11.unwrap();
            abs_errs.push(e.abs());
            if (e.abs() as f64) < r.5 as f64 * 0.10 {
                within10 += 1;
            }
        }
        println!(
            "{:<8} {:<28} {:>5} {:>6} {:>7.0} {:>8} {:>8} {:>8.0} {:>6} {:?}",
            r.0,
            r.1.chars().take(28).collect::<String>(),
            r.3,
            if r.2 { "yes" } else { "no" },
            r.4,
            r.5,
            if r.6 { r.7.to_string() } else { "DNF".into() },
            r.8,
            r.9,
            r.10
        );
    }
    abs_errs.sort();
    let med = abs_errs.get(abs_errs.len() / 2).copied().unwrap_or(0);
    println!("\nreplays {}: maps compiled start-to-finish {}, sim finished {} (within 10% of real time: {}), median |error| of finished {} ms", rows.len(), compiled_full, finished, within10, med);
    let mut dnf_progress: BTreeMap<u32, usize> = BTreeMap::new();
    for r in rows.iter().filter(|r| !r.6) {
        *dnf_progress
            .entry(((r.8 / r.4.max(1.0)) * 10.0) as u32 * 10)
            .or_default() += 1;
    }
    println!("DNF progress histogram (% of track): {:?}", dnf_progress);
    Ok(())
}
