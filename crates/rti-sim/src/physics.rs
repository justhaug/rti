use rti_core::{Action, CarState, PhysicsParams, TICK_SECONDS};
use serde::{Deserialize, Serialize};

use crate::geom::{wrap_angle, TrackGeom};

/// Physics effects that exist in the "real" world (oracle stand-in) but are
/// *not* part of the public model. The public simulator always runs with
/// `Ext::NONE`; discovering and modelling these is what sim-improvement
/// research is about. Keep this struct out of `PhysicsParams` on purpose.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Ext {
    /// Aerodynamic downforce: extra grip proportional to speed^2 (1/m).
    pub downforce: f32,
    /// Extra first-order lag on steering input (seconds).
    pub steer_lag: f32,
    /// Gear-shift power dip: fraction of engine power lost around
    /// `gear_speed` m/s (band of ±4 m/s).
    pub gear_dip: f32,
    pub gear_speed: f32,
    /// Grip loss when yaw rate is high (spin-out tendency), per rad/s.
    pub yaw_grip_loss: f32,
}

impl Ext {
    pub const NONE: Ext = Ext {
        downforce: 0.0,
        steer_lag: 0.0,
        gear_dip: 0.0,
        gear_speed: 0.0,
        yaw_grip_loss: 0.0,
    };
}

/// A simulator instance: physics params + compiled track. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Sim {
    pub params: PhysicsParams,
    pub geom: TrackGeom,
    pub ext: Ext,
}

impl Sim {
    pub fn new(params: PhysicsParams, geom: TrackGeom) -> Sim {
        Sim {
            params,
            geom,
            ext: Ext::NONE,
        }
    }

    pub fn with_ext(mut self, ext: Ext) -> Sim {
        self.ext = ext;
        self
    }

    /// Initial state on the start line, pointing down the first segment.
    pub fn initial_state(&self) -> CarState {
        let t = &self.geom.track;
        CarState {
            x: t.nodes[0].x,
            y: t.nodes[0].y,
            heading: t.start_heading(),
            ..Default::default()
        }
    }

