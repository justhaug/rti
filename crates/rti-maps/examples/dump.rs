//! Dump a .Map.Gbx: `cargo run -p rti-maps --example dump -- file.Map.Gbx`
use std::collections::BTreeMap;

fn main() -> anyhow::Result<()> {
    for path in std::env::args().skip(1) {
        let data = std::fs::read(&path)?;
        let m = rti_maps::parse_map(&data)?;
        println!("== {path}");
        println!("name={:?} uid={} author={} deco={} type={} vehicle={} size={:?} cps={} laps={} author_ms={}", m.info.name, m.info.uid, m.info.author_nick, m.info.decoration, m.info.map_type, m.info.vehicle, m.info.size, m.info.nb_checkpoints, m.info.nb_laps, m.info.author_ms);
        println!(
            "header chunks: {:?}",
            m.header_chunks
                .iter()
                .map(|c| format!("{c:08X}"))
                .collect::<Vec<_>>()
        );
        println!(
            "body chunks: {:?}",
            m.body_chunks
                .iter()
                .map(|c| format!("{c:08X}"))
                .collect::<Vec<_>>()
        );
        println!(
            "blocks: {}  items: {}  warnings: {:?}",
            m.blocks.len(),
            m.items.len(),
            m.warnings
        );
        let mut freq: BTreeMap<&str, usize> = BTreeMap::new();
        for b in &m.blocks {
            *freq.entry(b.name.as_str()).or_default() += 1;
        }
        let mut v: Vec<_> = freq.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        for (n, c) in v.iter().take(40) {
            println!("  {c:5} {n}");
        }
        for b in m
            .blocks
            .iter()
            .filter(|b| b.waypoint_tag.is_some())
            .take(10)
        {
            println!(
                "  waypoint {:?} {:?} at {:?} dir {}",
                b.name, b.waypoint_tag, b.coord, b.dir
            );
        }
        let mut ifreq: BTreeMap<&str, usize> = BTreeMap::new();
        for i in &m.items {
            *ifreq.entry(i.model.as_str()).or_default() += 1;
        }
        for (n, c) in ifreq.iter().take(15) {
            println!("  item {c:5} {n}");
        }
    }
    Ok(())
}
