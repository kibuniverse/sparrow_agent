import { LoadingInline } from './LoadingInline'
import type { TraceState } from '../state/traceReducer'

interface ThinkingPreviewProps {
  state: TraceState
  onOpenDetail: () => void
}

export function ThinkingPreview({ state, onOpenDetail }: ThinkingPreviewProps) {
  if (state.status === 'idle' || !state.taskId) {
    return null
  }

  const statusText = getStatusText(state)
  const reasoning = tailText(state.latestReasoningText, 240)

  return (
    <section className="mt-3 rounded-md border border-slate-300 bg-white p-4 shadow-sm">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="space-y-1">
          <div className="flex items-center gap-2 text-sm font-medium text-slate-950">
            {state.status === 'running' ? <LoadingInline label={statusText} /> : <span>{statusText}</span>}
          </div>
          {reasoning ? <p className="max-w-3xl text-sm text-slate-600">{reasoning}</p> : null}
        </div>
        <button
          aria-label="查看任务详情"
          className="inline-flex h-9 items-center rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700 transition hover:border-sky-500 hover:text-sky-700"
          onClick={onOpenDetail}
          type="button"
        >
          查看详情
        </button>
      </div>
    </section>
  )
}

function getStatusText(state: TraceState): string {
  if (state.status === 'failed') {
    return `任务执行失败：${state.error ?? '未知错误'}`
  }

  if (state.status === 'succeeded') {
    return '任务执行完成'
  }

  const runningNode = state.latestRunningNodeId ? state.nodesById[state.latestRunningNodeId] : null
  const currentNode = runningNode ?? findLatestOutputNode(state)
  if (!currentNode) {
    return '任务正在运行'
  }

  if (currentNode.detail.type === 'model_call') {
    return `模型调用（第 ${currentNode.round ?? 1} 轮）正在思考`
  }

  if (currentNode.detail.type === 'model_output') {
    if (currentNode.detail.kind === 'tool_calls' && currentNode.detail.toolCalls.length > 0) {
      const names = currentNode.detail.toolCalls.map((toolCall) => toolCall.name).join('、')
      return `准备调用 ${currentNode.detail.toolCalls.length} 个工具：${names}`
    }
    return '正在生成回复'
  }

  return `正在执行 ${currentNode.detail.name}`
}

function findLatestOutputNode(state: TraceState) {
  const nodes = Object.values(state.nodesById)
  for (let index = nodes.length - 1; index >= 0; index -= 1) {
    const node = nodes[index]
    if (node.detail.type === 'model_output') {
      return node
    }
  }
  return null
}

function tailText(value: string, maxLength: number): string {
  if (value.length <= maxLength) {
    return value
  }
  return value.slice(value.length - maxLength)
}
