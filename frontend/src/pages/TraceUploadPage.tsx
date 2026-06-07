import { useCallback, useMemo, useRef, useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { TraceDetailPanel } from '../components/TraceDetailPanel'
import { TraceReplayControls } from '../components/TraceReplayControls'
import { TraceTimeline } from '../components/TraceTimeline'
import { useTraceReplay } from '../hooks/useTraceReplay'
import { useAppStore } from '../store'
import {
  applyTraceEvent,
  applyTraceSnapshot,
  createInitialTraceState,
  type TraceState,
} from '../state/traceReducer'
import type { TaskSnapshot, TraceArchive, TraceEvent } from '../types/trace'

type ViewMode = 'upload' | 'preview' | 'replay'

export function TraceUploadPage() {
  const navigate = useNavigate()
  const resetTrace = useAppStore((s) => s.resetTrace)

  const [viewMode, setViewMode] = useState<ViewMode>('upload')
  const [error, setError] = useState<string | null>(null)
  const [fileName, setFileName] = useState<string | null>(null)
  const [snapshot, setSnapshot] = useState<TaskSnapshot | null>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)

  const [previewState, setPreviewState] = useState<TraceState>(() => createInitialTraceState())
  const [replayState, setReplayState] = useState<TraceState>(() => createInitialTraceState())

  const activeState = viewMode === 'replay' ? replayState : previewState

  const selectNode = useCallback(
    (nodeId: string) => {
      const updater = (current: TraceState) => ({
        ...current,
        selectedNodeId: current.nodesById[nodeId] ? nodeId : current.selectedNodeId,
      })
      if (viewMode === 'replay') {
        setReplayState(updater)
      } else {
        setPreviewState(updater)
      }
    },
    [viewMode],
  )

  const selectedNode = useMemo(
    () => (activeState.selectedNodeId ? activeState.nodesById[activeState.selectedNodeId] ?? null : null),
    [activeState.nodesById, activeState.selectedNodeId],
  )

  const handleReplayEvent = useCallback((event: TraceEvent) => {
    setReplayState((current) => applyTraceEvent(current, event))
  }, [])

  const replay = useTraceReplay({
    events: snapshot?.events ?? [],
    onEvent: handleReplayEvent,
  })

  const enterReplay = useCallback(() => {
    setReplayState(createInitialTraceState())
    replay.restart()
    setViewMode('replay')
  }, [replay])

  const enterPreview = useCallback(() => {
    replay.pause()
    setViewMode('preview')
  }, [replay])

  const handleRestart = useCallback(() => {
    setReplayState(createInitialTraceState())
    replay.restart()
  }, [replay])

  const parseFile = useCallback((file: File) => {
    setFileName(file.name)
    setError(null)

    const reader = new FileReader()
    reader.onload = () => {
      try {
        const json = JSON.parse(reader.result as string)

        let parsed: TaskSnapshot
        if (isTraceArchive(json)) {
          parsed = json.task
        } else if (isTaskSnapshot(json)) {
          parsed = json
        } else {
          throw new Error('无法识别的 JSON 结构。请上传 TraceArchive 或 TaskSnapshot 格式的文件。')
        }

        setSnapshot(parsed)
        setPreviewState(applyTraceSnapshot(createInitialTraceState(), parsed))
        setReplayState(createInitialTraceState())
        setViewMode('preview')
      } catch (caught: unknown) {
        const message = caught instanceof Error ? caught.message : 'JSON 解析失败。'
        setError(message)
      }
    }
    reader.onerror = () => {
      setError('文件读取失败。')
    }
    reader.readAsText(file)
  }, [])

  const handleFileInput = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0]
      if (file) parseFile(file)
    },
    [parseFile],
  )

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault()
      e.stopPropagation()
      const file = e.dataTransfer.files[0]
      if (file) parseFile(file)
    },
    [parseFile],
  )

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault()
    e.stopPropagation()
  }, [])

  const handleReset = useCallback(() => {
    setViewMode('upload')
    setError(null)
    setFileName(null)
    setSnapshot(null)
    setPreviewState(createInitialTraceState())
    setReplayState(createInitialTraceState())
    if (fileInputRef.current) fileInputRef.current.value = ''
  }, [])

  const openTask = (taskId: string) => {
    resetTrace()
    navigate({ to: '/tasks/$taskId', params: { taskId } })
  }

  const isLoaded = viewMode === 'preview' || viewMode === 'replay'

  return (
    <main className="min-h-dvh bg-slate-50">
      <div className="mx-auto w-full max-w-7xl px-4 py-6 sm:px-6">
        <div className="mb-5 flex flex-wrap items-center justify-between gap-3 border-b border-slate-300 pb-4">
          <div>
            <h1 className="text-2xl font-semibold text-slate-950">
              {viewMode === 'replay' ? 'Trace 回放' : '上传 Trace 文件'}
            </h1>
            <p className="mt-1 text-sm text-slate-600">
              {isLoaded ? fileName : '选择或拖拽 .json 文件进行解析预览'}
            </p>
          </div>
          <div className="flex gap-2">
            {isLoaded ? (
              <>
                {viewMode === 'preview' ? (
                  <button
                    className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700"
                    onClick={enterReplay}
                    type="button"
                  >
                    回放
                  </button>
                ) : (
                  <button
                    className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700"
                    onClick={enterPreview}
                    type="button"
                  >
                    预览
                  </button>
                )}
                <button
                  className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700"
                  onClick={handleReset}
                  type="button"
                >
                  重新上传
                </button>
              </>
            ) : null}
            <button
              className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700"
              onClick={() => navigate({ to: '/' })}
              type="button"
            >
              返回聊天
            </button>
          </div>
        </div>

        {viewMode === 'upload' ? (
          error ? (
            <div className="space-y-3">
              <div className="rounded-md border border-red-300 bg-red-50 p-4 text-sm text-red-700">
                {error}
              </div>
              <UploadZone
                fileInputRef={fileInputRef}
                onDrop={handleDrop}
                onDragOver={handleDragOver}
                onFileInput={handleFileInput}
              />
            </div>
          ) : (
            <UploadZone
              fileInputRef={fileInputRef}
              onDrop={handleDrop}
              onDragOver={handleDragOver}
              onFileInput={handleFileInput}
            />
          )
        ) : null}

        {viewMode === 'replay' ? (
          <TraceReplayControls
            currentIndex={replay.currentIndex}
            isComplete={replay.isComplete}
            isPlaying={replay.isPlaying}
            onPause={replay.pause}
            onPlay={replay.play}
            onRestart={handleRestart}
            onStep={replay.step}
            total={replay.total}
          />
        ) : null}

        {isLoaded ? (
          <div className={viewMode === 'replay' ? 'mt-5 grid gap-5 lg:grid-cols-[minmax(0,1fr)_24rem]' : 'grid gap-5 lg:grid-cols-[minmax(0,1fr)_24rem]'}>
            <TraceTimeline onSelectNode={selectNode} state={activeState} />
            <TraceDetailPanel
              className="lg:sticky lg:top-24 lg:max-h-[calc(100dvh-7rem)] lg:overflow-auto"
              node={selectedNode}
              onOpenTask={openTask}
            />
          </div>
        ) : null}
      </div>
    </main>
  )
}

