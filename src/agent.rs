use std::time::Instant;

use anyhow::Result;
use futures_util::StreamExt;
use indicatif::{InMemoryTerm, ProgressBar, ProgressDrawTarget, ProgressStyle};
use serde_json::{Value, json};

use crate::{
    api::{ChatCompletionRequest, ChatMessage, ChoiceMessage, ThinkingConfig, Usage},
    client::DeepSeekClient,
    config::AppConfig,
    console::ConsoleTraceRenderer,
    debug_log,
    local_tools::LocalToolProvider,
    mcp::{client::McpClient, filesystem_provider::McpToolProvider},
    streaming::{AgentEventSink, AgentStreamEvent, StreamAccumulator},
    tool_provider::ToolProvider,
    tool_registry::ToolRegistry,
    tool_result_processor::{ToolResultProcessor, ToolResultProcessorConfig},
    trace::{DEFAULT_SNAPSHOT_MAX_BYTES, JsonSnapshot, TraceEventType, TraceSink, trace_id},
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

        let tool_result_processor = ToolResultProcessor::new(ToolResultProcessorConfig {
            max_injected_chars: config.tool_results.max_injected_chars,
            output_dir: config.tool_results.output_dir.clone(),
        });
        let mut tool_registry = ToolRegistry::with_result_processor(tool_result_processor);

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
                                    let display =
                                        root.canonicalize().unwrap_or_else(|_| root.clone());
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

        if self.config.streaming.enabled {
            self.run_streaming_loop().await
        } else {
            self.run_non_streaming_loop().await
        }
    }

    pub async fn handle_user_input_with_trace(
        &mut self,
        input: impl Into<String>,
        sink: &dyn TraceSink,
    ) -> Result<()> {
        let started = Instant::now();
        let input = input.into();

        sink.emit(
            TraceEventType::TaskStarted,
            json!({
                "message": {
                    "role": "user",
                    "content": input,
                },
            }),
        );

        self.messages.push(ChatMessage::user(input));

        match self.run_streaming_trace_loop(sink).await {
            Ok(final_answer) => {
                sink.emit(
                    TraceEventType::TaskCompleted,
                    json!({
                        "duration_ms": started.elapsed().as_millis() as u64,
                        "final_answer": final_answer,
                    }),
                );
                Ok(())
            }
            Err(error) => {
                sink.emit(
                    TraceEventType::TaskFailed,
                    json!({
                        "duration_ms": started.elapsed().as_millis() as u64,
                        "error": error.to_string(),
                    }),
                );
                Err(error)
            }
        }
    }

    async fn run_non_streaming_loop(&mut self) -> Result<()> {
        for round in 0..self.config.max_tool_rounds {
            debug_log!("=== Tool round {round} (non-streaming) ===");
            self.log_messages();

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

    async fn run_streaming_loop(&mut self) -> Result<()> {
        let mut renderer = ConsoleTraceRenderer::new(&self.config.streaming);

        for round in 0..self.config.max_tool_rounds {
            debug_log!("=== Tool round {round} (streaming) ===");
            self.log_messages();

            let request = self.build_request();
            let completed = {
                let stream = self.client.chat_completion_stream(&request);
                let mut accumulator = StreamAccumulator::new();
                let mut stream = Box::pin(stream);
                while let Some(chunk_result) = stream.next().await {
                    let chunk = chunk_result?;
                    accumulator.push(chunk, &mut renderer, round)?;
                }
                accumulator.finish(&mut renderer)?
            };

            if let Some(usage) = &completed.usage {
                self.context_usage.update_from_usage(usage);
            }

            match self.handle_assistant_message(&completed.message).await {
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

    async fn run_streaming_trace_loop(&mut self, sink: &dyn TraceSink) -> Result<String> {
        for round in 0..self.config.max_tool_rounds {
            debug_log!("=== Tool round {round} (streaming trace) ===");
            self.log_messages();

            let request = self.build_request();
            let model_call_id = trace_id("model");
            let started = Instant::now();

            sink.emit(
                TraceEventType::ModelCallStarted,
                json!({
                    "node_id": model_call_id,
                    "round": round + 1,
                    "model": request.model,
                    "request": self.model_request_snapshot(&request),
                }),
            );

            let mut forwarder = StreamingTraceForwarder::new(model_call_id.clone(), sink);
            let completed = {
                let stream = self.client.chat_completion_stream(&request);
                let mut accumulator = StreamAccumulator::new();
                let mut stream = Box::pin(stream);
                while let Some(chunk_result) = stream.next().await {
                    let chunk = chunk_result?;
                    accumulator.push(chunk, &mut forwarder, round + 1)?;
                }
                accumulator.finish(&mut forwarder)?
            };

            if let Some(usage) = &completed.usage {
                self.context_usage.update_from_usage(usage);
            }

            let parent_model_output_id = forwarder.emit_completed(&completed.message);

            sink.emit(
                TraceEventType::ModelCallCompleted,
                json!({
                    "node_id": model_call_id,
                    "duration_ms": started.elapsed().as_millis() as u64,
                    "finish_reason": completed.finish_reason,
                    "usage": completed.usage.as_ref().map(trace_usage),
                    "response": JsonSnapshot::from_value(
                        json!({
                            "has_content": completed
                                .message
                                .content
                                .as_deref()
                                .is_some_and(|content| !content.is_empty()),
                            "tool_call_count": completed
                                .message
                                .tool_calls
                                .as_ref()
                                .map(|tool_calls| tool_calls.len())
                                .unwrap_or(0),
                        }),
                        DEFAULT_SNAPSHOT_MAX_BYTES,
                    ),
                }),
            );

            match self
                .handle_assistant_message_with_trace(
                    &completed.message,
                    parent_model_output_id.as_deref(),
                    sink,
                )
                .await
            {
                TracedTurnStatus::Continue => continue,
                TracedTurnStatus::Complete(final_answer) => return Ok(final_answer),
            }
        }

        anyhow::bail!(
            "reached the maximum number of tool rounds ({})",
            self.config.max_tool_rounds
        );
    }

    fn log_messages(&self) {
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
    }

    fn build_request(&self) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: self.config.model.clone(),
            messages: self.messages.clone(),
            tools: Some(self.tool_registry.definitions().to_vec()),
            thinking: Some(ThinkingConfig::enabled()),
            reasoning_effort: Some(self.config.reasoning_effort.clone()),
            stream: None,
            stream_options: None,
        }
    }

    fn model_request_snapshot(&self, request: &ChatCompletionRequest) -> JsonSnapshot {
        JsonSnapshot::from_value(
            json!({
                "model": request.model,
                "message_count": request.messages.len(),
                "tool_count": request.tools.as_ref().map(|tools| tools.len()).unwrap_or(0),
                "thinking": request.thinking,
                "reasoning_effort": request.reasoning_effort,
            }),
            DEFAULT_SNAPSHOT_MAX_BYTES,
        )
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
            if !self.config.streaming.enabled && !content.is_empty() {
                println!("agent> {content}");
            }
            self.messages.push(ChatMessage::assistant(
                content,
                message.reasoning_content.clone(),
            ));
        }

        TurnStatus::Complete
    }

    async fn handle_assistant_message_with_trace(
        &mut self,
        message: &ChoiceMessage,
        parent_model_output_id: Option<&str>,
        sink: &dyn TraceSink,
    ) -> TracedTurnStatus {
        if let Some(tool_calls) = message.tool_calls.as_deref() {
            debug_log!(
                "Assistant requests {} traced tool call(s): {:?}",
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

            let results = self
                .tool_registry
                .execute_all_traced(
                    tool_calls,
                    parent_model_output_id.unwrap_or("output_unknown"),
                    sink,
                )
                .await;

            for result in results {
                self.messages
                    .push(ChatMessage::tool(result.content, &result.tool_call_id));
            }

            return TracedTurnStatus::Continue;
        }

        let final_answer = message.content.clone().unwrap_or_default();
        self.messages.push(ChatMessage::assistant(
            final_answer.clone(),
            message.reasoning_content.clone(),
        ));

        TracedTurnStatus::Complete(final_answer)
    }
}

struct StreamingTraceForwarder<'a> {
    model_call_id: String,
    sink: &'a dyn TraceSink,
    output_node_id: Option<String>,
    output_kind: Option<&'static str>,
}

impl<'a> StreamingTraceForwarder<'a> {
    fn new(model_call_id: String, sink: &'a dyn TraceSink) -> Self {
        Self {
            model_call_id,
            sink,
            output_node_id: None,
            output_kind: None,
        }
    }

    #[cfg(test)]
    fn output_node_id(&self) -> Option<&str> {
        self.output_node_id.as_deref()
    }

    fn ensure_output_started(&mut self, kind: &'static str) -> String {
        if self.output_kind == Some(kind)
            && let Some(node_id) = &self.output_node_id
        {
            return node_id.clone();
        }

        let node_id = trace_id("output");
        self.output_node_id = Some(node_id.clone());
        self.output_kind = Some(kind);
        self.sink.emit(
            TraceEventType::ModelOutputStarted,
            json!({
                "node_id": node_id,
                "parent_model_call_id": self.model_call_id,
                "kind": kind,
            }),
        );
        node_id
    }

    fn emit_completed(&mut self, message: &ChoiceMessage) -> Option<String> {
        let has_tool_calls = message
            .tool_calls
            .as_ref()
            .is_some_and(|tool_calls| !tool_calls.is_empty());
        let has_content = message
            .content
            .as_deref()
            .is_some_and(|content| !content.is_empty());

        let kind = if has_tool_calls {
            "tool_calls"
        } else if has_content {
            "final_answer"
        } else {
            return None;
        };

        let node_id = self.ensure_output_started(kind);
        let tool_calls = message
            .tool_calls
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(index, tool_call)| {
                json!({
                    "index": index,
                    "tool_call_id": tool_call.id,
                    "name": tool_call.function.name,
                    "arguments": JsonSnapshot::from_text(
                        &tool_call.function.arguments,
                        DEFAULT_SNAPSHOT_MAX_BYTES,
                    ),
                })
            })
            .collect::<Vec<_>>();

        self.sink.emit(
            TraceEventType::ModelOutputCompleted,
            json!({
                "node_id": node_id,
                "kind": kind,
                "content": message.content.clone().unwrap_or_default(),
                "tool_calls": tool_calls,
            }),
        );

        self.output_node_id.clone()
    }
}

