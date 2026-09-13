# RTI repository guide (for coding agents, human or model)

RTI = Recursive Trackmania Intelligence: a self-contained automated research
system whose domain is Trackmania. It improves its own simulator, search
methods, models, experiments and tooling. You are editing the code that RTI
itself uses and improves.

## Layout

```
crates/rti-core      shared data model: Track, CarState, Action, Trajectory, PhysicsParams,
                     ExperimentSpec, CostRecord (ledger), Provenance, RtiConfig
crates/rti-sim       replicated physics (sim/). Deterministic, rewindable, batched (rayon).
                     geom.rs (track queries), physics.rs (Sim::step), rollout.rs, observe.rs, tracks.rs
crates/rti-oracle    truth world (oracle/): Oracle trait, HiddenSim stand-in, TM2020 bridge client
crates/rti-search    discovery methods over the sim: random shooting, CEM, CMA-ES, beam, local opt, MAP-Elites, policy
crates/rti-nn        pure-Rust MLP, policy/value nets, behaviour cloning (no GPU, no Python needed)
crates/rti-archive   DuckDB archive + BLAKE3 content-addressed store (data/)
crates/rti-llm       OpenRouter client, role→model routing, cost accounting
crates/rti-agent     coding worker harness: worktree, gates, builtin/external agents
crates/rti-research  the research loop: context → hypothesis → experiment → analysis → findings
crates/rti-maps      TMX client, GBX (.Map.Gbx) decoder, block catalog, map → Track compiler
crates/rti-server    `rti serve`: JSON API, background loop, operator agent, single-page UI
crates/rti-cli       the `rti` binary
tracks/              track definitions (TOML); docs/ design docs; prompts/ role prompts
```

## Invariants (gates enforce most of these)

1. `rti-sim` is a pure function of (state, action, params, track, ext). No randomness, no clocks,
   no global state. `deterministic_replay` and `savestate_resume_matches_full_run` tests must pass.
2. Every physics constant lives in `PhysicsParams` so calibration can fit it. Bump
   `PhysicsParams::VERSION` when the *meaning* of the model changes. Keep `to_vec`/`from_vec`/`names` in sync.
3. `CarState` is the savestate. Add fields at the end, keep `Default` meaningful.
4. `ExperimentSpec` is the LLM-facing schema; changes need matching updates to `schema_doc()` and the
   experiment runner in `rti-research`.
5. Throughput matters: `cargo run --release -p rti-sim --example bench` is a gate (no >15% regression).
6. Trajectory `world` strings are `sim:<params-hash>` or `oracle:<name>`; search experiment results carry
   `finished` and `best_time_ms` so `method_stats` can aggregate them.
7. Nothing in the research loop may read `HiddenSim::truth()`; the oracle is a black box.
8. Map ingestion is best effort and must stay that way: `parse_map` and `compile_track` report
   what they could not decode (`warnings`, `unrecognized`) instead of failing. Extending block
   coverage happens in `catalog.toml` / `catalog::DEFAULT_CATALOG` and `compile::local_geometry`.

## Workflow

- `cargo fmt --all && cargo build --workspace && cargo test --workspace` before finishing.
- Run search tests in release (`cargo test -p rti-search --release`) — they simulate millions of ticks.
- Prefer adding a focused test next to the behaviour you change.
- Do not commit; the harness commits and merges once gates pass.

## Useful commands

```
cargo run --release -p rti-sim --example bench          # throughput
cargo run --release -p rti-sim --example handdrive       # drivability sanity check
cargo run --release -p rti-search --example compare -- hairpin 20000000
cargo run --release -p rti-cli -- --help
```
