use serde::{Deserialize, Serialize};
use std::fmt;

/// BLAKE3 content hash, hex encoded. Used as the key for every artifact in
/// the content-addressed store (trajectories, models, datasets, plots...).
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(transparent)]
pub struct ContentHash(pub String);

impl ContentHash {
    pub fn of_bytes(bytes: &[u8]) -> Self {
        ContentHash(blake3::hash(bytes).to_hex().to_string())
    }

    /// Hash of the canonical JSON encoding of a value.
    pub fn of_json<T: Serialize>(value: &T) -> Self {
        let bytes = serde_json::to_vec(value).expect("serializable");
        Self::of_bytes(&bytes)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn short(&self) -> &str {
        &self.0[..12.min(self.0.len())]
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "b3:{}", self.short())
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ContentHash {
    fn from(s: String) -> Self {
        ContentHash(s)
    }
}
