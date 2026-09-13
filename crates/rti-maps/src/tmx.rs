//! Trackmania Exchange client (trackmania.exchange API).
//!
//! Endpoints used:
//! * `GET /api/maps?fields=...&count=N&...`   search (returns {More, Results})
//! * `GET /mapgbx/{MapId}`                     download .Map.Gbx
//! * `GET /api/replays?mapId=...`              replay leaderboard (best effort)

use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const TMX_BASE: &str = "https://trackmania.exchange";
pub const USER_AGENT: &str =
    "RTI/0.1 (Recursive Trackmania Intelligence research bot; github.com/justhaug/rti)";

pub const MAP_FIELDS: &str = "MapId,MapUid,Name,GbxMapName,Authors,Length,Tags,AwardCount,Difficulty,Environment,VehicleName,MapType,UploadedAt,OnlineWR,ReplayCount";

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TmxMap {
    #[serde(rename = "MapId")]
    pub map_id: u64,
    #[serde(rename = "MapUid", default)]
    pub map_uid: String,
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "GbxMapName", default)]
    pub gbx_map_name: String,
    #[serde(rename = "Authors", default)]
    pub authors: Vec<serde_json::Value>,
    /// Author time in ms.
    #[serde(rename = "Length", default)]
    pub length: u64,
    #[serde(rename = "Tags", default)]
    pub tags: Vec<serde_json::Value>,
    #[serde(rename = "AwardCount", default)]
    pub award_count: u64,
    #[serde(rename = "Difficulty", default)]
    pub difficulty: i64,
    #[serde(rename = "Environment", default)]
    pub environment: i64,
    #[serde(rename = "VehicleName", default)]
    pub vehicle_name: String,
    #[serde(rename = "MapType", default)]
    pub map_type: String,
    #[serde(rename = "UploadedAt", default)]
    pub uploaded_at: String,
    #[serde(rename = "OnlineWR", default)]
    pub online_wr: serde_json::Value,
    #[serde(rename = "ReplayCount", default)]
    pub replay_count: u64,
}

impl TmxMap {
    pub fn author_names(&self) -> Vec<String> {
        self.authors
            .iter()
            .filter_map(|a| a["User"]["Name"].as_str().map(|s| s.to_string()))
            .collect()
    }
    pub fn tag_names(&self) -> Vec<String> {
        self.tags
            .iter()
            .filter_map(|t| t["Name"].as_str().map(|s| s.to_string()))
            .collect()
    }
    pub fn wr_ms(&self) -> Option<u64> {
        self.online_wr["RecordTime"].as_u64().filter(|&t| t > 0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SearchResult {
    #[serde(rename = "More", default)]
    pub more: bool,
    #[serde(rename = "Results", default)]
    pub results: Vec<TmxMap>,
}

pub struct TmxClient {
    agent: ureq::Agent,
    base: String,
}

impl Default for TmxClient {
    fn default() -> Self {
        Self::new()
    }
}

impl TmxClient {
    pub fn new() -> TmxClient {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent(USER_AGENT)
            .build()
            .into();
        TmxClient {
            agent,
            base: TMX_BASE.into(),
        }
    }

    pub fn with_base(mut self, base: &str) -> Self {
        self.base = base.trim_end_matches('/').to_string();
        self
    }

    /// Search maps. `params` are raw query parameters passed through to the
    /// API (e.g. `name`, `author`, `tag`, `order1`, `count`, `after`).
    pub fn search(&self, params: &[(&str, &str)]) -> anyhow::Result<SearchResult> {
        let mut req = self
            .agent
            .get(format!("{}/api/maps", self.base))
            .query("fields", MAP_FIELDS);
        let mut has_count = false;
        for (k, v) in params {
            if *k == "count" {
                has_count = true;
            }
            req = req.query(k, v);
        }
        if !has_count {
            req = req.query("count", "20");
        }
        let body: serde_json::Value = req.call()?.body_mut().read_json()?;
        if body.get("Results").is_none() {
            anyhow::bail!(
                "TMX search error: {}",
                crate::compile::truncate(&body.to_string(), 300)
            );
        }
        Ok(serde_json::from_value(body)?)
    }

    pub fn map_info(&self, map_id: u64) -> anyhow::Result<TmxMap> {
        let r = self.search(&[("id", &map_id.to_string()), ("count", "1")])?;
        r.results
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("TMX map {map_id} not found"))
    }

    /// Download the .Map.Gbx bytes.
    pub fn download(&self, map_id: u64) -> anyhow::Result<Vec<u8>> {
        let mut resp = self
            .agent
            .get(format!("{}/mapgbx/{map_id}", self.base))
            .call()?;
        let bytes = resp
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_vec()?;
        anyhow::ensure!(
            bytes.starts_with(b"GBX"),
            "TMX did not return a GBX file for map {map_id} ({} bytes)",
            bytes.len()
        );
        Ok(bytes)
    }

    /// Replay leaderboard for a map (fields best effort).
    pub fn replays(&self, map_id: u64, count: usize) -> anyhow::Result<serde_json::Value> {
        let body: serde_json::Value = self
            .agent
            .get(format!("{}/api/replays", self.base))
            .query("mapId", map_id.to_string())
            .query("count", count.to_string())
            .query(
                "fields",
                "ReplayId,ReplayTime,ReplayScore,Position,User.Name,User.UserId,ReplayAt",
            )
            .call()?
            .body_mut()
            .read_json()?;
        Ok(body)
    }

    /// Parse a TMX id from a number, a `/maps/123` URL or `tmx:123`.
    pub fn parse_id(s: &str) -> Option<u64> {
        let s = s.trim();
        if let Ok(n) = s.parse::<u64>() {
            return Some(n);
        }
        let s = s.strip_prefix("tmx:").unwrap_or(s);
        if let Ok(n) = s.parse::<u64>() {
            return Some(n);
        }
        for marker in [
            "/maps/",
            "/mapshow/",
            "/mapgbx/",
            "/tracks/view/",
            "/maps/view/",
        ] {
            if let Some(i) = s.find(marker) {
                let rest = &s[i + marker.len()..];
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(n) = digits.parse() {
                    return Some(n);
                }
            }
        }
        None
    }
}
