//! Discovery methods that exploit the deterministic, rewindable simulator.
//!
//! All continuous methods share one parametrisation (`genome`): `K` control
//! points, each with a steering value and a drive value in [-1, 1]. Steering
//! is linearly interpolated between control points, drive is piecewise
//! constant and decoded to gas/brake. Beam search and local optimisation
//! work on per-tick action sequences directly.

pub mod beam;
pub mod cma;
pub mod common;
pub mod genome;
pub mod local;
pub mod map_elites;
pub mod population;

pub use common::{run, Budget, HistoryPoint, SearchOutcome};
pub use genome::{decode, Genome};
