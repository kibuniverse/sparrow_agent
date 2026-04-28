use anyhow::{Context, Result};

use crate::api::{ChatCompletionRequest, ChatCompletionResponse};

const API_URL: &str = "https://api.deepseek.com/chat/completions";

pub struct DeepSeekClient {
    http: reqwest::Client,
}

impl DeepSeekClient {
    pub fn new(api_key: &str) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Content-Type", "application/json".parse().unwrap());
        headers.insert(
            "Authorization",
            format!("Bearer {api_key}").parse().unwrap(),
        );

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .expect("failed to build HTTP client");

        Self { http }
    }

    pub async fn chat_completion(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse> {
        self.http
            .post(API_URL)
            .json(request)
            .send()
            .await
            .context("failed to send chat completion request")?
            .error_for_status()
            .context("chat completion request failed")?
            .json()
            .await
            .context("failed to parse chat completion response")
    }
}
