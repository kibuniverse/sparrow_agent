# Agent 调用过程可视化 Agent Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 改造 Rust Agent，使其在执行用户任务时产出结构化 trace 事件，并通过 HTTP/SSE 接口供 `frontend/` 实时展示模型调用、模型输出和工具调用详情。

**Architecture:** 保留现有 CLI 和 `AgentStreamEvent` 控制台渲染能力，新增一套面向前端的 `TraceEvent` 事件模型。`Agent` 在模型请求、流式 reasoning、模型输出、工具执行和任务完成时写入 `TraceStore`；`server.rs` 使用 Axum 暴露任务创建、任务快照和 SSE 订阅接口。前端使用方式见 `docs/agent-call-visualization-frontend-plan.md`。

**Tech Stack:** Rust 2024, tokio, axum, serde, reqwest, futures-util, async-stream, ulid, chrono, tower-http.

---

## 1. 当前 Agent 现状

已有能力：

- `src/agent.rs` 维护消息历史、构造 DeepSeek 请求、驱动多轮工具调用循环。
- `src/streaming.rs` 已有 `AgentStreamEvent`、`AgentEventSink` 和 `StreamAccumulator`，能从 DeepSeek SSE 聚合 `reasoning_content`、`content` 和 `tool_calls`。
- `src/console.rs` 已有 `ConsoleTraceRenderer`，可以把思考过程输出到 CLI。
- `src/tool_registry.rs` 负责按工具名分发并执行所有工具。
- `src/client.rs` 已经支持 DeepSeek 流式 SSE 解析，但当前流式请求没有启用 `stream_options.include_usage`。

缺口：

- 没有浏览器可调用的 HTTP API。
- 没有任务 ID、会话 ID、事件序号和事件历史。
- 现有 `AgentStreamEvent` 只能表达模型流式增量，不能表达工具执行开始、工具执行完成、节点层级和详情面板所需的完整元数据。
- 工具执行没有耗时和结构化入参/出参事件。

## 2. 总体方案

新增 Agent Trace 层：

```text
frontend
  -> POST /api/agent/tasks
  -> GET /api/agent/tasks/:task_id/events

server.rs
  -> ConversationStore
  -> Agent::handle_user_input_with_trace()
      -> DeepSeekClient::chat_completion_stream()
      -> StreamAccumulator
      -> TraceSink
      -> ToolRegistry::execute_all_traced()
  -> TraceStore
  -> SSE replay + live broadcast
```

核心原则：

- Agent 不依赖前端 UI，只产出稳定事件。
- 前端不解析 DeepSeek 原始 SSE，只消费 Agent 事件。
- CLI 保持可用；没有启动 server 模式时仍使用现有 REPL。
- `reasoning_content` 只展示上游模型 API 显式返回的内容；不伪造或暴露系统内部隐藏推理。
- 事件中所有大字段都带 `truncated` 标识，避免任务详情页被大工具输出撑爆。

## 3. HTTP 接口契约

### 3.1 启动服务

新增启动方式：

```bash
cargo run -- --server
```

默认监听：

```text
127.0.0.1:8787
```

可通过环境变量覆盖：

```bash
SPARROW_SERVER_ADDR=127.0.0.1:8787 cargo run -- --server
```

### 3.2 创建任务

`POST /api/agent/tasks`

请求：

```json
{
  "conversation_id": "conv_01JZ4N0Y3GB9HKW98D5Z9F3R2A",
  "client_message_id": "msg_01JZ4N13M9T2ES9NKR3BVJ8GQ5",
  "message": "帮我分析这个仓库的结构",
  "stream": true
}
```

行为：

- `conversation_id` 为空时创建新会话。
- 同一会话同一时间只允许一个 running task。
- `client_message_id` 已存在且任务仍在 store 中时，返回原任务，避免前端重复提交。
- 第一版固定使用流式模型调用；`stream = false` 返回 `400 unsupported_mode`。

成功响应，状态码 `202`：

