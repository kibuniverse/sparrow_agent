use std::time::Instant;

use anyhow::Result;
use futures_util::future::join_all;
use serde_json::json;

use crate::{
    api::{ToolCall, ToolDef},
    debug_log,
    tool_provider::ToolProvider,
    trace::{DEFAULT_SNAPSHOT_MAX_BYTES, JsonSnapshot, TraceEventType, TraceSink, trace_id},
};

pub struct ToolRegistry {
    providers: Vec<Box<dyn ToolProvider>>,
    definitions: Vec<ToolDef>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            definitions: Vec::new(),
        }
    }

    pub fn add_provider(&mut self, provider: Box<dyn ToolProvider>) {
        debug_log!("Adding tool provider: {}", provider.id());
        self.definitions
            .extend(provider.definitions().iter().cloned());
        self.providers.push(provider);
    }

    pub fn definitions(&self) -> &[ToolDef] {
        &self.definitions
    }

    pub async fn execute_all(&self, tool_calls: &[ToolCall]) -> Vec<ToolExecutionResult> {
        join_all(tool_calls.iter().map(|tool_call| async move {
            debug_log!(
                "Executing tool: name={}, id={}, args={}",
                tool_call.function.name,
                tool_call.id,
                tool_call.function.arguments,
            );
            let content = match self.execute(tool_call).await {
                Ok(content) => {
                    debug_log!(
                        "Tool '{}' succeeded, result length: {}",
                        tool_call.function.name,
                        content.len()
                    );
                    content
                }
                Err(error) => {
                    debug_log!("Tool '{}' failed: {error}", tool_call.function.name);
                    format!("Tool execution failed: {error}")
                }
            };

            ToolExecutionResult {
                tool_call_id: tool_call.id.clone(),
                content,
            }
        }))
        .await
    }

    pub async fn execute_all_traced(
        &self,
        tool_calls: &[ToolCall],
        parent_model_output_id: &str,
        sink: &dyn TraceSink,
    ) -> Vec<ToolExecutionResult> {
        let started_calls = tool_calls
            .iter()
            .enumerate()
            .map(|(index, tool_call)| {
                let node_id = trace_id("tool");
                let started = Instant::now();

                sink.emit(
                    TraceEventType::ToolCallStarted,
                    json!({
                        "node_id": node_id,
                        "parent_model_output_id": parent_model_output_id,
                        "index": index,
                        "tool_call_id": tool_call.id,
                        "name": tool_call.function.name,
                        "arguments": JsonSnapshot::from_text(
                            &tool_call.function.arguments,
                            DEFAULT_SNAPSHOT_MAX_BYTES,
                        ),
                    }),
                );

                (node_id, started, tool_call)
            })
            .collect::<Vec<_>>();

        join_all(
            started_calls
                .into_iter()
                .map(|(node_id, started, tool_call)| async move {
                    debug_log!(
                        "Executing traced tool: name={}, id={}, args={}",
                        tool_call.function.name,
                        tool_call.id,
                        tool_call.function.arguments,
                    );

                    let content = match self.execute(tool_call).await {
                        Ok(content) => {
                            let duration_ms = started.elapsed().as_millis() as u64;
                            let output =
                                JsonSnapshot::from_text(&content, DEFAULT_SNAPSHOT_MAX_BYTES);
                            debug_log!(
                                "Traced tool '{}' succeeded, result length: {}",
                                tool_call.function.name,
                                content.len()
                            );
                            sink.emit(
                                TraceEventType::ToolCallCompleted,
                                json!({
                                    "node_id": node_id,
                                    "duration_ms": duration_ms,
                                    "output": output,
                                }),
                            );
                            content
                        }
                        Err(error) => {
                            let duration_ms = started.elapsed().as_millis() as u64;
                            let error_message = error.to_string();
                            debug_log!(
                                "Traced tool '{}' failed: {error_message}",
                                tool_call.function.name
                            );
                            sink.emit(
                                TraceEventType::ToolCallFailed,
                                json!({
                                    "node_id": node_id,
                                    "duration_ms": duration_ms,
                                    "error": error_message,
                                }),
                            );
                            format!("Tool execution failed: {error_message}")
                        }
                    };

                    ToolExecutionResult {
                        tool_call_id: tool_call.id.clone(),
                        content,
                    }
                }),
        )
        .await
    }

    async fn execute(&self, tool_call: &ToolCall) -> Result<String> {
        for provider in &self.providers {
            if let Some(result) = provider.execute(tool_call).await? {
                return Ok(result);
            }
        }
        anyhow::bail!("unknown tool: {}", tool_call.function.name);
    }
}

pub struct ToolExecutionResult {
    pub tool_call_id: String,
    pub content: String,
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use anyhow::Result;
    use serde_json::Value;
    use tokio::sync::Barrier;