    /// Advance one tick. Pure function of (state, action, params, track, ext).
    #[inline]
    pub fn step(&self, s: &mut CarState, a: Action) {
        if s.finished {
            return;
        }
        let p = &self.params;
        let e = &self.ext;
        let dt = TICK_SECONDS;
        let a = a.clamped();

        // --- where are we ---
        let loc = self.geom.locate(s.x, s.y, s.seg as usize);
        let on_track = loc.lateral.abs() <= loc.half_width;
        let si = loc.surface.index();
        let mut grip_mult = p.surface_grip[si];
        let mut drive_mult = p.surface_drive[si];
        let mut drag_mult = 1.0;
        if !on_track {
            grip_mult *= p.offtrack_grip;
            drive_mult *= p.offtrack_grip;
            drag_mult = p.offtrack_drag;
            s.offtrack_ticks += 1;
        }

        // --- steering column ---
        let steer_target = if e.steer_lag > 0.0 {
            // extra lag implemented as blend toward input
            let k = dt / (e.steer_lag + dt);
            s.steer_pos + (a.steer - s.steer_pos) * k
        } else {
            a.steer
        };
        let max_delta = p.steer_rate * dt;
        s.steer_pos += (steer_target - s.steer_pos).clamp(-max_delta, max_delta);

        // --- drive engagement ---
        let drive_target = if a.gas { 1.0 } else { 0.0 };
        s.drive += (drive_target - s.drive) * (dt / p.drive_tau).min(1.0);

        // --- car-frame velocity ---
        let (ch, sh) = (s.heading.cos(), s.heading.sin());
        let vf = s.vx * ch + s.vy * sh;
        let vl = -s.vx * sh + s.vy * ch;
        let speed = (vf * vf + vl * vl).sqrt();

        // --- available grip ---
        let mut a_max = p.grip * grip_mult;
        if e.downforce > 0.0 {
            a_max += e.downforce * speed * speed;
        }
        if e.yaw_grip_loss > 0.0 {
            a_max *= (1.0 - e.yaw_grip_loss * s.yaw_rate.abs()).max(0.2);
        }
        // above slip onset the friction circle shrinks slightly (tyre load)
        if speed > p.slip_onset {
            a_max *= 1.0 / (1.0 + 0.004 * (speed - p.slip_onset));
        }

        // --- yaw from kinematic bicycle, limited by lateral grip ---
        let delta = s.steer_pos * p.steer_max / (1.0 + vf.abs() / p.steer_speed_falloff);
        let omega_des = vf * delta.tan() / p.wheelbase;
        let omega_max = a_max / vf.abs().max(1.0);
        let omega = omega_des.clamp(-omega_max, omega_max);
        let a_lat_used = (omega * vf).abs();
        s.yaw_rate = omega;
        s.heading = wrap_angle(s.heading + omega * dt);

        // --- longitudinal ---
        let mut engine = 0.0;
        if a.gas {
            engine = p.engine_accel * drive_mult * s.drive
                / (1.0 + vf.max(0.0) / p.engine_falloff_speed);
            if e.gear_dip > 0.0 {
                let d = (vf - e.gear_speed).abs();
                if d < 4.0 {
                    engine *= 1.0 - e.gear_dip * (1.0 - d / 4.0);
                }
            }
        }
        let mut brake = 0.0;
        if a.brake {
            brake = p.brake_decel * drive_mult;
        }
        // traction-limited longitudinal accel (remaining friction circle)
        let a_long_avail = (a_max * a_max - a_lat_used * a_lat_used).max(0.0).sqrt();
        let mut a_long = engine - brake * vf.signum() * (vf.abs() > 0.05) as u8 as f32;
        if a.brake && vf.abs() <= 0.05 {
            a_long = engine.min(0.0);
        }
        a_long = a_long.clamp(-a_long_avail, a_long_avail);
        // drag and rolling resistance do not need traction
        a_long -= p.drag * drag_mult * vf * vf.abs();
        if vf.abs() > 0.05 {
            a_long -= p.rolling * vf.signum();
        }
        let mut vf2 = vf + a_long * dt;
        if vf.abs() <= 0.05 && !a.gas && vf2.abs() < 0.05 {
            vf2 = 0.0;
        }

        // --- lateral: re-express old velocity in the new heading frame; tyres
        // pull lateral velocity toward zero with whatever grip is left ---
        let (ch2, sh2) = (s.heading.cos(), s.heading.sin());
        // world velocity before lateral correction, using updated forward speed
        let wx = vf2 * ch - vl * sh;
        let wy = vf2 * sh + vl * ch;
        let vf3 = wx * ch2 + wy * sh2;
        let mut vl3 = -wx * sh2 + wy * ch2;
        let lat_avail = (a_max * a_max - (a_long.min(a_long_avail)).powi(2))
            .max(0.0)
            .sqrt();
        let corr = vl3.clamp(-lat_avail * dt, lat_avail * dt);
        vl3 -= corr;
        vl3 *= p.slide_damping;

        // --- recompose, cap, integrate ---
        let mut vx = vf3 * ch2 - vl3 * sh2;
        let mut vy = vf3 * sh2 + vl3 * ch2;
        let sp = (vx * vx + vy * vy).sqrt();
        if sp > p.max_speed {
            vx *= p.max_speed / sp;
            vy *= p.max_speed / sp;
        }
        s.vx = vx;
        s.vy = vy;
        s.x += vx * dt;
        s.y += vy * dt;
        s.tick += 1;

        // --- track interaction after move ---
        let loc2 = self.geom.locate(s.x, s.y, loc.seg);
        s.seg = loc2.seg as u32;
        let over = loc2.lateral.abs() - loc2.half_width;
        if self.geom.track.walls && over > 0.0 {
            // push back inside and take the hit
            let nx = -loc2.dir.sin();
            let ny = loc2.dir.cos();
            let sign = loc2.lateral.signum();
            s.x -= nx * sign * over;
            s.y -= ny * sign * over;
            let vn = s.vx * nx + s.vy * ny;
            if vn * sign > 0.0 {
                // remove outward normal velocity, scale the rest
                s.vx -= vn * nx;
                s.vy -= vn * ny;
                s.vx *= p.wall_restitution;
                s.vy *= p.wall_restitution;
                s.wall_hits += 1;
            }
        }
        s.progress = loc2.progress;

        // --- checkpoints and finish ---
        let cps = &self.geom.checkpoint_dist;
        while (s.next_checkpoint as usize) < cps.len()
            && s.progress >= cps[s.next_checkpoint as usize]
        {
            s.next_checkpoint += 1;
        }
        if (s.next_checkpoint as usize) >= cps.len() && s.progress >= self.geom.finish_dist {
            s.finished = true;
            s.finish_tick = s.tick;
        }
    }

    /// Whether this state should stop a rollout early: finished, fell off an
    /// open-edge track, or exceeded the tick limit.
    #[inline]
    pub fn is_terminal(&self, s: &CarState, max_ticks: u32) -> bool {
        if s.finished || s.tick >= max_ticks {
            return true;
        }
        if !self.geom.track.walls {
            let loc = self.geom.locate(s.x, s.y, s.seg as usize);
            if loc.lateral.abs() > 3.0 * loc.half_width {
                return true;
            }
        }
        false
    }
}
