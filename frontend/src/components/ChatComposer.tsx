import { useState } from 'react'
import { LoadingInline } from './LoadingInline'

interface ChatComposerProps {
  disabled: boolean
  onSubmit: (message: string) => Promise<void>
}

export function ChatComposer({ disabled, onSubmit }: ChatComposerProps) {
  const [message, setMessage] = useState('')
  const [isComposing, setIsComposing] = useState(false)
  const [isSubmitting, setIsSubmitting] = useState(false)

  const submit = async () => {
    const trimmed = message.trim()
    if (!trimmed || disabled || isSubmitting) {
      return
    }

    setIsSubmitting(true)
    try {
      await onSubmit(trimmed)
      setMessage('')
    } finally {
      setIsSubmitting(false)
    }
  }

  return (
    <div className="rounded-md border border-slate-300 bg-white p-3 shadow-sm">
      <textarea
        aria-label="消息内容"
        className="min-h-28 w-full resize-none bg-transparent text-base text-slate-950 outline-none placeholder:text-slate-400"
        disabled={disabled || isSubmitting}
        onChange={(event) => setMessage(event.target.value)}
        onCompositionEnd={() => setIsComposing(false)}
        onCompositionStart={() => setIsComposing(true)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && !event.shiftKey && !isComposing) {
            event.preventDefault()
            void submit()
          }
        }}
        placeholder="输入要交给 Agent 的任务"
        value={message}
      />
      <div className="mt-3 flex items-center justify-between border-t border-slate-200 pt-3">
        <span className="min-h-5">
          {isSubmitting ? <LoadingInline label="正在创建任务" /> : null}
        </span>
        <button
          aria-label="发送消息"
          className="inline-flex h-10 items-center justify-center rounded-md bg-slate-950 px-4 text-sm font-medium text-white transition hover:bg-slate-800 disabled:cursor-not-allowed disabled:bg-slate-400"
          disabled={!message.trim() || disabled || isSubmitting}
          onClick={() => void submit()}
          type="button"
        >
          发送
        </button>
      </div>
    </div>
  )
}
