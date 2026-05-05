# Sparrow 文件读写能力实现方案

状态：建议方案
日期：2026-05-05
适用项目：`sparrow_agent`

## 1. 背景

Sparrow 当前是一个最小可运行的 Rust Agent：`Agent` 维护消息历史并驱动工具调用循环，`ToolRegistry` 将 DeepSeek/OpenAI 风格的 function tools 暴露给模型，具体工具在 `tools.rs` 中实现。

现有能力包括：

- `getWeather`：演示用天气工具。
- `webSearch`：通过 Tavily 搜索网络信息。
- `runRustWasm`：将模型生成的 Rust 代码编译为 WASM 并在无 WASI 的沙盒中执行。

文件读写能力不能通过给 `runRustWasm` 注入宿主文件系统来实现。WASM 工具目前的安全价值正是无文件系统、无网络、无环境变量；把文件读写塞进 WASM 会破坏隔离边界，也让审计和确认变得困难。

推荐做法是把文件系统作为显式工具能力接入，并让权限、确认、日志和路径边界全部在工具层可见。

## 2. 社区主流方向

截至 2026-05-05，社区里给 Agent 增加本地文件读写能力的主流方式是 MCP-first：

- Sparrow 作为 MCP Host/Client。
- 通过 stdio 启动一个 filesystem MCP Server。
- 使用 MCP 的 `tools/list` 发现工具，再将 MCP tool 映射成模型可调用的 function tool。
- 使用 MCP Roots 暴露允许访问的目录边界。
- 对写入、移动、覆盖等敏感操作做用户确认。

这一路线比自定义本地文件工具更适合长期演进，因为它可以复用社区已有的 filesystem server，也可以在未来接入 GitHub、数据库、浏览器等 MCP server，而不必为每个能力重写一套工具协议。

主要参考：

- MCP Tools 规范：<https://modelcontextprotocol.io/specification/2025-06-18/server/tools>
- MCP Roots 规范：<https://modelcontextprotocol.io/specification/2025-06-18/client/roots>
- MCP Transports 规范：<https://modelcontextprotocol.io/specification/2025-06-18/basic/transports>
- 官方 filesystem MCP server：<https://github.com/modelcontextprotocol/servers/blob/main/src/filesystem/README.md>

## 3. 目标

本方案的目标是为 Sparrow 增加安全、可配置、可审计的文件读写能力：

- 模型可以读取、搜索、列出用户授权目录中的文件。
- 模型可以在用户确认后创建、写入、编辑、移动文件。
- 所有文件操作都限制在配置的 Roots 内。
- 写操作默认需要人类确认。
- 文件能力与现有工具调用循环兼容。
- 实现方式遵循 MCP 工具发现和调用模型，避免绑定到单一文件工具实现。
- 不破坏现有 WASM 沙盒安全边界。

## 4. 非目标

本阶段不做以下事情：

- 不把宿主文件系统挂进 `runRustWasm`。
- 不允许模型无确认地覆盖、移动、删除用户文件。
- 不实现完整 IDE 或代码编辑器 UI。
- 不把文件内容长期缓存进配置文件。
- 不默认读取密钥、环境文件、Git 内部对象等敏感路径。
- 不在第一阶段处理大型二进制文件的深度解析。

## 5. 总体架构

当前架构：

```text
User Input
  -> Agent
      -> DeepSeek Chat Completion
          -> ToolRegistry
              -> getWeather
              -> webSearch
              -> runRustWasm
```

目标架构：

```text
User Input
  -> Agent
      -> DeepSeek Chat Completion
          -> ToolRegistry
              -> LocalToolProvider
                  -> getWeather
                  -> webSearch
                  -> runRustWasm
              -> McpToolProvider(filesystem)
                  -> McpClient
                      -> StdioTransport
                          -> @modelcontextprotocol/server-filesystem
```

核心原则：

- `Agent` 不关心工具来自本地还是 MCP。
- `ToolRegistry` 负责统一暴露工具定义、分发工具调用。
- `LocalToolProvider` 负责现有本地工具。
- `McpToolProvider` 负责 MCP server 生命周期、工具发现、调用转发和结果归一化。
- 文件权限、确认、路径校验放在 Sparrow 客户端侧和 filesystem server 双层执行。

## 6. MCP 接入方式

### 6.1 Transport

第一阶段使用 MCP stdio transport：

