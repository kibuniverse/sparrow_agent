# Agent 调用过程可视化 Frontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `frontend/` 中实现聊天输入、输入框下方的思考过程简览，以及可跳转的 Agent 任务详情页，完整展示模型调用、模型输出和工具调用层级。

**Architecture:** 前端通过 `POST /api/agent/tasks` 创建任务，通过 `GET /api/agent/tasks/:task_id/events` 订阅 Agent 结构化 SSE 事件，并用本地 reducer 将事件归并为时间线节点。聊天页只展示当前任务的轻量状态和“查看详情”入口；任务详情页展示左侧调用时间线和右侧选中节点详情。Agent 侧接口与事件定义见 `docs/agent-call-visualization-agent-plan.md`。

**Tech Stack:** React 19, TypeScript 6, Vite 8, Tailwind CSS 4, browser `EventSource`, existing `pnpm` scripts.

---

## 1. 当前前端现状

`frontend/src/App.tsx` 目前只渲染一行 `show model full trace`，没有路由、状态管理、API 层或 trace UI。`frontend/src/index.css` 只引入 Tailwind 并重置 `body` margin。这个状态适合直接建立一个小而清晰的前端结构，不需要先迁移旧页面。

## 2. 用户交互目标

1. 用户在聊天页输入内容，按 Enter 或点击确认后，前端创建 Agent 任务。
2. 任务启动后，输入框底部展示当前模型调用的简短思考过程、运行状态和“查看详情”按钮。
3. 用户点击“查看详情”后，页面切换到任务详情页 `/tasks/:taskId`。
4. 任务详情页按调用顺序从上到下展示：
   - 模型调用（第 X 轮）
   - 模型输出（工具调用或生成结果）
   - 工具调用 1、工具调用 2、工具调用 3
5. 时间线中的每个节点都可以点击。点击后，右侧详情面板展示该节点的输入、输出、耗时、token、模型名或工具参数等信息。
6. 所有运行中状态使用同一个 loading 组件和同一套样式，避免模型调用、模型输出、工具调用各自展示不一致。

## 3. 页面结构

### 3.1 ChatPage

路径：`/`

主要区域：

- 消息列表：展示用户消息和最终 Agent 回复。第一版只需要支持当前浏览器会话内的消息历史。
- 输入区：`textarea` 支持 Enter 提交、Shift+Enter 换行；任务运行中禁用重复提交。
- 思考简览区：位于输入框下方，展示当前任务最近的 `model_call.reasoning_delta` 聚合文本、当前轮次、工具调用状态和详情按钮。

思考简览区文案规则：

- 模型正在思考：`模型调用（第 1 轮）正在思考`
- 已进入工具调用：`准备调用 3 个工具：webSearch、read_file、runRustWasm`
- 工具正在执行：`正在执行 webSearch`
- 正在生成最终结果：`正在生成回复`
- 失败：`任务执行失败：<错误摘要>`

思考简览只展示最近 240 个字符。完整内容进入详情页。

### 3.2 TaskDetailPage

路径：`/tasks/:taskId`

布局：

- 左侧主区域：trace 时间线，按事件顺序渲染节点树。
- 右侧固定详情面板：展示当前选中节点的详细信息。
- 顶部任务条：返回聊天页、任务状态、开始时间、总耗时。

时间线层级采用缩进和细连接线表达，不做卡片嵌套卡片：

```text
模型调用（第 1 轮）
  模型输出：工具调用
    工具调用 1：webSearch
    工具调用 2：read_file
    工具调用 3：runRustWasm
模型调用（第 2 轮）
  模型输出：生成结果
```

节点渲染规则：

- `model_call`：两行结构。第一行标题为 `模型调用（第 X 轮）`，第二行为思考过程。运行中时显示统一 loading。
- `model_output`：如果 `kind = "tool_calls"`，标题为 `工具调用` 并展示工具列表；如果 `kind = "final_answer"`，标题为 `生成结果` 并展示回答摘要。
- `tool_call`：标题为 `工具调用 X：<tool_name>`，执行中显示统一 loading；完成后显示状态、耗时和输出摘要。

