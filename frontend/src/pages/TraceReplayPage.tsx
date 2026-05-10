import { useCallback, useEffect, useMemo, useState } from 'react'
import { getTraceArchive } from '../api/agentTrace'
import { LoadingInline } from '../components/LoadingInline'
import { TraceDetailPanel } from '../components/TraceDetailPanel'
import { TraceReplayControls } from '../components/TraceReplayControls'
import { TraceTimeline } from '../components/TraceTimeline'
import { useTraceReplay } from '../hooks/useTraceReplay'
import { applyTraceEvent, createInitialTraceState, type TraceState } from '../state/traceReducer'
import type { TraceArchive, TraceEvent } from '../types/trace'

interface TraceReplayPageProps {
  fileName: string
  onBack: () => void
}

export function TraceReplayPage({ fileName, onBack }: TraceReplayPageProps) {
  const [archive, setArchive] = useState<TraceArchive | null>(null)
  const [state, setState] = useState<TraceState>(() => createInitialTraceState())
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    getTraceArchive(fileName)
      .then((loaded) => {
        if (!cancelled) {
          setArchive(loaded)
          setState(createInitialTraceState())
        }
      })
      .catch(() => {
        if (!cancelled) {
          setError('无法加载 trace 文件。')
        }
      })
    return () => {
      cancelled = true
    }
  }, [fileName])

  const handleReplayEvent = useCallback((event: TraceEvent) => {
    setState((current) => applyTraceEvent(current, event))
  }, [])

  const replay = useTraceReplay({
    events: archive?.task.events ?? [],
    onEvent: handleReplayEvent,
  })

  const restart = useCallback(() => {
    setState(createInitialTraceState())
    replay.restart()
  }, [replay])

  const selectedNode = useMemo(
    () => (state.selectedNodeId ? state.nodesById[state.selectedNodeId] ?? null : null),
    [state.nodesById, state.selectedNodeId],
  )

  return (
    <main className="min-h-dvh bg-slate-50">
      <div className="mx-auto w-full max-w-7xl px-4 py-6 sm:px-6">
        <div className="mb-5 flex flex-wrap items-center justify-between gap-3">
          <div>
            <h1 className="text-2xl font-semibold text-slate-950">Trace 回放</h1>
            <p className="mt-1 text-sm text-slate-600">{fileName}</p>
          </div>
          <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={onBack} type="button">
            返回聊天
          </button>
        </div>
        <TraceReplayControls
          currentIndex={replay.currentIndex}
          isComplete={replay.isComplete}
          isPlaying={replay.isPlaying}
          onPause={replay.pause}
          onPlay={replay.play}
          onRestart={restart}
          onStep={replay.step}
          total={replay.total}
        />
        {!archive && !error ? <LoadingInline label="正在加载 trace 文件" /> : null}
        {error ? <div className="mt-4 rounded-md border border-red-300 bg-red-50 p-4 text-sm text-red-700">{error}</div> : null}
        <div className="mt-5 grid gap-5 lg:grid-cols-[minmax(0,1fr)_24rem]">
          <TraceTimeline onSelectNode={(nodeId) => setState((current) => ({ ...current, selectedNodeId: nodeId }))} state={state} />
          <TraceDetailPanel node={selectedNode} />
        </div>
      </div>
    </main>
  )
}
