import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { TraceEvent } from '../types/trace'

interface UseTraceReplayOptions {
  events: TraceEvent[]
  onEvent: (event: TraceEvent) => void
}

function timestampGapMs(previous: TraceEvent | undefined, next: TraceEvent | undefined) {
  if (!previous || !next) {
    return 0
  }

  const previousTime = Date.parse(previous.timestamp)
  const nextTime = Date.parse(next.timestamp)
  if (!Number.isFinite(previousTime) || !Number.isFinite(nextTime)) {
    return 0
  }

  return Math.max(0, nextTime - previousTime)
}

export function useTraceReplay({ events, onEvent }: UseTraceReplayOptions) {
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

  const step = useCallback((count = 1) => {
    const stepCount = Math.max(0, Math.floor(count))
    if (stepCount === 0) {
      return
    }

    const nextIndex = Math.min(currentIndex + stepCount, sortedEvents.length)
    for (let eventIndex = currentIndex; eventIndex < nextIndex; eventIndex += 1) {
      onEventRef.current(sortedEvents[eventIndex])
    }
    setCurrentIndex(nextIndex)
    if (nextIndex >= sortedEvents.length) {
      setIsPlaying(false)
    }
  }, [currentIndex, sortedEvents])

  const restart = useCallback(() => {
    setCurrentIndex(0)
    setIsPlaying(false)
  }, [])

  useEffect(() => {
    if (!isPlaying) {
      return
    }
    if (currentIndex >= sortedEvents.length) {
      return
    }
    const delayMs = timestampGapMs(sortedEvents[currentIndex - 1], sortedEvents[currentIndex])
    const timer = window.setTimeout(() => step(), delayMs)
    return () => window.clearTimeout(timer)
  }, [currentIndex, isPlaying, sortedEvents, step])

  return {
    currentIndex,
    total: sortedEvents.length,
    isComplete: currentIndex >= sortedEvents.length,
    isPlaying,
    play: () => setIsPlaying(currentIndex < sortedEvents.length),
    pause: () => setIsPlaying(false),
    restart,
    step,
  }
}
