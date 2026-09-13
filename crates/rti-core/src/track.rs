use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::ContentHash;

/// Driving surface of a track segment. TM2020 blocks are built from a fixed
/// set of materials with very different handling; each gets its own grip,
/// drive and drag multipliers in `PhysicsParams`, so calibration can fit
/// them independently.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// Concrete / tech road: the reference surface.
    #[default]
    Asphalt,
    Dirt,
    Grass,
    Ice,
    /// Bumpy road: full grip, but the surface throws the car around.
    Bump,
    /// Plastic platform: very high grip, springy.
    Plastic,
    /// Water: heavy drag, little grip.
    Water,
    Sand,
    Snow,
    /// Track walls and metal structures (wall rides).
    Metal,
    /// Penalty surface ("sausage"): kills speed.
    Penalty,
}

/// Number of distinct surfaces; `PhysicsParams` holds one entry per surface.
pub const N_SURFACES: usize = 11;

impl Surface {
    pub fn index(self) -> usize {
        match self {
            Surface::Asphalt => 0,
            Surface::Dirt => 1,
            Surface::Grass => 2,
            Surface::Ice => 3,
            Surface::Bump => 4,
            Surface::Plastic => 5,
            Surface::Water => 6,
            Surface::Sand => 7,
            Surface::Snow => 8,
            Surface::Metal => 9,
            Surface::Penalty => 10,
        }
    }

    pub fn from_index(i: usize) -> Surface {
        Surface::ALL[i.min(N_SURFACES - 1)]
    }

    pub const ALL: [Surface; N_SURFACES] = [
        Surface::Asphalt,
        Surface::Dirt,
        Surface::Grass,
        Surface::Ice,
        Surface::Bump,
        Surface::Plastic,
        Surface::Water,
        Surface::Sand,
        Surface::Snow,
        Surface::Metal,
        Surface::Penalty,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Surface::Asphalt => "asphalt",
            Surface::Dirt => "dirt",
            Surface::Grass => "grass",
            Surface::Ice => "ice",
            Surface::Bump => "bump",
            Surface::Plastic => "plastic",
            Surface::Water => "water",
            Surface::Sand => "sand",
            Surface::Snow => "snow",
            Surface::Metal => "metal",
            Surface::Penalty => "penalty",
        }
    }
}

/// One node of the track centerline polyline.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackNode {
    pub x: f32,
    pub y: f32,
    /// Surface height at this node (metres, up). 0 for planar tracks.
    #[serde(default)]
    pub h: f32,
    /// Half-width of the drivable surface at this node (meters).
    pub half_width: f32,
    #[serde(default)]
    pub surface: Surface,
}

/// A track is a 2D centerline polyline with widths, a start pose, ordered
/// checkpoints (as node indices) and a finish node. Real TM2020 tracks are
/// 3D block maps; the projected-centerline representation is deliberately
/// simple so that the first simulator is tractable. Improving it is a
/// research task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub nodes: Vec<TrackNode>,
    /// Node indices that must be crossed in order before the finish.
    #[serde(default)]
    pub checkpoints: Vec<usize>,
    /// Node index of the finish line (defaults to the last node).
    #[serde(default)]
    pub finish: Option<usize>,
    /// Maximum race length in ticks before a run is declared DNF.
    #[serde(default = "default_max_ticks")]
    pub max_ticks: u32,
    /// Optional TM2020 map identifier (UID) for oracle verification.
    #[serde(default)]
    pub tm_map_uid: Option<String>,
    /// Content hash of the serialised 3D world (`rti_sim::World`) in the CAS,
    /// when the track was compiled from a real map.
    #[serde(default)]
    pub world_hash: Option<String>,
    /// Marker pieces in that world: (piece id, kind) with 1 = checkpoint, 2 = finish.
    #[serde(default)]
    pub markers: Vec<(u32, u8)>,
    /// Map file path relative to the game's Maps folder, for the bridge's
    /// `load_map` (e.g. "RTI/tmx356566.Map.Gbx").
    #[serde(default)]
    pub tm_map_file: Option<String>,
    /// Transform from TM2020 world coordinates to this track's 2D frame
    /// (see docs/oracle.md). Only needed for real-game verification.
    #[serde(default)]
    pub tm_frame: Option<TmFrame>,
    /// If true the track edge is a wall (car is clamped, loses speed).
    /// If false the car may leave the surface onto low-grip ground and is
    /// declared DNF beyond three half-widths.
    #[serde(default = "default_true")]
    pub walls: bool,
}

