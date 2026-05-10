# Sparrow Agent Frontend

这是 Sparrow Agent 的 React/Vite 前端，用于创建 Agent 任务、展示实时 reasoning 预览，并查看模型调用与工具执行 trace。

## 技术栈

- React 19
- TypeScript
- Vite
- Tailwind CSS 4
- Vitest + Testing Library

## 运行

先启动后端：

```bash
cargo run -- --server
```

再启动前端开发服务：

```bash
cd frontend
pnpm install
pnpm dev
```

Vite 会将 `/api` 代理到 `http://127.0.0.1:8787`。如需修改后端地址，请更新 `vite.config.ts`。

## 主要结构

| 路径 | 说明 |
|------|------|
| `src/App.tsx` | 应用状态入口，管理 conversation、messages 和 trace state |
| `src/pages/ChatPage.tsx` | 聊天页，提交用户消息并展示最新任务入口 |
| `src/pages/TaskDetailPage.tsx` | 任务详情页，加载 snapshot 并展示 trace 树 |
| `src/api/agentTrace.ts` | 后端 API client 和错误类型 |
| `src/hooks/useTaskStream.ts` | SSE 订阅、断线重连和 `after_seq` 续传 |
| `src/state/traceReducer.ts` | 将后端 trace events 归并为前端可渲染的节点树 |
| `src/types/trace.ts` | 与后端 trace 契约对应的 TypeScript 类型 |
| `src/components/` | 聊天输入、reasoning 预览、trace timeline 和详情面板 |

## 可用命令

```bash
pnpm dev
pnpm lint
pnpm test
pnpm build
pnpm preview
```

## 后端契约

前端依赖以下后端接口：

| 接口 | 用途 |
|------|------|
| `POST /api/agent/tasks` | 创建 `stream: true` 的 Agent 任务 |
| `GET /api/agent/tasks/:task_id` | 获取任务 snapshot |
| `GET /api/agent/tasks/:task_id/events?after_seq=0` | 订阅 trace SSE 事件 |

SSE 事件名固定为 `trace`，事件体对应 `src/types/trace.ts` 中的 `TraceEvent`。
