/**
 * @vitest-environment jsdom
 */
import '@testing-library/jest-dom/vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import type { CreateAgentTaskResponse, TaskSnapshot, TraceEvent } from './types/trace'

class FakeEventSource {
  static instances: FakeEventSource[] = []

  url: string
  closed = false
  private listeners: Record<string, Array<(event: MessageEvent<string>) => void>> = {}

  constructor(url: string) {
    this.url = url
    FakeEventSource.instances.push(this)
  }

  addEventListener(type: string, listener: (event: MessageEvent<string>) => void) {
    this.listeners[type] = [...(this.listeners[type] ?? []), listener]
  }

  close() {
    this.closed = true
  }

  emitTrace(event: TraceEvent) {
    for (const listener of this.listeners.trace ?? []) {
      listener(new MessageEvent('trace', { data: JSON.stringify(event) }))
    }
  }

  onerror: ((event: Event) => void) | null = null
}

const snapshot = (value: unknown) => ({
  value,
  text: JSON.stringify(value),
  truncated: false,
})

const events: TraceEvent[] = [
  {
    seq: 1,
    task_id: 'task_stream',
    conversation_id: 'conv_stream',
    timestamp: '2026-05-10T01:00:00.000Z',
    type: 'task.started',
    payload: { message: { role: 'user', content: '分析仓库' } },
  },
  {
    seq: 2,
    task_id: 'task_stream',
    conversation_id: 'conv_stream',
    timestamp: '2026-05-10T01:00:01.000Z',
    type: 'model_call.started',
    payload: {
      node_id: 'model_1',
      round: 1,
      model: 'deepseek-chat',
      request: snapshot({ messages: 1 }),
    },
  },
  {
    seq: 3,
    task_id: 'task_stream',
    conversation_id: 'conv_stream',
    timestamp: '2026-05-10T01:00:02.000Z',
    type: 'model_call.reasoning_delta',
    payload: { node_id: 'model_1', delta: '需要先读取仓库入口。' },
  },
  {
    seq: 4,
    task_id: 'task_stream',
    conversation_id: 'conv_stream',
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
    task_id: 'task_stream',
    conversation_id: 'conv_stream',
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
    task_id: 'task_stream',
    conversation_id: 'conv_stream',
    timestamp: '2026-05-10T01:00:05.000Z',
    type: 'tool_call.started',
    payload: {
      node_id: 'tool_1',
      parent_model_output_id: 'output_1',
      index: 0,
      tool_call_id: 'call_1',
      name: 'read_file',
      arguments: snapshot({ path: 'Cargo.toml' }),
    },
  },
  {
    seq: 7,
    task_id: 'task_stream',
    conversation_id: 'conv_stream',
    timestamp: '2026-05-10T01:00:06.000Z',
    type: 'tool_call.completed',
    payload: {
      node_id: 'tool_1',
      duration_ms: 42,
      output: snapshot({ bytes: 120 }),
    },
  },
]

describe('App trace visualization', () => {
  beforeEach(() => {
    FakeEventSource.instances = []
    window.history.replaceState(null, '', '/')
    vi.stubGlobal('EventSource', FakeEventSource)
    vi.stubGlobal('fetch', vi.fn(mockFetch))
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('submits a chat task, shows thinking preview, opens detail, and displays tool output', async () => {
    render(<App />)

    fireEvent.change(screen.getByLabelText('消息内容'), { target: { value: '分析仓库' } })
    fireEvent.click(screen.getByRole('button', { name: '发送消息' }))

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1))

    for (const event of events) {
      FakeEventSource.instances[0].emitTrace(event)
    }

    expect(await screen.findByText('需要先读取仓库入口。')).toBeInTheDocument()
    expect(screen.getByText('准备调用 1 个工具：read_file')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: '查看任务详情' }))

    expect(await screen.findByRole('heading', { name: '任务详情' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '查看 工具调用 1：read_file' }))

    expect(screen.getByRole('heading', { name: '工具调用 1：read_file' })).toBeInTheDocument()
    expect(screen.getByText(/"bytes": 120/)).toBeInTheDocument()
  })
})

async function mockFetch(input: string | URL | Request, init?: RequestInit): Promise<Response> {
  const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url

  if (url === '/api/agent/tasks' && init?.method === 'POST') {
    const response: CreateAgentTaskResponse = {
      task_id: 'task_stream',
      conversation_id: 'conv_stream',
      events_url: '/api/agent/tasks/task_stream/events',
      snapshot_url: '/api/agent/tasks/task_stream',
    }
    return jsonResponse(response, 202)
  }

  if (url === '/api/agent/tasks/task_stream') {
    const response: TaskSnapshot = {
      task_id: 'task_stream',
      conversation_id: 'conv_stream',
      status: 'running',
      created_at: '2026-05-10T01:00:00.000Z',
      updated_at: '2026-05-10T01:00:06.000Z',
      events,
    }
    return jsonResponse(response, 200)
  }

  return jsonResponse({ error: { code: 'not_found', message: 'Not found', retryable: false } }, 404)
}

function jsonResponse(value: unknown, status: number): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}
