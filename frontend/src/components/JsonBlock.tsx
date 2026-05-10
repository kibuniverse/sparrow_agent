import type { JsonSnapshot } from '../types/trace'

interface JsonBlockProps {
  snapshot: JsonSnapshot | null
  emptyLabel?: string
}

export function JsonBlock({ snapshot, emptyLabel = '无内容' }: JsonBlockProps) {
  if (!snapshot) {
    return <p className="text-sm text-slate-500">{emptyLabel}</p>
  }

  const text =
    snapshot.text ||
    (typeof snapshot.value === 'string' ? snapshot.value : JSON.stringify(snapshot.value, null, 2))

  return (
    <pre className="max-h-[280px] overflow-auto rounded-md border border-slate-300 bg-slate-950 p-3 text-xs leading-relaxed text-slate-50">
      {formatJsonText(text)}
    </pre>
  )
}

function formatJsonText(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2)
  } catch {
    return text
  }
}
