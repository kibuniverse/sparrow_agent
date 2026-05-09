# Sparrow 模型思考过程流式展示方案

状态：建议方案
日期：2026-05-08
适用项目：`sparrow_agent`

## 1. 背景

Sparrow 当前已经启用 DeepSeek thinking 模式，并在响应结构中支持 `reasoning_content`。这意味着模型返回最终回答前产生的思考内容已经可以被 API 返回，也可以被 Agent 记录到消息历史中。

但当前实现仍是非流式调用：用户输入后，Agent 会等待 DeepSeek 完整返回，再一次性打印最终回答。对于需要较长推理或多轮工具调用的任务，这会带来几个问题：

- 用户在等待期间看不到模型是否正在推理、是否已经进入答案阶段、是否准备调用工具。
- 已有 `reasoning_content` 只能在响应完成后被消费，不能实时展示。
- 工具调用前的思考过程无法作为连续事件呈现，调试 Agent 行为时可观察性不足。
- 前端目录目前只是 Vite 壳，标题为 `show model full trace`，还没有与 Rust Agent 的事件流对接。

本方案目标是在不破坏现有工具调用循环的前提下，增加一条“流式模型事件”链路，使 CLI 能先落地实时展示思考过程，并为后续 Web UI 展示完整 trace 留出清晰接口。

## 2. 当前模型调用方案分析

当前关键调用链如下：

```text
main.rs
  -> Agent::new(config)
  -> read_user_input(">>> ")
  -> Agent::handle_user_input(input)
      -> messages.push(user)
      -> for round in 0..max_tool_rounds
          -> build_request()
              -> model
              -> messages.clone()
              -> tools
              -> thinking: enabled
              -> reasoning_effort
              -> stream: None
          -> DeepSeekClient::chat_completion(request)
              -> POST /chat/completions
              -> response.text().await
              -> serde_json::from_str<ChatCompletionResponse>()
          -> handle_assistant_message(choice.message)
              -> if tool_calls:
                    append assistant message with reasoning_content and tool_calls
                    execute tools
                    append tool messages
                    continue
                 else:
                    println!("agent> {content}")
                    append assistant message with content and reasoning_content
                    complete
```

现状中的主要事实：

- `src/api.rs` 的 `ChatCompletionRequest` 已有 `stream: Option<bool>` 字段，但 `Agent::build_request()` 固定传 `None`。
- `src/api.rs` 已有非流式响应结构，`ChoiceMessage` 包含 `content`、`reasoning_content` 和 `tool_calls`。
- `src/client.rs` 的 `DeepSeekClient::chat_completion()` 使用 `response.text().await` 一次性读取完整 body，没有解析 SSE。
- `src/agent.rs` 的工具调用循环以完整 `ChoiceMessage` 为边界，必须等整个响应结束后才决定是否执行工具。
- `src/console.rs` 只负责输入提示、上下文 footer 和密钥输入，没有负责流式输出的 renderer。
- `frontend/src/App.tsx` 目前没有数据源、状态模型或事件流连接。

当前设计的优点是边界简单：HTTP 客户端只返回完整响应，Agent 只处理完整消息。增加流式能力时，应保留这个优点，把“流式 chunk 解析”和“chunk 聚合成 ChoiceMessage”隔离出来，不让工具执行、消息历史和 UI 渲染混在一起。

## 3. DeepSeek 流式思考协议要点

根据 DeepSeek 官方 thinking mode 文档：

- 思考模式通过 `thinking: {"type": "enabled"}` 开启，思考强度通过 `reasoning_effort` 控制。
- 思考内容通过 `reasoning_content` 返回，和最终回答 `content` 同级。
- 流式调用时，请求需要设置 `stream: true`。
- 每个流式 chunk 中，思考增量位于 `choices[0].delta.reasoning_content`，最终回答增量位于 `choices[0].delta.content`。
- 如果模型进行了工具调用，产生工具调用的 assistant 消息里的 `reasoning_content` 必须在后续请求中完整回传，否则 API 可能返回 400。
- 如果模型未进行工具调用，普通 assistant 消息里的历史 `reasoning_content` 在后续上下文中不是必需项；传入 DeepSeek V4 thinking mode 会被忽略。

参考：

- DeepSeek Thinking Mode: <https://api-docs.deepseek.com/guides/thinking_mode>
- DeepSeek 思考模式中文文档：<https://api-docs.deepseek.com/zh-cn/guides/thinking_mode>

## 4. 目标

本方案分两阶段落地：

- 第一阶段：Rust CLI 支持流式展示模型思考过程和最终回答。
- 第二阶段：提供结构化 Agent 事件流，为前端展示完整 trace 做桥接。

能力目标：

