use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::process::Command;

/// Exact provenance for a finding / model / experiment: code, world, inputs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub created_at: DateTime<Utc>,
    pub git_commit: String,
    pub git_dirty: bool,
    pub sim_version: u32,
    /// Hash of PhysicsParams used (if any).
    #[serde(default)]
    pub params_hash: Option<String>,
    #[serde(default)]
    pub track_hash: Option<String>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// Ids of experiments / findings / artifacts this depends on.
    #[serde(default)]
    pub parents: Vec<String>,
    #[serde(default)]
    pub hostname: String,
}

impl Provenance {
    pub fn now() -> Self {
        let (git_commit, git_dirty) = git_head();
        Provenance {
            created_at: Utc::now(),
            git_commit,
            git_dirty,
            sim_version: crate::PhysicsParams::VERSION,
            params_hash: None,
            track_hash: None,
            seed: None,
            parents: vec![],
            hostname: std::fs::read_to_string("/etc/hostname")
                .unwrap_or_default()
                .trim()
                .to_string(),
        }
    }

    pub fn with_params(mut self, h: &crate::ContentHash) -> Self {
        self.params_hash = Some(h.0.clone());
        self
    }
    pub fn with_track(mut self, h: &crate::ContentHash) -> Self {
        self.track_hash = Some(h.0.clone());
        self
    }
    pub fn with_seed(mut self, s: u64) -> Self {
        self.seed = Some(s);
        self
    }
    pub fn with_parent(mut self, id: impl Into<String>) -> Self {
        self.parents.push(id.into());
        self
    }
}

pub fn git_head() -> (String, bool) {
    let commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    (commit, dirty)
}
