# Tool Result Post Processing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 增加一个统一的工具调用后处理单元：当工具调用成功结果超过 20,000 个字符时，返回给模型的 `tool` message 自动截断，并把完整原始输出保存到本地文件。

**Architecture:** 在 `ToolRegistry` 调用具体 `ToolProvider` 成功拿到原始 `String` 后、构造 `ToolExecutionResult` 前插入 `ToolResultProcessor`。该处理器负责按字符数判断是否超限、持久化完整原始输出、生成给模型看的截断提示和预览；`execute_all` 与 `execute_all_traced` 共用同一处理路径，避免 trace 和非 trace 行为分叉。

**Tech Stack:** Rust 2024, Tokio, `anyhow`, `serde_json`, `ulid`, Cargo tests.

---

## Requirement Interpretation

用户需求包含三件事：

1. 工具调用结果长度超过 `20000` 个字符时触发截断逻辑。
2. 超出的内容不能直接丢弃，需要写入本地文件。
3. 返回给大模型的内容前面要拼接提示，明确说明本次工具输出过长、原始总长度为多少、已被截断、完整输出保存在哪个文件，并建议模型用更精确参数重新调用或换工具。

这里有一个小的口径统一：提示文案要求“完整的输出已经被保存在文件 XX 中”，所以落盘文件应保存完整原始输出，而不是只保存截断后的剩余尾部。返回给模型的 `tool` message 只注入提示加预算内预览。

## Current Code Context

- `src/tool_registry.rs` 负责分发工具调用，当前 `execute_all()` 和 `execute_all_traced()` 都直接把 provider 返回的 `String` 写入 `ToolExecutionResult.content`。
- `src/agent.rs` 在 `handle_assistant_message()` 与 `handle_assistant_message_with_trace()` 中把 `ToolExecutionResult.content` 追加为 `ChatMessage::tool(...)`，这正是需要控制进入模型上下文的边界。
- `src/trace.rs` 已有 `JsonSnapshot` 和 `DEFAULT_SNAPSHOT_MAX_BYTES`，它只影响可视化 trace 快照，不会阻止完整工具结果进入下一轮模型请求。
- `src/config.rs` 已用环境变量配置 filesystem 和 streaming，可沿用这个模式增加工具结果后处理配置。

## File Structure

- Create: `src/tool_result_processor.rs`
  - 定义 `ToolResultProcessor`、配置、输入、输出和元数据。
  - 负责字符计数、UTF-8 安全截断、文件名清洗、完整输出落盘和模型提示拼接。
  - 包含单元测试。
- Modify: `src/lib.rs`
  - 暴露 `tool_result_processor` 模块。
- Modify: `src/config.rs`
  - 增加 `ToolResultConfig`，从环境变量读取最大注入字符数和输出目录。
- Modify: `src/agent.rs`
  - 使用 `AppConfig.tool_results` 构造 `ToolResultProcessor`，注入 `ToolRegistry`。
- Modify: `src/tool_registry.rs`
  - 持有 `ToolResultProcessor`。
  - 在工具调用成功后统一调用处理器。
  - 扩展 `ToolExecutionResult`，携带处理元数据。
  - trace 模式下在 `tool_call.completed` payload 中附带元数据，方便前端或日志展示。
- Modify: `.gitignore`
  - 忽略 `/.sparrow_agent/tool_outputs/`，避免运行时大输出进入 git 状态。

## Behavior Contract

- 阈值按 Rust `char` 计数，不按 byte 计数，避免中文和 emoji 被错误计算。
- 当原始工具输出字符数 `<= max_injected_chars` 时：
  - 不写文件。
  - 返回给模型的内容保持原样。
  - `metadata.truncated = false`。
- 当原始工具输出字符数 `> max_injected_chars` 时：
  - 将完整原始输出写入本地文件。
  - 返回给模型的内容为“提示文案 + 原始输出前缀预览”。
  - 预览部分使用 `max_injected_chars - notice_chars` 的剩余预算；在正常配置路径下，最终 `tool` message 总字符数不超过 `max_injected_chars`。如果用户配置了极长输出目录导致提示文案本身超过预算，则保留完整提示文案并省略预览，避免截断文件路径。
  - `metadata.truncated = true`，并记录原始字符数、注入字符数和文件路径。
