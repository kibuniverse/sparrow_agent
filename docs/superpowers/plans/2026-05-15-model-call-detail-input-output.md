# Model Call Detail Input Output Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让可视化调用详情页中的“模型调用”节点能查看本次调用模型时完整的输入 messages 列表，以及模型返回的详细输出。

**Architecture:** 后端继续通过现有 `model_call.started` 与 `model_call.completed` trace 事件传递详情，但把 `request` 从摘要扩展为包含 `messages` 的可截断 JSON 快照，把 `response` 从摘要扩展为包含 assistant message、finish reason 和 usage 的输出快照。前端保持 reducer 的节点结构不变，在详情面板中从 `JsonSnapshot.value` 解析结构化 messages 和 response message，优先展示易读列表，同时保留原始 JSON 作为调试兜底。旧 trace archive 没有详细字段时保持兼容展示。

**Tech Stack:** Rust 2024, serde/serde_json, existing trace `JsonSnapshot`, React 19, TypeScript 6, Tailwind CSS 4, Vitest, Testing Library.

---

## File Structure

- Modify: `src/agent.rs`
  - Replace the model request snapshot helper with a free helper that records `messages`.
  - Add a response snapshot helper that records the assistant output message.
  - Add unit tests for both helpers inside the existing `#[cfg(test)] mod tests`.
- No change expected: `src/trace.rs`
  - Existing `JsonSnapshot::from_value()` already redacts secret-keyed fields and truncates large payloads.
- Modify: `frontend/src/types/trace.ts`
  - Add lightweight TypeScript interfaces for model request/response snapshot values.
  - Keep existing event payload shape compatible: `request` and `response` remain `JsonSnapshot`.
- Create: `frontend/src/components/ModelMessageList.tsx`
  - Render a readable list of chat messages with role, content, reasoning content, tool calls, and tool call ids.
  - Provide safe parsing helpers for old snapshots and malformed data.
- Modify: `frontend/src/components/TraceDetailPanel.tsx`
  - For `model_call` detail, show “输入 messages” and “模型输出” sections before raw JSON sections.
  - Keep existing reasoning, token, request JSON, and response JSON sections.
- Create: `frontend/src/components/TraceDetailPanel.test.tsx`
  - Verify model input messages and output content/tool calls appear in the detail panel.
- Modify: `frontend/src/state/traceReducer.test.ts`
  - Verify reducer preserves detailed request/response snapshots on model call nodes.
- Modify if needed: `frontend/src/App.test.tsx`
  - Update mocked model call events to use the new richer snapshot shape only if type checks require it.

## Contract

### `model_call.started` payload after this change

```json
{
  "node_id": "model_01H...",
  "round": 1,
  "model": "deepseek-v4-pro",
  "request": {
    "value": {
      "model": "deepseek-v4-pro",
      "message_count": 3,
      "messages": [
        {
          "role": "system",
          "content": "You are Sparrow...",
          "tool_calls": null,
          "tool_call_id": null,
          "reasoning_content": null
        },
        {
          "role": "user",
          "content": "分析仓库",
          "tool_calls": null,
          "tool_call_id": null,
          "reasoning_content": null
        },
        {
          "role": "tool",
          "content": "{\"files\":[\"Cargo.toml\"]}",
          "tool_calls": null,
          "tool_call_id": "call_1",
          "reasoning_content": null
        }
      ],
      "tool_count": 6,
      "thinking": { "type": "enabled" },
      "reasoning_effort": "high"
    },
    "text": "...",
    "truncated": false
  }
}
```

### `model_call.completed` payload after this change

