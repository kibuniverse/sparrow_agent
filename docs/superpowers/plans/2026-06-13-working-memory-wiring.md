# WorkingMemory 接通 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connect the dormant `WorkingMemory` module by adding a model-callable `updateMemory` tool whose structured writes are buffered and applied into `ContextManager` before each model request.

**Architecture:** A new `MemoryToolProvider` holds an `Arc<Mutex<Vec<MemoryDelta>>>` pending-delta buffer. On tool execution it parses the call, pushes a `MemoryDelta`, and returns an ack. The `Agent` holds a clone of that Arc and, at the top of `build_request` (before `compile_request`), drains the buffer into `ContextManager` via `apply_memory_delta`. `ContextManager` stays the sole writer of `working_memory`; the compiler already injects memory into requests under budget control. Sub-agents are isolated because `updateMemory` is excluded from `SubAgentConfig.allowed_tools`.

**Tech Stack:** Rust 2024 edition, tokio, serde_json, async-trait. Tests: inline `#[cfg(test)]` modules + a `tests/` contract test, run with `cargo test`.

**Spec:** `docs/superpowers/specs/2026-06-13-working-memory-wiring-design.md`

---

## File Structure

| File | Responsibility | Action |
|---|---|---|
| `src/context/memory.rs` | `WorkingMemory` data model + the new `MemoryDelta`/`MemoryScope`/`DeltaOutcome` types and `apply_delta` mutation logic | Modify |
| `src/context/mod.rs` | Re-export new public types from the `context` module | Modify |
| `src/context/manager.rs` | `ContextManager::apply_memory_delta` — single delegation point to `working_memory` | Modify |
| `src/config.rs` | `MemoryConfig` (enabled toggle) + wire into `AppConfig` + extend default system prompt | Modify |
| `src/memory_tool.rs` | `MemoryToolProvider` (tool def + arg parsing + delta buffering) | Create |
| `src/lib.rs` | Declare `pub mod memory_tool;` | Modify |
| `src/agent.rs` | `memory_buffer` field, provider registration, `drain_memory()`, call site in `build_request` | Modify |
| `tests/memory_tool_contract.rs` | Contract test for the provider | Create |

Task order is chosen so the crate compiles after every task.

---

### Task 1: MemoryDelta model + `apply_delta` in `memory.rs`

**Files:**
- Modify: `src/context/memory.rs` (append types + method + test module)

This task has real logic → test-driven. The three enum types are added first so the test module compiles structurally, then the tests are written against the not-yet-existing `apply_delta` (red), then the method is implemented (green).

- [ ] **Step 1: Add the enum types**

Append to `src/context/memory.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryScope {
    All,
    Facts,
    Decisions,
    OpenQuestions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryDelta {
    SetGoal { goal: String },
    AddFact { content: String },
    AddDecision { content: String },
    AddQuestion { content: String },
    ResolveQuestion { needle: String },
    RemoveFact { needle: String },
    Clear { scope: MemoryScope },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaOutcome {
    Applied,
    NotFound,
    Cleared(usize),
}
```

- [ ] **Step 2: Write the failing test module**

