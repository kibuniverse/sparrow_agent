# WorkingMemory 接通设计方案

日期：2026-06-13
状态：已确认

## 背景

`src/context/memory.rs` 定义了结构化工作记忆 `WorkingMemory`（`task_goal` / `durable_facts` / `decisions` / `open_questions` / `artifact_refs`），并由 `ContextCompiler` 在编译请求时通过 `to_context_message` 注入到发给模型的消息里。注入位置在 system prompt 之后、history summary 之前、recent turns 之前——即记忆能在对话历史被压缩后仍然存活。

但经代码核查，整份 `src/` 中**没有任何代码写入 `WorkingMemory`**：

- 对 `WorkingMemory` 字段的赋值 / `push` 操作：0 处。
- `MemoryItem::new` 调用点：0 处（仅有定义）。
- 不存在 memory 写入工具（`remember*` 命中全部属于 bash 审批缓存，无关）。

后果：`ContextManager::new` 中 `working_memory` 永远是 `Default::default()`（空），`to_context_message` 因 `is_empty()` 恒返回 `None`，`include_memory` 分支永不产生消息。整套记忆模块处于**搭好骨架但完全休眠**的状态——数据模型、渲染、注入位置、`max_memory_tokens` 预算约束都在，只差"谁往里写"。

本方案的目标是接通这条写入路径，让 WorkingMemory 真正工作。

## 目标

1. 提供一条模型可显式调用的写入路径，让模型按需把持久事实、决定、待解决问题写入 `WorkingMemory`。
2. 写入结果在下一轮模型调用前反映到上下文（`to_context_message`）。
3. 不破坏现有所有权模型：`ContextManager` 仍是 `working_memory` 的唯一所有者 / 写者；`WorkingMemory` 仍是自身结构的唯一专家。
4. 不改动 `ToolProvider` trait；对 `compiler.rs` 零改动。
5. 子 Agent 与记忆隔离：子 Agent 既看不到、也写不到父 Agent 的记忆。
6. 全程内存，不引入磁盘持久化（作为后续可叠加项）。

非目标：

- 不做跨会话磁盘持久化。
- 不做自动 post-turn LLM 提取（写入完全由模型显式触发）。
- 不做记忆自动淘汰 / 智能截断策略（沿用现有 `max_memory_tokens` 渲染截断 + 模型 `remove_*` / `clear`）。
- 不让子 Agent 读写父记忆。

## 关键决策（已与用户确认）

| 决策 | 选择 |
|---|---|
| 写入机制 | 模型显式工具（新增 `updateMemory`） |
| 写入语义 | 结构化字段操作（set_goal / add_fact / add_decision / add_question / resolve_question / remove_fact / clear） |
| 生命周期 | 仅内存，不持久化 |
| 子 Agent 关系 | 完全隔离 |
| write-back 桥 | 方案 A：待应用增量缓冲，由 Agent 在 `build_request` 排空 |

## 架构约束与桥接方案

`ToolProvider::execute(&self, tool_call: &ToolCall) -> Result<Option<String>>` 是 stateless 的，拿不到 `ContextManager.working_memory`。三可选桥接方案：

- **方案 A（采用）**：`MemoryToolProvider` 持 `Arc<Mutex<Vec<MemoryDelta>>>` 待应用缓冲；`execute` 解析调用、压入缓冲、回 ack；Agent 持同一 Arc 克隆，在 `build_request` 排空并经 `ContextManager::apply_memory_delta` 应用。
- 方案 B（不采用）：把 `working_memory` 改为 `Arc<Mutex<WorkingMemory>>`，波及 compiler / manager，侵入大，且即时写入并不需要。
- 方案 C（不采用）：回调闭包，ergonomics 差。

方案 A 改动面最小，保持单一所有权，"下次编译前统一应用"天然契合"记忆跨轮持久"语义。

## 推荐方案

### 组件划分与归属

| 新增 / 改动 | 内容 |
|---|---|
| 🆕 `src/memory_tool.rs` | `MemoryToolProvider`（实现 `ToolProvider`）+ `updateMemory` 工具定义 + 参数解析；持 `Arc<Mutex<Vec<MemoryDelta>>>` 缓冲 |
| ✏️ `src/context/memory.rs` | 新增 `MemoryDelta`、`MemoryScope`、`DeltaOutcome`；`impl WorkingMemory { fn apply_delta(&mut self, delta) -> DeltaOutcome }` |
| ✏️ `src/context/manager.rs` | 新增 `ContextManager::apply_memory_delta(&mut self, delta) -> DeltaOutcome`（委托 + 刷新 `updated_at`） |
| ✏️ `src/lib.rs` | `pub mod memory_tool;` |
| ✏️ `src/agent.rs` | Agent 新增字段 `memory_buffer: Arc<Mutex<Vec<MemoryDelta>>>`；`new_inner` 创建缓冲 / 克隆给 provider / 存引用；`build_request` 开头排空 |
| ✏️ `src/config.rs` | 默认 system prompt 增补 `updateMemory` 使用指引；新增 `memory.enabled`（默认 true） |

