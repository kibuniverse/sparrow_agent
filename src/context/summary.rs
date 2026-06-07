use anyhow::Result;

use crate::context::{
    budget::TokenEstimator,
    transcript::{ConversationTurn, ToolExchange, ToolExchangeSummary, TurnSummary},
};

#[async_trait::async_trait]
pub trait ContextSummarizer: Send + Sync {
    async fn summarize_turn(&self, turn: &ConversationTurn) -> Result<TurnSummary>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FallbackContextSummarizer {
    pub max_tool_exchange_tokens: usize,
}

#[async_trait::async_trait]
impl ContextSummarizer for FallbackContextSummarizer {
    async fn summarize_turn(&self, turn: &ConversationTurn) -> Result<TurnSummary> {
        Ok(fallback_turn_summary(
            turn,
            self.max_tool_exchange_tokens.max(512),
            &TokenEstimator,
        ))
    }
}

pub fn fallback_turn_summary(
    turn: &ConversationTurn,
    max_tool_exchange_tokens: usize,
    estimator: &TokenEstimator,
) -> TurnSummary {
    let mut parts = vec![format!("Turn {} summary:", turn.id)];
    parts.push(format!(
        "- user: {}",
        estimator
            .truncate_text_to_tokens(turn.user.message.content.as_deref().unwrap_or(""), 1_000)
    ));

    for exchange in &turn.tool_exchanges {
        let summary = fallback_tool_exchange_summary(exchange, max_tool_exchange_tokens, estimator);
        parts.push(format!("- {}", summary.content.replace('\n', "\n  ")));
    }

    if let Some(assistant) = &turn.assistant {
        parts.push(format!(
            "- assistant: {}",
            estimator.truncate_text_to_tokens(
                assistant.message.content.as_deref().unwrap_or(""),
                2_000,
            )
        ));
    }

    TurnSummary {
        content: parts.join("\n"),
    }
}

pub fn fallback_tool_exchange_summary(
    exchange: &ToolExchange,
    max_tool_exchange_tokens: usize,
    estimator: &TokenEstimator,
) -> ToolExchangeSummary {
    let mut lines = vec!["Tool exchange summary:".to_string()];

    for tool_call in exchange.tool_calls() {
        lines.push(format!(
            "- model requested: {}({})",
            tool_call.function.name,
            estimator.truncate_text_to_tokens(&tool_call.function.arguments, 512)
        ));
    }

    lines.push(format!("- status: {:?}", exchange.status));

    for result in &exchange.tool_results {
        let tool_name = exchange
            .tool_calls()
            .iter()
            .find(|tool_call| tool_call.id == result.tool_call_id)
            .map(|tool_call| tool_call.function.name.as_str())
            .unwrap_or("unknown_tool");
        lines.push(format!(
            "- result for {}: original_chars={}, injected_chars={}, truncated={}",
            tool_name,
            result.metadata.original_chars,
            result.metadata.injected_chars,
            result.metadata.truncated
        ));
        if let Some(path) = result.metadata.artifact_path_display() {
            lines.push(format!("- artifact: {path}"));
        }

        let excerpt_budget = max_tool_exchange_tokens.saturating_div(2).max(256);
        let excerpt = estimator.truncate_text_to_tokens(&result.content, excerpt_budget);
        if !excerpt.trim().is_empty() {
            lines.push(format!("- excerpt: {}", excerpt.replace('\n', "\n  ")));
        }
    }

    ToolExchangeSummary {
        content: lines.join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        api::{ChatMessage, FunctionCall, ToolCall},
        context::{
            budget::TokenEstimator,
            summary::fallback_tool_exchange_summary,
            transcript::{
                ToolExchange, ToolExchangeStatus, TranscriptMessage, TranscriptToolResult,
            },
        },
        tool_result_processor::ToolResultMetadata,
    };

    #[test]
    fn tool_exchange_summary_includes_artifact_metadata() {
        let exchange = ToolExchange {
            assistant_message: TranscriptMessage {
                id: "msg_1".into(),
                message: ChatMessage {
                    role: "assistant".into(),
                    content: Some(String::new()),
                    reasoning_content: Some("Need to inspect.".into()),
                    tool_calls: Some(vec![ToolCall {
                        id: "call_1".into(),
                        kind: "function".into(),
                        function: FunctionCall {
                            name: "read_file".into(),
                            arguments: r#"{"path":"src/main.rs"}"#.into(),
                        },
                    }]),
                    tool_call_id: None,
                },
            },
            tool_results: vec![TranscriptToolResult {
                id: "msg_2".into(),
                tool_call_id: "call_1".into(),
                content: "result preview".into(),
                metadata: ToolResultMetadata {
                    original_chars: 50_000,
                    injected_chars: 20_000,
                    truncated: true,
                    artifact_path: Some(PathBuf::from(".sparrow_agent/tool_outputs/out.txt")),
                },
            }],
            summary: None,
            status: ToolExchangeStatus::Completed,
        };

        let summary = fallback_tool_exchange_summary(&exchange, 1_000, &TokenEstimator);

        assert!(summary.content.contains("read_file"));
        assert!(summary.content.contains("original_chars=50000"));
        assert!(summary.content.contains("truncated=true"));
        assert!(
            summary
                .content
                .contains(".sparrow_agent/tool_outputs/out.txt")
        );
    }
}
