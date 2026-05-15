use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const DEFAULT_SNAPSHOT_MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub seq: u64,
    pub task_id: String,
    pub conversation_id: String,
    pub timestamp: DateTime<Utc>,
    #[serde(rename = "type")]
    pub event_type: TraceEventType,
    pub payload: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceEventType {
    #[serde(rename = "task.started")]
    TaskStarted,
    #[serde(rename = "task.completed")]
    TaskCompleted,
    #[serde(rename = "task.failed")]
    TaskFailed,
    #[serde(rename = "model_call.started")]
    ModelCallStarted,
    #[serde(rename = "model_call.reasoning_delta")]
    ModelCallReasoningDelta,
    #[serde(rename = "model_call.completed")]
    ModelCallCompleted,
    #[serde(rename = "model_output.started")]
    ModelOutputStarted,
    #[serde(rename = "model_output.delta")]
    ModelOutputDelta,
    #[serde(rename = "model_output.completed")]
    ModelOutputCompleted,
    #[serde(rename = "tool_call.started")]
    ToolCallStarted,
    #[serde(rename = "tool_call.completed")]
    ToolCallCompleted,
    #[serde(rename = "tool_call.failed")]
    ToolCallFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonSnapshot {
    pub value: Value,
    pub text: String,
    pub truncated: bool,
}

impl JsonSnapshot {
    pub fn from_text(text: impl AsRef<str>, max_bytes: usize) -> Self {
        let text = text.as_ref();

        match serde_json::from_str::<Value>(text) {
            Ok(mut value) => {
                redact_json_value(&mut value);
                Self::from_redacted_value(value, max_bytes)
            }
            Err(_) => {
                let (text, truncated) = truncate_to_utf8_boundary(text, max_bytes);
                Self {
                    value: json!({ "raw": text }),
                    text,
                    truncated,
                }
            }
        }
    }

    pub fn from_value(mut value: Value, max_bytes: usize) -> Self {
        redact_json_value(&mut value);
        Self::from_redacted_value(value, max_bytes)
    }

    fn from_redacted_value(value: Value, max_bytes: usize) -> Self {
        let text = serde_json::to_string(&value).unwrap_or_else(|_| String::new());
        let (text, truncated) = truncate_to_utf8_boundary(&text, max_bytes);

        Self {
            value,
            text,
            truncated,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "succeeded")]
    Succeeded,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "cancelled")]
    Cancelled,
}

pub trait TraceSink: Send + Sync {
    fn emit(&self, event_type: TraceEventType, payload: Value);
}

pub fn redact_json_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if is_secret_key(key) {
                    *child = Value::String("[REDACTED]".into());
                } else {
                    redact_json_value(child);
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                redact_json_value(child);
            }
        }
        _ => {}
    }
}

pub fn trace_id(prefix: &str) -> String {
    format!("{prefix}_{}", ulid::Ulid::new())
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    ["api_key", "authorization", "password", "secret"]
        .iter()
        .any(|needle| key.contains(needle))
        || key == "token"
        || key.ends_with("token")
        || key.ends_with("_token")
        || key.ends_with("-token")
        || key.ends_with(".token")
}

fn truncate_to_utf8_boundary(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_string(), false);
    }

    if max_bytes == 0 {
        return (String::new(), true);
    }

    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }

    (text[..end].to_string(), true)
}
