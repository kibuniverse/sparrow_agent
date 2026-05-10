import { LoadingInline } from './LoadingInline'
import type { TraceNode } from '../types/trace'

interface TraceNodeRowProps {
  node: TraceNode
  depth: number
  selected: boolean
  onSelect: (nodeId: string) => void
}

export function TraceNodeRow({ node, depth, selected, onSelect }: TraceNodeRowProps) {
  return (
    <button
      aria-current={selected ? 'true' : undefined}
      aria-label={`查看 ${node.title}`}
      className="trace-node-row grid w-full grid-cols-[1rem_1fr] gap-3 rounded-md px-3 py-3 text-left transition hover:bg-slate-100"
      onClick={() => onSelect(node.id)}
      style={{ marginLeft: `${depth * 1.5}rem`, width: `calc(100% - ${depth * 1.5}rem)` }}
      type="button"
    >
      <span className={`mt-1 h-3 w-3 rounded-full ${statusClassName(node.status)}`} />
      <span>
        <span className="flex flex-wrap items-center gap-2">
          <span className="font-medium text-slate-950">{node.title}</span>
          {node.status === 'running' ? <LoadingInline label="运行中" /> : null}
        </span>
        <span className="mt-1 block break-words text-sm text-slate-600">{node.subtitle}</span>
      </span>
    </button>
  )
}

function statusClassName(status: TraceNode['status']): string {
  if (status === 'succeeded') {
    return 'bg-green-600'
  }
  if (status === 'failed') {
    return 'bg-red-600'
  }
  if (status === 'running') {
    return 'bg-sky-600 motion-safe:animate-pulse'
  }
  return 'bg-slate-300'
}
