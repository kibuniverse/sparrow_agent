# TanStack Router Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hand-rolled router with TanStack Router (file-based, auto code splitting), manage shared state with Zustand, fix base path, and add 404 handling.

**Architecture:** TanStack Router v1 with Vite plugin (`autoCodeSplitting: true`) generates a route tree from `src/routes/` files. Zustand store holds all shared state previously in `App.tsx`. Route files are thin wrappers around updated page components. Each page uses its own `useTaskStream` hook for streaming.

**Tech Stack:** `@tanstack/react-router`, `@tanstack/router-plugin`, `zustand`, React 19, Vite 8

---

### Task 1: Install dependencies and configure Vite

**Files:**
- Modify: `vite.config.ts`
- Modify: `package.json` (via npm install)

- [ ] **Step 1: Install packages**

```bash
cd /Users/yankaizhi/RustProjects/sparrow_agent/frontend
pnpm install @tanstack/react-router zustand
pnpm install -D @tanstack/router-plugin
```

- [ ] **Step 2: Update `vite.config.ts`**

Replace the entire file:

```ts
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { TanStackRouterVite } from '@tanstack/router-plugin/vite'

export default defineConfig({
  base: '/sparrow_agent/',
  plugins: [
    TanStackRouterVite({ autoCodeSplitting: true }),
    react(),
    tailwindcss(),
  ],
  server: {
    proxy: {
      '/api': 'http://127.0.0.1:8787',
    },
  },
})
```

- [ ] **Step 3: Commit**

```bash
git add package.json package-lock.json vite.config.ts
git commit -m "chore: install tanstack-router, router-plugin, and zustand"
```

---

### Task 2: Create Zustand store

**Files:**
- Create: `src/store.ts`

This replaces all shared state in `App.tsx` (traceState, conversationId, messages, completedTaskIds, and all callbacks). The store also defines and exports the `ChatMessage` type.

- [ ] **Step 1: Create `src/store.ts`**

```ts
import { create } from 'zustand'
import { createAgentTask } from './api/agentTrace'
import {
  applyTraceEvent,
  applyTraceSnapshot,
  createInitialTraceState,
  type TraceState,
} from './state/traceReducer'
import type { TaskSnapshot, TraceArchive, TraceEvent } from './types/trace'

export interface ChatMessage {
  id: string
  role: 'user' | 'assistant'
  content: string
}

interface AppStore {
  traceState: TraceState
  conversationId: string | null
  messages: ChatMessage[]
  completedTaskIds: Set<string>

  selectNode: (nodeId: string) => void
  handleTraceEvent: (event: TraceEvent) => void
  applySnapshot: (snapshot: TaskSnapshot) => void
  applyArchive: (archive: TraceArchive) => void
  submitMessage: (message: string) => Promise<void>
  resetTrace: () => void
}

export const useAppStore = create<AppStore>((set, get) => ({
  traceState: createInitialTraceState(),
  conversationId: null,
  messages: [],
  completedTaskIds: new Set(),

  selectNode: (nodeId) =>
    set((state) => ({
      traceState: {
        ...state.traceState,
        selectedNodeId: state.traceState.nodesById[nodeId]
          ? nodeId
          : state.traceState.selectedNodeId,
      },
    })),

  handleTraceEvent: (event) =>
    set((state) => {
      const newTraceState = applyTraceEvent(state.traceState, event)
      if (
        event.type === 'task.completed' &&
        !state.completedTaskIds.has(event.task_id)
      ) {
        const newCompleted = new Set(state.completedTaskIds)
        newCompleted.add(event.task_id)
        return {
          traceState: newTraceState,
          completedTaskIds: newCompleted,
          messages: [
            ...state.messages,
            {
              id: `${event.task_id}-assistant`,
              role: 'assistant' as const,
              content: event.payload.final_answer,
            },
          ],
        }
      }
      return { traceState: newTraceState }
    }),

  applySnapshot: (snapshot) =>
    set((state) => ({
      traceState: applyTraceSnapshot(state.traceState, snapshot),
      conversationId: snapshot.conversation_id,
    })),

  applyArchive: (archive) =>
    set({
      traceState: applyTraceSnapshot(
        createInitialTraceState(),
        archive.task,
      ),
      conversationId: archive.task.conversation_id,
    }),

  submitMessage: async (message) => {
    const clientMessageId = createClientId('msg')
    set((state) => ({
      messages: [
        ...state.messages,
        { id: clientMessageId, role: 'user' as const, content: message },
      ],
    }))
    const response = await createAgentTask({
      conversation_id: get().conversationId,
      client_message_id: clientMessageId,
      message,
      stream: true,
    })
    set({
      conversationId: response.conversation_id,
      traceState: {
        ...createInitialTraceState(),
        taskId: response.task_id,
        conversationId: response.conversation_id,
        status: 'running',
      },
    })
  },

  resetTrace: () =>
    set({
      traceState: createInitialTraceState(),
    }),
}))

function createClientId(prefix: string): string {
  if ('randomUUID' in crypto) {
    return `${prefix}_${crypto.randomUUID()}`
  }
  return `${prefix}_${Date.now()}_${Math.random().toString(16).slice(2)}`
}
```

