import { useEffect, useMemo, useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { AgentTraceApiError, getTraceArchive } from '../api/agentTrace'
import { LoadingInline } from '../components/LoadingInline'
import { TraceDetailPanel } from '../components/TraceDetailPanel'
import { TraceTimeline } from '../components/TraceTimeline'
import { useAppStore } from '../store'
import type { TraceArchive } from '../types/trace'

interface TraceArchivePageProps {
  fileName: string
}

export function TraceArchivePage({ fileName }: TraceArchivePageProps) {
  const navigate = useNavigate()
  const traceState = useAppStore((s) => s.traceState)
  const applyArchive = useAppStore((s) => s.applyArchive)
  const selectNode = useAppStore((s) => s.selectNode)
  const resetTrace = useAppStore((s) => s.resetTrace)

  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    getTraceArchive(fileName)
      .then((archive: TraceArchive) => {
        if (!cancelled) {
          applyArchive(archive)
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
  }, [fileName, applyArchive])

  useEffect(() => {
    if (traceState.selectedNodeId || traceState.rootNodeIds.length === 0) {
      return
    }
    const fallback = traceState.rootNodeIds.at(-1)
    if (fallback) {
      selectNode(fallback)
    }
  }, [selectNode, traceState.rootNodeIds, traceState.selectedNodeId])

  const selectedNode = useMemo(
    () => (traceState.selectedNodeId ? traceState.nodesById[traceState.selectedNodeId] ?? null : null),
    [traceState.nodesById, traceState.selectedNodeId],
  )

  const openTask = (taskId: string) => {
    resetTrace()
    navigate({ to: '/tasks/$taskId', params: { taskId } })
  }

  return (
    <main className="min-h-dvh bg-slate-50">
      <div className="mx-auto w-full max-w-7xl px-4 py-6 sm:px-6">
        <div className="mb-5 flex flex-wrap items-center justify-between gap-3 border-b border-slate-300 pb-4">
          <div>
            <h1 className="text-2xl font-semibold text-slate-950">Trace 预览</h1>
            <p className="mt-1 text-sm text-slate-600">{fileName}</p>
          </div>
          <div className="flex gap-2">
            <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={() => navigate({ to: '/replay/$fileName', params: { fileName } })} type="button">
              回放
            </button>
            <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={() => { resetTrace(); navigate({ to: '/' }) }} type="button">
              返回聊天
            </button>
          </div>
        </div>
        {isLoading ? <LoadingInline label="正在加载 trace 文件" /> : null}
        {error ? <div className="rounded-md border border-red-300 bg-red-50 p-4 text-sm text-red-700">{error}</div> : null}
        <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_24rem]">
          <TraceTimeline onSelectNode={selectNode} state={traceState} />
          <TraceDetailPanel
            className="lg:sticky lg:top-24 lg:max-h-[calc(100dvh-7rem)] lg:overflow-auto"
            node={selectedNode}
            onOpenTask={openTask}
          />
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
