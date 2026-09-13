//! Route quality without physics: a compiled route can be judged against a
//! human replay using only checkpoint split times. For a correct route the
//! fraction of the distance reached at checkpoint k should track the fraction
//! of the time elapsed; a route that takes a wrong branch or mirrors a turn
//! diverges. Reports per-map deviation and a corpus summary, so geometry
//! conventions can be compared without involving the car at all.
//!
//! `cargo run --release -p rti-maps --example routecheck -- <replays dir> [--list]`
use rti_maps::catalog::Catalog;
use rti_sim::TrackGeom;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dir = &args[1];
    let list = args.iter().any(|a| a == "--list");
    let cat = Catalog::default_catalog();
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".Replay.Gbx"))
        .collect();
    files.sort();
    let mut devs: Vec<(String, f32, usize, f32, f32)> = vec![];
    let mut no_cps = 0;
    let mut no_route = 0;
    let mut why: std::collections::BTreeMap<String, usize> = Default::default();
    for rf in files {
        let id = rf
            .file_name()
            .unwrap()
            .to_string_lossy()
            .trim_end_matches(".Replay.Gbx")
            .to_string();
        let Ok(r) = std::fs::read(&rf)
            .map_err(anyhow::Error::from)
            .and_then(|d| rti_maps::parse_replay(&d))
        else {
            continue;
        };
        let Some(g) = r.ghosts.first() else { continue };
        let Some(m) = &r.map else { continue };
        if g.race_time_ms == 0 || g.checkpoint_times_ms.len() < 2 {
            no_cps += 1;
            continue;
        }
        let routed = args.iter().any(|a| a == "--routed");
        let compiled = if routed {
            rti_maps::compile::compile_track_routed(m, &cat, "x").map(|(t, r, _, _)| (t, r))
        } else {
            rti_maps::compile_track(m, &cat, "x")
        };
        let (track, rep) = match compiled {
            Ok(x) => x,
            Err(e) => {
                *why.entry(format!("compile: {e}").chars().take(60).collect::<String>())
                    .or_default() += 1;
                no_route += 1;
                continue;
            }
        };
        let geom = TrackGeom::new(track.clone());
        // the last checkpoint time is the finish on most maps
        let total_t = g.race_time_ms as f32;
        let total_d = geom.finish_dist.max(1.0);
        if !rep.finish_found {
            *why.entry("no finish on the route".into()).or_default() += 1;
            no_route += 1;
            continue;
        }
        if geom.checkpoint_dist.is_empty() {
            *why.entry("no checkpoint on the route".into()).or_default() += 1;
            no_route += 1;
            continue;
        }
        // compare as many checkpoints as both sides have
        let n = geom
            .checkpoint_dist
            .len()
            .min(g.checkpoint_times_ms.len().saturating_sub(1));
        if n == 0 {
            no_cps += 1;
            continue;
        }
        let mut dev = 0.0f32;
        for k in 0..n {
            let fd = geom.checkpoint_dist[k] / total_d;
            let ft = g.checkpoint_times_ms[k] as f32 / total_t;
            dev += (fd - ft).abs();
        }
        dev /= n as f32;
        let avg_speed = total_d / (total_t / 1000.0);
        devs.push((id, dev, n, total_d, avg_speed));
    }
    devs.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    if list {
        println!(
            "{:<10} {:>8} {:>4} {:>8} {:>9}",
            "map", "dev", "cps", "route m", "avg m/s"
        );
        for (id, dev, n, d, v) in &devs {
            println!("{:<10} {:>8.3} {:>4} {:>8.0} {:>9.1}", id, dev, n, d, v);
        }
    }
    let good = devs.iter().filter(|d| d.1 < 0.05).count();
    let ok = devs.iter().filter(|d| d.1 < 0.10).count();
    let plausible = devs.iter().filter(|d| (5.0..=70.0).contains(&d.4)).count();
    let med = devs.get(devs.len() / 2).map(|d| d.1).unwrap_or(0.0);
    println!(
        "\nroutes judged {} (skipped: {} without usable checkpoints, {} without a start-to-finish route)",
        devs.len(),
        no_cps,
        no_route
    );
    let mut w: Vec<_> = why.into_iter().collect();
    w.sort_by_key(|x| std::cmp::Reverse(x.1));
    for (k, n) in w.iter().take(8) {
        println!("  {n:>4}  {k}");
    }
    println!(
        "checkpoint-position deviation: median {:.3}; under 0.05 (route essentially right): {}; under 0.10: {}; plausible average speed: {}",
        med, good, ok, plausible
    );
    Ok(())
}
