//! Runpod GraphQL client: pod status, resume, stop.

use rti_core::config::CloudConfig;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const Q_POD: &str = r#"query Pod($input: PodFilter!) { pod(input: $input) { id name desiredStatus costPerHr runtime { uptimeInSeconds ports { ip isIpPublic privatePort publicPort type } } } }"#;
pub const M_RESUME: &str =
    r#"mutation Resume($input: PodResumeInput!) { podResume(input: $input) { id desiredStatus } }"#;
pub const M_STOP: &str =
    r#"mutation Stop($input: PodStopInput!) { podStop(input: $input) { id desiredStatus } }"#;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PodPort {
    pub ip: String,
    pub public: bool,
    pub private_port: u16,
    pub public_port: u16,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PodStatus {
    pub id: String,
    pub name: String,
    /// "RUNNING" | "EXITED" | ...
    pub desired_status: String,
    pub running: bool,
    pub uptime_secs: u64,
    pub cost_per_hr: f64,
    pub ports: Vec<PodPort>,
}

impl PodStatus {
    /// Public (ip, port) mapped to a given private port, if exposed.
    pub fn public_endpoint(&self, private_port: u16) -> Option<(String, u16)> {
        self.ports
            .iter()
            .find(|p| p.private_port == private_port && p.public)
            .map(|p| (p.ip.clone(), p.public_port))
    }
}

pub struct RunpodClient {
    agent: ureq::Agent,
    url: String,
    key: String,
}

impl RunpodClient {
    pub fn new(url: &str, key: &str) -> RunpodClient {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .into();
        RunpodClient {
            agent,
            url: url.to_string(),
            key: key.to_string(),
        }
    }

    pub fn from_config(cfg: &CloudConfig) -> anyhow::Result<RunpodClient> {
        let key = std::env::var(&cfg.runpod_api_key_env)
            .ok()
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("Runpod API key missing: set ${}", cfg.runpod_api_key_env)
            })?;
        Ok(Self::new(&cfg.runpod_api_url, &key))
    }

    fn graphql(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let body = serde_json::json!({"query": query, "variables": variables});
        let mut resp = self
            .agent
            .post(&self.url)
            .header("Authorization", &format!("Bearer {}", self.key))
            .header("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|e| anyhow::anyhow!("Runpod request failed: {e}"))?;
        let v: serde_json::Value = resp.body_mut().read_json()?;
        if let Some(errs) = v.get("errors").and_then(|e| e.as_array()) {
            if !errs.is_empty() {
                let msgs: Vec<String> = errs
                    .iter()
                    .map(|e| e["message"].as_str().unwrap_or("unknown").to_string())
                    .collect();
                anyhow::bail!("Runpod GraphQL error: {}", msgs.join("; "));
            }
        }
        Ok(v["data"].clone())
    }

    pub fn pod_status(&self, pod_id: &str) -> anyhow::Result<PodStatus> {
        let data = self.graphql(Q_POD, serde_json::json!({"input": {"podId": pod_id}}))?;
        let p = &data["pod"];
        if p.is_null() {
            anyhow::bail!("Runpod: pod {pod_id} not found");
        }
        Ok(parse_pod(p))
    }

    pub fn resume_pod(&self, pod_id: &str, gpu_count: u32) -> anyhow::Result<String> {
        let data = self.graphql(
            M_RESUME,
            serde_json::json!({"input": {"podId": pod_id, "gpuCount": gpu_count}}),
        )?;
        Ok(data["podResume"]["desiredStatus"]
            .as_str()
            .unwrap_or("")
            .to_string())
    }

    pub fn stop_pod(&self, pod_id: &str) -> anyhow::Result<String> {
        let data = self.graphql(M_STOP, serde_json::json!({"input": {"podId": pod_id}}))?;
        Ok(data["podStop"]["desiredStatus"]
            .as_str()
            .unwrap_or("")
            .to_string())
    }
}

pub fn parse_pod(p: &serde_json::Value) -> PodStatus {
    let desired = p["desiredStatus"].as_str().unwrap_or("").to_string();
    let runtime = &p["runtime"];
    let ports = runtime["ports"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| PodPort {
                    ip: x["ip"].as_str().unwrap_or("").to_string(),
                    public: x["isIpPublic"].as_bool().unwrap_or(false),
                    private_port: x["privatePort"].as_u64().unwrap_or(0) as u16,
                    public_port: x["publicPort"].as_u64().unwrap_or(0) as u16,
                    kind: x["type"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    PodStatus {
        id: p["id"].as_str().unwrap_or("").to_string(),
        name: p["name"].as_str().unwrap_or("").to_string(),
        running: desired == "RUNNING" && !runtime.is_null(),
        desired_status: desired,
        uptime_secs: runtime["uptimeInSeconds"].as_u64().unwrap_or(0),
        cost_per_hr: p["costPerHr"].as_f64().unwrap_or(0.0),
        ports,
    }
}
