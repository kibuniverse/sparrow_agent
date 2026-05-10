import { ChatComposer } from '../components/ChatComposer'
import { ThinkingPreview } from '../components/ThinkingPreview'
import type { TraceState } from '../state/traceReducer'

export interface ChatMessage {
  id: string
  role: 'user' | 'assistant'
  content: string
}

interface ChatPageProps {
  messages: ChatMessage[]
  traceState: TraceState
  onSubmitMessage: (message: string) => Promise<void>
  onOpenTask: (taskId: string) => void
}

export function ChatPage({ messages, traceState, onSubmitMessage, onOpenTask }: ChatPageProps) {
  const running = traceState.status === 'running'

  return (
    <main className="min-h-dvh bg-slate-50">
      <div className="mx-auto flex min-h-dvh w-full max-w-5xl flex-col px-4 py-6 sm:px-6">
        <section className="flex-1 space-y-3 overflow-y-auto pb-6">
          {messages.length === 0 ? (
            <div className="flex min-h-[45vh] items-center justify-center text-center">
              <h1 className="text-2xl font-semibold text-slate-950">Agent Trace</h1>
            </div>
          ) : (
            messages.map((message) => (
              <article
                className={`max-w-[80%] rounded-md px-4 py-3 text-sm leading-6 ${
                  message.role === 'user'
                    ? 'ml-auto bg-slate-950 text-white'
                    : 'mr-auto border border-slate-300 bg-white text-slate-800'
                }`}
                key={message.id}
              >
                {message.content}
              </article>
            ))
          )}
        </section>
        <div className="pb-4">
          <ChatComposer disabled={running} onSubmit={onSubmitMessage} />
          <ThinkingPreview
            onOpenDetail={() => traceState.taskId && onOpenTask(traceState.taskId)}
            state={traceState}
          />
        </div>
      </div>
    </main>
  )
}
