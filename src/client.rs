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
    ) -> Result<ChatCompletionResponse, Box<dyn std::error::Error>> {
        let response = self.http.post(API_URL).json(request).send().await?;
        let text = response.text().await?;
        let parsed_response: ChatCompletionResponse = serde_json::from_str(&text)?;
        Ok(parsed_response)
    }
}
