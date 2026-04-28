use anyhow::{Context, Result};

use crate::{
    api::{ChatCompletionRequest, ChatCompletionResponse},
    debug_log,
};

fn truncate_str(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        let mut end = max_len;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

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
        let request_json = serde_json::to_string(request).context("failed to serialize request")?;
        debug_log!("Sending request to DeepSeek API, payload:\n{request_json}");

        let response = self
            .http
            .post(API_URL)
            .json(request)
            .send()
            .await
            .context("failed to send chat completion request")?;

        let status = response.status();
        debug_log!("API response status: {status}");

        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!("<failed to read body: {e}>"));
            debug_log!("API error response body:\n{body}");
            anyhow::bail!("chat completion request failed with status {status}: {body}");
        }

        let body = response
            .text()
            .await
            .context("failed to read response body")?;
        debug_log!(
            "API response body (first 2000 chars):\n{}",
            truncate_str(&body, 2000)
        );

        serde_json::from_str(&body).context("failed to parse chat completion response")
    }
}
