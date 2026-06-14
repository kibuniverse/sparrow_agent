use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;

use crate::{
    api::{ToolCall, ToolDef},
    context::{MemoryDelta, MemoryScope},
    tool_provider::ToolProvider,
};

const UPDATE_MEMORY_TOOL: &str = "updateMemory";

/// Shared, pending buffer of memory operations written by the tool and
/// drained by the `Agent` before each model request.
pub type MemoryBuffer = Arc<Mutex<Vec<MemoryDelta>>>;

pub fn new_memory_buffer() -> MemoryBuffer {
    Arc::new(Mutex::new(Vec::new()))
}

pub struct MemoryToolProvider {
    buffer: MemoryBuffer,
    definitions: Vec<ToolDef>,
}

impl MemoryToolProvider {
    pub fn new(buffer: MemoryBuffer) -> Self {
        Self {
            buffer,
            definitions: vec![update_memory_tool()],
        }
    }
}

#[async_trait::async_trait]
impl ToolProvider for MemoryToolProvider {
    fn id(&self) -> &str {
        "memory"
    }

    fn definitions(&self) -> &[ToolDef] {
        &self.definitions
    }

    async fn execute(&self, tool_call: &ToolCall) -> Result<Option<String>> {
        if tool_call.function.name != UPDATE_MEMORY_TOOL {
            return Ok(None);
        }
        let operation_label = match self.buffer.lock() {
            Ok(mut buffer) => {
                let parsed = parse_update_memory(&tool_call.function.arguments)?;
                buffer.push(parsed.delta);
                parsed.operation_label
            }
            Err(_) => {
                return Ok(Some(
                    "Memory update failed: memory buffer unavailable.".to_string(),
                ));
            }
        };
        Ok(Some(format!(
            "Memory update queued: {operation_label}. It will apply on the next step."
        )))
    }
}

#[derive(Deserialize)]
struct UpdateMemoryArgs {
    operation: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    goal: Option<String>,
    #[serde(default)]
    needle: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

struct ParsedUpdate {
    delta: MemoryDelta,
    operation_label: &'static str,
}

fn parse_update_memory(arguments: &str) -> Result<ParsedUpdate> {
    let args: UpdateMemoryArgs = serde_json::from_str(arguments)
        .with_context(|| format!("invalid arguments for {UPDATE_MEMORY_TOOL}"))?;
    let (delta, operation_label) = match args.operation.as_str() {
        "set_goal" => (
            MemoryDelta::SetGoal {
                goal: args.goal.unwrap_or_default(),
            },
            "set_goal",
        ),
        "add_fact" => (
            MemoryDelta::AddFact {
                content: args.content.unwrap_or_default(),
            },
            "add_fact",
        ),
        "add_decision" => (
            MemoryDelta::AddDecision {
                content: args.content.unwrap_or_default(),
            },
            "add_decision",
        ),
        "add_question" => (
            MemoryDelta::AddQuestion {
                content: args.content.unwrap_or_default(),
            },
            "add_question",
        ),
        "resolve_question" => (
            MemoryDelta::ResolveQuestion {
                needle: args.needle.unwrap_or_default(),
            },
            "resolve_question",
        ),
        "remove_fact" => (
            MemoryDelta::RemoveFact {
                needle: args.needle.unwrap_or_default(),
            },
            "remove_fact",
        ),
        "clear" => (
            MemoryDelta::Clear {
                scope: parse_scope(args.scope.as_deref()),
            },
            "clear",
        ),
        other => anyhow::bail!("unknown operation '{other}' for {UPDATE_MEMORY_TOOL}"),
    };
    Ok(ParsedUpdate {
        delta,
        operation_label,
    })
}

fn parse_scope(scope: Option<&str>) -> MemoryScope {
    match scope {
        Some("facts") => MemoryScope::Facts,
        Some("decisions") => MemoryScope::Decisions,
        Some("open_questions") => MemoryScope::OpenQuestions,
        _ => MemoryScope::All,
    }
}

fn update_memory_tool() -> ToolDef {
    let mut tool = ToolDef::function(
        UPDATE_MEMORY_TOOL,
        "Persist a durable fact, decision, or open question for the ongoing task. Use sparingly for things that must survive conversation compaction. Each call performs exactly one operation.",
    );
    tool.function.parameters = Some(json!({
        "type": "object",
        "properties": {
            "operation": {
                "type": "string",
                "enum": ["set_goal","add_fact","add_decision","add_question","resolve_question","remove_fact","clear"],
                "description": "The memory operation to perform."
            },
            "content": {
                "type": "string",
                "description": "Text to store, for add_fact / add_decision / add_question."
            },
            "goal": {
                "type": "string",
                "description": "The task goal, for set_goal. An empty string clears the goal."
            },
            "needle": {
                "type": "string",
                "description": "Case-insensitive substring; the first matching item is removed, for resolve_question / remove_fact."
            },
            "scope": {
                "type": "string",
                "enum": ["all","facts","decisions","open_questions"],
                "description": "What to clear, for clear. Defaults to all."
            }
        },
        "required": ["operation"]
    }));
    tool
}
