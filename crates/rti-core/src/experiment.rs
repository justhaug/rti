use serde::{Deserialize, Serialize};

use crate::ContentHash;

/// Discovery methods available to the research loop. Neural nets provide
/// intuition; explicit search exploits deterministic, rewindable physics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMethod {
    /// Sample random piecewise-constant input sequences; keep the best.
    RandomShooting,
    /// Cross-entropy method over control-point parametrization.
    Cem,
    /// (mu/mu_w, lambda)-CMA-ES over the same parametrization.
    CmaEs,
    /// Beam search over discrete macro-actions from savestates.
    Beam,
    /// Local hill-climbing on an existing trajectory (needs `warm_start`).
    LocalOpt,
    /// MAP-Elites: quality-diversity archive over behaviour descriptors.
    MapElites,
    /// Roll out a trained policy net with exploration noise.
    Policy,
}

impl SearchMethod {
    pub const ALL: [SearchMethod; 7] = [
        SearchMethod::RandomShooting,
        SearchMethod::Cem,
        SearchMethod::CmaEs,
        SearchMethod::Beam,
        SearchMethod::LocalOpt,
        SearchMethod::MapElites,
        SearchMethod::Policy,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            SearchMethod::RandomShooting => "random_shooting",
            SearchMethod::Cem => "cem",
            SearchMethod::CmaEs => "cma_es",
            SearchMethod::Beam => "beam",
            SearchMethod::LocalOpt => "local_opt",
            SearchMethod::MapElites => "map_elites",
            SearchMethod::Policy => "policy",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|m| m.name() == s)
    }
}

/// Method hyper-parameters. Every field is optional; methods fill defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SearchParams {
    /// Population size per generation (CEM/CMA-ES/random/MAP-Elites).
    pub population: Option<usize>,
    /// Fraction of population kept as elites (CEM).
    pub elite_frac: Option<f32>,
    /// Number of control points for piecewise-constant steering.
    pub control_points: Option<usize>,
    /// Initial exploration std-dev in control-point space.
    pub sigma: Option<f32>,
    /// Beam width (beam search).
    pub beam_width: Option<usize>,
    /// Ticks per macro-action (beam / local opt granularity).
    pub macro_ticks: Option<u32>,
    /// Number of quantized steering levels for discrete action sets.
    pub steer_levels: Option<usize>,
    /// Max iterations (generations / local-opt passes).
    pub iters: Option<usize>,
    /// Model hash of a policy net (Policy method) or value net (Beam heuristic).
    pub model: Option<ContentHash>,
    /// Exploration noise for policy rollouts.
    pub noise: Option<f32>,
    /// MAP-Elites grid resolution per descriptor dimension.
    pub grid: Option<usize>,
    /// Maximum race ticks per rollout (defaults to track.max_ticks).
    pub max_ticks: Option<u32>,
}

