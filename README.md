# RTI — Recursive Trackmania Intelligence

A self-contained automated research system whose universe is Trackmania. RTI owns a fast
replicated physics simulator, learns its laws by comparing against an oracle (the real game or a
stand-in), searches it with many discovery methods, trains models on what it finds, writes better
tools for studying it, and keeps a persistent scientific archive of everything with exact
provenance and cost accounting. A cheap LLM (GLM 5.3 Flash via OpenRouter by default) is the
long-running researcher; coding is just another tool it can invoke.

See `docs/architecture.md` for the design and `AGENTS.md` for the repository guide.

## Quick start

```bash
cargo build --release
export OPENROUTER_API_KEY=...            # optional; without it the scripted brain runs
./target/release/rti init                # writes rti.toml, tracks/, data/
./target/release/rti bench               # sim throughput
./target/release/rti search hairpin --method beam --ticks 20000000
./target/release/rti verify <trajectory-hash>
./target/release/rti research --cycles 5 --with-tasks
./target/release/rti context             # what the researcher sees
./target/release/rti query "select method, track, min(time_ms) from trajectories where finished group by 1,2"
```

Useful examples:

```bash
cargo run --release -p rti-sim --example handdrive              # drivability sanity check
cargo run --release -p rti-search --example compare -- hairpin 20000000
```

## Configuration (`rti.toml`)

`rti init` writes the defaults. Key sections:

* `[llm]` — OpenRouter base URL, `api_key_env`, `roles` (`researcher`, `coder`, `critic`,
  `escalation` → model ids), `fallbacks`.
* `[budget]` — `max_total_usd`, `max_usd_per_cycle`, `max_ticks_per_experiment`, `default_ticks`.
* `[oracle]` — `kind = "hidden_sim" | "tm2020"`, bridge host/port, `disagreement_threshold_m`.
* `[agent]` — `backend = "builtin" | "external"`, `external_command`, gates, benchmark tolerance.
* `[research]` — `brain = "llm" | "scripted"`, critic on/off, verification threshold.
* `[value]` — weights of the value function.

## Status

Bootstrapped harness. Simulator, search, nets, archive, oracle contract, LLM roles, coding
harness and the research loop all run end to end against the hidden-sim oracle. The TM2020
bridge plugin (Openplanet under Proton) is not included; the client and protocol are
(`docs/oracle.md`).