## 4. 前端接口契约

前端只依赖 Agent 服务暴露的 HTTP/SSE 接口，不直接依赖 DeepSeek 原始 SSE。

### 4.1 创建任务

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

字段说明：

- `conversation_id`：可选。为空时 Agent 服务创建新会话。
- `client_message_id`：前端生成的幂等 ID，重复提交时服务端可以去重。
- `message`：用户输入内容。
- `stream`：第一版固定传 `true`。

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

### 4.2 订阅任务事件

`GET /api/agent/tasks/:task_id/events?after_seq=0`

SSE frame：

```text
event: trace
id: 12
data: {"seq":12,"task_id":"task_01JZ4N18T4BSX2G6X93K5E8GAT","conversation_id":"conv_01JZ4N0Y3GB9HKW98D5Z9F3R2A","timestamp":"2026-05-10T14:25:19.120Z","type":"tool_call.completed","payload":{"node_id":"tool_01JZ4N1ACXK7TT3B7JV0X7HHN5","duration_ms":842,"output":{"value":{"summary":"..."},"text":"{\"summary\":\"...\"}","truncated":false}}}
```

断线重连规则：

- 前端保存已处理的最大 `seq`。
- `EventSource` 断开后重新连接 `/events?after_seq=<lastSeq>`。
- 如果服务端返回 `404 task_not_found`，前端展示任务不存在。
- 如果服务端返回 `410 task_expired`，前端展示任务已过期，并引导回聊天页。

### 4.3 获取任务快照

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

前端进入详情页时先拉快照，再从快照中最大 `seq` 继续订阅 SSE。

## 5. TypeScript 数据类型

创建 `frontend/src/types/trace.ts`：

