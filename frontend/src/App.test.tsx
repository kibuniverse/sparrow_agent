/**
 * @vitest-environment jsdom
 */
import '@testing-library/jest-dom/vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { createMemoryHistory, createRouter, RouterProvider } from '@tanstack/react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { routeTree } from './routeTree.gen'
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

  static lastUrl(): string | null {
    return FakeEventSource.instances.at(-1)?.url ?? null
  }
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
      child_task_id: 'task_sub_123',
      child_conversation_id: 'conv_sub_123',
    },
  },
]

const childEvents: TraceEvent[] = [
  {
    seq: 1,
    task_id: 'task_sub_123',
    conversation_id: 'conv_sub_123',
    timestamp: '2026-05-10T01:01:00.000Z',
    type: 'model_call.started',
    payload: {
      node_id: 'child_model_1',
      round: 1,
      model: 'deepseek-chat',
      request: snapshot({ messages: 1 }),
    },
  },
  {
    seq: 2,
    task_id: 'task_sub_123',
    conversation_id: 'conv_sub_123',
    timestamp: '2026-05-10T01:01:01.000Z',
    type: 'model_output.started',
    payload: {
      node_id: 'child_output_1',
      parent_model_call_id: 'child_model_1',
      kind: 'final_answer',
    },
  },
  {
    seq: 3,
    task_id: 'task_sub_123',
    conversation_id: 'conv_sub_123',
    timestamp: '2026-05-10T01:01:02.000Z',
    type: 'model_output.completed',
    payload: {
      node_id: 'child_output_1',
      kind: 'final_answer',
      content: 'child trace done',
      tool_calls: [],
    },
  },
  {
    seq: 4,
    task_id: 'task_sub_123',
    conversation_id: 'conv_sub_123',
    timestamp: '2026-05-10T01:01:03.000Z',
    type: 'task.completed',
    payload: { duration_ms: 3000, final_answer: 'child trace done' },
  },
]

const cliEvents: TraceEvent[] = [
  {
    seq: 1,
    task_id: 'task_cli_1',
    conversation_id: 'conv_cli_1',
    timestamp: '2026-05-10T02:00:00.000Z',
    type: 'task.started',
    payload: { message: { role: 'user', content: 'hello from cli' } },
  },
  {
    seq: 2,
    task_id: 'task_cli_1',
    conversation_id: 'conv_cli_1',
    timestamp: '2026-05-10T02:00:01.000Z',
    type: 'model_call.started',
    payload: {
      node_id: 'model_cli_1',
      round: 1,
      model: 'deepseek-chat',
      request: snapshot({ messages: 2 }),
    },
  },
]

const archive = {
  schema_version: 1 as const,
  exported_at: '2026-05-10T02:00:02.000Z',
  source: 'cli',
  task: {
    task_id: 'task_cli_1',
    conversation_id: 'conv_cli_1',
    status: 'succeeded' as const,
    created_at: '2026-05-10T02:00:00.000Z',
    updated_at: '2026-05-10T02:00:02.000Z',
    events: [
      ...cliEvents,
      {
        seq: 3,
        task_id: 'task_cli_1',
        conversation_id: 'conv_cli_1',
        timestamp: '2026-05-10T02:00:02.000Z',
        type: 'task.completed',
        payload: { duration_ms: 2000, final_answer: 'cli done' },
      },
    ],
  },
}

function renderApp(initialPath = '/') {
  const history = createMemoryHistory({
    initialEntries: [`/sparrow_agent${initialPath}`],
  })
  const router = createRouter({
    routeTree,
    history,
    basepath: '/sparrow_agent',
  })
  return render(<RouterProvider router={router} />)
}

describe('App trace visualization', () => {
  beforeEach(() => {
    FakeEventSource.instances = []
    vi.stubGlobal('EventSource', FakeEventSource)
    vi.stubGlobal('fetch', vi.fn(mockFetch))
  })

  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
  })

  it('submits a chat task, shows thinking preview, opens detail, and displays tool output', async () => {
    renderApp('/')

    fireEvent.change(screen.getByLabelText('消息内容'), { target: { value: '分析仓库' } })
    fireEvent.click(screen.getByRole('button', { name: '发送消息' }))

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1))

    for (const event of events) {
      FakeEventSource.instances[0].emitTrace(event)
    }

    expect(await screen.findByText('需要先读取仓库入口。')).toBeInTheDocument()
    expect(screen.getByText('准备调用 1 个工具：read_file')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: '查看详情' }))

    expect(await screen.findByRole('heading', { name: '任务详情' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '查看 工具调用 1：read_file' }))

    expect(screen.getByRole('heading', { name: '工具调用 1：read_file' })).toBeInTheDocument()
    expect(screen.getByText(/"bytes": 120/)).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: '打开子任务 task_sub_123' }))

    expect(await screen.findAllByText('child trace done')).not.toHaveLength(0)
  })

  it('streams a CLI-created task from a direct browser link', async () => {
    renderApp('/tasks/task_cli_1')

    expect(await screen.findByRole('heading', { name: '任务详情' })).toBeInTheDocument()
    await waitFor(() => expect(FakeEventSource.lastUrl()).toBe(
      '/api/agent/tasks/task_cli_1/events?after_seq=2',
    ))
  })

  it('opens a generated trace archive in preview mode', async () => {
    renderApp('/trace-files/task_cli_1.sparrow-trace.json')

    expect(await screen.findByRole('heading', { name: 'Trace 预览' })).toBeInTheDocument()
    expect(screen.getByText('task_cli_1.sparrow-trace.json')).toBeInTheDocument()
  })

  it('opens a generated trace archive in replay mode', async () => {
    renderApp('/replay/task_cli_1.sparrow-trace.json')

    expect(await screen.findByRole('heading', { name: 'Trace 回放' })).toBeInTheDocument()
    expect(await screen.findByRole('button', { name: '播放' })).toBeInTheDocument()
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

  if (url === '/api/agent/tasks/task_sub_123') {
    const response: TaskSnapshot = {
      task_id: 'task_sub_123',
      conversation_id: 'conv_sub_123',
      status: 'succeeded',
      created_at: '2026-05-10T01:01:00.000Z',
      updated_at: '2026-05-10T01:01:03.000Z',
      events: childEvents,
    }
    return jsonResponse(response, 200)
  }

  if (url === '/api/agent/tasks/task_cli_1') {
    const response: TaskSnapshot = {
      task_id: 'task_cli_1',
      conversation_id: 'conv_cli_1',
      status: 'running',
      created_at: '2026-05-10T02:00:00.000Z',
      updated_at: '2026-05-10T02:00:01.000Z',
      events: cliEvents,
    }
    return jsonResponse(response, 200)
  }

  if (url === '/api/agent/trace-files/task_cli_1.sparrow-trace.json') {
    return jsonResponse(archive, 200)
  }

  return jsonResponse({ error: { code: 'not_found', message: 'Not found', retryable: false } }, 404)
}

function jsonResponse(value: unknown, status: number): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}
