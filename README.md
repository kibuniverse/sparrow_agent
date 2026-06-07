# Sparrow Agent

Sparrow Agent 是一个 Rust 编写的本地 Agent 实验项目。它以 DeepSeek Chat Completion 为模型后端，支持命令行多轮对话、流式 reasoning 展示、并行工具调用、Tavily Web 搜索、Rust WASM 沙盒执行、可显式启用的 Bash 命令工具、基于 MCP filesystem server 的受控文件系统工具，以及子 Agent 隔离任务委派。项目还包含一个 React/Vite 前端，用于实时查看 Agent 调用链路和工具执行 trace。

## 功能特性

- **多轮对话**：CLI 和 Server 模式都会为会话维护消息历史，支持连续上下文对话。
- **流式输出**：默认启用 DeepSeek SSE 流式调用，可在 CLI 展示 reasoning 与最终回答，在 Server 模式转成结构化 trace。
- **工具调用循环**：模型可连续请求工具，工具结果会回填到消息历史后继续请求模型。
- **并行工具执行**：同一轮模型返回的多个工具调用会并发执行，并在 trace 中独立记录开始、完成和失败事件。
- **可插拔工具提供者**：本地工具、MCP 工具和子 Agent 工具统一实现 `ToolProvider`，由 `ToolRegistry` 汇总定义并分发调用。
- **Web 搜索**：内置 `webSearch`，通过 Tavily API 返回答案、摘要和来源链接。
- **Rust WASM 沙盒执行**：内置 `runRustWasm`，将模型生成的 Rust 代码编译到 `wasm32-unknown-unknown` 并用 wasmtime 隔离执行。
- **Bash 命令工具**：内置 `runBashCommand`，默认启用；仅 CLI Agent 可用，使用智能审批自动放行低风险命令，高风险命令仍会确认或拦截，并限制 cwd、超时、输出和环境变量。
- **子 Agent 任务委派**：内置 `runSubAgentTask`，可在隔离的消息上下文中运行子任务。子 Agent 继承父 Agent 的配置但限制工具集、轮数和深度，结果会结构化返回给父 Agent。
- **MCP 文件系统工具**：默认尝试通过 `npx @modelcontextprotocol/server-filesystem` 接入文件系统工具，支持 roots、只读/读写模式、写入确认和敏感路径 denylist。
- **工具结果后处理**：超过阈值的工具输出会被截断并保存完整内容到本地文件（`.sparrow_agent/tool_outputs`），注入模型的摘要中包含文件路径和截断提示。
- **Agent 调用可视化**：HTTP API + SSE 会推送 task、model call、model output、tool call 等结构化事件，前端可实时展示调用树和详情。
- **Trace 归档与压缩**：CLI 观察模式每轮任务完成后写入 `.sparrow-trace.json` 归档文件，支持 V2 压缩格式（事件合并、请求快照 diff/keyframe 编码），可通过前端回放。
- **安全配置管理**：API 密钥可交互式初始化并保存到本地配置文件，也可由环境变量覆盖。

## 安装

