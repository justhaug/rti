//! Small pure-Rust neural-net toolkit for RTI: an MLP with manual backprop
//! and Adam, policy/value nets over `rti_sim::observe` features, behaviour
//! cloning from archived trajectories, and policy rollouts.
//!
//! Deliberately dependency-free (no GPU, no Python on the research box).
//! Improving this (better architectures, RL, faster training) is a
//! research task for RTI's coding worker.

pub mod bc;
pub mod mlp;
pub mod policy;
pub mod rollout;

pub use bc::{behavior_clone, build_dataset, BcConfig, BcMetrics, Dataset};
pub use mlp::{Activation, Loss, Mlp, TrainConfig};
pub use policy::{Policy, PolicyBundle, ValueNet};
pub use rollout::{policy_rollout, policy_rollout_batch};
