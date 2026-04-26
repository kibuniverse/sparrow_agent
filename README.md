# Sparrow Agent

一个最小可运行的 Rust Agent 框架示例，支持基础对话、工具注册和工具调用循环。

## 运行

```bash
cargo run
```

启动后可以输入：

```text
时间
echo hello
add 1 2 3
quit
```

## 当前结构

- `ChatModel`：模型抽象，负责根据消息历史和工具列表决定下一步动作。
- `Tool`：工具抽象，负责暴露名称、描述和调用逻辑。
- `ToolRegistry`：工具注册表，负责保存工具和执行工具调用。
- `Agent`：对话编排器，负责维护消息历史、执行模型动作和工具调用循环。
- `DemoModel`：不依赖外部 API 的演示模型，方便本地直接运行和测试。

## 已内置工具

- `time`：返回当前 Unix 时间戳。
- `echo`：原样返回输入文本。
- `add`：计算空格分隔数字的总和。

## 测试

```bash
cargo test
```
