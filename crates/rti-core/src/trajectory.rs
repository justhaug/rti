use serde::{Deserialize, Serialize};

use crate::{Action, CarState, ContentHash};

/// Outcome summary of one run of an input sequence on a track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct RunResult {
    pub finished: bool,
    /// Race time in ms if finished, else the tick budget consumed.
    pub time_ms: u32,
    pub ticks: u32,
    pub checkpoints_hit: u32,
    /// Track progress reached (meters along centerline).
    pub progress: f32,
    pub max_speed: f32,
    pub mean_speed: f32,
    pub offtrack_ticks: u32,
    pub wall_hits: u32,
}

impl RunResult {
    /// Scalar objective (lower is better): finish time in ms, or a large
    /// penalty scaled by remaining distance when unfinished.
    pub fn objective(&self, track_length: f32) -> f64 {
        if self.finished {
            self.time_ms as f64
        } else {
            let remaining = (track_length - self.progress).max(0.0) as f64;
            1_000_000.0 + remaining * 1000.0 + self.ticks as f64
        }
    }
}

/// An input sequence together with where it came from and what it did.
/// The `states` field is optional (dense telemetry is large) and is what
/// oracle comparisons use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trajectory {
    pub track_hash: ContentHash,
    pub track_name: String,
    /// Hash of the `PhysicsParams` (sim world) or the oracle name.
    pub world: String,
    pub actions: Vec<Action>,
    #[serde(default)]
    pub states: Vec<CarState>,
    pub result: RunResult,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub parent: Option<ContentHash>,
}

impl Trajectory {
    pub fn hash(&self) -> ContentHash {
        ContentHash::of_json(self)
    }

    pub fn without_states(&self) -> Trajectory {
        Trajectory {
            states: Vec::new(),
            ..self.clone()
        }
    }
}

/// Per-tick divergence between two runs of the same inputs in two worlds
/// (sim vs oracle, or sim v1 vs sim v2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Divergence {
    pub ticks_compared: u32,
    pub mean_pos_err: f32,
    pub max_pos_err: f32,
    pub final_pos_err: f32,
    pub mean_speed_err: f32,
    /// First tick where position error exceeded 1 m, if any.
    pub first_tick_over_1m: Option<u32>,
    pub time_ms_a: u32,
    pub time_ms_b: u32,
    pub both_finished: bool,
}

impl Divergence {
    pub fn compute(
        a: &[CarState],
        b: &[CarState],
        ra: &crate::RunResult,
        rb: &crate::RunResult,
    ) -> Divergence {
        let n = a.len().min(b.len());
        let mut sum_pos = 0.0f64;
        let mut max_pos = 0.0f32;
        let mut sum_speed = 0.0f64;
        let mut first = None;
        for i in 0..n {
            let dx = a[i].x - b[i].x;
            let dy = a[i].y - b[i].y;
            let d = (dx * dx + dy * dy).sqrt();
            sum_pos += d as f64;
            if d > max_pos {
                max_pos = d;
            }
            if first.is_none() && d > 1.0 {
                first = Some(a[i].tick);
            }
            sum_speed += (a[i].speed() - b[i].speed()).abs() as f64;
        }
        let final_pos_err = if n > 0 {
            let (p, q) = (a[n - 1], b[n - 1]);
            ((p.x - q.x).powi(2) + (p.y - q.y).powi(2)).sqrt()
        } else {
            0.0
        };
        Divergence {
            ticks_compared: n as u32,
            mean_pos_err: if n > 0 {
                (sum_pos / n as f64) as f32
            } else {
                0.0
            },
            max_pos_err: max_pos,
            final_pos_err,
            mean_speed_err: if n > 0 {
                (sum_speed / n as f64) as f32
            } else {
                0.0
            },
            first_tick_over_1m: first,
            time_ms_a: ra.time_ms,
            time_ms_b: rb.time_ms,
            both_finished: ra.finished && rb.finished,
        }
    }
}
