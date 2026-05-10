/**
 * @vitest-environment jsdom
 */
import { renderHook, act } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useTaskStream } from './useTaskStream'
import type { TraceEvent } from '../types/trace'

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

  emitError() {
    this.onerror?.(new Event('error'))
  }

  onerror: ((event: Event) => void) | null = null
}

const modelStarted = (seq: number): TraceEvent => ({
  seq,
  task_id: 'task_stream',
  conversation_id: 'conv_stream',
  timestamp: `2026-05-10T01:00:0${seq}.000Z`,
  type: 'model_call.started',
  payload: {
    node_id: `model_${seq}`,
    round: seq,
    model: 'deepseek-chat',
    request: { value: {}, text: '{}', truncated: false },
  },
})

describe('useTaskStream', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    FakeEventSource.instances = []
    vi.stubGlobal('EventSource', FakeEventSource)
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    vi.useRealTimers()
  })

  it('reconnects from the latest sequence after repeated errors', async () => {
    const onEvent = vi.fn()

    renderHook(() =>
      useTaskStream({
        taskId: 'task_stream',
        enabled: true,
        lastSeq: 0,
        onEvent,
      }),
    )

    expect(FakeEventSource.instances[0].url).toBe(
      '/api/agent/tasks/task_stream/events?after_seq=0',
    )

    act(() => {
      FakeEventSource.instances[0].emitTrace(modelStarted(1))
      FakeEventSource.instances[0].emitError()
    })

    expect(FakeEventSource.instances[0].closed).toBe(true)

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000)
    })

    expect(FakeEventSource.instances[1].url).toBe(
      '/api/agent/tasks/task_stream/events?after_seq=1',
    )

    act(() => {
      FakeEventSource.instances[1].emitTrace(modelStarted(2))
      FakeEventSource.instances[1].emitError()
    })

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000)
    })

    expect(FakeEventSource.instances[2].url).toBe(
      '/api/agent/tasks/task_stream/events?after_seq=2',
    )
    expect(onEvent).toHaveBeenCalledTimes(2)
  })

  it('closes the active connection on unmount', () => {
    const { unmount } = renderHook(() =>
      useTaskStream({
        taskId: 'task_stream',
        enabled: true,
        lastSeq: 0,
        onEvent: vi.fn(),
      }),
    )

    unmount()

    expect(FakeEventSource.instances[0].closed).toBe(true)
  })
})
