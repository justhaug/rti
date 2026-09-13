//! Trace the hand controller on one track: `cargo run --release -p rti-sim --example trace -- hairpin`
use rti_core::{Action, PhysicsParams};
use rti_sim::geom::wrap_angle;
use rti_sim::{tracks, Sim, TrackGeom};

fn main() {
    let name = std::env::args().nth(1).unwrap_or("hairpin".into());
    let t = tracks::builtin()
        .into_iter()
        .find(|t| t.name == name)
        .unwrap();
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
        if s.tick % 25 == 0 {
            println!("t={:5} prog={:6.1} lat={:6.2}/{:.1} v={:5.1} vmax={:5.1} steer={:5.2} yaw={:5.2} hdg={:5.2} dir={:5.2} walls={} curv={:.3}", s.tick, loc.progress, loc.lateral, loc.half_width, v, vmax, steer, s.yaw_rate, s.heading, loc.dir, s.wall_hits, sim.geom.curvature_at(loc.progress));
        }
        sim.step(&mut s, a);
    }
}
