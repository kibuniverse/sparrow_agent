import { useEffect, useMemo, useState } from 'react'
import { AgentTraceApiError, getTraceArchive } from '../api/agentTrace'
import { LoadingInline } from '../components/LoadingInline'
import { TraceDetailPanel } from '../components/TraceDetailPanel'
import { TraceTimeline } from '../components/TraceTimeline'
import type { TraceArchive } from '../types/trace'
import type { TraceState } from '../state/traceReducer'

interface TraceArchivePageProps {
  fileName: string
  state: TraceState
  onApplyArchive: (archive: TraceArchive) => void
  onBack: () => void
  onReplay: (fileName: string) => void
  onSelectNode: (nodeId: string) => void
}

export function TraceArchivePage({
  fileName,
  state,
  onApplyArchive,
  onBack,
  onReplay,
  onSelectNode,
}: TraceArchivePageProps) {
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    getTraceArchive(fileName)
      .then((archive) => {
        if (!cancelled) {
          onApplyArchive(archive)
        }
      })
      .catch((caught: unknown) => {
        if (!cancelled) {
          setError(readArchiveError(caught))
        }
      })
      .finally(() => {
        if (!cancelled) {
          setIsLoading(false)
        }
      })

    return () => {
      cancelled = true
    }
  }, [fileName, onApplyArchive])

  useEffect(() => {
    if (state.selectedNodeId || state.rootNodeIds.length === 0) {
      return
    }
    const fallback = state.rootNodeIds.at(-1)
    if (fallback) {
      onSelectNode(fallback)
    }
  }, [onSelectNode, state.rootNodeIds, state.selectedNodeId])

  const selectedNode = useMemo(
    () => (state.selectedNodeId ? state.nodesById[state.selectedNodeId] ?? null : null),
    [state.nodesById, state.selectedNodeId],
  )

  return (
    <main className="min-h-dvh bg-slate-50">
      <div className="mx-auto w-full max-w-7xl px-4 py-6 sm:px-6">
        <div className="mb-5 flex flex-wrap items-center justify-between gap-3 border-b border-slate-300 pb-4">
          <div>
            <h1 className="text-2xl font-semibold text-slate-950">Trace 预览</h1>
            <p className="mt-1 text-sm text-slate-600">{fileName}</p>
          </div>
          <div className="flex gap-2">
            <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={() => onReplay(fileName)} type="button">
              回放
            </button>
            <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={onBack} type="button">
              返回聊天
            </button>
          </div>
        </div>
        {isLoading ? <LoadingInline label="正在加载 trace 文件" /> : null}
        {error ? <div className="rounded-md border border-red-300 bg-red-50 p-4 text-sm text-red-700">{error}</div> : null}
        <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_24rem]">
          <TraceTimeline onSelectNode={onSelectNode} state={state} />
          <TraceDetailPanel node={selectedNode} />
        </div>
      </div>
    </main>
  )
}

function readArchiveError(error: unknown): string {
  if (error instanceof AgentTraceApiError) {
    return error.message
  }
  return '无法加载 trace 文件。'
}
