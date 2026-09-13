//! `rti-oracle-watchdog`: run on the oracle pod. Env: RUNPOD_API_KEY,
//! RUNPOD_POD_ID (set by Runpod), optional WATCHDOG_PORT (27016),
//! WATCHDOG_IDLE_MINUTES (10), WATCHDOG_BOOT_GRACE_MINUTES (15).
fn main() -> anyhow::Result<()> {
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let cfg = rti_infra::watchdog::WatchdogConfig {
        port: env("WATCHDOG_PORT", "27016").parse()?,
        idle_minutes: env("WATCHDOG_IDLE_MINUTES", "10").parse()?,
        api_url: env("RUNPOD_API_URL", "https://api.runpod.io/graphql"),
        api_key: std::env::var("RUNPOD_API_KEY")
            .map_err(|_| anyhow::anyhow!("RUNPOD_API_KEY not set"))?,
        pod_id: std::env::var("RUNPOD_POD_ID")
            .map_err(|_| anyhow::anyhow!("RUNPOD_POD_ID not set"))?,
        boot_grace_minutes: env("WATCHDOG_BOOT_GRACE_MINUTES", "15").parse()?,
    };
    eprintln!(
        "rti-oracle-watchdog on :{} (idle {} min, pod {})",
        cfg.port, cfg.idle_minutes, cfg.pod_id
    );
    rti_infra::watchdog::run(cfg)
}
