# Sparrow Agent 上下文管理模块设计

日期：2026-06-07
状态：设计草案
适用项目：`sparrow_agent`

## 背景

当前 Sparrow Agent 仍以 `Agent.messages: Vec<ChatMessage>` 作为唯一会话状态。每一轮 `build_request()` 都直接 clone 全量 messages，把系统提示、用户输入、assistant 回复、tool_calls、tool result 和历史 reasoning 一起发送给模型。

项目已经有一些缓解机制：

- `ToolResultProcessor` 会截断过长工具输出，并把完整结果落盘到 `.sparrow_agent/tool_outputs`。
- CLI 会显示 context usage。
- trace archive 已经支持对归档事件和 request snapshot 做压缩。

但这些还不是“上下文管理模块”。现状仍然缺少 Host 侧的请求上下文编译、长期记忆摘要、历史裁剪、工具交换压缩、预算选择和可观测的上下文决策。结果是：长对话、项目分析、多轮工具调用和子 Agent 都会持续增加 prompt 体积，直到速度、成本、注意力质量或模型窗口成为问题。

## 目标

1. 引入独立的上下文管理模块，替代 `Agent` 直接维护并发送全量 `messages`。
2. 每轮模型调用前，根据模型窗口、保留预算和当前任务，编译一个最小充分的 `ContextPack`。
3. 保留完整审计日志，但不把完整日志等同于模型输入。
4. 对历史对话、工具调用交换、工具输出和项目资料做分层保留、摘要和引用。
5. 确保工具调用协议合法：不能产生 orphan tool message，也不能破坏 assistant tool_calls 与 tool results 的相邻关系。
6. 支持 CLI、Server、浏览器 trace、trace archive 和子 Agent。
7. 上下文压缩过程本身可测试、可观测、可回放。

非目标：

- 第一阶段不引入向量数据库或 embedding 检索。
- 第一阶段不重写 MCP filesystem server，只在 Sparrow Agent 层管理工具结果和请求消息。
- 第一阶段不改变现有 HTTP API 契约。
- 不把模型 reasoning 原文提炼为长期工作记忆；但对 DeepSeek 系列模型，只要某条历史 assistant message 被完整保留进请求，就必须保留它的 `reasoning_content`，否则接口兼容性会出问题。

## 当前问题定位

关键路径如下：

```text
Agent::handle_user_input
  -> self.messages.push(user)
  -> build_request()
      -> messages.clone()
  -> model stream
  -> handle_assistant_message()
      -> messages.push(assistant)
      -> ToolRegistry::execute_all(...)
      -> messages.push(tool result)
```

问题集中在四点：

1. `messages` 同时承担审计日志、工作记忆和请求上下文三种职责。
2. 工具结果进入 `messages` 后，即使已截断，也会在后续每轮反复发送。
3. 历史 assistant 的 `reasoning_content` 会被序列化进后续请求，占用预算；但 DeepSeek 系列模型要求后续调用继续携带已返回的 `reasoning_content`，不能简单删除该字段。
4. 没有为当前用户输入、近期工具协议、长期摘要、文件片段、最终回答预留不同预算。

## 模型兼容性约束

上下文管理不能只按“内容价值”裁剪消息，还必须遵守具体模型的消息协议。对当前项目默认使用的 DeepSeek 系列模型，设计约束如下：

- 如果请求中保留某条历史 assistant message，且该 message 原始返回中包含 `reasoning_content`，则编译后的请求必须继续携带该字段。
- 不允许通过“删除 assistant.reasoning_content 但保留 assistant.content/tool_calls”的方式节省预算。
- 如果历史 turn 太大，应把整段旧 turn 压缩成普通摘要 message，而不是局部改写原 assistant message。
- 摘要 message 不伪造 `reasoning_content`，只描述已经完成的对话事实、工具结论和 artifact 引用。
- 模型兼容性策略应由 Host 侧根据 `model` 自动选择，DeepSeek 默认 `preserve_reasoning_content = true`。

因此，本文后续的“压缩 reasoning”只表示“把旧 turn 整体摘要化后不再发送原始 reasoning”，不表示对仍被保留的 assistant message 删除 `reasoning_content`。

## 推荐架构

新增 `src/context/` 模块族：

```text
src/context/
  mod.rs
  manager.rs
  transcript.rs
  compiler.rs
  budget.rs
  memory.rs
  summary.rs
  artifact_ref.rs
```