/// The schema an LLM researcher must emit to run something. Everything here
/// is executable by the experiment runner without any code changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExperimentSpec {
    /// Run a discovery method on a track in the replicated simulator.
    Search {
        track: String,
        method: SearchMethod,
        /// Simulation tick budget (e.g. 20_000_000).
        budget_ticks: u64,
        #[serde(default)]
        seed: u64,
        #[serde(default)]
        params: SearchParams,
        /// Trajectory hash to start from (LocalOpt requires it; others may
        /// seed their population with it).
        #[serde(default)]
        warm_start: Option<ContentHash>,
        /// Physics params hash to use (defaults to the current calibrated set).
        #[serde(default)]
        physics: Option<ContentHash>,
    },
    /// Replay a trajectory's inputs in the oracle and measure divergence.
    Verify { trajectory: ContentHash },
    /// Fit physics params to oracle telemetry. Probes the oracle with
    /// `probe_runs` diverse input sequences (plus any given trajectories),
    /// then searches param space to minimise divergence.
    Calibrate {
        #[serde(default)]
        tracks: Vec<String>,
        #[serde(default)]
        trajectories: Vec<ContentHash>,
        #[serde(default = "default_probe_runs")]
        probe_runs: usize,
        budget_ticks: u64,
        #[serde(default)]
        seed: u64,
        /// Physics params hash to start from.
        #[serde(default)]
        physics: Option<ContentHash>,
    },
    /// Behaviour-clone a policy net from the best archived trajectories.
    TrainBc {
        #[serde(default)]
        tracks: Vec<String>,
        #[serde(default = "default_top_k")]
        top_k: usize,
        #[serde(default = "default_epochs")]
        epochs: usize,
        #[serde(default = "default_hidden")]
        hidden: usize,
        #[serde(default)]
        seed: u64,
        /// Also fit a value head predicting remaining time.
        #[serde(default = "default_true")]
        value_head: bool,
    },
    /// Measure simulator throughput.
    Benchmark {
        #[serde(default = "default_bench_ticks")]
        ticks: u64,
    },
    /// Import a real TM2020 map (Trackmania Exchange id/URL or a local
    /// .Map.Gbx path) and compile it into a simulator track.
    ImportMap {
        /// TMX map id, TMX URL, `tmx:<id>`, or a path to a .Map.Gbx file.
        source: String,
        /// Track name to register (defaults to a slug of the map name).
        #[serde(default)]
        name: Option<String>,
    },
    /// Procedurally generate and register a new track.
    GenerateTrack {
        name: String,
        seed: u64,
        #[serde(default = "default_segments")]
        segments: usize,
        #[serde(default = "default_half_width")]
        half_width: f32,
    },
}

fn default_probe_runs() -> usize {
    16
}
fn default_top_k() -> usize {
    20
}
fn default_epochs() -> usize {
    30
}
fn default_hidden() -> usize {
    64
}
fn default_true() -> bool {
    true
}
fn default_bench_ticks() -> u64 {
    20_000_000
}
fn default_segments() -> usize {
    8
}
fn default_half_width() -> f32 {
    8.0
}

impl ExperimentSpec {
    pub fn kind(&self) -> &'static str {
        match self {
            ExperimentSpec::Search { .. } => "search",
            ExperimentSpec::Verify { .. } => "verify",
            ExperimentSpec::Calibrate { .. } => "calibrate",
            ExperimentSpec::TrainBc { .. } => "train_bc",
            ExperimentSpec::Benchmark { .. } => "benchmark",
            ExperimentSpec::GenerateTrack { .. } => "generate_track",
            ExperimentSpec::ImportMap { .. } => "import_map",
        }
    }

    /// Rough tick budget for planning/accounting.
    pub fn budget_ticks(&self) -> u64 {
        match self {
            ExperimentSpec::Search { budget_ticks, .. } => *budget_ticks,
            ExperimentSpec::Calibrate { budget_ticks, .. } => *budget_ticks,
            ExperimentSpec::Benchmark { ticks } => *ticks,
            _ => 0,
        }
    }

    /// Human-readable JSON schema hint given to LLM roles.
    pub fn schema_doc() -> &'static str {
        r#"ExperimentSpec (JSON object, discriminated by "kind"):
- {"kind":"search","track":<name>,"method":"random_shooting|cem|cma_es|beam|local_opt|map_elites|policy","budget_ticks":<int>,"seed":<int>,"params":{population?,elite_frac?,control_points?,sigma?,beam_width?,macro_ticks?,steer_levels?,iters?,model?,noise?,grid?,max_ticks?},"warm_start":<trajectory hash|null>,"physics":<params hash|null>}
- {"kind":"verify","trajectory":<trajectory hash>}
- {"kind":"calibrate","tracks":[<name>...],"trajectories":[<hash>...],"probe_runs":<int>,"budget_ticks":<int>,"seed":<int>,"physics":<params hash|null>}
- {"kind":"train_bc","tracks":[<name>...],"top_k":<int>,"epochs":<int>,"hidden":<int>,"seed":<int>,"value_head":<bool>}
- {"kind":"benchmark","ticks":<int>}
- {"kind":"generate_track","name":<str>,"seed":<int>,"segments":<int>,"half_width":<float>}
- {"kind":"import_map","source":<TMX id | TMX URL | path to .Map.Gbx>,"name":<track name|null>}  (downloads a real map from trackmania.exchange and compiles it; the compile report says how much of the map was understood)"#
    }
}