```json
{
  "node_id": "model_01H...",
  "duration_ms": 812,
  "finish_reason": "tool_calls",
  "usage": {
    "prompt_tokens": 100,
    "completion_tokens": 20,
    "total_tokens": 120,
    "reasoning_tokens": 8
  },
  "response": {
    "value": {
      "message": {
        "role": "assistant",
        "content": "",
        "reasoning_content": "需要先查看 Cargo.toml。",
        "tool_calls": [
          {
            "id": "call_1",
            "type": "function",
            "function": {
              "name": "read_file",
              "arguments": "{\"path\":\"Cargo.toml\"}"
            }
          }
        ]
      },
      "finish_reason": "tool_calls",
      "usage": {
        "prompt_tokens": 100,
        "completion_tokens": 20,
        "total_tokens": 120,
        "reasoning_tokens": 8
      },
      "has_content": false,
      "tool_call_count": 1
    },
    "text": "...",
    "truncated": false
  }
}
```

## Task 1: Backend Model Call Snapshots

**Files:**
- Modify: `src/agent.rs`

- [ ] **Step 1: Write failing tests for detailed request and response snapshots**

Add the following imports to the existing `#[cfg(test)] mod tests` import block in `src/agent.rs`:

```rust
use crate::api::{
    ChatCompletionRequest, ChatMessage, ChoiceMessage, CompletionTokensDetails, FunctionCall,
    PromptTokensDetails, ThinkingConfig, ToolCall, Usage,
};
use serde_json::json;
```

Add these tests inside the same test module:

```rust
#[test]
fn model_request_snapshot_includes_full_messages() {
    let request = ChatCompletionRequest {
        model: "deepseek-v4-pro".into(),
        messages: vec![
            ChatMessage::system("system prompt"),
            ChatMessage::user("user question"),
            ChatMessage::tool(r#"{"result":"ok"}"#, "call_1"),
        ],
        tools: None,
        thinking: Some(ThinkingConfig::enabled()),
        reasoning_effort: Some("high".into()),
        stream: None,
        stream_options: None,
    };

    let snapshot = model_request_snapshot(&request);

    assert_eq!(snapshot.value["model"], "deepseek-v4-pro");
    assert_eq!(snapshot.value["message_count"], 3);
    assert_eq!(snapshot.value["messages"][0]["role"], "system");
    assert_eq!(snapshot.value["messages"][1]["content"], "user question");
    assert_eq!(snapshot.value["messages"][2]["tool_call_id"], "call_1");
    assert_eq!(snapshot.value["thinking"], json!({ "type": "enabled" }));
}

#[test]
fn model_response_snapshot_includes_assistant_message_output() {
    let message = ChoiceMessage {
        role: "assistant".into(),
        content: Some(String::new()),
        reasoning_content: Some("Need to inspect files.".into()),
        tool_calls: Some(vec![ToolCall {
            id: "call_1".into(),
            kind: "function".into(),
            function: FunctionCall {
                name: "read_file".into(),
                arguments: r#"{"path":"Cargo.toml"}"#.into(),
            },
        }]),
    };
    let usage = Usage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
        prompt_tokens_details: PromptTokensDetails { cached_tokens: 0 },
        completion_tokens_details: CompletionTokensDetails {
            reasoning_tokens: 3,
        },
        prompt_cache_hit_tokens: 0,
        prompt_cache_miss_tokens: 10,
    };

    let snapshot = model_response_snapshot(&message, Some("tool_calls"), Some(&usage));

    assert_eq!(snapshot.value["message"]["role"], "assistant");
    assert_eq!(
        snapshot.value["message"]["reasoning_content"],
        "Need to inspect files."
    );
    assert_eq!(snapshot.value["message"]["tool_calls"][0]["function"]["name"], "read_file");
    assert_eq!(snapshot.value["finish_reason"], "tool_calls");
    assert_eq!(snapshot.value["usage"]["reasoning_tokens"], 3);
    assert_eq!(snapshot.value["tool_call_count"], 1);
}
```

- [ ] **Step 2: Run the backend unit tests and confirm they fail**

Run:

```bash
cargo test agent::tests::model_request_snapshot_includes_full_messages agent::tests::model_response_snapshot_includes_assistant_message_output
```

