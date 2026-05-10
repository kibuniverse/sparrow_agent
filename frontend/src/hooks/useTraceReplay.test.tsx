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

  it('plays events with fake timers', () => {
    vi.useFakeTimers()
    const received: TraceEvent[] = []
    const { result } = renderHook(() => useTraceReplay({ events, onEvent: (event) => received.push(event), intervalMs: 100 }))

    act(() => result.current.play())
    act(() => vi.advanceTimersByTime(100))
    act(() => vi.advanceTimersByTime(100))

    expect(received.map((event) => event.seq)).toEqual([1, 2])
    expect(result.current.isPlaying).toBe(false)
    vi.useRealTimers()
  })
})
