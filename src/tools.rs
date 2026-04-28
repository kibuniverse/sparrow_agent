use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

const TAVILY_SEARCH_URL: &str = "https://api.tavily.com/search";
const TAVILY_SEARCH_DEPTH: &str = "basic";
const TAVILY_MAX_RESULTS: u8 = 5;

pub async fn get_weather(location: &str) -> String {
    sleep(Duration::from_secs(2)).await;
    format!("The weather in {location} is sunny with a high of 25°C.")
}

pub async fn web_search(api_key: &str, query: &str) -> Result<String> {
    let client = Client::new();
    let request = TavilySearchRequest {
        api_key: api_key.into(),
        query: query.into(),
        search_depth: TAVILY_SEARCH_DEPTH.into(),
        include_answer: true,
        max_results: TAVILY_MAX_RESULTS,
    };

    let body: TavilySearchResponse = client
        .post(TAVILY_SEARCH_URL)
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .context("failed to send Tavily search request")?
        .error_for_status()
        .context("Tavily search request failed")?
        .json()
        .await
        .context("failed to parse Tavily search response")?;

    Ok(format_search_response(&body))
}

#[derive(Debug, Serialize)]
struct TavilySearchRequest {
    api_key: String,
    query: String,
    search_depth: String,
    include_answer: bool,
    max_results: u8,
}

#[derive(Debug, Deserialize)]
struct TavilySearchResponse {
    answer: Option<String>,
    results: Vec<TavilyResult>,
}

#[derive(Debug, Deserialize)]
struct TavilyResult {
    title: String,
    url: String,
    content: String,
}

fn format_search_response(response: &TavilySearchResponse) -> String {
    let mut output = String::new();

    if let Some(answer) = &response.answer {
        output.push_str(answer);
        output.push_str("\n\n");
    }

    for result in &response.results {
        output.push_str(&format!(
            "**{}**\n{}\n{}\n\n",
            result.title, result.url, result.content
        ));
    }

    if output.is_empty() {
        return "No results found.".into();
    }

    output
}
