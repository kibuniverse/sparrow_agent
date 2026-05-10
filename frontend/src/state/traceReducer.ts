import type {
  JsonSnapshot,
  ModelCallDetail,
  ModelOutputDetail,
  TaskSnapshot,
  TaskStatus,
  ToolCallDetail,
  ToolCallPreview,
  TraceEvent,
  TraceNode,
} from '../types/trace'

export interface TraceState {
  taskId: string | null
  conversationId: string | null
  status: TaskStatus | 'idle'
  lastSeq: number
  rootNodeIds: string[]
  nodesById: Record<string, TraceNode>
  selectedNodeId: string | null
  latestReasoningText: string
  latestRunningNodeId: string | null
  finalAnswer: string
  error: string | null
  startedAt: string | null
  updatedAt: string | null
  durationMs: number | null
}

export function createInitialTraceState(): TraceState {
  return {
    taskId: null,
    conversationId: null,
    status: 'idle',
    lastSeq: 0,
    rootNodeIds: [],
    nodesById: {},
    selectedNodeId: null,
    latestReasoningText: '',
    latestRunningNodeId: null,
    finalAnswer: '',
    error: null,
    startedAt: null,
    updatedAt: null,
    durationMs: null,
  }
}

export function applyTraceSnapshot(state: TraceState, snapshot: TaskSnapshot): TraceState {
  const base: TraceState = {
    ...state,
    taskId: snapshot.task_id,
    conversationId: snapshot.conversation_id,
    status: snapshot.status,
    startedAt: snapshot.created_at,
    updatedAt: snapshot.updated_at,
  }

  return snapshot.events
    .slice()
    .sort((left, right) => left.seq - right.seq)
    .reduce(applyTraceEvent, base)
}