归属原则：`WorkingMemory::apply_delta` 是记忆结构的唯一专家；`ContextManager` 是 `working_memory` 唯一写者；`MemoryToolProvider` 只做"解析 + 入缓冲 + 回 ack"，不碰 ContextManager。

### 数据模型

```rust
pub enum MemoryScope { All, Facts, Decisions, OpenQuestions }

pub enum MemoryDelta {
    SetGoal { goal: String },           // 覆写 task_goal；空串视为清空
    AddFact { content: String },
    AddDecision { content: String },
    AddQuestion { content: String },
    ResolveQuestion { needle: String }, // 移除首条包含 needle 的 open_question（大小写不敏感）
    RemoveFact { needle: String },      // 移除首条包含 needle 的 durable_fact
    Clear { scope: MemoryScope },
}

pub enum DeltaOutcome { Applied, NotFound, Cleared(usize) }
```

`apply_delta` 约定：空 `content` / 空 `needle` 视为无效，返回 `NotFound`（no-op），不 panic。`ResolveQuestion` / `RemoveFact` 只移除**首条**匹配项；无匹配返回 `NotFound`。

### 数据流（每轮 round）

```
round N:
  build_request()
    ├─ 【新】drain self.memory_buffer → 逐条 context.apply_memory_delta
    ├─ context.compile_request(...)        // memory 已最新，注入在 summary 之前
    └─ 返回 request
  model 调用 → 可能在本轮调用 updateMemory
  handle_assistant_message → 执行工具
    └─ MemoryToolProvider.execute: 解析 → push(MemoryDelta) → 返回 ack
  ── 回到 round N+1 ──
```

排空点：`agent.rs` `build_request`（agent.rs:380）第 381 行 `compile_request` **之前**。子 Agent 的 `build_request` 同样排空，但其缓冲恒空（调不到该工具），为 no-op。

ack 文案：`Memory update queued: {operation}` + `It will apply on the next step.`。诚实——工具只确认入队，不谎报最终态（工具看不到 ContextManager）；模型本就从注入的 memory message 知道当前态。

### 工具 schema

```jsonc
{
  "name": "updateMemory",
  "description": "Persist a durable fact, decision, or open question for the ongoing task. Use sparingly for things that must survive conversation compaction. Each call is one operation.",
  "parameters": {
    "type": "object",
    "required": ["operation"],
    "properties": {
      "operation": { "type": "string", "enum": ["set_goal","add_fact","add_decision","add_question","resolve_question","remove_fact","clear"] },
      "content":  { "type": "string", "description": "for add_fact/add_decision/add_question" },
      "goal":     { "type": "string", "description": "for set_goal (empty clears)" },
      "needle":   { "type": "string", "description": "substring to match for resolve_question/remove_fact (first match removed)" },
      "scope":    { "type": "string", "enum": ["all","facts","decisions","open_questions"], "description": "for clear (default all)" }
    }
  }
}
```

### 子 Agent 隔离

`MemoryToolProvider` 在 `new_inner` 无条件加入（对齐 `LocalToolProvider`）。子 Agent 经 `restrict_to_allowed_tools` 过滤——只要 `updateMemory` ∉ `SubAgentConfig.allowed_tools`（默认列表不含它），子 Agent 既看不到定义，即便请求也会被 `effective_allowed_tools`（sub_agent.rs:327）拒绝。父 Agent 想记住子任务结果时，读回结构化结果后自行调用 `updateMemory`。

### 错误处理

- 参数解析沿用 `local_tools` 约定（serde struct + `?`）：非法 args 经 `ToolRegistry` 现有错误路径以错误工具结果回给模型，模型可自我纠正。
- 缓冲 mutex 锁中毒：降级为 ack 文案 `memory buffer unavailable`，不中断 turn。
- 无匹配 / 空操作：返回 `NotFound` 类 ack（如 `no matching open_question to resolve`），no-op，不报错。

### 测试

- `memory.rs` 单元测试：`apply_delta` 每个操作 + 边界（覆写 goal、首条匹配移除、无匹配 no-op、clear 各 scope、空 content）。
- `memory.rs` 集成测试：连续 apply 多个 delta 后 `to_context_message` 反映最终态。
- `tests/memory_tool_contract.rs`：provider 解析合法 / 非法 args、入缓冲行为、ack 文案。
- 排空逻辑：抽 `Agent::drain_memory()`（或对 `ContextManager::apply_memory_delta` 链路）单测。

## 影响面

新增 1 个文件，5 处点状改动；不改 `ToolProvider` trait，不改 compiler / manager 所有权模型；子 Agent 隔离零额外成本。`max_memory_tokens` 渲染截断已存在，无需新增淘汰策略。
