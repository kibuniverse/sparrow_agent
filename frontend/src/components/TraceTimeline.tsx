import { TraceNodeRow } from './TraceNodeRow'
import type { TraceState } from '../state/traceReducer'

interface TraceTimelineProps {
  state: TraceState
  onSelectNode: (nodeId: string) => void
}

export function TraceTimeline({ state, onSelectNode }: TraceTimelineProps) {
  if (state.rootNodeIds.length === 0) {
    return (
      <div className="rounded-md border border-slate-300 bg-white p-6 text-sm text-slate-500">
        暂无 trace 事件
      </div>
    )
  }

  return (
    <div className="trace-timeline space-y-1 border-l border-slate-300 pl-3">
      {state.rootNodeIds.map((nodeId) => (
        <TraceBranch
          key={nodeId}
          depth={0}
          nodeId={nodeId}
          onSelectNode={onSelectNode}
          state={state}
        />
      ))}
    </div>
  )
}

function TraceBranch({
  state,
  nodeId,
  depth,
  onSelectNode,
}: {
  state: TraceState
  nodeId: string
  depth: number
  onSelectNode: (nodeId: string) => void
}) {
  const node = state.nodesById[nodeId]
  if (!node) {
    return null
  }

  return (
    <div>
      <TraceNodeRow
        depth={depth}
        node={node}
        onSelect={onSelectNode}
        selected={state.selectedNodeId === node.id}
      />
      {node.childrenIds.map((childId) => (
        <TraceBranch
          key={childId}
          depth={depth + 1}
          nodeId={childId}
          onSelectNode={onSelectNode}
          state={state}
        />
      ))}
    </div>
  )
}
