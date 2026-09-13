//! Quick throughput check: `cargo run --release -p rti-sim --example bench`
use rti_core::{Action, PhysicsParams};
use rti_sim::{rollout_batch, tracks, Sim, TrackGeom};
use std::time::Instant;

fn main() {
    let t = tracks::builtin()
        .into_iter()
        .find(|t| t.name == "loop")
        .unwrap();
    let sim = Sim::new(PhysicsParams::default(), TrackGeom::new(t));
    let seqs: Vec<Vec<Action>> = (0..2048)
        .map(|k| {
            (0..4000)
                .map(|i| {
                    Action::new(
                        ((i + k * 37) as f32 * 0.013).sin() * 0.7,
                        true,
                        i % 211 < 20,
                    )
                })
                .collect()
        })
        .collect();
    let start = Instant::now();
    let (_, ticks) = rollout_batch(&sim, &sim.initial_state(), &seqs, 4000);
    let dt = start.elapsed().as_secs_f64();
    println!(
        "{ticks} ticks in {dt:.2}s = {:.1} Mticks/s ({} threads)",
        ticks as f64 / dt / 1e6,
        rayon::current_num_threads()
    );
    let start = Instant::now();
    let r = rti_sim::rollout(&sim, &sim.initial_state(), &seqs[0], 4000, false);
    let dt = start.elapsed().as_secs_f64();
    println!(
        "single-thread: {:.2} Mticks/s, result {:?}",
        r.ticks as f64 / dt / 1e6,
        r.result
    );
}