```json
{
  "task_id": "task_01JZ4N18T4BSX2G6X93K5E8GAT",
  "conversation_id": "conv_01JZ4N0Y3GB9HKW98D5Z9F3R2A",
  "events_url": "/api/agent/tasks/task_01JZ4N18T4BSX2G6X93K5E8GAT/events",
  "snapshot_url": "/api/agent/tasks/task_01JZ4N18T4BSX2G6X93K5E8GAT"
}
```

错误响应：

```json
{
  "error": {
    "code": "conversation_busy",
    "message": "Conversation already has a running task.",
    "retryable": true
  }
}
```

### 3.3 获取任务快照

`GET /api/agent/tasks/:task_id`

响应：

```json
{
  "task_id": "task_01JZ4N18T4BSX2G6X93K5E8GAT",
  "conversation_id": "conv_01JZ4N0Y3GB9HKW98D5Z9F3R2A",
  "status": "running",
  "created_at": "2026-05-10T14:25:18.220Z",
  "updated_at": "2026-05-10T14:25:19.120Z",
  "events": []
}
```

### 3.4 订阅任务事件

`GET /api/agent/tasks/:task_id/events?after_seq=0`

SSE 输出：

```text
event: trace
id: 12
data: {"seq":12,"task_id":"task_01JZ4N18T4BSX2G6X93K5E8GAT","conversation_id":"conv_01JZ4N0Y3GB9HKW98D5Z9F3R2A","timestamp":"2026-05-10T14:25:19.120Z","type":"tool_call.completed","payload":{"node_id":"tool_01JZ4N1ACXK7TT3B7JV0X7HHN5","duration_ms":842,"output":{"value":{"summary":"..."},"text":"{\"summary\":\"...\"}","truncated":false}}}
```

服务端行为：

- 先 replay `seq > after_seq` 的历史事件。
- 然后订阅 live broadcast。
- 心跳每 15 秒发送一次 comment frame：`: keep-alive`。
- 任务完成后保持连接 2 秒，让前端收到最终事件后自然断开。

## 4. TraceEvent JSON Schema

所有事件使用统一 envelope：

```json
{
  "seq": 1,
  "task_id": "task_01JZ4N18T4BSX2G6X93K5E8GAT",
  "conversation_id": "conv_01JZ4N0Y3GB9HKW98D5Z9F3R2A",
  "timestamp": "2026-05-10T14:25:18.220Z",
  "type": "model_call.started",
  "payload": {}
}
```

事件类型：

### 4.1 task.started

```json
{
  "message": {
    "role": "user",
    "content": "帮我分析这个仓库的结构"
  }
}
```

### 4.2 model_call.started

```json
{
  "node_id": "model_01JZ4N19M0TN8SRW3YMZAK7VA1",
  "round": 1,
  "model": "deepseek-v4-pro",
  "request": {
    "value": {
      "model": "deepseek-v4-pro",
      "message_count": 2,
      "tool_count": 6,
      "thinking": {"type": "enabled"},
      "reasoning_effort": "high"
    },
    "text": "{\"model\":\"deepseek-v4-pro\",\"message_count\":2,\"tool_count\":6,\"thinking\":{\"type\":\"enabled\"},\"reasoning_effort\":\"high\"}",
    "truncated": false
  }
}
```

### 4.3 model_call.reasoning_delta

```json
{
  "node_id": "model_01JZ4N19M0TN8SRW3YMZAK7VA1",
  "delta": "我需要先查看项目结构，确认前端和 agent 的边界。"
}
```

### 4.4 model_output.started

```json
{
  "node_id": "output_01JZ4N1A3H0N51QSEJ5J9VBTX6",
  "parent_model_call_id": "model_01JZ4N19M0TN8SRW3YMZAK7VA1",
  "kind": "tool_calls"
}
```

`kind` 可选值：

- `tool_calls`
- `final_answer`

### 4.5 model_output.delta

最终回答增量：

```json
{
  "node_id": "output_01JZ4N1A3H0N51QSEJ5J9VBTX6",
  "kind": "final_answer",
  "content_delta": "这是分析结果的第一段。"
}
```

工具调用增量：

