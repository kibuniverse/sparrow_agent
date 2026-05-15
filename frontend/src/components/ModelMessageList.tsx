import type {
  JsonSnapshot,
  ModelMessageSnapshot,
  ModelRequestSnapshotValue,
  ModelResponseSnapshotValue,
} from '../types/trace'
import { JsonBlock } from './JsonBlock'

interface ModelMessageListProps {
  messages: ModelMessageSnapshot[]
  emptyLabel: string
}

export function ModelMessageList({ messages, emptyLabel }: ModelMessageListProps) {
  if (messages.length === 0) {
    return <p className="text-sm text-slate-500">{emptyLabel}</p>
  }

  return (
    <ol className="space-y-3">
      {messages.map((message, index) => (
        <li key={`${message.role}-${index}`} className="border-l-2 border-slate-300 pl-3">
          <div className="flex flex-wrap items-center gap-2">
            <span className="rounded bg-slate-200 px-2 py-0.5 text-xs font-medium text-slate-700">
              {message.role}
            </span>
            {message.tool_call_id ? (
              <span className="text-xs text-slate-500">tool_call_id: {message.tool_call_id}</span>
            ) : null}
          </div>
          {message.content ? (
            <p className="mt-2 whitespace-pre-wrap text-sm text-slate-800">{message.content}</p>
          ) : null}
          {message.reasoning_content ? (
            <p className="mt-2 whitespace-pre-wrap text-sm text-slate-600">
              {message.reasoning_content}
            </p>
          ) : null}
          {message.tool_calls?.length ? (
            <div className="mt-2 space-y-2">
              {message.tool_calls.map((toolCall, toolIndex) => (
                <div key={toolCall.id ?? toolIndex} className="rounded-md bg-slate-100 p-2">
                  <p className="text-sm font-medium text-slate-800">
                    {toolCall.function?.name ?? `工具 ${toolIndex + 1}`}
                  </p>
                  {toolCall.id ? (
                    <p className="mt-1 text-xs text-slate-500">id: {toolCall.id}</p>
                  ) : null}
                  {toolCall.function?.arguments ? (
                    <JsonBlock snapshot={textJsonSnapshot(toolCall.function.arguments)} />
                  ) : null}
                </div>
              ))}
            </div>
          ) : null}
        </li>
      ))}
    </ol>
  )
}

export function requestMessagesFromSnapshot(snapshot: JsonSnapshot | null): ModelMessageSnapshot[] {
  const value = snapshot?.value as ModelRequestSnapshotValue | null | undefined
  return Array.isArray(value?.messages) ? value.messages : []
}

export function responseMessageFromSnapshot(snapshot: JsonSnapshot | null): ModelMessageSnapshot[] {
  const value = snapshot?.value as ModelResponseSnapshotValue | null | undefined
  return value?.message ? [value.message] : []
}

function textJsonSnapshot(text: string): JsonSnapshot {
  return {
    value: parseJsonOrText(text),
    text,
    truncated: false,
  }
}

function parseJsonOrText(text: string): unknown {
  try {
    return JSON.parse(text)
  } catch {
    return text
  }
}
