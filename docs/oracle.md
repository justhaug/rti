# The oracle: TM2020 as ground truth

RTI has two physics worlds. `sim/` (crate `rti-sim`) is the replicated model that runs at
~100 M ticks/s. `oracle/` (crate `rti-oracle`) is the authoritative world. Interesting sim
results are replayed in the oracle; disagreement becomes a simulator-improvement task.

Two oracle implementations exist:

| kind         | what it is                                                                | use                       |
|--------------|---------------------------------------------------------------------------|---------------------------|
| `hidden_sim` | the replicated sim with perturbed constants **and** unmodelled effects    | default; fully offline    |
| `tm2020`     | the real game via an Openplanet bridge plugin (JSON lines over TCP)       | when the game is running  |

Select with `[oracle] kind = "..."` in `rti.toml`.

## hidden_sim

`HiddenSim::from_seed` builds a world whose `PhysicsParams` are perturbed ±12% and which adds
effects the public model does not have (`rti_sim::Ext`): aerodynamic downforce, extra steering
lag, a gear-shift power dip, and grip loss at high yaw rate. Nothing in the research loop may
read those values (`HiddenSim::truth()` is test-only). Calibration can fit the perturbed
constants; it cannot fit the missing effects — that is the point. When calibration plateaus
the loop queues a coding task to extend the model, exactly as it would with the real game.

## tm2020 bridge contract

RTI is the TCP **client**. The game side is a plugin (Openplanet, running in TM2020 under
Proton) that listens on `[oracle] tm2020_host:tm2020_port` (default `127.0.0.1:27015`) and
speaks newline-delimited JSON. Every request gets exactly one response line.

A reference plugin lives in `deploy/openplanet/RTIBridge/` (`deploy/tm2020-local.sh` installs it
into the prefix). It implements `hello`, `ping`, `state`, `load_map` (by file in the game's Maps
folder) and `capture` (telemetry of whatever is driving). It does **not** implement `run`:
Openplanet has no supported input injection, so replaying RTI's inputs in the game needs a
TAS/TMInterface-style tool. Until then, verification against the real game is capture-based
(drive or ghost-play the line, compare telemetry), and `Tm2020::run` returns the bridge's error.

Requests (`crates/rti-oracle/src/protocol.rs`):

```jsonc
{"cmd":"hello","protocol":1}
{"cmd":"ping"}
{"cmd":"load_map","uid":"<map uid>","file":"RTI/<track>.Map.Gbx"}   // file relative to the Maps folder
{"cmd":"state"}                                                       // current car state
{"cmd":"capture","max_ticks":6000}                                    // telemetry of the current drive
{"cmd":"run","inputs":[{"action":{"steer":-0.5,"gas":true,"brake":false},"ticks":120}, ...],
 "max_ticks":12000,"telemetry":true}
```

Responses:

```jsonc
{"ok":true,"protocol":1,"game":"TM2020"}                    // hello
{"ok":true}                                                  // ping / load_map
{"ok":true,
 "result":{"finished":true,"race_time_ms":23450,"ticks":2345,"checkpoints":3},
 "states":[{"tick":0,"pos":[x,y,z],"vel":[vx,vy,vz],"yaw":0.0,"speed_kmh":0,"cp":0,
            "finished":false,"race_time_ms":0}, ...]}          // run
{"ok":false,"error":"..."}
```

Semantics the plugin must provide:

* `run` restarts the race deterministically (reset to start, wait for the countdown), then
  applies the inputs at 100 Hz race ticks in order, holding each for `ticks` ticks. Inputs are
  the TAS-style run-length encoding produced by `rti_core::action::compress_actions`.
* Telemetry is one entry per tick (tick 0 = start state). `pos` is world space, y up. `yaw`
  is the car's heading in the x/z plane in radians.
* Determinism: the same inputs must reproduce the same telemetry (TM2020 is deterministic
  for a given map and inputs when the run is started from a clean reset).

RTI projects world coordinates into a track's 2D frame with `Track.tm_frame`
(`{origin:[x,y,z], yaw}`); a track that should be verified in the game needs `tm_map_uid`
and `tm_frame` set. The mock bridge in `crates/rti-oracle/tests/oracle.rs` shows a complete
server implementation against the hidden sim — a good reference while writing the plugin.

## Cost accounting

Every oracle run is recorded in the ledger as `oracle_ticks`. The real game runs at 100 ticks/s
(10 ms per tick); the hidden sim reports 0.2 µs/tick. The research loop uses
`Oracle::ms_per_tick` to decide when verification is worth it.

## Replays: the human input channel that needs no plugin

TM2020 autosaves a `.Replay.Gbx` for every personal best in
`Documents/Trackmania/Replays/Autosaves` (inside the Proton prefix). Those replays contain the
full map and the player's **exact per-tick inputs** (no position samples: the game regenerates
the ghost deterministically). `rti replay import <file>` decodes them (`rti-maps/src/replay.rs`,
format after GBX.NET) and registers the map as a track and the inputs as a trajectory in world
`human:<login>` with the real race time. `rti replay watch --every 10` (and the server loop)
import new autosaves automatically, so driving in the game feeds RTI without Openplanet.

What this gives the research loop today:

* human lines as `warm_start` for local optimisation and CEM;
* a free sim-vs-reality probe: replaying the human inputs in the simulator and comparing the
  finish time and checkpoint splits with the replay's (the summary prints both);
* the map file itself, without TMX.

What it does not give: RTI's own inputs played back in the game (still needs a TAS-style tool),
and per-tick positions (would need the Openplanet bridge in developer mode, i.e. Club access).