- Sparrow 启动 MCP server 作为子进程。
- Sparrow 向 server 的 `stdin` 写入 JSON-RPC 消息。
- Sparrow 从 server 的 `stdout` 按行读取 JSON-RPC 消息。
- server 的 `stderr` 仅作为日志处理，不进入模型上下文。

推荐默认 server：

```json
{
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-filesystem"]
}
```

可选 Docker server：

```json
{
  "command": "docker",
  "args": [
    "run",
    "-i",
    "--rm",
    "--mount",
    "type=bind,src=${workspaceRoot},dst=/projects/workspace",
    "mcp/filesystem",
    "/projects"
  ]
}
```

默认优先使用 `npx`，因为本地开发体验简单；生产或更强隔离场景再切换 Docker。

### 6.2 Initialization

Sparrow 初始化 MCP server 时声明：

```json
{
  "capabilities": {
    "roots": {
      "listChanged": true
    }
  },
  "clientInfo": {
    "name": "sparrow_agent",
    "version": "0.1.0"
  },
  "protocolVersion": "2025-06-18"
}
```

filesystem server 初始化后会请求 `roots/list`。Sparrow 返回配置中的 Roots：

```json
{
  "roots": [
    {
      "uri": "file:///Users/example/project",
      "name": "sparrow workspace"
    }
  ]
}
```

Roots 是文件访问的第一道边界。命令行参数也可以指定允许目录，但推荐支持 Roots，因为它允许未来在运行时更新授权目录。

### 6.3 Tool discovery

初始化完成后，Sparrow 调用：

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/list"
}
```

然后把 MCP tool 转换为现有的 `ToolDef`：

```text
MCP Tool
  name: read_text_file
  description: ...
  inputSchema: ...

Sparrow ToolDef
  function.name: mcp__filesystem__read_text_file
  function.description: ...
  function.parameters: inputSchema
```

工具名必须加命名空间，避免和本地工具重名：

```text
mcp__{server_id}__{tool_name}
```

例如：

- `mcp__filesystem__read_text_file`
- `mcp__filesystem__list_directory`
- `mcp__filesystem__edit_file`

`ToolRegistry` 内部保留反向映射：

```text
mcp__filesystem__read_text_file -> server_id=filesystem, tool_name=read_text_file
```

### 6.4 Tool call

当模型调用 `mcp__filesystem__read_text_file` 时，Sparrow 转换为 MCP `tools/call`：

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "read_text_file",
    "arguments": {
      "path": "README.md"
    }
  }
}
```

MCP tool 返回的 `content` 需要归一化为 `String`，再通过现有 `ChatMessage::tool(...)` 放回模型上下文。

初版只向模型传递文本内容：

- `text`：直接拼接。
- `image` / `audio`：返回简短说明，不直接塞入 base64。
- `resource_link`：返回 URI、名称、mimeType。
- `structuredContent`：如存在且有助于模型理解，可以追加 JSON 摘要。

## 7. 支持的文件工具

官方 filesystem MCP server 当前提供的常用工具包括：

### 7.1 第一阶段：只读工具

默认先开放只读工具，风险最低：

- `read_text_file`：读取 UTF-8 文本文件，支持 `head` / `tail`。
- `read_multiple_files`：读取多个文件。
- `list_directory`：列目录。
- `list_directory_with_sizes`：列目录并显示大小。
- `directory_tree`：返回 JSON 目录树。
- `search_files`：搜索文件或目录。
- `get_file_info`：读取文件元数据。
- `list_allowed_directories`：查看 server 当前允许目录。

### 7.2 第二阶段：写入工具

写入工具必须经过确认：

- `create_directory`：创建目录。
- `write_file`：创建或覆盖文件。
- `edit_file`：基于文本替换做局部编辑。
- `move_file`：移动或重命名文件/目录。

`edit_file` 必须优先 dry run。只有用户确认 diff 后，Sparrow 才能用 `dryRun: false` 再调用一次。

### 7.3 暂缓或特殊处理

- `read_media_file`：可以保留在工具列表中，但第一阶段建议默认禁用或只返回文件元数据，避免把大块 base64 放进上下文。
- 删除能力：官方 filesystem server README 中描述目录能力时提到创建、列出、删除目录，但当前工具列表以 `create_directory`、`move_file`、`write_file`、`edit_file` 为主。若未来 server 暴露删除工具，Sparrow 必须默认禁用，除非引入更强的确认和回收机制。

## 8. 配置设计

### 8.1 AppConfig 扩展

`src/config.rs` 中的 `AppConfig` 增加：