```json
{
  "node_id": "output_01JZ4N1A3H0N51QSEJ5J9VBTX6",
  "kind": "tool_calls",
  "tool_call": {
    "index": 0,
    "tool_call_id": "call_abc",
    "name": "webSearch",
    "arguments_delta": "{\"query\":\"sparrow agent\""
  }
}
```

### 4.6 model_output.completed

```json
{
  "node_id": "output_01JZ4N1A3H0N51QSEJ5J9VBTX6",
  "kind": "tool_calls",
  "content": "",
  "tool_calls": [
    {
      "index": 0,
      "tool_call_id": "call_abc",
      "name": "webSearch",
      "arguments": {
        "value": {"query": "sparrow agent"},
        "text": "{\"query\":\"sparrow agent\"}",
        "truncated": false
      }
    }
  ]
}
```

### 4.7 tool_call.started

```json
{
  "node_id": "tool_01JZ4N1ACXK7TT3B7JV0X7HHN5",
  "parent_model_output_id": "output_01JZ4N1A3H0N51QSEJ5J9VBTX6",
  "index": 0,
  "tool_call_id": "call_abc",
  "name": "webSearch",
  "arguments": {
    "value": {"query": "sparrow agent"},
    "text": "{\"query\":\"sparrow agent\"}",
    "truncated": false
  }
}
```

### 4.8 tool_call.completed

```json
{
  "node_id": "tool_01JZ4N1ACXK7TT3B7JV0X7HHN5",
  "duration_ms": 842,
  "output": {
    "value": {"summary": "搜索结果摘要"},
    "text": "{\"summary\":\"搜索结果摘要\"}",
    "truncated": false
  }
}
```

### 4.9 tool_call.failed

```json
{
  "node_id": "tool_01JZ4N1ACXK7TT3B7JV0X7HHN5",
  "duration_ms": 842,
  "error": "Tool execution failed: request timeout"
}
```

### 4.10 model_call.completed

```json
{
  "node_id": "model_01JZ4N19M0TN8SRW3YMZAK7VA1",
  "duration_ms": 2310,
  "finish_reason": "tool_calls",
  "usage": {
    "prompt_tokens": 3200,
    "completion_tokens": 180,
    "total_tokens": 3380,
    "reasoning_tokens": 120
  },
  "response": {
    "value": {
      "has_content": false,
      "tool_call_count": 1
    },
    "text": "{\"has_content\":false,\"tool_call_count\":1}",
    "truncated": false
  }
}
```

### 4.11 task.completed

```json
{
  "duration_ms": 8100,
  "final_answer": "完整最终回复"
}
```

### 4.12 task.failed

```json
{
  "duration_ms": 8100,
  "error": "chat completion request failed with status 500"
}
```

## 5. Rust 类型设计

创建 `src/trace.rs`：

```rust
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct TraceEvent {
    pub seq: u64,
    pub task_id: String,
    pub conversation_id: String,
    pub timestamp: DateTime<Utc>,
    #[serde(rename = "type")]
    pub event_type: TraceEventType,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub enum TraceEventType {
    #[serde(rename = "task.started")]
    TaskStarted,
    #[serde(rename = "task.completed")]
    TaskCompleted,
    #[serde(rename = "task.failed")]
    TaskFailed,
    #[serde(rename = "model_call.started")]
    ModelCallStarted,
    #[serde(rename = "model_call.reasoning_delta")]
    ModelCallReasoningDelta,
    #[serde(rename = "model_call.completed")]
    ModelCallCompleted,
    #[serde(rename = "model_output.started")]
    ModelOutputStarted,
    #[serde(rename = "model_output.delta")]
    ModelOutputDelta,
    #[serde(rename = "model_output.completed")]
    ModelOutputCompleted,
    #[serde(rename = "tool_call.started")]
    ToolCallStarted,
    #[serde(rename = "tool_call.completed")]
    ToolCallCompleted,
    #[serde(rename = "tool_call.failed")]
    ToolCallFailed,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonSnapshot {
    pub value: serde_json::Value,
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum TaskStatus {
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "succeeded")]
    Succeeded,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "cancelled")]
    Cancelled,
}

pub trait TraceSink: Send + Sync {
    fn emit(&self, event_type: TraceEventType, payload: serde_json::Value);
}
```

