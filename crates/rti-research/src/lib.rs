//! The research loop:
//!
//! ```text
//! read accumulated knowledge → select valuable uncertainty → form hypothesis
//!   → design experiment → run sim ticks → analyze + archive
//!   → oracle verification where warranted → train / improve → repeat
//! ```
//!
//! `runner` executes `ExperimentSpec`s. `context` summarises the archive for
//! the brain. `brain` is either an LLM (researcher/critic/analyst roles via
//! OpenRouter) or a scripted fallback so the loop works offline. `cycle`
//! ties it together and `tasks` executes queued coding tasks.

pub mod brain;
pub mod context;
pub mod cycle;
pub mod runner;
pub mod session;
pub mod tasks;
pub mod value;

pub use brain::{Brain, Conclusion, Proposal};
pub use cycle::{run_cycle, CycleReport};
pub use runner::{run_experiment, ExperimentReport};
pub use session::Session;