```rust
pub struct AppConfig {
    pub api_key: String,
    pub tavily_api_key: String,
    pub model: String,
    pub system_prompt: String,
    pub reasoning_effort: String,
    pub max_tool_rounds: usize,
    pub filesystem: FilesystemConfig,
    pub mcp_servers: Vec<McpServerConfig>,
}
```

新增配置类型：

```rust
pub struct FilesystemConfig {
    pub enabled: bool,
    pub roots: Vec<PathBuf>,
    pub mode: FilesystemMode,
    pub confirm: ConfirmationPolicy,
    pub deny_patterns: Vec<String>,
    pub max_read_bytes: u64,
    pub max_write_bytes: u64,
}

pub enum FilesystemMode {
    ReadOnly,
    ReadWrite,
}

pub enum ConfirmationPolicy {
    Never,
    Writes,
    Always,
}

pub struct McpServerConfig {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    pub enabled: bool,
}
```

### 8.2 默认配置

默认开启只读文件能力，Root 为当前工作目录：

```json
{
  "filesystem": {
    "enabled": true,
    "roots": ["."],
    "mode": "read-only",
    "confirm": "writes",
    "denyPatterns": [
      ".git/**",
      ".env",
      ".env.*",
      "**/id_rsa",
      "**/id_ed25519",
      ".sparrow_agent/**"
    ],
    "maxReadBytes": 262144,
    "maxWriteBytes": 262144
  },
  "mcpServers": [
    {
      "id": "filesystem",
      "enabled": true,
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem"]
    }
  ]
}
```

说明：

- `roots` 支持相对路径，相对当前启动目录解析。
- `mode` 默认为 `read-only`，需要用户显式改为 `read-write` 才开放写入。
- `confirm` 即使在 `read-only` 下也保留，方便后续切换。
- `denyPatterns` 是 Sparrow 客户端侧额外防线，不替代 filesystem server 的 allowed directories。

### 8.3 环境变量覆盖

建议新增：

| 环境变量 | 说明 |
| --- | --- |
| `SPARROW_FILESYSTEM_ENABLED` | `true` / `false` |
| `SPARROW_FILESYSTEM_ROOTS` | 以平台路径分隔符分隔的 Roots |
| `SPARROW_FILESYSTEM_MODE` | `read-only` / `read-write` |
| `SPARROW_FILESYSTEM_CONFIRM` | `never` / `writes` / `always` |
| `SPARROW_MCP_FILESYSTEM_COMMAND` | 覆盖 filesystem server 命令 |
| `SPARROW_MCP_FILESYSTEM_ARGS` | 覆盖 filesystem server 参数，JSON array 字符串 |

## 9. 代码改造方案

### 9.1 ToolProvider 抽象

新增 `src/tool_provider.rs`：

```rust
use anyhow::Result;

use crate::api::{ToolCall, ToolDef};

#[async_trait::async_trait]
pub trait ToolProvider: Send + Sync {
    fn id(&self) -> &str;
    fn definitions(&self) -> &[ToolDef];
    async fn execute(&self, tool_call: &ToolCall) -> Result<Option<String>>;
}
```

返回 `Option<String>` 的原因：

- `Some(result)`：该 provider 处理了这个工具。
- `None`：不是该 provider 的工具，继续交给下一个 provider。

`ToolRegistry` 改为：

```rust
pub struct ToolRegistry {
    providers: Vec<Box<dyn ToolProvider>>,
    definitions: Vec<ToolDef>,
}
```

`execute(...)` 从硬编码 match 改成遍历 provider。

### 9.2 LocalToolProvider

把当前 `tool_registry.rs` 中的本地工具逻辑迁移到 `src/local_tools.rs`：

- 保留 `GET_WEATHER_TOOL`、`WEB_SEARCH_TOOL`、`RUN_RUST_WASM_TOOL`。
- 保留现有 JSON schema。
- 保留 `parse_arguments<T>(...)`。
- 继续调用 `tools::get_weather`、`tools::web_search`、`tools::run_rust_wasm`。

迁移后 `ToolRegistry::new(...)` 类似：

```rust
let mut registry = ToolRegistry::default();
registry.add_provider(Box::new(LocalToolProvider::new(tavily_api_key)));

if config.filesystem.enabled {
    registry.add_provider(Box::new(McpToolProvider::connect(...).await?));
}
```

### 9.3 Agent 初始化改造

当前 `Agent::new(config)` 是同步方法，但 MCP 初始化需要启动子进程并握手。

