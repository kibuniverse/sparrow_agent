import { useEffect, useRef, useState } from 'react'
import { buildTaskEventsUrl } from '../api/agentTrace'
import type { TraceEvent } from '../types/trace'

const reconnectDelays = [1000, 2000, 5000, 10000]

export interface UseTaskStreamOptions {
  taskId: string | null
  enabled: boolean
  lastSeq: number
  onEvent: (event: TraceEvent) => void
  onError?: (message: string) => void
}

export interface UseTaskStreamState {
  isConnected: boolean
  streamError: string | null
}

export function useTaskStream(options: UseTaskStreamOptions): UseTaskStreamState {
  const { taskId, enabled, lastSeq, onEvent, onError } = options
  const latestSeqRef = useRef(lastSeq)
  const onEventRef = useRef(onEvent)
  const onErrorRef = useRef(onError)
  const [isConnected, setIsConnected] = useState(false)
  const [streamError, setStreamError] = useState<string | null>(null)

  useEffect(() => {
    latestSeqRef.current = Math.max(latestSeqRef.current, lastSeq)
  }, [lastSeq])

  useEffect(() => {
    onEventRef.current = onEvent
  }, [onEvent])

  useEffect(() => {
    onErrorRef.current = onError
  }, [onError])

  useEffect(() => {
    if (!enabled || !taskId) {
      return
    }

    const streamTaskId = taskId
    let disposed = false
    let source: EventSource | null = null
    let reconnectTimer: ReturnType<typeof window.setTimeout> | null = null
    let reconnectAttempt = 0

    const clearReconnectTimer = () => {
      if (reconnectTimer !== null) {
        window.clearTimeout(reconnectTimer)
        reconnectTimer = null
      }
    }

    const closeSource = () => {
      if (source) {
        source.close()
        source = null
      }
      setIsConnected(false)
    }

    const scheduleReconnect = () => {
      const delay = reconnectDelays[Math.min(reconnectAttempt, reconnectDelays.length - 1)]
      reconnectAttempt += 1
      reconnectTimer = window.setTimeout(connect, delay)
    }

    const handleTrace = (message: MessageEvent<string>) => {
      try {
        const event = JSON.parse(message.data) as TraceEvent
        latestSeqRef.current = Math.max(latestSeqRef.current, event.seq)
        setStreamError(null)
        onEventRef.current(event)
      } catch {
        const messageText = '无法解析 Agent trace 事件。'
        setStreamError(messageText)
        onErrorRef.current?.(messageText)
      }
    }

    const handleError = () => {
      if (disposed) {
        return
      }

      const messageText = 'Agent 事件流已断开，正在重连。'
      setStreamError(messageText)
      onErrorRef.current?.(messageText)
      closeSource()
      clearReconnectTimer()
      scheduleReconnect()
    }

    function connect() {
      if (disposed) {
        return
      }

      closeSource()
      source = new EventSource(buildTaskEventsUrl(streamTaskId, latestSeqRef.current))
      source.addEventListener('trace', handleTrace)
      source.onerror = handleError
      setIsConnected(true)
    }

    connect()

    return () => {
      disposed = true
      clearReconnectTimer()
      closeSource()
    }
  }, [enabled, taskId])

  return { isConnected, streamError }
}
