//! Artifact sync to an S3/R2 bucket by shelling out to rclone/aws
//! (`[cloud] sync_command`), plus DuckDB snapshots.
//!
//! The DuckDB file must not be copied while an `Archive` holds it open;
//! `rti cloud sync` runs with the archive closed. Long-running servers
//! should only sync `cas/` and `snapshots/` written by `snapshot_db`.

use rti_core::config::CloudConfig;
use std::path::Path;
use std::process::Command;

fn remote(cfg: &CloudConfig, sub: &str) -> String {
    let prefix = cfg.s3_prefix.trim_matches('/');
    if prefix.is_empty() {
        format!("{}:{}/{}", "r2", cfg.s3_bucket, sub)
    } else {
        format!("{}:{}/{}/{}", "r2", cfg.s3_bucket, prefix, sub)
    }
}

fn run(cfg: &CloudConfig, src: &str, dst: &str) -> anyhow::Result<()> {
    let cmd = cfg.sync_command.replace("{src}", src).replace("{dst}", dst);
    tracing::info!(%cmd, "sync");
    let out = Command::new("bash").arg("-lc").arg(&cmd).output()?;
    if !out.status.success() {
        anyhow::bail!(
            "sync failed ({cmd}): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Copy `rti.duckdb` to `snapshots/rti-<timestamp>.duckdb` (archive must be closed).
pub fn snapshot_db(data_dir: &Path) -> anyhow::Result<std::path::PathBuf> {
    let db = data_dir.join("rti.duckdb");
    anyhow::ensure!(db.exists(), "no archive at {}", db.display());
    let dir = data_dir.join("snapshots");
    std::fs::create_dir_all(&dir)?;
    let dst = dir.join(format!(
        "rti-{}.duckdb",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    ));
    std::fs::copy(&db, &dst)?;
    Ok(dst)
}

pub fn sync_push(cfg: &CloudConfig, data_dir: &Path, with_snapshot: bool) -> anyhow::Result<()> {
    anyhow::ensure!(
        !cfg.s3_bucket.is_empty(),
        "[cloud] s3_bucket is not configured"
    );
    if with_snapshot {
        snapshot_db(data_dir)?;
    }
    run(
        cfg,
        data_dir.join("cas").to_str().unwrap(),
        &remote(cfg, "cas"),
    )?;
    let snaps = data_dir.join("snapshots");
    if snaps.is_dir() {
        run(cfg, snaps.to_str().unwrap(), &remote(cfg, "snapshots"))?;
    }
    Ok(())
}

pub fn sync_pull(cfg: &CloudConfig, data_dir: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        !cfg.s3_bucket.is_empty(),
        "[cloud] s3_bucket is not configured"
    );
    std::fs::create_dir_all(data_dir.join("cas"))?;
    run(
        cfg,
        &remote(cfg, "cas"),
        data_dir.join("cas").to_str().unwrap(),
    )?;
    std::fs::create_dir_all(data_dir.join("snapshots"))?;
    run(
        cfg,
        &remote(cfg, "snapshots"),
        data_dir.join("snapshots").to_str().unwrap(),
    )
}
