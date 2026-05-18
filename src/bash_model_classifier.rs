use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    api::{ChatCompletionRequest, ChatMessage},
    bash_risk::{BashRiskLevel, BashRiskRequest, PolicyCandidate},
    client::DeepSeekClient,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRiskResponse {
    pub risk: BashRiskLevel,
    pub confidence: f32,
    pub reason: String,
    pub policy_candidate: Option<PolicyCandidate>,
}

pub struct ModelRiskClassifier {
    client: DeepSeekClient,
    threshold: f32,
}

impl ModelRiskClassifier {
    pub fn new(api_key: &str, threshold: f32) -> Self {
        Self {
            client: DeepSeekClient::new(api_key),
            threshold,
        }
    }

    pub async fn classify(&self, request: &BashRiskRequest) -> Result<Option<ModelRiskResponse>> {
        let prompt = serde_json::json!({
            "command": request.command,
            "cwd": request.cwd,
            "allowed_roots": request.allowed_roots,
            "timeout_ms": request.timeout_ms,
            "instruction": "Classify only low-risk bash commands. Return strict JSON with risk, confidence, reason, and optional policy_candidate. Never label destructive, privileged, or remote script commands as low.",
        });
        let request = ChatCompletionRequest {
            model: "deepseek-chat".into(),
            messages: vec![
                ChatMessage::system(
                    "You are a conservative bash command risk classifier. Return JSON only.",
                ),
                ChatMessage::user(prompt.to_string()),
            ],
            tools: None,
            thinking: None,
            reasoning_effort: None,
            stream: None,
            stream_options: None,
        };

        let response = self.client.chat_completion(&request).await?;
        let Some(content) = response
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_deref())
        else {
            return Ok(None);
        };
        let parsed: ModelRiskResponse =
            serde_json::from_str(content).context("model returned invalid bash risk JSON")?;
        if parsed.risk == BashRiskLevel::Low && parsed.confidence >= self.threshold {
            Ok(Some(parsed))
        } else {
            Ok(None)
        }
    }
}