`JsonSnapshot` 构造规则：

- 如果原始文本是合法 JSON，`value` 使用解析后的 JSON。
- 如果原始文本不是合法 JSON，`value = {"raw": "<text>"}`。
- `text` 最大保留 64 KiB。
- 超出限制时截断并设置 `truncated = true`。
- 任何 key 名包含 `api_key`、`token`、`authorization`、`password`、`secret` 时，值替换为 `"[REDACTED]"`。

## 6. 存储与广播设计

创建 `src/trace_store.rs`。

核心结构：

```rust
pub struct TraceStore {
    tasks: tokio::sync::RwLock<HashMap<String, StoredTask>>,
}

pub struct StoredTask {
    pub task_id: String,
    pub conversation_id: String,
    pub client_message_id: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub events: Vec<TraceEvent>,
    pub next_seq: u64,
    pub tx: tokio::sync::broadcast::Sender<TraceEvent>,
}
```

方法：

- `create_task(conversation_id, client_message_id) -> StoredTaskHandle`
- `append_event(task_id, event_type, payload) -> TraceEvent`
- `snapshot(task_id) -> TaskSnapshot`
- `subscribe(task_id, after_seq) -> (Vec<TraceEvent>, broadcast::Receiver<TraceEvent>)`
- `mark_succeeded(task_id)`
- `mark_failed(task_id, error)`

保留策略：

- 第一版内存保存，不落盘。
- 已完成任务保留 30 分钟。
- 每个任务最多保存 10,000 个事件；超过限制时任务失败并发出 `task.failed`，错误为 `trace event limit exceeded`。

## 7. Agent 改造点

### 7.1 新增带 Trace 的入口

在 `src/agent.rs` 增加：

```rust
pub async fn handle_user_input_with_trace(
    &mut self,
    input: impl Into<String>,
    sink: &dyn TraceSink,
) -> anyhow::Result<()>
```

行为：

- 追加 user message 前发出 `task.started`。
- 每轮模型请求前发出 `model_call.started`。
- 流式处理中将 `ReasoningDelta` 转换为 `model_call.reasoning_delta`。
- 首次出现 `content` 时创建 `model_output.started(kind = "final_answer")`。
- 首次出现 `tool_call_delta` 时创建 `model_output.started(kind = "tool_calls")`。
- response 聚合完成后发出 `model_output.completed` 和 `model_call.completed`。
- 工具执行开始/完成/失败由 `ToolRegistry::execute_all_traced()` 发出。
- 最终 answer 完成后发出 `task.completed`。
- 任意错误路径发出 `task.failed`，再返回错误。

### 7.2 模型调用耗时

每轮调用使用 `std::time::Instant` 计时：

```rust
let started = Instant::now();
let request = self.build_request();
// stream and accumulate
let duration_ms = started.elapsed().as_millis() as u64;
```

`model_call.completed.duration_ms` 只覆盖模型 API 从请求开始到 response 聚合完成的时间，不包含后续工具执行时间。

### 7.3 流式 usage

在 `src/client.rs` 的 `chat_completion_stream()` 中启用 usage 返回：

```rust
stream_request.stream = Some(true);
stream_request.stream_options = Some(StreamOptions { include_usage: true });
```

这样 `StreamAccumulator` 已有的 `Usage` 分支可以填充 `model_call.completed.usage`。如果上游模型没有返回 usage，则事件中 `usage = null`，前端详情面板展示 `未返回 token 用量`。

### 7.4 模型请求与响应快照

`model_call.started.request` 不保存完整 messages 内容，默认保存安全摘要：

```json
{
  "model": "deepseek-v4-pro",
  "message_count": 8,
  "tool_count": 6,
  "thinking": {"type": "enabled"},
  "reasoning_effort": "high"
}
```

如果后续需要完整调试，可用环境变量开启：