建议改为：

```rust
impl Agent {
    pub async fn new(config: AppConfig) -> Result<Self> {
        ...
        let tool_registry = ToolRegistry::new(&config).await?;
        ...
    }
}
```

`main.rs` 中改为：

```rust
let mut agent = Agent::new(config).await?;
```

### 9.4 MCP 模块结构

新增目录：

```text
src/mcp/
  mod.rs
  client.rs
  protocol.rs
  stdio_transport.rs
  filesystem_provider.rs
```

职责：

- `protocol.rs`：MCP JSON-RPC 请求、响应、错误、tool、root、content 类型。
- `stdio_transport.rs`：子进程生命周期、stdin 写入、stdout 按行读取、stderr 日志收集。
- `client.rs`：请求 ID 管理、initialize、tools/list、tools/call、roots/list 响应处理。
- `filesystem_provider.rs`：工具命名空间映射、Sparrow 侧路径校验、确认策略、结果归一化。

### 9.5 JSON-RPC 请求管理

`McpClient` 需要处理三类消息：

- Response：匹配 pending request id。
- Request：server 主动请求 client，例如 `roots/list`。
- Notification：例如 `notifications/tools/list_changed`。

第一阶段必须支持：

- client -> server：`initialize`
- client -> server：`notifications/initialized`
- client -> server：`tools/list`
- client -> server：`tools/call`
- server -> client：`roots/list`
- server -> client：`notifications/tools/list_changed`

`tools/list_changed` 的处理方式：

1. 重新调用 `tools/list`。
2. 更新 `McpToolProvider` 的工具定义。
3. 刷新 `ToolRegistry.definitions`。

如果当前轮对话中工具列表变化，可以等到下一次用户输入前生效，避免中途改变已发给模型的工具集合。

## 10. 安全模型

### 10.1 双层路径边界

第一层：MCP filesystem server 的 allowed directories / Roots。

第二层：Sparrow 客户端侧校验。

Sparrow 在调用任何 filesystem tool 前都要执行：

1. 解析工具参数中的路径字段。
2. 将相对路径按当前工作目录或匹配 root 解析。
3. 对已存在路径使用 `canonicalize`，处理 symlink。
4. 对不存在的写入目标，canonicalize 其最近存在的父目录。
5. 确认解析后的路径位于某个 root 下。
6. 检查 deny patterns。
7. 检查文件大小限制。

需要检查的参数名：

- `path`
- `paths`
- `source`
- `destination`
- `excludePatterns` 不作为路径边界判断，但要限制长度。

### 10.2 Deny patterns

默认拒绝：

```text
.git/**
.env
.env.*
**/id_rsa
**/id_ed25519
**/*.pem
**/*.key
.sparrow_agent/**
```

如果用户明确需要读取敏感文件，应通过配置显式调整 deny patterns，而不是由模型在对话中绕过。

### 10.3 写操作确认

写工具默认需要确认：

- `write_file`
- `edit_file`
- `create_directory`
- `move_file`

确认内容至少包括：

```text
Sparrow wants to call filesystem tool:
  tool: edit_file
  path: src/main.rs
  mode: write

Preview:
  <diff or compact operation summary>

Approve? [y/N]
```

确认策略：

- `never`：不提示确认。只建议在 CI、测试或完全受控目录中使用。
- `writes`：只写操作确认。默认值。
- `always`：所有文件操作都确认。

### 10.4 edit_file dry run

`edit_file` 执行流程：

1. 如果模型没有传 `dryRun: true`，Sparrow 拦截并先用 `dryRun: true` 调用一次。
2. 将 dry run 返回的 diff 展示给用户。
3. 用户确认后，Sparrow 再使用原参数并设置 `dryRun: false` 调用。
4. 如果用户拒绝，向模型返回 `Tool execution denied by user`。

这样可以避免模型直接做不可见编辑。

### 10.5 输出限制

为避免大文件塞满模型上下文：

- 单次读取默认最大 256 KiB。
- `read_text_file` 如果未指定 `head` / `tail` 且文件过大，返回错误并建议模型使用 `head` 或 `tail`。
- `read_multiple_files` 总输出默认最大 512 KiB。
- `directory_tree` 和 `search_files` 限制结果数量。
- 二进制和媒体文件第一阶段只返回元数据。

### 10.6 审计日志

新增结构化工具调用日志，默认仅在 `SPARROW_DEBUG` 开启时输出。

日志字段：

