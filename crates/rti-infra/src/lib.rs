//! Cloud lifecycle for RTI: a persistent coordinator, a stopped-by-default
//! GPU oracle pod (Runpod) started only for verification/rendering,
//! disposable Fly worker machines, S3/R2 artifact sync, hard budgets and an
//! oracle-side shutdown watchdog that is independent of the research agent.
//!
//! Nothing here is required for local operation: with
//! `[cloud] provider_oracle = "none"` every function is a cheap no-op.

pub mod budget;
pub mod fly;
pub mod oracle_lifecycle;
pub mod runpod;
pub mod sync;
pub mod watchdog;
pub mod workers;

pub use budget::{Budget, BudgetState, SpendKind};
pub use fly::{FlyClient, MachineSpec};
pub use oracle_lifecycle::OracleLifecycle;
pub use runpod::{PodStatus, RunpodClient};
pub use workers::{spawn_worker, WorkerHandle};

use rti_core::config::CloudConfig;
use serde::{Deserialize, Serialize};

/// Summary for the CLI and UI.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CloudStatus {
    pub provider_oracle: String,
    pub pod: Option<PodStatus>,
    pub pod_error: Option<String>,
    pub budget: BudgetState,
    pub oracle_hours_today: f64,
    pub oracle_daily_hours_cap: f64,
    pub compute_usd_today: f64,
    pub compute_usd_month: f64,
    pub openrouter_usd_today: f64,
    pub openrouter_usd_month: f64,
}

/// Provider status plus budget usage read from `<data_dir>/cloud_state.json`.
pub fn status(cfg: &CloudConfig, data_dir: &std::path::Path) -> CloudStatus {
    let budget = Budget::open(cfg, data_dir);
    let st = budget.state();
    let mut out = CloudStatus {
        provider_oracle: cfg.provider_oracle.clone(),
        pod: None,
        pod_error: None,
        oracle_hours_today: st.oracle_seconds_today / 3600.0,
        oracle_daily_hours_cap: cfg.oracle_daily_hours,
        compute_usd_today: st.compute_usd_today,
        compute_usd_month: st.compute_usd_month,
        openrouter_usd_today: st.openrouter_usd_today,
        openrouter_usd_month: st.openrouter_usd_month,
        budget: st,
    };
    if cfg.provider_oracle == "runpod" && !cfg.runpod_pod_id.is_empty() {
        match RunpodClient::from_config(cfg).and_then(|c| c.pod_status(&cfg.runpod_pod_id)) {
            Ok(p) => out.pod = Some(p),
            Err(e) => out.pod_error = Some(e.to_string()),
        }
    }
    out
}

pub use oracle_lifecycle::ManagedOracle;
