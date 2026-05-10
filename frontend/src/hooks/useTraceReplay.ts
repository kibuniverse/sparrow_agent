import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { TraceEvent } from '../types/trace'

interface UseTraceReplayOptions {
  events: TraceEvent[]
  onEvent: (event: TraceEvent) => void
  intervalMs?: number
}

export function useTraceReplay({ events, onEvent, intervalMs = 600 }: UseTraceReplayOptions) {
  const sortedEvents = useMemo(
    () => events.slice().sort((left, right) => left.seq - right.seq),
    [events],
  )
  const [currentIndex, setCurrentIndex] = useState(0)
  const [isPlaying, setIsPlaying] = useState(false)
  const onEventRef = useRef(onEvent)

  useEffect(() => {
    onEventRef.current = onEvent
  }, [onEvent])

  const step = useCallback(() => {
    setCurrentIndex((index) => {
      const event = sortedEvents[index]
      if (!event) {
        return index
      }
      onEventRef.current(event)
      return index + 1
    })
  }, [sortedEvents])

  const restart = useCallback(() => {
    setCurrentIndex(0)
    setIsPlaying(false)
  }, [])

  useEffect(() => {
    if (!isPlaying) {
      return
    }
    if (currentIndex >= sortedEvents.length) {
      setIsPlaying(false)
      return
    }
    const timer = window.setTimeout(step, intervalMs)
    return () => window.clearTimeout(timer)
  }, [currentIndex, intervalMs, isPlaying, sortedEvents.length, step])

  return {
    currentIndex,
    total: sortedEvents.length,
    isComplete: currentIndex >= sortedEvents.length,
    isPlaying,
    play: () => setIsPlaying(true),
    pause: () => setIsPlaying(false),
    restart,
    step,
  }
}
