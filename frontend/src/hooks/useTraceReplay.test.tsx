/**
 * @vitest-environment jsdom
 */
import { renderHook, act } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { useTraceReplay } from './useTraceReplay'
import type { TraceEvent } from '../types/trace'

const events: TraceEvent[] = [
  {
    seq: 1,
    task_id: 'task_1',
    conversation_id: 'conv_1',
    timestamp: '2026-05-10T00:00:00.000Z',
    type: 'task.started',
    payload: { message: { role: 'user', content: 'hi' } },
  },
  {
    seq: 2,
    task_id: 'task_1',
    conversation_id: 'conv_1',
    timestamp: '2026-05-10T00:00:01.000Z',
    type: 'task.completed',
    payload: { duration_ms: 1000, final_answer: 'done' },
  },
]

const timedEvents: TraceEvent[] = [
  {
    ...events[0],
    seq: 3,
    timestamp: '2026-05-10T00:00:05.000Z',
  },
  {
    ...events[1],
    seq: 4,
    timestamp: '2026-05-10T00:00:07.500Z',
  },
]

describe('useTraceReplay', () => {
  it('steps through events manually', () => {
    const received: TraceEvent[] = []
    const { result } = renderHook(() => useTraceReplay({ events, onEvent: (event) => received.push(event) }))

    act(() => result.current.step())
    act(() => result.current.step())
    act(() => result.current.step())

    expect(received.map((event) => event.seq)).toEqual([1, 2])
    expect(result.current.currentIndex).toBe(2)
    expect(result.current.isComplete).toBe(true)
  })

  it('steps through multiple events in one manual jump', () => {
    const received: TraceEvent[] = []
    const { result } = renderHook(() => useTraceReplay({ events, onEvent: (event) => received.push(event) }))

    act(() => result.current.step(2))
    act(() => result.current.step(2))

    expect(received.map((event) => event.seq)).toEqual([1, 2])
    expect(result.current.currentIndex).toBe(2)
    expect(result.current.isComplete).toBe(true)
  })

  it('plays events using timestamp gaps between adjacent replay events', () => {
    vi.useFakeTimers()
    const received: TraceEvent[] = []
    const { result } = renderHook(() => useTraceReplay({ events: timedEvents, onEvent: (event) => received.push(event) }))

    act(() => result.current.play())
    act(() => vi.advanceTimersByTime(0))
    expect(received.map((event) => event.seq)).toEqual([3])

    act(() => vi.advanceTimersByTime(2499))
    expect(received.map((event) => event.seq)).toEqual([3])

    act(() => vi.advanceTimersByTime(1))

    expect(received.map((event) => event.seq)).toEqual([3, 4])
    expect(result.current.isPlaying).toBe(false)
    vi.useRealTimers()
  })
})