```ts
export type TaskStatus = 'running' | 'succeeded' | 'failed' | 'cancelled'
export type TraceNodeType = 'model_call' | 'model_output' | 'tool_call'
export type TraceStatus = 'pending' | 'running' | 'succeeded' | 'failed'
export type ModelOutputKind = 'tool_calls' | 'final_answer'

export interface TraceEventEnvelope<TType extends string = string, TPayload = unknown> {
  seq: number
  task_id: string
  conversation_id: string
  timestamp: string
  type: TType
  payload: TPayload
}

export type TraceEvent =
  | TraceEventEnvelope<'task.started', TaskStartedPayload>
  | TraceEventEnvelope<'task.completed', TaskCompletedPayload>
  | TraceEventEnvelope<'task.failed', TaskFailedPayload>
  | TraceEventEnvelope<'model_call.started', ModelCallStartedPayload>
  | TraceEventEnvelope<'model_call.reasoning_delta', ModelCallReasoningDeltaPayload>
  | TraceEventEnvelope<'model_call.completed', ModelCallCompletedPayload>
  | TraceEventEnvelope<'model_output.started', ModelOutputStartedPayload>
  | TraceEventEnvelope<'model_output.delta', ModelOutputDeltaPayload>
  | TraceEventEnvelope<'model_output.completed', ModelOutputCompletedPayload>
  | TraceEventEnvelope<'tool_call.started', ToolCallStartedPayload>
  | TraceEventEnvelope<'tool_call.completed', ToolCallCompletedPayload>
  | TraceEventEnvelope<'tool_call.failed', ToolCallFailedPayload>

export interface TraceNode {
  id: string
  taskId: string
  parentId: string | null
  type: TraceNodeType
  status: TraceStatus
  title: string
  subtitle: string
  round: number | null
  startedAt: string | null
  completedAt: string | null
  durationMs: number | null
  childrenIds: string[]
  detail: TraceNodeDetail
}

export type TraceNodeDetail =
  | ModelCallDetail
  | ModelOutputDetail
  | ToolCallDetail

export interface ModelCallDetail {
  type: 'model_call'
  model: string
  request: JsonSnapshot
  response: JsonSnapshot | null
  reasoningText: string
  usage: TokenUsage | null
  finishReason: string | null
}

export interface ModelOutputDetail {
  type: 'model_output'
  kind: ModelOutputKind
  content: string
  toolCalls: ToolCallPreview[]
}

export interface ToolCallDetail {
  type: 'tool_call'
  toolCallId: string
  name: string
  arguments: JsonSnapshot
  output: JsonSnapshot | null
  error: string | null
}

export interface JsonSnapshot {
  value: unknown
  text: string
  truncated: boolean
}

export interface TokenUsage {
  prompt_tokens: number
  completion_tokens: number
  total_tokens: number
  reasoning_tokens: number
}

export interface ToolCallPreview {
  nodeId: string
  toolCallId: string
  name: string
  arguments: JsonSnapshot
}

export interface TaskStartedPayload {
  message: {
    role: 'user'
    content: string
  }
}

export interface TaskCompletedPayload {
  duration_ms: number
  final_answer: string
}

export interface TaskFailedPayload {
  duration_ms: number
  error: string
}

export interface ModelCallStartedPayload {
  node_id: string
  round: number
  model: string
  request: JsonSnapshot
}

export interface ModelCallReasoningDeltaPayload {
  node_id: string
  delta: string
}

export interface ModelCallCompletedPayload {
  node_id: string
  duration_ms: number
  finish_reason: string | null
  usage: TokenUsage | null
  response: JsonSnapshot
}

export interface ModelOutputStartedPayload {
  node_id: string
  parent_model_call_id: string
  kind: ModelOutputKind
}

export type ModelOutputDeltaPayload =
  | {
      node_id: string
      kind: 'final_answer'
      content_delta: string
    }
  | {
      node_id: string
      kind: 'tool_calls'
      tool_call: {
        index: number
        tool_call_id: string | null
        name: string | null
        arguments_delta: string | null
      }
    }

export interface ModelOutputCompletedPayload {
  node_id: string
  kind: ModelOutputKind
  content: string
  tool_calls: Array<{
    index: number
    tool_call_id: string
    name: string
    arguments: JsonSnapshot
  }>
}

export interface ToolCallStartedPayload {
  node_id: string
  parent_model_output_id: string
  index: number
  tool_call_id: string
  name: string
  arguments: JsonSnapshot
}

export interface ToolCallCompletedPayload {
  node_id: string
  duration_ms: number
  output: JsonSnapshot
}

export interface ToolCallFailedPayload {
  node_id: string
  duration_ms: number
  error: string
}
```

每个 payload 的字段与 Agent 文档保持一致。前端 reducer 不猜测服务端内部状态，只根据事件显式更新节点。

## 6. 前端状态模型

创建 `frontend/src/state/traceReducer.ts`。

状态结构：

```ts
export interface TraceState {
  taskId: string | null
  conversationId: string | null
  status: TaskStatus | 'idle'
  lastSeq: number
  rootNodeIds: string[]
  nodesById: Record<string, TraceNode>
  selectedNodeId: string | null
  latestReasoningText: string
  latestRunningNodeId: string | null
  finalAnswer: string
  error: string | null
}
```

归并规则：

- `task.started`：设置 `taskId`、`conversationId`、`status = "running"`。
- `model_call.started`：创建顶层 `model_call` 节点，默认选中该节点。
- `model_call.reasoning_delta`：追加到对应模型节点 `detail.reasoningText`，同时更新 `latestReasoningText`。
- `model_output.started`：创建 `model_output` 节点并挂到 `parent_model_call_id`。
- `model_output.delta`：如果是最终回答则追加 `content`；如果是工具调用则更新 `toolCalls` 列表中的工具名称和参数。
- `tool_call.started`：创建 `tool_call` 节点并挂到 `parent_model_output_id`。
- `tool_call.completed`：写入输出、耗时，状态改为 `succeeded`。
- `tool_call.failed`：写入错误、耗时，状态改为 `failed`。
- `model_call.completed`：写入 usage、finish reason、响应快照、耗时，状态改为 `succeeded`。
- `task.completed`：状态改为 `succeeded`，写入最终回答。
- `task.failed`：状态改为 `failed`，写入错误摘要。

