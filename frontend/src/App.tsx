
import { useCallback, useRef, useState } from 'react'
import { createAgentTask } from './api/agentTrace'
import { useTaskStream } from './hooks/useTaskStream'
import { ChatPage, type ChatMessage } from './pages/ChatPage'
import { TaskDetailPage } from './pages/TaskDetailPage'
import { TraceArchivePage } from './pages/TraceArchivePage'
import { TraceReplayPage } from './pages/TraceReplayPage'
import { navigateTo, useRoute } from './router'
import {
  applyTraceEvent,
  applyTraceSnapshot,
  createInitialTraceState,
  type TraceState,
} from './state/traceReducer'
import type { TaskSnapshot, TraceArchive, TraceEvent } from './types/trace'

function App() {
  const route = useRoute()
  const completedTaskIds = useRef(new Set<string>())
  const [traceState, setTraceState] = useState<TraceState>(() => createInitialTraceState())
  const [conversationId, setConversationId] = useState<string | null>(null)
  const [messages, setMessages] = useState<ChatMessage[]>([])

  const selectNode = useCallback((nodeId: string) => {
    setTraceState((current) => ({
      ...current,
      selectedNodeId: current.nodesById[nodeId] ? nodeId : current.selectedNodeId,
    }))
  }, [])

  const handleTraceEvent = useCallback((event: TraceEvent) => {
    setTraceState((current) => applyTraceEvent(current, event))
    if (event.type === 'task.completed' && !completedTaskIds.current.has(event.task_id)) {
      completedTaskIds.current.add(event.task_id)
      setMessages((current) => [
        ...current,
        { id: `${event.task_id}-assistant`, role: 'assistant', content: event.payload.final_answer },
      ])
    }
  }, [])

  const applySnapshot = useCallback((snapshot: TaskSnapshot) => {
    setTraceState((current) => applyTraceSnapshot(current, snapshot))
    setConversationId(snapshot.conversation_id)
  }, [])

  const applyArchive = useCallback((archive: TraceArchive) => {
    setTraceState(applyTraceSnapshot(createInitialTraceState(), archive.task))
    setConversationId(archive.task.conversation_id)
  }, [])

  const activeTaskId = route.name === 'task' ? route.taskId : traceState.taskId

  useTaskStream({
    taskId: activeTaskId,
    enabled: traceState.status === 'running' && Boolean(activeTaskId),
    lastSeq: traceState.lastSeq,
    onEvent: handleTraceEvent,
  })

  const submitMessage = useCallback(
    async (message: string) => {
      const clientMessageId = createClientId('msg')
      setMessages((current) => [...current, { id: clientMessageId, role: 'user', content: message }])
      const response = await createAgentTask({
        conversation_id: conversationId,
        client_message_id: clientMessageId,
        message,
        stream: true,
      })
      setConversationId(response.conversation_id)
      setTraceState({
        ...createInitialTraceState(),
        taskId: response.task_id,
        conversationId: response.conversation_id,
        status: 'running',
      })
    },
    [conversationId],
  )

  if (route.name === 'trace-file') {
    return (
      <TraceArchivePage
        fileName={route.fileName}
        onApplyArchive={applyArchive}
        onBack={() => navigateTo('/')}
        onReplay={(fileName) => navigateTo(`/replay/${encodeURIComponent(fileName)}`)}
        onSelectNode={selectNode}
        state={traceState}
      />
    )
  }

  if (route.name === 'replay') {
    return <TraceReplayPage fileName={route.fileName} onBack={() => navigateTo('/')} />
  }

  if (route.name === 'task') {
    return (
      <TaskDetailPage
        onApplySnapshot={applySnapshot}
        onBack={() => navigateTo('/')}
        onSelectNode={selectNode}
        state={traceState}
        taskId={route.taskId}
      />
    )
  }

  return (
    <ChatPage
      messages={messages}
      onOpenTask={(taskId) => navigateTo(`/tasks/${encodeURIComponent(taskId)}`)}
      onSubmitMessage={submitMessage}
      traceState={traceState}
    />
  )
}

function createClientId(prefix: string): string {
  if ('randomUUID' in crypto) {
    return `${prefix}_${crypto.randomUUID()}`
  }
  return `${prefix}_${Date.now()}_${Math.random().toString(16).slice(2)}`
}

export default App