```text
SPARROW_TRACE_FULL_MODEL_IO=true
```

开启后仍需要经过 redaction 和 64 KiB 截断。

### 7.5 工具执行事件

在 `src/tool_registry.rs` 增加：

```rust
pub async fn execute_all_traced(
    &self,
    tool_calls: &[ToolCall],
    parent_model_output_id: &str,
    sink: &dyn TraceSink,
) -> Vec<ToolExecutionResult>
```

每个工具按现有顺序执行。对每个工具：

1. 生成 `tool_<ulid>` 节点 ID。
2. 发出 `tool_call.started`，包含工具名和参数快照。
3. 调用现有 `execute(tool_call)`。
4. 成功时发出 `tool_call.completed`，包含输出快照和耗时。
5. 失败时发出 `tool_call.failed`，并仍返回当前兼容行为中的 `Tool execution failed: ...` 作为 tool message 内容。

第一版保持顺序执行，不引入并发工具执行，避免改变 Agent 语义。

## 8. Server 设计

创建 `src/server.rs`。

路由：

```rust
Router::new()
    .route("/api/health", get(health))
    .route("/api/agent/tasks", post(create_task))
    .route("/api/agent/tasks/:task_id", get(get_task))
    .route("/api/agent/tasks/:task_id/events", get(stream_task_events))
```

应用状态：

```rust
pub struct ServerState {
    pub config: AppConfig,
    pub conversations: ConversationStore,
    pub traces: Arc<TraceStore>,
}
```

会话存储：

```rust
pub struct ConversationStore {
    agents: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<Agent>>>>,
    running_tasks: tokio::sync::Mutex<HashMap<String, String>>,
}
```

创建任务流程：

1. 校验请求体。
2. 获取或创建 `conversation_id`。
3. 检查该会话是否已有 running task。
4. 创建 trace task。
5. `tokio::spawn` 后台执行 `Agent::handle_user_input_with_trace()`。
6. 立即返回 `202`。
7. 后台任务结束后释放 `running_tasks[conversation_id]`。

SSE 输出使用 `axum::response::sse::{Event, Sse}`。每个 `TraceEvent` 序列化为 `event: trace`，`id` 使用 `seq`。

## 9. 文件改造清单

### Create

- `src/trace.rs`：TraceEvent、TraceSink、JsonSnapshot、redaction 和 snapshot helpers。
- `src/trace_store.rs`：任务事件存储、seq 管理和 broadcast。
- `src/server.rs`：Axum HTTP/SSE 服务。
- `src/conversation_store.rs`：conversation 到 Agent 实例的映射和 running task 锁。

### Modify

- `Cargo.toml`：增加以下依赖：

```toml
axum = "0.8"
tower-http = { version = "0.6", features = ["cors"] }
tokio-stream = { version = "0.1", features = ["sync"] }
ulid = "1"
chrono = { version = "0.4", features = ["serde"] }
```

- `src/lib.rs`：导出 `trace`、`trace_store`、`server`、`conversation_store`。
- `src/main.rs`：增加 `--server` 启动分支，默认仍进入 CLI REPL。
- `src/client.rs`：流式请求设置 `stream_options.include_usage = true`。
- `src/agent.rs`：新增 `handle_user_input_with_trace()`，在模型调用和输出阶段发出 trace。
- `src/streaming.rs`：保留 `AgentStreamEvent`，增加 trace bridge 所需的 output kind 判断辅助方法。
- `src/tool_registry.rs`：新增 `execute_all_traced()`。
- `README.md`：补充 server 启动方式和前端联调命令。

## 10. 实施任务

### Task 1: Trace 类型与快照工具

**Files:**

- Create: `src/trace.rs`
- Modify: `src/lib.rs`

- [ ] 定义第 5 节中的 Rust 类型。
- [ ] 实现 `JsonSnapshot::from_text(text, max_bytes)`。
- [ ] 实现 `redact_json_value(value)`，递归处理对象和数组。
- [ ] 增加单元测试：合法 JSON、非法 JSON、大文本截断、secret key redaction。
- [ ] 运行 `cargo test trace`.