职责划分：

| 模块 | 职责 |
| --- | --- |
| `manager.rs` | `ContextManager` 门面，供 `Agent` 记录消息和编译请求上下文 |
| `transcript.rs` | 完整会话日志、turn 结构、工具交换结构和保留策略 |
| `compiler.rs` | 把 transcript、working memory、当前输入编译为 `Vec<ChatMessage>` |
| `budget.rs` | 模型窗口、token 估算、预算分区、阈值和压缩触发 |
| `memory.rs` | 长期工作记忆：目标、事实、决策、开放问题、重要 artifact |
| `summary.rs` | 历史摘要和工具交换摘要的生成、更新和 fallback |
| `artifact_ref.rs` | 对工具输出、文件片段、搜索结果的稳定引用描述 |

整体调用链：

```text
Agent
  -> ContextManager::record_user(...)
  -> ContextManager::compile_request(...)
      -> ContextCompiler
      -> CompiledContext { messages, report }
  -> DeepSeekClient
  -> ContextManager::record_assistant(...)
  -> ToolRegistry::execute_all(...)
  -> ContextManager::record_tool_result(...)
  -> ContextManager::maybe_compact(...)
```

`Agent` 不再直接暴露一个增长型 `Vec<ChatMessage>` 给请求构造。完整历史由 `ContextManager` 管理，请求输入由 `ContextCompiler` 临时生成。

## 核心数据结构

### ContextManager

```rust
pub struct ContextManager {
    transcript: ConversationTranscript,
    working_memory: WorkingMemory,
    policy: ContextPolicy,
    estimator: TokenEstimator,
}
```

主要方法：

```rust
impl ContextManager {
    pub fn new(system_prompt: String, model: String, policy: ContextPolicy) -> Self;

    pub fn record_user(&mut self, content: String) -> MessageId;
    pub fn record_assistant(&mut self, message: ChoiceMessage) -> MessageId;
    pub fn record_tool_result(
        &mut self,
        tool_call_id: String,
        content: String,
        metadata: ToolResultMetadata,
    ) -> MessageId;

    pub fn compile_request(&mut self, input: CompileInput) -> anyhow::Result<CompiledContext>;
    pub async fn maybe_compact(&mut self, summarizer: &dyn ContextSummarizer) -> anyhow::Result<CompactionReport>;
}
```

`record_assistant` 接收模型返回的完整 `ChoiceMessage`，但存储时要区分：

- `content`：可进入历史或摘要。
- `tool_calls`：必须保留协议结构，直到对应 tool results 完成。
- `reasoning_content`：保留到审计日志和 transcript；DeepSeek 模型下，如果该 assistant message 被完整编入下一轮请求，必须随 message 一起发送。

### ConversationTranscript

```rust
pub struct ConversationTranscript {
    system_prompt: String,
    turns: Vec<ConversationTurn>,
    pending_tool_exchange: Option<ToolExchange>,
}

pub struct ConversationTurn {
    id: TurnId,
    user: TranscriptMessage,
    assistant: Option<TranscriptMessage>,
    tool_exchanges: Vec<ToolExchange>,
    summary: Option<TurnSummary>,
    retention: RetentionClass,
}

pub struct ToolExchange {
    assistant_message: TranscriptMessage,
    tool_results: Vec<TranscriptToolResult>,
    summary: Option<ToolExchangeSummary>,
    status: ToolExchangeStatus,
}
```

`ToolExchange` 是关键结构。压缩时不能把 assistant tool_calls 和 tool results 拆散，否则请求会违反 tool calling 协议。策略是：

- 最近一轮未完成工具交换必须完整保留。
- 已完成且较旧的工具交换可以整体替换成摘要 assistant/user-style context message。
- 绝不发送孤立的 `role=tool` message。

### WorkingMemory

```rust
pub struct WorkingMemory {
    task_goal: Option<String>,
    durable_facts: Vec<MemoryItem>,
    decisions: Vec<MemoryItem>,
    open_questions: Vec<MemoryItem>,
    artifact_refs: Vec<ContextArtifactRef>,
    updated_at: DateTime<Utc>,
}
```

Working memory 是长期压缩层，适合保存：

- 当前任务目标和验收标准。
- 用户明确偏好。
- 已确认的项目结构事实。
- 已读文件的模块职责摘要。
- 关键 bug 假设、已排除方案和下一步计划。

