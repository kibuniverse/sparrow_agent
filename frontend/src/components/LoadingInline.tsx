export function LoadingInline({ label = '运行中' }: { label?: string }) {
  return (
    <span className="inline-flex items-center gap-2 text-sm text-slate-600" aria-live="polite">
      <span className="h-2 w-2 rounded-full bg-sky-500 motion-safe:animate-pulse" />
      <span>{label}</span>
    </span>
  )
}