- [ ] **Step 2: Commit**

```bash
git add src/store.ts
git commit -m "feat: create Zustand store with shared app state"
```

---

### Task 3: Create root route

**Files:**
- Create: `src/routes/__root.tsx`

- [ ] **Step 1: Create `src/routes/` directory and `__root.tsx`**

```bash
mkdir -p src/routes
```

Create `src/routes/__root.tsx`:

```tsx
import { createRootRoute, Link, Outlet } from '@tanstack/react-router'

const rootRoute = createRootRoute({
  component: RootLayout,
  defaultNotFoundComponent: () => (
    <div className="flex min-h-dvh flex-col items-center justify-center gap-4">
      <h1 className="text-2xl font-semibold text-slate-950">404</h1>
      <p className="text-sm text-slate-600">页面不存在</p>
      <Link
        to="/"
        className="text-sm text-blue-600 hover:text-blue-800 hover:underline"
      >
        返回首页
      </Link>
    </div>
  ),
})

function RootLayout() {
  return <Outlet />
}

export { rootRoute }
```

- [ ] **Step 2: Commit**

```bash
git add src/routes/__root.tsx
git commit -m "feat: add TanStack Router root route with 404 handling"
```

---

### Task 4: Update ChatPage and create chat route

**Files:**
- Modify: `src/pages/ChatPage.tsx`
- Create: `src/routes/index.lazy.tsx`

ChatPage now reads state from the Zustand store, uses TanStack Router's `useNavigate` for all navigation, and includes its own `useTaskStream` call for SSE streaming.

- [ ] **Step 1: Update `src/pages/ChatPage.tsx`**

Replace the entire file:

```tsx
import { useNavigate } from '@tanstack/react-router'
import { ChatComposer } from '../components/ChatComposer'
import { ThinkingPreview } from '../components/ThinkingPreview'
import { useTaskStream } from '../hooks/useTaskStream'
import { useAppStore } from '../store'

export function ChatPage() {
  const navigate = useNavigate()
  const traceState = useAppStore((s) => s.traceState)
  const messages = useAppStore((s) => s.messages)
  const submitMessage = useAppStore((s) => s.submitMessage)
  const handleTraceEvent = useAppStore((s) => s.handleTraceEvent)
  const resetTrace = useAppStore((s) => s.resetTrace)

  const running = traceState.status === 'running'

  useTaskStream({
    taskId: traceState.taskId,
    enabled: running && Boolean(traceState.taskId),
    lastSeq: traceState.lastSeq,
    onEvent: handleTraceEvent,
  })

  return (
    <main className="min-h-dvh bg-slate-50">
      <div className="mx-auto flex min-h-dvh w-full max-w-5xl flex-col px-4 py-6 sm:px-6">
        <section className="flex-1 space-y-3 overflow-y-auto pb-6">
          {messages.length === 0 ? (
            <div className="flex min-h-[45vh] flex-col items-center justify-center gap-4 text-center">
              <h1 className="text-2xl font-semibold text-slate-950">Agent Trace</h1>
              <button
                className="text-sm text-blue-600 hover:text-blue-800 hover:underline"
                onClick={() => navigate({ to: '/upload' })}
                type="button"
              >
                上传 Trace 文件预览
              </button>
            </div>
          ) : (
            messages.map((message) => (
              <article
                className={`max-w-[80%] rounded-md px-4 py-3 text-sm leading-6 ${
                  message.role === 'user'
                    ? 'ml-auto bg-slate-950 text-white'
                    : 'mr-auto border border-slate-300 bg-white text-slate-800'
                }`}
                key={message.id}
              >
                {message.content}
              </article>
            ))
          )}
        </section>
        <div className="pb-4">
          <ChatComposer disabled={running} onSubmit={submitMessage} />
          <ThinkingPreview
            onOpenDetail={() => {
              if (traceState.taskId) {
                resetTrace()
                navigate({ to: '/tasks/$taskId', params: { taskId: traceState.taskId } })
              }
            }}
            state={traceState}
          />
        </div>
      </div>
    </main>
  )
}
```

- [ ] **Step 2: Create `src/routes/index.lazy.tsx`**

```tsx
import { createLazyFileRoute } from '@tanstack/react-router'
import { ChatPage } from '../pages/ChatPage'

export const Route = createLazyFileRoute('/')({
  component: ChatPage,
})
```

- [ ] **Step 3: Commit**

```bash
git add src/pages/ChatPage.tsx src/routes/index.lazy.tsx
git commit -m "feat: update ChatPage to use Zustand store and TanStack Router"
```

---

### Task 5: Update TaskDetailPage and create task route

**Files:**
- Modify: `src/pages/TaskDetailPage.tsx`
- Create: `src/routes/tasks/$taskId.lazy.tsx`