Expected: FAIL because `model_request_snapshot` is still an `Agent` method and `model_response_snapshot` does not exist.

- [ ] **Step 3: Replace the request snapshot method call with a free helper**

In `Agent::run_streaming_trace_loop()`, change:

```rust
"request": self.model_request_snapshot(&request),
```

to:

```rust
"request": model_request_snapshot(&request),
```

- [ ] **Step 4: Replace the response snapshot payload with the detailed helper**

In the `TraceEventType::ModelCallCompleted` emit payload, replace the current `response` block:

```rust
"response": JsonSnapshot::from_value(
    json!({
        "has_content": completed
            .message
            .content
            .as_deref()
            .is_some_and(|content| !content.is_empty()),
        "tool_call_count": completed
            .message
            .tool_calls
            .as_ref()
            .map(|tool_calls| tool_calls.len())
            .unwrap_or(0),
    }),
    DEFAULT_SNAPSHOT_MAX_BYTES,
),
```

with:

```rust
"response": model_response_snapshot(
    &completed.message,
    completed.finish_reason.as_deref(),
    completed.usage.as_ref(),
),
```

- [ ] **Step 5: Move and expand the request snapshot helper**

Delete the `impl Agent` method:

```rust
fn model_request_snapshot(&self, request: &ChatCompletionRequest) -> JsonSnapshot {
    JsonSnapshot::from_value(
        json!({
            "model": request.model,
            "message_count": request.messages.len(),
            "tool_count": request.tools.as_ref().map(|tools| tools.len()).unwrap_or(0),
            "thinking": request.thinking,
            "reasoning_effort": request.reasoning_effort,
        }),
        DEFAULT_SNAPSHOT_MAX_BYTES,
    )
}
```

Add this free helper near `trace_usage()`:

```rust
fn model_request_snapshot(request: &ChatCompletionRequest) -> JsonSnapshot {
    JsonSnapshot::from_value(
        json!({
            "model": request.model,
            "message_count": request.messages.len(),
            "messages": request.messages,
            "tool_count": request.tools.as_ref().map(|tools| tools.len()).unwrap_or(0),
            "thinking": request.thinking,
            "reasoning_effort": request.reasoning_effort,
        }),
        DEFAULT_SNAPSHOT_MAX_BYTES,
    )
}
```

- [ ] **Step 6: Add the model response snapshot helper**

Add this helper next to `model_request_snapshot()`:

```rust
fn model_response_snapshot(
    message: &ChoiceMessage,
    finish_reason: Option<&str>,
    usage: Option<&Usage>,
) -> JsonSnapshot {
    JsonSnapshot::from_value(
        json!({
            "message": {
                "role": message.role,
                "content": message.content,
                "reasoning_content": message.reasoning_content,
                "tool_calls": message.tool_calls,
            },
            "finish_reason": finish_reason,
            "usage": usage.map(trace_usage),
            "has_content": message
                .content
                .as_deref()
                .is_some_and(|content| !content.is_empty()),
            "tool_call_count": message
                .tool_calls
                .as_ref()
                .map(|tool_calls| tool_calls.len())
                .unwrap_or(0),
        }),
        DEFAULT_SNAPSHOT_MAX_BYTES,
    )
}
```

- [ ] **Step 7: Run the targeted backend tests**

Run:

```bash
cargo test agent::tests::model_request_snapshot_includes_full_messages agent::tests::model_response_snapshot_includes_assistant_message_output
```

Expected: PASS.

- [ ] **Step 8: Format Rust code**

Run:

```bash
cargo fmt
```

Expected: no output or only formatting changes in touched Rust files.

- [ ] **Step 9: Commit backend snapshot changes**

```bash
git add src/agent.rs
git commit -m "feat: include model call input and output snapshots"
```

## Task 2: Frontend Types And Snapshot Helpers

