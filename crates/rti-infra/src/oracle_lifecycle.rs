//! Start the GPU oracle pod only when needed, keep it alive while used,
//! stop it when idle — with the daily-hours budget enforced.

use chrono::{DateTime, Utc};
use rti_core::config::CloudConfig;
use rti_oracle::Oracle;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::budget::{Budget, SpendDecision, SpendKind};
use crate::runpod::RunpodClient;

pub struct OracleLifecycle {
    cfg: CloudConfig,
    pub budget: Budget,
    oracle: Box<dyn Oracle>,
    heartbeat_url: Option<String>,
}

impl OracleLifecycle {
    pub fn new(
        cfg: &CloudConfig,
        data_dir: &Path,
        oracle: Box<dyn Oracle>,
        oracle_host: &str,
    ) -> OracleLifecycle {
        let heartbeat_url = if cfg.provider_oracle == "none" {
            None
        } else {
            Some(format!(
                "http://{}:{}/heartbeat",
                oracle_host, cfg.oracle_watchdog_port
            ))
        };
        OracleLifecycle {
            cfg: cfg.clone(),
            budget: Budget::open(cfg, data_dir),
            oracle,
            heartbeat_url,
        }
    }

    pub fn oracle(&self) -> &dyn Oracle {
        self.oracle.as_ref()
    }

    fn managed(&self) -> bool {
        self.cfg.provider_oracle == "runpod" && !self.cfg.runpod_pod_id.is_empty()
    }

    /// Make sure the oracle answers. For a managed pod: budget check, resume,
    /// wait for `Oracle::available()` up to `oracle_boot_timeout_secs`.
    pub fn ensure_up(&self) -> anyhow::Result<()> {
        if self.oracle.available().is_ok() {
            self.touch();
            return Ok(());
        }
        if !self.managed() {
            return self.oracle.available();
        }
        // assume a session costs at least 10 minutes of the daily hours cap
        match self.budget.can_spend(SpendKind::Oracle, 10.0 / 60.0) {
            SpendDecision::Ok => {}
            SpendDecision::Denied(m) | SpendDecision::NeedsApproval(m) => {
                anyhow::bail!("oracle start refused: {m}")
            }
        }
        let client = RunpodClient::from_config(&self.cfg)?;
        let st = client.pod_status(&self.cfg.runpod_pod_id)?;
        if !st.running {
            tracing::info!(pod = %st.id, "resuming oracle pod");
            client.resume_pod(&self.cfg.runpod_pod_id, 1)?;
        }
        self.budget
            .set_oracle_activity(Some(Utc::now()), Some(Utc::now()));
        let start = Instant::now();
        let timeout = Duration::from_secs(self.cfg.oracle_boot_timeout_secs);
        loop {
            if self.oracle.available().is_ok() {
                self.touch();
                return Ok(());
            }
            if start.elapsed() > timeout {
                let _ = client.stop_pod(&self.cfg.runpod_pod_id);
                self.budget.clear_oracle_session();
                anyhow::bail!(
                    "oracle pod did not become available within {}s; stopped it again",
                    self.cfg.oracle_boot_timeout_secs
                );
            }
            self.send_heartbeat();
            std::thread::sleep(Duration::from_secs(5));
        }
    }

    /// Record activity (keeps the idle timer and the pod watchdog alive).
    pub fn touch(&self) {
        self.budget.set_oracle_activity(None, Some(Utc::now()));
        self.send_heartbeat();
    }

    fn send_heartbeat(&self) {
        if let Some(url) = &self.heartbeat_url {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(3)))
                .build()
                .into();
            let _ = agent.post(url).send_empty();
        }
    }

    /// Stop the pod if idle longer than the configured minutes. Returns true
    /// when a stop was issued.
    pub fn maybe_stop_idle(&self) -> anyhow::Result<bool> {
        if !self.managed() {
            return Ok(false);
        }
        let st = self.budget.state();
        if st.oracle_started_at.is_none() {
            return Ok(false);
        }
        let last = st
            .oracle_last_activity
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Utc));
        if idle_expired(last, Utc::now(), self.cfg.oracle_idle_shutdown_minutes) {
            self.stop()?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn stop(&self) -> anyhow::Result<()> {
        if !self.managed() {
            return Ok(());
        }
        let client = RunpodClient::from_config(&self.cfg)?;
        client.stop_pod(&self.cfg.runpod_pod_id)?;
        let secs = self.budget.clear_oracle_session();
        tracing::info!(secs = ?secs, "oracle pod stopped");
        Ok(())
    }
}

/// Pure idle decision shared with the pod-side watchdog.
pub fn idle_expired(
    last_activity: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    idle_minutes: u64,
) -> bool {
    match last_activity {
        None => true,
        Some(t) => (now - t).num_seconds() >= (idle_minutes as i64) * 60,
    }
}

/// An `Oracle` that manages its own cloud lifecycle: boots the pod before
/// use (within budget), records activity, and stops it when idle via
/// `maintenance()`. Transparent when `provider_oracle = "none"`.
pub struct ManagedOracle {
    inner: OracleLifecycle,
}

impl ManagedOracle {
    pub fn new(
        cfg: &CloudConfig,
        data_dir: &Path,
        oracle: Box<dyn Oracle>,
        oracle_host: &str,
    ) -> ManagedOracle {
        ManagedOracle {
            inner: OracleLifecycle::new(cfg, data_dir, oracle, oracle_host),
        }
    }

    pub fn lifecycle(&self) -> &OracleLifecycle {
        &self.inner
    }
}

impl Oracle for ManagedOracle {
    fn name(&self) -> String {
        self.inner.oracle().name()
    }
    fn available(&self) -> anyhow::Result<()> {
        self.inner.ensure_up()?;
        let r = self.inner.oracle().available();
        self.inner.touch();
        r
    }
    fn run(
        &self,
        track: &rti_core::Track,
        actions: &[rti_core::Action],
        max_ticks: u32,
    ) -> anyhow::Result<rti_oracle::OracleRun> {
        self.inner.ensure_up()?;
        let r = self.inner.oracle().run(track, actions, max_ticks);
        self.inner.touch();
        if let Ok(run) = &r {
            let secs = run.ticks as f64 * self.inner.oracle().ms_per_tick() / 1000.0;
            self.inner.budget.record_oracle_seconds(secs);
        }
        r
    }
    fn ms_per_tick(&self) -> f64 {
        self.inner.oracle().ms_per_tick()
    }
    fn maintenance(&self) -> anyhow::Result<()> {
        self.inner.maybe_stop_idle().map(|_| ())
    }
}
