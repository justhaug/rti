//! Data-driven speed profile from replays: for runs that start with a
//! straight full-throttle launch (steer 0, gas on, no brake) up to the first
//! checkpoint, print (distance to cp1 along the compiled track, real split
//! time). Used to fit acceleration / top speed of the physics model.
//! `cargo run --release -p rti-maps --example speedfit -- <replays dir>`
use rti_maps::catalog::Catalog;
use rti_maps::replay::inputs_to_actions;

fn main() -> anyhow::Result<()> {
    let dir = std::env::args().nth(1).expect("replays dir");
    let cat = Catalog::default_catalog();
    let mut files: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".Replay.Gbx"))
        .collect();
    files.sort();
    println!(
        "{:<8} {:>8} {:>8} {:>9} {:>8}  note",
        "map", "cp1 m", "cp1 ms", "mean km/h", "steer%"
    );
    for rf in files {
        let Ok(replay) = rti_maps::parse_replay(&std::fs::read(&rf)?) else {
            continue;
        };
        let Some(g) = replay.ghosts.first() else {
            continue;
        };
        let Some(map) = &replay.map else { continue };
        let Ok((track, _rep)) = rti_maps::compile_track(map, &cat, "x") else {
            continue;
        };
        let geom = rti_sim::TrackGeom::new(track.clone());
        let Some(&cp1) = geom.checkpoint_dist.first() else {
            continue;
        };
        let Some(&t1) = g.checkpoint_times_ms.first() else {
            continue;
        };
        if t1 == 0 {
            continue;
        }
        let acts = inputs_to_actions(&g.inputs, (t1 / 10) as u32);
        let steer_ticks = acts.iter().filter(|a| a.steer.abs() > 0.01).count();
        let gas_ticks = acts.iter().filter(|a| a.gas).count();
        let brake_ticks = acts.iter().filter(|a| a.brake).count();
        // straightness of the compiled path to cp1: sum of |heading change|
        let mut turn = 0.0f32;
        let mut d = 0.0;
        while d < cp1 {
            turn += geom.curvature_at(d).abs() * 4.0;
            d += 4.0;
        }
        let mean_kmh = cp1 / (t1 as f32 / 1000.0) * 3.6;
        let note = if steer_ticks * 100 / acts.len().max(1) < 5
            && brake_ticks == 0
            && gas_ticks * 100 / acts.len().max(1) > 95
            && turn < 0.3
        {
            "LAUNCH"
        } else {
            ""
        };
        println!(
            "{:<8} {:>8.0} {:>8} {:>9.0} {:>7}%  {} turn {:.2} gas {}% brake {}",
            rf.file_name()
                .unwrap()
                .to_string_lossy()
                .trim_end_matches(".Replay.Gbx"),
            cp1,
            t1,
            mean_kmh,
            steer_ticks * 100 / acts.len().max(1),
            note,
            turn,
            gas_ticks * 100 / acts.len().max(1),
            brake_ticks
        );
    }
    Ok(())
}
