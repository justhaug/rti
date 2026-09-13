//! Fly Machines REST client (api.machines.dev) for disposable workers.

use rti_core::config::CloudConfig;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MachineSpec {
    pub image: String,
    pub cmd: Vec<String>,
    /// e.g. "performance-4x", "shared-cpu-2x"
    pub size: String,
    pub region: String,
    pub env: BTreeMap<String, String>,
    pub auto_destroy: bool,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Machine {
    pub id: String,
    pub name: String,
    pub state: String,
    pub region: String,
    pub instance_id: Option<String>,
    pub exit_code: Option<i64>,
}

pub struct FlyClient {
    agent: ureq::Agent,
    base: String,
    token: String,
}

impl FlyClient {
    pub fn new(base: &str, token: &str) -> FlyClient {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .into();
        FlyClient {
            agent,
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    pub fn from_config(cfg: &CloudConfig) -> anyhow::Result<FlyClient> {
        let token = std::env::var(&cfg.fly_api_token_env)
            .ok()
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("Fly API token missing: set ${}", cfg.fly_api_token_env)
            })?;
        Ok(Self::new(&cfg.fly_api_url, &token))
    }

    fn get(&self, path: &str) -> anyhow::Result<serde_json::Value> {
        let mut r = self
            .agent
            .get(format!("{}{}", self.base, path))
            .header("Authorization", &format!("Bearer {}", self.token))
            .call()
            .map_err(|e| anyhow::anyhow!("Fly GET {path}: {e}"))?;
        Ok(r.body_mut().read_json()?)
    }

    fn post(&self, path: &str, body: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let mut r = self
            .agent
            .post(format!("{}{}", self.base, path))
            .header("Authorization", &format!("Bearer {}", self.token))
            .send_json(&body)
            .map_err(|e| anyhow::anyhow!("Fly POST {path}: {e}"))?;
        let text = r.body_mut().read_to_string().unwrap_or_default();
        if text.trim().is_empty() {
            return Ok(serde_json::json!({}));
        }
        Ok(serde_json::from_str(&text).unwrap_or(serde_json::json!({"raw": text})))
    }

    fn delete(&self, path: &str) -> anyhow::Result<()> {
        self.agent
            .delete(format!("{}{}", self.base, path))
            .header("Authorization", &format!("Bearer {}", self.token))
            .call()
            .map_err(|e| anyhow::anyhow!("Fly DELETE {path}: {e}"))?;
        Ok(())
    }

    pub fn list_machines(&self, app: &str) -> anyhow::Result<Vec<Machine>> {
        let v = self.get(&format!("/apps/{app}/machines"))?;
        Ok(v.as_array()
            .map(|a| a.iter().map(parse_machine).collect())
            .unwrap_or_default())
    }

    pub fn create_machine(&self, app: &str, spec: &MachineSpec) -> anyhow::Result<Machine> {
        let (cpu_kind, cpus) = parse_size(&spec.size);
        let mut body = serde_json::json!({
            "region": spec.region,
            "config": {
                "image": spec.image,
                "env": spec.env,
                "auto_destroy": spec.auto_destroy,
                "restart": {"policy": "no"},
                "guest": {"cpu_kind": cpu_kind, "cpus": cpus, "memory_mb": cpus * 2048},
                "init": {"cmd": spec.cmd}
            }
        });
        if let Some(n) = &spec.name {
            body["name"] = serde_json::Value::String(n.clone());
        }
        let v = self.post(&format!("/apps/{app}/machines"), body)?;
        if v["id"].is_null() {
            anyhow::bail!("Fly create machine returned no id: {v}");
        }
        Ok(parse_machine(&v))
    }

    pub fn machine_status(&self, app: &str, id: &str) -> anyhow::Result<Machine> {
        Ok(parse_machine(
            &self.get(&format!("/apps/{app}/machines/{id}"))?,
        ))
    }

    pub fn stop_machine(&self, app: &str, id: &str) -> anyhow::Result<()> {
        self.post(
            &format!("/apps/{app}/machines/{id}/stop"),
            serde_json::json!({}),
        )?;
        Ok(())
    }

    pub fn destroy_machine(&self, app: &str, id: &str) -> anyhow::Result<()> {
        self.delete(&format!("/apps/{app}/machines/{id}?force=true"))
    }

    /// Poll until the machine reaches `state` (e.g. "started", "stopped", "destroyed").
    pub fn wait_for_state(
        &self,
        app: &str,
        id: &str,
        state: &str,
        timeout: Duration,
    ) -> anyhow::Result<Machine> {
        let start = Instant::now();
        loop {
            let m = self.machine_status(app, id)?;
            if m.state == state {
                return Ok(m);
            }
            if start.elapsed() > timeout {
                anyhow::bail!(
                    "machine {id} did not reach {state} within {:?} (state {})",
                    timeout,
                    m.state
                );
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

fn parse_size(size: &str) -> (String, u64) {
    // "performance-4x" → (performance, 4); "shared-cpu-2x" → (shared, 2)
    let n = size
        .rsplit('-')
        .next()
        .and_then(|s| s.trim_end_matches('x').parse::<u64>().ok())
        .unwrap_or(1);
    let kind = if size.starts_with("performance") {
        "performance"
    } else {
        "shared"
    };
    (kind.to_string(), n)
}

pub fn parse_machine(v: &serde_json::Value) -> Machine {
    Machine {
        id: v["id"].as_str().unwrap_or("").to_string(),
        name: v["name"].as_str().unwrap_or("").to_string(),
        state: v["state"].as_str().unwrap_or("").to_string(),
        region: v["region"].as_str().unwrap_or("").to_string(),
        instance_id: v["instance_id"].as_str().map(|s| s.to_string()),
        exit_code: v["events"].as_array().and_then(|evs| {
            evs.iter()
                .find_map(|e| e["request"]["exit_event"]["exit_code"].as_i64())
        }),
    }
}
