use anyhow::Result;
use indicatif::{InMemoryTerm, ProgressBar, ProgressDrawTarget, ProgressStyle};

use crate::{
    api::{ChatCompletionRequest, ChatMessage, ChoiceMessage, ThinkingConfig, Usage},
    client::DeepSeekClient,
    config::AppConfig,
    debug_log,
    local_tools::LocalToolProvider,
    mcp::{client::McpClient, filesystem_provider::McpToolProvider},
    tool_provider::ToolProvider,
    tool_registry::ToolRegistry,
};

const CONTEXT_PROGRESS_BAR_WIDTH: usize = 24;
const DEEPSEEK_V4_CONTEXT_TOKENS: u32 = 1_000_000;

pub struct Agent {
    client: DeepSeekClient,
    config: AppConfig,
    messages: Vec<ChatMessage>,
    tool_registry: ToolRegistry,
    context_usage: ContextUsage,
}

impl Agent {
    pub async fn new(config: AppConfig) -> Result<Self> {
        let client = DeepSeekClient::new(&config.api_key);
        let messages = vec![ChatMessage::system(&config.system_prompt)];
        let context_usage = ContextUsage::for_model(&config.model);

        let mut tool_registry = ToolRegistry::new();

        // Add local tools
        tool_registry.add_provider(Box::new(LocalToolProvider::new(&config.tavily_api_key)));

        // Add MCP filesystem tools if enabled
        if config.filesystem.enabled {
            for server_config in &config.mcp_servers {
                if !server_config.enabled {
                    continue;
                }

                match McpClient::connect(
                    server_config.id.clone(),
                    &server_config.command,
                    &server_config.args,
                    config.filesystem.roots.clone(),
                )
                .await
                {
                    Ok(mcp_client) => {
                        match McpToolProvider::new(config.filesystem.clone(), mcp_client).await {
                            Ok(provider) => {
                                println!(
                                    "Filesystem tools enabled ({} tools from '{}').",
                                    provider.definitions().len(),
                                    server_config.id,
                                );
                                println!("Roots:");
                                for root in &config.filesystem.roots {
                                    let display = root.canonicalize().unwrap_or_else(|_| root.clone());
                                    println!("  - {}", display.display());
                                }
                                println!("Mode: {:?}", config.filesystem.mode);
                                tool_registry.add_provider(Box::new(provider));
                            }
                            Err(e) => {
                                eprintln!(
                                    "Warning: filesystem MCP provider init failed for '{}': {e}",
                                    server_config.id,
                                );
                                eprintln!("Filesystem tools disabled for this session.");
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "Warning: MCP server '{}' failed to connect: {e}",
                            server_config.id,
                        );
                        eprintln!("Filesystem tools disabled for this session.");
                        eprintln!(
                            "Hint: ensure '{}' is available (e.g., npx is installed and Node.js is present).",
                            server_config.command,
                        );
                    }
                }
            }
        }

        Ok(Self {
            client,
            config,
            messages,
            tool_registry,
            context_usage,
        })
    }

    pub fn context_usage_line(&self) -> String {
        self.context_usage.render_line()
    }

    pub async fn handle_user_input(&mut self, input: impl Into<String>) -> Result<()> {
        self.messages.push(ChatMessage::user(input));

        for round in 0..self.config.max_tool_rounds {
            debug_log!("=== Tool round {round} ===");
            debug_log!("Message count: {}", self.messages.len());
            for (i, msg) in self.messages.iter().enumerate() {
                let content_str = msg.content.as_deref().unwrap_or("<None>");
                let preview_len = content_str.len().min(80);
                debug_log!(
                    "msg[{i}] role={}, content={:?}..., tool_calls={}, tool_call_id={}",
                    msg.role,
                    &content_str[..preview_len],
                    msg.tool_calls.as_ref().map(|tc| tc.len()).unwrap_or(0),
                    msg.tool_call_id.as_deref().unwrap_or("<None>"),
                );
            }

            let request = self.build_request();
            let response = self.client.chat_completion(&request).await?;
            self.context_usage.update_from_usage(&response.usage);

            let Some(choice) = response.choices.first() else {
                debug_log!("Empty choices in response");
                return Ok(());
            };

            debug_log!(
                "Response: finish_reason={}, has_content={}, has_tool_calls={}",
                choice.finish_reason,
                choice.message.content.is_some(),
                choice.message.tool_calls.is_some(),
            );

            match self.handle_assistant_message(&choice.message).await {
                TurnStatus::Continue => continue,
                TurnStatus::Complete => return Ok(()),
            }
        }

        eprintln!(
            "Error: reached the maximum number of tool rounds ({})",
            self.config.max_tool_rounds
        );
        Ok(())
    }

    fn build_request(&self) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: self.config.model.clone(),
            messages: self.messages.clone(),
            tools: Some(self.tool_registry.definitions().to_vec()),
            thinking: Some(ThinkingConfig::enabled()),
            reasoning_effort: Some(self.config.reasoning_effort.clone()),
            stream: None,
        }
    }

    async fn handle_assistant_message(&mut self, message: &ChoiceMessage) -> TurnStatus {
        if let Some(tool_calls) = message.tool_calls.as_deref() {
            debug_log!(
                "Assistant requests {} tool call(s): {:?}",
                tool_calls.len(),
                tool_calls
                    .iter()
                    .map(|tc| &tc.function.name)
                    .collect::<Vec<_>>(),
            );

            self.messages.push(ChatMessage {
                role: "assistant".into(),
                content: Some(String::new()),
                reasoning_content: message.reasoning_content.clone(),
                tool_calls: message.tool_calls.clone(),
                tool_call_id: None,
            });

            let results = self.tool_registry.execute_all(tool_calls).await;
            for result in &results {
                let preview_len = result.content.len().min(120);
                debug_log!(
                    "Tool result: id={}, content={:?}...",
                    result.tool_call_id,
                    &result.content[..preview_len],
                );
            }
            for result in results {
                self.messages
                    .push(ChatMessage::tool(result.content, &result.tool_call_id));
            }

            return TurnStatus::Continue;
        }

        if let Some(content) = &message.content {
            println!("agent> {content}");
            self.messages.push(ChatMessage::assistant(
                content,
                message.reasoning_content.clone(),
            ));
        }

        TurnStatus::Complete
    }
}

