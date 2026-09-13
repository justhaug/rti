use rti_core::CarState;

use crate::geom::{wrap_angle, TrackGeom};

/// Look-ahead distances (meters) sampled along the centerline.
pub const LOOKAHEAD: [f32; 6] = [8.0, 16.0, 32.0, 64.0, 100.0, 160.0];
/// Observation vector size. Keep in sync with `observe`.
pub const OBS_DIM: usize = 8 + LOOKAHEAD.len() * 3;

/// Egocentric observation for policy/value nets: car dynamics, position
/// relative to the centerline, and the shape of the road ahead. All
/// features are roughly unit-scaled.
pub fn observe(geom: &TrackGeom, s: &CarState) -> [f32; OBS_DIM] {
    let loc = geom.locate(s.x, s.y, s.seg as usize);
    let mut o = [0.0f32; OBS_DIM];
    o[0] = s.forward_speed() / 60.0;
    o[1] = s.lateral_speed() / 20.0;
    o[2] = s.yaw_rate / 2.0;
    o[3] = s.steer_pos;
    o[4] = s.drive;
    o[5] = (loc.lateral / loc.half_width).clamp(-2.0, 2.0);
    o[6] = wrap_angle(s.heading - loc.dir) / std::f32::consts::PI;
    o[7] = 1.0 - (loc.progress / geom.total_len).clamp(0.0, 1.0);
    for (i, &d) in LOOKAHEAD.iter().enumerate() {
        let ahead = (loc.progress + d).min(geom.total_len);
        let base = 8 + i * 3;
        o[base] = wrap_angle(geom.dir_at(ahead) - s.heading) / std::f32::consts::PI;
        o[base + 1] = geom.curvature_at(ahead) * 20.0;
        o[base + 2] = geom.half_width_at(ahead) / 10.0;
    }
    o
}