export function applyTraceEvent(state: TraceState, event: TraceEvent): TraceState {
  if (event.seq <= state.lastSeq) {
    return state
  }

  const base: TraceState = {
    ...state,
    taskId: event.task_id,
    conversationId: event.conversation_id,
    lastSeq: event.seq,
    updatedAt: event.timestamp,
  }

  switch (event.type) {
    case 'task.started':
      return {
        ...base,
        status: 'running',
        error: null,
        startedAt: event.timestamp,
      }

    case 'task.completed': {
      const completedState: TraceState = {
        ...base,
        status: 'succeeded',
        finalAnswer: event.payload.final_answer,
        error: null,
        durationMs: event.payload.duration_ms,
        latestRunningNodeId: null,
        selectedNodeId: selectFinalOutputNode(base) ?? base.selectedNodeId,
      }

      return completeRunningFinalOutput(
        completedState,
        event.payload.final_answer,
        event.timestamp,
      )
    }

    case 'task.failed':
      return {
        ...base,
        status: 'failed',
        error: event.payload.error,
        durationMs: event.payload.duration_ms,
        latestRunningNodeId: null,
      }

    case 'model_call.started': {
      const detail: ModelCallDetail = {
        type: 'model_call',
        model: event.payload.model,
        request: event.payload.request,
        response: null,
        reasoningText: '',
        usage: null,
        finishReason: null,
      }
      const node = createNode({
        id: event.payload.node_id,
        taskId: event.task_id,
        parentId: null,
        type: 'model_call',
        status: 'running',
        title: `模型调用（第 ${event.payload.round} 轮）`,
        subtitle: '正在思考',
        round: event.payload.round,
        timestamp: event.timestamp,
        detail,
      })

      return upsertNode({
        state: {
          ...base,
          status: 'running',
          latestRunningNodeId: event.payload.node_id,
          selectedNodeId: base.selectedNodeId ?? event.payload.node_id,
        },
        node,
        root: true,
      })
    }

    case 'model_call.reasoning_delta':
      return updateNode(base, event.payload.node_id, (node) => {
        if (node.detail.type !== 'model_call') {
          return node
        }

        const reasoningText = `${node.detail.reasoningText}${event.payload.delta}`
        return {
          ...node,
          subtitle: truncate(reasoningText, 180) || '正在思考',
          detail: { ...node.detail, reasoningText },
        }
      }, {
        latestReasoningText: mergeLatestReasoning(base, event.payload.node_id, event.payload.delta),
        latestRunningNodeId: event.payload.node_id,
      })

    case 'model_call.completed':
      return updateNode(base, event.payload.node_id, (node) => {
        if (node.detail.type !== 'model_call') {
          return node
        }

        return {
          ...node,
          status: 'succeeded',
          completedAt: event.timestamp,
          durationMs: event.payload.duration_ms,
          subtitle: node.detail.reasoningText ? truncate(node.detail.reasoningText, 180) : '完成',
          detail: {
            ...node.detail,
            response: event.payload.response,
            usage: event.payload.usage,
            finishReason: event.payload.finish_reason,
          },
        }
      }, { latestRunningNodeId: null })

    case 'model_output.started': {
      const detail: ModelOutputDetail = {
        type: 'model_output',
        kind: event.payload.kind,
        content: '',
        toolCalls: [],
      }
      const node = createNode({
        id: event.payload.node_id,
        taskId: event.task_id,
        parentId: event.payload.parent_model_call_id,
        type: 'model_output',
        status: 'running',
        title: modelOutputTitle(event.payload.kind),
        subtitle: modelOutputSubtitle(detail),
        round: null,
        timestamp: event.timestamp,
        detail,
      })

      const nextState = upsertNode({
        state: {
          ...base,
          latestRunningNodeId: event.payload.node_id,
          selectedNodeId: base.selectedNodeId ?? event.payload.node_id,
        },
        node,
        parentId: event.payload.parent_model_call_id,
      })

      return event.payload.kind === 'tool_calls'
        ? mergeSiblingContentOutputIntoToolOutput(nextState, event.payload.node_id)
        : nextState
    }

    case 'model_output.delta': {
      if (event.payload.kind === 'final_answer') {
        const payload = event.payload
        return updateNode(base, event.payload.node_id, (node) => {
          if (node.detail.type !== 'model_output') {
            return node
          }

          const content = `${node.detail.content}${payload.content_delta}`
          const detail: ModelOutputDetail = { ...node.detail, content }
          return {
            ...node,
            subtitle: modelOutputSubtitle(detail),
            detail,
          }
        }, { latestRunningNodeId: event.payload.node_id })
      }

      const payload = event.payload
      const nextState = updateNode(base, event.payload.node_id, (node) => {
        if (node.detail.type !== 'model_output') {
          return node
        }

        const toolCalls = mergeToolPreview(node.detail.toolCalls, {
          nodeId: `tool_pending_${payload.tool_call.index}`,
          toolCallId: payload.tool_call.tool_call_id ?? `pending_${payload.tool_call.index}`,
          name: payload.tool_call.name ?? `工具 ${payload.tool_call.index + 1}`,
          arguments: textSnapshot(payload.tool_call.arguments_delta ?? ''),
        })
        const detail: ModelOutputDetail = { ...node.detail, toolCalls }
        return {
          ...node,
          subtitle: modelOutputSubtitle(detail),
          detail,
        }
      }, { latestRunningNodeId: event.payload.node_id })

      return mergeSiblingContentOutputIntoToolOutput(nextState, event.payload.node_id)
    }

    case 'model_output.completed': {
      const nextState = updateNode(base, event.payload.node_id, (node) => {
        if (node.detail.type !== 'model_output') {
          return node
        }

        const completedToolCalls = event.payload.tool_calls.map((toolCall) => ({
          nodeId: findToolNodeId(base, toolCall.tool_call_id) ?? `tool_pending_${toolCall.index}`,
          toolCallId: toolCall.tool_call_id,
          name: toolCall.name,
          arguments: toolCall.arguments,
        }))
        const toolCalls = completedToolCalls.length > 0 ? completedToolCalls : node.detail.toolCalls
        const detail: ModelOutputDetail = {
          ...node.detail,
          kind: event.payload.kind,
          content: event.payload.content,
          toolCalls,
        }
        return {
          ...node,
          status: 'succeeded',
          completedAt: event.timestamp,
          title: modelOutputTitle(event.payload.kind),
          subtitle: modelOutputSubtitle(detail),
          detail,
        }
      }, { latestRunningNodeId: null })

      return event.payload.kind === 'tool_calls'
        ? mergeSiblingContentOutputIntoToolOutput(nextState, event.payload.node_id)
        : nextState
    }

    case 'tool_call.started': {
      const detail: ToolCallDetail = {
        type: 'tool_call',
        toolCallId: event.payload.tool_call_id,
        name: event.payload.name,
        arguments: event.payload.arguments,
        output: null,
        error: null,
      }
      const node = createNode({
        id: event.payload.node_id,
        taskId: event.task_id,
        parentId: event.payload.parent_model_output_id,
        type: 'tool_call',
        status: 'running',
        title: `工具调用 ${event.payload.index + 1}：${event.payload.name}`,
        subtitle: '正在执行',
        round: null,
        timestamp: event.timestamp,
        detail,
      })

      const withNode = upsertNode({
        state: {
          ...base,
          latestRunningNodeId: event.payload.node_id,
          selectedNodeId: base.selectedNodeId ?? event.payload.node_id,
        },
        node,
        parentId: event.payload.parent_model_output_id,
      })

      return updateNode(withNode, event.payload.parent_model_output_id, (parent) => {
        if (parent.detail.type !== 'model_output') {
          return parent
        }

        const toolCalls = mergeToolPreview(parent.detail.toolCalls, {
          nodeId: event.payload.node_id,
          toolCallId: event.payload.tool_call_id,
          name: event.payload.name,
          arguments: event.payload.arguments,
        }, `tool_pending_${event.payload.index}`)
        return {
          ...parent,
          subtitle: toolSummary(toolCalls),
          detail: { ...parent.detail, toolCalls },
        }
      })
    }

    case 'tool_call.completed':
      return updateNode(base, event.payload.node_id, (node) => {
        if (node.detail.type !== 'tool_call') {
          return node
        }

        return {
          ...node,
          status: 'succeeded',
          completedAt: event.timestamp,
          durationMs: event.payload.duration_ms,
          subtitle: snapshotSummary(event.payload.output),
          detail: { ...node.detail, output: event.payload.output, error: null },
        }
      }, { latestRunningNodeId: null })

    case 'tool_call.failed':
      return updateNode(base, event.payload.node_id, (node) => {
        if (node.detail.type !== 'tool_call') {
          return node
        }

        return {
          ...node,
          status: 'failed',
          completedAt: event.timestamp,
          durationMs: event.payload.duration_ms,
          subtitle: event.payload.error,
          detail: { ...node.detail, error: event.payload.error },
        }
      }, { latestRunningNodeId: null })
  }
}

