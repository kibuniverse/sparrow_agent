interface TraceReplayControlsProps {
  currentIndex: number
  total: number
  isComplete: boolean
  isPlaying: boolean
  onPause: () => void
  onPlay: () => void
  onRestart: () => void
  onStep: () => void
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
  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-slate-300 pb-4">
      <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={isPlaying ? onPause : onPlay} type="button">
        {isPlaying ? '暂停' : '播放'}
      </button>
      <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" disabled={isComplete} onClick={onStep} type="button">
        单步
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
