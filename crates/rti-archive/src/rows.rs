//! Row types returned by `Archive` queries.

use rti_core::CostRecord;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackRow {
    pub hash: String,
    pub name: String,
    pub length_m: f64,
    pub n_nodes: i64,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PhysicsRow {
    pub hash: String,
    pub label: String,
    pub version: i64,
    pub source_experiment: Option<String>,
    pub oracle_mean_err: Option<f64>,
    pub is_current: bool,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExperimentRow {
    pub id: String,
    pub cycle: Option<i64>,
    pub kind: String,
    pub method: Option<String>,
    pub track: Option<String>,
    pub spec: serde_json::Value,
    pub hypothesis_id: Option<String>,
    pub status: String,
    pub result: Option<serde_json::Value>,
    pub summary: Option<String>,
    pub cost: Option<CostRecord>,
    pub value: Option<serde_json::Value>,
    pub value_total: Option<f64>,
    pub provenance: serde_json::Value,
    pub started_at: String,
    pub finished_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrajectoryRow {
    pub hash: String,
    pub track_name: String,
    pub track_hash: String,
    pub world: String,
    pub method: Option<String>,
    pub experiment_id: Option<String>,
    pub finished: bool,
    pub time_ms: u32,
    pub progress: f64,
    pub ticks: u64,
    pub has_states: bool,
    pub parent: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerificationRow {
    pub id: String,
    pub trajectory_hash: String,
    pub oracle: String,
    pub oracle_trajectory_hash: Option<String>,
    pub experiment_id: Option<String>,
    pub divergence: serde_json::Value,
    pub mean_pos_err: f64,
    pub max_pos_err: f64,
    pub time_ms_sim: u32,
    pub time_ms_oracle: u32,
    pub both_finished: bool,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct VerificationStats {
    pub n: u64,
    pub mean_pos_err: f64,
    pub mean_abs_time_diff_ms: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelRow {
    pub hash: String,
    pub kind: String,
    pub experiment_id: Option<String>,
    pub meta: serde_json::Value,
    pub metrics: serde_json::Value,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Finding {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub cycle: Option<i64>,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub confidence: f32,
    #[serde(default)]
    pub experiment_ids: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HypothesisRow {
    pub id: String,
    pub cycle: Option<i64>,
    pub text: String,
    pub rationale: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmCallRow {
    pub id: String,
    pub role: String,
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub usd: f64,
    pub latency_ms: u64,
    pub cycle: Option<i64>,
    pub experiment_id: Option<String>,
    pub ok: bool,
    pub error: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmModelStats {
    pub model: String,
    pub n: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub usd: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TaskRow {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub cycle: Option<i64>,
    pub kind: String,
    pub title: String,
    pub description: String,
    #[serde(default = "default_pending")]
    pub status: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub result_json: Option<serde_json::Value>,
    #[serde(default)]
    pub cost: Option<CostRecord>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

fn default_pending() -> String {
    "pending".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CycleRow {
    pub id: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub summary: Option<String>,
    pub cost: Option<CostRecord>,
    pub brain: String,
    pub model: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MethodStats {
    pub method: String,
    pub track: String,
    pub n_runs: u64,
    pub finished_runs: u64,
    pub best_time_ms: Option<u32>,
    pub sim_ticks: u64,
    pub wall_ms: u64,
    pub llm_usd: f64,
}

impl TaskRow {
    pub fn new(
        kind: &str,
        title: &str,
        description: &str,
        priority: i32,
        cycle: Option<i64>,
    ) -> TaskRow {
        TaskRow {
            id: String::new(),
            cycle,
            kind: kind.into(),
            title: title.into(),
            description: description.into(),
            status: "pending".into(),
            priority,
            branch: None,
            result_json: None,
            cost: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}