- timestamp
- tool name
- normalized paths
- read/write classification
- confirmation result
- duration
- success/error

不得记录完整文件内容和密钥值。

## 11. 用户体验

### 11.1 启动提示

当文件能力启用时，启动输出应显示：

```text
Filesystem tools enabled.
Roots:
  - /Users/example/project
Mode: read-only
Write confirmation: writes
```

如果 MCP server 启动失败：

- 不应导致 Sparrow 整体不可用。
- 应提示 filesystem tools disabled，并保留聊天、搜索、WASM 等其他工具。
- 如果用户显式要求必须启用文件能力，可以在未来增加 strict 模式。

### 11.2 REPL 命令

第三阶段新增命令：

| 命令 | 作用 |
| --- | --- |
| `/fs` | 显示文件能力状态、roots、mode |
| `/fs roots` | 显示当前 roots |
| `/fs allow <path>` | 增加 root，并发送 `notifications/roots/list_changed` |
| `/fs readonly` | 切到只读模式 |
| `/fs readwrite` | 切到读写模式 |
| `/fs off` | 当前会话禁用文件能力 |

这些命令不进入模型上下文，由 `main.rs` 或新的 command router 直接处理。

## 12. 分阶段实施计划

### 阶段 0：文档和结构准备

- 新增本设计文档。
- 在 README 加入口。
- 确认 crate 依赖策略。

推荐依赖：

```toml
async-trait = "0.1"
serde_path_to_error = "0.1"
globset = "0.4"
url = "2"
```

说明：

- `async-trait` 简化 dyn provider 的 async trait。
- `globset` 用于 deny patterns。
- `url` 用于 `file://` root URI。
- `serde_path_to_error` 可改善 MCP JSON 解析错误定位。

### 阶段 1：MCP 只读接入

任务：

- 新增 `ToolProvider` 抽象。
- 将现有工具迁移到 `LocalToolProvider`。
- 实现 `McpClient` 和 `StdioTransport`。
- 支持 initialize、roots/list、tools/list、tools/call。
- 接入 filesystem MCP server。
- 只开放只读工具。
- 添加路径校验和输出限制。

验收：

- 用户能让模型读取 `README.md`。
- 用户能让模型列出 `src/`。
- 用户能让模型搜索 Rust 文件。
- 访问 root 外路径会被拒绝。
- `.env`、`.git/**` 等 deny pattern 会被拒绝。
- `cargo test` 通过。

### 阶段 2：写入能力

任务：

- 将 mode 从 `read-only` 扩展到 `read-write`。
- 开放 `create_directory`、`write_file`、`edit_file`、`move_file`。
- 实现写操作确认。
- `edit_file` 强制 dry run。
- 添加审计日志。

验收：

- 模型可在确认后创建新文件。
- 模型可在确认后编辑文件。
- 用户拒绝后文件不变，模型收到明确拒绝结果。
- 覆盖文件前必须确认。
- root 外写入被拒绝。

### 阶段 3：运行时控制

任务：

- 增加 `/fs` 命令组。
- 支持运行时更新 roots。
- 支持 `notifications/roots/list_changed`。
- 工具列表变化后刷新 registry。

验收：

- 用户可在不重启 Sparrow 的情况下增加 root。
- 用户可切换只读/读写模式。
- 当前会话禁用文件能力后，模型不再看到 filesystem tools。

### 阶段 4：增强和硬化

任务：

- 支持 read media 的安全摘要。
- 支持更好的 diff 展示。
- 支持 HTTP MCP transport。
- 支持多个 MCP server。
- 支持 per-root read-only / read-write 权限。
- 支持持久化 audit log。

验收：

- 多 server 工具名不冲突。
- filesystem server 重启后可恢复。
- 大文件和二进制文件不会污染上下文。

## 13. 测试计划

### 13.1 单元测试

路径安全：

- 相对路径在 root 内可访问。
- `../` 逃逸被拒绝。
- symlink 指向 root 外被拒绝。
- 不存在文件的父目录校验正确。
- deny patterns 生效。

工具映射：

- MCP tool name 正确映射为 `mcp__filesystem__...`。
- 反向映射正确。
- 本地工具与 MCP 工具重名时不会冲突。

结果归一化：

- text content 正确合并。
- image/audio content 不直接输出 base64。
- MCP `isError: true` 转换为工具错误文本。

确认策略：

- read-only 模式拒绝写工具。
- `writes` 模式只确认写工具。
- `always` 模式所有工具都确认。
- 用户拒绝后不调用实际写工具。

