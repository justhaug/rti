# RTI architecture

RTI (Recursive Trackmania Intelligence) is a self-contained automated research system whose
domain is Trackmania. It recursively improves its simulator, search methods, models,
experiments and its own research tooling. This document is the map; `AGENTS.md` is the
guide for anyone (human or model) editing the code.

## Principles

1. **One codebase, Linux-native, Rust.** No GPU or Python required. Neural nets are a small
   pure-Rust MLP; brute-force intelligence comes from batched deterministic simulation.
2. **Two physics worlds.** `sim/` for throughput, `oracle/` for truth. Verification is an
   experiment like any other; disagreement is a research task.
3. **Persistent scientific memory.** DuckDB is the structured archive; BLAKE3
   content-addressed files hold trajectories, models, datasets; git holds code lineage.
   Every experiment records exact provenance (commit, params hash, track hash, seed).
4. **Many discovery methods.** Nets provide intuition, explicit search exploits
   rewindable physics. Methods compete on the ledger.
5. **Cheap cognition, rare escalation.** OpenRouter roles (`researcher`, `coder`, `critic`,
   `escalation`) map to models in `rti.toml`. Default: GLM 5.3 Flash everywhere except the
   critic. Frontier models are optional escalation paths, not infrastructure.
6. **Coding is a tool.** Coding tasks run in isolated worktrees and must pass deterministic
   gates before merging.
7. **Ledger from day one.** Sim ticks, oracle ticks, wall/CPU time and LLM cost per
   experiment, so RTI can learn which methods buy verified milliseconds cheapest.

## Crates

```
rti-core      data model, config, experiment schema, ledger, provenance
rti-sim       replicated physics: TrackGeom, Sim::step, rollouts, observations, tracks
rti-oracle    Oracle trait, HiddenSim, TM2020 bridge client + protocol
rti-search    random shooting, CEM, CMA-ES, beam, local opt, MAP-Elites, policy search
rti-nn        MLP/Adam, policy + value nets, behaviour cloning, policy rollouts
rti-archive   DuckDB schema (tracks, physics, experiments, trajectories, verifications,
              models, findings, hypotheses, ledger, llm_calls, tasks, cycles) + CAS
rti-llm       OpenRouter client, role routing, retries/fallbacks, cost accounting, JSON mode
rti-agent     coding harness: worktree, builtin tool-loop agent or external command, gates
rti-research  Session, context builder, brains (LLM / scripted), experiment runner, cycle, tasks
rti-maps      Trackmania Exchange client, GBX decoder, block catalog, map → Track compiler
rti-server    long-running process: JSON API, background loop, operator agent, UI
rti-media     detect → compare → render → compose → narrate → publish (docs/media.md)
rti-infra     Fly/Runpod/R2 lifecycle, budget ledger, managed oracle, watchdog (docs/deploy.md)
rti-cli       `rti` binary
```

## The central loop (`rti research`)

```
read accumulated knowledge        context::build_context  (archive → markdown)
select uncertainty / opportunity  Brain::propose          (researcher role, JSON)
form hypothesis + design          Proposal{hypothesis, prediction, ExperimentSpec}
critic pass                       Brain::critique         (accept / revise / reject)
run millions of sim ticks         runner::run_experiment
analyze + archive                 Brain::conclude → Finding, ValueScore, ledger
verification where warranted      follow-up tasks: verify / calibrate / coding
train / improve                   ExperimentSpec::TrainBc, coding tasks
repeat
```

`ExperimentSpec` is the fixed, executable vocabulary the researcher chooses from:
`search`, `verify`, `calibrate`, `train_bc`, `benchmark`, `generate_track`, `import_map`. Anything
that needs new code becomes a coding task instead.

## Processes

`rti serve` is the intended long-running deployment: one process owns the DuckDB archive
(single writer), runs the research loop in a background thread, executes queued tasks, and
exposes everything over HTTP (`docs/maps.md` and `crates/rti-server/src/api.rs` list the
endpoints). The operator agent in the UI is an LLM tool loop over the same operations the CLI
offers, so a person can direct the researcher ("focus on ice maps", "verify that trajectory",
"import this TMX map") and inspect the archive while it runs. The CLI is for one-off runs and
scripting against the same data directory when the server is not running.

## Value function

```
V = race_improvement + information_gain + novelty + sim_accuracy + efficiency
```

Weights live in `[value]`. Components are normalised to [0, 1] per experiment
(`rti-research/src/value.rs`); `efficiency` divides the other four by cost (Mticks and $).
Every experiment row stores the decomposition and total so the researcher can see what has
paid off.

## Simulator

Planar bicycle model with a brake-priority friction circle, surface grip multipliers, steering
slew and speed-dependent lock, drive engagement, walls (impact-proportional speed loss) or
open edges, checkpoints and finish. All constants are in `PhysicsParams`. Tracks are 2D
centerline polylines with widths and surfaces; `TrackGeom` precomputes segment tables for
fast localisation. Throughput on this machine: ~7 M ticks/s per core, ~100 M ticks/s batched.

This is a *first* model. Its whole purpose is to be wrong in ways the oracle reveals.

## Search parametrisation

Continuous methods (random, CEM, CMA-ES, MAP-Elites) share a *distance-indexed* genome:
K control points spaced along the centerline, each with steer and drive values; steering is
interpolated, drive is decoded to gas/coast/brake. Beam search works on discrete macro-actions
from savestates with a physics-informed heuristic (progress + speed − overspeed penalty from
`TrackGeom::safe_speed`), or a value net. Local optimisation perturbs per-tick actions.

## Coding harness

`rti-agent` creates `.rti/worktrees/<task>` on branch `rti/<task>`, runs the builtin agent
(read/write/edit/search/bash tools, no git) or an external command
(`[agent] external_command`, e.g. a Pi-style agent), then gates: `cargo fmt --check`,
`cargo build`, `cargo test --workspace`, and the sim benchmark (≤15% regression). Passing
changes are committed and merged into the main checkout; failures keep the branch.

## Data layout

```
data/rti.duckdb            structured archive
data/cas/<xx>/<hash>       trajectories, models, datasets (BLAKE3)
tracks/*.toml              track definitions
.rti/worktrees/            coding-task worktrees (gitignored)
```
