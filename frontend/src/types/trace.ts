export type TaskStatus = 'running' | 'succeeded' | 'failed' | 'cancelled'
export type TraceNodeType = 'model_call' | 'model_output' | 'tool_call'
export type TraceStatus = 'pending' | 'running' | 'succeeded' | 'failed'
export type ModelOutputKind = 'tool_calls' | 'final_answer'

export interface TraceEventEnvelope<TType extends string = string, TPayload = unknown> {
  seq: number
  task_id: string
  conversation_id: string
  timestamp: string
  type: TType
  payload: TPayload
}

export type TraceEvent =
  | TraceEventEnvelope<'task.started', TaskStartedPayload>
  | TraceEventEnvelope<'task.completed', TaskCompletedPayload>
  | TraceEventEnvelope<'task.failed', TaskFailedPayload>
  | TraceEventEnvelope<'model_call.started', ModelCallStartedPayload>
  | TraceEventEnvelope<'model_call.reasoning_delta', ModelCallReasoningDeltaPayload>
  | TraceEventEnvelope<'model_call.completed', ModelCallCompletedPayload>
  | TraceEventEnvelope<'model_output.started', ModelOutputStartedPayload>
  | TraceEventEnvelope<'model_output.delta', ModelOutputDeltaPayload>
  | TraceEventEnvelope<'model_output.completed', ModelOutputCompletedPayload>
  | TraceEventEnvelope<'tool_call.started', ToolCallStartedPayload>
  | TraceEventEnvelope<'tool_call.completed', ToolCallCompletedPayload>
  | TraceEventEnvelope<'tool_call.failed', ToolCallFailedPayload>

export interface TraceNode {
  id: string
  taskId: string
  parentId: string | null
  type: TraceNodeType
  status: TraceStatus
  title: string
  subtitle: string
  round: number | null
  startedAt: string | null
  completedAt: string | null
  durationMs: number | null
  childrenIds: string[]
  detail: TraceNodeDetail
}

export type TraceNodeDetail = ModelCallDetail | ModelOutputDetail | ToolCallDetail

export interface ModelCallDetail {
  type: 'model_call'
  model: string
  request: JsonSnapshot
  response: JsonSnapshot | null
  reasoningText: string
  usage: TokenUsage | null
  finishReason: string | null
}

export interface ModelOutputDetail {
  type: 'model_output'
  kind: ModelOutputKind
  content: string
  toolCalls: ToolCallPreview[]
}

export interface ToolCallDetail {
  type: 'tool_call'
  toolCallId: string
  name: string
  arguments: JsonSnapshot
  output: JsonSnapshot | null
  error: string | null
}

export interface JsonSnapshot {
  value: unknown
  text: string
  truncated: boolean
}

export interface ModelToolCallSnapshot {
  id?: string | null
  type?: string | null
  function?: {
    name?: string | null
    arguments?: string | null
  } | null
}

export interface ModelMessageSnapshot {
  role: string
  content?: string | null
  reasoning_content?: string | null
  tool_calls?: ModelToolCallSnapshot[] | null
  tool_call_id?: string | null
}

export interface ModelRequestSnapshotValue {
  model?: string
  message_count?: number
  messages?: ModelMessageSnapshot[]
  tool_count?: number
  thinking?: unknown
  reasoning_effort?: string | null
}

export interface ModelResponseSnapshotValue {
  message?: ModelMessageSnapshot | null
  finish_reason?: string | null
  usage?: TokenUsage | null
  has_content?: boolean
  tool_call_count?: number
}

export interface TokenUsage {
  prompt_tokens: number
  completion_tokens: number
  total_tokens: number
  reasoning_tokens: number
}

export interface ToolCallPreview {
  nodeId: string
  toolCallId: string
  name: string
  arguments: JsonSnapshot
}

export interface TaskStartedPayload {
  message: {
    role: 'user'
    content: string
  }
}

export interface TaskCompletedPayload {
  duration_ms: number
  final_answer: string
}

export interface TaskFailedPayload {
  duration_ms: number
  error: string
}

export interface ModelCallStartedPayload {
  node_id: string
  round: number
  model: string
  request: JsonSnapshot
}

export interface ModelCallReasoningDeltaPayload {
  node_id: string
  delta: string
}

export interface ModelCallCompletedPayload {
  node_id: string
  duration_ms: number
  finish_reason: string | null
  usage: TokenUsage | null
  response: JsonSnapshot
}

export interface ModelOutputStartedPayload {
  node_id: string
  parent_model_call_id: string
  kind: ModelOutputKind
}

export type ModelOutputDeltaPayload =
  | {
      node_id: string
      kind: 'final_answer'
      content_delta: string
    }
  | {
      node_id: string
      kind: 'tool_calls'
      tool_call: {
        index: number
        tool_call_id: string | null
        name: string | null
        arguments_delta: string | null
      }
    }

export interface ModelOutputCompletedPayload {
  node_id: string
  kind: ModelOutputKind
  content: string
  tool_calls: Array<{
    index: number
    tool_call_id: string
    name: string
    arguments: JsonSnapshot
  }>
}

export interface ToolCallStartedPayload {
  node_id: string
  parent_model_output_id: string
  index: number
  tool_call_id: string
  name: string
  arguments: JsonSnapshot
}

export interface ToolCallCompletedPayload {
  node_id: string
  duration_ms: number
  output: JsonSnapshot
}

export interface ToolCallFailedPayload {
  node_id: string
  duration_ms: number
  error: string
}

export interface CreateAgentTaskRequest {
  conversation_id?: string | null
  client_message_id: string
  message: string
  stream: true
}

export interface CreateAgentTaskResponse {
  task_id: string
  conversation_id: string
  events_url: string
  snapshot_url: string
}

export interface TaskSnapshot {
  task_id: string
  conversation_id: string
  status: TaskStatus
  created_at: string
  updated_at: string
  events: TraceEvent[]
}

export interface AgentApiErrorBody {
  error: {
    code: string
    message: string
    retryable: boolean
  }
}

export type TraceArchiveSource = 'cli' | 'server' | 'imported' | string

export interface TraceArchive {
  schema_version: number
  exported_at: string
  source: TraceArchiveSource
  task: TaskSnapshot
}
