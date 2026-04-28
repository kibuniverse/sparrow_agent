use std::time::Duration;

pub async fn get_weather(location: &str) -> String {
    trpl::sleep(Duration::from_secs(2)).await;
    format!("The weather in {location} is sunny with a high of 25°C.")
}

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
struct TavilySearchRequest {
    api_key: String,
    query: String,
    search_depth: String,
    include_answer: bool,
    max_results: u8,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TavilySearchResponse {
    query: String,
    answer: Option<String>,
    results: Vec<TavilyResult>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TavilyResult {
    title: String,
    url: String,
    content: String,
    score: Option<f64>,
}
pub async fn web_search(api_key: &str, query: &str) -> Result<String> {
    let client = Client::new();
    let request = TavilySearchRequest {
        api_key: api_key.into(),
        query: query.into(),
        search_depth: "basic".into(),
        include_answer: true,
        max_results: 5,
    };

    let response = client
        .post("https://api.tavily.com/search")
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await?;

    let body: TavilySearchResponse = response.json().await?;

    let mut output = String::new();
    if let Some(answer) = &body.answer {
        output.push_str(answer);
        output.push_str("\n\n");
    }

    for result in &body.results {
        output.push_str(&format!("**{}**\n{}\n{}\n\n", result.title, result.url, result.content));
    }

    if output.is_empty() {
        output = "No results found.".into();
    }

    Ok(output)
}