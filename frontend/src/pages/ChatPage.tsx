import { useNavigate } from '@tanstack/react-router'
import { ChatComposer } from '../components/ChatComposer'
import { ThinkingPreview } from '../components/ThinkingPreview'
import { useTaskStream } from '../hooks/useTaskStream'
import { useAppStore } from '../store'

export function ChatPage() {
  const navigate = useNavigate()
  const traceState = useAppStore((s) => s.traceState)
  const messages = useAppStore((s) => s.messages)
  const submitMessage = useAppStore((s) => s.submitMessage)
  const handleTraceEvent = useAppStore((s) => s.handleTraceEvent)
  const resetTrace = useAppStore((s) => s.resetTrace)

  const running = traceState.status === 'running'

  useTaskStream({
    taskId: traceState.taskId,
    enabled: running && Boolean(traceState.taskId),
    lastSeq: traceState.lastSeq,
    onEvent: handleTraceEvent,
  })

  return (
    <main className="min-h-dvh bg-slate-50">
      <div className="mx-auto flex min-h-dvh w-full max-w-5xl flex-col px-4 py-6 sm:px-6">
        <section className="flex-1 space-y-3 overflow-y-auto pb-6">
          {messages.length === 0 ? (
            <div className="flex min-h-[45vh] flex-col items-center justify-center gap-4 text-center">
              <h1 className="text-2xl font-semibold text-slate-950">Agent Trace</h1>
              <button
                className="text-sm text-blue-600 hover:text-blue-800 hover:underline"
                onClick={() => navigate({ to: '/upload' })}
                type="button"
              >
                上传 Trace 文件预览
              </button>
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
          <ChatComposer disabled={running} onSubmit={submitMessage} />
          <ThinkingPreview
            onOpenDetail={() => {
              if (traceState.taskId) {
                resetTrace()
                navigate({ to: '/tasks/$taskId', params: { taskId: traceState.taskId } })
              }
            }}
            state={traceState}
          />
        </div>
      </div>
    </main>
  )
}