fn default_true() -> bool {
    true
}

/// Rigid transform mapping TM2020 world coordinates (x, y-up, z) onto the
/// track's planar frame: `local = R(-yaw) * ((x, z) - origin_xz)`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct TmFrame {
    pub origin: [f32; 3],
    /// Rotation (radians) of the track's +x axis in the TM x/z plane.
    pub yaw: f32,
}

impl TmFrame {
    pub fn to_local(&self, p: [f32; 3]) -> (f32, f32) {
        let dx = p[0] - self.origin[0];
        let dz = p[2] - self.origin[2];
        let (c, s) = (self.yaw.cos(), self.yaw.sin());
        (c * dx + s * dz, -s * dx + c * dz)
    }
}

fn default_max_ticks() -> u32 {
    12_000 // 120 s
}

impl Track {
    pub fn load(path: &Path) -> anyhow::Result<Track> {
        let text = std::fs::read_to_string(path)?;
        let track: Track = if path.extension().map(|e| e == "json").unwrap_or(false) {
            serde_json::from_str(&text)?
        } else {
            toml::from_str(&text)?
        };
        track.validate()?;
        Ok(track)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.nodes.len() >= 2, "track needs at least two nodes");
        for (i, n) in self.nodes.iter().enumerate() {
            anyhow::ensure!(n.half_width > 0.5, "node {i} has non-positive width");
        }
        for &c in &self.checkpoints {
            anyhow::ensure!(c < self.nodes.len(), "checkpoint {c} out of range");
        }
        if let Some(f) = self.finish {
            anyhow::ensure!(f < self.nodes.len(), "finish {f} out of range");
        }
        Ok(())
    }

    pub fn finish_node(&self) -> usize {
        self.finish.unwrap_or(self.nodes.len() - 1)
    }

    pub fn hash(&self) -> ContentHash {
        ContentHash::of_json(self)
    }

    /// Total centerline length in meters.
    pub fn length(&self) -> f32 {
        self.nodes
            .windows(2)
            .map(|w| ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt())
            .sum()
    }

    pub fn start_heading(&self) -> f32 {
        let a = self.nodes[0];
        let b = self.nodes[1];
        (b.y - a.y).atan2(b.x - a.x)
    }

    /// Procedural track builder used by tests and by the research loop when it
    /// wants fresh environments. `turns` is a list of (arc_degrees, radius,
    /// surface) segments; positive degrees turn left.
    pub fn from_segments(name: &str, half_width: f32, segments: &[(f32, f32, Surface)]) -> Track {
        let mut nodes = vec![TrackNode {
            x: 0.0,
            y: 0.0,
            h: 0.0,
            half_width,
            surface: Surface::Asphalt,
        }];
        let mut x = 0.0f32;
        let mut y = 0.0f32;
        let mut heading = 0.0f32;
        for &(deg, radius, surface) in segments {
            if deg.abs() < 1e-3 {
                // straight of length `radius`
                let steps = (radius / 8.0).ceil().max(1.0) as usize;
                for _ in 0..steps {
                    x += heading.cos() * radius / steps as f32;
                    y += heading.sin() * radius / steps as f32;
                    nodes.push(TrackNode {
                        x,
                        y,
                        h: 0.0,
                        half_width,
                        surface,
                    });
                }
            } else {
                let total = deg.to_radians();
                let steps = ((deg.abs() / 6.0).ceil() as usize).max(2);
                let d = total / steps as f32;
                for _ in 0..steps {
                    heading += d;
                    x += heading.cos() * radius * d.abs();
                    y += heading.sin() * radius * d.abs();
                    nodes.push(TrackNode {
                        x,
                        y,
                        h: 0.0,
                        half_width,
                        surface,
                    });
                }
            }
        }
        Track {
            name: name.to_string(),
            description: String::new(),
            nodes,
            checkpoints: vec![],
            finish: None,
            max_ticks: default_max_ticks(),
            tm_map_uid: None,
            world_hash: None,
            markers: Vec::new(),
            tm_map_file: None,
            tm_frame: None,
            walls: true,
        }
    }
}