TaskDetailPage reads from the store, includes its own `useTaskStream`, and uses TanStack Router navigation.

- [ ] **Step 1: Update `src/pages/TaskDetailPage.tsx`**

Replace the entire file:

```tsx
import { useEffect, useMemo, useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { AgentTraceApiError, getTaskSnapshot } from '../api/agentTrace'
import { LoadingInline } from '../components/LoadingInline'
import { TraceDetailPanel } from '../components/TraceDetailPanel'
import { TraceTimeline } from '../components/TraceTimeline'
import { useTaskStream } from '../hooks/useTaskStream'
import { useAppStore } from '../store'

interface TaskDetailPageProps {
  taskId: string
}

export function TaskDetailPage({ taskId }: TaskDetailPageProps) {
  const navigate = useNavigate()
  const traceState = useAppStore((s) => s.traceState)
  const applySnapshot = useAppStore((s) => s.applySnapshot)
  const selectNode = useAppStore((s) => s.selectNode)
  const resetTrace = useAppStore((s) => s.resetTrace)
  const handleTraceEvent = useAppStore((s) => s.handleTraceEvent)

  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false

    getTaskSnapshot(taskId)
      .then((snapshot) => {
        if (!cancelled) {
          applySnapshot(snapshot)
        }
      })
      .catch((caught: unknown) => {
        if (!cancelled) {
          setError(readSnapshotError(caught))
        }
      })
      .finally(() => {
        if (!cancelled) {
          setIsLoading(false)
        }
      })

    return () => {
      cancelled = true
    }
  }, [applySnapshot, taskId])

  useEffect(() => {
    if (traceState.selectedNodeId || traceState.rootNodeIds.length === 0) {
      return
    }

    const fallback = traceState.latestRunningNodeId ?? traceState.rootNodeIds.at(-1)
    if (fallback) {
      selectNode(fallback)
    }
  }, [selectNode, traceState.latestRunningNodeId, traceState.rootNodeIds, traceState.selectedNodeId])

  useTaskStream({
    taskId,
    enabled: traceState.status === 'running',
    lastSeq: traceState.lastSeq,
    onEvent: handleTraceEvent,
  })

  const selectedNode = useMemo(
    () => (traceState.selectedNodeId ? traceState.nodesById[traceState.selectedNodeId] ?? null : null),
    [traceState.nodesById, traceState.selectedNodeId],
  )

  const openTask = (childTaskId: string) => {
    resetTrace()
    navigate({ to: '/tasks/$taskId', params: { taskId: childTaskId } })
  }

  return (
    <main className="min-h-dvh bg-slate-50">
      <div className="mx-auto w-full max-w-7xl px-4 py-6 sm:px-6">
        <div className="mb-5 flex flex-wrap items-center justify-between gap-3 border-b border-slate-300 pb-4">
          <div>
            <h1 className="text-2xl font-semibold text-slate-950">任务详情</h1>
            <p className="mt-1 text-sm text-slate-600">
              {traceState.status === 'running' ? 'running' : traceState.status} · {traceState.startedAt ?? taskId}
            </p>
          </div>
          <button
            className="inline-flex h-9 items-center rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700 transition hover:border-sky-500 hover:text-sky-700"
            onClick={() => {
              resetTrace()
              navigate({ to: '/' })
            }}
            type="button"
          >
            返回聊天
          </button>
        </div>

        {isLoading ? (
          <div className="py-8">
            <LoadingInline label="正在加载任务快照" />
          </div>
        ) : null}
        {error ? (
          <div className="rounded-md border border-red-300 bg-red-50 p-4 text-sm text-red-700">
            {error}
          </div>
        ) : null}

        <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_24rem]">
          <TraceTimeline onSelectNode={selectNode} state={traceState} />
          <TraceDetailPanel
            className="lg:sticky lg:top-24 lg:max-h-[calc(100dvh-7rem)] lg:overflow-auto"
            node={selectedNode}
            onOpenTask={openTask}
          />
        </div>
      </div>
    </main>
  )
}

function readSnapshotError(error: unknown): string {
  if (error instanceof AgentTraceApiError) {
    if (error.status === 404 || error.code === 'task_not_found') {
      return '任务不存在。'
    }
    if (error.status === 410 || error.code === 'task_expired') {
      return '任务已过期。'
    }
    return error.message
  }
  return '无法加载任务快照。'
}
```

- [ ] **Step 2: Create directory and `src/routes/tasks/$taskId.lazy.tsx`**

```bash
mkdir -p src/routes/tasks
```

Create `src/routes/tasks/$taskId.lazy.tsx`:

```tsx
import { createLazyFileRoute } from '@tanstack/react-router'
import { TaskDetailPage } from '../../pages/TaskDetailPage'

function TaskDetailRoute() {
  const { taskId } = Route.useParams()
  return <TaskDetailPage taskId={taskId} />
}

export const Route = createLazyFileRoute('/tasks/$taskId')({
  component: TaskDetailRoute,
})
```

