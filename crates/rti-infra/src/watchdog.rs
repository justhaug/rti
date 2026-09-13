//! Pod-side shutdown watchdog. Runs ON the oracle pod, independent of RTI:
//! serves `GET /health` and `POST /heartbeat`; if no heartbeat arrives for
//! `idle_minutes`, stops its own pod via the Runpod API (pod id from
//! `RUNPOD_POD_ID`, which Runpod sets inside pods).

use chrono::{DateTime, Utc};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::oracle_lifecycle::idle_expired;
use crate::runpod::RunpodClient;

pub struct WatchdogConfig {
    pub port: u16,
    pub idle_minutes: u64,
    pub api_url: String,
    pub api_key: String,
    pub pod_id: String,
    /// Grace period after boot before idleness counts.
    pub boot_grace_minutes: u64,
}

/// Decide whether to stop: pure so it can be tested with fake clocks.
pub fn should_stop(
    started: DateTime<Utc>,
    last_heartbeat: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    cfg_idle_minutes: u64,
    boot_grace_minutes: u64,
) -> bool {
    if (now - started).num_seconds() < (boot_grace_minutes as i64) * 60 {
        return false;
    }
    idle_expired(last_heartbeat.or(Some(started)), now, cfg_idle_minutes)
}

pub fn run(cfg: WatchdogConfig) -> anyhow::Result<()> {
    let last: Arc<Mutex<Option<DateTime<Utc>>>> = Arc::new(Mutex::new(None));
    let started = Utc::now();
    let server = tiny_http::Server::http(("0.0.0.0", cfg.port))
        .map_err(|e| anyhow::anyhow!("bind {}: {e}", cfg.port))?;
    let last_http = last.clone();
    std::thread::spawn(move || {
        for req in server.incoming_requests() {
            let url = req.url().to_string();
            let body = if url.starts_with("/heartbeat") {
                *last_http.lock().unwrap() = Some(Utc::now());
                "ok\n"
            } else if url.starts_with("/health") {
                "ok\n"
            } else {
                "rti-oracle-watchdog\n"
            };
            let _ = req.respond(tiny_http::Response::from_string(body));
        }
    });
    let client = RunpodClient::new(&cfg.api_url, &cfg.api_key);
    loop {
        std::thread::sleep(Duration::from_secs(30));
        let lh = *last.lock().unwrap();
        if should_stop(
            started,
            lh,
            Utc::now(),
            cfg.idle_minutes,
            cfg.boot_grace_minutes,
        ) {
            eprintln!(
                "watchdog: no heartbeat for {} min; stopping pod {}",
                cfg.idle_minutes, cfg.pod_id
            );
            match client.stop_pod(&cfg.pod_id) {
                Ok(_) => std::thread::sleep(Duration::from_secs(300)),
                Err(e) => eprintln!("watchdog: stop failed: {e}"),
            }
        }
    }
}