可以直接使用仓库中的 `install.sh` 安装最新 GitHub Release 产物：

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://raw.githubusercontent.com/kibuniverse/sparrow_agent/main/install.sh | bash
```

安装脚本会自动检测当前系统架构，从 GitHub Releases 下载最新版本对应的预编译包，校验 `.sha256` 后安装，并在全局 bin 目录创建两个命令：

- `sparrow_agent`
- `spa`

默认会把版本化的二进制安装到 `~/.sparrow_agent/releases/<tag>/<target>/sparrow_agent`，并优先在 `$CARGO_HOME/bin`、`~/.cargo/bin` 或 `~/.local/bin` 下创建命令链接。安装后可以运行：

```bash
sparrow_agent --help
spa --help
```

如需指定命令链接目录，可设置 `SPARROW_AGENT_BIN_DIR`：

```bash
SPARROW_AGENT_BIN_DIR=/usr/local/bin bash install.sh
```

如果目标目录中已存在非符号链接文件，脚本会停止；确认要覆盖时可设置 `SPARROW_AGENT_FORCE=1`。

## 快速开始

### 依赖准备

- Rust toolchain，项目使用 Rust 2024 edition。
- 如需运行 `runRustWasm`，安装 WASM 编译目标：

```bash
rustup target add wasm32-unknown-unknown
```

- 如需默认 MCP 文件系统工具，确保本机可运行 `npx`。
- 如需前端开发服务，安装 `pnpm`。

### 配置 API Key

首次运行时，如果没有检测到环境变量或配置文件，命令行会提示输入 `DEEPSEEK_API_KEY` 和 `TAVILY_API_KEY`，并保存到 `~/.sparrow_agent/config.json`：

```bash
cargo run
```

也可以通过环境变量直接提供，环境变量优先级高于配置文件：

```bash
export DEEPSEEK_API_KEY=your_deepseek_api_key
export TAVILY_API_KEY=your_tavily_api_key
cargo run
```

启动后输入自然语言问题即可对话，输入 `exit` 或 `quit` 退出。

## CLI 模式

默认模式是命令行对话：

```bash
cargo run
```

CLI 每轮输入前会显示上下文窗口使用情况。当前代码对 `deepseek-v4-flash` 和 `deepseek-v4-pro` 按 1,000,000 token 上下文窗口渲染进度条，其他模型显示未知窗口大小。

流式输出默认开启：

- reasoning 内容按流式增量展示；
- 最终回答按流式增量展示；
- 工具调用参数增量默认不展示，可通过环境变量打开。

### CLI + 浏览器观察模式

如果希望仍然从 CLI 输入任务，但在浏览器里查看每一轮 agent loop，可先构建前端，然后使用观察模式启动：

```bash
cd frontend
pnpm install
pnpm build
cd ..
cargo run -- --inspect
```

也可以使用等价参数：

```bash
cargo run -- --browser-trace
```

观察服务默认监听 `127.0.0.1:8787`，可通过 `SPARROW_INSPECT_ADDR` 覆盖：

```bash
SPARROW_INSPECT_ADDR=127.0.0.1:9797 cargo run -- --inspect
```

每次在 CLI 输入一条消息后，终端会先打印本轮实时任务地址：

```text
inspect> http://127.0.0.1:8787/tasks/task_01...
```

打开该地址即可查看本轮 loop 的模型调用、reasoning 增量、工具调用、工具输出和最终回答。任务完成或失败后，CLI 会将完整 trace 写入 `SPARROW_TRACE_DIR` 指定目录；如果未设置该变量，默认写入运行目录下的 `.sparrow_agent/traces`：

```text
trace> /Users/me/project/.sparrow_agent/traces/task_01....sparrow-trace.json
replay> http://127.0.0.1:8787/replay/task_01....sparrow-trace.json
```

打开 `replay>` 地址可在前端按事件顺序回放完整 trace；将路径中的 `/replay/` 改成 `/trace-files/` 可直接预览最终状态。

## Server 与前端模式

后端服务默认监听 `127.0.0.1:8787`：

```bash
cargo run -- --server
```

可通过 `SPARROW_SERVER_ADDR` 覆盖监听地址：

```bash
SPARROW_SERVER_ADDR=127.0.0.1:8787 cargo run -- --server
```

前端开发服务会把 `/api` 代理到 `http://127.0.0.1:8787`：

```bash
cd frontend
pnpm install
pnpm dev
```

前端能力：

- 聊天页创建流式 Agent 任务；
- 展示最新 reasoning 预览；
- 任务详情页加载 task snapshot；
- 通过 SSE 实时合并 trace 事件；
- 将模型调用、模型输出和工具调用归并为可选中的调用树；
- 支持子 Agent 任务的嵌套 trace 展示；
- Trace 归档文件直接预览（`/trace-files/`）和按事件回放（`/replay/`）；
- EventSource 断线后会按 1s、2s、5s、10s 退避重连，并使用 `after_seq` 续传。

## HTTP API

