//! Compare all methods on a track: `cargo run --release -p rti-search --example compare -- hairpin 5000000`
use rti_core::experiment::{SearchMethod, SearchParams};
use rti_core::PhysicsParams;
use rti_search::common::SearchInput;
use rti_search::{run, Budget};
use rti_sim::{tracks, Sim, TrackGeom};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = args.get(1).map(|s| s.as_str()).unwrap_or("hairpin");
    let ticks: u64 = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000_000);
    let t = tracks::builtin()
        .into_iter()
        .find(|t| t.name == name)
        .expect("track");
    let sim = Sim::new(PhysicsParams::default(), TrackGeom::new(t));
    let params = SearchParams::default();
    for m in SearchMethod::ALL {
        if m == SearchMethod::Policy {
            continue;
        }
        let inp = SearchInput {
            sim: &sim,
            params: &params,
            budget: Budget {
                max_ticks: ticks,
                max_wall_ms: None,
            },
            seed: 1,
            warm_start: None,
            policy: None,
        };
        let out = run(m, &inp).unwrap();
        let r = &out.best_result;
        println!(
            "{:>16}: finished={} time={}ms progress={:.0}m walls={} evals={} ticks={} wall={}ms",
            m.name(),
            r.finished,
            r.time_ms,
            r.progress,
            r.wall_hits,
            out.evaluations,
            out.ticks,
            out.wall_ms
        );
    }
}
