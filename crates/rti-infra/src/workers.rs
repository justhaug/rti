//! Disposable Fly worker machines for heavy CPU experiments.
//!
//! The worker image must contain the `rti` binary (the `deploy/Dockerfile`
//! image works). A worker runs e.g. `rti --root /work experiment @spec.json`
//! against a scratch data dir, then `rti --root /work cloud sync --push` so
//! results land in the bucket; the coordinator pulls them with
//! `rti cloud sync --pull`. The machine auto-destroys on exit.

use rti_core::config::CloudConfig;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

use crate::budget::{Budget, SpendDecision, SpendKind};
use crate::fly::{FlyClient, Machine, MachineSpec};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerHandle {
    pub app: String,
    pub machine_id: String,
    pub name: String,
}

/// Rough hourly price of a Fly performance-Nx machine (USD), for budgeting.
pub fn estimate_hourly_usd(size: &str) -> f64 {
    let n: f64 = size
        .rsplit('-')
        .next()
        .and_then(|s| s.trim_end_matches('x').parse().ok())
        .unwrap_or(1.0);
    if size.starts_with("performance") {
        0.045 * n
    } else {
        0.01 * n
    }
}

pub fn spawn_worker(
    cfg: &CloudConfig,
    budget: &Budget,
    cmd: &[String],
    env: BTreeMap<String, String>,
    expected_hours: f64,
) -> anyhow::Result<WorkerHandle> {
    let fly = FlyClient::from_config(cfg)?;
    let running = fly
        .list_machines(&cfg.fly_app)?
        .iter()
        .filter(|m| {
            m.name.starts_with("rti-worker-") && m.state != "destroyed" && m.state != "stopped"
        })
        .count() as u32;
    anyhow::ensure!(
        running < cfg.max_parallel_workers,
        "max_parallel_workers ({}) reached",
        cfg.max_parallel_workers
    );
    let usd = estimate_hourly_usd(&cfg.fly_worker_size) * expected_hours.max(0.05);
    match budget.can_spend(SpendKind::Compute, usd) {
        SpendDecision::Ok => {}
        SpendDecision::Denied(m) | SpendDecision::NeedsApproval(m) => {
            anyhow::bail!("worker spawn refused: {m}")
        }
    }
    let name = format!("rti-worker-{}", chrono::Utc::now().format("%Y%m%d%H%M%S"));
    let spec = MachineSpec {
        image: cfg.fly_worker_image.clone(),
        cmd: cmd.to_vec(),
        size: cfg.fly_worker_size.clone(),
        region: cfg.fly_region.clone(),
        env,
        auto_destroy: true,
        name: Some(name.clone()),
    };
    let m = fly.create_machine(&cfg.fly_app, &spec)?;
    budget.record_compute_usd(usd);
    Ok(WorkerHandle {
        app: cfg.fly_app.clone(),
        machine_id: m.id,
        name,
    })
}

/// Wait until the worker stops or is destroyed; returns the last machine state.
pub fn await_worker(
    cfg: &CloudConfig,
    h: &WorkerHandle,
    timeout: Duration,
) -> anyhow::Result<Machine> {
    let fly = FlyClient::from_config(cfg)?;
    let start = std::time::Instant::now();
    loop {
        let m = match fly.machine_status(&h.app, &h.machine_id) {
            Ok(m) => m,
            Err(e) if e.to_string().contains("404") => {
                return Ok(Machine {
                    id: h.machine_id.clone(),
                    name: h.name.clone(),
                    state: "destroyed".into(),
                    ..Default::default()
                })
            }
            Err(e) => return Err(e),
        };
        if m.state == "stopped" || m.state == "destroyed" {
            return Ok(m);
        }
        if start.elapsed() > timeout {
            let _ = fly.destroy_machine(&h.app, &h.machine_id);
            anyhow::bail!("worker {} timed out after {:?}; destroyed", h.name, timeout);
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}
