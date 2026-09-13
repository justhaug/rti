//! Block census + catalog coverage over a directory of maps.
//! `cargo run --release -p rti-maps --example census -- <dir> [--names]`
use rti_maps::catalog::Catalog;
use std::collections::BTreeMap;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dir = &args[1];
    let show_names = args.iter().any(|a| a == "--names");
    let cat = Catalog::default_catalog();
    let mut freq: BTreeMap<String, (usize, usize)> = BTreeMap::new(); // name -> (count, maps)
    let mut maps = 0;
    let mut tot_rec = 0f64;
    let mut tot_chain = 0f64;
    let mut finished = 0;
    let mut rows = vec![];
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".Map.Gbx"))
        .collect();
    files.sort();
    for f in files {
        let data = match std::fs::read(&f) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let m = match rti_maps::parse_map(&data) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("{}: {e}", f.display());
                continue;
            }
        };
        if m.blocks.is_empty() {
            continue;
        }
        maps += 1;
        let mut seen = std::collections::HashSet::new();
        for b in &m.blocks {
            let e = freq.entry(b.name.clone()).or_default();
            e.0 += 1;
            if seen.insert(b.name.clone()) {
                e.1 += 1;
            }
        }
        let rec = m
            .blocks
            .iter()
            .filter(|b| cat.resolve(&b.name).is_some())
            .count();
        let (chained, fin, len) = match rti_maps::compile_track(&m, &cat, "x") {
            Ok((t, r)) => (r.blocks_chained, r.finish_found, t.length()),
            Err(_) => (0, false, 0.0),
        };
        if fin {
            finished += 1;
        }
        let cov = if rec > 0 {
            chained as f64 / rec as f64
        } else {
            0.0
        };
        tot_rec += rec as f64 / m.blocks.len().max(1) as f64;
        tot_chain += cov;
        rows.push((
            f.file_name().unwrap().to_string_lossy().to_string(),
            m.info.name.clone(),
            m.blocks.len(),
            rec,
            chained,
            fin,
            len,
        ));
    }
    rows.sort_by(|a, b| b.4.cmp(&a.4));
    for r in &rows {
        println!(
            "{:<20} {:<32} blocks {:>6} recognised {:>5} chained {:>4} finish {:<5} {:>6.0} m",
            r.0,
            r.1.chars().take(32).collect::<String>(),
            r.2,
            r.3,
            r.4,
            r.5,
            r.6
        );
    }
    println!("\nmaps {maps}: mean recognised fraction {:.1}%, mean chained/recognised {:.1}%, finish reached in {finished}", tot_rec / maps as f64 * 100.0, tot_chain / maps as f64 * 100.0);
    if show_names {
        let mut v: Vec<_> = freq.iter().collect();
        v.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then(b.1 .0.cmp(&a.1 .0)));
        println!("\nblock names by number of maps using them (unrecognised only):");
        for (n, (c, mc)) in v.iter().take(400) {
            if cat.resolve(n).is_none() {
                println!("  {mc:>4} maps {c:>7} blocks  {n}");
            }
        }
    }
    Ok(())
}