impl AgentEventSink for StreamingTraceForwarder<'_> {
    fn on_event(&mut self, event: &AgentStreamEvent) -> anyhow::Result<()> {
        match event {
            AgentStreamEvent::ReasoningDelta(delta) => {
                self.sink.emit(
                    TraceEventType::ModelCallReasoningDelta,
                    json!({
                        "node_id": self.model_call_id,
                        "delta": delta,
                    }),
                );
            }
            AgentStreamEvent::AnswerStarted => {
                self.ensure_output_started("final_answer");
            }
            AgentStreamEvent::AnswerDelta(content_delta) => {
                let node_id = self.ensure_output_started("final_answer");
                self.sink.emit(
                    TraceEventType::ModelOutputDelta,
                    json!({
                        "node_id": node_id,
                        "kind": "final_answer",
                        "content_delta": content_delta,
                    }),
                );
            }
            AgentStreamEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            } => {
                let node_id = self.ensure_output_started("tool_calls");
                self.sink.emit(
                    TraceEventType::ModelOutputDelta,
                    json!({
                        "node_id": node_id,
                        "kind": "tool_calls",
                        "tool_call": {
                            "index": index,
                            "tool_call_id": id,
                            "name": name,
                            "arguments_delta": arguments_delta,
                        },
                    }),
                );
            }
            AgentStreamEvent::ResponseStarted { .. }
            | AgentStreamEvent::ReasoningStarted
            | AgentStreamEvent::ResponseFinished { .. }
            | AgentStreamEvent::Usage(_) => {}
        }

        Ok(())
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