Append the test module at the end of `src/context/memory.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::budget::TokenEstimator;

    fn item(text: &str) -> MemoryItem {
        MemoryItem::new(text)
    }

    #[test]
    fn set_goal_overwrites_and_empty_clears() {
        let mut mem = WorkingMemory::default();
        assert_eq!(
            mem.apply_delta(&MemoryDelta::SetGoal {
                goal: "ship v1".into()
            }),
            DeltaOutcome::Applied
        );
        assert_eq!(mem.task_goal.as_deref(), Some("ship v1"));

        assert_eq!(
            mem.apply_delta(&MemoryDelta::SetGoal {
                goal: "   ".into()
            }),
            DeltaOutcome::Applied
        );
        assert_eq!(mem.task_goal, None);
    }

    #[test]
    fn add_fact_appends_and_ignores_empty() {
        let mut mem = WorkingMemory::default();
        assert_eq!(
            mem.apply_delta(&MemoryDelta::AddFact {
                content: "rust edition 2024".into()
            }),
            DeltaOutcome::Applied
        );
        assert_eq!(
            mem.apply_delta(&MemoryDelta::AddFact {
                content: "  ".into()
            }),
            DeltaOutcome::NotFound
        );
        assert_eq!(mem.durable_facts.len(), 1);
        assert_eq!(mem.durable_facts[0].content, "rust edition 2024");
    }

    #[test]
    fn remove_fact_removes_first_case_insensitive_match() {
        let mut mem = WorkingMemory::default();
        mem.durable_facts.push(item("API base is /v1"));
        mem.durable_facts.push(item("API base is /v2"));

        assert_eq!(
            mem.apply_delta(&MemoryDelta::RemoveFact {
                needle: "api base".into()
            }),
            DeltaOutcome::Applied
        );
        assert_eq!(mem.durable_facts.len(), 1);
        assert_eq!(mem.durable_facts[0].content, "API base is /v2");
    }

    #[test]
    fn remove_fact_no_match_is_not_found() {
        let mut mem = WorkingMemory::default();
        mem.durable_facts.push(item("hello"));

        assert_eq!(
            mem.apply_delta(&MemoryDelta::RemoveFact {
                needle: "missing".into()
            }),
            DeltaOutcome::NotFound
        );
        assert_eq!(mem.durable_facts.len(), 1);
    }

    #[test]
    fn resolve_question_removes_first_match() {
        let mut mem = WorkingMemory::default();
        mem.open_questions.push(item("Which DB?"));
        mem.open_questions.push(item("Which port?"));

        assert_eq!(
            mem.apply_delta(&MemoryDelta::ResolveQuestion {
                needle: "port".into()
            }),
            DeltaOutcome::Applied
        );
        assert_eq!(mem.open_questions.len(), 1);
        assert_eq!(mem.open_questions[0].content, "Which DB?");
    }

    #[test]
    fn clear_scopes_correctly() {
        let mut mem = WorkingMemory::default();
        mem.durable_facts.push(item("a"));
        mem.decisions.push(item("d"));
        mem.open_questions.push(item("q"));
        mem.task_goal = Some("g".into());

        assert_eq!(
            mem.apply_delta(&MemoryDelta::Clear {
                scope: MemoryScope::Facts
            }),
            DeltaOutcome::Cleared(1)
        );
        assert!(mem.durable_facts.is_empty());
        assert_eq!(mem.decisions.len(), 1);

        assert_eq!(
            mem.apply_delta(&MemoryDelta::Clear {
                scope: MemoryScope::All
            }),
            DeltaOutcome::Cleared(2)
        );
        assert!(mem.decisions.is_empty() && mem.open_questions.is_empty());
        assert_eq!(mem.task_goal, None);
    }

    #[test]
    fn apply_then_render_reflects_state() {
        let mut mem = WorkingMemory::default();
        mem.apply_delta(&MemoryDelta::SetGoal {
            goal: "migrate config".into(),
        });
        mem.apply_delta(&MemoryDelta::AddFact {
            content: "config is toml".into(),
        });

        let message = mem.to_context_message(&TokenEstimator, 4_096).unwrap();
        let content = message.content.unwrap();
        assert!(content.contains("Task goal"));
        assert!(content.contains("migrate config"));
        assert!(content.contains("config is toml"));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib context::memory::`
Expected: COMPILE ERROR — the test module calls `mem.apply_delta(...)`, but `apply_delta` is not yet defined. This is the red state.

- [ ] **Step 4: Implement `apply_delta` and helpers**

Add these methods inside the existing `impl WorkingMemory { ... }` block (after `to_context_message`):

```rust
    pub fn apply_delta(&mut self, delta: &MemoryDelta) -> DeltaOutcome {
        let outcome = match delta {
            MemoryDelta::SetGoal { goal } => {
                let goal = goal.trim();
                self.task_goal = if goal.is_empty() {
                    None
                } else {
                    Some(goal.to_string())
                };
                DeltaOutcome::Applied
            }
            MemoryDelta::AddFact { content } => push_non_empty(&mut self.durable_facts, content),
            MemoryDelta::AddDecision { content } => push_non_empty(&mut self.decisions, content),
            MemoryDelta::AddQuestion { content } => push_non_empty(&mut self.open_questions, content),
            MemoryDelta::ResolveQuestion { needle } => {
                remove_first_match(&mut self.open_questions, needle)
            }
            MemoryDelta::RemoveFact { needle } => remove_first_match(&mut self.durable_facts, needle),
            MemoryDelta::Clear { scope } => DeltaOutcome::Cleared(self.clear_scope(scope)),
        };

        if outcome != DeltaOutcome::NotFound {
            self.updated_at = Utc::now();
        }
        outcome
    }

    fn clear_scope(&mut self, scope: &MemoryScope) -> usize {
        match scope {
            MemoryScope::All => {
                let n = self.durable_facts.len() + self.decisions.len() + self.open_questions.len();
                self.durable_facts.clear();
                self.decisions.clear();
                self.open_questions.clear();
                self.task_goal = None;
                n
            }
            MemoryScope::Facts => clear_vec(&mut self.durable_facts),
            MemoryScope::Decisions => clear_vec(&mut self.decisions),
            MemoryScope::OpenQuestions => clear_vec(&mut self.open_questions),
        }
    }
```