不适合保存：

- 大段原始代码。
- 模型 reasoning 原文。
- 临时命令输出全文。
- 已过期的工具参数增量。

### CompiledContext

```rust
pub struct CompiledContext {
    pub messages: Vec<ChatMessage>,
    pub report: ContextCompileReport,
}

pub struct ContextCompileReport {
    pub estimated_prompt_tokens: usize,
    pub target_prompt_tokens: usize,
    pub reserved_completion_tokens: usize,
    pub included_recent_turns: usize,
    pub summarized_turns: usize,
    pub included_tool_exchanges: usize,
    pub summarized_tool_exchanges: usize,
    pub reasoning_policy: String,
    pub preserved_reasoning_messages: usize,
    pub summarized_reasoning_messages: usize,
    pub warnings: Vec<String>,
}
```

`report` 应进入 trace，便于前端或归档解释“为什么这一轮没发送全部历史”。

## 预算策略

新增配置：

```text
SPARROW_CONTEXT_ENABLED=true
SPARROW_CONTEXT_TARGET_RATIO=0.60
SPARROW_CONTEXT_COMPACTION_TRIGGER_RATIO=0.40
SPARROW_CONTEXT_RESERVED_COMPLETION_TOKENS=8192
SPARROW_CONTEXT_RECENT_TURNS=6
SPARROW_CONTEXT_MAX_TOOL_EXCHANGE_TOKENS=4096
SPARROW_CONTEXT_MAX_MEMORY_TOKENS=12000
SPARROW_CONTEXT_MAX_SUMMARY_TOKENS=8000
SPARROW_CONTEXT_REASONING_POLICY=auto
```

`SPARROW_CONTEXT_REASONING_POLICY` 可选值：

| 值 | 行为 |
| --- | --- |
| `auto` | 根据模型选择；DeepSeek 系列保留历史 assistant 的 `reasoning_content` |
| `preserve` | 只要 assistant message 被完整保留，就保留 `reasoning_content` |
| `omit_when_supported` | 仅用于确认模型允许省略 reasoning 的场景 |

默认必须是 `auto`，不能默认剥离 DeepSeek 的 reasoning。

预算分区建议：

| 分区 | 默认比例或上限 | 说明 |
| --- | ---: | --- |
| 系统提示和工具定义 | 必保留 | 工具定义不在 messages 中，但会影响总请求大小 |
| 当前用户任务 | 必保留 | 不允许被摘要替代 |
| 未完成工具交换 | 必保留 | 协议安全要求 |
| 最近对话 | 最近 6 turns | 保留真实文本 |
| Working memory | 12k tokens | 长期摘要层 |
| 已完成工具交换摘要 | 4k tokens/交换上限 | 只保留结论、状态、artifact 引用 |
| completion reserve | 8k tokens 或配置值 | 防止输出空间被 prompt 挤占 |

Token 估算第一阶段不需要完美，但要稳定保守：

```text
estimated_tokens = max(
  chars / 3,
  utf8_bytes / 4,
  cjk_chars * 2 + ascii_chars / 4
) + message_overhead
```

后续可以用真实 `usage.prompt_tokens` 校准模型级估算系数。

## 编译算法

`ContextCompiler` 每轮按优先级装包：

1. 系统提示。
2. 工作记忆 message。
3. 已摘要历史的 summary message。
4. 最近 N 个完整 turns。
5. 当前未完成或刚完成的 tool exchange。
6. 当前用户输入。

伪代码：

```text
compile(transcript, memory, budget):
  pack = []
  pack.push(system_prompt)
  pack.push(memory_as_system_or_user_message(memory))

  for summary in transcript.old_summaries:
    pack.try_push(summary, priority=medium)

  for turn in transcript.recent_turns_reverse():
    pack.try_push_full_turn(turn, priority=high)

  if transcript.pending_tool_exchange:
    pack.force_push(protocol_safe_tool_exchange)

  pack.force_push(current_user_message)

  if pack.over_budget:
    compact older full turns into summaries
    for DeepSeek: never delete reasoning_content from retained assistant messages
    reduce tool exchange previews
    if still over budget: keep only system + memory + current turn + protocol-required messages
```

消息合法性检查必须作为编译后的硬校验：

