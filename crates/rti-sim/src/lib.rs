//! `sim/`: the replicated TM2020 physics world.
//!
//! Design goals, in order: deterministic, rewindable (a `CarState` is a
//! savestate), batched throughput (millions of ticks per second per core),
//! and *calibratable* (every constant lives in `PhysicsParams`).
//!
//! The model is a planar bicycle model with a friction circle, surface grip
//! multipliers, steering slew, drive engagement and wall/off-track handling.
//! It is intentionally a first approximation: making it agree with the
//! oracle is one of RTI's standing research tasks.

pub mod geom;
pub mod observe;
pub mod physics;
pub mod rollout;
pub mod tracks;
pub mod world;

pub use geom::TrackGeom;
pub use observe::{observe, OBS_DIM};
pub use physics::{Ext, Sim};
pub use rollout::{rollout, rollout_batch, rollout_with, Rollout, StopReason};
pub use world::{Layer, Patch, World};
