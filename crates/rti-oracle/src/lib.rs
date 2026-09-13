//! `oracle/`: the authoritative physics world.
//!
//! Two implementations:
//! * `HiddenSim` – the replicated simulator with perturbed parameters *and*
//!   physics terms the public model does not have (`rti_sim::Ext`). This is
//!   the stand-in truth world so that the whole verify → disagree → improve
//!   loop runs without the game.
//! * `Tm2020` – a client for the real game via an Openplanet bridge plugin
//!   speaking the JSON-lines protocol in `docs/oracle.md`. The plugin itself
//!   (and the Proton/Steam setup) is not part of this crate.

pub mod hidden;
pub mod protocol;
pub mod tm2020;

use rti_core::config::OracleConfig;
use rti_core::trajectory::Divergence;
use rti_core::{Action, CarState, RunResult, Track, Trajectory};
use rti_sim::{rollout, Sim};

/// Telemetry from an oracle run.
#[derive(Clone, Debug)]
pub struct OracleRun {
    pub states: Vec<CarState>,
    pub result: RunResult,
    pub ticks: u64,
}

pub trait Oracle: Send + Sync {
    fn name(&self) -> String;
    /// Cheap connectivity/config check.
    fn available(&self) -> anyhow::Result<()>;
    /// Replay `actions` from the start line on `track`.
    fn run(&self, track: &Track, actions: &[Action], max_ticks: u32) -> anyhow::Result<OracleRun>;
    /// Rough wall-clock cost per simulated tick (for the ledger and for
    /// deciding when verification is worth it). Real-time game = 10 ms/tick.
    fn ms_per_tick(&self) -> f64;
}

pub fn from_config(cfg: &OracleConfig) -> anyhow::Result<Box<dyn Oracle>> {
    match cfg.kind.as_str() {
        "hidden_sim" => Ok(Box::new(hidden::HiddenSim::from_seed(cfg.hidden_seed))),
        "tm2020" => Ok(Box::new(tm2020::Tm2020::new(
            &cfg.tm2020_host,
            cfg.tm2020_port,
            cfg.tm2020_timeout_secs,
        ))),
        other => anyhow::bail!("unknown oracle kind {other:?} (expected hidden_sim|tm2020)"),
    }
}

/// Outcome of verifying a sim trajectory against the oracle.
#[derive(Clone, Debug)]
pub struct Verification {
    pub divergence: Divergence,
    pub oracle_trajectory: Trajectory,
    pub oracle_ticks: u64,
}

/// Replay `traj.actions` in both the given sim and the oracle and compare.
pub fn verify(oracle: &dyn Oracle, sim: &Sim, traj: &Trajectory) -> anyhow::Result<Verification> {
    let max_ticks = sim.geom.track.max_ticks;
    let sim_run = rollout(sim, &sim.initial_state(), &traj.actions, max_ticks, true);
    let orun = oracle.run(&sim.geom.track, &traj.actions, max_ticks)?;
    let divergence =
        Divergence::compute(&sim_run.states, &orun.states, &sim_run.result, &orun.result);
    let oracle_trajectory = Trajectory {
        track_hash: sim.geom.track.hash(),
        track_name: sim.geom.track.name.clone(),
        world: format!("oracle:{}", oracle.name()),
        actions: traj.actions.clone(),
        states: orun.states,
        result: orun.result,
        method: format!("verify:{}", traj.method),
        parent: Some(traj.hash()),
    };
    Ok(Verification {
        divergence,
        oracle_trajectory,
        oracle_ticks: orun.ticks,
    })
}