interface UploadZoneProps {
  fileInputRef: React.RefObject<HTMLInputElement | null>
  onDrop: (e: React.DragEvent) => void
  onDragOver: (e: React.DragEvent) => void
  onFileInput: (e: React.ChangeEvent<HTMLInputElement>) => void
}

function UploadZone({ fileInputRef, onDrop, onDragOver, onFileInput }: UploadZoneProps) {
  const [dragging, setDragging] = useState(false)

  return (
    <label
      className={`flex min-h-[40vh] cursor-pointer flex-col items-center justify-center rounded-lg border-2 border-dashed transition-colors ${
        dragging ? 'border-blue-500 bg-blue-50' : 'border-slate-300 bg-white hover:border-slate-400'
      }`}
      onDragEnter={() => setDragging(true)}
      onDragLeave={() => setDragging(false)}
      onDrop={(e) => {
        setDragging(false)
        onDrop(e)
      }}
      onDragOver={(e) => {
        onDragOver(e)
        setDragging(true)
      }}
    >
      <svg className="mb-3 h-10 w-10 text-slate-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
        <path strokeLinecap="round" strokeLinejoin="round" d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5m-13.5-9L12 3m0 0l4.5 4.5M12 3v13.5" />
      </svg>
      <p className="text-sm font-medium text-slate-700">
        拖拽文件到此处，或<span className="text-blue-600">点击选择文件</span>
      </p>
      <p className="mt-1 text-xs text-slate-500">支持 .json 格式（TraceArchive / TaskSnapshot）</p>
      <input accept=".json,application/json" className="sr-only" onChange={onFileInput} ref={fileInputRef} type="file" />
    </label>
  )
}

function isTraceArchive(json: unknown): json is TraceArchive {
  if (typeof json !== 'object' || json === null) return false
  const obj = json as Record<string, unknown>
  return obj.schema_version !== undefined && obj.task !== undefined
}

function isTaskSnapshot(json: unknown): json is TaskSnapshot {
  if (typeof json !== 'object' || json === null) return false
  const obj = json as Record<string, unknown>
  return Array.isArray(obj.events) && typeof obj.task_id === 'string'
}