| 接口 | 说明 |
|------|------|
| `GET /api/health` | 健康检查，返回 `{ "ok": true }` |
| `POST /api/agent/tasks` | 创建流式 Agent 任务，目前仅支持 `stream: true` |
| `GET /api/agent/tasks/:task_id` | 获取任务快照和历史 trace |
| `GET /api/agent/tasks/:task_id/events?after_seq=0` | 订阅 trace SSE 事件，先 replay 历史事件，再推送 live 事件 |
| `GET /api/agent/trace-files/:file_name` | 读取已保存的 trace 归档文件（仅 CLI 观察模式提供） |

创建任务请求：

```json
{
  "conversation_id": null,
  "client_message_id": "msg_...",
  "message": "帮我分析这个项目",
  "stream": true
}
```

同一个 `conversation_id` 同时只能运行一个任务；如果会话忙，接口返回 `409 conversation_busy`。同一个 `client_message_id` 会复用已有任务，避免重复提交。

## 配置

### 环境变量

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `DEEPSEEK_API_KEY` | DeepSeek API 密钥 | 必需，除非配置文件已保存 |
| `TAVILY_API_KEY` | Tavily API 密钥 | 必需，除非配置文件已保存 |
| `SPARROW_CONFIG_PATH` | 自定义配置文件路径 | `~/.sparrow_agent/config.json` |
| `SPARROW_DEBUG` | 启用调试日志，设为任意值开启 | 关闭 |
| `SPARROW_SERVER_ADDR` | Server 模式监听地址 | `127.0.0.1:8787` |
| `SPARROW_INSPECT_ADDR` | CLI 浏览器观察模式监听地址 | `127.0.0.1:8787` |
| `SPARROW_TRACE_DIR` | CLI 观察模式完成后写入 trace 文件的目录 | `<运行目录>/.sparrow_agent/traces` |
| `SPARROW_STREAMING_ENABLED` | 是否启用模型流式调用 | `true` |
| `SPARROW_SHOW_REASONING` | CLI 是否展示 reasoning | `true` |
| `SPARROW_SHOW_TOOL_CALL_DELTAS` | CLI 是否展示工具调用参数增量 | `false` |
| `SPARROW_BASH_ENABLED` | 是否启用 CLI Bash 命令工具 `runBashCommand` | `true` |
| `SPARROW_BASH_ROOTS` | Bash 工具允许使用的 cwd 根目录列表，Unix 用 `:` 分隔，Windows 用 `;` 分隔 | `.` |
| `SPARROW_BASH_APPROVAL_MODE` | Bash 审批模式：`smart` 自动放行低风险命令，`always` 每次确认，`never` 不提示但仍拦截 blocked 命令 | `smart` |
| `SPARROW_BASH_APPROVAL_POLICY_PATH` | 低风险审批策略缓存 JSON 文件位置 | `~/.sparrow_agent/bash_approval_policies.json` |
| `SPARROW_BASH_APPROVAL_POLICY_TTL_DAYS` | 新增低风险策略的默认过期天数 | `90` |
| `SPARROW_BASH_MODEL_LOW_RISK_THRESHOLD` | 灰区命令由模型判为低风险时的最低置信度 | `0.85` |
| `SPARROW_BASH_TIMEOUT_MS` | Bash 工具默认超时毫秒数，上限固定为 `120000` | `30000` |
| `SPARROW_BASH_MAX_COMMAND_CHARS` | Bash 工具单条命令最大字符数 | `8192` |
| `SPARROW_BASH_STREAM_MAX_BYTES` | Bash 工具 stdout/stderr 各自注入前保留的最大字节数 | `8192` |
| `SPARROW_BASH_ENV_ALLOWLIST` | Bash 工具传入子进程的环境变量 allowlist，逗号分隔；名称包含 key/token/secret/password/authorization 的变量仍会被过滤 | `PATH,HOME,USER,TERM,TMPDIR` |
| `SPARROW_FILESYSTEM_ENABLED` | 是否启用 MCP 文件系统工具 | `true` |
| `SPARROW_FILESYSTEM_ROOTS` | 允许访问的根目录列表，Unix 用 `:` 分隔，Windows 用 `;` 分隔 | `.` |
| `SPARROW_FILESYSTEM_MODE` | 文件系统模式：`read-only` 或 `read-write` | `read-write` |
| `SPARROW_FILESYSTEM_CONFIRM` | 确认策略：`never`、`writes`、`always` | `writes` |
| `SPARROW_TOOL_RESULT_MAX_CHARS` | 工具输出注入模型的最大字符数，超出部分截断并保存到文件 | `20000` |
| `SPARROW_TOOL_OUTPUT_DIR` | 工具输出完整内容保存目录 | `.sparrow_agent/tool_outputs` |
| `SPARROW_MCP_FILESYSTEM_COMMAND` | MCP filesystem server 启动命令 | `npx` |
| `SPARROW_MCP_FILESYSTEM_ARGS` | MCP filesystem server 参数，JSON 字符串数组 | `["-y","@modelcontextprotocol/server-filesystem","<当前目录>"]` |
| `SPARROW_SUB_AGENT_ENABLED` | 是否启用子 Agent 工具 `runSubAgentTask` | `true` |
| `SPARROW_SUB_AGENT_MAX_DEPTH` | 子 Agent 最大递归深度（每层减 1） | `1` |
| `SPARROW_SUB_AGENT_MAX_CONCURRENT` | 同一轮最大并发子 Agent 数 | `3` |
| `SPARROW_SUB_AGENT_MAX_TOOL_ROUNDS` | 每个子 Agent 最大工具调用轮数 | `8` |
| `SPARROW_SUB_AGENT_TIMEOUT_MS` | 每个子 Agent 超时毫秒数 | `120000` |
| `SPARROW_SUB_AGENT_ALLOWED_TOOLS` | 子 Agent 默认允许使用的工具列表，逗号分隔 | `webSearch,mcp__filesystem__read_file,mcp__filesystem__search_files` |
| `SPARROW_SUB_AGENT_INHERIT_FILESYSTEM` | 子 Agent 是否继承父 Agent 的文件系统工具 | `true` |
| `SPARROW_SUB_AGENT_INHERIT_BASH` | 子 Agent 是否继承父 Agent 的 Bash 工具 | `false` |

