use serde::{Deserialize, Serialize};

/// Full dynamic state of the car. This is the savestate: the simulator is a
/// pure function `step(state, action, params, track) -> state`, so cloning
/// this struct is a rewind point.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct CarState {
    pub tick: u32,
    pub x: f32,
    pub y: f32,
    /// Heading angle in radians, 0 = +x.
    pub heading: f32,
    pub vx: f32,
    pub vy: f32,
    pub yaw_rate: f32,
    /// Current steering column position (steering has finite slew rate).
    pub steer_pos: f32,
    /// Engine "gear" proxy: normalized drive state in [0, 1].
    pub drive: f32,
    /// Track progress along centerline in meters (monotone within a lap).
    pub progress: f32,
    /// Index of the next checkpoint to hit.
    pub next_checkpoint: u32,
    pub finished: bool,
    /// Tick at which the finish was crossed (valid when `finished`).
    pub finish_tick: u32,
    /// Accumulated off-track ticks (used for penalties/novelty descriptors).
    pub offtrack_ticks: u32,
    /// Number of wall contacts so far.
    pub wall_hits: u32,
    /// Centerline segment the car was last located on (search hint).
    pub seg: u32,
    /// Consecutive ticks with negligible speed (stuck detection).
    pub stuck_ticks: u32,
    /// Height above the map origin (metres). 0 on planar tracks.
    pub h: f32,
    /// Vertical speed (m/s).
    pub vh: f32,
    /// Ticks spent airborne in the current flight (0 when grounded).
    pub air_ticks: u32,
    /// Bitmask of checkpoint pieces already touched (3D worlds; up to 64).
    pub cp_mask: u64,
    /// Height of the last surface the car was grounded on (3D worlds).
    pub ground_h: f32,
}

impl CarState {
    pub fn speed(&self) -> f32 {
        (self.vx * self.vx + self.vy * self.vy).sqrt()
    }

    /// Forward (longitudinal) speed in the car frame; negative when reversing.
    pub fn forward_speed(&self) -> f32 {
        self.vx * self.heading.cos() + self.vy * self.heading.sin()
    }

    pub fn lateral_speed(&self) -> f32 {
        -self.vx * self.heading.sin() + self.vy * self.heading.cos()
    }

    pub fn time_ms(&self) -> u32 {
        self.tick * crate::TICK_MS
    }

    /// Dense float encoding for datasets / nets. Keep in sync with `DIM`.
    pub const DIM: usize = 10;
    pub fn to_vec(&self) -> [f32; Self::DIM] {
        [
            self.x,
            self.y,
            self.heading,
            self.vx,
            self.vy,
            self.yaw_rate,
            self.steer_pos,
            self.drive,
            self.progress,
            self.next_checkpoint as f32,
        ]
    }
}
