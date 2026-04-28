use std::env;

use anyhow::{Context, Result};

const DEFAULT_MODEL: &str = "deepseek-v4-flash";
const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant.";
const DEFAULT_REASONING_EFFORT: &str = "high";
const DEFAULT_MAX_TOOL_ROUNDS: usize = 6;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub api_key: String,
    pub tavily_api_key: String,
    pub model: String,
    pub system_prompt: String,
    pub reasoning_effort: String,
    pub max_tool_rounds: usize,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let api_key = env::var("DEEPSEEK_API_KEY")
            .context("DEEPSEEK_API_KEY environment variable is not set")?;
        let tavily_api_key =
            env::var("TAVILY_API_KEY").context("TAVILY_API_KEY environment variable is not set")?;

        Ok(Self {
            api_key,
            tavily_api_key,
            model: DEFAULT_MODEL.into(),
            system_prompt: DEFAULT_SYSTEM_PROMPT.into(),
            reasoning_effort: DEFAULT_REASONING_EFFORT.into(),
            max_tool_rounds: DEFAULT_MAX_TOOL_ROUNDS,
        })
    }
}
