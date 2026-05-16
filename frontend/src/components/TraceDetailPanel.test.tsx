/**
 * @vitest-environment jsdom
 */
import '@testing-library/jest-dom/vitest'
import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { TraceDetailPanel } from './TraceDetailPanel'
import type { TraceNode } from '../types/trace'

const snapshot = (value: unknown) => ({
  value,
  text: JSON.stringify(value),
  truncated: false,
})

describe('TraceDetailPanel', () => {
  afterEach(() => {
    cleanup()
  })

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