Add free helper functions at the end of the file (after the existing `fn push_items`):

```rust
fn push_non_empty(items: &mut Vec<MemoryItem>, content: &str) -> DeltaOutcome {
    let content = content.trim();
    if content.is_empty() {
        DeltaOutcome::NotFound
    } else {
        items.push(MemoryItem::new(content));
        DeltaOutcome::Applied
    }
}

fn remove_first_match(items: &mut Vec<MemoryItem>, needle: &str) -> DeltaOutcome {
    let needle = needle.trim();
    if needle.is_empty() {
        return DeltaOutcome::NotFound;
    }
    let lower = needle.to_lowercase();
    match items
        .iter()
        .position(|item| item.content.to_lowercase().contains(&lower))
    {
        Some(index) => {
            items.remove(index);
            DeltaOutcome::Applied
        }
        None => DeltaOutcome::NotFound,
    }
}

fn clear_vec(items: &mut Vec<MemoryItem>) -> usize {
    let n = items.len();
    items.clear();
    n
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib context::memory::`
Expected: PASS — 7 tests.

- [ ] **Step 6: Commit**

```bash
git add src/context/memory.rs
git commit -m "feat(memory): add MemoryDelta model and apply_delta"
```

---

### Task 2: `ContextManager::apply_memory_delta` delegation

**Files:**
- Modify: `src/context/mod.rs:9-12` (re-exports)
- Modify: `src/context/manager.rs` (import + method + test)

- [ ] **Step 1: Re-export the new types**

In `src/context/mod.rs`, the current `pub use` block is:

```rust
pub use budget::{ContextPolicy, ReasoningRetentionPolicy};
pub use compiler::{CompileInput, CompiledContext, ContextCompileReport};
pub use manager::{CompactionReport, ContextManager};
pub use summary::{ContextSummarizer, FallbackContextSummarizer};
```

Add a line for the memory types:

```rust
pub use memory::{DeltaOutcome, MemoryDelta, MemoryScope};
```

- [ ] **Step 2: Add the delegation method**

In `src/context/manager.rs`, update the `memory::` import on line 6 from:

```rust
        memory::WorkingMemory,
```

to:

```rust
        memory::{DeltaOutcome, MemoryDelta, WorkingMemory},
```

Then add this method inside `impl ContextManager` (after `record_tool_result`, before `compile_request`):

```rust
    pub fn apply_memory_delta(&mut self, delta: MemoryDelta) -> DeltaOutcome {
        self.working_memory.apply_delta(&delta)
    }
```

- [ ] **Step 3: Write the failing test**

In the existing `#[cfg(test)] mod tests` of `src/context/manager.rs`, the import block (lines 113-121) is:

```rust
    use crate::{
        api::ChoiceMessage,
        context::{
            ContextManager, ContextSummarizer,
            budget::ContextPolicy,
            compiler::CompileInput,
            transcript::{ConversationTurn, TurnSummary},
        },
    };
```

Change it to also import `MemoryDelta` and `DeltaOutcome`:

```rust
    use crate::{
        api::ChoiceMessage,
        context::{
            ContextManager, ContextSummarizer, DeltaOutcome, MemoryDelta,
            budget::ContextPolicy,
            compiler::CompileInput,
            transcript::{ConversationTurn, TurnSummary},
        },
    };
```

Add this test inside the test module:

```rust
    #[test]
    fn apply_memory_delta_then_compile_injects_it() {
        let mut manager =
            ContextManager::new("system".into(), "deepseek-v4-pro".into(), ContextPolicy::default());

        let outcome = manager.apply_memory_delta(MemoryDelta::AddFact {
            content: "fact one".into(),
        });
        assert_eq!(outcome, DeltaOutcome::Applied);

        let compiled = manager.compile_request(CompileInput).unwrap();
        let memory_content = compiled.messages.iter().find_map(|message| {
            message
                .content
                .as_deref()
                .filter(|content| content.starts_with("Working memory for the ongoing task"))
        });
        let memory_content = memory_content.expect("working memory message should be injected");
        assert!(memory_content.contains("fact one"));
    }
```

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test --lib context::manager::tests::apply_memory_delta_then_compile_injects_it`
Expected: FAIL — method does not exist yet (if you added the test before Step 2's method). If you did Step 2 first, skip to Step 5.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --lib context::manager`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/context/mod.rs src/context/manager.rs
git commit -m "feat(memory): wire apply_memory_delta into ContextManager"
```

---

### Task 3: `MemoryConfig` toggle in `config.rs`

**Files:**
- Modify: `src/config.rs` (struct + default + from_env + AppConfig field + 2 construction sites + system prompt)

Config plumbing — no isolated unit test (env-dependent); verified by compile + later integration.

- [ ] **Step 1: Add the `MemoryConfig` struct**

In `src/config.rs`, immediately after the `ContextConfig` `impl` block ends (after line 687, the closing `}` of `impl ContextConfig`), insert:

```rust
// ── Memory config ─────────────────────────────────────────────────────

const DEFAULT_MEMORY_ENABLED: bool = true;

#[derive(Debug, Clone)]
pub struct MemoryConfig {
    pub enabled: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_MEMORY_ENABLED,
        }
    }
}

impl MemoryConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();
        config.enabled = read_env_value("SPARROW_MEMORY_ENABLED")
            .and_then(|value| value.parse::<bool>().ok())
            .unwrap_or(config.enabled);
        config
    }
}
```

- [ ] **Step 2: Add the field to `AppConfig`**

In the `AppConfig` struct definition (lines 49-64), change the tail:

```rust
    pub bash: BashConfig,
    pub sub_agent: SubAgentConfig,
    pub context: ContextConfig,
}
```

to:

```rust
    pub bash: BashConfig,
    pub sub_agent: SubAgentConfig,
    pub context: ContextConfig,
    pub memory: MemoryConfig,
}
```

- [ ] **Step 3: Wire into both construction sites**

There are two `Ok(Self { ... })` blocks in `AppConfig` (lines 110-124 in `load_or_initialize`, and lines 133-147 in `from_env`). In BOTH, change:

```rust
            sub_agent: SubAgentConfig::from_env(),
            context: ContextConfig::from_env(),
        })
```

to:

```rust
            sub_agent: SubAgentConfig::from_env(),
            context: ContextConfig::from_env(),
            memory: MemoryConfig::from_env(),
        })
```

- [ ] **Step 4: Extend the default system prompt**

Change the `DEFAULT_SYSTEM_PROMPT` constant (line 24) from:

```rust
const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant. when read the file, ignore the build targer files like target dir in rust project, ouptput or dist dir in frontend project.  do not reade the entire project dir tree. read the file in entry file first.";
```

to:

```rust
const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant. when read the file, ignore the build targer files like target dir in rust project, ouptput or dist dir in frontend project.  do not reade the entire project dir tree. read the file in entry file first. You have an `updateMemory` tool: call it to record the task goal, durable facts, decisions, and open questions that must survive across the conversation. Use it sparingly for things that matter later; use resolve_question and remove_fact to retire items that are no longer relevant.";
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly (the new field is initialized in both sites).

- [ ] **Step 6: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): add MemoryConfig toggle and prompt guidance"
```

---

### Task 4: `MemoryToolProvider` + contract test

**Files:**
- Create: `src/memory_tool.rs`
- Modify: `src/lib.rs`
- Create: `tests/memory_tool_contract.rs`

Real parsing logic → test-driven via the contract test. Declare the module and write the test first (red: module/provider not found), then create the provider (green).

- [ ] **Step 1: Declare the module**

In `src/lib.rs`, add (alphabetical placement is fine; e.g. after `pub mod local_tools;`):

```rust
pub mod memory_tool;
```

- [ ] **Step 2: Create the provider**

Create `src/memory_tool.rs`:

```rust
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
```

- [ ] **Step 3: Write the contract test**

Create `tests/memory_tool_contract.rs`:

```rust
use std::sync::Arc;