function createNode(input: {
  id: string
  taskId: string
  parentId: string | null
  type: TraceNode['type']
  status: TraceNode['status']
  title: string
  subtitle: string
  round: number | null
  timestamp: string
  detail: TraceNode['detail']
}): TraceNode {
  return {
    id: input.id,
    taskId: input.taskId,
    parentId: input.parentId,
    type: input.type,
    status: input.status,
    title: input.title,
    subtitle: input.subtitle,
    round: input.round,
    startedAt: input.timestamp,
    completedAt: null,
    durationMs: null,
    childrenIds: [],
    detail: input.detail,
  }
}

function upsertNode(input: {
  state: TraceState
  node: TraceNode
  parentId?: string
  root?: boolean
}): TraceState {
  const existing = input.state.nodesById[input.node.id]
  const node = existing ? mergeNode(existing, input.node) : input.node
  const state: TraceState = {
    ...input.state,
    nodesById: {
      ...input.state.nodesById,
      [input.node.id]: node,
    },
  }

  if (input.root) {
    return {
      ...state,
      rootNodeIds: appendUnique(state.rootNodeIds, input.node.id),
    }
  }

  if (input.parentId) {
    return addChild(state, input.parentId, input.node.id)
  }

  return state
}

function mergeNode(existing: TraceNode, incoming: TraceNode): TraceNode {
  return {
    ...existing,
    ...incoming,
    childrenIds: existing.childrenIds,
    detail: { ...existing.detail, ...incoming.detail } as TraceNode['detail'],
  }
}

function addChild(state: TraceState, parentId: string, childId: string): TraceState {
  return updateNode(state, parentId, (parent) => ({
    ...parent,
    childrenIds: appendUnique(parent.childrenIds, childId),
  }))
}

function updateNode(
  state: TraceState,
  nodeId: string,
  updater: (node: TraceNode) => TraceNode,
  patch: Partial<TraceState> = {},
): TraceState {
  const node = state.nodesById[nodeId]
  if (!node) {
    return { ...state, ...patch }
  }

  return {
    ...state,
    ...patch,
    nodesById: {
      ...state.nodesById,
      [nodeId]: updater(node),
    },
  }
}

function mergeLatestReasoning(state: TraceState, nodeId: string, delta: string): string {
  const node = state.nodesById[nodeId]
  if (node?.detail.type === 'model_call') {
    return `${node.detail.reasoningText}${delta}`
  }
  return `${state.latestReasoningText}${delta}`
}

function mergeToolPreview(
  existing: ToolCallPreview[],
  incoming: ToolCallPreview,
  pendingNodeId?: string,
): ToolCallPreview[] {
  const index = existing.findIndex(
    (toolCall) =>
      toolCall.nodeId === incoming.nodeId ||
      toolCall.nodeId === pendingNodeId ||
      toolCall.toolCallId === incoming.toolCallId,
  )

  if (index === -1) {
    return [...existing, incoming]
  }

  return existing.map((toolCall, currentIndex) =>
    currentIndex === index
      ? {
          ...toolCall,
          ...incoming,
          arguments:
            toolCall.nodeId.startsWith('tool_pending_') && incoming.arguments.text
              ? incoming.arguments
              : mergeSnapshots(toolCall.arguments, incoming.arguments),
        }
      : toolCall,
  )
}

function findToolNodeId(state: TraceState, toolCallId: string): string | null {
  const node = Object.values(state.nodesById).find(
    (candidate) =>
      candidate.detail.type === 'tool_call' && candidate.detail.toolCallId === toolCallId,
  )
  return node?.id ?? null
}