### 13.2 集成测试

不依赖 Node 的测试：

- 实现一个 fake MCP server 子进程或内存 transport。
- 模拟 initialize、roots/list、tools/list、tools/call。
- 验证 `ToolRegistry.execute_all(...)` 能正确执行 MCP 工具。

可选 Node 测试：

- 如果环境中存在 `npx`，启动官方 filesystem server。
- 使用 tempdir 作为 root。
- 读写 tempdir 中的文件。
- 标记为 ignored 或仅在 CI 条件满足时运行。

### 13.3 手动验收脚本

只读：

```text
请读取 README.md 并总结这个项目的能力。
请列出 src 目录下的 Rust 文件。
请搜索包含 ToolRegistry 的文件。
```

越权：

```text
请读取 ../Cargo.toml。
请读取 .git/config。
请读取 .env。
```

写入：

```text
请在 docs 下创建一个 hello.md，内容为 Hello Sparrow。
请把 docs/hello.md 中的 Sparrow 改成 MCP。
```

预期：

- 越权和 deny 文件被拒绝。
- 写入前出现确认。
- 拒绝确认时文件不变。

## 14. 风险和应对

| 风险 | 应对 |
| --- | --- |
| MCP server 未安装或 npx 不可用 | 启动失败时禁用 filesystem tools，并提示安装/配置方式 |
| 工具列表变化导致模型调用旧工具 | 每轮请求前刷新 definitions，或在 list_changed 后下一轮生效 |
| 大文件撑爆上下文 | 强制大小限制，要求 head/tail 或搜索 |
| symlink 逃逸 root | canonicalize 已存在路径；写目标检查最近存在父目录 |
| 写操作误覆盖 | 默认 read-only；read-write 下写操作确认；edit_file dry run |
| server annotations 不可信 | annotations 只作为提示，Sparrow 维护自己的工具分类 allowlist |
| 模型尝试读取密钥 | deny patterns 默认拒绝；敏感路径需用户显式配置 |
| JSON-RPC 并发复杂 | 第一阶段串行调用工具；后续再优化并发 |

## 15. 兼容性

现有用户体验保持兼容：

- 没有配置文件能力时，Sparrow 仍可聊天、搜索、执行 WASM。
- 现有环境变量继续生效。
- 现有 `ToolRegistry.execute_all(...)` 对 Agent 的接口可以保持不变。
- `runRustWasm` 继续保持无 WASI 的安全设计。

需要注意的破坏性改动：

- `Agent::new(...)` 会从同步函数变为 async，并返回 `Result<Self>`。
- `ToolRegistry::new(...)` 会从同步函数变为 async。
- 配置文件结构会新增字段，但需要 serde default 保持旧配置可读。

## 16. 推荐落地顺序

优先顺序：

1. 增加配置类型和默认值。
2. 抽 `ToolProvider`。
3. 迁移本地工具，不改变行为。
4. 实现 fake MCP transport 测试基础协议。
5. 实现 stdio transport。
6. 接入 filesystem server 的只读工具。
7. 加路径校验、deny patterns、大小限制。
8. 实现写入确认和 `edit_file` dry run。
9. 增加 `/fs` 命令组。
10. 加 README 使用说明。

每一步都应保持 `cargo fmt --check`、`cargo check`、`cargo test` 通过。

## 17. 完成定义

第一版文件能力完成时，必须满足：

- 用户能通过自然语言读取授权 root 内的文本文件。
- 用户能列目录、查找文件、获取文件元数据。
- 默认只读。
- root 外路径无法访问。
- deny patterns 默认保护敏感文件。
- 写能力需要显式配置为 read-write。
- 写操作前有确认。
- `edit_file` 先 dry run 再确认应用。
- MCP server 启动失败不会导致 Sparrow 整体崩溃。
- 文档说明如何启用、禁用和配置文件能力。
- 单元测试覆盖路径安全、工具映射、确认策略。

## 18. 后续扩展

完成 filesystem MCP server 后，同样的 `McpToolProvider` 可以复用到：

- GitHub MCP server： issue、PR、代码搜索。
- Browser MCP server：网页导航和截图。
- Database MCP server：只读查询或受控写入。
- Project-specific MCP server：为 Sparrow 暴露自定义业务工具。

因此，本方案的长期价值不只是文件读写，而是把 Sparrow 从“内置几个工具的 Agent”升级为“可以接入标准工具生态的 MCP Host”。
