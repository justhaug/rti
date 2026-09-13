//! `media/`: RTI's public research journal.
//!
//! ```text
//! verified discovery → detect (interestingness) → compare (where the time came from)
//!   → render (new run POV, old run + new ghost, difference card, "why it works")
//!   → compose (FFmpeg, 9:16 MP4) → narrate (title / description / explanation)
//!   → review in the UI → publish (YouTube, private first)
//! ```
//!
//! Rendering has two backends: the game itself through the oracle bridge
//! (`Oracle::render`, authoritative footage) and the built-in simulator
//! renderer (`render.rs`), which draws a top-down chase view from archived
//! telemetry and works anywhere. The composition, narration and publishing
//! steps are identical for both.

pub mod compare;
pub mod compose;
pub mod detect;
pub mod font;
pub mod job;
pub mod narration;
pub mod pipeline;
pub mod publish;
pub mod render;

pub use compare::{compare, Comparison};
pub use detect::{scan, Candidate, Interestingness};
pub use job::{CameraDef, ComparisonMode, MediaJob, OverlayDef};
pub use pipeline::{produce, scan_and_produce, MediaConfig};
