//! Where does the chain break? Prints recognised pieces near the chain's end.
//! `cargo run --release -p rti-maps --example chaindebug -- map.Map.Gbx`
use rti_maps::catalog::Catalog;
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("map");
    let m = rti_maps::parse_map(&std::fs::read(&path)?)?;
    let cat = Catalog::default_catalog();
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--near") {
        let v: Vec<i32> = args[i + 1].split(',').map(|s| s.parse().unwrap()).collect();
        for b in &m.blocks {
            if (b.coord[0] as i32 - v[0]).abs() <= 2 && (b.coord[2] as i32 - v[1]).abs() <= 2 {
                println!(
                    "  near {:<44} dir {} at {:?} y {} {}",
                    b.name,
                    b.dir,
                    b.coord,
                    b.coord[1] as f32 * 8.0,
                    if cat.resolve(&b.name).is_some() {
                        "[recognised]"
                    } else {
                        ""
                    }
                );
            }
        }
        return Ok(());
    }
    let rec: Vec<_> = m
        .blocks
        .iter()
        .filter(|b| cat.resolve(&b.name).is_some())
        .collect();
    println!(
        "{:?}: {} blocks, {} recognised",
        m.info.name,
        m.blocks.len(),
        rec.len()
    );
    for b in rec.iter().take(80) {
        let r = cat.resolve(&b.name).unwrap();
        println!(
            "  {:<44} dir {} at {:?}{} {:?} len {} size {} {:?}",
            b.name,
            b.dir,
            b.coord,
            if b.free {
                format!(
                    " FREE {:?} yaw {:.2}",
                    b.free_pos,
                    b.free_pyr.map(|p| p[1]).unwrap_or(0.0)
                )
            } else {
                String::new()
            },
            r.template.shape,
            r.template.len,
            r.template.size,
            r.template.marker
        );
    }
    for force in [None, Some(true), Some(false)] {
        match rti_maps::compile::compile_track_with(&m, &cat, "dbg", force) {
            Ok((t, rep)) => {
                println!(
                    "force {:?}: chained {} start={} finish={} len {:.0} m conv {} gaps {}",
                    force,
                    rep.blocks_chained,
                    rep.start_found,
                    rep.finish_found,
                    t.length(),
                    rep.convention,
                    rep.bridged_gaps
                );
                for (k, c) in rep.chain.iter().enumerate().take(12) {
                    println!("   chain[{k}] {c}");
                }
                let last = t.nodes.last().unwrap();
                println!(
                    "   ends at ({:.1}, {:.1}) = cell ({:.2}, {:.2})",
                    last.x,
                    last.y,
                    last.x / 32.0,
                    last.y / 32.0
                );
                let first = t.nodes[0];
                println!(
                    "   starts at ({:.1}, {:.1}) = cell ({:.2}, {:.2})",
                    first.x,
                    first.y,
                    first.x / 32.0,
                    first.y / 32.0
                );
                // nearest blocks (any) to the break point
                let mut near: Vec<(f32, String)> = m
                    .blocks
                    .iter()
                    .map(|b| {
                        let (bx, bz) = if b.free {
                            b.free_pos.map(|p| (p[0], p[2])).unwrap_or((0.0, 0.0))
                        } else {
                            (
                                b.coord[0] as f32 * 32.0 + 16.0,
                                b.coord[2] as f32 * 32.0 + 16.0,
                            )
                        };
                        let d = ((bx - last.x).powi(2) + (bz - last.y).powi(2)).sqrt();
                        (
                            d,
                            format!(
                                "{} dir {} at {:?} y {} {}",
                                b.name,
                                b.dir,
                                b.coord,
                                b.coord[1] as f32 * 8.0,
                                if cat.resolve(&b.name).is_some() {
                                    "[recognised]"
                                } else {
                                    ""
                                }
                            ),
                        )
                    })
                    .collect();
                near.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                for (d, n) in near.iter().take(8) {
                    println!("   near end ({d:.0} m): {n}");
                }
            }
            Err(e) => println!("force {force:?}: compile error: {e}"),
        }
    }
    Ok(())
}