- [ ] **Step 3: Commit**

```bash
git add src/pages/TaskDetailPage.tsx src/routes/tasks/\$taskId.lazy.tsx
git commit -m "feat: update TaskDetailPage to use Zustand store and TanStack Router"
```

---

### Task 6: Update TraceArchivePage and create archive route

**Files:**
- Modify: `src/pages/TraceArchivePage.tsx`
- Create: `src/routes/trace-files/$fileName.lazy.tsx`

- [ ] **Step 1: Update `src/pages/TraceArchivePage.tsx`**

Replace the entire file:

```tsx
import { useEffect, useMemo, useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { AgentTraceApiError, getTraceArchive } from '../api/agentTrace'
import { LoadingInline } from '../components/LoadingInline'
import { TraceDetailPanel } from '../components/TraceDetailPanel'
import { TraceTimeline } from '../components/TraceTimeline'
import { useAppStore } from '../store'
import type { TraceArchive } from '../types/trace'

interface TraceArchivePageProps {
  fileName: string
}

export function TraceArchivePage({ fileName }: TraceArchivePageProps) {
  const navigate = useNavigate()
  const traceState = useAppStore((s) => s.traceState)
  const applyArchive = useAppStore((s) => s.applyArchive)
  const selectNode = useAppStore((s) => s.selectNode)
  const resetTrace = useAppStore((s) => s.resetTrace)

  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    getTraceArchive(fileName)
      .then((archive: TraceArchive) => {
        if (!cancelled) {
          applyArchive(archive)
        }
      })
      .catch((caught: unknown) => {
        if (!cancelled) {
          setError(readArchiveError(caught))
        }
      })
      .finally(() => {
        if (!cancelled) {
          setIsLoading(false)
        }
      })

    return () => {
      cancelled = true
    }
  }, [fileName, applyArchive])

  useEffect(() => {
    if (traceState.selectedNodeId || traceState.rootNodeIds.length === 0) {
      return
    }
    const fallback = traceState.rootNodeIds.at(-1)
    if (fallback) {
      selectNode(fallback)
    }
  }, [selectNode, traceState.rootNodeIds, traceState.selectedNodeId])

  const selectedNode = useMemo(
    () => (traceState.selectedNodeId ? traceState.nodesById[traceState.selectedNodeId] ?? null : null),
    [traceState.nodesById, traceState.selectedNodeId],
  )

  const openTask = (taskId: string) => {
    resetTrace()
    navigate({ to: '/tasks/$taskId', params: { taskId } })
  }

  return (
    <main className="min-h-dvh bg-slate-50">
      <div className="mx-auto w-full max-w-7xl px-4 py-6 sm:px-6">
        <div className="mb-5 flex flex-wrap items-center justify-between gap-3 border-b border-slate-300 pb-4">
          <div>
            <h1 className="text-2xl font-semibold text-slate-950">Trace 预览</h1>
            <p className="mt-1 text-sm text-slate-600">{fileName}</p>
          </div>
          <div className="flex gap-2">
            <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={() => navigate({ to: '/replay/$fileName', params: { fileName } })} type="button">
              回放
            </button>
            <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={() => { resetTrace(); navigate({ to: '/' }) }} type="button">
              返回聊天
            </button>
          </div>
        </div>
        {isLoading ? <LoadingInline label="正在加载 trace 文件" /> : null}
        {error ? <div className="rounded-md border border-red-300 bg-red-50 p-4 text-sm text-red-700">{error}</div> : null}
        <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_24rem]">
          <TraceTimeline onSelectNode={selectNode} state={traceState} />
          <TraceDetailPanel
            className="lg:sticky lg:top-24 lg:max-h-[calc(100dvh-7rem)] lg:overflow-auto"
            node={selectedNode}
            onOpenTask={openTask}
          />
        </div>
      </div>
    </main>
  )
}

function readArchiveError(error: unknown): string {
  if (error instanceof AgentTraceApiError) {
    return error.message
  }
  return '无法加载 trace 文件。'
}
```

- [ ] **Step 2: Create directory and `src/routes/trace-files/$fileName.lazy.tsx`**

```bash
mkdir -p src/routes/trace-files
```

Create `src/routes/trace-files/$fileName.lazy.tsx`:

```tsx
import { createLazyFileRoute } from '@tanstack/react-router'
import { TraceArchivePage } from '../../pages/TraceArchivePage'

function TraceArchiveRoute() {
  const { fileName } = Route.useParams()
  return <TraceArchivePage fileName={decodeURIComponent(fileName)} />
}

export const Route = createLazyFileRoute('/trace-files/$fileName')({
  component: TraceArchiveRoute,
})
```

- [ ] **Step 3: Commit**

```bash
git add src/pages/TraceArchivePage.tsx src/routes/trace-files/\$fileName.lazy.tsx
git commit -m "feat: update TraceArchivePage to use Zustand store and TanStack Router"
```

---

### Task 7: Update TraceReplayPage and create replay route

