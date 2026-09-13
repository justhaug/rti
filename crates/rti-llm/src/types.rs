use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Parsed JSON arguments (the wire format carries a string).
    pub arguments: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    pub fn system(s: impl Into<String>) -> Message {
        Message {
            role: "system".into(),
            content: s.into(),
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
        }
    }
    pub fn user(s: impl Into<String>) -> Message {
        Message {
            role: "user".into(),
            content: s.into(),
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
        }
    }
    pub fn assistant(s: impl Into<String>) -> Message {
        Message {
            role: "assistant".into(),
            content: s.into(),
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
        }
    }
    pub fn tool_result(call_id: &str, name: &str, content: impl Into<String>) -> Message {
        Message {
            role: "tool".into(),
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: Some(call_id.to_string()),
            name: Some(name.to_string()),
        }
    }

    /// Wire encoding (OpenAI-compatible chat message).
    pub fn to_wire(&self) -> serde_json::Value {
        let mut v = serde_json::json!({"role": self.role, "content": self.content});
        if !self.tool_calls.is_empty() {
            v["tool_calls"] = serde_json::Value::Array(
                self.tool_calls
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "id": t.id,
                            "type": "function",
                            "function": {"name": t.name, "arguments": serde_json::to_string(&t.arguments).unwrap_or_default()}
                        })
                    })
                    .collect(),
            );
        }
        if let Some(id) = &self.tool_call_id {
            v["tool_call_id"] = serde_json::Value::String(id.clone());
        }
        if let Some(n) = &self.name {
            v["name"] = serde_json::Value::String(n.clone());
        }
        v
    }
}

/// A function tool exposed to the model.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON schema of the arguments object.
    pub parameters: serde_json::Value,
}

impl ToolDef {
    pub fn to_wire(&self) -> serde_json::Value {
        serde_json::json!({"type": "function", "function": {"name": self.name, "description": self.description, "parameters": self.parameters}})
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub usd: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatResponse {
    pub message: Message,
    pub usage: Usage,
    pub model: String,
    pub latency_ms: u64,
    pub finish_reason: String,
}

impl ChatResponse {
    pub fn text(&self) -> &str {
        &self.message.content
    }
}