**Files:**
- Modify: `frontend/src/types/trace.ts`
- Create: `frontend/src/components/ModelMessageList.tsx`

- [ ] **Step 1: Add frontend model snapshot interfaces**

Append these interfaces after `JsonSnapshot` in `frontend/src/types/trace.ts`:

```ts
export interface ModelToolCallSnapshot {
  id?: string | null
  type?: string | null
  function?: {
    name?: string | null
    arguments?: string | null
  } | null
}

export interface ModelMessageSnapshot {
  role: string
  content?: string | null
  reasoning_content?: string | null
  tool_calls?: ModelToolCallSnapshot[] | null
  tool_call_id?: string | null
}

export interface ModelRequestSnapshotValue {
  model?: string
  message_count?: number
  messages?: ModelMessageSnapshot[]
  tool_count?: number
  thinking?: unknown
  reasoning_effort?: string | null
}

export interface ModelResponseSnapshotValue {
  message?: ModelMessageSnapshot | null
  finish_reason?: string | null
  usage?: TokenUsage | null
  has_content?: boolean
  tool_call_count?: number
}
```

- [ ] **Step 2: Create the message list component with parsing helpers**

Create `frontend/src/components/ModelMessageList.tsx`:

```tsx
import type {
  JsonSnapshot,
  ModelMessageSnapshot,
  ModelRequestSnapshotValue,
  ModelResponseSnapshotValue,
} from '../types/trace'
import { JsonBlock } from './JsonBlock'

interface ModelMessageListProps {
  messages: ModelMessageSnapshot[]
  emptyLabel: string
}

export function ModelMessageList({ messages, emptyLabel }: ModelMessageListProps) {
  if (messages.length === 0) {
    return <p className="text-sm text-slate-500">{emptyLabel}</p>
  }

  return (
    <ol className="space-y-3">
      {messages.map((message, index) => (
        <li key={`${message.role}-${index}`} className="border-l-2 border-slate-300 pl-3">
          <div className="flex flex-wrap items-center gap-2">
            <span className="rounded bg-slate-200 px-2 py-0.5 text-xs font-medium text-slate-700">
              {message.role}
            </span>
            {message.tool_call_id ? (
              <span className="text-xs text-slate-500">tool_call_id: {message.tool_call_id}</span>
            ) : null}
          </div>
          {message.content ? (
            <p className="mt-2 whitespace-pre-wrap text-sm text-slate-800">{message.content}</p>
          ) : null}
          {message.reasoning_content ? (
            <p className="mt-2 whitespace-pre-wrap text-sm text-slate-600">
              {message.reasoning_content}
            </p>
          ) : null}
          {message.tool_calls?.length ? (
            <div className="mt-2 space-y-2">
              {message.tool_calls.map((toolCall, toolIndex) => (
                <div key={toolCall.id ?? toolIndex} className="rounded-md bg-slate-100 p-2">
                  <p className="text-sm font-medium text-slate-800">
                    {toolCall.function?.name ?? `工具 ${toolIndex + 1}`}
                  </p>
                  {toolCall.id ? (
                    <p className="mt-1 text-xs text-slate-500">id: {toolCall.id}</p>
                  ) : null}
                  {toolCall.function?.arguments ? (
                    <JsonBlock snapshot={textJsonSnapshot(toolCall.function.arguments)} />
                  ) : null}
                </div>
              ))}
            </div>
          ) : null}
        </li>
      ))}
    </ol>
  )
}

export function requestMessagesFromSnapshot(snapshot: JsonSnapshot | null): ModelMessageSnapshot[] {
  const value = snapshot?.value as ModelRequestSnapshotValue | null | undefined
  return Array.isArray(value?.messages) ? value.messages : []
}

export function responseMessageFromSnapshot(snapshot: JsonSnapshot | null): ModelMessageSnapshot[] {
  const value = snapshot?.value as ModelResponseSnapshotValue | null | undefined
  return value?.message ? [value.message] : []
}

function textJsonSnapshot(text: string): JsonSnapshot {
  return {
    value: parseJsonOrText(text),
    text,
    truncated: false,
  }
}

function parseJsonOrText(text: string): unknown {
  try {
    return JSON.parse(text)
  } catch {
    return text
  }
}
```

