use std::{collections::HashSet, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{sync::Semaphore, time::timeout};

use crate::{
    agent::Agent,
    api::{ToolCall, ToolDef},
    config::{AppConfig, SubAgentConfig},
    tool_provider::{ToolExecutionTraceContext, ToolProvider},
    trace::{TraceEventType, TraceSink, trace_id},
    trace_store::TraceStoreSink,
};

const RUN_SUB_AGENT_TASK_TOOL: &str = "runSubAgentTask";
const SUB_AGENT_TOOL_MAX_ROUNDS_CAP: usize = 20;
const RUN_BASH_COMMAND_TOOL: &str = "runBashCommand";

pub struct SubAgentToolProvider {
    config: AppConfig,
    coordinator: SubAgentCoordinator,
    runner: SubAgentRunner,
    definitions: Vec<ToolDef>,
    semaphore: Arc<Semaphore>,
}

impl SubAgentToolProvider {
    pub fn new(config: AppConfig) -> Self {
        let max_concurrent = config.sub_agent.max_concurrent.max(1);

        Self {
            coordinator: SubAgentCoordinator::new(config.sub_agent.clone()),
            runner: SubAgentRunner::new(config.clone()),
            definitions: vec![run_sub_agent_task_tool()],
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            config,
        }
    }
}

#[async_trait::async_trait]
impl ToolProvider for SubAgentToolProvider {
    fn id(&self) -> &str {
        "sub-agent"
    }

    fn definitions(&self) -> &[ToolDef] {
        &self.definitions
    }

    async fn execute(&self, tool_call: &ToolCall) -> Result<Option<String>> {
        self.execute_inner(tool_call, None).await
    }

    async fn execute_traced(
        &self,
        tool_call: &ToolCall,
        trace_context: Option<&ToolExecutionTraceContext>,
    ) -> Result<Option<String>> {
        self.execute_inner(tool_call, trace_context.cloned()).await
    }
}

impl SubAgentToolProvider {
    async fn execute_inner(
        &self,
        tool_call: &ToolCall,
        trace_context: Option<ToolExecutionTraceContext>,
    ) -> Result<Option<String>> {
        if tool_call.function.name != RUN_SUB_AGENT_TASK_TOOL {
            return Ok(None);
        }

        let request: SubAgentRequest = serde_json::from_str(&tool_call.function.arguments)
            .with_context(|| format!("invalid arguments for {RUN_SUB_AGENT_TASK_TOOL}"))?;

        let plan = match self
            .coordinator
            .check_eligibility(&request, self.config.max_tool_rounds)
        {
            Ok(plan) => plan,
            Err(result) => return Ok(Some(serde_json::to_string(&result)?)),
        };

        let child_task_id = plan.child_task_id.clone();
        let child_conversation_id = plan.child_conversation_id.clone();
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .context("sub-agent concurrency limiter closed")?;

        let timeout_ms = self.config.sub_agent.timeout_ms;
        let result = match timeout(
            Duration::from_millis(timeout_ms),
            self.runner.run(request, plan, trace_context.clone()),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                let warning = format!("sub-agent task timed out after {timeout_ms} ms");
                if let Some(trace_context) = &trace_context {
                    trace_context.parent_trace.store.mark_failed(
                        &child_task_id,
                        timeout_ms,
                        warning.clone(),
                    );
                }

                SubAgentResult::timeout(child_task_id, child_conversation_id, warning)
            }
        };

        drop(permit);
        Ok(Some(serde_json::to_string(&result)?))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubAgentRequest {
    pub task: String,
    pub context_pack: String,
    pub expected_output: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub max_tool_rounds: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct SubAgentRunPlan {
    pub task: String,
    pub context_pack: String,
    pub expected_output: String,
    pub allowed_tools: Vec<String>,
    pub max_tool_rounds: usize,
    pub child_task_id: String,
    pub child_conversation_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SubAgentResult {
    pub status: String,
    pub final_answer: String,
    pub summary: String,
    pub child_task_id: Option<String>,
    pub child_conversation_id: Option<String>,
    pub artifact_refs: Vec<String>,
    pub usage: SubAgentUsage,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl SubAgentResult {
    fn rejected(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            status: "rejected".into(),
            final_answer: String::new(),
            summary: reason.clone(),
            child_task_id: None,
            child_conversation_id: None,
            artifact_refs: Vec::new(),
            usage: SubAgentUsage::default(),
            warnings: Vec::new(),
            reason: Some(reason),
        }
    }

    fn failed(
        child_task_id: impl Into<String>,
        child_conversation_id: impl Into<String>,
        warning: impl Into<String>,
    ) -> Self {
        let warning = warning.into();
        Self {
            status: "failed".into(),
            final_answer: String::new(),
            summary: "sub-agent did not complete the task".into(),
            child_task_id: Some(child_task_id.into()),
            child_conversation_id: Some(child_conversation_id.into()),
            artifact_refs: Vec::new(),
            usage: SubAgentUsage::default(),
            warnings: vec![warning],
            reason: None,
        }
    }

    fn succeeded(
        final_answer: impl Into<String>,
        child_task_id: impl Into<String>,
        child_conversation_id: impl Into<String>,
        model: impl Into<String>,
        rounds: usize,
    ) -> Self {
        let final_answer = final_answer.into();
        Self {
            status: "succeeded".into(),
            summary: summarize_for_parent(&final_answer),
            final_answer,
            child_task_id: Some(child_task_id.into()),
            child_conversation_id: Some(child_conversation_id.into()),
            artifact_refs: Vec::new(),
            usage: SubAgentUsage {
                model: Some(model.into()),
                rounds: Some(rounds),
            },
            warnings: Vec::new(),
            reason: None,
        }
    }

    fn timeout(
        child_task_id: impl Into<String>,
        child_conversation_id: impl Into<String>,
        warning: impl Into<String>,
    ) -> Self {
        Self {
            status: "timeout".into(),
            final_answer: String::new(),
            summary: "sub-agent timed out before completing the task".into(),
            child_task_id: Some(child_task_id.into()),
            child_conversation_id: Some(child_conversation_id.into()),
            artifact_refs: Vec::new(),
            usage: SubAgentUsage::default(),
            warnings: vec![warning.into()],
            reason: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct SubAgentUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rounds: Option<usize>,
}

pub struct SubAgentCoordinator {
    config: SubAgentConfig,
}

impl SubAgentCoordinator {
    pub fn new(config: SubAgentConfig) -> Self {
        Self { config }
    }

    pub fn check_eligibility(
        &self,
        request: &SubAgentRequest,
        parent_max_tool_rounds: usize,
    ) -> std::result::Result<SubAgentRunPlan, SubAgentResult> {
        if !self.config.enabled {
            return Err(SubAgentResult::rejected("sub-agent execution is disabled"));
        }
        if self.config.max_depth == 0 {
            return Err(SubAgentResult::rejected(
                "sub-agent recursion depth limit has been reached",
            ));
        }
        if request.task.trim().is_empty() {
            return Err(SubAgentResult::rejected("task cannot be empty"));
        }
        if request.context_pack.trim().is_empty() {
            return Err(SubAgentResult::rejected("context_pack cannot be empty"));
        }
        if request.expected_output.trim().is_empty() {
            return Err(SubAgentResult::rejected("expected_output cannot be empty"));
        }
        if depends_on_parent_context(&request.context_pack) {
            return Err(SubAgentResult::rejected(
                "context_pack depends on the parent conversation instead of explicit facts",
            ));
        }

        let allowed_tools = self.effective_allowed_tools(request)?;
        let requested_rounds = request
            .max_tool_rounds
            .unwrap_or(self.config.max_tool_rounds)
            .max(1);
        let max_tool_rounds = requested_rounds
            .min(self.config.max_tool_rounds.max(1))
            .min(parent_max_tool_rounds.max(1))
            .min(SUB_AGENT_TOOL_MAX_ROUNDS_CAP);

        Ok(SubAgentRunPlan {
            task: request.task.clone(),
            context_pack: request.context_pack.clone(),
            expected_output: request.expected_output.clone(),
            allowed_tools,
            max_tool_rounds,
            child_task_id: trace_id("task_sub"),
            child_conversation_id: trace_id("conv_sub"),
        })
    }

    fn effective_allowed_tools(
        &self,
        request: &SubAgentRequest,
    ) -> std::result::Result<Vec<String>, SubAgentResult> {
        let configured = self
            .config
            .allowed_tools
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let requested_tools = if request.allowed_tools.is_empty() {
            self.config.allowed_tools.clone()
        } else {
            request.allowed_tools.clone()
        };

        let mut seen = HashSet::new();
        let mut effective = Vec::new();
        for tool_name in requested_tools {
            if tool_name == RUN_BASH_COMMAND_TOOL && !self.config.inherit_bash {
                return Err(SubAgentResult::rejected(format!(
                    "requested tool '{tool_name}' is not allowed because sub-agents do not inherit bash"
                )));
            }
            if !configured.contains(&tool_name) {
                return Err(SubAgentResult::rejected(format!(
                    "requested tool '{tool_name}' is not allowed for sub-agents"
                )));
            }
            if seen.insert(tool_name.clone()) {
                effective.push(tool_name);
            }
        }

        Ok(effective)
    }
}

pub struct SubAgentRunner {
    parent_config: AppConfig,
}

impl SubAgentRunner {
    pub fn new(parent_config: AppConfig) -> Self {
        Self { parent_config }
    }

    pub async fn run(
        &self,
        request: SubAgentRequest,
        plan: SubAgentRunPlan,
        trace_context: Option<ToolExecutionTraceContext>,
    ) -> SubAgentResult {
        let mut config = self.derive_child_config(&plan);
        config.system_prompt = sub_agent_system_prompt(&self.parent_config.system_prompt);
        let child_sink = trace_context
            .as_ref()
            .map(|trace_context| create_child_trace_sink(&plan, trace_context));

        let mut agent =
            match Agent::new_with_tool_allowlist(config.clone(), plan.allowed_tools).await {
                Ok(agent) => agent,
                Err(error) => {
                    return SubAgentResult::failed(
                        plan.child_task_id,
                        plan.child_conversation_id,
                        format!("failed to initialize child agent: {error}"),
                    );
                }
            };

        let prompt = sub_agent_user_prompt(&request);
        let result = if let Some(sink) = &child_sink {
            agent
                .handle_user_input_with_trace_result(prompt, sink)
                .await
        } else {
            let sink = NoopTraceSink;
            agent
                .handle_user_input_with_trace_result(prompt, &sink)
                .await
        };

        match result {
            Ok(final_answer) => SubAgentResult::succeeded(
                final_answer,
                plan.child_task_id,
                plan.child_conversation_id,
                config.model,
                config.max_tool_rounds,
            ),
            Err(error) => SubAgentResult::failed(
                plan.child_task_id,
                plan.child_conversation_id,
                error.to_string(),
            ),
        }
    }

    fn derive_child_config(&self, plan: &SubAgentRunPlan) -> AppConfig {
        let mut config = self.parent_config.clone();
        config.max_tool_rounds = plan.max_tool_rounds;
        config.sub_agent.max_depth = config.sub_agent.max_depth.saturating_sub(1);
        config.sub_agent.enabled = config.sub_agent.enabled && config.sub_agent.max_depth > 0;

        if !self.parent_config.sub_agent.inherit_filesystem {
            config.filesystem.enabled = false;
            for server in &mut config.mcp_servers {
                server.enabled = false;
            }
        }

        config.bash.enabled =
            self.parent_config.bash.enabled && self.parent_config.sub_agent.inherit_bash;
        config
    }
}

struct NoopTraceSink;

impl TraceSink for NoopTraceSink {
    fn emit(&self, _event_type: TraceEventType, _payload: serde_json::Value) {}
}

fn create_child_trace_sink(
    plan: &SubAgentRunPlan,
    trace_context: &ToolExecutionTraceContext,
) -> ParentLinkedTraceSink {
    let child_client_message_id = trace_id("msg_sub");
    trace_context.parent_trace.store.create_task_with_id(
        plan.child_task_id.clone(),
        plan.child_conversation_id.clone(),
        child_client_message_id,
    );
    let inner = TraceStoreSink::new(
        Arc::clone(&trace_context.parent_trace.store),
        plan.child_task_id.clone(),
    );

    ParentLinkedTraceSink::new(
        inner,
        SubAgentParentTrace {
            parent_task_id: trace_context.parent_trace.task_id.clone(),
            tool_call_id: trace_context.tool_call_id.clone(),
            parent_model_output_id: trace_context.parent_model_output_id.clone(),
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SubAgentParentTrace {
    parent_task_id: String,
    tool_call_id: String,
    parent_model_output_id: String,
}

struct ParentLinkedTraceSink {
    inner: TraceStoreSink,
    parent: SubAgentParentTrace,
}

impl ParentLinkedTraceSink {
    fn new(inner: TraceStoreSink, parent: SubAgentParentTrace) -> Self {
        Self { inner, parent }
    }
}

impl TraceSink for ParentLinkedTraceSink {
    fn emit(&self, event_type: TraceEventType, payload: Value) {
        let payload = if event_type == TraceEventType::TaskStarted {
            payload_with_parent_trace(payload, &self.parent)
        } else {
            payload
        };

        self.inner.emit(event_type, payload);
    }

    fn context(&self) -> Option<crate::trace::TraceSinkContext> {
        self.inner.context()
    }
}

fn payload_with_parent_trace(mut payload: Value, parent: &SubAgentParentTrace) -> Value {
    let parent_payload = json!({
        "task_id": parent.parent_task_id,
        "tool_call_id": parent.tool_call_id,
        "parent_model_output_id": parent.parent_model_output_id,
    });

    match &mut payload {
        Value::Object(map) => {
            map.insert("parent".into(), parent_payload);
            payload
        }
        _ => json!({
            "payload": payload,
            "parent": parent_payload,
        }),
    }
}

fn run_sub_agent_task_tool() -> ToolDef {
    let mut tool = ToolDef::function(
        RUN_SUB_AGENT_TASK_TOOL,
        "Run a self-contained sub-agent task in an isolated message context. Use this only when the task can be completed independently from the current conversation after the required context is provided explicitly. The parent agent receives only the final structured result.",
    );
    tool.function.parameters = Some(json!({
        "type": "object",
        "properties": {
            "task": {
                "type": "string",
                "description": "The exact self-contained task for the sub-agent."
            },
            "context_pack": {
                "type": "string",
                "description": "Only the facts, file paths, constraints, and acceptance criteria required to complete the task."
            },
            "expected_output": {
                "type": "string",
                "description": "The required result format, for example summary, JSON, findings list, implementation notes, or artifact references."
            },
            "allowed_tools": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional narrow list of tools the sub-agent may use. Empty means use the configured sub-agent default."
            },
            "max_tool_rounds": {
                "type": ["integer", "null"],
                "minimum": 1,
                "maximum": SUB_AGENT_TOOL_MAX_ROUNDS_CAP,
                "description": "Optional stricter maximum tool rounds for this subtask."
            }
        },
        "required": ["task", "context_pack", "expected_output"],
        "additionalProperties": false
    }));
    tool
}

fn depends_on_parent_context(context_pack: &str) -> bool {
    let lower = context_pack.to_ascii_lowercase();
    let implicit_markers = [
        "see above",
        "as above",
        "as discussed",
        "we discussed",
        "previously discussed",
        "the previous context",
        "parent conversation",
        "见上文",
        "如上",
        "上文",
        "刚才",
        "之前讨论",
        "前面说",
    ];

    implicit_markers.iter().any(|marker| lower.contains(marker))
}

fn sub_agent_system_prompt(parent_system_prompt: &str) -> String {
    let sub_agent_rules = [
        "You are a focused sub-agent running in an isolated message context.",
        "Complete only the task provided by the user message.",
        "Use only explicit facts from the task and context pack.",
        "Do not ask the parent agent for hidden context or rely on prior conversation.",
        "Return the final answer in the requested format.",
    ]
    .join(" ");

    format!(
        "{}\n\nSub-agent execution rules:\n{sub_agent_rules}",
        parent_system_prompt.trim()
    )
}

fn sub_agent_user_prompt(request: &SubAgentRequest) -> String {
    format!(
        "Task:\n{}\n\nContext pack:\n{}\n\nExpected output:\n{}",
        request.task.trim(),
        request.context_pack.trim(),
        request.expected_output.trim()
    )
}

fn summarize_for_parent(final_answer: &str) -> String {
    const MAX_SUMMARY_CHARS: usize = 400;
    let trimmed = final_answer.trim();
    if trimmed.chars().count() <= MAX_SUMMARY_CHARS {
        return trimmed.to_string();
    }

    trimmed.chars().take(MAX_SUMMARY_CHARS).collect::<String>()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{
        trace::{TraceEventType, TraceSinkContext},
        trace_store::TraceStore,
    };

    use super::*;

    #[test]
    fn parent_linked_trace_sink_adds_parent_metadata_to_child_task_started() {
        let store = Arc::new(TraceStore::new());
        let plan = SubAgentRunPlan {
            task: "Read local files".into(),
            context_pack: "Root: /tmp/project".into(),
            expected_output: "Return findings".into(),
            allowed_tools: Vec::new(),
            max_tool_rounds: 1,
            child_task_id: "task_sub_fixed".into(),
            child_conversation_id: "conv_sub_fixed".into(),
        };
        let trace_context = ToolExecutionTraceContext {
            parent_trace: TraceSinkContext {
                store: Arc::clone(&store),
                task_id: "task_parent".into(),
            },
            tool_call_id: "call_sub".into(),
            parent_model_output_id: "output_parent".into(),
        };
        let sink = create_child_trace_sink(&plan, &trace_context);

        assert_eq!(
            store.snapshot("task_sub_fixed").unwrap().conversation_id,
            "conv_sub_fixed"
        );

        sink.emit(
            TraceEventType::TaskStarted,
            json!({
                "message": {
                    "role": "user",
                    "content": "child prompt"
                }
            }),
        );

        let snapshot = store.snapshot("task_sub_fixed").unwrap();
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0].event_type, TraceEventType::TaskStarted);
        assert_eq!(
            snapshot.events[0].payload["parent"]["task_id"],
            "task_parent"
        );
        assert_eq!(
            snapshot.events[0].payload["parent"]["tool_call_id"],
            "call_sub"
        );
        assert_eq!(
            snapshot.events[0].payload["parent"]["parent_model_output_id"],
            "output_parent"
        );
    }
}
