import { JsonBlock } from './JsonBlock'
import type { TraceNode } from '../types/trace'

interface TraceDetailPanelProps {
  className?: string
  node: TraceNode | null
}

export function TraceDetailPanel({ className = 'lg:sticky lg:top-6', node }: TraceDetailPanelProps) {
  const panelClassName = `rounded-md border border-slate-300 bg-white p-5 ${className}`

  if (!node) {
    return (
      <aside className={`${panelClassName} text-sm text-slate-500`}>
        选择一个节点查看详情
      </aside>
    )
  }

  return (
    <aside className={`${panelClassName} shadow-sm`}>
      <div className="mb-5">
        <p className="text-xs font-medium uppercase tracking-[0.14em] text-slate-500">{node.status}</p>
        <h2 className="mt-1 text-xl font-semibold text-slate-950">{node.title}</h2>
        <p className="mt-1 text-sm text-slate-600">{node.subtitle}</p>
      </div>
      <dl className="grid grid-cols-2 gap-3 text-sm">
        <DetailItem label="开始时间" value={formatDate(node.startedAt)} />
        <DetailItem label="耗时" value={formatDuration(node.durationMs)} />
      </dl>
      <div className="mt-5 space-y-5">{renderDetail(node)}</div>
    </aside>
  )
}

function renderDetail(node: TraceNode) {
  if (node.detail.type === 'model_call') {
    return (
      <>
        <DetailItem label="模型" value={node.detail.model} />
        <DetailItem label="结束原因" value={node.detail.finishReason ?? '运行中'} />
        {node.detail.usage ? (
          <DetailItem
            label="Token"
            value={`${node.detail.usage.total_tokens} total / ${node.detail.usage.reasoning_tokens} reasoning`}
          />
        ) : null}
        <section>
          <h3 className="mb-2 text-sm font-medium text-slate-950">思考过程</h3>
          <p className="whitespace-pre-wrap rounded-md bg-slate-100 p-3 text-sm text-slate-700">
            {node.detail.reasoningText || '暂无内容'}
          </p>
        </section>
        <section>
          <h3 className="mb-2 text-sm font-medium text-slate-950">请求</h3>
          <JsonBlock snapshot={node.detail.request} />
        </section>
        <section>
          <h3 className="mb-2 text-sm font-medium text-slate-950">响应</h3>
          <JsonBlock snapshot={node.detail.response} />
        </section>
      </>
    )
  }

  if (node.detail.type === 'model_output') {
    return (
      <>
        <DetailItem
          label="输出类型"
          value={node.detail.kind === 'tool_calls' ? '工具调用' : '生成结果'}
        />
        {node.detail.content ? (
          <section>
            <h3 className="mb-2 text-sm font-medium text-slate-950">内容</h3>
            <p className="whitespace-pre-wrap rounded-md bg-slate-100 p-3 text-sm text-slate-700">
              {node.detail.content}
            </p>
          </section>
        ) : null}
        {node.detail.toolCalls.length > 0 ? (
          <section>
            <h3 className="mb-2 text-sm font-medium text-slate-950">工具列表</h3>
            <ul className="space-y-2 text-sm text-slate-700">
              {node.detail.toolCalls.map((toolCall) => (
                <li key={toolCall.nodeId} className="rounded-md bg-slate-100 px-3 py-2">
                  {toolCall.name}
                </li>
              ))}
            </ul>
          </section>
        ) : null}
      </>
    )
  }

  return (
    <>
      <DetailItem label="工具名" value={node.detail.name} />
      <DetailItem label="调用 ID" value={node.detail.toolCallId} />
      {node.detail.error ? <DetailItem label="错误" value={node.detail.error} /> : null}
      <section>
        <h3 className="mb-2 text-sm font-medium text-slate-950">参数</h3>
        <JsonBlock snapshot={node.detail.arguments} />
      </section>
      <section>
        <h3 className="mb-2 text-sm font-medium text-slate-950">输出</h3>
        <JsonBlock snapshot={node.detail.output} />
      </section>
    </>
  )
}

function DetailItem({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="text-xs font-medium text-slate-500">{label}</dt>
      <dd className="mt-1 break-words text-sm text-slate-800">{value}</dd>
    </div>
  )
}

function formatDate(value: string | null): string {
  if (!value) {
    return '未知'
  }
  return new Date(value).toLocaleString()
}

function formatDuration(value: number | null): string {
  if (value === null) {
    return '运行中'
  }
  return `${value} ms`
}