#[derive(Debug, Clone, Copy)]
struct ContextUsage {
    used_tokens: u32,
    total_tokens: Option<u32>,
}

impl ContextUsage {
    fn for_model(model: &str) -> Self {
        Self {
            used_tokens: 0,
            total_tokens: model_context_window_tokens(model),
        }
    }

    fn update_from_usage(&mut self, usage: &Usage) {
        self.used_tokens = usage.total_tokens;
    }

    fn render_line(&self) -> String {
        match self.total_tokens {
            Some(total_tokens) => {
                let percent = self.percent_used(total_tokens);
                format!(
                    "context> {} {} / {} tokens ({percent:.2}%)",
                    render_progress_bar(self.used_tokens, Some(total_tokens)),
                    format_token_count(self.used_tokens),
                    format_token_count(total_tokens),
                )
            }
            None => format!(
                "context> {} {} / unknown tokens",
                render_progress_bar(self.used_tokens, None),
                format_token_count(self.used_tokens)
            ),
        }
    }

    fn percent_used(&self, total_tokens: u32) -> f64 {
        if total_tokens == 0 {
            0.0
        } else {
            self.used_tokens as f64 / total_tokens as f64 * 100.0
        }
    }
}

enum TurnStatus {
    Continue,
    Complete,
}

fn model_context_window_tokens(model: &str) -> Option<u32> {
    match model {
        "deepseek-v4-flash" | "deepseek-v4-pro" => Some(DEEPSEEK_V4_CONTEXT_TOKENS),
        _ => None,
    }
}

fn render_progress_bar(used_tokens: u32, total_tokens: Option<u32>) -> String {
    let Some(total_tokens) = total_tokens.filter(|total_tokens| *total_tokens > 0) else {
        return render_indicatif_progress_bar(0, 1, ContextUsageColor::Green);
    };

    let color = ContextUsageColor::for_usage(used_tokens, total_tokens);
    render_indicatif_progress_bar(used_tokens.min(total_tokens), total_tokens, color)
}