### Task 2: TraceStore

**Files:**

- Create: `src/trace_store.rs`
- Modify: `src/lib.rs`

- [ ] 实现任务创建、事件追加、快照读取和订阅。
- [ ] 确保 `append_event()` 在同一 task 内单调递增 `seq`。
- [ ] 任务完成或失败时更新 `status` 和 `updated_at`。
- [ ] 增加单元测试：历史 replay、after_seq 过滤、status 更新、事件上限。
- [ ] 运行 `cargo test trace_store`.

### Task 3: 工具调用 trace

**Files:**

- Modify: `src/tool_registry.rs`

- [ ] 增加 `execute_all_traced()`。
- [ ] 在工具执行前发出 `tool_call.started`。
- [ ] 成功时发出 `tool_call.completed`，失败时发出 `tool_call.failed`。
- [ ] 保持 `execute_all()` 原行为，CLI 非 trace 路径不受影响。
- [ ] 增加单元测试：成功工具、失败工具、未知工具都产生正确事件。
- [ ] 运行 `cargo test tool_registry`.

### Task 4: Agent 模型调用 trace

**Files:**

- Modify: `src/agent.rs`
- Modify: `src/client.rs`
- Modify: `src/streaming.rs`

- [ ] 新增 `handle_user_input_with_trace()`。
- [ ] 在 `chat_completion_stream()` 中设置 `stream_options.include_usage = true`。
- [ ] 每一轮开始发出 `model_call.started`。
- [ ] 将 reasoning delta 映射为 `model_call.reasoning_delta`。
- [ ] 根据 stream delta 创建 `model_output.started(kind)` 和 `model_output.delta`。
- [ ] 聚合完成后发出 `model_output.completed` 和 `model_call.completed`。
- [ ] 工具调用场景改用 `execute_all_traced()`。
- [ ] 最终回答发出 `task.completed`。
- [ ] 错误路径发出 `task.failed`。
- [ ] 运行 `cargo test agent streaming`.

### Task 5: HTTP/SSE 服务

**Files:**

- Create: `src/server.rs`
- Create: `src/conversation_store.rs`
- Modify: `src/main.rs`
- Modify: `src/lib.rs`
- Modify: `Cargo.toml`

- [ ] 添加 Axum 相关依赖。
- [ ] 实现 `/api/health`。
- [ ] 实现 `POST /api/agent/tasks`。
- [ ] 实现 `GET /api/agent/tasks/:task_id`。
- [ ] 实现 `GET /api/agent/tasks/:task_id/events`，支持 replay 和 live broadcast。
- [ ] 实现 `--server` 启动分支。
- [ ] 增加集成测试：创建任务返回 202，snapshot 返回历史事件，SSE 可收到 replay 事件。
- [ ] 运行 `cargo test server`.

### Task 6: 联调与文档

**Files:**

- Modify: `README.md`

- [ ] 记录 server 启动方式：`cargo run -- --server`。
- [ ] 记录 frontend 启动方式：`cd frontend && pnpm dev`。
- [ ] 记录默认 API 地址和 `SPARROW_SERVER_ADDR`。
- [ ] 运行 `cargo fmt --check`。
- [ ] 运行 `cargo check`。
- [ ] 运行 `cargo test`。

## 11. 验收标准

- `cargo run -- --server` 能启动 HTTP 服务，`GET /api/health` 返回成功。
- `POST /api/agent/tasks` 能创建任务并立即返回 `task_id`。
- SSE 事件包含 task、model_call、model_output、tool_call 的完整层级关系。
- 每个事件都有递增 `seq`、`task_id`、`conversation_id`、UTC timestamp 和明确 `type`。
- 工具调用详情包含入参、出参、耗时和错误信息。
- 模型调用详情包含模型名、请求摘要、响应摘要、token usage 和耗时。
- 任务详情页刷新后可以通过 snapshot 恢复历史事件。
- CLI 默认行为不变；不带 `--server` 时仍进入现有 REPL。
- `cargo fmt --check`、`cargo check`、`cargo test` 全部通过。
