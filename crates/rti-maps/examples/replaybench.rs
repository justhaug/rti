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
        let (track, report, world, markers) =
            match rti_maps::compile::compile_track3_full(&map, &cat, &id) {
                Ok(x) => x,
                Err(_) => continue,
            };
        let markers_dbg = markers.clone();
        let flat = args.iter().any(|a| a == "--flat");
        let geom = if flat {
            TrackGeom::new(track.clone())
        } else {
            TrackGeom::new(track.clone())
                .with_world(world)
                .with_markers(markers)
        };
        let sim = Sim::new(params.clone(), geom);
        let acts = inputs_to_actions(&ghost.inputs, ghost.ticks + 500);
        let ro = rollout(&sim, &sim.initial_state(), &acts, ghost.ticks + 500, true);
        if let Some(i) = args.iter().position(|a| a == "--trace-end") {
            if args[i + 1] == id {
                let n = ro.states.len();
                println!("trace-end {id}: finish node {:?} finish_dist {:.1} total {:.1} checkpoints {:?}", track.finish, sim.geom.finish_dist, sim.geom.total_len, sim.geom.checkpoint_dist);
                for st in ro.states.iter().skip(n.saturating_sub(300)).step_by(15) {
                    let loc = sim.geom.locate(st.x, st.y, st.seg as usize);
                    let layers: Vec<String> = sim
                        .geom
                        .world
                        .as_ref()
                        .map(|w| {
                            (0..12)
                                .filter_map(|k| {
                                    w.layer_at(st.x, st.y, st.h - 60.0 + k as f32 * 10.0, 5.0)
                                })
                                .map(|l| format!("{:.0}", l.y))
                                .collect::<std::collections::BTreeSet<_>>()
                                .into_iter()
                                .collect()
                        })
                        .unwrap_or_default();
                    println!("  t={:4} xz ({:6.0},{:6.0}) cell ({:3.0},{:3.0}) prog {:7.1} lat {:6.1} v {:5.1} h {:6.1} air {:3} cp {} walls {:3} layers {:?}", st.tick, st.x, st.y, (st.x / 32.0).floor(), (st.y / 32.0).floor(), st.progress, loc.lateral, st.speed(), st.h, st.air_ticks, st.next_checkpoint, st.wall_hits, layers);
                }
            }
        }
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
            ro.stop,
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
    // Honest accuracy metric: a compiled route is only usable when its length
    // is plausible for the real time (5-70 m/s average). Short stubs that the
    // car trivially "finishes" do not count.
    let usable: Vec<_> = rows
        .iter()
        .filter(|r| {
            let v = r.4 / (r.5 as f32 / 1000.0);
            (5.0..=70.0).contains(&v)
        })
        .collect();
    let u_fin = usable.iter().filter(|r| r.6).count();
    let u_10 = usable
        .iter()
        .filter(|r| {
            r.11.map(|e| (e.abs() as f64) < r.5 as f64 * 0.1)
                .unwrap_or(false)
        })
        .count();
    let u_25 = usable
        .iter()
        .filter(|r| {
            r.11.map(|e| (e.abs() as f64) < r.5 as f64 * 0.25)
                .unwrap_or(false)
        })
        .count();
    let mut u_errs: Vec<i64> = usable
        .iter()
        .filter_map(|r| r.11.map(|e| e.abs()))
        .collect();
    u_errs.sort();
    println!("\nreplays {}: maps compiled start-to-finish {}, sim finished {} (within 10%: {}), median |error| {} ms", rows.len(), compiled_full, finished, within10, med);
    if args.iter().any(|a| a == "--stops") {
        let mut by: std::collections::BTreeMap<String, usize> = Default::default();
        for r in &usable {
            *by.entry(format!("{:?}", r.12)).or_default() += 1;
        }
        println!("usable stop reasons: {by:?}");
        for r in usable
            .iter()
            .filter(|r| format!("{:?}", r.12) != "Finished")
        {
            println!(
                "  {:<8} {:?} at {:>6.0}/{:.0} m ({:>3.0}%) real {:>6} ms walls {:>5}",
                r.0,
                r.12,
                r.8,
                r.4,
                r.8 / r.4 * 100.0,
                r.5,
                r.9
            );
        }
    }
    if args.iter().any(|a| a == "--usable") {
        for r in &usable {
            println!("usable {:<8} chain {:>4} finish {:<3} len {:>6.0} real {:>7} sim {:>7} prog {:>6.0} walls {:>5}", r.0, r.3, if r.2 { "yes" } else { "no" }, r.4, r.5, if r.6 { r.7.to_string() } else { "DNF".into() }, r.8, r.9);
        }
    }
    println!(
        "usable geometry (plausible route length for the real time): {} runs; sim finished {}, within 25%: {}, within 10%: {}, median |error| {:?} ms",
        usable.len(),
        u_fin,
        u_25,
        u_10,
        u_errs.get(u_errs.len() / 2)
    );
    let mut dnf_progress: BTreeMap<u32, usize> = BTreeMap::new();
    for r in rows.iter().filter(|r| !r.6) {
        *dnf_progress
            .entry(((r.8 / r.4.max(1.0)) * 10.0) as u32 * 10)
            .or_default() += 1;
    }
    println!("DNF progress histogram (% of track): {:?}", dnf_progress);
    Ok(())
}