    use super::ToolRegistry;
    use crate::{
        api::{FunctionCall, ToolCall, ToolDef},
        tool_provider::ToolProvider,
        trace::{TraceEventType, TraceSink},
    };

    struct StaticProvider {
        definitions: Vec<ToolDef>,
    }

    struct BarrierProvider {
        definitions: Vec<ToolDef>,
        barrier: Arc<Barrier>,
    }

    #[async_trait::async_trait]
    impl ToolProvider for StaticProvider {
        fn id(&self) -> &str {
            "static"
        }

        fn definitions(&self) -> &[ToolDef] {
            &self.definitions
        }

        async fn execute(&self, tool_call: &ToolCall) -> Result<Option<String>> {
            if tool_call.function.name == "knownTool" {
                return Ok(Some(r#"{"ok":true}"#.into()));
            }
            Ok(None)
        }
    }

    #[async_trait::async_trait]
    impl ToolProvider for BarrierProvider {
        fn id(&self) -> &str {
            "barrier"
        }

        fn definitions(&self) -> &[ToolDef] {
            &self.definitions
        }

        async fn execute(&self, tool_call: &ToolCall) -> Result<Option<String>> {
            if tool_call.function.name != "knownTool" {
                return Ok(None);
            }

            self.barrier.wait().await;
            Ok(Some(format!(r#"{{"id":"{}"}}"#, tool_call.id)))
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<(TraceEventType, Value)>>,
    }

    impl TraceSink for RecordingSink {
        fn emit(&self, event_type: TraceEventType, payload: Value) {
            self.events.lock().unwrap().push((event_type, payload));
        }
    }

    #[tokio::test]
    async fn traced_execution_emits_started_and_completed_events() {
        let mut registry = ToolRegistry::new();
        registry.add_provider(Box::new(StaticProvider {
            definitions: vec![ToolDef::function("knownTool", "Known tool")],
        }));
        let sink = RecordingSink::default();

        let results = registry
            .execute_all_traced(&[tool_call("call_1", "knownTool")], "output_1", &sink)
            .await;

        assert_eq!(results[0].content, r#"{"ok":true}"#);
        let events = sink.events.lock().unwrap();
        assert_eq!(events[0].0, TraceEventType::ToolCallStarted);
        assert_eq!(events[0].1["parent_model_output_id"], "output_1");
        assert_eq!(events[0].1["name"], "knownTool");
        assert_eq!(events[1].0, TraceEventType::ToolCallCompleted);
        assert_eq!(events[1].1["output"]["value"]["ok"], true);
    }

    #[tokio::test]
    async fn traced_execution_emits_failed_event_for_unknown_tool() {
        let registry = ToolRegistry::new();
        let sink = RecordingSink::default();

        let results = registry
            .execute_all_traced(&[tool_call("call_1", "missingTool")], "output_1", &sink)
            .await;

        assert!(results[0].content.starts_with("Tool execution failed:"));
        let events = sink.events.lock().unwrap();
        assert_eq!(events[0].0, TraceEventType::ToolCallStarted);
        assert_eq!(events[1].0, TraceEventType::ToolCallFailed);
        assert!(
            events[1].1["error"]
                .as_str()
                .unwrap()
                .contains("unknown tool")
        );
    }

    #[tokio::test]
    async fn traced_execution_starts_all_tools_before_any_completion() {
        let mut registry = ToolRegistry::new();
        registry.add_provider(Box::new(BarrierProvider {
            definitions: vec![ToolDef::function("knownTool", "Known tool")],
            barrier: Arc::new(Barrier::new(2)),
        }));
        let sink = RecordingSink::default();

        let results = tokio::time::timeout(
            Duration::from_millis(250),
            registry.execute_all_traced(
                &[
                    tool_call("call_1", "knownTool"),
                    tool_call("call_2", "knownTool"),
                ],
                "output_1",
                &sink,
            ),
        )
        .await
        .expect("tool calls should execute concurrently");

        assert_eq!(results.len(), 2);
        let events = sink.events.lock().unwrap();
        assert_eq!(events[0].0, TraceEventType::ToolCallStarted);
        assert_eq!(events[1].0, TraceEventType::ToolCallStarted);
        assert!(
            events[2..]
                .iter()
                .any(|event| event.0 == TraceEventType::ToolCallCompleted)
        );
        assert!(
            events[2..]
                .iter()
                .all(|event| event.0 != TraceEventType::ToolCallStarted)
        );
    }

    fn tool_call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            kind: "function".into(),
            function: FunctionCall {
                name: name.into(),
                arguments: r#"{"input":"value"}"#.into(),
            },
        }
    }
}