- [ ] **Step 3: Run frontend typecheck and confirm the new file compiles**

Run:

```bash
cd frontend && pnpm build
```

Expected: PASS or unrelated existing failures. If the new component fails lint/typecheck, fix the new file before continuing.

- [ ] **Step 4: Commit frontend types and helpers**

```bash
git add frontend/src/types/trace.ts frontend/src/components/ModelMessageList.tsx
git commit -m "feat: add model message snapshot helpers"
```

## Task 3: Model Call Detail Panel UI

**Files:**
- Modify: `frontend/src/components/TraceDetailPanel.tsx`

- [ ] **Step 1: Import the new helpers**

At the top of `TraceDetailPanel.tsx`, add:

```tsx
import {
  ModelMessageList,
  requestMessagesFromSnapshot,
  responseMessageFromSnapshot,
} from './ModelMessageList'
```

- [ ] **Step 2: Add structured input and output sections for model calls**

In the `node.detail.type === 'model_call'` branch, place these sections after the reasoning section and before the raw request JSON section:

```tsx
<section>
  <h3 className="mb-2 text-sm font-medium text-slate-950">输入 messages</h3>
  <ModelMessageList
    messages={requestMessagesFromSnapshot(node.detail.request)}
    emptyLabel="该 trace 未包含详细 messages"
  />
</section>
<section>
  <h3 className="mb-2 text-sm font-medium text-slate-950">模型输出</h3>
  <ModelMessageList
    messages={responseMessageFromSnapshot(node.detail.response)}
    emptyLabel={node.status === 'running' ? '运行中' : '暂无输出'}
  />
</section>
```

Keep these existing raw JSON sections after the structured sections:

```tsx
<section>
  <h3 className="mb-2 text-sm font-medium text-slate-950">请求 JSON</h3>
  <JsonBlock snapshot={node.detail.request} />
</section>
<section>
  <h3 className="mb-2 text-sm font-medium text-slate-950">响应 JSON</h3>
  <JsonBlock snapshot={node.detail.response} />
</section>
```

- [ ] **Step 3: Run frontend tests**

Run:

```bash
cd frontend && pnpm test -- TraceDetailPanel
```

Expected: no tests found until Task 4 creates them, or PASS if the test file already exists.

- [ ] **Step 4: Commit detail panel UI changes**

```bash
git add frontend/src/components/TraceDetailPanel.tsx
git commit -m "feat: show model call messages in detail panel"
```

## Task 4: Frontend Regression Tests

**Files:**
- Create: `frontend/src/components/TraceDetailPanel.test.tsx`
- Modify: `frontend/src/state/traceReducer.test.ts`

- [ ] **Step 1: Add a detail panel component test**

Create `frontend/src/components/TraceDetailPanel.test.tsx`:

