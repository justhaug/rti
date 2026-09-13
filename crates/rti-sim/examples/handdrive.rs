//! Sanity check that built-in tracks are drivable by a simple
//! centerline-following controller with speed limiting from curvature.
//! `cargo run --release -p rti-sim --example handdrive`
use rti_core::{Action, PhysicsParams};
use rti_sim::geom::wrap_angle;
use rti_sim::{tracks, Sim, TrackGeom};

fn main() {
    for t in tracks::builtin() {
        let sim = Sim::new(PhysicsParams::default(), TrackGeom::new(t));
        let mut s = sim.initial_state();
        while !sim.is_terminal(&s, 12000) {
            let loc = sim.geom.locate(s.x, s.y, s.seg as usize);
            let v = s.forward_speed().max(1.0);
            let mut vmax = 200.0f32;
            let mut d = 5.0;
            while d < 120.0 {
                let c = sim.geom.curvature_at(loc.progress + d).abs();
                if c > 1e-4 {
                    let vc = (sim.params.grip * 0.85 / c).sqrt();
                    let vb = (vc * vc + 2.0 * sim.params.brake_decel * 0.8 * d).sqrt();
                    vmax = vmax.min(vb);
                }
                d += 5.0;
            }
            let ahead = sim.geom.dir_at(loc.progress + 8.0 + v * 0.3);
            let herr = wrap_angle(ahead - s.heading);
            let steer = (2.0 * herr - 0.08 * loc.lateral).clamp(-1.0, 1.0);
            let a = Action::new(steer, v < vmax, v > vmax + 1.0);
            sim.step(&mut s, a);
        }
        println!(
            "{:>14}: finished={} time={}ms progress={:.0}/{:.0}m walls={} stuck={}",
            sim.geom.track.name,
            s.finished,
            s.finish_tick * 10,
            s.progress,
            sim.geom.total_len,
            s.wall_hits,
            s.stuck_ticks
        );
    }
}
