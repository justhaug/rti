//! Wire protocol between RTI and the TM2020 bridge plugin (Openplanet).
//! Newline-delimited JSON over TCP. RTI is the client. See docs/oracle.md.

use rti_core::action::ActionRun;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Hello {
        protocol: u32,
    },
    /// Load a map by UID (or file path if the plugin supports it).
    LoadMap {
        uid: String,
    },
    /// Reset to the start line, replay the given inputs, return telemetry.
    Run {
        /// Run-length encoded inputs.
        inputs: Vec<ActionRun>,
        max_ticks: u32,
        /// Return per-tick states (otherwise just the result).
        telemetry: bool,
    },
    Ping,
}

/// TM2020 world-frame car state as reported by the plugin.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TmState {
    pub tick: u32,
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    /// Yaw in the x/z plane (radians).
    pub yaw: f32,
    #[serde(default)]
    pub speed_kmh: f32,
    #[serde(default)]
    pub cp: u32,
    #[serde(default)]
    pub finished: bool,
    #[serde(default)]
    pub race_time_ms: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TmResult {
    pub finished: bool,
    pub race_time_ms: u32,
    pub ticks: u32,
    pub checkpoints: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub protocol: Option<u32>,
    #[serde(default)]
    pub game: Option<String>,
    #[serde(default)]
    pub result: Option<TmResult>,
    #[serde(default)]
    pub states: Vec<TmState>,
}

impl Response {
    pub fn ok() -> Response {
        Response {
            ok: true,
            error: None,
            protocol: Some(PROTOCOL_VERSION),
            game: None,
            result: None,
            states: vec![],
        }
    }
    pub fn err(msg: impl Into<String>) -> Response {
        Response {
            ok: false,
            error: Some(msg.into()),
            protocol: None,
            game: None,
            result: None,
            states: vec![],
        }
    }
}