```tsx
/**
 * @vitest-environment jsdom
 */
import '@testing-library/jest-dom/vitest'
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { TraceDetailPanel } from './TraceDetailPanel'
import type { TraceNode } from '../types/trace'

const snapshot = (value: unknown) => ({
  value,
  text: JSON.stringify(value),
  truncated: false,
})

describe('TraceDetailPanel', () => {
  it('renders model call input messages and output message', () => {
    const node: TraceNode = {
      id: 'model_1',
      taskId: 'task_1',
      parentId: null,
      type: 'model_call',
      status: 'succeeded',
      title: '模型调用（第 1 轮）',
      subtitle: '完成',
      round: 1,
      startedAt: '2026-05-15T00:00:00.000Z',
      completedAt: '2026-05-15T00:00:01.000Z',
      durationMs: 1000,
      childrenIds: [],
      detail: {
        type: 'model_call',
        model: 'deepseek-v4-pro',
        request: snapshot({
          model: 'deepseek-v4-pro',
          message_count: 2,
          messages: [
            { role: 'system', content: 'system prompt' },
            { role: 'user', content: '分析仓库' },
          ],
        }),
        response: snapshot({
          message: {
            role: 'assistant',
            content: '我会先读取 Cargo.toml。',
            reasoning_content: 'Need repo entry.',
            tool_calls: [
              {
                id: 'call_1',
                type: 'function',
                function: {
                  name: 'read_file',
                  arguments: '{"path":"Cargo.toml"}',
                },
              },
            ],
          },
          finish_reason: 'tool_calls',
          tool_call_count: 1,
        }),
        reasoningText: 'Need repo entry.',
        usage: {
          prompt_tokens: 10,
          completion_tokens: 5,
          total_tokens: 15,
          reasoning_tokens: 3,
        },
        finishReason: 'tool_calls',
      },
    }

    render(<TraceDetailPanel node={node} />)

    expect(screen.getByRole('heading', { name: '模型调用（第 1 轮）' })).toBeInTheDocument()
    expect(screen.getByText('输入 messages')).toBeInTheDocument()
    expect(screen.getByText('system prompt')).toBeInTheDocument()
    expect(screen.getByText('分析仓库')).toBeInTheDocument()
    expect(screen.getByText('模型输出')).toBeInTheDocument()
    expect(screen.getByText('我会先读取 Cargo.toml。')).toBeInTheDocument()
    expect(screen.getByText('read_file')).toBeInTheDocument()
    expect(screen.getByText(/"path": "Cargo.toml"/)).toBeInTheDocument()
  })

  it('keeps old model call traces readable when detailed messages are absent', () => {
    const node: TraceNode = {
      id: 'model_old',
      taskId: 'task_old',
      parentId: null,
      type: 'model_call',
      status: 'succeeded',
      title: '模型调用（第 1 轮）',
      subtitle: '完成',
      round: 1,
      startedAt: null,
      completedAt: null,
      durationMs: null,
      childrenIds: [],
      detail: {
        type: 'model_call',
        model: 'deepseek-chat',
        request: snapshot({ model: 'deepseek-chat', message_count: 2 }),
        response: snapshot({ has_content: true, tool_call_count: 0 }),
        reasoningText: '',
        usage: null,
        finishReason: null,
      },
    }

    render(<TraceDetailPanel node={node} />)

    expect(screen.getByText('该 trace 未包含详细 messages')).toBeInTheDocument()
    expect(screen.getByText('暂无输出')).toBeInTheDocument()
    expect(screen.getByText(/"message_count": 2/)).toBeInTheDocument()
  })
})
```

- [ ] **Step 2: Add reducer preservation assertions**

In `frontend/src/state/traceReducer.test.ts`, update the first model call event payload to include detailed snapshots:

```ts
request: jsonSnapshot({
  model: 'deepseek-chat',
  message_count: 2,
  messages: [
    { role: 'system', content: 'system prompt' },
    { role: 'user', content: 'Inspect repo' },
  ],
}),
```

Add a `model_call.completed` event after the reasoning event:

```ts
{
  seq: 4,
  task_id: 'task_1',
  conversation_id: 'conv_1',
  timestamp: '2026-05-10T01:00:02.500Z',
  type: 'model_call.completed',
  payload: {
    node_id: 'model_1',
    duration_ms: 500,
    finish_reason: 'tool_calls',
    usage: null,
    response: jsonSnapshot({
      message: {
        role: 'assistant',
        content: '',
        reasoning_content: 'Need repository map.',
        tool_calls: [
          {
            id: 'call_1',
            type: 'function',
            function: {
              name: 'read_file',
              arguments: '{"path":"Cargo.toml"}',
            },
          },
        ],
      },
      finish_reason: 'tool_calls',
      tool_call_count: 1,
    }),
  },
},
```

