# Sparrow Agent

一个最小可运行的 Rust Agent 示例，支持基础对话、工具调用、Web 搜索和 WASM 沙盒代码执行。

## 功能特性

- **多轮对话**：维护完整消息历史，支持上下文连续对话
- **工具调用循环**：模型可自动调用工具并获取结果，最多连续 6 轮工具调用
- **深度思考模式**：启用 DeepSeek 的 thinking/reasoning 模式，支持可配置的推理强度
- **Web 搜索**：通过 Tavily API 搜索网页信息
- **WASM 沙盒执行**：将 LLM 生成的 Rust 代码编译为 WASM 并在安全沙盒中运行
- **安全配置管理**：API 密钥安全存储，支持环境变量优先覆盖

## 运行

首次运行时可以直接启动，命令行会引导输入 `DEEPSEEK_API_KEY` 和 `TAVILY_API_KEY`，并保存到 `~/.sparrow_agent/config.json`：

```bash
cargo run
```

如果已经配置环境变量，环境变量会优先生效：

```bash
export DEEPSEEK_API_KEY=your_deepseek_api_key
export TAVILY_API_KEY=your_tavily_api_key
cargo run
```

启动后输入自然语言问题即可对话，输入 `exit` 或 `quit` 退出。

## 配置

### 环境变量

| 变量 | 说明 | 优先级 |
|------|------|--------|
| `DEEPSEEK_API_KEY` | DeepSeek API 密钥（必需） | 高于配置文件 |
| `TAVILY_API_KEY` | Tavily 搜索 API 密钥（必需） | 高于配置文件 |
| `SPARROW_CONFIG_PATH` | 自定义配置文件路径（可选） | — |
| `SPARROW_DEBUG` | 启用调试日志，设为任意值开启（可选） | — |

### 默认配置

| 配置项 | 默认值 |
|--------|--------|
| 模型 | `deepseek-v4-flash` |
| 系统提示词 | `You are a helpful assistant.` |
| 推理强度 | `high` |
| 最大工具调用轮数 | `6` |

### 配置文件

配置文件存储在 `~/.sparrow_agent/config.json`，Unix 系统上文件权限为 `0600`（仅所有者可读写）。

## 架构

```
用户输入 → Agent 编排器 → DeepSeek API
               ↑              ↓
          工具结果 ← 工具注册器 ← 工具调用
                           ↓
              ┌────────────┼────────────┐
              ↓            ↓            ↓
          getWeather   webSearch   runRustWasm
                                    ↓
                            WASM 沙盒执行
```

Agent 编排器驱动多轮工具循环：发送消息 → 接收响应 → 若含工具调用则执行工具 → 将结果追加到消息历史 → 再次请求模型，直到模型返回文本回复或达到最大轮数。

## 已内置工具

| 工具名 | 说明 |
|--------|------|
| `getWeather` | 返回指定地点的示例天气结果（演示用） |
| `webSearch` | 通过 Tavily API 搜索网页，返回摘要和来源链接 |
| `runRustWasm` | 将 Rust 代码编译为 WASM 并在沙盒中安全执行 |

## WASM 沙盒

`runRustWasm` 工具将 LLM 生成的 Rust 代码安全执行，具备以下安全机制：

- **Fuel 计量**：初始 Fuel 为 1,000,000，防止无限循环
- **无 WASI**：不注入文件系统、网络、环境变量等宿主接口，代码完全隔离
- **编译超时**：10 秒编译超时限制
- **输出限制**：结果最大 64 KiB，编译 stderr 最大 16 KiB
- **内存安全**：通过 wastime 导出内存读取结果，含边界检查

用户代码需定义 `pub fn run() -> String`，编译目标为 `wasm32-unknown-unknown`。

## 模块结构

| 文件 | 说明 |
|------|------|
| `src/main.rs` | 二进制入口，加载配置、启动 REPL、转发用户输入 |
| `src/lib.rs` | 库入口，集中导出项目模块 |
| `src/config.rs` | 应用配置加载与持久化，API 密钥交互式输入 |
| `src/console.rs` | 命令行输入输出，密钥安全输入（禁用回显） |
| `src/agent.rs` | Agent 编排器，维护消息历史、构造请求、驱动工具循环 |
| `src/tool_registry.rs` | 工具定义注册、参数 JSON 解析和调用分发 |
| `src/tools.rs` | 具体工具实现：天气示例、Tavily 搜索、WASM 执行入口 |
| `src/client.rs` | DeepSeek HTTP 客户端，发送 Chat Completion 请求 |
| `src/api.rs` | DeepSeek Chat Completion 请求和响应数据结构 |
| `src/rust_wasm_runner.rs` | Rust→WASM 编译与 wastime 沙盒执行 |
| `src/debug.rs` | 调试日志工具，通过 `SPARROW_DEBUG` 环境变量控制 |

## 依赖

| 依赖 | 用途 |
|------|------|
| `tokio` | 异步运行时 |
| `reqwest` | HTTP 客户端，调用 DeepSeek 和 Tavily API |
| `serde` / `serde_json` | JSON 序列化与反序列化 |
| `wasmtime` | WASM 运行时，执行沙盒代码 |
| `anyhow` | 错误处理与上下文传播 |
| `tempfile` | 为 WASM 编译创建临时目录 |

## 检查

```bash
cargo fmt --check
cargo check
cargo test
```

## 许可证

MIT