use sparrow_agent::{
    api::{FunctionCall, ToolCall},
    context::{MemoryDelta, MemoryScope},
    memory_tool::{MemoryToolProvider, new_memory_buffer},
    tool_provider::ToolProvider,
};

fn update_call(arguments: &str) -> ToolCall {
    ToolCall {
        id: "call_1".into(),
        kind: "function".into(),
        function: FunctionCall {
            name: "updateMemory".into(),
            arguments: arguments.into(),
        },
    }
}

#[tokio::test]
async fn queues_valid_add_fact_delta() {
    let buffer = new_memory_buffer();
    let provider = MemoryToolProvider::new(Arc::clone(&buffer));

    let result = provider
        .execute(&update_call(r#"{"operation":"add_fact","content":"the sky is blue"}"#))
        .await
        .unwrap()
        .unwrap();

    assert!(
        result.starts_with("Memory update queued: add_fact"),
        "unexpected ack: {result}"
    );

    let queued = buffer.lock().unwrap();
    assert_eq!(queued.len(), 1);
    match &queued[0] {
        MemoryDelta::AddFact { content } => assert_eq!(content, "the sky is blue"),
        other => panic!("expected AddFact, got {other:?}"),
    }
}

#[tokio::test]
async fn clear_defaults_to_all_scope() {
    let buffer = new_memory_buffer();
    let provider = MemoryToolProvider::new(Arc::clone(&buffer));

    provider
        .execute(&update_call(r#"{"operation":"clear"}"#))
        .await
        .unwrap();

    match &buffer.lock().unwrap()[0] {
        MemoryDelta::Clear { scope } => assert_eq!(*scope, MemoryScope::All),
        other => panic!("expected Clear, got {other:?}"),
    }
}

#[tokio::test]
async fn rejects_unknown_operation_without_queueing() {
    let buffer = new_memory_buffer();
    let provider = MemoryToolProvider::new(Arc::clone(&buffer));

    let result = provider
        .execute(&update_call(r#"{"operation":"teleport"}"#))
        .await;

    assert!(result.is_err(), "unknown operation must error");
    assert!(buffer.lock().unwrap().is_empty(), "nothing should be queued on error");
}

#[tokio::test]
async fn exposes_single_update_memory_definition() {
    let provider = MemoryToolProvider::new(new_memory_buffer());
    let definitions = provider.definitions();
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].function.name, "updateMemory");
}
```

- [ ] **Step 4: Run the contract test to verify it passes**

Run: `cargo test --test memory_tool_contract`
Expected: PASS — 4 tests (valid add_fact queuing, clear defaulting to all-scope, unknown-operation rejection without queueing, single definition exposed). The contract test exercises `parse_update_memory` end-to-end through `execute`.

- [ ] **Step 5: Commit**

```bash
git add src/memory_tool.rs src/lib.rs tests/memory_tool_contract.rs
git commit -m "feat(memory): add updateMemory tool provider"
```

---

### Task 5: Wire the provider + buffer + drain into `Agent`

**Files:**
- Modify: `src/agent.rs` (imports, struct field, `new_inner`, `build_request`, new `drain_memory`)

- [ ] **Step 1: Add imports**

In `src/agent.rs`, add a top-level import after line 6 (`use serde_json::{Value, json};`):

```rust
use std::sync::Arc;
```

In the existing `use crate::{ ... }` block (lines 8-23), add a line for the memory tool (e.g. after the `local_tools::LocalToolProvider` line):

```rust
    memory_tool::{MemoryBuffer, MemoryToolProvider, new_memory_buffer},
```

- [ ] **Step 2: Add the struct field**

Change the `Agent` struct (lines 28-34):

```rust
pub struct Agent {
    client: DeepSeekClient,
    config: AppConfig,
    context: ContextManager,
    tool_registry: ToolRegistry,
    context_usage: ContextUsage,
}
```

to:

```rust
pub struct Agent {
    client: DeepSeekClient,
    config: AppConfig,
    context: ContextManager,
    tool_registry: ToolRegistry,
    context_usage: ContextUsage,
    memory_buffer: MemoryBuffer,
}
```

- [ ] **Step 3: Create the buffer and register the provider in `new_inner`**

In `new_inner`, immediately after the line (line 61):

```rust
        let mut tool_registry = ToolRegistry::with_result_processor(tool_result_processor);
```

add:

```rust
        let memory_buffer = new_memory_buffer();
```

Then, immediately after the `LocalToolProvider` registration block (the `tool_registry.add_provider(Box::new(LocalToolProvider::new(...)));` call ending around line 85), add:

```rust
        if config.memory.enabled {
            tool_registry.add_provider(Box::new(MemoryToolProvider::new(Arc::clone(
                &memory_buffer,
            ))));
        }
```

- [ ] **Step 4: Store the buffer in the returned `Agent`**

In the `Ok(Self { ... })` at the end of `new_inner` (around line 152), add the field:

```rust
        Ok(Self {
            client,
            config,
            context,
            tool_registry,
            context_usage,
            memory_buffer,
        })
```

- [ ] **Step 5: Add `drain_memory` and call it from `build_request`**

Add this method to `impl Agent` (place it right before `fn build_request`):

```rust
    fn drain_memory(&mut self) {
        let deltas = match self.memory_buffer.lock() {
            Ok(mut buffer) => std::mem::take(&mut *buffer),
            Err(_) => {
                debug_log!("memory buffer lock poisoned; skipping drain");
                return;
            }
        };
        for delta in deltas {
            self.context.apply_memory_delta(delta);
        }
    }
```

In `build_request` (line 380), change:

```rust
    fn build_request(&mut self) -> Result<(ChatCompletionRequest, ContextCompileReport)> {
        let compiled = self.context.compile_request(CompileInput)?;
```

to:

```rust
    fn build_request(&mut self) -> Result<(ChatCompletionRequest, ContextCompileReport)> {
        self.drain_memory();
        let compiled = self.context.compile_request(CompileInput)?;
```

- [ ] **Step 6: Verify it compiles and existing tests still pass**

Run: `cargo test`
Expected: PASS — all prior tests (including the provider contract test and the memory model tests) compile and pass. No behavior change for sub-agents: `updateMemory` is not in `DEFAULT_SUB_AGENT_ALLOWED_TOOLS` (config.rs:43-47), so `restrict_to_allowed_tools` removes it from sub-agent registries.

- [ ] **Step 7: Commit**

```bash
git add src/agent.rs
git commit -m "feat(memory): wire updateMemory into agent loop"
```

---

### Task 6: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Full test suite**

Run: `cargo test`
Expected: PASS — every test, including the 7 memory model tests, the manager test, and the 4 contract tests.

- [ ] **Step 2: Clean release build**

Run: `cargo build`
Expected: builds with no errors.

- [ ] **Step 3: Lint (if toolchain has clippy)**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings. (If clippy is unavailable in the environment, skip this step and note it.)

- [ ] **Step 4: Confirm isolation invariant**

Inspect that `updateMemory` does not appear in `DEFAULT_SUB_AGENT_ALLOWED_TOOLS` (`src/config.rs`). It must not — sub-agents get it neither in definitions (filtered by `restrict_to_allowed_tools`) nor on request (rejected by `effective_allowed_tools` in `src/sub_agent.rs:327`).

---

## Out of scope (explicitly)

- **No frontend work.** `updateMemory` is an ordinary tool; the existing trace pipeline surfaces its calls/acks like any other tool call. No UI change needed.
- **No disk persistence.** Memory lives only for the agent's lifetime (decided in brainstorming).
- **No automatic extraction.** Writes are entirely model-driven via the tool.
- **No eviction policy.** The compiler already drops the memory message when over budget (`src/context/compiler.rs:106`); explicit `remove_fact`/`resolve_question`/`clear` handle logical retirement.

## Notes for the implementer

- `Utc::now()` is used inside `apply_delta` — `chrono::Utc` is already imported at the top of `src/context/memory.rs`.
- The `debug_log!` macro used in `drain_memory` is already in scope in `src/agent.rs` (imported via `use crate::{..., debug_log, ...}`).
- Sub-agents construct their own `Agent` via `new_with_tool_allowlist` → `new_inner`, so each gets its own fresh, empty `memory_buffer`. Their `drain_memory` is a no-op because they can never call the tool. This is the intended isolation.
- If `cargo test` in Task 5 Step 6 surfaces a dead-code warning for `MemoryToolProvider` before wiring, that resolves once Task 5 lands.
