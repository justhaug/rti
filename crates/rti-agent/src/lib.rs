//! Coding-worker harness: coding capability as a research tool.
//!
//! A `CodingTask` is executed in an isolated git worktree by either the
//! built-in LLM tool-loop agent or an external command (a Pi-like coding
//! agent). Every change then has to pass deterministic gates (fmt, build,
//! tests, benchmark regression) before it is merged into `main`.

pub mod builtin;
pub mod gates;
pub mod task;
pub mod workspace;

pub use gates::{run_gates, GateResult};
pub use task::{run_task, CodingTask, TaskOutcome, TaskStatus};
pub use workspace::Workspace;
