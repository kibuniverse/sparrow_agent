# TanStack Router Migration Design

## Problem

The current hand-rolled router (`src/router.ts`) has four issues:

1. **Base path mismatch** — Routes match bare paths (`/`, `/tasks/$id`) but Vite serves under `/sparrow_agent/`, causing path resolution failures.
2. **No 404 handling** — Unmatched paths silently fall through to the chat page.
3. **No code splitting** — All pages are eagerly imported in `App.tsx`.
4. **No route guards** — No mechanism to protect routes or run pre-navigation logic.

## Approach

File-based routing with TanStack Router + Vite plugin (`autoCodeSplitting: true`). Shared state managed by Zustand.

## Directory Structure

```
src/
├── store.ts                 # Zustand store (shared state + actions)
├── router.ts                # TanStack router instance (replaces old router.ts)
├── routes/
│   ├── __root.tsx           # Root layout + 404 component
│   ├── index.lazy.tsx       # / → ChatPage
│   ├── upload.lazy.tsx      # /upload → TraceUploadPage
│   ├── tasks/
│   │   └── $taskId.lazy.tsx # /tasks/$taskId → TaskDetailPage
│   ├── trace-files/
│   │   └── $fileName.lazy.tsx # /trace-files/$fileName → TraceArchivePage
│   └── replay/
│       └── $fileName.lazy.tsx # /replay/$fileName → TraceReplayPage
├── App.tsx                  # Simplified: router creation + RouterProvider
└── main.tsx                 # Unchanged
```

## Dependencies

- `@tanstack/react-router` — runtime
- `@tanstack/router-plugin` (dev) — Vite plugin for file-based route generation + auto code splitting
- `zustand` — shared state management

## Shared State — Zustand Store

A single Zustand store (`src/store.ts`) holds all shared state and actions currently in `App.tsx`:

**State:**
- `traceState: TraceState`
- `conversationId: string | null`
- `messages: ChatMessage[]`
- `completedTaskIds: Set<string>` (via ` useRef`-like pattern or stored directly)

**Actions:**
- `selectNode(nodeId)` — update selected node in trace state
- `handleTraceEvent(event)` — apply trace event, push assistant message on task completion
- `applySnapshot(snapshot)` — apply task snapshot, set conversation ID
- `applyArchive(archive)` — reset trace state and apply archive
- `submitMessage(message)` — create agent task, update conversation, reset trace state
- `openTask(taskId)` — reset trace state, navigate to task detail
- `resetTrace()` — reset trace state to initial

Router context remains empty. Pages import the store directly via `useAppStore()`.

## Router Configuration

**Vite plugin** (`vite.config.ts`):

```ts
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

**Router instance** (`src/router.ts`):

```ts
import { createRouter } from '@tanstack/react-router'
import { routeTree } from './routeTree.gen'

const router = createRouter({
  routeTree,
  basepath: '/sparrow_agent',
})

export type Router = typeof router
export { router }
```

## 404 Handling

`__root.tsx` sets `defaultNotFoundComponent`:

```tsx
defaultNotFoundComponent: () => (
  <div>
    <h1>404</h1>
    <p>页面不存在</p>
    <Link to="/">返回首页</Link>
  </div>
)
```

Unmatched paths automatically render this component.

## Base Path Fix

`basepath: '/sparrow_agent'` on the router instance ensures all internal path resolution includes the prefix. This aligns with `vite.config.ts`'s `base: '/sparrow_agent/'`. No manual path matching needed.

## Code Splitting

All page components live in `.lazy.tsx` files. The Vite plugin with `autoCodeSplitting: true` generates separate chunks loaded on first navigation. No route-level loaders needed — all data flows through Zustand store or SSE streaming.

## App.tsx Simplification

Current `App.tsx` (~151 lines) shrinks to ~30 lines:

```tsx
import { RouterProvider } from '@tanstack/react-router'
import { router } from './router'

function App() {
  return <RouterProvider router={router} />
}

export default App
```

All state logic moves to `src/store.ts`. All page rendering moves to route files.

## Migration Steps (summary)

1. Install dependencies (`@tanstack/react-router`, `@tanstack/router-plugin`, `zustand`)
2. Update `vite.config.ts` — add TanStack Router Vite plugin
3. Create `src/store.ts` — Zustand store with all shared state and actions
4. Create `src/routes/__root.tsx` — root layout + 404
5. Create lazy route files for each page
6. Create new `src/router.ts` — router instance with `basepath`
7. Simplify `src/App.tsx` — just `RouterProvider`
8. Update `src/main.tsx` if needed
9. Delete old `src/router.ts` (replaced)
10. Update `ChatPage.tsx` — replace `navigateTo` with `useNavigate`
11. Update tests — adapt to TanStack Router's routing model
