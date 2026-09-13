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

/// A run is declared stuck (terminal) after this many consecutive
/// near-stationary ticks.
pub const STUCK_TICKS: u32 = 200;
/// Gravity (m/s²).
pub const GRAVITY: f32 = 9.81;
/// Falling this far below the last surface is a DNF (fell off the map).
pub const FALL_LIMIT_M: f32 = 40.0;
/// Height steps smaller than this keep the car grounded (template seams,
/// curbs); larger drops launch it.
pub const GROUND_SNAP_M: f32 = 2.5;
/// Probe distance used to find the outward normal of a road edge.
pub const EDGE_PROBE_M: f32 = 2.5;

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
        let heading = t.start_heading();
        let (mut x, mut y, mut h) = (t.nodes[0].x, t.nodes[0].y, t.nodes[0].h);
        if let Some(w) = self.geom.world.as_ref() {
            // snap the spawn onto the nearest drivable sample along the track
            for k in 0..8 {
                let px = t.nodes[0].x + heading.cos() * k as f32 * 0.5;
                let py = t.nodes[0].y + heading.sin() * k as f32 * 0.5;
                if let Some(g) = w.sample_below(px, py, h + 8.0, 8.0) {
                    x = px;
                    y = py;
                    h = g.y;
                    break;
                }
            }
        }
        CarState {
            x,
            y,
            h,
            heading,
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
        // 3D world: the surface under the car decides grip, slope and surface type
        let ground = self
            .geom
            .world
            .as_ref()
            .and_then(|w| w.sample(s.x, s.y, s.h, 4.0));
        let airborne = s.air_ticks > 0;
        let on_track = match &ground {
            Some(_) => !airborne,
            None => loc.lateral.abs() <= loc.half_width && self.geom.world.is_none(),
        };
        let si = ground
            .map(|g| g.surface.index())
            .unwrap_or(loc.surface.index());
        let mut grip_mult = p.surface_grip[si];
        let mut drive_mult = p.surface_drive[si];
        let mut drag_mult = p.surface_drag[si];
        if airborne {
            // no tyre forces in the air
            grip_mult = 0.0;
            drive_mult = 0.0;
        } else if !on_track {
            grip_mult *= p.offtrack_grip;
            drive_mult *= p.offtrack_grip;
            drag_mult = p.offtrack_drag;
            s.offtrack_ticks += 1;
        }
        // normal load on slopes/banking scales the available grip
        let n_y = ground.map(|g| g.normal[1]).unwrap_or(1.0).clamp(0.2, 1.0);
        if !airborne {
            grip_mult *= n_y;
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
        let mut a_max = p.grip * grip_mult + p.downforce * grip_mult * speed * speed;
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

        // --- longitudinal demand first (brake priority in the friction circle) ---
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
        let moving = vf.abs() > 0.05;
        let mut a_drive = engine;
        if a.brake {
            if moving {
                a_drive = engine - p.brake_decel * drive_mult * vf.signum();
            } else {
                a_drive = engine.min(0.0);
            }
        }
        // traction-limited by the full friction circle
        a_drive = a_drive.clamp(-a_max, a_max);
        let lat_avail = (a_max * a_max - a_drive * a_drive).max(0.0).sqrt();

        // --- yaw from kinematic bicycle, limited by remaining lateral grip ---
        let delta = s.steer_pos * p.steer_max / (1.0 + vf.abs() / p.steer_speed_falloff);
        let omega_des = vf * delta.tan() / p.wheelbase;
        let omega_max = lat_avail / vf.abs().max(1.0);
        let omega = omega_des.clamp(-omega_max, omega_max);
        s.yaw_rate = omega;
        s.heading = wrap_angle(s.heading + omega * dt);

        // drag and rolling resistance do not need traction
        let mut a_long = a_drive - p.drag * drag_mult * vf * vf.abs();
        if moving {
            a_long -= p.rolling * vf.signum();
        }
        let mut vf2 = vf + a_long * dt;
        if !moving && !a.gas && vf2.abs() < 0.05 {
            vf2 = 0.0;
        }

        // --- lateral: re-express old velocity in the new heading frame; tyres
        // pull lateral velocity toward zero with whatever grip is left ---
        let (ch2, sh2) = (s.heading.cos(), s.heading.sin());
        let wx = vf2 * ch - vl * sh;
        let wy = vf2 * sh + vl * ch;
        let vf3 = wx * ch2 + wy * sh2;
        let mut vl3 = -wx * sh2 + wy * ch2;
        let corr = vl3.clamp(-lat_avail * dt, lat_avail * dt);
        vl3 -= corr;
        vl3 *= p.slide_damping;

        // --- recompose, cap, integrate ---
        let mut vx = vf3 * ch2 - vl3 * sh2;
        let mut vy = vf3 * sh2 + vl3 * ch2;
        if let Some(g) = ground {
            if !airborne {
                // gravity component along the surface (downhill push / banking)
                vx += GRAVITY * g.normal[1] * g.normal[0] * dt;
                vy += GRAVITY * g.normal[1] * g.normal[2] * dt;
            }
        }
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

        // --- vertical dynamics on a 3D world ---
        if let Some(world) = self.geom.world.as_ref() {
            let prev_x = s.x - vx * dt;
            let prev_y = s.y - vy * dt;
            let under = world.sample_below(s.x, s.y, s.h, GROUND_SNAP_M);
            match under {
                Some(g) if !airborne && s.h - g.y <= GROUND_SNAP_M + s.vh.max(0.0) * dt => {
                    // grounded: follow the surface
                    s.vh = (g.y - s.h) / dt;
                    s.h = g.y;
                    s.ground_h = g.y;
                    s.air_ticks = 0;
                }
                Some(g) if airborne => {
                    s.vh -= GRAVITY * dt;
                    s.h += s.vh * dt;
                    s.air_ticks += 1;
                    if s.h <= g.y {
                        // landing: absorb vertical speed, keep most horizontal speed
                        s.h = g.y;
                        s.ground_h = g.y;
                        s.vh = 0.0;
                        s.air_ticks = 0;
                        s.vx *= p.landing_keep;
                        s.vy *= p.landing_keep;
                    }
                }
                Some(_) => {
                    // surface dropped away: take off
                    s.vh -= GRAVITY * dt;
                    s.h += s.vh * dt;
                    s.air_ticks += 1;
                }
                None => {
                    // nothing drivable near our height: road pieces have walls, open pieces let us fall
                    if !airborne && ground.map(|g| !g.open).unwrap_or(false) {
                        // Road edge. Derive the outward normal from the drivable
                        // mask around the previous position (no centerline needed),
                        // then slide along the edge and lose speed with the impact.
                        let (mut nx, mut ny) = (0.0f32, 0.0f32);
                        for k in 0..8 {
                            let a = k as f32 * std::f32::consts::FRAC_PI_4;
                            let (dx, dy) = (a.cos(), a.sin());
                            if world
                                .layer_at(
                                    prev_x + dx * EDGE_PROBE_M,
                                    prev_y + dy * EDGE_PROBE_M,
                                    s.h,
                                    4.0,
                                )
                                .is_none()
                            {
                                nx += dx;
                                ny += dy;
                            }
                        }
                        let nl = (nx * nx + ny * ny).sqrt();
                        let (nx, ny) = if nl > 1e-3 {
                            (nx / nl, ny / nl)
                        } else {
                            let mx = s.x - prev_x;
                            let my = s.y - prev_y;
                            let ml = (mx * mx + my * my).sqrt().max(1e-6);
                            (mx / ml, my / ml)
                        };
                        let (tx, ty) = (-ny, nx);
                        let along = (s.x - prev_x) * tx + (s.y - prev_y) * ty;
                        let mut placed = false;
                        for nudge in [0.0f32, 0.5, 1.5, 3.0] {
                            let cand = (
                                prev_x + along * tx - nx * nudge,
                                prev_y + along * ty - ny * nudge,
                            );
                            if world
                                .layer_below(cand.0, cand.1, s.h, GROUND_SNAP_M)
                                .is_some()
                            {
                                s.x = cand.0;
                                s.y = cand.1;
                                placed = true;
                                break;
                            }
                        }
                        if !placed {
                            s.x = prev_x;
                            s.y = prev_y;
                        }
                        let vn = s.vx * nx + s.vy * ny;
                        if vn > 0.0 {
                            let speed = s.speed().max(1e-3);
                            let impact = vn / speed;
                            s.vx -= vn * nx;
                            s.vy -= vn * ny;
                            let keep = (1.0 - (1.0 - p.wall_restitution) * impact) * 0.995;
                            s.vx *= keep;
                            s.vy *= keep;
                            if impact > 0.1 {
                                s.wall_hits += 1;
                            }
                        }
                    } else {
                        s.vh -= GRAVITY * dt;
                        s.h += s.vh * dt;
                        s.air_ticks += 1;
                    }
                }
            }
        }

        // --- track interaction after move ---
        let loc2 = self.geom.locate(s.x, s.y, loc.seg);
        s.seg = loc2.seg as u32;
        let over = loc2.lateral.abs() - loc2.half_width;
        if self.geom.world.is_none() && self.geom.track.walls && over > 0.0 {
            // push back inside; kill the outward normal velocity; lose speed in
            // proportion to how head-on the contact is, plus grinding friction
            let nx = -loc2.dir.sin();
            let ny = loc2.dir.cos();
            let sign = loc2.lateral.signum();
            s.x -= nx * sign * over;
            s.y -= ny * sign * over;
            let vn = s.vx * nx + s.vy * ny;
            if vn * sign > 0.0 {
                let speed = s.speed().max(1e-3);
                let impact = vn.abs() / speed;
                s.vx -= vn * nx;
                s.vy -= vn * ny;
                let keep = (1.0 - (1.0 - p.wall_restitution) * impact) * 0.995;
                s.vx *= keep;
                s.vy *= keep;
                if impact > 0.1 {
                    s.wall_hits += 1;
                }
            }
        }
        s.progress = loc2.progress;
        if s.tick > 150 && s.speed() < 1.0 {
            s.stuck_ticks += 1;
        } else {
            s.stuck_ticks = 0;
        }

        // --- checkpoints and finish by marker pieces (3D worlds) ---
        if self.geom.world.is_some() && !self.geom.markers.is_empty() {
            if let Some(g) = self
                .geom
                .world
                .as_ref()
                .and_then(|w| w.sample(s.x, s.y, s.h, 2.5))
            {
                if s.air_ticks == 0 {
                    if let Some(&(_, kind)) = self.geom.markers.iter().find(|m| m.0 == g.piece) {
                        let idx = self
                            .geom
                            .markers
                            .iter()
                            .filter(|m| m.1 == 1)
                            .position(|m| m.0 == g.piece);
                        match kind {
                            1 => {
                                if let Some(i) = idx {
                                    if i < 64 && s.cp_mask & (1 << i) == 0 {
                                        s.cp_mask |= 1 << i;
                                        s.next_checkpoint += 1;
                                    }
                                }
                            }
                            2 => {
                                let need = self.geom.n_checkpoint_pieces().min(64) as u32;
                                if s.next_checkpoint >= need {
                                    s.finished = true;
                                    s.finish_tick = s.tick;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            return;
        }

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
        if s.finished || s.tick >= max_ticks || s.stuck_ticks >= STUCK_TICKS {
            return true;
        }
        if self.geom.world.is_some()
            && s.air_ticks > 0
            && s.h < self.geom.height_at(s.progress) - FALL_LIMIT_M
        {
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
