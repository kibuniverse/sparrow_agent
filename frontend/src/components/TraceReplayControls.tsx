import { useState } from 'react'

interface TraceReplayControlsProps {
  currentIndex: number
  total: number
  isComplete: boolean
  isPlaying: boolean
  onPause: () => void
  onPlay: () => void
  onRestart: () => void
  onStep: (count?: number) => void
}

export function TraceReplayControls({
  currentIndex,
  total,
  isComplete,
  isPlaying,
  onPause,
  onPlay,
  onRestart,
  onStep,
}: TraceReplayControlsProps) {
  const [stepCount, setStepCount] = useState(1)
  const remaining = Math.max(0, total - currentIndex)
  const jumpCount = Math.max(1, Math.min(stepCount, Math.max(1, remaining)))

  return (
    <div className="sticky top-0 z-20 flex flex-wrap items-center gap-2 border-b border-slate-300 bg-slate-50/95 py-3 backdrop-blur">
      <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={isPlaying ? onPause : onPlay} type="button">
        {isPlaying ? '暂停' : '播放'}
      </button>
      <label className="flex h-9 items-center gap-2 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700">
        <span>步数</span>
        <input
          className="w-16 bg-transparent text-right outline-none"
          disabled={isComplete}
          min={1}
          onChange={(event) => {
            const nextCount = Number.parseInt(event.currentTarget.value, 10)
            setStepCount(Number.isFinite(nextCount) ? Math.max(1, nextCount) : 1)
          }}
          type="number"
          value={stepCount}
        />
      </label>
      <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" disabled={isComplete} onClick={() => onStep(jumpCount)} type="button">
        前进
      </button>
      <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={onRestart} type="button">
        重来
      </button>
      <div className="min-w-40 text-sm text-slate-600">
        {currentIndex} / {total}
      </div>
    </div>
  )
}
