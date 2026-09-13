//! OpenRouter client with role-based model routing, cost accounting and
//! budget enforcement. RTI must never structurally depend on one frontier
//! model: callers ask for a *role* ("researcher", "coder", "critic",
//! "escalation") and the config decides which model serves it.

pub mod client;
pub mod json;
pub mod types;

pub use client::{ChatOptions, LlmClient, UsageSink};
pub use json::extract_json;
pub use types::{ChatResponse, Message, ToolCall, ToolDef, Usage};
