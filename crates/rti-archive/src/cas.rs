//! BLAKE3 content-addressed store: `<root>/<hash[0..2]>/<hash>`.
//!
//! Every large artifact (trajectory telemetry, model weights, datasets,
//! plots) lives here; the DuckDB tables hold hashes and summaries.

use anyhow::Context;
use rti_core::ContentHash;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Cas {
    root: PathBuf,
}

impl Cas {
    pub fn open(root: impl Into<PathBuf>) -> anyhow::Result<Cas> {
        let root = root.into();
        std::fs::create_dir_all(&root)
            .with_context(|| format!("creating CAS root {}", root.display()))?;
        Ok(Cas { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self, hash: &ContentHash) -> PathBuf {
        let h = hash.as_str();
        let prefix = if h.len() >= 2 { &h[..2] } else { "xx" };
        self.root.join(prefix).join(h)
    }

    pub fn exists(&self, hash: &ContentHash) -> bool {
        self.path(hash).is_file()
    }

    /// Store bytes; returns the hash. Atomic (temp file + rename), idempotent.
    pub fn put_bytes(&self, bytes: &[u8]) -> anyhow::Result<ContentHash> {
        let hash = ContentHash::of_bytes(bytes);
        let path = self.path(&hash);
        if path.is_file() {
            return Ok(hash);
        }
        let dir = path.parent().expect("cas path has parent");
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".{}.{}.tmp", hash.as_str(), std::process::id()));
        std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
        match std::fs::rename(&tmp, &path) {
            Ok(()) => {}
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                if !path.is_file() {
                    return Err(e).with_context(|| format!("renaming into {}", path.display()));
                }
            }
        }
        Ok(hash)
    }

    pub fn put_json<T: Serialize>(&self, value: &T) -> anyhow::Result<ContentHash> {
        let bytes = serde_json::to_vec(value)?;
        self.put_bytes(&bytes)
    }

    pub fn get_bytes(&self, hash: &ContentHash) -> anyhow::Result<Vec<u8>> {
        let path = self.path(hash);
        std::fs::read(&path).with_context(|| format!("CAS object {} not found", hash))
    }

    pub fn get_json<T: DeserializeOwned>(&self, hash: &ContentHash) -> anyhow::Result<T> {
        let bytes = self.get_bytes(hash)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Number of objects and total bytes stored.
    pub fn stats(&self) -> anyhow::Result<(u64, u64)> {
        let mut n = 0u64;
        let mut bytes = 0u64;
        for entry in walkdir::WalkDir::new(&self.root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                n += 1;
                bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
        Ok((n, bytes))
    }
}