function selectFinalOutputNode(state: TraceState): string | null {
  const nodes = Object.values(state.nodesById)
  for (let index = nodes.length - 1; index >= 0; index -= 1) {
    const node = nodes[index]
    if (node.detail.type === 'model_output' && node.detail.kind === 'final_answer') {
      return node.id
    }
  }
  return null
}

function mergeSiblingContentOutputIntoToolOutput(
  state: TraceState,
  toolOutputId: string,
): TraceState {
  const toolOutput = state.nodesById[toolOutputId]
  if (
    !toolOutput ||
    toolOutput.detail.type !== 'model_output' ||
    toolOutput.detail.kind !== 'tool_calls' ||
    !toolOutput.parentId
  ) {
    return state
  }

  const parent = state.nodesById[toolOutput.parentId]
  if (!parent) {
    return state
  }

  const contentOutputId = parent.childrenIds.find((childId) => {
    const child = state.nodesById[childId]
    return (
      childId !== toolOutputId &&
      child?.detail.type === 'model_output' &&
      child.detail.kind === 'final_answer'
    )
  })

  if (!contentOutputId) {
    return state
  }

  const contentOutput = state.nodesById[contentOutputId]
  if (!contentOutput || contentOutput.detail.type !== 'model_output') {
    return state
  }

  const content = toolOutput.detail.content || contentOutput.detail.content
  const detail: ModelOutputDetail = { ...toolOutput.detail, content }
  const nodesById = {
    ...state.nodesById,
    [parent.id]: {
      ...parent,
      childrenIds: parent.childrenIds.filter((childId) => childId !== contentOutputId),
    },
    [toolOutputId]: {
      ...toolOutput,
      title: modelOutputTitle('tool_calls'),
      subtitle: modelOutputSubtitle(detail),
      detail,
    },
  }
  delete nodesById[contentOutputId]

  return {
    ...state,
    nodesById,
    selectedNodeId:
      state.selectedNodeId === contentOutputId ? toolOutputId : state.selectedNodeId,
    latestRunningNodeId:
      state.latestRunningNodeId === contentOutputId ? toolOutputId : state.latestRunningNodeId,
  }
}

function completeRunningFinalOutput(
  state: TraceState,
  finalAnswer: string,
  completedAt: string,
): TraceState {
  const entry = Object.entries(state.nodesById).find(([, node]) => {
    return (
      node.status === 'running' &&
      node.detail.type === 'model_output' &&
      node.detail.kind === 'final_answer'
    )
  })

  if (!entry) {
    return state
  }

  const [nodeId, node] = entry
  return {
    ...state,
    selectedNodeId: state.selectedNodeId ?? nodeId,
    nodesById: {
      ...state.nodesById,
      [nodeId]: {
        ...node,
        status: 'succeeded',
        completedAt,
        subtitle: truncate(finalAnswer, 180),
        detail:
          node.detail.type === 'model_output'
            ? { ...node.detail, content: finalAnswer }
            : node.detail,
      },
    },
  }
}

function modelOutputTitle(kind: ModelOutputDetail['kind']): string {
  return kind === 'tool_calls' ? '工具调用' : '生成结果'
}

function modelOutputSubtitle(detail: ModelOutputDetail): string {
  if (detail.kind === 'tool_calls') {
    return truncate(detail.content, 180) || toolSummary(detail.toolCalls)
  }

  return truncate(detail.content, 180) || '正在生成回复'
}

function mergeSnapshots(existing: JsonSnapshot, incoming: JsonSnapshot): JsonSnapshot {
  if (!existing.text) {
    return incoming
  }

  if (!incoming.text) {
    return existing
  }

  return {
    value: incoming.value,
    text: `${existing.text}${incoming.text}`,
    truncated: existing.truncated || incoming.truncated,
  }
}

function textSnapshot(text: string): JsonSnapshot {
  return {
    value: text,
    text,
    truncated: false,
  }
}

function toolSummary(toolCalls: ToolCallPreview[]): string {
  if (toolCalls.length === 0) {
    return '准备调用工具'
  }

  const names = toolCalls.map((toolCall) => toolCall.name).join('、')
  return `准备调用 ${toolCalls.length} 个工具：${names}`
}

function snapshotSummary(snapshot: JsonSnapshot): string {
  if (typeof snapshot.value === 'string') {
    return truncate(snapshot.value, 180)
  }
  return truncate(snapshot.text, 180)
}

function truncate(value: string, maxLength: number): string {
  if (value.length <= maxLength) {
    return value
  }
  return `${value.slice(0, maxLength - 1)}…`
}

function appendUnique(values: string[], value: string): string[] {
  return values.includes(value) ? values : [...values, value]
}