### 默认运行参数

| 配置项 | 默认值 |
|--------|--------|
| 模型 | `deepseek-v4-pro` |
| 系统提示词 | `You are a helpful assistant.` + 运行时文件系统上下文（当前工作目录和可执行文件路径） |
| 推理强度 | `high` |
| 最大工具调用轮数 | `100` |
| 文件系统最大读取字节数 | `262144` |
| 文件系统最大写入字节数 | `262144` |
| 工具输出最大注入字符数 | `20000` |

### 配置文件

配置文件只保存 API key：

```json
{
  "deepseek_api_key": "...",
  "tavily_api_key": "..."
}
```

Unix 系统上文件权限会设置为 `0600`。运行参数目前主要通过环境变量控制。

## 架构

```text
CLI / React Frontend
        |
        v
 main.rs / server.rs
        |
        v
 ConversationStore  ->  Agent
                         |
                         v
                  DeepSeekClient
                         |
               streamed / non-streamed response
                         |
                         v
                   ToolRegistry
                  /        |        \
                 v         v         v
        LocalToolProvider  SubAgentToolProvider  McpToolProvider
          |        |              |                   |
          v        v              v                   v
     webSearch  runRustWasm  runBashCommand      MCP filesystem tools
                            runSubAgentTask
```

核心流程：

1. `main.rs` 加载 `AppConfig`，根据是否传入 `--server` 或 `--inspect` 启动 CLI REPL、CLI 浏览器观察模式或 Axum Server。
2. `Agent` 维护 `ChatMessage` 历史，构造 DeepSeek Chat Completion 请求，并开启 thinking/reasoning 配置。
3. `DeepSeekClient` 负责普通请求和 SSE 流式请求；流式响应由 `StreamAccumulator` 归并为完整 assistant message。
4. 如果 assistant message 包含工具调用，`ToolRegistry` 将调用分发给对应 provider，并行执行同一轮工具。
5. 工具结果经过 `ToolResultProcessor` 截断处理（超出阈值的内容保存到本地文件并注入截断提示）后，作为 `tool` message 追加回历史，Agent 进入下一轮模型请求，直到返回最终文本或达到最大轮数。
6. Server 模式下，`TraceStoreSink` 把关键阶段写入内存 `TraceStore`，前端通过 snapshot 和 SSE 消费这些事件。
7. CLI 观察模式下，每轮任务完成后通过 `trace_file` 模块写入压缩的 `.sparrow-trace.json` 归档文件。