- `role=tool` 之前必须有含对应 `tool_call_id` 的 assistant message。
- assistant tool_calls 后必须包含全部 tool results，除非这是发送给模型前的未完成状态，但 Sparrow 当前不会在工具未完成时请求模型。
- DeepSeek 模型下，完整保留的历史 assistant message 不得丢失其原始 `reasoning_content`。
- 不发送空的历史 tool exchange 占位。

## 压缩策略

压缩分两类：turn 压缩和工具交换压缩。

### Turn 压缩

当估算 prompt 超过 `compaction_trigger_ratio * context_window` 时，把较旧 turns 压缩到 `WorkingMemory` 和 `TurnSummary`：

```text
输入：
- 用户问题
- assistant 最终回答
- 工具交换摘要
- 重要 artifact refs

输出：
- task_progress
- durable_facts
- decisions
- open_questions
- files_or_artifacts
```

摘要生成优先使用模型 summarizer。模型不可用时使用 fallback：

- 保留用户输入前 1,000 字符。
- 保留 assistant 最终回答前 2,000 字符。
- 保留工具名、状态、artifact path、截断元数据。

### 工具交换压缩

工具结果进入 transcript 时已经经过 `ToolResultProcessor`。Context 模块继续把已完成工具交换压缩成：

```text
Tool exchange summary:
- model requested: runBashCommand(...)
- result status: exited / timed_out / failed
- important stdout/stderr excerpt
- artifact: .sparrow_agent/tool_outputs/...
- warning: output truncated
```

对 filesystem/read/search 这类工具，应保留路径、范围、hash、命中行和 artifact 引用，而不是保留全文。

## Trace 集成

新增 trace event 类型建议：

```text
context.compiled
context.compacted
```

第一阶段如果不想扩展前端 union，可以先把 `ContextCompileReport` 附加到 `model_call.started.payload.context`：

```json
{
  "node_id": "model_...",
  "round": 4,
  "model": "deepseek-v4-pro",
  "context": {
    "estimated_prompt_tokens": 42000,
    "target_prompt_tokens": 600000,
    "included_recent_turns": 6,
    "summarized_turns": 12,
    "reasoning_policy": "preserve",
    "preserved_reasoning_messages": 6,
    "summarized_reasoning_messages": 15
  },
  "request": { "...": "existing snapshot" }
}
```

前端可以后续在模型调用详情中展示该 report。trace archive 不需要特殊处理，因为 report 是普通 JSON payload。

## 与现有模块的集成点

### Agent

替换字段：

```rust
- messages: Vec<ChatMessage>
+ context: ContextManager
```

替换方法：

```rust
fn build_request(&mut self) -> ChatCompletionRequest {
    let compiled = self.context.compile_request(...)?;
    ChatCompletionRequest {
        messages: compiled.messages,
        ...
    }
}
```

`handle_assistant_message` 和 `handle_assistant_message_with_trace` 不再直接 push 到 `messages`，而是记录到 context。

### ToolResultProcessor

保持现有职责，但结果 metadata 要成为 context 决策输入：

- `original_chars`
- `injected_chars`
- `truncated`
- `artifact_path`

Context 模块根据 metadata 生成工具交换摘要和 artifact ref。

### ConversationStore

Server 模式仍然每个 conversation 维护一个 `Agent`。由于 `ContextManager` 在 `Agent` 内部，现有 busy 逻辑无需大改。

### SubAgent

子 Agent 使用独立 `ContextManager`。父 Agent 不能把自己的完整 working memory 自动注入子 Agent；只能通过 `runSubAgentTask.context_pack` 显式传递必要事实。

### Trace Replay

如果第一阶段只把 context report 放进现有 `model_call.started` payload，前端旧逻辑可以忽略该字段。后续再为 `TraceDetailPanel` 增加上下文报告展示。

## 配置结构

建议在 `AppConfig` 增加：

```rust
pub struct ContextConfig {
    pub enabled: bool,
    pub target_ratio: f32,
    pub compaction_trigger_ratio: f32,
    pub reserved_completion_tokens: usize,
    pub recent_turns: usize,
    pub max_tool_exchange_tokens: usize,
    pub max_memory_tokens: usize,
    pub max_summary_tokens: usize,
    pub reasoning_policy: ReasoningRetentionPolicy,
}
```

```rust
pub enum ReasoningRetentionPolicy {
    Auto,
    Preserve,
    OmitWhenSupported,
}
```

