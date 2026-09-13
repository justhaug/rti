//! ASCII dump of the rasterised 3D world around the compiled track.
//! `cargo run --release -p rti-maps --example worldmap -- map.Map.Gbx [cell_m]`
use rti_maps::catalog::Catalog;
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let m = rti_maps::parse_map(&std::fs::read(&a[1])?)?;
    let step: f32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(4.0);
    let (track, rep, world) =
        rti_maps::compile::compile_track3(&m, &Catalog::default_catalog(), "x")?;
    println!(
        "{:?}: chained {} finish {} len {:.0} m; world {}x{} cells, {} layers",
        m.info.name,
        rep.blocks_chained,
        rep.finish_found,
        track.length(),
        world.w,
        world.h,
        world.layers.len()
    );
    let xs: Vec<f32> = track.nodes.iter().map(|n| n.x).collect();
    let ys: Vec<f32> = track.nodes.iter().map(|n| n.y).collect();
    let (x0, x1) = (
        xs.iter().cloned().fold(f32::INFINITY, f32::min) - 40.0,
        xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max) + 40.0,
    );
    let (z0, z1) = (
        ys.iter().cloned().fold(f32::INFINITY, f32::min) - 40.0,
        ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max) + 40.0,
    );
    let hmin = track
        .nodes
        .iter()
        .map(|n| n.h)
        .fold(f32::INFINITY, f32::min);
    let mut z = z1;
    while z >= z0 {
        let mut line = String::new();
        let mut x = x0;
        while x <= x1 {
            let on_line = track
                .nodes
                .iter()
                .any(|n| (n.x - x).abs() < step / 2.0 && (n.y - z).abs() < step / 2.0);
            let ch = if on_line {
                'o'
            } else {
                // any layer at this cell
                let mut best: Option<f32> = None;
                for probe_h in [hmin, hmin + 8.0, hmin + 16.0, hmin + 32.0, hmin + 64.0] {
                    if let Some(l) = world.layer_at(x, z, probe_h, 40.0) {
                        best = Some(best.map_or(l.y, |b: f32| b.max(l.y)));
                    }
                }
                match best {
                    Some(y) => {
                        let lvl = ((y - hmin) / 8.0).round() as i32;
                        if lvl <= 0 {
                            '#'
                        } else {
                            char::from_digit((lvl as u32).min(9), 10).unwrap()
                        }
                    }
                    None => '.',
                }
            };
            line.push(ch);
            x += step;
        }
        println!("{line}");
        z -= step;
    }
    let s0 = &track.nodes[0];
    println!(
        "sample at spawn: {:?}; layers there: {:?}",
        world.sample(s0.x, s0.y, s0.h, 6.0),
        (0..6)
            .filter_map(|k| world.layer_at(s0.x, s0.y, s0.h - 40.0 + k as f32 * 16.0, 8.0))
            .collect::<Vec<_>>()
    );
    println!(
        "start {:?} h {:.1}; finish node {:?}",
        (track.nodes[0].x, track.nodes[0].y),
        track.nodes[0].h,
        track
            .finish
            .map(|f| (track.nodes[f].x, track.nodes[f].y, track.nodes[f].h))
    );
    Ok(())
}