- 如果超限输出落盘失败：
  - 不返回误导性的“已保存”提示。
  - 将该工具调用视为失败，让现有错误路径生成 `Tool execution failed: ...`。

## User-Facing Tool Message

超限后返回给模型的内容格式固定为：

```text
工具输出过长：本次工具调用 `<tool_name>` 的结果总长度为 <original_chars> 个字符，已经被截断。完整的输出已经被保存在文件 `<artifact_path>` 中。请考虑以更精确的参数调用该函数，或者更换更适合检索、分页或摘要的函数。

--- 截断后的输出预览 ---
<preview>
```

其中 `<preview>` 是原始输出的前缀，长度为 `max_injected_chars - notice_chars`，保持 UTF-8 字符边界。

## Implementation Tasks

### Task 1: Add Tool Result Processor Unit

**Files:**
- Create: `src/tool_result_processor.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write processor tests**

Create `src/tool_result_processor.rs` with tests first. The tests define the public API and expected behavior:

```rust
#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        ToolResultInput, ToolResultProcessor, ToolResultProcessorConfig,
    };

    #[test]
    fn leaves_small_tool_output_unchanged() {
        let temp = tempdir().unwrap();
        let processor = ToolResultProcessor::new(ToolResultProcessorConfig {
            max_injected_chars: 80,
            output_dir: temp.path().join("tool_outputs"),
        });

        let processed = processor
            .process(ToolResultInput {
                tool_call_id: "call_small".to_string(),
                tool_name: "read_file".to_string(),
                content: "short output".to_string(),
            })
            .unwrap();

        assert_eq!(processed.content, "short output");
        assert!(!processed.metadata.truncated);
        assert_eq!(processed.metadata.original_chars, 12);
        assert_eq!(processed.metadata.injected_chars, 12);
        assert!(processed.metadata.artifact_path.is_none());
        assert!(!temp.path().join("tool_outputs").exists());
    }

    #[test]
    fn truncates_large_output_and_saves_complete_original_output() {
        let temp = tempdir().unwrap();
        let processor = ToolResultProcessor::new(ToolResultProcessorConfig {
            max_injected_chars: 700,
            output_dir: temp.path().join("tool_outputs"),
        });
        let original = "abcdef".repeat(160);

        let processed = processor
            .process(ToolResultInput {
                tool_call_id: "call_large".to_string(),
                tool_name: "read_file".to_string(),
                content: original.clone(),
            })
            .unwrap();

        assert!(processed.metadata.truncated);
        assert_eq!(processed.metadata.original_chars, original.chars().count());
        assert!(processed.metadata.injected_chars <= 700);
        assert!(processed.content.contains("工具输出过长"));
        assert!(processed.content.contains("结果总长度为 960 个字符"));
        assert!(processed.content.contains("已经被截断"));
        assert!(processed.content.contains("完整的输出已经被保存在文件"));
        assert!(processed.content.contains("请考虑以更精确的参数调用该函数"));
        assert!(processed.content.contains("--- 截断后的输出预览 ---"));

        let artifact_path = processed.metadata.artifact_path.as_ref().unwrap();
        assert!(artifact_path.starts_with(temp.path().join("tool_outputs")));
        assert_eq!(fs::read_to_string(artifact_path).unwrap(), original);
    }

    #[test]
    fn truncates_without_splitting_multibyte_characters() {
        let temp = tempdir().unwrap();
        let processor = ToolResultProcessor::new(ToolResultProcessorConfig {
            max_injected_chars: 700,
            output_dir: temp.path().join("tool_outputs"),
        });
        let original = "你好🙂".repeat(300);

        let processed = processor
            .process(ToolResultInput {
                tool_call_id: "call_unicode".to_string(),
                tool_name: "unicode_tool".to_string(),
                content: original.clone(),
            })
            .unwrap();

        assert!(processed.metadata.truncated);
        assert!(processed.metadata.injected_chars <= 700);
        assert!(processed.content.is_char_boundary(processed.content.len()));
        assert_eq!(
            fs::read_to_string(processed.metadata.artifact_path.unwrap()).unwrap(),
            original,
        );
    }

    #[test]
    fn sanitizes_tool_name_and_tool_call_id_in_artifact_filename() {
        let temp = tempdir().unwrap();
        let processor = ToolResultProcessor::new(ToolResultProcessorConfig {
            max_injected_chars: 160,
            output_dir: temp.path().join("tool_outputs"),
        });

        let processed = processor
            .process(ToolResultInput {
                tool_call_id: "call/with:unsafe chars".to_string(),
                tool_name: "mcp__filesystem__read/file".to_string(),
                content: "x".repeat(300),
            })
            .unwrap();

        let filename = processed
            .metadata
            .artifact_path
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        assert!(!filename.contains('/'));
        assert!(!filename.contains(':'));
        assert!(filename.contains("mcp__filesystem__read_file"));
        assert!(filename.contains("call_with_unsafe_chars"));
    }
}
```

Run:

```bash
cargo test tool_result_processor
```

Expected before implementation: compilation fails because the module types are not defined.

- [ ] **Step 2: Implement the processor**

Replace the top of `src/tool_result_processor.rs` with this implementation while keeping the tests below it:

```rust
use std::{
    fs,
    path::PathBuf,
};

