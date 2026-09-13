//! Shared vocabulary for RTI: tracks, car states, actions, trajectories,
//! physics parameters, experiment specs, provenance and the compute ledger.
//!
//! Everything in this crate is plain data with serde derives so that it can be
//! archived, hashed, and handed to LLM roles as JSON.

pub mod action;
pub mod config;
pub mod experiment;
pub mod hash;
pub mod ledger;
pub mod params;
pub mod provenance;
pub mod state;
pub mod track;
pub mod trajectory;
pub mod value;

pub use action::Action;
pub use config::RtiConfig;
pub use experiment::{ExperimentSpec, SearchMethod};
pub use hash::ContentHash;
pub use ledger::CostRecord;
pub use params::PhysicsParams;
pub use provenance::Provenance;
pub use state::CarState;
pub use track::{Surface, Track};
pub use trajectory::{RunResult, Trajectory};
pub use value::ValueScore;

/// Simulation tick length. TM2020 physics runs at 100 Hz.
pub const TICK_SECONDS: f32 = 0.01;
pub const TICK_MS: u32 = 10;