默认启用，但可以通过 `SPARROW_CONTEXT_ENABLED=false` 回退到旧行为，便于排查。

## 测试计划

后端 contract tests：

1. `compile_request_preserves_recent_turns_under_budget`
2. `compile_request_summarizes_old_turns_when_over_budget`
3. `compile_request_never_emits_orphan_tool_messages`
4. `compile_request_keeps_pending_tool_exchange_complete`
5. `compile_request_preserves_deepseek_reasoning_content_for_retained_assistant_messages`
6. `tool_exchange_summary_includes_artifact_metadata`
7. `context_manager_falls_back_when_summarizer_fails`
8. `sub_agent_context_does_not_inherit_parent_transcript`
9. `trace_model_call_started_includes_context_report`
10. `disabled_context_config_matches_legacy_message_order`
11. `compile_request_can_omit_reasoning_only_when_policy_allows`

前端 tests：

1. 模型调用详情可以展示 context report。
2. 旧 trace 没有 context 字段时仍正常渲染。
3. trace replay 遇到 compacted context metadata 不报错。

## 分阶段实施

### Phase 1：编译层和 DeepSeek reasoning 兼容

- 新增 `ContextManager`、`ConversationTranscript`、`ContextCompiler`、`ContextBudget`。
- `Agent` 改为通过 context 编译 messages。
- 默认保留最近 turns，旧 turns 暂不调用模型摘要，只做 fallback summary。
- DeepSeek 模型下，完整保留的 assistant 历史消息必须保留 `reasoning_content`。
- 超预算时优先摘要旧 turn，而不是删除已保留 assistant message 上的 `reasoning_content`。
- 增加消息合法性测试。

### Phase 2：模型摘要和 working memory

- 增加 `ContextSummarizer`，使用 DeepSeek 非工具调用生成结构化摘要。
- 在任务完成后或超过阈值时更新 `WorkingMemory`。
- trace 中记录 compaction report。

### Phase 3：artifact 引用和按需恢复

- 将 `.sparrow_agent/tool_outputs` 产物升级为可寻址 artifact refs。
- 工具结果摘要中稳定引用 artifact。
- 增加按 artifact/path/range 恢复局部上下文的工具或 Host 内部能力。

### Phase 4：前端可视化

- 在模型调用详情中展示上下文预算、最近 turn 数、摘要 turn 数、reasoning 保留策略和被摘要化的 reasoning 数。
- 增加 conversation-level memory 视图。

## 主要风险和处理

| 风险 | 处理 |
| --- | --- |
| 摘要丢失重要细节 | 最近 turns 完整保留；摘要保留 artifact refs；可按需恢复 |
| tool calling 协议被破坏 | 编译后做合法性校验；工具交换整体保留或整体摘要 |
| token 估算不准 | 保守估算；用真实 usage 校准；保留 completion reserve |
| DeepSeek 历史 reasoning 被误删导致接口失败 | 模型兼容性策略默认 `auto`；DeepSeek 下完整保留 assistant message 时强制保留 `reasoning_content` |
| reasoning 占用大量上下文 | 对旧 turn 做整体摘要；不对保留的 assistant message 做字段级删除 |
| 压缩调用增加延迟 | 优先在 turn 完成后压缩；请求前只做必要 fallback |
| Server 长会话内存增长 | transcript 可增加最大 turn 数和归档策略；第一阶段先管理 prompt，不删除审计日志 |

## 推荐第一版验收标准

第一版完成后，应满足：

1. `Agent` 不再直接用全量 `messages.clone()` 构造请求。
2. 长对话超过预算时，请求 messages 数量和估算 tokens 保持在目标预算内。
3. 所有现有 Rust 测试和前端测试通过。
4. 新增上下文 contract tests 覆盖工具消息合法性。
5. trace 中可以看到每次模型调用的 context compile report。
6. 关闭 `SPARROW_CONTEXT_ENABLED` 后可以回退旧行为。

## 结论

上下文管理模块的核心不是简单截断历史，而是把“完整审计日志”和“本轮模型输入”分离。Sparrow Agent 已经具备工具执行、工具结果后处理、trace 和子 Agent 的基础设施；下一步应在 `Agent` 和 `ChatCompletionRequest` 之间增加 `ContextManager`，由它统一决定哪些内容被完整保留、哪些内容被摘要、哪些内容只作为 artifact 引用存在。
