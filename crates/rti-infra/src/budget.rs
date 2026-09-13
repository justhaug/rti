//! Hard spending controls, persisted in `<data_dir>/cloud_state.json` so
//! they survive restarts and are independent of the research agent.

use chrono::{DateTime, Datelike, Utc};
use rti_core::config::CloudConfig;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpendKind {
    Oracle,
    Compute,
    OpenRouter,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct BudgetState {
    /// YYYY-MM-DD the daily counters belong to.
    pub day: String,
    /// YYYY-MM the monthly counters belong to.
    pub month: String,
    pub oracle_seconds_today: f64,
    pub oracle_seconds_month: f64,
    pub compute_usd_today: f64,
    pub compute_usd_month: f64,
    pub openrouter_usd_today: f64,
    pub openrouter_usd_month: f64,
    /// Last time the oracle was used (RFC3339), for idle shutdown.
    pub oracle_last_activity: Option<String>,
    /// When the current oracle session started (RFC3339), if running.
    pub oracle_started_at: Option<String>,
}

/// Outcome of a spend check.
#[derive(Clone, Debug, PartialEq)]
pub enum SpendDecision {
    Ok,
    /// Over a daily or monthly cap; message says which.
    Denied(String),
    /// Under the caps but above `require_approval_above_usd`.
    NeedsApproval(String),
}

pub struct Budget {
    cfg: CloudConfig,
    path: PathBuf,
    state: Mutex<BudgetState>,
}

fn day_of(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d").to_string()
}
fn month_of(t: DateTime<Utc>) -> String {
    format!("{:04}-{:02}", t.year(), t.month())
}

impl Budget {
    pub fn open(cfg: &CloudConfig, data_dir: &Path) -> Budget {
        let path = data_dir.join("cloud_state.json");
        let state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let b = Budget {
            cfg: cfg.clone(),
            path,
            state: Mutex::new(state),
        };
        b.rollover(Utc::now());
        b
    }

    pub fn state(&self) -> BudgetState {
        self.state.lock().unwrap().clone()
    }

    fn save(&self, st: &BudgetState) {
        if let Some(p) = self.path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        if let Ok(s) = serde_json::to_string_pretty(st) {
            let _ = std::fs::write(&self.path, s);
        }
    }

    /// Reset daily/monthly counters when the calendar moved on.
    pub fn rollover(&self, now: DateTime<Utc>) {
        let mut st = self.state.lock().unwrap();
        let (d, m) = (day_of(now), month_of(now));
        let mut changed = false;
        if st.day != d {
            st.day = d;
            st.oracle_seconds_today = 0.0;
            st.compute_usd_today = 0.0;
            st.openrouter_usd_today = 0.0;
            changed = true;
        }
        if st.month != m {
            st.month = m;
            st.oracle_seconds_month = 0.0;
            st.compute_usd_month = 0.0;
            st.openrouter_usd_month = 0.0;
            changed = true;
        }
        if changed {
            self.save(&st);
        }
    }

    pub fn record_oracle_seconds(&self, secs: f64) {
        self.rollover(Utc::now());
        let mut st = self.state.lock().unwrap();
        st.oracle_seconds_today += secs;
        st.oracle_seconds_month += secs;
        self.save(&st);
    }

    pub fn record_compute_usd(&self, usd: f64) {
        self.record_usd(SpendKind::Compute, usd)
    }

    pub fn record_usd(&self, kind: SpendKind, usd: f64) {
        self.rollover(Utc::now());
        let mut st = self.state.lock().unwrap();
        match kind {
            SpendKind::Compute => {
                st.compute_usd_today += usd;
                st.compute_usd_month += usd;
            }
            SpendKind::OpenRouter => {
                st.openrouter_usd_today += usd;
                st.openrouter_usd_month += usd;
            }
            SpendKind::Oracle => {}
        }
        self.save(&st);
    }

    /// Can we spend `usd` (or, for the oracle, `usd` = hours) now?
    pub fn can_spend(&self, kind: SpendKind, amount: f64) -> SpendDecision {
        self.rollover(Utc::now());
        let st = self.state.lock().unwrap();
        decide(&self.cfg, &st, kind, amount)
    }

    pub fn set_oracle_activity(
        &self,
        started: Option<DateTime<Utc>>,
        active: Option<DateTime<Utc>>,
    ) {
        let mut st = self.state.lock().unwrap();
        if let Some(s) = started {
            st.oracle_started_at = Some(s.to_rfc3339());
        }
        if let Some(a) = active {
            st.oracle_last_activity = Some(a.to_rfc3339());
        }
        self.save(&st);
    }

    pub fn clear_oracle_session(&self) -> Option<f64> {
        let mut st = self.state.lock().unwrap();
        let secs = st
            .oracle_started_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|s| (Utc::now() - s.with_timezone(&Utc)).num_seconds() as f64);
        st.oracle_started_at = None;
        if let Some(s) = secs {
            st.oracle_seconds_today += s;
            st.oracle_seconds_month += s;
        }
        self.save(&st);
        secs
    }
}

/// Pure decision so it can be unit-tested without a file.
pub fn decide(cfg: &CloudConfig, st: &BudgetState, kind: SpendKind, amount: f64) -> SpendDecision {
    match kind {
        SpendKind::Oracle => {
            let hours_today = st.oracle_seconds_today / 3600.0;
            if hours_today + amount > cfg.oracle_daily_hours {
                return SpendDecision::Denied(format!(
                    "oracle daily hours cap: {:.2}h used + {:.2}h requested > {:.2}h",
                    hours_today, amount, cfg.oracle_daily_hours
                ));
            }
            SpendDecision::Ok
        }
        SpendKind::Compute => {
            if st.compute_usd_today + amount > cfg.compute_daily_usd {
                return SpendDecision::Denied(format!(
                    "compute daily cap: ${:.2} used + ${:.2} > ${:.2}",
                    st.compute_usd_today, amount, cfg.compute_daily_usd
                ));
            }
            if st.compute_usd_month + amount > cfg.compute_monthly_usd {
                return SpendDecision::Denied(format!(
                    "compute monthly cap: ${:.2} used + ${:.2} > ${:.2}",
                    st.compute_usd_month, amount, cfg.compute_monthly_usd
                ));
            }
            if amount > cfg.require_approval_above_usd {
                return SpendDecision::NeedsApproval(format!(
                    "${amount:.2} exceeds the approval threshold ${:.2}",
                    cfg.require_approval_above_usd
                ));
            }
            SpendDecision::Ok
        }
        SpendKind::OpenRouter => {
            if st.openrouter_usd_today + amount > cfg.openrouter_daily_usd {
                return SpendDecision::Denied(format!(
                    "OpenRouter daily cap: ${:.2} used + ${:.2} > ${:.2}",
                    st.openrouter_usd_today, amount, cfg.openrouter_daily_usd
                ));
            }
            if st.openrouter_usd_month + amount > cfg.openrouter_monthly_usd {
                return SpendDecision::Denied(format!(
                    "OpenRouter monthly cap: ${:.2} used + ${:.2} > ${:.2}",
                    st.openrouter_usd_month, amount, cfg.openrouter_monthly_usd
                ));
            }
            if amount > cfg.require_approval_above_usd {
                return SpendDecision::NeedsApproval(format!(
                    "${amount:.2} exceeds the approval threshold ${:.2}",
                    cfg.require_approval_above_usd
                ));
            }
            SpendDecision::Ok
        }
    }
}
