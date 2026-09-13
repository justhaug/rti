//! `cargo run -p rti-maps --example replay -- file.Replay.Gbx`
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("replay");
    let r = rti_maps::parse_replay(&std::fs::read(&path)?)?;
    println!(
        "map {:?} uid {} player {:?} ({}) time {} ms; embedded map {} bytes; warnings {:?}",
        r.map_name,
        r.map_uid,
        r.player_nickname,
        r.player_login,
        r.time_ms,
        r.map_bytes.len(),
        r.warnings
    );
    for g in &r.ghosts {
        println!("ghost login {} race {} ms cps {:?} respawns {} inputs v{} ticks {} start_offset {} events {} warnings {:?}", g.login, g.race_time_ms, g.checkpoint_times_ms, g.respawns, g.input_version, g.ticks, g.start_offset_ms, g.inputs.len(), g.warnings);
        for e in g.inputs.iter().take(40) {
            println!("  {:6} ms {:?}", e.time_ms, e.event);
        }
        let acts = rti_maps::replay::inputs_to_actions(&g.inputs, g.ticks);
        let gas = acts.iter().filter(|a| a.gas).count();
        let brake = acts.iter().filter(|a| a.brake).count();
        let steer_nonzero = acts.iter().filter(|a| a.steer.abs() > 0.01).count();
        println!(
            "  actions: {} ticks, gas {} brake {} steering {}",
            acts.len(),
            gas,
            brake,
            steer_nonzero
        );
    }
    Ok(())
}
