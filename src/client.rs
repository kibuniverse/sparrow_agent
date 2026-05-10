use anyhow::{Context, Result};
use futures_util::StreamExt;

use crate::{
    api::{
        ChatCompletionRequest, ChatCompletionResponse, ChatCompletionStreamChunk, StreamOptions,
    },
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

#[cfg(test)]
mod tests {
    use crate::api::{ChatCompletionRequest, StreamOptions};

    use super::streaming_request;

    #[test]
    fn streaming_request_includes_usage_options() {
        let request = ChatCompletionRequest {
            model: "deepseek-chat".into(),
            messages: Vec::new(),
            tools: None,
            thinking: None,
            reasoning_effort: None,
            stream: None,
            stream_options: None,
        };

        let request = streaming_request(&request);

        assert_eq!(request.stream, Some(true));
        assert!(matches!(
            request.stream_options,
            Some(StreamOptions {
                include_usage: true
            })
        ));
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

    pub fn chat_completion_stream(
        &self,
        request: &ChatCompletionRequest,
    ) -> impl futures_util::Stream<Item = Result<ChatCompletionStreamChunk>> + '_ {
        let stream_request = streaming_request(request);

        debug_log!(
            "Sending streaming request, payload:\n{}",
            serde_json::to_string(&stream_request).unwrap_or_default()
        );

        let http = self.http.clone();

        async_stream::stream! {
            let response = match http
                .post(API_URL)
                .json(&stream_request)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    yield Err(anyhow::anyhow!("failed to send streaming request: {e}"));
                    return;
                }
            };

            let status = response.status();
            debug_log!("Stream API response status: {status}");

            if !status.is_success() {
                let body = response
                    .text()
                    .await
                    .unwrap_or_else(|e| format!("<failed to read body: {e}>"));
                debug_log!("Stream API error response body:\n{body}");
                yield Err(anyhow::anyhow!(
                    "streaming request failed with status {status}: {body}"
                ));
                return;
            }

            let byte_stream = response.bytes_stream();
            let mut buffer = String::new();

            let mut byte_stream = Box::pin(byte_stream);

            while let Some(chunk_result) = byte_stream.next().await {
                let bytes = match chunk_result {
                    Ok(b) => b,
                    Err(e) => {
                        yield Err(anyhow::anyhow!("error reading stream bytes: {e}"));
                        return;
                    }
                };

                let text = match std::str::from_utf8(&bytes) {
                    Ok(t) => t,
                    Err(e) => {
                        yield Err(anyhow::anyhow!("invalid UTF-8 in stream: {e}"));
                        return;
                    }
                };

                buffer.push_str(text);

                while let Some(frame_end) = buffer.find("\n\n").or_else(|| buffer.find("\r\n\r\n"))
                {
                    let frame = buffer[..frame_end].to_string();
                    let separator_len = if buffer[frame_end..].starts_with("\r\n\r\n") { 4 } else { 2 };
                    buffer = buffer[frame_end + separator_len..].to_string();

                    for line in frame.lines() {
                        let line = line.trim();
                        let Some(payload) = line.strip_prefix("data:") else {
                            continue;
                        };
                        let payload = payload.trim();

                        if payload == "[DONE]" {
                            debug_log!("Stream received [DONE]");
                            return;
                        }

                        match serde_json::from_str::<ChatCompletionStreamChunk>(payload) {
                            Ok(chunk) => {
                                debug_log!("Stream chunk parsed: {} choice(s)", chunk.choices.len());
                                yield Ok(chunk);
                            }
                            Err(e) => {
                                let preview = truncate_str(payload, 500);
                                debug_log!("Stream chunk parse error: {e}, payload: {preview}");
                                yield Err(anyhow::anyhow!(
                                    "failed to parse stream chunk: {e} — payload: {preview}"
                                ));
                                return;
                            }
                        }
                    }
                }
            }

            debug_log!("Stream byte stream ended");
        }
    }
}

fn streaming_request(request: &ChatCompletionRequest) -> ChatCompletionRequest {
    let mut stream_request = request.clone();
    stream_request.stream = Some(true);
    stream_request.stream_options = Some(StreamOptions {
        include_usage: true,
    });
    stream_request
}