## 子 Agent 系统

`runSubAgentTask` 工具允许父 Agent 将独立子任务委派给隔离的子 Agent 执行。子 Agent 收到明确的 task、context_pack 和 expected_output 后独立运行，父 Agent 仅收到结构化的最终结果。

核心特性：

- **隔离上下文**：子 Agent 拥有独立的消息历史和系统提示词，不依赖父 Agent 的对话上下文；
- **工具限制**：默认仅允许 `webSearch`、`mcp__filesystem__read_file`、`mcp__filesystem__search_files`，可通过 `allowed_tools` 参数按需缩小；
- **递归深度控制**：默认最大深度为 1（子 Agent 不能再创建孙 Agent），每层递减；
- **并发限制**：同一轮最多同时运行 3 个子 Agent，超出等待信号量释放；
- **超时保护**：每个子 Agent 默认 120 秒超时，超时后返回 timeout 状态；
- **结构化结果**：返回 `SubAgentResult`，包含 `status`（succeeded/failed/timeout/rejected）、`final_answer`、`summary`、`child_task_id` 和 `warnings`；
- **Trace 集成**：子 Agent 的 trace 事件会关联到父 Agent 的 trace 树中，前端可通过 `child_task_id` 导航到子任务详情。

子 Agent 失败不会导致父 Agent 任务失败——父 Agent 会收到 failed 状态的 `SubAgentResult` 并可据此决定后续策略。

## Trace 事件模型

后端 trace 事件按 task 递增 `seq`，事件负载中的 JSON 快照会自动脱敏 `api_key`、`token`、`authorization`、`password`、`secret` 等字段，并限制最大快照大小。

主要事件类型：

| 类型 | 说明 |
|------|------|
| `task.started` / `task.completed` / `task.failed` | 任务生命周期 |
| `model_call.started` / `model_call.reasoning_delta` / `model_call.completed` | 模型调用、reasoning 增量和用量信息 |
| `model_output.started` / `model_output.delta` / `model_output.completed` | 最终回答或工具调用输出 |
| `tool_call.started` / `tool_call.completed` / `tool_call.failed` | 单个工具调用生命周期 |

`TraceStore` 默认每个任务最多保留 10,000 个事件，超过限制会将任务标记为 failed。

### Trace 归档与压缩

CLI 观察模式每轮任务完成后会将 trace 序列化为 `.sparrow-trace.json` 归档文件。归档采用 V2 压缩格式：

- **事件合并（Event Run）**：连续的 `reasoning_delta`、`model_output.delta` 和 `tool_call.delta` 事件会被合并为单条事件，记录起止 seq 和合并数量；
- **请求快照编码**：模型的 `request` 快照仅保存完整 keyframe 每 8 次，中间使用 delta frame（基于 hash 校验的 `retain_prefix`/`append`/`replace_range` 操作）；
- **SHA-256 完整性校验**：所有快照编码使用 SHA-256 哈希校验，delta frame 会验证 base hash 和目标 hash。

## 已内置工具

| 工具名 | Provider | 说明 |
|--------|----------|------|
| `webSearch` | local | 使用 Tavily 搜索网页，最多返回 5 条结果和 Tavily answer |
| `runRustWasm` | local | 编译并执行定义了 `pub fn run() -> String` 的 Rust 代码 |
| `runBashCommand` | local | 默认启用在 CLI 中执行单条非交互 Bash 命令，返回结构化 stdout/stderr/exit code/timeout 信息 |
| `runSubAgentTask` | sub-agent | 在隔离消息上下文中运行独立子任务，仅使用显式提供的上下文，返回结构化结果 |
| `mcp__filesystem__*` | MCP | 来自 `@modelcontextprotocol/server-filesystem` 的文件系统工具，具体列表由 MCP server 动态发现 |