- 用户输入后立即看到模型开始思考，而不是等待完整响应。
- `reasoning_content` 和 `content` 分区展示，避免思考文本和最终回答混在一起。
- 工具调用场景下，能够流式展示工具调用前的思考过程，响应结束后再执行工具。
- 流式结束后仍能聚合出与非流式 `ChoiceMessage` 等价的结构，复用现有消息历史和工具循环。
- 网络错误、半截 SSE、JSON 解析错误能够带上下文返回，避免静默卡死。
- 保留非流式调用作为 fallback，便于调试和回归。

## 5. 非目标

第一阶段不做以下事情：

- 不改变工具定义协议和工具执行语义。
- 不实现完整浏览器端 Agent 后端。
- 不把工具调用也改成边生成边执行；工具仍在 assistant response 完成后执行。
- 不在 UI 中持久保存所有思考过程到磁盘。
- 不把 DeepSeek OpenAI 格式迁移为 Anthropic 格式。

## 6. 总体设计

新增一层流式事件与聚合器：

```text
DeepSeek SSE bytes
  -> DeepSeekClient::chat_completion_stream()
      -> ChatCompletionStreamChunk
      -> AgentStreamEvent
  -> StreamAccumulator
      -> aggregated ChoiceMessage
      -> optional Usage
  -> Agent
      -> ConsoleTraceRenderer
      -> existing handle_assistant_message-like tool loop
```

核心原则：

- 客户端负责 HTTP 与 SSE 解析。
- 聚合器负责把 delta 还原成完整 assistant message。
- Agent 负责工具循环和消息历史。
- Renderer 只消费事件，不拥有业务状态。

这样 CLI、未来 WebSocket/SSE 后端和测试都可以复用同一套 `AgentStreamEvent`。

## 7. API 类型扩展

