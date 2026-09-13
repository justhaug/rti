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
    pub media: MediaConfig,
    pub cloud: CloudConfig,
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
            media: MediaConfig::default(),
            cloud: CloudConfig::default(),
        }
    }
}

/// Cloud topology: a persistent coordinator (Fly Machine or any VM), a
/// stopped-by-default GPU oracle pod (Runpod) that RTI starts only for
/// verification/rendering, disposable Fly worker machines for heavy CPU
/// experiments, and an S3/R2 bucket for artifacts. All spending is capped.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudConfig {
    /// "none" | "runpod" | "aws" — who hosts the TM2020 oracle.
    pub provider_oracle: String,
    pub runpod_api_key_env: String,
    pub runpod_pod_id: String,
    pub runpod_api_url: String,
    pub fly_api_token_env: String,
    pub fly_api_url: String,
    pub fly_app: String,
    pub fly_worker_image: String,
    pub fly_worker_size: String,
    pub fly_region: String,
    pub s3_bucket: String,
    /// S3-compatible endpoint (Cloudflare R2: https://<account>.r2.cloudflarestorage.com).
    pub s3_endpoint: String,
    pub s3_prefix: String,
    /// Shell template used for object sync; `{src}` and `{dst}` are substituted.
    pub sync_command: String,
    /// Stop the oracle pod after this many minutes without activity.
    pub oracle_idle_shutdown_minutes: u64,
    pub oracle_boot_timeout_secs: u64,
    /// Watchdog HTTP port on the oracle pod (heartbeats / health).
    pub oracle_watchdog_port: u16,
    pub oracle_daily_hours: f64,
    pub max_oracle_instances: u32,
    pub max_parallel_workers: u32,
    pub compute_daily_usd: f64,
    pub compute_monthly_usd: f64,
    pub openrouter_daily_usd: f64,
    pub openrouter_monthly_usd: f64,
    /// Any single spend above this needs a human approval.
    pub require_approval_above_usd: f64,
}

impl Default for CloudConfig {
    fn default() -> Self {
        CloudConfig {
            provider_oracle: "none".into(),
            runpod_api_key_env: "RUNPOD_API_KEY".into(),
            runpod_pod_id: String::new(),
            runpod_api_url: "https://api.runpod.io/graphql".into(),
            fly_api_token_env: "FLY_API_TOKEN".into(),
            fly_api_url: "https://api.machines.dev/v1".into(),
            fly_app: "rti".into(),
            fly_worker_image: "registry.fly.io/rti:latest".into(),
            fly_worker_size: "performance-4x".into(),
            fly_region: "iad".into(),
            s3_bucket: String::new(),
            s3_endpoint: String::new(),
            s3_prefix: "rti".into(),
            sync_command: "rclone sync {src} {dst}".into(),
            oracle_idle_shutdown_minutes: 10,
            oracle_boot_timeout_secs: 600,
            oracle_watchdog_port: 27016,
            oracle_daily_hours: 2.0,
            max_oracle_instances: 1,
            max_parallel_workers: 4,
            compute_daily_usd: 5.0,
            compute_monthly_usd: 75.0,
            openrouter_daily_usd: 2.0,
            openrouter_monthly_usd: 30.0,
            require_approval_above_usd: 10.0,
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
        roles.insert("critic".into(), "z-ai/glm-5.3-flash".into());
        roles.insert("operator".into(), "z-ai/glm-5.3-flash".into());
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
    /// The game's Maps folder (inside the Proton prefix on Linux). Imported
    /// maps are copied to `<maps_dir>/RTI/<track>.Map.Gbx` so the bridge can
    /// load them by file. Empty = don't copy.
    pub tm2020_maps_dir: String,
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
            tm2020_maps_dir: default_maps_dir(),
            disagreement_threshold_m: 2.0,
        }
    }
}

fn default_maps_dir() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let p = format!("{home}/.steam/root/steamapps/compatdata/2225070/pfx/drive_c/users/steamuser/Documents/Trackmania/Maps");
    if std::path::Path::new(&p).is_dir() {
        p
    } else {
        String::new()
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

/// Interestingness weights for media generation:
/// I = w1·WR improvement + w2·technique novelty + w3·leaderboard significance + w4·visual weirdness
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct InterestWeights {
    pub wr_improvement: f64,
    pub technique_novelty: f64,
    pub leaderboard_significance: f64,
    pub visual_weirdness: f64,
    /// Produce media when the weighted score reaches this.
    pub threshold: f64,
}

impl Default for InterestWeights {
    fn default() -> Self {
        InterestWeights {
            wr_improvement: 1.0,
            technique_novelty: 1.0,
            leaderboard_significance: 1.0,
            visual_weirdness: 0.7,
            threshold: 0.6,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct YoutubeConfig {
    pub client_id_env: String,
    pub client_secret_env: String,
    pub refresh_token_env: String,
    /// "private" (default), "unlisted" or "public" for the initial upload.
    pub upload_privacy: String,
    pub category_id: String,
}

impl Default for YoutubeConfig {
    fn default() -> Self {
        YoutubeConfig {
            client_id_env: "YOUTUBE_CLIENT_ID".into(),
            client_secret_env: "YOUTUBE_CLIENT_SECRET".into(),
            refresh_token_env: "YOUTUBE_REFRESH_TOKEN".into(),
            upload_privacy: "private".into(),
            category_id: "20".into(),
        }
    }
}

/// `[media]`: automatic video generation and publishing.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaConfig {
    pub enabled: bool,
    /// Only make videos for oracle-verified runs.
    pub require_verified: bool,
    pub weights: InterestWeights,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub crf: u32,
    pub youtube: YoutubeConfig,
    /// Publish automatically when score ≥ auto_publish_threshold and verified.
    pub auto_publish: bool,
    pub auto_publish_threshold: f64,
}

impl Default for MediaConfig {
    fn default() -> Self {
        MediaConfig {
            enabled: true,
            require_verified: true,
            weights: InterestWeights::default(),
            width: 1080,
            height: 1920,
            fps: 30,
            crf: 23,
            youtube: YoutubeConfig::default(),
            auto_publish: false,
            auto_publish_threshold: 1.5,
        }
    }
}
