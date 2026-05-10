# Frontend Loop Tool Parallel Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix loop trace rendering so final model output nodes do not stay stuck in `running`, and traced tool calls execute/display as parallel siblings instead of serialized work.

**Architecture:** Keep the trace event contract unchanged. Add a frontend reducer fallback that reconciles any running final-answer `model_output` when `task.completed` arrives, and change the Rust tool registry to emit all traced tool start events before awaiting tool futures concurrently. Preserve result order when appending tool messages back to the conversation.

**Tech Stack:** Rust 2024, Tokio, `futures-util::future::join_all`, React 19, TypeScript, Vitest, Cargo tests.

---

## Root Cause Summary

- `frontend/src/state/traceReducer.ts` marks `model_output` nodes as succeeded only when it receives `model_output.completed`. If the stream reaches `task.completed` while a final-answer output node is still `running`, the reducer clears only `latestRunningNodeId`; the row still renders `LoadingInline label="运行中"`.
- `src/tool_registry.rs` executes traced tools in a `for` loop and awaits each `self.execute(tool_call)` before starting the next tool. This causes `tool_call.started` and `tool_call.completed` events to appear as start/finish pairs instead of all sibling tool calls being visible as running together.

## File Structure

- Modify `frontend/src/state/traceReducer.test.ts`: add a regression test for `task.completed` closing a running final-answer output when no explicit `model_output.completed` event has arrived.
- Modify `frontend/src/state/traceReducer.ts`: add a small helper that finalizes running final-answer output nodes from `task.completed.final_answer`.
- Modify `src/tool_registry.rs`: make `execute_all` and `execute_all_traced` concurrent with `join_all`; for traced execution, emit all `tool_call.started` events before awaiting tool results.
- Add/extend tests in `src/tool_registry.rs`: prove traced execution starts all tools before any completion and that multiple calls can enter provider execution concurrently.
- Keep this plan in `docs/superpowers/plans/2026-05-10-frontend-loop-tool-parallel-fix.md`.

## Implementation Tasks

### Task 1: Frontend Final Output Completion Fallback

**Files:**
- Modify: `frontend/src/state/traceReducer.test.ts`
- Modify: `frontend/src/state/traceReducer.ts`

- [ ] **Step 1: Write the failing reducer test**

Add a Vitest case to `frontend/src/state/traceReducer.test.ts`:

```ts
it('marks a running final answer output as succeeded when the task completes', () => {
  const state = applyTraceSnapshot(createInitialTraceState(), {
    task_id: 'task_final',
    conversation_id: 'conv_final',
    status: 'succeeded',
    created_at: '2026-05-10T02:00:00.000Z',
    updated_at: '2026-05-10T02:00:04.000Z',
    events: [
      {
        seq: 1,
        task_id: 'task_final',
        conversation_id: 'conv_final',
        timestamp: '2026-05-10T02:00:01.000Z',
        type: 'model_call.started',
        payload: {
          node_id: 'model_final',
          round: 1,
          model: 'deepseek-chat',
          request: jsonSnapshot({}),
        },
      },
      {
        seq: 2,
        task_id: 'task_final',
        conversation_id: 'conv_final',
        timestamp: '2026-05-10T02:00:02.000Z',
        type: 'model_output.started',
        payload: {
          node_id: 'output_final',
          parent_model_call_id: 'model_final',
          kind: 'final_answer',
        },
      },
      {
        seq: 3,
        task_id: 'task_final',
        conversation_id: 'conv_final',
        timestamp: '2026-05-10T02:00:03.000Z',
        type: 'model_output.delta',
        payload: {
          node_id: 'output_final',
          kind: 'final_answer',
          content_delta: 'Final answer',
        },
      },
      {
        seq: 4,
        task_id: 'task_final',
        conversation_id: 'conv_final',
        timestamp: '2026-05-10T02:00:04.000Z',
        type: 'task.completed',
        payload: { duration_ms: 3000, final_answer: 'Final answer' },
      },
    ],
  })

  expect(state.nodesById.output_final.status).toBe('succeeded')
  expect(state.nodesById.output_final.completedAt).toBe('2026-05-10T02:00:04.000Z')
  expect(state.nodesById.output_final.subtitle).toBe('Final answer')
  expect(state.nodesById.output_final.detail).toMatchObject({
    type: 'model_output',
    kind: 'final_answer',
    content: 'Final answer',
  })
})
```

- [ ] **Step 2: Run the targeted frontend test and verify it fails**

Run:

```bash
cd frontend && pnpm test src/state/traceReducer.test.ts
```

Expected before implementation: the new test fails because `output_final.status` is still `running`.

- [ ] **Step 3: Implement minimal reducer fallback**

In `frontend/src/state/traceReducer.ts`, update the `task.completed` case to pass the terminal timestamp and final answer through a helper:

```ts
case 'task.completed': {
  const completedState: TraceState = {
    ...base,
    status: 'succeeded',
    finalAnswer: event.payload.final_answer,
    error: null,
    durationMs: event.payload.duration_ms,
    latestRunningNodeId: null,
    selectedNodeId: selectFinalOutputNode(base) ?? base.selectedNodeId,
  }

  return completeRunningFinalOutput(
    completedState,
    event.payload.final_answer,
    event.timestamp,
  )
}
```

Add the helper near the other reducer helpers:

