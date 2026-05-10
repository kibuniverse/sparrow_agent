import { useEffect, useMemo, useState } from 'react'
import { AgentTraceApiError, getTaskSnapshot } from '../api/agentTrace'
import { LoadingInline } from '../components/LoadingInline'
import { TraceDetailPanel } from '../components/TraceDetailPanel'
import { TraceTimeline } from '../components/TraceTimeline'
import type { TraceState } from '../state/traceReducer'
import type { TaskSnapshot } from '../types/trace'

interface TaskDetailPageProps {
  taskId: string
  state: TraceState
  onApplySnapshot: (snapshot: TaskSnapshot) => void
  onBack: () => void
  onSelectNode: (nodeId: string) => void
}

export function TaskDetailPage({
  taskId,
  state,
  onApplySnapshot,
  onBack,
  onSelectNode,
}: TaskDetailPageProps) {
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false

    getTaskSnapshot(taskId)
      .then((snapshot) => {
        if (!cancelled) {
          onApplySnapshot(snapshot)
        }
      })
      .catch((caught: unknown) => {
        if (!cancelled) {
          setError(readSnapshotError(caught))
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
  }, [onApplySnapshot, taskId])

  useEffect(() => {
    if (state.selectedNodeId || state.rootNodeIds.length === 0) {
      return
    }

    const fallback = state.latestRunningNodeId ?? state.rootNodeIds.at(-1)
    if (fallback) {
      onSelectNode(fallback)
    }
  }, [onSelectNode, state.latestRunningNodeId, state.rootNodeIds, state.selectedNodeId])

  const selectedNode = useMemo(
    () => (state.selectedNodeId ? state.nodesById[state.selectedNodeId] ?? null : null),
    [state.nodesById, state.selectedNodeId],
  )

  return (
    <main className="min-h-dvh bg-slate-50">
      <div className="mx-auto w-full max-w-7xl px-4 py-6 sm:px-6">
        <div className="mb-5 flex flex-wrap items-center justify-between gap-3 border-b border-slate-300 pb-4">
          <div>
            <h1 className="text-2xl font-semibold text-slate-950">任务详情</h1>
            <p className="mt-1 text-sm text-slate-600">
              {state.status === 'running' ? 'running' : state.status} · {state.startedAt ?? taskId}
            </p>
          </div>
          <button
            className="inline-flex h-9 items-center rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700 transition hover:border-sky-500 hover:text-sky-700"
            onClick={onBack}
            type="button"
          >
            返回聊天
          </button>
        </div>

        {isLoading ? (
          <div className="py-8">
            <LoadingInline label="正在加载任务快照" />
          </div>
        ) : null}
        {error ? (
          <div className="rounded-md border border-red-300 bg-red-50 p-4 text-sm text-red-700">
            {error}
          </div>
        ) : null}

        <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_24rem]">
          <TraceTimeline onSelectNode={onSelectNode} state={state} />
          <TraceDetailPanel node={selectedNode} />
        </div>
      </div>
    </main>
  )
}

function readSnapshotError(error: unknown): string {
  if (error instanceof AgentTraceApiError) {
    if (error.status === 404 || error.code === 'task_not_found') {
      return '任务不存在。'
    }
    if (error.status === 410 || error.code === 'task_expired') {
      return '任务已过期。'
    }
    return error.message
  }
  return '无法加载任务快照。'
}
