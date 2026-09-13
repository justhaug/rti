//! `maps/`: ingest real Trackmania maps.
//!
//! ```text
//! TMX map id/URL → download .Map.Gbx → GBX reader (header + LZO body)
//!   → block/item placements → asset catalog → chained centerline → rti_core::Track
//! ```
//!
//! The GBX reader is generic and best-effort: it decodes the container, the
//! CGameCtnChallenge header chunks and the block list, and degrades
//! gracefully on unknown data (returning whatever was parsed with warnings).
//! The catalog (`catalog.rs`, TOML-driven) maps vanilla block names to
//! geometry templates; `compile.rs` chains them from start to finish into
//! the planar track format the simulator understands. Everything the
//! catalog does not know is reported as coverage so RTI can extend it.

pub mod catalog;
pub mod compile;
pub mod container;
pub mod gbx;
pub mod reader;
pub mod replay;
pub mod shape3d;
pub mod tmx;

pub use compile::{compile_track, CompileReport};
pub use gbx::{parse_map, MapBlock, MapInfo, MapItem, ParsedMap};
pub use replay::{parse_replay, ParsedReplay};
pub use tmx::{TmxClient, TmxMap};
