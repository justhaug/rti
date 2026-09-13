use rti_core::{Surface, Track};

/// Precomputed centerline geometry for fast queries.
#[derive(Clone, Debug)]
pub struct TrackGeom {
    pub track: Track,
    /// 3D drivable surface (None = planar track).
    pub world: Option<std::sync::Arc<crate::world::World>>,
    /// node heights
    pub hs: Vec<f32>,
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
    /// cumulative distance at each node
    pub cum: Vec<f32>,
    /// per-segment unit direction and length
    pub dx: Vec<f32>,
    pub dy: Vec<f32>,
    pub len: Vec<f32>,
    /// half width per node
    pub hw: Vec<f32>,
    pub surface: Vec<Surface>,
    pub total_len: f32,
    pub checkpoint_dist: Vec<f32>,
    pub finish_dist: f32,
}

/// Result of locating a point on the track.
#[derive(Clone, Copy, Debug, Default)]
pub struct Located {
    pub seg: usize,
    /// Fraction along the segment.
    pub t: f32,
    /// Distance along the centerline.
    pub progress: f32,
    /// Signed lateral offset (positive = left of travel direction).
    pub lateral: f32,
    pub half_width: f32,
    pub surface: Surface,
    /// Direction of the centerline here (radians).
    pub dir: f32,
}

impl TrackGeom {
    pub fn new(track: Track) -> TrackGeom {
        let n = track.nodes.len();
        let xs: Vec<f32> = track.nodes.iter().map(|n| n.x).collect();
        let ys: Vec<f32> = track.nodes.iter().map(|n| n.y).collect();
        let mut cum = vec![0.0f32; n];
        let mut dx = vec![0.0f32; n - 1];
        let mut dy = vec![0.0f32; n - 1];
        let mut len = vec![0.0f32; n - 1];
        for i in 0..n - 1 {
            let ddx = xs[i + 1] - xs[i];
            let ddy = ys[i + 1] - ys[i];
            let l = (ddx * ddx + ddy * ddy).sqrt().max(1e-4);
            dx[i] = ddx / l;
            dy[i] = ddy / l;
            len[i] = l;
            cum[i + 1] = cum[i] + l;
        }
        let hw = track.nodes.iter().map(|n| n.half_width).collect();
        let hs = track.nodes.iter().map(|n| n.h).collect();
        let surface = track.nodes.iter().map(|n| n.surface).collect();
        let total_len = cum[n - 1];
        let checkpoint_dist = track.checkpoints.iter().map(|&c| cum[c]).collect();
        let finish_dist = cum[track.finish_node()];
        TrackGeom {
            world: None,
            hs,
            track,
            xs,
            ys,
            cum,
            dx,
            dy,
            len,
            hw,
            surface,
            total_len,
            checkpoint_dist,
            finish_dist,
        }
    }

    pub fn with_world(mut self, world: crate::world::World) -> TrackGeom {
        if !world.is_empty() {
            self.world = Some(std::sync::Arc::new(world));
        }
        self
    }

    /// Surface height at centerline distance `d`.
    pub fn height_at(&self, d: f32) -> f32 {
        let s = self.seg_at(d);
        let t = ((d - self.cum[s]) / self.len[s]).clamp(0.0, 1.0);
        self.hs[s] * (1.0 - t) + self.hs[s + 1] * t
    }

    pub fn n_segs(&self) -> usize {
        self.len.len()
    }

    #[inline]
    fn project(&self, seg: usize, x: f32, y: f32) -> (f32, f32) {
        // returns (t, squared distance)
        let px = x - self.xs[seg];
        let py = y - self.ys[seg];
        let t = ((px * self.dx[seg] + py * self.dy[seg]) / self.len[seg]).clamp(0.0, 1.0);
        let cx = self.xs[seg] + self.dx[seg] * self.len[seg] * t;
        let cy = self.ys[seg] + self.dy[seg] * self.len[seg] * t;
        (t, (x - cx).powi(2) + (y - cy).powi(2))
    }

