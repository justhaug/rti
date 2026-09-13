//! Declarative media jobs: everything a renderer (game or simulator) needs.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraDef {
    /// Top-down view following the car, north up.
    TopDownChase { pixels_per_meter: f32 },
    /// Whole track in frame, static.
    Overview,
}

impl Default for CameraDef {
    fn default() -> Self {
        CameraDef::TopDownChase {
            pixels_per_meter: 6.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonMode {
    /// Part 1: new run. Part 2: old run with the new run as a ghost.
    #[default]
    Ghost,
    /// New run only.
    Solo,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayDef {
    pub title_card: bool,
    pub hud: bool,
    pub difference_card: bool,
    /// Slow-motion replay of the section where most time was gained.
    pub why_it_works: bool,
    /// Max seconds per run part before playback is sped up.
    pub max_part_seconds: f32,
}

impl Default for OverlayDef {
    fn default() -> Self {
        OverlayDef {
            title_card: true,
            hud: true,
            difference_card: true,
            why_it_works: true,
            max_part_seconds: 45.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaJob {
    pub id: String,
    pub kind: String,
    pub track_name: String,
    pub map_hash: Option<String>,
    pub new_trajectory: String,
    pub old_trajectory: Option<String>,
    pub camera: CameraDef,
    pub comparison: ComparisonMode,
    pub overlay: OverlayDef,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Labels shown in the HUD/title (e.g. "RTI", "previous WR").
    pub new_label: String,
    pub old_label: String,
}

impl MediaJob {
    pub fn short(track: &str, new_traj: &str, old_traj: Option<&str>) -> MediaJob {
        MediaJob {
            id: uuid::Uuid::new_v4().to_string()[..12].to_string(),
            kind: "wr_short".into(),
            track_name: track.into(),
            map_hash: None,
            new_trajectory: new_traj.into(),
            old_trajectory: old_traj.map(|s| s.to_string()),
            camera: CameraDef::default(),
            comparison: if old_traj.is_some() {
                ComparisonMode::Ghost
            } else {
                ComparisonMode::Solo
            },
            overlay: OverlayDef::default(),
            width: 1080,
            height: 1920,
            fps: 30,
            new_label: "RTI".into(),
            old_label: "previous best".into(),
        }
    }
}
