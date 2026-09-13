use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use rti_core::config::LlmConfig;
use serde::de::DeserializeOwned;

use crate::json::extract_json;
use crate::types::{ChatResponse, Message, ToolCall, ToolDef, Usage};

/// Receives every LLM call for the ledger (implemented by the archive).
pub trait UsageSink: Send + Sync {
    fn record(
        &self,
        role: &str,
        model: &str,
        usage: &Usage,
        latency_ms: u64,
        ok: bool,
        error: Option<&str>,
    );
}

pub struct NoopSink;
impl UsageSink for NoopSink {
    fn record(&self, _: &str, _: &str, _: &Usage, _: u64, _: bool, _: Option<&str>) {}
}

#[derive(Clone, Debug, Default)]
pub struct ChatOptions {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Ask the provider for a JSON object response.
    pub json_mode: bool,
    /// Override the role's model.
    pub model: Option<String>,
}

pub struct LlmClient {
    cfg: LlmConfig,
    api_key: Option<String>,
    agent: ureq::Agent,
    /// model id -> (usd per prompt token, usd per completion token)
    pricing: RwLock<HashMap<String, (f64, f64)>>,
    spent_usd: Mutex<f64>,
    budget_usd: Mutex<Option<f64>>,
    sink: Box<dyn UsageSink>,
}

