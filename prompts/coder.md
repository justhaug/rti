(The built-in coder's system prompt lives in crates/rti-agent/src/builtin.rs; this file is the extra
context appended to every coding task. Keep it short; AGENTS.md is also appended.)

You are improving RTI's own tooling. Typical tasks: make the simulator agree with the oracle on an observed
discrepancy, speed up batched rollouts, add a search operator, add instrumentation, fix a bug found by the
research loop. Read the failing/relevant tests first. Verify with `cargo test --workspace` and, for sim
changes, `cargo run --release -p rti-sim --example bench`.
