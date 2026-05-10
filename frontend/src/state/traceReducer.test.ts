import { describe, expect, it } from 'vitest'
import { applyTraceEvent, applyTraceSnapshot, createInitialTraceState } from './traceReducer'
import type { TraceEvent } from '../types/trace'

const jsonSnapshot = (value: unknown) => ({
  value,
  text: JSON.stringify(value),
  truncated: false,
})

describe('traceReducer', () => {
  it('builds a nested model output and tool call timeline', () => {
    let state = createInitialTraceState()
    const events: TraceEvent[] = [
      {
        seq: 1,
        task_id: 'task_1',
        conversation_id: 'conv_1',
        timestamp: '2026-05-10T01:00:00.000Z',
        type: 'task.started',
        payload: { message: { role: 'user', content: 'Inspect repo' } },
      },
      {
        seq: 2,
        task_id: 'task_1',
        conversation_id: 'conv_1',
        timestamp: '2026-05-10T01:00:01.000Z',
        type: 'model_call.started',
        payload: {
          node_id: 'model_1',
          round: 1,
          model: 'deepseek-chat',
          request: jsonSnapshot({ messages: 1 }),
        },
      },
      {
        seq: 3,
        task_id: 'task_1',
        conversation_id: 'conv_1',
        timestamp: '2026-05-10T01:00:02.000Z',
        type: 'model_call.reasoning_delta',
        payload: { node_id: 'model_1', delta: 'Need repository map.' },
      },
      {
        seq: 4,
        task_id: 'task_1',
        conversation_id: 'conv_1',
        timestamp: '2026-05-10T01:00:03.000Z',
        type: 'model_output.started',
        payload: {
          node_id: 'output_1',
          parent_model_call_id: 'model_1',
          kind: 'tool_calls',
        },
      },
      {
        seq: 5,
        task_id: 'task_1',
        conversation_id: 'conv_1',
        timestamp: '2026-05-10T01:00:04.000Z',
        type: 'model_output.delta',
        payload: {
          node_id: 'output_1',
          kind: 'tool_calls',
          tool_call: {
            index: 0,
            tool_call_id: 'call_1',
            name: 'read_file',
            arguments_delta: '{"path":"Cargo.toml"}',
          },
        },
      },
      {
        seq: 6,
        task_id: 'task_1',
        conversation_id: 'conv_1',
        timestamp: '2026-05-10T01:00:05.000Z',
        type: 'tool_call.started',
        payload: {
          node_id: 'tool_1',
          parent_model_output_id: 'output_1',
          index: 0,
          tool_call_id: 'call_1',
          name: 'read_file',
          arguments: jsonSnapshot({ path: 'Cargo.toml' }),
        },
      },
      {
        seq: 7,
        task_id: 'task_1',
        conversation_id: 'conv_1',
        timestamp: '2026-05-10T01:00:06.000Z',
        type: 'tool_call.completed',
        payload: {
          node_id: 'tool_1',
          duration_ms: 42,
          output: jsonSnapshot({ bytes: 120 }),
        },
      },
    ]

    for (const event of events) {
      state = applyTraceEvent(state, event)
    }

    expect(state.status).toBe('running')
    expect(state.lastSeq).toBe(7)
    expect(state.rootNodeIds).toEqual(['model_1'])
    expect(state.nodesById.model_1.childrenIds).toEqual(['output_1'])
    expect(state.nodesById.output_1.childrenIds).toEqual(['tool_1'])
    expect(state.nodesById.model_1.detail.type).toBe('model_call')
    expect(state.latestReasoningText).toBe('Need repository map.')
    expect(state.nodesById.output_1.detail).toMatchObject({
      type: 'model_output',
      kind: 'tool_calls',
      toolCalls: [{ nodeId: 'tool_1', toolCallId: 'call_1', name: 'read_file' }],
    })
    expect(state.nodesById.tool_1).toMatchObject({
      status: 'succeeded',
      durationMs: 42,
      title: '工具调用 1：read_file',
    })
  })

  it('records final answers and selects the final output when a task completes', () => {
    const state = applyTraceSnapshot(createInitialTraceState(), {
      task_id: 'task_2',
      conversation_id: 'conv_2',
      status: 'succeeded',
      created_at: '2026-05-10T01:00:00.000Z',
      updated_at: '2026-05-10T01:00:08.000Z',
      events: [
        {
          seq: 1,
          task_id: 'task_2',
          conversation_id: 'conv_2',
          timestamp: '2026-05-10T01:00:01.000Z',
          type: 'model_call.started',
          payload: {
            node_id: 'model_2',
            round: 2,
            model: 'deepseek-chat',
            request: jsonSnapshot({}),
          },
        },
        {
          seq: 2,
          task_id: 'task_2',
          conversation_id: 'conv_2',
          timestamp: '2026-05-10T01:00:02.000Z',
          type: 'model_output.started',
          payload: {
            node_id: 'output_2',
            parent_model_call_id: 'model_2',
            kind: 'final_answer',
          },
        },
        {
          seq: 3,
          task_id: 'task_2',
          conversation_id: 'conv_2',
          timestamp: '2026-05-10T01:00:03.000Z',
          type: 'model_output.delta',
          payload: {
            node_id: 'output_2',
            kind: 'final_answer',
            content_delta: 'Final ',
          },
        },
        {
          seq: 4,
          task_id: 'task_2',
          conversation_id: 'conv_2',
          timestamp: '2026-05-10T01:00:04.000Z',
          type: 'model_output.completed',
          payload: {
            node_id: 'output_2',
            kind: 'final_answer',
            content: 'Final answer',
            tool_calls: [],
          },
        },
        {
          seq: 5,
          task_id: 'task_2',
          conversation_id: 'conv_2',
          timestamp: '2026-05-10T01:00:05.000Z',
          type: 'task.completed',
          payload: { duration_ms: 5000, final_answer: 'Final answer' },
        },
      ],
    })

    expect(state.status).toBe('succeeded')
    expect(state.finalAnswer).toBe('Final answer')
    expect(state.selectedNodeId).toBe('output_2')
    expect(state.nodesById.output_2.subtitle).toBe('Final answer')
  })

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

  it('merges assistant content into the tool call output for the same model call', () => {
    const state = applyTraceSnapshot(createInitialTraceState(), {
      task_id: 'task_mixed_output',
      conversation_id: 'conv_mixed_output',
      status: 'running',
      created_at: '2026-05-10T03:00:00.000Z',
      updated_at: '2026-05-10T03:00:05.000Z',
      events: [
        {
          seq: 1,
          task_id: 'task_mixed_output',
          conversation_id: 'conv_mixed_output',
          timestamp: '2026-05-10T03:00:01.000Z',
          type: 'model_call.started',
          payload: {
            node_id: 'model_mixed',
            round: 1,
            model: 'deepseek-chat',
            request: jsonSnapshot({}),
          },
        },
        {
          seq: 2,
          task_id: 'task_mixed_output',
          conversation_id: 'conv_mixed_output',
          timestamp: '2026-05-10T03:00:02.000Z',
          type: 'model_output.started',
          payload: {
            node_id: 'output_content',
            parent_model_call_id: 'model_mixed',
            kind: 'final_answer',
          },
        },
        {
          seq: 3,
          task_id: 'task_mixed_output',
          conversation_id: 'conv_mixed_output',
          timestamp: '2026-05-10T03:00:03.000Z',
          type: 'model_output.delta',
          payload: {
            node_id: 'output_content',
            kind: 'final_answer',
            content_delta: '我先读取仓库入口。',
          },
        },
        {
          seq: 4,
          task_id: 'task_mixed_output',
          conversation_id: 'conv_mixed_output',
          timestamp: '2026-05-10T03:00:04.000Z',
          type: 'model_output.started',
          payload: {
            node_id: 'output_tool',
            parent_model_call_id: 'model_mixed',
            kind: 'tool_calls',
          },
        },
        {
          seq: 5,
          task_id: 'task_mixed_output',
          conversation_id: 'conv_mixed_output',
          timestamp: '2026-05-10T03:00:05.000Z',
          type: 'model_output.delta',
          payload: {
            node_id: 'output_tool',
            kind: 'tool_calls',
            tool_call: {
              index: 0,
              tool_call_id: 'call_1',
              name: 'read_file',
              arguments_delta: '{"path":"Cargo.toml"}',
            },
          },
        },
      ],
    })

    expect(state.nodesById.model_mixed.childrenIds).toEqual(['output_tool'])
    expect(state.nodesById.output_content).toBeUndefined()
    expect(state.nodesById.output_tool).toMatchObject({
      title: '工具调用',
      subtitle: '我先读取仓库入口。',
      detail: {
        type: 'model_output',
        kind: 'tool_calls',
        content: '我先读取仓库入口。',
        toolCalls: [{ name: 'read_file' }],
      },
    })
  })

  it('ignores events whose sequence has already been applied', () => {
    const startEvent: TraceEvent = {
      seq: 1,
      task_id: 'task_3',
      conversation_id: 'conv_3',
      timestamp: '2026-05-10T01:00:01.000Z',
      type: 'model_call.started',
      payload: {
        node_id: 'model_3',
        round: 1,
        model: 'deepseek-chat',
        request: jsonSnapshot({}),
      },
    }

    const state = applyTraceEvent(applyTraceEvent(createInitialTraceState(), startEvent), {
      ...startEvent,
      payload: { ...startEvent.payload, node_id: 'model_duplicate' },
    })

    expect(state.lastSeq).toBe(1)
    expect(state.rootNodeIds).toEqual(['model_3'])
    expect(state.nodesById.model_duplicate).toBeUndefined()
  })
})