在 `src/api.rs` 增加流式响应类型，保持与现有非流式类型并存：

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionStreamChunk {
    pub id: Option<String>,
    pub object: Option<String>,
    pub created: Option<u64>,
    pub model: Option<String>,
    pub choices: Vec<StreamChoice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamChoice {
    pub index: u32,
    pub delta: ChoiceDelta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChoiceDelta {
    pub role: Option<String>,
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallDelta {
    pub index: u32,
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub function: Option<FunctionCallDelta>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FunctionCallDelta {
    pub name: Option<String>,
    pub arguments: Option<String>,
}
```

如果后续需要在最后一个 chunk 中拿到 usage，可进一步给 request 增加 `stream_options`：

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub stream_options: Option<StreamOptions>,

#[derive(Debug, Clone, Serialize)]
pub struct StreamOptions {
    pub include_usage: bool,
}
```

第一阶段可以先不强依赖 `stream_options`，因为上下文用量目前只依赖非流式 `usage`。若流式响应无法提供 usage，则 `ContextUsage` 暂时保持上一轮数值，并在输出中标注 usage unavailable；第二步再开启 include usage。

## 8. HTTP 客户端改造

在 `src/client.rs` 保留现有 `chat_completion()`，新增 `chat_completion_stream()`：

```rust
pub async fn chat_completion_stream(
    &self,
    request: &ChatCompletionRequest,
) -> Result<impl Stream<Item = Result<ChatCompletionStreamChunk>> + '_>
```

实现要点：

- 克隆 request，将 `stream` 设置为 `Some(true)`。
- 仍复用 `reqwest::Client`、默认 headers 和错误响应处理。
- 使用 `response.bytes_stream()` 读取字节流。
- 按 SSE frame 解析 `data: ...` 行。
- 遇到 `data: [DONE]` 结束。
- 跳过空行和非 `data:` 行。
- 对 JSON 解析错误附带原始 data 片段，便于调试。

建议新增依赖：

```toml
futures-util = "0.3"
```

解析策略：

```text
buffer += incoming bytes
while buffer contains "\n\n" or "\r\n\r\n":
  frame = split first frame
  for line in frame.lines():
    if line starts with "data:":
      payload = trim after "data:"
      if payload == "[DONE]": finish
      else parse ChatCompletionStreamChunk
```

注意不要把 SSE 解析散落到 Agent 中；否则后续前端桥接会重复同样逻辑。

## 9. 流式事件模型

新增建议文件：`src/streaming.rs`。

定义 Agent 层事件：

```rust
pub enum AgentStreamEvent {
    ResponseStarted { round: usize },
    ReasoningStarted,
    ReasoningDelta(String),
    AnswerStarted,
    AnswerDelta(String),
    ToolCallDelta {
        index: u32,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: Option<String>,
    },
    ResponseFinished {
        finish_reason: Option<String>,
    },
    Usage(Usage),
}
```

定义事件接收接口：

```rust
pub trait AgentEventSink {
    fn on_event(&mut self, event: &AgentStreamEvent) -> anyhow::Result<()>;
}
```

第一阶段由 CLI 使用同步 sink 即可；第二阶段可增加 `mpsc::Sender<AgentStreamEvent>` 实现，把事件发给 WebSocket/SSE 服务。

## 10. StreamAccumulator 设计

新增 `StreamAccumulator`，职责是把 chunk delta 聚合成完整 `ChoiceMessage`：

```rust
pub struct StreamAccumulator {
    role: Option<String>,
    content: String,
    reasoning_content: String,
    tool_calls: BTreeMap<u32, ToolCallBuilder>,
    finish_reason: Option<String>,
    usage: Option<Usage>,
}
```

处理规则：

- `delta.role`：记录一次即可。
- `delta.reasoning_content`：追加到 `reasoning_content`，并发出 `ReasoningDelta`。
- `delta.content`：追加到 `content`，并发出 `AnswerDelta`。
- `delta.tool_calls`：按 `index` 聚合，同一工具调用的 `function.arguments` 是字符串增量，需要追加。
- `finish_reason`：记录最后一个非空值。
- `usage`：如果流式 chunk 提供 usage，则保存并用于更新 `ContextUsage`。

聚合完成后输出：

```rust
pub struct CompletedStreamResponse {
    pub message: ChoiceMessage,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
}
```

`ChoiceMessage` 的构造要与现有非流式路径一致：

- 有工具调用时：`content` 可以是 `Some(content)` 或 `Some(String::new())`，但必须保留 `reasoning_content` 和完整 `tool_calls`。
- 无工具调用时：保留最终 `content`；是否继续保存 `reasoning_content` 可通过配置控制，默认保存以保持当前行为，但后续可优化为只在工具调用消息中回传。

## 11. Agent 改造方案

在 `src/agent.rs` 中增加流式路径：

```rust
pub async fn handle_user_input_streaming(
    &mut self,
    input: impl Into<String>,
    sink: &mut dyn AgentEventSink,
) -> Result<()>
```

也可以直接让现有 `handle_user_input()` 内部根据配置选择流式或非流式：

```text
if config.streaming.enabled:
  run_streaming_rounds(input, sink)
else:
  run_non_streaming_rounds(input)
```

每轮处理：

```text
build_request()
request.stream = Some(true)
client.chat_completion_stream(request)
for chunk in stream:
  accumulator.push(chunk, sink)
completed = accumulator.finish()
if usage exists:
  context_usage.update_from_usage(usage)
match completed.message.tool_calls:
  Some(tool_calls) -> append assistant, execute tools, append tool messages, continue
  None -> append assistant, complete
```

这里要避免把 `handle_assistant_message()` 直接承担“打印最终回答”的职责。建议拆成两个更小的函数：

```rust
async fn apply_assistant_message(&mut self, message: ChoiceMessage) -> TurnStatus
async fn execute_tool_calls_for_message(&mut self, message: &ChoiceMessage) -> TurnStatus
```

非流式路径也可以改用这些函数，减少两条路径的行为漂移。

## 12. CLI 展示方案

新增 `ConsoleTraceRenderer`，可放在 `src/console.rs` 或新文件 `src/trace_renderer.rs`。

展示原则：

- 思考区和最终回答区分开。
- TTY 下可以使用较轻量的 ANSI 样式；非 TTY 下输出纯文本。
- 每次 delta 到达就 flush stdout。
- 工具调用只展示摘要，不把完整 JSON 参数流打乱主要阅读。

建议输出形态：

```text
thinking>
...实时 reasoning_content...

agent>
...实时 content...

tool> webSearch {"query":"..."}
tool> webSearch completed
```

TTY 样式建议：

- `thinking>` 使用 dim 或灰色。
- `agent>` 使用普通亮度。
- 工具调用状态单独一行。
- 当从 thinking 切换到 answer 时自动补换行。

非 TTY 或日志场景：

- 每个区块都用明确前缀，避免控制字符污染重定向输出。
- 不使用回车覆盖或动态 spinner。

## 13. 前端展示桥接

第二阶段引入本地事件服务，而不是让 Vite 前端直接持有 DeepSeek API Key。

建议架构：

```text
React UI
  -> local backend endpoint
      POST /api/chat
      GET  /api/chat/{turn_id}/events  或 WebSocket /ws
  -> AgentEventSink implementation
      -> stream AgentStreamEvent to browser
```

事件 JSON 示例：

```json
{"type":"response_started","round":0}
{"type":"reasoning_delta","text":"..."}
{"type":"answer_delta","text":"..."}
{"type":"tool_call_delta","index":0,"name":"webSearch","arguments_delta":"..."}
{"type":"response_finished","finish_reason":"stop"}
```

前端状态模型：

```ts
type TraceSegment =
  | { type: 'reasoning'; text: string; status: 'streaming' | 'done' }
  | { type: 'answer'; text: string; status: 'streaming' | 'done' }
  | { type: 'tool'; name: string; argsPreview: string; status: 'pending' | 'running' | 'done' | 'error' }
```

UI 建议：

- 左侧为对话消息，右侧或可折叠区域为 trace。
- 思考过程默认展开或根据配置展开；如果后续考虑隐私，可增加“显示思考过程”开关。
- 工具调用以紧凑状态行展示，点击后查看参数和结果摘要。
- 不在浏览器 localStorage 持久化完整思考文本，除非用户明确开启导出。

## 14. 配置建议

在 `AppConfig` 增加：

```rust
pub struct StreamingConfig {
    pub enabled: bool,
    pub show_reasoning: bool,
    pub show_tool_call_deltas: bool,
}
```

环境变量：

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `SPARROW_STREAMING_ENABLED` | `true` | 是否启用流式模型输出 |
| `SPARROW_SHOW_REASONING` | `true` | 是否展示 `reasoning_content` |
| `SPARROW_SHOW_TOOL_CALL_DELTAS` | `false` | 是否展示工具参数增量 |

默认启用流式，是因为这是本方案的核心体验；保留关闭开关，是为了定位 API 兼容问题或终端渲染问题。

## 15. 错误处理与降级

需要覆盖的失败场景：

- API 返回非 2xx：复用现有错误 body 读取逻辑。
- SSE 中途断开：返回带上下文的错误，并保留已经打印的 partial output。
- 某个 chunk JSON 解析失败：错误中包含 payload 前 500 字符。
- 工具调用 delta 聚合不完整：返回 `invalid streamed tool call`，不要执行半截工具参数。
- 流式 usage 缺失：不更新 `ContextUsage`，保留上一轮显示。

可选降级策略：

- 如果流式请求失败且还没有输出任何 delta，可自动重试一次非流式请求。
- 如果已经输出 partial delta，不自动重试，避免用户看到重复内容。

## 16. 测试计划

单元测试：

- SSE parser 能处理 `\n\n`、`\r\n\r\n`、多 data 行和 `[DONE]`。
- `StreamAccumulator` 能聚合 reasoning/content delta。
- `StreamAccumulator` 能按 index 聚合 tool call arguments。
- 非工具调用消息聚合后与 `ChoiceMessage` 结构一致。
- 工具调用消息聚合后保留 `reasoning_content`。

集成测试：

- 用 mock SSE 流模拟 reasoning -> content -> done。
- 用 mock SSE 流模拟 reasoning -> tool_calls -> done -> tool result -> second model request。
- `SPARROW_STREAMING_ENABLED=false` 时仍走现有非流式路径。

手动验收：

- 运行 `cargo run`，输入一个需要长推理的问题，能看到 `thinking>` 逐步输出。
- 模型最终回答进入 `agent>` 区块。
- 触发 `webSearch` 或 filesystem MCP 工具时，工具执行前的思考内容不会丢失。
- 工具调用后的下一轮模型请求不会因为缺少 `reasoning_content` 返回 400。

## 17. 分阶段实施清单

第一阶段：CLI 流式展示

1. 在 `src/api.rs` 增加流式 chunk 类型。
2. 在 `src/client.rs` 增加 SSE parser 与 `chat_completion_stream()`。
3. 新增 `src/streaming.rs`，实现 `AgentStreamEvent` 与 `StreamAccumulator`。
4. 新增或扩展 console renderer，实时展示 thinking/answer/tool 状态。
5. 改造 `Agent::handle_user_input()`，通过配置选择流式路径。
6. 增加单元测试和 mock SSE 集成测试。

第二阶段：前端 trace

1. 增加本地 backend 入口，暴露 chat event stream。
2. 实现 `AgentEventSink` 的 channel/SSE/WebSocket 版本。
3. 改造 React 状态模型，消费事件并渲染 trace。
4. 增加工具调用详情面板和 reasoning 展示开关。
5. 做端到端手动验收。

第三阶段：体验与治理

1. 支持导出 trace，但默认不持久化完整 reasoning。
2. 对超长 reasoning 做前端折叠和内存上限。
3. 将上下文窗口治理方案里的 usage 与流式 usage 打通。
4. 增加模型供应商抽象，避免流式协议绑定在 Agent 层。

## 18. 推荐落地顺序

推荐先实现第一阶段，不先做前端。原因是现有项目的真实 Agent 能力在 Rust CLI 中，流式解析、聚合、工具调用回传这些关键风险都在 Rust 侧。CLI 跑通后，前端只需要消费稳定的 `AgentStreamEvent`，不必重新理解 DeepSeek SSE 协议。

最小可交付版本定义：

- `cargo test` 通过。
- `SPARROW_STREAMING_ENABLED=true` 时默认流式输出。
- 终端能区分显示 `thinking>` 和 `agent>`。
- 工具调用场景下完整保留并回传 `reasoning_content`。
- 非流式路径仍可通过配置开启。
