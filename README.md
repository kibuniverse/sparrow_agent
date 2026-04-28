# Sparrow Agent

一个最小可运行的 Rust Agent 示例，支持基础对话、工具注册和模型工具调用循环。

## 运行

运行前需要配置环境变量：

```bash
export DEEPSEEK_API_KEY=your_deepseek_api_key
export TAVILY_API_KEY=your_tavily_api_key
cargo run
```

启动后输入自然语言问题即可对话，输入 `exit` 或 `quit` 退出。

## 模块结构

- `src/main.rs`：二进制入口，只负责加载配置、启动 REPL、转发用户输入。
- `src/lib.rs`：库入口，集中导出项目模块。
- `src/config.rs`：应用配置和环境变量读取。
- `src/console.rs`：命令行输入和退出命令判断。
- `src/agent.rs`：Agent 编排器，维护消息历史、构造请求、处理模型响应。
- `src/tool_registry.rs`：工具定义、参数解析和工具调用分发。
- `src/tools.rs`：具体工具实现，包括天气示例和 Tavily 搜索。
- `src/client.rs`：DeepSeek HTTP 客户端。
- `src/api.rs`：DeepSeek Chat Completion 请求和响应数据结构。

## 已内置工具

- `getWeather`：返回指定地点的示例天气结果。
- `webSearch`：通过 Tavily 搜索网页信息。

## 检查

```bash
cargo fmt --check
cargo check
cargo test
```