## WASM 沙盒

`runRustWasm` 会在临时目录生成一个最小 Rust crate，编译为 `cdylib` WASM 后用 wasmtime 执行。用户代码必须定义：

```rust
pub fn run() -> String {
    "hello from wasm".to_string()
}
```

安全与资源限制：

- **无 WASI**：不注入文件系统、网络、环境变量等宿主接口。
- **Fuel 计量**：初始 fuel 为 `1_000_000`，用于限制长时间运行。
- **编译超时**：`cargo build --release --target wasm32-unknown-unknown` 限制 10 秒。
- **输出限制**：结果最大 64 KiB。
- **错误限制**：编译 stderr 最多保留 16 KiB。
- **内存边界检查**：通过导出的 `result_ptr` / `result_len` 从 WASM memory 读取结果。

## Bash 命令工具安全边界

`runBashCommand` 默认启用，仅在 CLI Agent 中暴露。Server 模式和浏览器观察模式中的 HTTP API 会移除交互式工具；在 `--inspect` 模式下，终端里的 CLI Agent 可以使用 Bash 工具，但旁路的浏览器 API 不会暴露它。

可通过以下方式关闭：

```bash
SPARROW_BASH_ENABLED=false cargo run
```

安全措施：

- 默认 `smart` 审批模式会用本地规则自动批准低风险命令，例如 `pwd`、`ls`、`rg`、`git status`、`cargo check`；
- `always` 模式会恢复逐条确认；`never` 模式不提示确认，但仍会运行 blocked hard rules，不能绕过明显危险命令；
- 中风险命令会在终端展示 risk、reason、cwd、timeout 和完整 command 后等待确认；带低风险候选策略时可输入 `a` 记住相似命令；
- 高风险命令每次都要求确认，且不会写入持久自动批准策略；
- blocked 命令会直接拒绝，例如删除系统关键路径、fork bomb 或明显资源耗尽攻击；
- 每次调用都是独立进程，不保留上一条命令的 cwd、环境或 shell 状态；
- 命令通过非交互 Bash 运行，不读取用户 profile 或 rc 文件；
- cwd 必须存在且位于 `SPARROW_BASH_ROOTS` 内；
- 默认超时 30 秒，模型传入的 `timeout_ms` 会被限制在 120 秒以内；
- stdout 和 stderr 分别按 `SPARROW_BASH_STREAM_MAX_BYTES` 截断，截断时不会切断 UTF-8 字符；
- 子进程默认清空环境，仅传入 allowlist 中的变量，且变量名包含 key、token、secret、password、authorization 的项总会被过滤；
- 低风险策略缓存保存在可读 JSON 文件中，默认路径为 `~/.sparrow_agent/bash_approval_policies.json`，Unix 权限为 `0600`，可手动删除或编辑来撤销策略。

注意：这是一个受控的本地命令执行工具，不是强 OS 沙盒。cwd root 校验用于约束工作目录，不能阻止命令显式访问系统上的其他绝对路径；因此本地 hard rules 会在策略缓存命中后重新运行。

## MCP 文件系统安全边界

文件系统工具默认启用，默认模式是 `read-write`，写入类工具默认需要用户确认。切换到只读模式：

```bash
SPARROW_FILESYSTEM_MODE=read-only cargo run
```

安全措施：

- 只允许访问 `SPARROW_FILESYSTEM_ROOTS` 内的路径；
- 默认拒绝 `.git/**`、`.env`、`.env.*`、私钥、证书、`.sparrow_agent/**` 等敏感路径；
- `read-write` 模式下写入工具默认需要用户确认；用户首次批准后，同一会话内后续写入操作自动放行，不再重复询问；
- `edit_file` 会先执行 dry run，展示 diff 后再次确认才会应用（同样受会话级批准缓存控制）；
- MCP 工具名会命名空间化为 `mcp__{server_id}__{tool_name}`，避免和本地工具重名。

## 模块结构