Increment the following event `seq` values in that test by one. Add these assertions near the existing `model_1.detail` assertions:

```ts
expect(state.nodesById.model_1.detail).toMatchObject({
  type: 'model_call',
  request: {
    value: {
      messages: [
        { role: 'system', content: 'system prompt' },
        { role: 'user', content: 'Inspect repo' },
      ],
    },
  },
  response: {
    value: {
      message: {
        role: 'assistant',
        tool_calls: [{ function: { name: 'read_file' } }],
      },
    },
  },
})
```

- [ ] **Step 3: Run frontend targeted tests**

Run:

```bash
cd frontend && pnpm test -- TraceDetailPanel traceReducer
```

Expected: PASS.

- [ ] **Step 4: Run frontend build**

Run:

```bash
cd frontend && pnpm build
```

Expected: PASS.

- [ ] **Step 5: Commit frontend regression tests**

```bash
git add frontend/src/components/TraceDetailPanel.test.tsx frontend/src/state/traceReducer.test.ts
git commit -m "test: cover model call input and output details"
```

## Task 5: Full Verification

**Files:**
- No new files expected.

- [ ] **Step 1: Run Rust tests**

Run:

```bash
cargo test
```

Expected: all Rust tests pass.

- [ ] **Step 2: Run frontend tests**

Run:

```bash
cd frontend && pnpm test
```

Expected: all frontend tests pass.

- [ ] **Step 3: Run frontend lint**

Run:

```bash
cd frontend && pnpm lint
```

Expected: PASS. Fix only issues introduced by this change.

- [ ] **Step 4: Run frontend build**

Run:

```bash
cd frontend && pnpm build
```

Expected: PASS.

- [ ] **Step 5: Manual browser smoke test**

Run the browser trace flow:

```bash
cargo run -- --browser-trace
```

Open the emitted replay URL, select a model call node, and verify:

- The “输入 messages” section lists system/user/assistant/tool messages in order.
- Tool-result messages show their `tool_call_id`.
- The “模型输出” section shows assistant content, reasoning content if present, and tool calls.
- The raw “请求 JSON” and “响应 JSON” sections still render.
- Old `.sparrow-trace.json` archives without detailed messages show the compatibility labels instead of crashing.

- [ ] **Step 6: Commit verification fixes if any**

If verification required fixes, commit them:

```bash
git add src/agent.rs frontend/src/types/trace.ts frontend/src/components/ModelMessageList.tsx frontend/src/components/TraceDetailPanel.tsx frontend/src/components/TraceDetailPanel.test.tsx frontend/src/state/traceReducer.test.ts
git commit -m "fix: stabilize model call detail rendering"
```

## Notes And Risks

- `JsonSnapshot::from_value()` already truncates by `DEFAULT_SNAPSHOT_MAX_BYTES`. If a task has a very large conversation history, the frontend may show only the truncated JSON text. The structured message list is best-effort and should gracefully fall back to raw JSON.
- Existing redaction only removes values whose keys look secret-bearing, such as `api_key`, `token`, `authorization`, `password`, and `secret`. Message content itself is intentionally visible for this feature because the user asked to inspect exact model inputs.
- This plan does not add a new trace event type. Keeping the current `model_call.started/completed` contract avoids reducer churn and keeps old archive replay compatibility.
- Tool definitions are still summarized by `tool_count`. The requirement is detailed input messages and output, so full tool schema capture is intentionally out of scope.

## Self-Review

- Spec coverage: the plan covers model input messages, model output, UI display, old trace compatibility, and backend/frontend tests.
- Placeholder scan: no unfinished placeholder markers remain.
- Type consistency: `ModelMessageSnapshot`, `ModelRequestSnapshotValue`, and `ModelResponseSnapshotValue` match the JSON shape emitted by `model_request_snapshot()` and `model_response_snapshot()`.
