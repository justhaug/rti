use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::value::ValueWeights;

/// Top-level configuration, loaded from `rti.toml` (falls back to defaults).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RtiConfig {
    pub data_dir: PathBuf,
    pub tracks_dir: PathBuf,
    pub llm: LlmConfig,
    pub budget: BudgetConfig,
    pub value: ValueWeights,
    pub oracle: OracleConfig,
    pub agent: AgentConfig,
    pub research: ResearchConfig,
}

impl Default for RtiConfig {
    fn default() -> Self {
        RtiConfig {
            data_dir: PathBuf::from("data"),
            tracks_dir: PathBuf::from("tracks"),
            llm: LlmConfig::default(),
            budget: BudgetConfig::default(),
            value: ValueWeights::default(),
            oracle: OracleConfig::default(),
            agent: AgentConfig::default(),
            research: ResearchConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key_env: String,
    /// role name -> OpenRouter model id
    pub roles: BTreeMap<String, String>,
    /// Fallback models tried in order when a role's model errors.
    pub fallbacks: Vec<String>,
    pub max_tokens: u32,
    pub temperature: f32,
    pub timeout_secs: u64,
    pub max_retries: u32,
    /// Optional HTTP-Referer / X-Title headers OpenRouter uses for rankings.
    pub app_name: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        let mut roles = BTreeMap::new();
        roles.insert("researcher".into(), "z-ai/glm-5.3-flash".into());
        roles.insert("coder".into(), "z-ai/glm-5.3-flash".into());
        roles.insert("critic".into(), "z-ai/glm-5.3".into());
        roles.insert("escalation".into(), "anthropic/claude-opus-5".into());
        LlmConfig {
            base_url: "https://openrouter.ai/api/v1".into(),
            api_key_env: "OPENROUTER_API_KEY".into(),
            roles,
            fallbacks: vec!["deepseek/deepseek-v4-flash".into()],
            max_tokens: 8192,
            temperature: 0.4,
            timeout_secs: 180,
            max_retries: 3,
            app_name: "RTI".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct BudgetConfig {
    /// Hard stop for LLM spend across the lifetime of the archive.
    pub max_total_usd: f64,
    /// Max LLM spend per research cycle.
    pub max_usd_per_cycle: f64,
    /// Max sim ticks a single experiment may request.
    pub max_ticks_per_experiment: u64,
    /// Default ticks the scripted (no-LLM) researcher uses.
    pub default_ticks: u64,
    /// Threads for batched simulation (0 = all cores).
    pub threads: usize,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        BudgetConfig {
            max_total_usd: 20.0,
            max_usd_per_cycle: 0.25,
            max_ticks_per_experiment: 2_000_000_000,
            default_ticks: 20_000_000,
            threads: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct OracleConfig {
    /// "hidden_sim" (stand-in truth world with unmodelled physics) or "tm2020".
    pub kind: String,
    /// Seed of the hidden-sim truth world.
    pub hidden_seed: u64,
    /// TM2020 bridge endpoint (Openplanet plugin, see docs/oracle.md).
    pub tm2020_host: String,
    pub tm2020_port: u16,
    pub tm2020_timeout_secs: u64,
    /// Divergence (mean position error, m) above which sim improvement is
    /// queued as a research task.
    pub disagreement_threshold_m: f32,
}

impl Default for OracleConfig {
    fn default() -> Self {
        OracleConfig {
            kind: "hidden_sim".into(),
            hidden_seed: 7,
            tm2020_host: "127.0.0.1".into(),
            tm2020_port: 27015,
            tm2020_timeout_secs: 120,
            disagreement_threshold_m: 2.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// "builtin" (LLM tool loop) or "external" (spawn `external_command`).
    pub backend: String,
    /// Command template for external coding agents; `{prompt_file}` and
    /// `{workdir}` are substituted. Example: "pi --yes --prompt-file {prompt_file}".
    pub external_command: String,
    pub max_turns: u32,
    pub max_usd_per_task: f64,
    pub command_timeout_secs: u64,
    /// Gates every change must pass before merging into main.
    pub gates: Vec<String>,
    /// Relative throughput regression allowed on the sim benchmark (0.1 = 10%).
    pub max_bench_regression: f64,
    pub auto_merge: bool,
}

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            backend: "builtin".into(),
            external_command: "pi --yes --prompt-file {prompt_file}".into(),
            max_turns: 40,
            max_usd_per_task: 1.0,
            command_timeout_secs: 600,
            gates: vec![
                "cargo fmt --all -- --check".into(),
                "cargo build --workspace".into(),
                "cargo test --workspace".into(),
            ],
            max_bench_regression: 0.15,
            auto_merge: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ResearchConfig {
    /// Use the LLM researcher ("llm") or the scripted fallback ("scripted").
    pub brain: String,
    pub use_critic: bool,
    /// Verify in the oracle whenever a sim result beats the verified best by this many ms.
    pub verify_improvement_ms: u32,
    /// Max coding tasks queued per cycle.
    pub max_coding_tasks_per_cycle: u32,
    /// How many recent experiments/findings to show the researcher.
    pub context_experiments: usize,
    pub context_findings: usize,
}

impl Default for ResearchConfig {
    fn default() -> Self {
        ResearchConfig {
            brain: "llm".into(),
            use_critic: true,
            verify_improvement_ms: 20,
            max_coding_tasks_per_cycle: 1,
            context_experiments: 25,
            context_findings: 20,
        }
    }
}

impl RtiConfig {
    /// Load `rti.toml` from `dir` (or defaults if absent). Relative paths in
    /// the config are resolved against `dir`.
    pub fn load(dir: &Path) -> anyhow::Result<RtiConfig> {
        let path = dir.join("rti.toml");
        let mut cfg: RtiConfig = if path.exists() {
            toml::from_str(&std::fs::read_to_string(&path)?)?
        } else {
            RtiConfig::default()
        };
        if cfg.data_dir.is_relative() {
            cfg.data_dir = dir.join(&cfg.data_dir);
        }
        if cfg.tracks_dir.is_relative() {
            cfg.tracks_dir = dir.join(&cfg.tracks_dir);
        }
        Ok(cfg)
    }

    pub fn model_for(&self, role: &str) -> Option<&str> {
        self.llm.roles.get(role).map(|s| s.as_str())
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }
}
