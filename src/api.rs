use serde::{Deserialize, Serialize};

// ── Request types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into() }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: "assistant".into(), content: content.into() }
    }
    
    pub fn tool(content: impl Into<String>) -> Self {
        Self { role: "tool".into(), content: content.into() }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ThinkingConfig {
    #[serde(rename = "type")]
    pub kind: String,
}

impl ThinkingConfig {
    pub fn enabled() -> Self {
        Self { kind: "enabled".into() }
    }
}

// ── Response types ─────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
    pub system_fingerprint: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: ChoiceMessage,
    pub logprobs: Option<serde_json::Value>,
    pub finish_reason: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ChoiceMessage {
    pub role: String,
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub prompt_tokens_details: PromptTokensDetails,
    pub completion_tokens_details: CompletionTokensDetails,
    pub prompt_cache_hit_tokens: u32,
    pub prompt_cache_miss_tokens: u32,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct PromptTokensDetails {
    pub cached_tokens: u32,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct CompletionTokensDetails {
    pub reasoning_tokens: u32,
}