impl LlmClient {
    pub fn new(cfg: &LlmConfig) -> LlmClient {
        let api_key = std::env::var(&cfg.api_key_env)
            .ok()
            .filter(|k| !k.trim().is_empty());
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(cfg.timeout_secs.max(10))))
            .build()
            .into();
        LlmClient {
            cfg: cfg.clone(),
            api_key,
            agent,
            pricing: RwLock::new(HashMap::new()),
            spent_usd: Mutex::new(0.0),
            budget_usd: Mutex::new(None),
            sink: Box::new(NoopSink),
        }
    }

    pub fn with_sink(mut self, sink: Box<dyn UsageSink>) -> Self {
        self.sink = sink;
        self
    }

    /// Set a hard USD ceiling for this client instance (e.g. per cycle).
    pub fn set_budget(&self, usd: Option<f64>) {
        *self.budget_usd.lock().unwrap() = usd;
    }

    pub fn spent(&self) -> f64 {
        *self.spent_usd.lock().unwrap()
    }

    pub fn reset_spent(&self) {
        *self.spent_usd.lock().unwrap() = 0.0;
    }

    pub fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    pub fn config(&self) -> &LlmConfig {
        &self.cfg
    }

    pub fn model_for(&self, role: &str) -> String {
        self.cfg.roles.get(role).cloned().unwrap_or_else(|| {
            self.cfg
                .roles
                .get("researcher")
                .cloned()
                .unwrap_or_else(|| "z-ai/glm-5.3-flash".into())
        })
    }

    /// Fetch per-token prices for cost estimation when the provider does not
    /// return `usage.cost`.
    pub fn fetch_pricing(&self) -> anyhow::Result<usize> {
        let url = format!("{}/models", self.cfg.base_url.trim_end_matches('/'));
        let body: serde_json::Value = self.agent.get(&url).call()?.body_mut().read_json()?;
        let mut map = self.pricing.write().unwrap();
        for m in body["data"].as_array().cloned().unwrap_or_default() {
            let id = m["id"].as_str().unwrap_or_default().to_string();
            let p = m["pricing"]["prompt"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            let c = m["pricing"]["completion"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            map.insert(id, (p, c));
        }
        Ok(map.len())
    }

    pub fn price_for(&self, model: &str) -> Option<(f64, f64)> {
        self.pricing.read().unwrap().get(model).copied()
    }

    /// One chat completion for `role`, with retries and model fallbacks.
    pub fn chat(
        &self,
        role: &str,
        messages: &[Message],
        tools: &[ToolDef],
        opts: &ChatOptions,
    ) -> anyhow::Result<ChatResponse> {
        let key = self.api_key.as_ref().ok_or_else(|| {
            anyhow::anyhow!("no API key: set ${} (OpenRouter)", self.cfg.api_key_env)
        })?;
        if let Some(b) = *self.budget_usd.lock().unwrap() {
            let spent = self.spent();
            if spent >= b {
                anyhow::bail!("LLM budget exhausted: spent ${spent:.4} of ${b:.4}");
            }
        }
        let primary = opts.model.clone().unwrap_or_else(|| self.model_for(role));
        let mut candidates = vec![primary.clone()];
        for f in &self.cfg.fallbacks {
            if !candidates.contains(f) {
                candidates.push(f.clone());
            }
        }
        let mut last_err = None;
        for model in candidates {
            for attempt in 0..self.cfg.max_retries.max(1) {
                let started = Instant::now();
                match self.call_once(key, &model, messages, tools, opts) {
                    Ok(mut resp) => {
                        resp.latency_ms = started.elapsed().as_millis() as u64;
                        *self.spent_usd.lock().unwrap() += resp.usage.usd;
                        self.sink
                            .record(role, &model, &resp.usage, resp.latency_ms, true, None);
                        return Ok(resp);
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        tracing::warn!(role, model, attempt, error = %msg, "llm call failed");
                        self.sink.record(
                            role,
                            &model,
                            &Usage::default(),
                            started.elapsed().as_millis() as u64,
                            false,
                            Some(&msg),
                        );
                        last_err = Some(e);
                        let retryable = msg.contains("429")
                            || msg.contains("5") && msg.contains("status")
                            || msg.contains("timed out")
                            || msg.contains("timeout");
                        if !retryable {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(500 * (1 << attempt.min(4))));
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no model candidates")))
    }

    fn call_once(
        &self,
        key: &str,
        model: &str,
        messages: &[Message],
        tools: &[ToolDef],
        opts: &ChatOptions,
    ) -> anyhow::Result<ChatResponse> {
        let url = format!(
            "{}/chat/completions",
            self.cfg.base_url.trim_end_matches('/')
        );
        let mut body = serde_json::json!({
            "model": model,
            "messages": messages.iter().map(|m| m.to_wire()).collect::<Vec<_>>(),
            "temperature": opts.temperature.unwrap_or(self.cfg.temperature),
            "max_tokens": opts.max_tokens.unwrap_or(self.cfg.max_tokens),
            "usage": {"include": true},
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(tools.iter().map(|t| t.to_wire()).collect());
        }
        if opts.json_mode {
            body["response_format"] = serde_json::json!({"type": "json_object"});
        }
        let resp = self
            .agent
            .post(&url)
            .header("Authorization", &format!("Bearer {key}"))
            .header("HTTP-Referer", "https://github.com/justhaug/rti")
            .header("X-Title", &self.cfg.app_name)
            .send_json(&body);
        let mut resp = match resp {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(code)) => anyhow::bail!("HTTP status {code} from {model}"),
            Err(e) => anyhow::bail!("request error: {e}"),
        };
        let v: serde_json::Value = resp.body_mut().read_json()?;
        if let Some(err) = v.get("error") {
            anyhow::bail!("provider error: {err}");
        }
        let choice = v["choices"]
            .get(0)
            .ok_or_else(|| anyhow::anyhow!("no choices in response: {v}"))?;
        let m = &choice["message"];
        let mut tool_calls = vec![];
        for tc in m["tool_calls"].as_array().cloned().unwrap_or_default() {
            let args_raw = tc["function"]["arguments"].clone();
            let arguments = match &args_raw {
                serde_json::Value::String(s) => serde_json::from_str(s)
                    .unwrap_or_else(|_| extract_json(s).unwrap_or(serde_json::json!({}))),
                other => other.clone(),
            };
            tool_calls.push(ToolCall {
                id: tc["id"].as_str().unwrap_or_default().to_string(),
                name: tc["function"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                arguments,
            });
        }
        let content = match &m["content"] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(parts) => parts
                .iter()
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join(""),
            _ => String::new(),
        };
        let prompt_tokens = v["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
        let completion_tokens = v["usage"]["completion_tokens"].as_u64().unwrap_or(0);
        let usd = v["usage"]["cost"].as_f64().unwrap_or_else(|| {
            let (p, c) = self.price_for(model).unwrap_or((0.0, 0.0));
            prompt_tokens as f64 * p + completion_tokens as f64 * c
        });
        Ok(ChatResponse {
            message: Message {
                role: "assistant".into(),
                content,
                tool_calls,
                tool_call_id: None,
                name: None,
            },
            usage: Usage {
                prompt_tokens,
                completion_tokens,
                usd,
            },
            model: v["model"].as_str().unwrap_or(model).to_string(),
            latency_ms: 0,
            finish_reason: choice["finish_reason"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        })
    }

    /// Ask for a JSON document and parse it into `T`; on a parse failure the
    /// error is fed back once so the model can correct itself.
    pub fn chat_json<T: DeserializeOwned>(
        &self,
        role: &str,
        messages: &[Message],
        opts: &ChatOptions,
    ) -> anyhow::Result<(T, Vec<ChatResponse>)> {
        let mut msgs = messages.to_vec();
        let mut responses = vec![];
        let mut o = opts.clone();
        o.json_mode = true;
        for attempt in 0..2 {
            let resp = match self.chat(role, &msgs, &[], &o) {
                Ok(r) => r,
                Err(e) if attempt == 0 && o.json_mode => {
                    // some models reject response_format; retry without it
                    tracing::warn!(error = %e, "json_mode call failed, retrying without response_format");
                    o.json_mode = false;
                    self.chat(role, &msgs, &[], &o)?
                }
                Err(e) => return Err(e),
            };
            let text = resp.text().to_string();
            responses.push(resp);
            match extract_json(&text)
                .ok_or_else(|| anyhow::anyhow!("no JSON found"))
                .and_then(|v| Ok(serde_json::from_value::<T>(v)?))
            {
                Ok(t) => return Ok((t, responses)),
                Err(e) => {
                    if attempt == 1 {
                        anyhow::bail!(
                            "could not parse model JSON after retry: {e}\n--- output ---\n{text}"
                        );
                    }
                    msgs.push(Message::assistant(text));
                    msgs.push(Message::user(format!("Your JSON could not be parsed: {e}. Reply with ONLY the corrected JSON object.")));
                }
            }
        }
        unreachable!()
    }
}
