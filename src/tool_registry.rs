use std::{collections::HashSet, time::Instant};

use anyhow::Result;
use futures_util::future::join_all;
use serde_json::{Value, json};

use crate::{
    api::{ToolCall, ToolDef},
    debug_log,
    tool_provider::{ToolExecutionTraceContext, ToolProvider},
    tool_result_processor::{ToolResultInput, ToolResultMetadata, ToolResultProcessor},
    trace::{DEFAULT_SNAPSHOT_MAX_BYTES, JsonSnapshot, TraceEventType, TraceSink, trace_id},
};

pub struct ToolRegistry {
    providers: Vec<Box<dyn ToolProvider>>,
    definitions: Vec<ToolDef>,
    result_processor: ToolResultProcessor,
    allowed_tool_names: Option<HashSet<String>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::with_result_processor(ToolResultProcessor::default())
    }

    pub fn with_result_processor(result_processor: ToolResultProcessor) -> Self {
        Self {
            providers: Vec::new(),
            definitions: Vec::new(),
            result_processor,
            allowed_tool_names: None,
        }
    }

    pub fn add_provider(&mut self, provider: Box<dyn ToolProvider>) {
        debug_log!("Adding tool provider: {}", provider.id());
        let allowed_tool_names = self.allowed_tool_names.as_ref();
        self.definitions
            .extend(provider.definitions().iter().filter_map(|definition| {
                allowed_tool_names
                    .is_none_or(|allowed| allowed.contains(&definition.function.name))
                    .then_some(definition.clone())
            }));
        self.providers.push(provider);
    }

    pub fn definitions(&self) -> &[ToolDef] {
        &self.definitions
    }

    pub fn restrict_to_allowed_tools(
        &mut self,
        allowed_tool_names: impl IntoIterator<Item = String>,
    ) {
        let allowed_tool_names = allowed_tool_names.into_iter().collect::<HashSet<_>>();
        self.definitions
            .retain(|definition| allowed_tool_names.contains(&definition.function.name));
        self.allowed_tool_names = Some(allowed_tool_names);
    }

    pub async fn execute_all(&self, tool_calls: &[ToolCall]) -> Vec<ToolExecutionResult> {
        join_all(tool_calls.iter().map(|tool_call| async move {
            debug_log!(
                "Executing tool: name={}, id={}, args={}",
                tool_call.function.name,
                tool_call.id,
                tool_call.function.arguments,
            );
            let (content, metadata) = match self.execute_and_process(tool_call).await {
                Ok(record) => {
                    debug_log!(
                        "Tool '{}' succeeded, original chars: {}, injected chars: {}, truncated: {}",
                        tool_call.function.name,
                        record.metadata.original_chars,
                        record.metadata.injected_chars,
                        record.metadata.truncated,
                    );
                    (record.content, record.metadata)
                }
                Err(error) => {
                    debug_log!("Tool '{}' failed: {error}", tool_call.function.name);
                    let content = format!("Tool execution failed: {error}");
                    let chars = content.chars().count();
                    (
                        content,
                        ToolResultMetadata {
                            original_chars: chars,
                            injected_chars: chars,
                            truncated: false,
                            artifact_path: None,
                        },
                    )
                }
            };

            ToolExecutionResult {
                tool_call_id: tool_call.id.clone(),
                content,
                metadata,
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
        let sink_context = sink.context();
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

                let trace_context =
                    sink_context
                        .as_ref()
                        .map(|parent_trace| ToolExecutionTraceContext {
                            parent_trace: parent_trace.clone(),
                            parent_model_output_id: parent_model_output_id.to_string(),
                            tool_call_id: tool_call.id.clone(),
                        });

                (node_id, started, tool_call, trace_context)
            })
            .collect::<Vec<_>>();

        join_all(
            started_calls
                .into_iter()
                .map(|(node_id, started, tool_call, trace_context)| async move {
                    debug_log!(
                        "Executing traced tool: name={}, id={}, args={}",
                        tool_call.function.name,
                        tool_call.id,
                        tool_call.function.arguments,
                    );

                    let (content, metadata) = match self
                        .execute_and_process_traced(tool_call, trace_context.as_ref())
                        .await
                    {
                        Ok(record) => {
                            let duration_ms = started.elapsed().as_millis() as u64;
                            let output =
                                JsonSnapshot::from_text(&record.content, DEFAULT_SNAPSHOT_MAX_BYTES);
                            let metadata = record.metadata;
                            debug_log!(
                                "Traced tool '{}' succeeded, original chars: {}, injected chars: {}, truncated: {}",
                                tool_call.function.name,
                                metadata.original_chars,
                                metadata.injected_chars,
                                metadata.truncated,
                            );
                            sink.emit(
                                TraceEventType::ToolCallCompleted,
                                tool_call_completed_payload(
                                    &node_id,
                                    duration_ms,
                                    output,
                                    &metadata,
                                    record.child_trace.as_ref(),
                                ),
                            );
                            (record.content, metadata)
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
                            let content = format!("Tool execution failed: {error_message}");
                            let chars = content.chars().count();
                            (
                                content,
                                ToolResultMetadata {
                                    original_chars: chars,
                                    injected_chars: chars,
                                    truncated: false,
                                    artifact_path: None,
                                },
                            )
                        }
                    };

                    ToolExecutionResult {
                        tool_call_id: tool_call.id.clone(),
                        content,
                        metadata,
                    }
                }),
        )
        .await
    }

    async fn execute_and_process(&self, tool_call: &ToolCall) -> Result<ToolExecutionRecord> {
        let content = self.execute(tool_call).await?;
        self.process_tool_content(tool_call, content)
    }

    async fn execute_and_process_traced(
        &self,
        tool_call: &ToolCall,
        trace_context: Option<&ToolExecutionTraceContext>,
    ) -> Result<ToolExecutionRecord> {
        let content = self.execute_traced(tool_call, trace_context).await?;
        self.process_tool_content(tool_call, content)
    }

    fn process_tool_content(
        &self,
        tool_call: &ToolCall,
        content: String,
    ) -> Result<ToolExecutionRecord> {
        let child_trace = child_trace_metadata_from_content(&content);
        let processed = self.result_processor.process(ToolResultInput {
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.function.name.clone(),
            content,
        })?;

        Ok(ToolExecutionRecord {
            content: processed.content,
            metadata: processed.metadata,
            child_trace,
        })
    }

    async fn execute(&self, tool_call: &ToolCall) -> Result<String> {
        if !self.tool_is_allowed(&tool_call.function.name) {
            anyhow::bail!(
                "tool '{}' is not allowed in this agent context",
                tool_call.function.name
            );
        }

        for provider in &self.providers {
            if let Some(result) = provider.execute(tool_call).await? {
                return Ok(result);
            }
        }
        anyhow::bail!("unknown tool: {}", tool_call.function.name);
    }

    async fn execute_traced(
        &self,
        tool_call: &ToolCall,
        trace_context: Option<&ToolExecutionTraceContext>,
    ) -> Result<String> {
        if !self.tool_is_allowed(&tool_call.function.name) {
            anyhow::bail!(
                "tool '{}' is not allowed in this agent context",
                tool_call.function.name
            );
        }

        for provider in &self.providers {
            if let Some(result) = provider.execute_traced(tool_call, trace_context).await? {
                return Ok(result);
            }
        }
        anyhow::bail!("unknown tool: {}", tool_call.function.name);
    }

    fn tool_is_allowed(&self, tool_name: &str) -> bool {
        self.allowed_tool_names
            .as_ref()
            .is_none_or(|allowed| allowed.contains(tool_name))
    }
}

pub struct ToolExecutionResult {
    pub tool_call_id: String,
    pub content: String,
    pub metadata: ToolResultMetadata,
}

struct ToolExecutionRecord {
    content: String,
    metadata: ToolResultMetadata,
    child_trace: Option<ToolChildTraceMetadata>,
}

struct ToolChildTraceMetadata {
    child_task_id: String,
    child_conversation_id: Option<String>,
}

fn tool_call_completed_payload(
    node_id: &str,
    duration_ms: u64,
    output: JsonSnapshot,
    metadata: &ToolResultMetadata,
    child_trace: Option<&ToolChildTraceMetadata>,
) -> Value {
    let mut payload = json!({
        "node_id": node_id,
        "duration_ms": duration_ms,
        "output": output,
        "output_metadata": tool_result_metadata_json(metadata),
    });

    if let (Some(child_trace), Value::Object(map)) = (child_trace, &mut payload) {
        map.insert(
            "child_task_id".into(),
            Value::String(child_trace.child_task_id.clone()),
        );
        if let Some(child_conversation_id) = &child_trace.child_conversation_id {
            map.insert(
                "child_conversation_id".into(),
                Value::String(child_conversation_id.clone()),
            );
        }
    }

    payload
}

fn child_trace_metadata_from_content(content: &str) -> Option<ToolChildTraceMetadata> {
    let value = serde_json::from_str::<Value>(content).ok()?;
    let child_task_id = value
        .get("child_task_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?
        .to_string();
    let child_conversation_id = value
        .get("child_conversation_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string);

    Some(ToolChildTraceMetadata {
        child_task_id,
        child_conversation_id,
    })
}

fn tool_result_metadata_json(metadata: &ToolResultMetadata) -> serde_json::Value {
    json!({
        "original_chars": metadata.original_chars,
        "injected_chars": metadata.injected_chars,
        "truncated": metadata.truncated,
        "artifact_path": metadata.artifact_path_display(),
    })
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

    struct LargeOutputProvider {
        definitions: Vec<ToolDef>,
        content: String,
    }

    struct ChildMetadataProvider {
        definitions: Vec<ToolDef>,
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

    #[async_trait::async_trait]
    impl ToolProvider for LargeOutputProvider {
        fn id(&self) -> &str {
            "large-output"
        }

        fn definitions(&self) -> &[ToolDef] {
            &self.definitions
        }

        async fn execute(&self, tool_call: &ToolCall) -> Result<Option<String>> {
            if tool_call.function.name == "largeTool" {
                return Ok(Some(self.content.clone()));
            }
            Ok(None)
        }
    }

    #[async_trait::async_trait]
    impl ToolProvider for ChildMetadataProvider {
        fn id(&self) -> &str {
            "child-metadata"
        }

        fn definitions(&self) -> &[ToolDef] {
            &self.definitions
        }

        async fn execute(&self, tool_call: &ToolCall) -> Result<Option<String>> {
            if tool_call.function.name == "runSubAgentTask" {
                return Ok(Some(
                    r#"{"status":"succeeded","final_answer":"done","summary":"done","child_task_id":"task_sub_123","child_conversation_id":"conv_sub_123","artifact_refs":[],"usage":{},"warnings":[]}"#
                        .into(),
                ));
            }
            Ok(None)
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
    async fn execute_all_truncates_large_tool_output_and_saves_artifact() {
        let temp = tempfile::tempdir().unwrap();
        let processor = crate::tool_result_processor::ToolResultProcessor::new(
            crate::tool_result_processor::ToolResultProcessorConfig {
                max_injected_chars: 420,
                output_dir: temp.path().join("tool_outputs"),
            },
        );
        let original = "large-output-line\n".repeat(80);
        let mut registry = ToolRegistry::with_result_processor(processor);
        registry.add_provider(Box::new(LargeOutputProvider {
            definitions: vec![ToolDef::function("largeTool", "Large output tool")],
            content: original.clone(),
        }));

        let results = registry
            .execute_all(&[tool_call("call_large", "largeTool")])
            .await;

        assert_eq!(results.len(), 1);
        assert!(results[0].metadata.truncated);
        assert!(results[0].content.contains("工具输出过长"));
        assert!(results[0].content.chars().count() <= 420);
        let artifact_path = results[0].metadata.artifact_path.as_ref().unwrap();
        assert_eq!(std::fs::read_to_string(artifact_path).unwrap(), original);
    }

    #[tokio::test]
    async fn execute_all_traced_emits_processed_output_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let processor = crate::tool_result_processor::ToolResultProcessor::new(
            crate::tool_result_processor::ToolResultProcessorConfig {
                max_injected_chars: 220,
                output_dir: temp.path().join("tool_outputs"),
            },
        );
        let original = "trace-large-output\n".repeat(80);
        let mut registry = ToolRegistry::with_result_processor(processor);
        registry.add_provider(Box::new(LargeOutputProvider {
            definitions: vec![ToolDef::function("largeTool", "Large output tool")],
            content: original.clone(),
        }));
        let sink = RecordingSink::default();

        let results = registry
            .execute_all_traced(&[tool_call("call_large", "largeTool")], "output_1", &sink)
            .await;

        assert!(results[0].metadata.truncated);
        let events = sink.events.lock().unwrap();
        let completed = events
            .iter()
            .find(|event| event.0 == TraceEventType::ToolCallCompleted)
            .unwrap();
        assert_eq!(completed.1["output_metadata"]["truncated"], true);
        assert_eq!(
            completed.1["output_metadata"]["original_chars"],
            original.chars().count(),
        );
        assert!(
            completed.1["output_metadata"]["artifact_path"]
                .as_str()
                .unwrap()
                .contains("tool_outputs")
        );
    }

    #[tokio::test]
    async fn traced_execution_emits_child_task_metadata_for_sub_agent_results() {
        let mut registry = ToolRegistry::new();
        registry.add_provider(Box::new(ChildMetadataProvider {
            definitions: vec![ToolDef::function("runSubAgentTask", "Run child task")],
        }));
        let sink = RecordingSink::default();

        let results = registry
            .execute_all_traced(
                &[tool_call("call_sub", "runSubAgentTask")],
                "output_1",
                &sink,
            )
            .await;

        assert_eq!(results.len(), 1);
        let events = sink.events.lock().unwrap();
        let completed = events
            .iter()
            .find(|event| event.0 == TraceEventType::ToolCallCompleted)
            .unwrap();
        assert_eq!(completed.1["child_task_id"], "task_sub_123");
        assert_eq!(completed.1["child_conversation_id"], "conv_sub_123");
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
    async fn restricted_registry_rejects_hidden_tool_execution() {
        let mut registry = ToolRegistry::new();
        registry.add_provider(Box::new(StaticProvider {
            definitions: vec![ToolDef::function("knownTool", "Known tool")],
        }));
        registry.restrict_to_allowed_tools(Vec::new());

        let results = registry
            .execute_all(&[tool_call("call_1", "knownTool")])
            .await;

        assert!(results[0].content.contains("not allowed"));
        assert!(registry.definitions().is_empty());
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