fn render_indicatif_progress_bar(
    used_tokens: u32,
    total_tokens: u32,
    color: ContextUsageColor,
) -> String {
    let term = InMemoryTerm::new(1, (CONTEXT_PROGRESS_BAR_WIDTH + 2) as u16);
    let draw_target = ProgressDrawTarget::term_like(Box::new(term.clone()));
    let style = ProgressStyle::with_template(&format!(
        "[{{bar:{CONTEXT_PROGRESS_BAR_WIDTH}.{}}}]",
        color.as_indicatif_style()
    ))
    .expect("context progress bar template should be valid")
    .progress_chars("==-");
    let progress = ProgressBar::with_draw_target(Some(total_tokens as u64), draw_target);

    progress.set_style(style);
    progress.set_position(used_tokens as u64);
    progress.force_draw();

    let contents = String::from_utf8_lossy(&term.contents_formatted()).into_owned();
    contents.trim_end().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContextUsageColor {
    Green,
    Yellow,
    Red,
}

impl ContextUsageColor {
    fn for_usage(used_tokens: u32, total_tokens: u32) -> Self {
        let percent = if total_tokens == 0 {
            0.0
        } else {
            used_tokens as f64 / total_tokens as f64 * 100.0
        };

        if percent < 40.0 {
            Self::Green
        } else if percent <= 70.0 {
            Self::Yellow
        } else {
            Self::Red
        }
    }

    fn as_indicatif_style(self) -> &'static str {
        match self {
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Red => "red",
        }
    }
}

fn format_token_count(value: u32) -> String {
    let digits = value.to_string();
    let first_group_len = digits.len() % 3;
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);

    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (index + 3 - first_group_len) % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(ch);
    }

    formatted
}

#[cfg(test)]
mod tests {
    use super::{
        ContextUsage, ContextUsageColor, format_token_count, model_context_window_tokens,
        render_progress_bar,
    };

    #[test]
    fn formats_token_counts_with_group_separators() {
        assert_eq!(format_token_count(0), "0");
        assert_eq!(format_token_count(42), "42");
        assert_eq!(format_token_count(1_234), "1,234");
        assert_eq!(format_token_count(1_000_000), "1,000,000");
    }

    #[test]
    fn knows_deepseek_v4_context_windows() {
        assert_eq!(
            model_context_window_tokens("deepseek-v4-flash"),
            Some(1_000_000)
        );
        assert_eq!(
            model_context_window_tokens("deepseek-v4-pro"),
            Some(1_000_000)
        );
        assert_eq!(model_context_window_tokens("custom-model"), None);
    }

    #[test]
    fn renders_context_usage_line_with_progress_bar() {
        let context_usage = ContextUsage {
            used_tokens: 250_000,
            total_tokens: Some(1_000_000),
        };

        assert_eq!(
            strip_ansi_codes(&context_usage.render_line()),
            "context> [=======-----------------] 250,000 / 1,000,000 tokens (25.00%)"
        );
    }

    #[test]
    fn renders_unknown_context_total_with_empty_progress_bar() {
        assert_eq!(
            strip_ansi_codes(&render_progress_bar(12_345, None)),
            "[------------------------]"
        );
    }

    #[test]
    fn selects_context_usage_colors_by_threshold() {
        assert_eq!(
            ContextUsageColor::for_usage(399_999, 1_000_000),
            ContextUsageColor::Green
        );
        assert_eq!(
            ContextUsageColor::for_usage(400_000, 1_000_000),
            ContextUsageColor::Yellow
        );
        assert_eq!(
            ContextUsageColor::for_usage(700_000, 1_000_000),
            ContextUsageColor::Yellow
        );
        assert_eq!(
            ContextUsageColor::for_usage(700_001, 1_000_000),
            ContextUsageColor::Red
        );
    }

    fn strip_ansi_codes(input: &str) -> String {
        let mut output = String::new();
        let mut chars = input.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '\x1b' && chars.peek() == Some(&'[') {
                chars.next();
                for ch in chars.by_ref() {
                    if ch.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }

            output.push(ch);
        }

        output
    }
}
