import { create } from 'zustand'
import { createAgentTask } from './api/agentTrace'
import {
  applyTraceEvent,
  applyTraceSnapshot,
  createInitialTraceState,
  type TraceState,
} from './state/traceReducer'
import type { TaskSnapshot, TraceArchive, TraceEvent } from './types/trace'

export interface ChatMessage {
  id: string
  role: 'user' | 'assistant'
  content: string
}

interface AppStore {
  traceState: TraceState
  conversationId: string | null
  messages: ChatMessage[]
  completedTaskIds: Set<string>

  selectNode: (nodeId: string) => void
  handleTraceEvent: (event: TraceEvent) => void
  applySnapshot: (snapshot: TaskSnapshot) => void
  applyArchive: (archive: TraceArchive) => void
  submitMessage: (message: string) => Promise<void>
  resetTrace: () => void
}

export const useAppStore = create<AppStore>((set, get) => ({
  traceState: createInitialTraceState(),
  conversationId: null,
  messages: [],
  completedTaskIds: new Set(),

  selectNode: (nodeId) =>
    set((state) => ({
      traceState: {
        ...state.traceState,
        selectedNodeId: state.traceState.nodesById[nodeId]
          ? nodeId
          : state.traceState.selectedNodeId,
      },
    })),

  handleTraceEvent: (event) =>
    set((state) => {
      const newTraceState = applyTraceEvent(state.traceState, event)
      if (
        event.type === 'task.completed' &&
        !state.completedTaskIds.has(event.task_id)
      ) {
        const newCompleted = new Set(state.completedTaskIds)
        newCompleted.add(event.task_id)
        return {
          traceState: newTraceState,
          completedTaskIds: newCompleted,
          messages: [
            ...state.messages,
            {
              id: `${event.task_id}-assistant`,
              role: 'assistant' as const,
              content: event.payload.final_answer,
            },
          ],
        }
      }
      return { traceState: newTraceState }
    }),

  applySnapshot: (snapshot) =>
    set((state) => ({
      traceState: applyTraceSnapshot(state.traceState, snapshot),
      conversationId: snapshot.conversation_id,
    })),

  applyArchive: (archive) =>
    set({
      traceState: applyTraceSnapshot(
        createInitialTraceState(),
        archive.task,
      ),
      conversationId: archive.task.conversation_id,
    }),

  submitMessage: async (message) => {
    const clientMessageId = createClientId('msg')
    set((state) => ({
      messages: [
        ...state.messages,
        { id: clientMessageId, role: 'user' as const, content: message },
      ],
    }))
    const response = await createAgentTask({
      conversation_id: get().conversationId,
      client_message_id: clientMessageId,
      message,
      stream: true,
    })
    set({
      conversationId: response.conversation_id,
      traceState: {
        ...createInitialTraceState(),
        taskId: response.task_id,
        conversationId: response.conversation_id,
        status: 'running',
      },
    })
  },

  resetTrace: () =>
    set({
      traceState: createInitialTraceState(),
    }),
}))

function createClientId(prefix: string): string {
  if ('randomUUID' in crypto) {
    return `${prefix}_${crypto.randomUUID()}`
  }
  return `${prefix}_${Date.now()}_${Math.random().toString(16).slice(2)}`
}