use anyhow::{Context, Result};

pub const DEFAULT_TOOL_RESULT_MAX_CHARS: usize = 20_000;

#[derive(Debug, Clone)]
pub struct ToolResultProcessorConfig {
    pub max_injected_chars: usize,
    pub output_dir: PathBuf,
}

impl Default for ToolResultProcessorConfig {
    fn default() -> Self {
        Self {
            max_injected_chars: DEFAULT_TOOL_RESULT_MAX_CHARS,
            output_dir: PathBuf::from(".sparrow_agent/tool_outputs"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolResultProcessor {
    config: ToolResultProcessorConfig,
}

impl ToolResultProcessor {
    pub fn new(config: ToolResultProcessorConfig) -> Self {
        Self { config }
    }

    pub fn process(&self, input: ToolResultInput) -> Result<ProcessedToolResult> {
        let original_chars = input.content.chars().count();

        if original_chars <= self.config.max_injected_chars {
            return Ok(ProcessedToolResult {
                metadata: ToolResultMetadata {
                    original_chars,
                    injected_chars: original_chars,
                    truncated: false,
                    artifact_path: None,
                },
                content: input.content,
            });
        }

        fs::create_dir_all(&self.config.output_dir).with_context(|| {
            format!(
                "failed to create tool output directory {}",
                self.config.output_dir.display()
            )
        })?;

        let artifact_path = self.artifact_path(input.tool_call_id, input.tool_name);
        fs::write(&artifact_path, input.content.as_bytes()).with_context(|| {
            format!(
                "failed to save oversized tool output to {}",
                artifact_path.display()
            )
        })?;

        let notice = format!(
            "工具输出过长：本次工具调用 `{}` 的结果总长度为 {} 个字符，已经被截断。完整的输出已经被保存在文件 `{}` 中。请考虑以更精确的参数调用该函数，或者更换更适合检索、分页或摘要的函数。\n\n--- 截断后的输出预览 ---\n",
            input.tool_name,
            original_chars,
            artifact_path.display(),
        );

        let notice_chars = notice.chars().count();
        let preview_budget = self.config.max_injected_chars.saturating_sub(notice_chars);
        let preview = take_chars(&input.content, preview_budget);
        let content = format!("{notice}{preview}");
        let injected_chars = content.chars().count();

        Ok(ProcessedToolResult {
            content,
            metadata: ToolResultMetadata {
                original_chars,
                injected_chars,
                truncated: true,
                artifact_path: Some(artifact_path),
            },
        })
    }

    fn artifact_path(&self, tool_call_id: &str, tool_name: &str) -> PathBuf {
        let id = ulid::Ulid::new();
        let safe_tool_name = sanitize_filename_part(tool_name);
        let safe_tool_call_id = sanitize_filename_part(tool_call_id);
        self.config.output_dir.join(format!(
            "{id}-{safe_tool_name}-{safe_tool_call_id}.txt"
        ))
    }
}

impl Default for ToolResultProcessor {
    fn default() -> Self {
        Self::new(ToolResultProcessorConfig::default())
    }
}

#[derive(Debug)]
pub struct ToolResultInput {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ProcessedToolResult {
    pub content: String,
    pub metadata: ToolResultMetadata,
}

#[derive(Debug, Clone)]
pub struct ToolResultMetadata {
    pub original_chars: usize,
    pub injected_chars: usize,
    pub truncated: bool,
    pub artifact_path: Option<PathBuf>,
}

impl ToolResultMetadata {
    pub fn artifact_path_display(&self) -> Option<String> {
        self.artifact_path
            .as_ref()
            .map(|path| path.display().to_string())
    }
}

fn take_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn sanitize_filename_part(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() {
        "unknown".into()
    } else {
        trimmed.chars().take(80).collect()
    }
}
```

Run:

```bash
cargo test tool_result_processor
```

Expected after implementation: all `tool_result_processor` tests pass.

- [ ] **Step 3: Export the module**

In `src/lib.rs`, add:

```rust
pub mod tool_result_processor;
```

Run:

```bash
cargo test tool_result_processor
```

Expected: tests still pass.

### Task 2: Add Runtime Configuration

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Add config fields and defaults**

Import the default constant near the existing imports:

```rust
use crate::tool_result_processor::DEFAULT_TOOL_RESULT_MAX_CHARS;
```

Add this constant near the other defaults:

```rust
const DEFAULT_TOOL_OUTPUT_DIR: &str = ".sparrow_agent/tool_outputs";
```

Add the field to `AppConfig`:

```rust
pub tool_results: ToolResultConfig,
```

Set it in both `load_or_initialize()` and `from_env()`:

```rust
tool_results: ToolResultConfig::from_env(),
```

- [ ] **Step 2: Define `ToolResultConfig`**

Add this section before the streaming config section:

```rust
// ── Tool result config ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ToolResultConfig {
    pub max_injected_chars: usize,
    pub output_dir: PathBuf,
}

impl ToolResultConfig {
    pub fn from_env() -> Self {
        let max_injected_chars = env::var("SPARROW_TOOL_RESULT_MAX_CHARS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_TOOL_RESULT_MAX_CHARS);

        let output_dir = env::var("SPARROW_TOOL_OUTPUT_DIR")
            .ok()
            .and_then(|value| clean_value(&value))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_TOOL_OUTPUT_DIR));

        Self {
            max_injected_chars,
            output_dir,
        }
    }
}
```

Run:

```bash
cargo check
```

Expected before wiring the processor: may fail if the import or field has not been used consistently. Expected after this step is complete: `cargo check` passes.

### Task 3: Wire Processor Into Tool Registry

**Files:**
- Modify: `src/tool_registry.rs`

- [ ] **Step 1: Update imports and registry state**

Add imports:

```rust
use crate::{
    api::{ToolCall, ToolDef},
    debug_log,
    tool_provider::ToolProvider,
    tool_result_processor::{
        ProcessedToolResult, ToolResultInput, ToolResultMetadata, ToolResultProcessor,
    },
    trace::{DEFAULT_SNAPSHOT_MAX_BYTES, JsonSnapshot, TraceEventType, TraceSink, trace_id},
};
```

Add the processor field:

```rust
pub struct ToolRegistry {
    providers: Vec<Box<dyn ToolProvider>>,
    definitions: Vec<ToolDef>,
    result_processor: ToolResultProcessor,
}
```

Update constructors:

```rust
impl ToolRegistry {
    pub fn new() -> Self {
        Self::with_result_processor(ToolResultProcessor::default())
    }

    pub fn with_result_processor(result_processor: ToolResultProcessor) -> Self {
        Self {
            providers: Vec::new(),
            definitions: Vec::new(),
            result_processor,
        }
    }
}
```

- [ ] **Step 2: Add a shared execute-and-process method**

Add this private method inside `impl ToolRegistry`:

```rust
async fn execute_and_process(&self, tool_call: &ToolCall) -> Result<ProcessedToolResult> {
    let content = self.execute(tool_call).await?;
    self.result_processor.process(ToolResultInput {
        tool_call_id: tool_call.id.clone(),
        tool_name: tool_call.function.name.clone(),
        content,
    })
}
```

- [ ] **Step 3: Use it in `execute_all()`**

Replace the success branch in `execute_all()`:

```rust
let (content, metadata) = match self.execute_and_process(tool_call).await {
    Ok(processed) => {
        debug_log!(
            "Tool '{}' succeeded, original chars: {}, injected chars: {}, truncated: {}",
            tool_call.function.name,
            processed.metadata.original_chars,
            processed.metadata.injected_chars,
            processed.metadata.truncated,
        );
        (processed.content, processed.metadata)
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
```

- [ ] **Step 4: Use it in `execute_all_traced()`**

Replace the success path in the traced async block:

```rust
let (content, metadata) = match self.execute_and_process(tool_call).await {
    Ok(processed) => {
        let duration_ms = started.elapsed().as_millis() as u64;
        let output = JsonSnapshot::from_text(&processed.content, DEFAULT_SNAPSHOT_MAX_BYTES);
        let metadata = processed.metadata;
        debug_log!(
            "Traced tool '{}' succeeded, original chars: {}, injected chars: {}, truncated: {}",
            tool_call.function.name,
            metadata.original_chars,
            metadata.injected_chars,
            metadata.truncated,
        );
        sink.emit(
            TraceEventType::ToolCallCompleted,
            json!({
                "node_id": node_id,
                "duration_ms": duration_ms,
                "output": output,
                "output_metadata": tool_result_metadata_json(&metadata),
            }),
        );
        (processed.content, metadata)
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
```

Add the helper near `ToolExecutionResult`:

```rust
fn tool_result_metadata_json(metadata: &ToolResultMetadata) -> serde_json::Value {
    json!({
        "original_chars": metadata.original_chars,
        "injected_chars": metadata.injected_chars,
        "truncated": metadata.truncated,
        "artifact_path": metadata.artifact_path_display(),
    })
}
```

Extend `ToolExecutionResult`:

```rust
pub struct ToolExecutionResult {
    pub tool_call_id: String,
    pub content: String,
    pub metadata: ToolResultMetadata,
}
```

Run:

```bash
cargo check
```

Expected before the agent wiring is updated: compilation may fail where `ToolExecutionResult` literals are incomplete. Expected after the literals are updated: `cargo check` passes.

### Task 4: Wire Configured Processor Into Agent

**Files:**
- Modify: `src/agent.rs`

- [ ] **Step 1: Import processor types**

Add to the existing crate imports:

```rust
tool_result_processor::{ToolResultProcessor, ToolResultProcessorConfig},
```

- [ ] **Step 2: Construct the registry with configured processor**

Replace:

```rust
let mut tool_registry = ToolRegistry::new();
```

with:

```rust
let tool_result_processor = ToolResultProcessor::new(ToolResultProcessorConfig {
    max_injected_chars: config.tool_results.max_injected_chars,
    output_dir: config.tool_results.output_dir.clone(),
});
let mut tool_registry = ToolRegistry::with_result_processor(tool_result_processor);
```

Run:

```bash
cargo check
```

Expected: Rust compilation passes.

### Task 5: Add Registry-Level Regression Tests

**Files:**
- Modify: `src/tool_registry.rs`

- [ ] **Step 1: Add a configurable provider for large output**

Inside the existing `#[cfg(test)] mod tests`, add:

```rust
struct LargeOutputProvider {
    definitions: Vec<ToolDef>,
    content: String,
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
```

- [ ] **Step 2: Add non-traced test**

Add:

```rust
#[tokio::test]
async fn execute_all_truncates_large_tool_output_and_saves_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let processor = crate::tool_result_processor::ToolResultProcessor::new(
        crate::tool_result_processor::ToolResultProcessorConfig {
            max_injected_chars: 220,
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
    assert!(results[0].content.chars().count() <= 220);
    let artifact_path = results[0].metadata.artifact_path.as_ref().unwrap();
    assert_eq!(std::fs::read_to_string(artifact_path).unwrap(), original);
}
```

- [ ] **Step 3: Add traced test**

Add:

```rust
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
```

Run:

```bash
cargo test tool_registry
```

Expected: all registry tests pass, including existing concurrency tests.

### Task 6: Ignore Runtime Output Artifacts

**Files:**
- Modify: `.gitignore`

- [ ] **Step 1: Ignore generated tool output files**

Add under the logs or environment section:

```gitignore
# Sparrow runtime artifacts
/.sparrow_agent/tool_outputs/
```

Run:

```bash
git status --short
```

Expected: future files under `.sparrow_agent/tool_outputs/` do not appear in git status. Existing unrelated trace files under `.sparrow_agent/traces/` are not touched by this task.

### Task 7: End-to-End Verification

**Files:**
- No additional file edits.

- [ ] **Step 1: Run focused tests**

Run:

```bash
cargo test tool_result_processor
cargo test tool_registry
```

Expected: both commands pass.

- [ ] **Step 2: Run full Rust tests**

Run:

```bash
cargo test
```

Expected: all Rust tests pass.

- [ ] **Step 3: Verify generated oversized output behavior manually**

Use a tiny temporary provider in a test or a debug-only call path with the default output directory to return a string larger than 20,000 characters. Confirm these facts in logs or assertions:

```text
original_chars > 20000
metadata.truncated == true
metadata.injected_chars <= 20000
metadata.artifact_path is Some(...)
returned content starts with "工具输出过长"
artifact file content equals the exact original tool output
```

## Trace Payload Shape

When a traced tool output is truncated, `tool_call.completed` should look like this:

```json
{
  "node_id": "tool_...",
  "duration_ms": 42,
  "output": {
    "value": {
      "raw": "工具输出过长：本次工具调用 ..."
    },
    "text": "工具输出过长：本次工具调用 ...",
    "truncated": false
  },
  "output_metadata": {
    "original_chars": 34812,
    "injected_chars": 20000,
    "truncated": true,
    "artifact_path": ".sparrow_agent/tool_outputs/01...-read_file-call_abc.txt"
  }
}
```

`output.truncated` remains the `JsonSnapshot` truncation marker for trace display snapshots. The new `output_metadata.truncated` is the authoritative marker for model-context truncation.

## Configuration

Default behavior:

```text
SPARROW_TOOL_RESULT_MAX_CHARS=20000
SPARROW_TOOL_OUTPUT_DIR=.sparrow_agent/tool_outputs
```

Recommended constraints:

- `SPARROW_TOOL_RESULT_MAX_CHARS=0` is invalid and falls back to `20000`.
- Relative `SPARROW_TOOL_OUTPUT_DIR` is resolved relative to the process working directory, matching existing `.sparrow_agent` runtime artifact behavior.
- The output directory should remain local to the workspace unless the user explicitly configures another path.

## Security and Operations Notes

- Artifact files may contain secrets because they store complete tool output. Files should be written with default local filesystem permissions first; a follow-up hardening pass can set Unix mode `0o600` with `OpenOptionsExt` if needed.
- The processor should not redact the artifact file, because the model-facing content says it is the complete output. Redaction belongs to display snapshots, not the raw artifact.
- The model only receives the file path. It can choose a narrower file-reading tool call if it needs the saved content, and filesystem roots/mode still gate actual access.
- Runtime artifact cleanup is outside this small feature. A later task can add retention by age, task id, or maximum directory size.

## Self-Review

- Spec coverage: The plan covers the 20,000-character threshold, truncation, local file persistence, model-facing notice, total original length, artifact path, and retry guidance.
- Placeholder scan: No step relies on undefined behavior or an unspecified module. Each touched file has concrete responsibilities and exact snippets for the main changes.
- Type consistency: `ToolResultProcessor`, `ToolResultProcessorConfig`, `ToolResultInput`, `ProcessedToolResult`, `ToolResultMetadata`, and `ToolExecutionResult.metadata` are used consistently across tasks.
- Scope check: The plan stays focused on post-processing successful tool output. It does not attempt broader context compilation, summarization, artifact cleanup, or frontend rendering changes.