重复事件处理：

- 如果 `event.seq <= state.lastSeq`，直接忽略。
- 节点创建事件如果遇到已存在 `node_id`，只合并字段，不重复添加 `childrenIds`。

## 7. 文件改造清单

### Create

- `frontend/src/types/trace.ts`：Trace 类型和 payload 类型。
- `frontend/src/api/agentTrace.ts`：`createAgentTask()`、`getTaskSnapshot()`、SSE URL 组装和错误解析。
- `frontend/src/hooks/useTaskStream.ts`：封装 `EventSource`、断线重连和 `after_seq`。
- `frontend/src/state/traceReducer.ts`：事件归并和 UI 状态。
- `frontend/src/components/LoadingInline.tsx`：统一 loading。
- `frontend/src/components/ChatComposer.tsx`：输入框和确认按钮。
- `frontend/src/components/ThinkingPreview.tsx`：输入框底部思考简览和详情按钮。
- `frontend/src/components/TraceTimeline.tsx`：详情页左侧时间线。
- `frontend/src/components/TraceNodeRow.tsx`：单个时间线节点。
- `frontend/src/components/TraceDetailPanel.tsx`：右侧详情面板。
- `frontend/src/components/JsonBlock.tsx`：格式化展示 JSON/text snapshot。
- `frontend/src/pages/ChatPage.tsx`：聊天页。
- `frontend/src/pages/TaskDetailPage.tsx`：任务详情页。
- `frontend/src/router.ts`：基于 History API 的轻量路由。

### Modify

- `frontend/src/App.tsx`：接入路由和全局 trace 状态。
- `frontend/src/index.css`：补充基础布局、统一 loading、时间线和详情面板样式。
- `frontend/vite.config.ts`：开发环境代理 `/api` 到 Agent 服务默认地址 `http://127.0.0.1:8787`。
- `frontend/package.json`：增加 `test` 脚本和测试依赖。

## 8. UI 样式规范

统一 loading 组件：

```tsx
export function LoadingInline({ label = '运行中' }: { label?: string }) {
  return (
    <span className="inline-flex items-center gap-2 text-sm text-slate-600" aria-live="polite">
      <span className="h-2 w-2 rounded-full bg-sky-500 motion-safe:animate-pulse" />
      <span>{label}</span>
    </span>
  )
}
```

布局颜色建议：

- 页面背景：`#f8fafc`
- 主文本：`#0f172a`
- 次级文本：`#475569`
- 边框：`#cbd5e1`
- 运行中 accent：`#0284c7`
- 成功 accent：`#16a34a`
- 失败 accent：`#dc2626`

交互规则：

- 时间线节点 hover 时只改变背景和左侧状态线颜色。
- 选中节点使用 `aria-current="true"`。
- 右侧详情面板在桌面端 sticky，在窄屏下移动到时间线下方。
- JSON 内容超过 280px 高度时内部滚动，不撑破页面。
- 所有按钮有明确 `aria-label`；Enter 提交只在输入法 composition 结束后触发。

## 9. 实施任务

### Task 1: 建立 Trace 类型和 API 层

**Files:**

- Create: `frontend/src/types/trace.ts`
- Create: `frontend/src/api/agentTrace.ts`
- Modify: `frontend/vite.config.ts`

- [ ] 定义第 5 节中的 TypeScript 类型。
- [ ] 实现 `createAgentTask(request)`，成功返回 `task_id`、`conversation_id`、`events_url`、`snapshot_url`。
- [ ] 实现 `getTaskSnapshot(taskId)`，返回任务快照和历史事件。
- [ ] 在 Vite dev server 中增加 `/api` proxy，目标为 `http://127.0.0.1:8787`。
- [ ] 运行 `pnpm build`，确认类型通过。

### Task 2: 实现事件 reducer

**Files:**

- Create: `frontend/src/state/traceReducer.ts`

