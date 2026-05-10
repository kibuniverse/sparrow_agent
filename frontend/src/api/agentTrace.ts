import type {
  AgentApiErrorBody,
  CreateAgentTaskRequest,
  CreateAgentTaskResponse,
  TaskSnapshot,
  TraceArchive,
} from '../types/trace'

export class AgentTraceApiError extends Error {
  code: string
  retryable: boolean
  status: number

  constructor(message: string, code: string, retryable: boolean, status: number) {
    super(message)
    this.name = 'AgentTraceApiError'
    this.code = code
    this.retryable = retryable
    this.status = status
  }
}

export async function createAgentTask(
  request: CreateAgentTaskRequest,
): Promise<CreateAgentTaskResponse> {
  const response = await fetch('/api/agent/tasks', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(request),
  })

  return readJsonResponse<CreateAgentTaskResponse>(response)
}

export async function getTaskSnapshot(taskId: string): Promise<TaskSnapshot> {
  const response = await fetch(`/api/agent/tasks/${encodeURIComponent(taskId)}`)
  return readJsonResponse<TaskSnapshot>(response)
}

export function buildTaskEventsUrl(taskId: string, afterSeq: number): string {
  const url = new URL(`/api/agent/tasks/${encodeURIComponent(taskId)}/events`, window.location.origin)
  url.searchParams.set('after_seq', String(afterSeq))
  return `${url.pathname}${url.search}`
}

export async function getTraceArchive(fileName: string): Promise<TraceArchive> {
  const response = await fetch(`/api/agent/trace-files/${encodeURIComponent(fileName)}`)
  return readJsonResponse<TraceArchive>(response)
}

async function readJsonResponse<T>(response: Response): Promise<T> {
  if (response.ok) {
    return response.json() as Promise<T>
  }

  const body = await readErrorBody(response)
  throw new AgentTraceApiError(
    body.error.message,
    body.error.code,
    body.error.retryable,
    response.status,
  )
}

async function readErrorBody(response: Response): Promise<AgentApiErrorBody> {
  try {
    return (await response.json()) as AgentApiErrorBody
  } catch {
    return {
      error: {
        code: `http_${response.status}`,
        message: response.statusText || 'Agent service request failed.',
        retryable: response.status >= 500,
      },
    }
  }
}