enum TracedTurnStatus {
    Continue,
    Complete(String),
}

fn trace_usage(usage: &Usage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "reasoning_tokens": usage.completion_tokens_details.reasoning_tokens,
    })
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
        ContextUsage, ContextUsageColor, StreamingTraceForwarder, format_token_count,
        model_context_window_tokens, render_progress_bar,
    };
    use crate::{
        streaming::{AgentEventSink, AgentStreamEvent},
        trace::{TraceEventType, TraceSink},
    };
    use serde_json::Value;
    use std::sync::Mutex;

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

    #[derive(Default)]
    struct RecordingTraceSink {
        events: Mutex<Vec<(TraceEventType, Value)>>,
    }

    impl TraceSink for RecordingTraceSink {
        fn emit(&self, event_type: TraceEventType, payload: Value) {
            self.events.lock().unwrap().push((event_type, payload));
        }
    }

    #[test]
    fn streaming_trace_forwarder_emits_reasoning_and_final_answer_output() {
        let sink = RecordingTraceSink::default();
        let mut forwarder = StreamingTraceForwarder::new("model_1".into(), &sink);

        forwarder
            .on_event(&AgentStreamEvent::ReasoningDelta("Thinking.".into()))
            .unwrap();
        forwarder
            .on_event(&AgentStreamEvent::AnswerStarted)
            .unwrap();
        forwarder
            .on_event(&AgentStreamEvent::AnswerDelta("Hello".into()))
            .unwrap();

        let events = sink.events.lock().unwrap();
        assert_eq!(events[0].0, TraceEventType::ModelCallReasoningDelta);
        assert_eq!(events[0].1["node_id"], "model_1");
        assert_eq!(events[1].0, TraceEventType::ModelOutputStarted);
        assert_eq!(events[1].1["kind"], "final_answer");
        assert_eq!(events[2].0, TraceEventType::ModelOutputDelta);
        assert_eq!(events[2].1["content_delta"], "Hello");
    }

    #[test]
    fn streaming_trace_forwarder_emits_tool_call_output() {
        let sink = RecordingTraceSink::default();
        let mut forwarder = StreamingTraceForwarder::new("model_1".into(), &sink);

        forwarder
            .on_event(&AgentStreamEvent::ToolCallDelta {
                index: 0,
                id: Some("call_1".into()),
                name: Some("read_file".into()),
                arguments_delta: Some(r#"{"path":"Cargo.toml"}"#.into()),
            })
            .unwrap();

        let events = sink.events.lock().unwrap();
        assert_eq!(events[0].0, TraceEventType::ModelOutputStarted);
        assert_eq!(events[0].1["kind"], "tool_calls");
        assert_eq!(events[1].0, TraceEventType::ModelOutputDelta);
        assert_eq!(events[1].1["tool_call"]["tool_call_id"], "call_1");
        assert!(forwarder.output_node_id().is_some());
    }
}