- [ ] 实现 `createInitialTraceState()`。
- [ ] 实现 `applyTraceEvent(state, event)`，覆盖第 6 节的所有事件归并规则。
- [ ] 实现 `applyTraceSnapshot(state, snapshot)`，按 `seq` 顺序回放历史事件。
- [ ] 添加 Vitest 测试，覆盖模型调用、工具调用、最终回答和重复事件忽略。
- [ ] 运行 `pnpm test -- traceReducer`。

### Task 3: 实现 SSE Hook

**Files:**

- Create: `frontend/src/hooks/useTaskStream.ts`

- [ ] 使用 `EventSource` 订阅 `/api/agent/tasks/:task_id/events?after_seq=<lastSeq>`。
- [ ] 解析 `trace` event，调用 reducer dispatch。
- [ ] 在 `error` 事件中关闭当前连接并按 1s、2s、5s、10s 退避重连。
- [ ] 组件卸载时关闭 `EventSource`，避免旧任务继续写入状态。
- [ ] 添加测试，模拟两次断线后从 `lastSeq` 继续。

### Task 4: 构建聊天页

**Files:**

- Create: `frontend/src/pages/ChatPage.tsx`
- Create: `frontend/src/components/ChatComposer.tsx`
- Create: `frontend/src/components/ThinkingPreview.tsx`
- Create: `frontend/src/components/LoadingInline.tsx`
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/index.css`

- [ ] 用户输入后调用 `createAgentTask()`。
- [ ] 创建任务成功后立即订阅 SSE。
- [ ] 在输入框下方展示 `ThinkingPreview`。
- [ ] 点击“查看详情”使用 History API 跳转到 `/tasks/:taskId`。
- [ ] 任务运行中禁止重复提交，同步展示 loading。
- [ ] 运行 `pnpm build` 和 `pnpm lint`。

### Task 5: 构建任务详情页

**Files:**

- Create: `frontend/src/pages/TaskDetailPage.tsx`
- Create: `frontend/src/components/TraceTimeline.tsx`
- Create: `frontend/src/components/TraceNodeRow.tsx`
- Create: `frontend/src/components/TraceDetailPanel.tsx`
- Create: `frontend/src/components/JsonBlock.tsx`
- Create: `frontend/src/router.ts`
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/index.css`

- [ ] 进入 `/tasks/:taskId` 时先调用 `getTaskSnapshot()`。
- [ ] 快照加载完成后从最大 `seq` 继续订阅 SSE。
- [ ] 左侧时间线按 `rootNodeIds` 和 `childrenIds` 渲染层级。
- [ ] 点击任意节点后在右侧详情面板展示 `TraceNodeDetail`。
- [ ] 当没有选中节点时，默认选中最近运行中节点；任务完成后默认选中最终输出节点。
- [ ] 窄屏下详情面板移动到时间线下方。
- [ ] 运行 `pnpm build` 和 `pnpm lint`。

### Task 6: 前端集成验证

**Files:**

- Modify: `frontend/package.json`

- [ ] 增加 `test` 脚本：`vitest run`。
- [ ] 增加测试依赖：`vitest`、`@testing-library/react`、`@testing-library/jest-dom`、`jsdom`。
- [ ] 使用 mock SSE 事件验证完整链路：创建任务、展示简览、跳转详情、点击工具节点、查看右侧详情。
- [ ] 运行 `pnpm test`、`pnpm lint`、`pnpm build`。

## 10. 验收标准

- 用户提交消息后 1 秒内页面出现任务运行状态或明确错误。
- 输入框底部能看到当前模型调用的思考过程简览。
- “查看详情”能打开 `/tasks/:taskId`，刷新详情页后能恢复历史 trace。
- 详情页严格按 `模型调用 -> 模型输出 -> 工具调用` 的层级展示。
- 模型调用、模型输出、工具调用节点都能点击，并在右侧展示对应详情。
- 运行中状态全部使用 `LoadingInline`。
- 断线重连不会重复渲染已处理事件。
- `pnpm test`、`pnpm lint`、`pnpm build` 全部通过。