**Files:**
- Modify: `src/pages/TraceReplayPage.tsx`
- Create: `src/routes/replay/$fileName.lazy.tsx`

TraceReplayPage manages its own local trace state (not the global store) for replay. It only uses TanStack Router for navigation and the store for `resetTrace`.

- [ ] **Step 1: Update `src/pages/TraceReplayPage.tsx`**

Replace the entire file:

```tsx
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { getTraceArchive } from '../api/agentTrace'
import { LoadingInline } from '../components/LoadingInline'
import { TraceDetailPanel } from '../components/TraceDetailPanel'
import { TraceReplayControls } from '../components/TraceReplayControls'
import { TraceTimeline } from '../components/TraceTimeline'
import { useTraceReplay } from '../hooks/useTraceReplay'
import { applyTraceEvent, createInitialTraceState, type TraceState } from '../state/traceReducer'
import { useAppStore } from '../store'
import type { TraceArchive, TraceEvent } from '../types/trace'

interface TraceReplayPageProps {
  fileName: string
}

export function TraceReplayPage({ fileName }: TraceReplayPageProps) {
  const navigate = useNavigate()
  const resetTrace = useAppStore((s) => s.resetTrace)

  const [archive, setArchive] = useState<TraceArchive | null>(null)
  const [state, setState] = useState<TraceState>(() => createInitialTraceState())
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    getTraceArchive(fileName)
      .then((loaded) => {
        if (!cancelled) {
          setArchive(loaded)
          setState(createInitialTraceState())
        }
      })
      .catch(() => {
        if (!cancelled) {
          setError('无法加载 trace 文件。')
        }
      })
    return () => {
      cancelled = true
    }
  }, [fileName])

  const handleReplayEvent = useCallback((event: TraceEvent) => {
    setState((current) => applyTraceEvent(current, event))
  }, [])

  const replay = useTraceReplay({
    events: archive?.task.events ?? [],
    onEvent: handleReplayEvent,
  })

  const restart = useCallback(() => {
    setState(createInitialTraceState())
    replay.restart()
  }, [replay])

  const selectedNode = useMemo(
    () => (state.selectedNodeId ? state.nodesById[state.selectedNodeId] ?? null : null),
    [state.nodesById, state.selectedNodeId],
  )

  const openTask = (taskId: string) => {
    resetTrace()
    navigate({ to: '/tasks/$taskId', params: { taskId } })
  }

  return (
    <main className="min-h-dvh bg-slate-50">
      <div className="mx-auto w-full max-w-7xl px-4 py-6 sm:px-6">
        <div className="mb-5 flex flex-wrap items-center justify-between gap-3">
          <div>
            <h1 className="text-2xl font-semibold text-slate-950">Trace 回放</h1>
            <p className="mt-1 text-sm text-slate-600">{fileName}</p>
          </div>
          <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={() => { resetTrace(); navigate({ to: '/' }) }} type="button">
            返回聊天
          </button>
        </div>
        <TraceReplayControls
          currentIndex={replay.currentIndex}
          isComplete={replay.isComplete}
          isPlaying={replay.isPlaying}
          onPause={replay.pause}
          onPlay={replay.play}
          onRestart={restart}
          onStep={replay.step}
          total={replay.total}
        />
        {!archive && !error ? <LoadingInline label="正在加载 trace 文件" /> : null}
        {error ? <div className="mt-4 rounded-md border border-red-300 bg-red-50 p-4 text-sm text-red-700">{error}</div> : null}
        <div className="mt-5 grid gap-5 lg:grid-cols-[minmax(0,1fr)_24rem]">
          <TraceTimeline onSelectNode={(nodeId) => setState((current) => ({ ...current, selectedNodeId: nodeId }))} state={state} />
          <TraceDetailPanel
            className="lg:sticky lg:top-24 lg:max-h-[calc(100dvh-7rem)] lg:overflow-auto"
            node={selectedNode}
            onOpenTask={openTask}
          />
        </div>
      </div>
    </main>
  )
}
```

- [ ] **Step 2: Create directory and `src/routes/replay/$fileName.lazy.tsx`**

```bash
mkdir -p src/routes/replay
```

Create `src/routes/replay/$fileName.lazy.tsx`:

```tsx
import { createLazyFileRoute } from '@tanstack/react-router'
import { TraceReplayPage } from '../../pages/TraceReplayPage'

function TraceReplayRoute() {
  const { fileName } = Route.useParams()
  return <TraceReplayPage fileName={decodeURIComponent(fileName)} />
}

export const Route = createLazyFileRoute('/replay/$fileName')({
  component: TraceReplayRoute,
})
```

- [ ] **Step 3: Commit**

```bash
git add src/pages/TraceReplayPage.tsx src/routes/replay/\$fileName.lazy.tsx
git commit -m "feat: update TraceReplayPage to use TanStack Router navigation"
```

---

### Task 8: Update TraceUploadPage and create upload route