| 路径 | 说明 |
|------|------|
| `src/main.rs` | 二进制入口，加载配置，启动 CLI REPL、CLI 观察模式或 Server |
| `src/lib.rs` | 库入口，导出项目模块 |
| `src/config.rs` | 应用配置、API key 初始化、所有环境变量解析和子 Agent/tool result 配置 |
| `src/agent.rs` | Agent 编排器，维护消息历史、模型请求、工具循环、trace 转发 |
| `src/client.rs` | DeepSeek HTTP/SSE 客户端 |
| `src/api.rs` | DeepSeek Chat Completion 请求、响应和工具调用数据结构 |
| `src/bash_runner.rs` | Bash 命令执行、cwd 校验、审批接入、超时、输出截断和环境变量过滤 |
| `src/bash_risk.rs` | Bash 命令风险等级、本地规则分类和命令形状规范化 |
| `src/bash_approval_policy.rs` | 低风险 Bash 审批策略缓存、matcher 和持久化 |
| `src/bash_approval_gate.rs` | Bash 智能审批编排，本地规则、策略缓存、模型灰区分类和审批摘要 |
| `src/bash_model_classifier.rs` | DeepSeek 灰区 Bash 风险分类封装 |
| `src/streaming.rs` | 流式响应累积器和 Agent stream event 抽象 |
| `src/tool_provider.rs` | 工具 provider trait，含 `execute` 和 `execute_traced` 接口 |
| `src/tool_registry.rs` | 工具定义汇总、调用分发、并行执行、traced execution 和工具结果后处理 |
| `src/local_tools.rs` | 本地工具 provider，注册 `webSearch`、`runRustWasm` 和 `runBashCommand` |
| `src/tools.rs` | Tavily 搜索和 WASM 工具入口 |
| `src/rust_wasm_runner.rs` | Rust 到 WASM 的编译与 wasmtime 沙盒运行 |
| `src/sub_agent.rs` | 子 Agent 工具 provider、协调器、运行器和结构化结果模型 |
| `src/tool_result_processor.rs` | 工具输出截断、完整内容保存和截断提示注入 |
| `src/mcp/` | MCP stdio transport、JSON-RPC protocol、client 和 filesystem provider |
| `src/server.rs` | Axum HTTP API、SSE、CORS、trace 文件读取和任务创建逻辑 |
| `src/conversation_store.rs` | Server 模式下按 conversation 复用 Agent，并限制同会话并发任务 |
| `src/trace.rs` | Trace 事件类型、JSON 快照、截断和敏感字段脱敏 |
| `src/trace_store.rs` | 内存 task/event 存储、seq 管理、snapshot、broadcast 和 TraceStoreSink |
| `src/trace_compaction.rs` | Trace 归档 V2 压缩：事件合并、请求快照 diff/keyframe 编码和 SHA-256 校验 |
| `src/trace_file.rs` | Trace 归档文件读写，支持 V1/V2 格式自动识别和展开 |
| `src/cli_observer.rs` | CLI 浏览器观察模式，启动内嵌 HTTP server + 浏览器任务 URL |
| `src/console.rs` | CLI 输入、密钥输入和流式渲染 |
| `src/debug.rs` | 调试日志开关 |
| `frontend/src/` | React 前端，包含聊天页、任务详情页、trace 归档页、trace 回放页、trace reducer 和 SSE hook |
| `tests/` | Server、Trace 和 TraceStore 的契约测试 |
| `docs/` | 功能设计方案和历史实施计划 |

## 测试与检查

根项目：

```bash
cargo fmt --check
cargo check
cargo test
```

前端：

```bash
cd frontend
pnpm lint
pnpm test
pnpm build
```

## 设计文档

- [文件读写能力实现方案](docs/filesystem-capability-implementation-plan.md)
- [上下文窗口治理方案](docs/context-window-management-plan.md)
- [模型思考过程流式展示方案](docs/streaming-thinking-display-plan.md)
- [Agent 调用过程可视化 Agent 方案](docs/agent-call-visualization-agent-plan.md)
- [Agent 调用过程可视化 Frontend 方案](docs/agent-call-visualization-frontend-plan.md)
- [Frontend Loop Tool Parallel Fix 计划](docs/superpowers/plans/2026-05-10-frontend-loop-tool-parallel-fix.md)

## 许可证

MIT