```ts
function completeRunningFinalOutput(
  state: TraceState,
  finalAnswer: string,
  completedAt: string,
): TraceState {
  const entry = Object.entries(state.nodesById).find(([, node]) => {
    return (
      node.status === 'running' &&
      node.detail.type === 'model_output' &&
      node.detail.kind === 'final_answer'
    )
  })

  if (!entry) {
    return state
  }

  const [nodeId, node] = entry
  return {
    ...state,
    selectedNodeId: state.selectedNodeId ?? nodeId,
    nodesById: {
      ...state.nodesById,
      [nodeId]: {
        ...node,
        status: 'succeeded',
        completedAt,
        subtitle: truncate(finalAnswer, 180),
        detail:
          node.detail.type === 'model_output'
            ? { ...node.detail, content: finalAnswer }
            : node.detail,
      },
    },
  }
}
```

- [ ] **Step 4: Run targeted frontend test and verify it passes**

Run:

```bash
cd frontend && pnpm test src/state/traceReducer.test.ts
```

Expected after implementation: all tests in `traceReducer.test.ts` pass.

### Task 2: Parallel Traced Tool Execution

**Files:**
- Modify: `src/tool_registry.rs`

- [ ] **Step 1: Write failing Rust tests**

Add tests to `src/tool_registry.rs`:

```rust
struct BarrierProvider {
    definitions: Vec<ToolDef>,
    barrier: Arc<tokio::sync::Barrier>,
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

#[tokio::test]
async fn traced_execution_starts_all_tools_before_any_completion() {
    let mut registry = ToolRegistry::new();
    registry.add_provider(Box::new(BarrierProvider {
        definitions: vec![ToolDef::function("knownTool", "Known tool")],
        barrier: Arc::new(tokio::sync::Barrier::new(2)),
    }));
    let sink = RecordingSink::default();

    let results = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        registry.execute_all_traced(
            &[tool_call("call_1", "knownTool"), tool_call("call_2", "knownTool")],
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
    assert!(events[2..].iter().any(|event| event.0 == TraceEventType::ToolCallCompleted));
    assert!(events[2..].iter().all(|event| event.0 != TraceEventType::ToolCallStarted));
}
```

- [ ] **Step 2: Run the targeted Rust test and verify it fails**

Run:

```bash
cargo test tool_registry::tests::traced_execution_starts_all_tools_before_any_completion
```

Expected before implementation: timeout failure because the first sequential tool waits at the barrier before the second tool starts.

- [ ] **Step 3: Implement concurrent execution**

Import `join_all`:

```rust
use futures_util::future::join_all;
```

Change `execute_all` to map each call into an async future and `join_all` them:

```rust
pub async fn execute_all(&self, tool_calls: &[ToolCall]) -> Vec<ToolExecutionResult> {
    join_all(tool_calls.iter().map(|tool_call| async move {
        debug_log!(
            "Executing tool: name={}, id={}, args={}",
            tool_call.function.name,
            tool_call.id,
            tool_call.function.arguments,
        );
        let content = match self.execute(tool_call).await {
            Ok(content) => content,
            Err(error) => {
                debug_log!("Tool '{}' failed: {error}", tool_call.function.name);
                format!("Tool execution failed: {error}")
            }
        };

        ToolExecutionResult {
            tool_call_id: tool_call.id.clone(),
            content,
        }
    }))
    .await
}
```

Change `execute_all_traced` so it emits all start events first, then awaits all executions concurrently:

```rust
let started_calls = tool_calls
    .iter()
    .enumerate()
    .map(|(index, tool_call)| {
        let node_id = trace_id("tool");
        let started = Instant::now();
        sink.emit(TraceEventType::ToolCallStarted, json!({ /* existing payload */ }));
        (node_id, started, tool_call)
    })
    .collect::<Vec<_>>();

join_all(started_calls.into_iter().map(|(node_id, started, tool_call)| async move {
    let content = match self.execute(tool_call).await {
        Ok(content) => {
            sink.emit(TraceEventType::ToolCallCompleted, json!({ /* existing payload */ }));
            content
        }
        Err(error) => {
            sink.emit(TraceEventType::ToolCallFailed, json!({ /* existing payload */ }));
            format!("Tool execution failed: {error}")
        }
    };

    ToolExecutionResult {
        tool_call_id: tool_call.id.clone(),
        content,
    }
}))
.await
```

- [ ] **Step 4: Run targeted Rust tests and verify they pass**

Run:

```bash
cargo test tool_registry::tests
```

Expected after implementation: all `tool_registry` tests pass, including existing single-tool success/failure tests and the new concurrency test.

### Task 3: Full Verification

**Files:**
- Verify only; no expected source edits.

- [ ] **Step 1: Run frontend unit tests**

Run:

```bash
cd frontend && pnpm test
```

Expected: all Vitest suites pass.

- [ ] **Step 2: Run frontend build**

Run:

```bash
cd frontend && pnpm build
```

Expected: TypeScript build and Vite build complete without errors.

- [ ] **Step 3: Run Rust tests**

Run:

```bash
cargo test
```

Expected: all Rust unit and integration tests pass.

- [ ] **Step 4: Review git diff**

Run:

```bash
git diff -- frontend/src/state/traceReducer.ts frontend/src/state/traceReducer.test.ts src/tool_registry.rs docs/superpowers/plans/2026-05-10-frontend-loop-tool-parallel-fix.md
```

Expected: diff is limited to the documented reducer fallback, tool parallelization, tests, and this plan.

## Design Self-Review

- Spec coverage: Covers both reported frontend symptoms: final-answer output no longer stays `running`, and traced tool calls start together and execute concurrently.
- Placeholder scan: No `TBD`, `TODO`, or deferred behavior remains.
- Type consistency: Uses existing `TraceState`, `TraceNode`, `TraceEvent`, `ToolCall`, `ToolExecutionResult`, and `TraceSink` types.
- Scope: No API schema change, no UI layout redesign, and no unrelated refactor.