    /// Locate a point using a segment hint (search a window around it; the
    /// car cannot skip many segments in one tick). Falls back to a global
    /// search if the window edge is the best match.
    pub fn locate(&self, x: f32, y: f32, hint: usize) -> Located {
        let n = self.n_segs();
        let hint = hint.min(n - 1);
        let lo = hint.saturating_sub(4);
        let hi = (hint + 6).min(n);
        let mut best = (usize::MAX, 0.0f32, f32::INFINITY);
        for s in lo..hi {
            let (t, d2) = self.project(s, x, y);
            if d2 < best.2 {
                best = (s, t, d2);
            }
        }
        if (best.0 == lo && lo > 0) || (best.0 == hi - 1 && hi < n) {
            for s in 0..n {
                let (t, d2) = self.project(s, x, y);
                if d2 < best.2 {
                    best = (s, t, d2);
                }
            }
        }
        self.located(best.0, best.1, x, y)
    }

    pub fn locate_global(&self, x: f32, y: f32) -> Located {
        let mut best = (0usize, 0.0f32, f32::INFINITY);
        for s in 0..self.n_segs() {
            let (t, d2) = self.project(s, x, y);
            if d2 < best.2 {
                best = (s, t, d2);
            }
        }
        self.located(best.0, best.1, x, y)
    }

    fn located(&self, seg: usize, t: f32, x: f32, y: f32) -> Located {
        let cx = self.xs[seg] + self.dx[seg] * self.len[seg] * t;
        let cy = self.ys[seg] + self.dy[seg] * self.len[seg] * t;
        // left normal = (-dy, dx)
        let lateral = (x - cx) * (-self.dy[seg]) + (y - cy) * self.dx[seg];
        let half_width = self.hw[seg] * (1.0 - t) + self.hw[seg + 1] * t;
        Located {
            seg,
            t,
            progress: self.cum[seg] + self.len[seg] * t,
            lateral,
            half_width,
            surface: self.surface[seg],
            dir: self.dy[seg].atan2(self.dx[seg]),
        }
    }

    /// Segment index containing centerline distance `d`.
    pub fn seg_at(&self, d: f32) -> usize {
        let d = d.clamp(0.0, self.total_len);
        match self.cum.binary_search_by(|c| c.partial_cmp(&d).unwrap()) {
            Ok(i) => i.min(self.n_segs() - 1),
            Err(i) => i.saturating_sub(1).min(self.n_segs() - 1),
        }
    }

    pub fn point_at(&self, d: f32) -> (f32, f32) {
        let s = self.seg_at(d);
        let t = ((d - self.cum[s]) / self.len[s]).clamp(0.0, 1.0);
        (
            self.xs[s] + self.dx[s] * self.len[s] * t,
            self.ys[s] + self.dy[s] * self.len[s] * t,
        )
    }

    pub fn dir_at(&self, d: f32) -> f32 {
        let s = self.seg_at(d);
        self.dy[s].atan2(self.dx[s])
    }

    pub fn half_width_at(&self, d: f32) -> f32 {
        let s = self.seg_at(d);
        let t = ((d - self.cum[s]) / self.len[s]).clamp(0.0, 1.0);
        self.hw[s] * (1.0 - t) + self.hw[s + 1] * t
    }

    pub fn surface_at(&self, d: f32) -> Surface {
        self.surface[self.seg_at(d)]
    }

    /// Signed curvature (1/m) around distance `d`, estimated from heading change.
    pub fn curvature_at(&self, d: f32) -> f32 {
        let h = 5.0;
        let a = self.dir_at((d - h).max(0.0));
        let b = self.dir_at((d + h).min(self.total_len));
        wrap_angle(b - a) / (2.0 * h)
    }
}

#[inline]
pub fn wrap_angle(a: f32) -> f32 {
    let mut a = a % std::f32::consts::TAU;
    if a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    } else if a < -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
}

impl TrackGeom {
    /// Highest speed at centerline distance `d` from which every upcoming
    /// corner within `lookahead` metres can still be taken, assuming
    /// `lat_accel` of cornering grip and `brake_decel` of braking (both
    /// already scaled by the surface). A physics-informed heuristic for
    /// search, not a rule of the world.
    pub fn safe_speed(&self, d: f32, lookahead: f32, lat_accel: f32, brake_decel: f32) -> f32 {
        let mut vmax = f32::INFINITY;
        let mut ahead = 2.0;
        while ahead < lookahead {
            let c = self.curvature_at(d + ahead).abs();
            if c > 1e-4 {
                let vc2 = lat_accel / c;
                let vb = (vc2 + 2.0 * brake_decel * ahead).sqrt();
                vmax = vmax.min(vb);
            }
            ahead += 4.0;
        }
        vmax
    }
}
