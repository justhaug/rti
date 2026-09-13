//! Debug the compiler on real maps: `cargo run -p rti-maps --example chain -- a.Map.Gbx ...`
use rti_maps::catalog::Catalog;
fn main() -> anyhow::Result<()> {
    let cat = Catalog::default_catalog();
    for path in std::env::args().skip(1) {
        let m = rti_maps::parse_map(&std::fs::read(&path)?)?;
        let rec: Vec<_> = m
            .blocks
            .iter()
            .filter(|b| cat.resolve(&b.name).is_some())
            .collect();
        println!(
            "== {} {:?}: {} blocks, {} recognised",
            path,
            m.info.name,
            m.blocks.len(),
            rec.len()
        );
        for b in rec.iter().take(60) {
            let r = cat.resolve(&b.name).unwrap();
            println!(
                "   {:<40} dir {} at {:?} {:?} {:?} flags {:08x}",
                b.name, b.dir, b.coord, r.template.shape, r.template.marker, b.flags
            );
        }
        match rti_maps::compile_track(&m, &cat, "dbg") {
            Ok((t, rep)) => println!(
                "   → chained {}/{} start={} finish={} cps={} len={:.0} m conv={} warn={:?}",
                rep.blocks_chained,
                rep.blocks_recognized,
                rep.start_found,
                rep.finish_found,
                rep.checkpoints,
                t.length(),
                rep.convention,
                rep.warnings
            ),
            Err(e) => println!("   → compile error: {e}"),
        }
    }
    Ok(())
}