**Files:**
- Modify: `src/pages/TraceUploadPage.tsx`
- Create: `src/routes/upload.lazy.tsx`

TraceUploadPage manages its own local state (preview/replay modes). Only navigation changes — `onBack` and `onOpenTask` props are removed, replaced by `useNavigate` and `resetTrace` from store.

- [ ] **Step 1: Update `src/pages/TraceUploadPage.tsx`**

Replace the entire file:

```tsx
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
```

- [ ] **Step 2: Create `src/routes/upload.lazy.tsx`**

```tsx
import { createLazyFileRoute } from '@tanstack/react-router'
import { TraceUploadPage } from '../pages/TraceUploadPage'

export const Route = createLazyFileRoute('/upload')({
  component: TraceUploadPage,
})
```

- [ ] **Step 3: Commit**

```bash
git add src/pages/TraceUploadPage.tsx src/routes/upload.lazy.tsx
git commit -m "feat: update TraceUploadPage to use TanStack Router navigation"
```

---

### Task 9: Create router instance, update App.tsx, and delete old router

**Files:**
- Replace: `src/router.ts` (delete old content, create TanStack Router instance)
- Modify: `src/App.tsx`

- [ ] **Step 1: Replace `src/router.ts`**

Replace the entire file with the TanStack Router instance:

```ts
import { createRouter } from '@tanstack/react-router'
import { routeTree } from './routeTree.gen'

const router = createRouter({
  routeTree,
  basepath: '/sparrow_agent',
})

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}

export { router }
```

- [ ] **Step 2: Update `src/App.tsx`**

Replace with minimal RouterProvider wrapper:

```tsx
import { RouterProvider } from '@tanstack/react-router'
import { router } from './router'

function App() {
  return <RouterProvider router={router} />
}

export default App
```

- [ ] **Step 3: Verify route tree was generated**

```bash
ls src/routeTree.gen.ts
```

If the file doesn't exist yet, run the dev server briefly to trigger generation:

```bash
npx vite build 2>&1 | head -20
```

- [ ] **Step 4: Commit**

```bash
git add src/router.ts src/App.tsx src/routeTree.gen.ts
git commit -m "feat: wire up TanStack Router with basepath, simplify App.tsx"
```

---

### Task 10: Update tests

**Files:**
- Modify: `src/App.test.tsx`

Tests render through a TanStack Router `RouterProvider` with memory history instead of directly rendering `<App />`. Route state is set via memory history initial entries instead of `window.history.replaceState`.

- [ ] **Step 1: Update `src/App.test.tsx`**

Key changes from original:
- Add `renderApp(initialPath)` helper that creates a memory history router with `/sparrow_agent` basepath
- Replace `window.history.replaceState` + `render(<App />)` with `renderApp('/path')`
- "查看任务详情" → "查看详情" (ThinkingPreview button label)
- Import `routeTree` from generated file, `createMemoryHistory`/`createRouter`/`RouterProvider` from TanStack Router

Replace the entire file:

```ts
/**
 * @vitest-environment jsdom
 */
import '@testing-library/jest-dom/vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { createMemoryHistory, createRouter, RouterProvider } from '@tanstack/react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { routeTree } from './routeTree.gen'
import type { CreateAgentTaskResponse, TaskSnapshot, TraceEvent } from './types/trace'

class FakeEventSource {
  static instances: FakeEventSource[] = []

  url: string
  closed = false
  private listeners: Record<string, Array<(event: MessageEvent<string>) => void>> = {}

  constructor(url: string) {
    this.url = url
    FakeEventSource.instances.push(this)
  }

  addEventListener(type: string, listener: (event: MessageEvent<string>) => void) {
    this.listeners[type] = [...(this.listeners[type] ?? []), listener]
  }

  close() {
    this.closed = true
  }

  emitTrace(event: TraceEvent) {
    for (const listener of this.listeners.trace ?? []) {
      listener(new MessageEvent('trace', { data: JSON.stringify(event) }))
    }
  }

  onerror: ((event: Event) => void) | null = null

  static lastUrl(): string | null {
    return FakeEventSource.instances.at(-1)?.url ?? null
  }
}

const snapshot = (value: unknown) => ({
  value,
  text: JSON.stringify(value),
  truncated: false,
})

const events: TraceEvent[] = [
  {
    seq: 1,
    task_id: 'task_stream',
    conversation_id: 'conv_stream',
    timestamp: '2026-05-10T01:00:00.000Z',
    type: 'task.started',
    payload: { message: { role: 'user', content: '分析仓库' } },
  },
  {
    seq: 2,
    task_id: 'task_stream',
    conversation_id: 'conv_stream',
    timestamp: '2026-05-10T01:00:01.000Z',
    type: 'model_call.started',
    payload: {
      node_id: 'model_1',
      round: 1,
      model: 'deepseek-chat',
      request: snapshot({ messages: 1 }),
    },
  },
  {
    seq: 3,
    task_id: 'task_stream',
    conversation_id: 'conv_stream',
    timestamp: '2026-05-10T01:00:02.000Z',
    type: 'model_call.reasoning_delta',
    payload: { node_id: 'model_1', delta: '需要先读取仓库入口。' },
  },
  {
    seq: 4,
    task_id: 'task_stream',
    conversation_id: 'conv_stream',
    timestamp: '2026-05-10T01:00:03.000Z',
    type: 'model_output.started',
    payload: {
      node_id: 'output_1',
      parent_model_call_id: 'model_1',
      kind: 'tool_calls',
    },
  },
  {
    seq: 5,
    task_id: 'task_stream',
    conversation_id: 'conv_stream',
    timestamp: '2026-05-10T01:00:04.000Z',
    type: 'model_output.delta',
    payload: {
      node_id: 'output_1',
      kind: 'tool_calls',
      tool_call: {
        index: 0,
        tool_call_id: 'call_1',
        name: 'read_file',
        arguments_delta: '{"path":"Cargo.toml"}',
      },
    },
  },
  {
    seq: 6,
    task_id: 'task_stream',
    conversation_id: 'conv_stream',
    timestamp: '2026-05-10T01:00:05.000Z',
    type: 'tool_call.started',
    payload: {
      node_id: 'tool_1',
      parent_model_output_id: 'output_1',
      index: 0,
      tool_call_id: 'call_1',
      name: 'read_file',
      arguments: snapshot({ path: 'Cargo.toml' }),
    },
  },
  {
    seq: 7,
    task_id: 'task_stream',
    conversation_id: 'conv_stream',
    timestamp: '2026-05-10T01:00:06.000Z',
    type: 'tool_call.completed',
    payload: {
      node_id: 'tool_1',
      duration_ms: 42,
      output: snapshot({ bytes: 120 }),
      child_task_id: 'task_sub_123',
      child_conversation_id: 'conv_sub_123',
    },
  },
]

const childEvents: TraceEvent[] = [
  {
    seq: 1,
    task_id: 'task_sub_123',
    conversation_id: 'conv_sub_123',
    timestamp: '2026-05-10T01:01:00.000Z',
    type: 'model_call.started',
    payload: {
      node_id: 'child_model_1',
      round: 1,
      model: 'deepseek-chat',
      request: snapshot({ messages: 1 }),
    },
  },
  {
    seq: 2,
    task_id: 'task_sub_123',
    conversation_id: 'conv_sub_123',
    timestamp: '2026-05-10T01:01:01.000Z',
    type: 'model_output.started',
    payload: {
      node_id: 'child_output_1',
      parent_model_call_id: 'child_model_1',
      kind: 'final_answer',
    },
  },
  {
    seq: 3,
    task_id: 'task_sub_123',
    conversation_id: 'conv_sub_123',
    timestamp: '2026-05-10T01:01:02.000Z',
    type: 'model_output.completed',
    payload: {
      node_id: 'child_output_1',
      kind: 'final_answer',
      content: 'child trace done',
      tool_calls: [],
    },
  },
  {
    seq: 4,
    task_id: 'task_sub_123',
    conversation_id: 'conv_sub_123',
    timestamp: '2026-05-10T01:01:03.000Z',
    type: 'task.completed',
    payload: { duration_ms: 3000, final_answer: 'child trace done' },
  },
]

const cliEvents: TraceEvent[] = [
  {
    seq: 1,
    task_id: 'task_cli_1',
    conversation_id: 'conv_cli_1',
    timestamp: '2026-05-10T02:00:00.000Z',
    type: 'task.started',
    payload: { message: { role: 'user', content: 'hello from cli' } },
  },
  {
    seq: 2,
    task_id: 'task_cli_1',
    conversation_id: 'conv_cli_1',
    timestamp: '2026-05-10T02:00:01.000Z',
    type: 'model_call.started',
    payload: {
      node_id: 'model_cli_1',
      round: 1,
      model: 'deepseek-chat',
      request: snapshot({ messages: 2 }),
    },
  },
]

const archive = {
  schema_version: 1 as const,
  exported_at: '2026-05-10T02:00:02.000Z',
  source: 'cli',
  task: {
    task_id: 'task_cli_1',
    conversation_id: 'conv_cli_1',
    status: 'succeeded' as const,
    created_at: '2026-05-10T02:00:00.000Z',
    updated_at: '2026-05-10T02:00:02.000Z',
    events: [
      ...cliEvents,
      {
        seq: 3,
        task_id: 'task_cli_1',
        conversation_id: 'conv_cli_1',
        timestamp: '2026-05-10T02:00:02.000Z',
        type: 'task.completed',
        payload: { duration_ms: 2000, final_answer: 'cli done' },
      },
    ],
  },
}

function renderApp(initialPath = '/') {
  const history = createMemoryHistory({
    initialEntries: [`/sparrow_agent${initialPath}`],
  })
  const router = createRouter({
    routeTree,
    history,
    basepath: '/sparrow_agent',
  })
  return render(<RouterProvider router={router} />)
}

describe('App trace visualization', () => {
  beforeEach(() => {
    FakeEventSource.instances = []
    vi.stubGlobal('EventSource', FakeEventSource)
    vi.stubGlobal('fetch', vi.fn(mockFetch))
  })

  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
  })

  it('submits a chat task, shows thinking preview, opens detail, and displays tool output', async () => {
    renderApp('/')

    fireEvent.change(screen.getByLabelText('消息内容'), { target: { value: '分析仓库' } })
    fireEvent.click(screen.getByRole('button', { name: '发送消息' }))

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1))

    for (const event of events) {
      FakeEventSource.instances[0].emitTrace(event)
    }

    expect(await screen.findByText('需要先读取仓库入口。')).toBeInTheDocument()
    expect(screen.getByText('准备调用 1 个工具：read_file')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: '查看详情' }))

    expect(await screen.findByRole('heading', { name: '任务详情' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '查看 工具调用 1：read_file' }))

    expect(screen.getByRole('heading', { name: '工具调用 1：read_file' })).toBeInTheDocument()
    expect(screen.getByText(/"bytes": 120/)).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: '打开子任务 task_sub_123' }))

    expect(await screen.findAllByText('child trace done')).not.toHaveLength(0)
  })

  it('streams a CLI-created task from a direct browser link', async () => {
    renderApp('/tasks/task_cli_1')

    expect(await screen.findByRole('heading', { name: '任务详情' })).toBeInTheDocument()
    await waitFor(() => expect(FakeEventSource.lastUrl()).toBe(
      '/api/agent/tasks/task_cli_1/events?after_seq=2',
    ))
  })

  it('opens a generated trace archive in preview mode', async () => {
    renderApp('/trace-files/task_cli_1.sparrow-trace.json')

    expect(await screen.findByRole('heading', { name: 'Trace 预览' })).toBeInTheDocument()
    expect(screen.getByText('task_cli_1.sparrow-trace.json')).toBeInTheDocument()
  })

  it('opens a generated trace archive in replay mode', async () => {
    renderApp('/replay/task_cli_1.sparrow-trace.json')

    expect(await screen.findByRole('heading', { name: 'Trace 回放' })).toBeInTheDocument()
    expect(await screen.findByRole('button', { name: '播放' })).toBeInTheDocument()
  })
})

async function mockFetch(input: string | URL | Request, init?: RequestInit): Promise<Response> {
  const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url

  if (url === '/api/agent/tasks' && init?.method === 'POST') {
    const response: CreateAgentTaskResponse = {
      task_id: 'task_stream',
      conversation_id: 'conv_stream',
      events_url: '/api/agent/tasks/task_stream/events',
      snapshot_url: '/api/agent/tasks/task_stream',
    }
    return jsonResponse(response, 202)
  }

  if (url === '/api/agent/tasks/task_stream') {
    const response: TaskSnapshot = {
      task_id: 'task_stream',
      conversation_id: 'conv_stream',
      status: 'running',
      created_at: '2026-05-10T01:00:00.000Z',
      updated_at: '2026-05-10T01:00:06.000Z',
      events,
    }
    return jsonResponse(response, 200)
  }

  if (url === '/api/agent/tasks/task_sub_123') {
    const response: TaskSnapshot = {
      task_id: 'task_sub_123',
      conversation_id: 'conv_sub_123',
      status: 'succeeded',
      created_at: '2026-05-10T01:01:00.000Z',
      updated_at: '2026-05-10T01:01:03.000Z',
      events: childEvents,
    }
    return jsonResponse(response, 200)
  }

  if (url === '/api/agent/tasks/task_cli_1') {
    const response: TaskSnapshot = {
      task_id: 'task_cli_1',
      conversation_id: 'conv_cli_1',
      status: 'running',
      created_at: '2026-05-10T02:00:00.000Z',
      updated_at: '2026-05-10T02:00:01.000Z',
      events: cliEvents,
    }
    return jsonResponse(response, 200)
  }

  if (url === '/api/agent/trace-files/task_cli_1.sparrow-trace.json') {
    return jsonResponse(archive, 200)
  }

  return jsonResponse({ error: { code: 'not_found', message: 'Not found', retryable: false } }, 404)
}

function jsonResponse(value: unknown, status: number): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}
```

- [ ] **Step 2: Commit**

```bash
git add src/App.test.tsx
git commit -m "test: update App tests for TanStack Router memory history"
```

---

### Task 11: Build verification and cleanup

- [ ] **Step 1: Run TypeScript type check**

```bash
npx tsc -b
```

Expected: no errors.

- [ ] **Step 2: Run tests**

```bash
npm test
```

Expected: all tests pass.

- [ ] **Step 3: Run production build**

```bash
npm run build
```

Expected: build succeeds with code-split chunks for each lazy route.

- [ ] **Step 4: Final commit if any fixes were needed**

```bash
git add -A
git commit -m "fix: address build and test issues from TanStack Router migration"
```
