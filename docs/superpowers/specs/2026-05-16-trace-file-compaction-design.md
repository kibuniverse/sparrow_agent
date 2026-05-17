# Trace 文件行数缩减设计方案

日期：2026-05-16
状态：草案

## 背景

当前 trace 归档由 `src/trace_file.rs` 在任务完成后将 `TraceStore` 的 `TaskSnapshot` 用 `serde_json::to_string_pretty` 写成 `.sparrow-trace.json`。后端实时链路和前端回放都消费同一种事件模型：

- 后端 `TraceStore` 按 task 递增 `seq` 保存 `TraceEvent`。
- `agent.rs` 在流式模型调用中逐 token/片段写入 `model_call.reasoning_delta` 和 `model_output.delta`。
- `model_call.started` 的 `payload.request` 通过 `model_request_snapshot` 保存完整 `messages`。
- 前端 `traceReducer` 按事件顺序还原模型调用、reasoning、模型输出和工具调用节点。

样例 trace 显示，文件过长不是单一原因：

| 文件样例 | 行数 | 事件数 | reasoning delta | reasoning 连续段 | output delta | output 连续段 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `task_01KREG...` | 83,793 | 6,479 | 396 | 11 | 5,964 | 12 |
| `task_01KRQ4...` | 75,225 | 5,484 | 347 | 10 | 5,014 | 11 |
| `task_01KR95...` | 47,578 | 3,621 | 330 | 9 | 3,164 | 10 |

另外，部分 trace 的 `model_call.started.payload.request.text` 总量达到 120KB 到 386KB，说明完整 `messages` 的重复保存也会显著增加体积。当前 `JsonSnapshot` 只截断 `text` 字段，`value` 仍保存完整 JSON，因此 pretty JSON 会把大量嵌套 message 展开成很多行。

## 目标

1. 显著减少归档 trace 文件的行数和体积，优先处理完成后写到本地的 `.sparrow-trace.json`。
2. 保留现有实时 UI、回放 UI 和调试语义：前端最终看到的仍可还原为当前 canonical `TraceEvent` 语义。
3. 对旧 trace 文件保持兼容，`schema_version = 1` 的文件仍可读取。
4. 压缩过程不绕过现有敏感字段脱敏和 snapshot 截断策略。
5. 出错时可回退到完整帧，避免为了压缩牺牲 trace 可读性和可恢复性。

非目标：

- 不只依赖 gzip 或 minified JSON。它们能减少物理行数/字节数，但不能解决重复事件和重复 `messages`。
- 不改变 Agent 调用模型、工具执行流程或模型上下文构造。
- 不在第一阶段改变实时 SSE 的粒度，避免影响正在运行中的 UI 体验。

## 推荐方案：归档 v2 压缩，读取时展开

新增 `schema_version = 2` 的 compact trace archive。写文件时对事件和 snapshot 做压缩；读文件时展开成当前前端已经理解的 canonical 结构。这样第一阶段只影响本地归档文件，实时 `TraceStore`、SSE、前端 reducer 可以保持稳定。

设计核心：

1. `TraceStore` 继续保存原始事件，保证运行时调试和订阅行为不变。
2. `write_trace_archive` 从 `TraceStore::snapshot` 取原始事件后，调用 `trace_compaction::compact_archive` 生成 v2 文件。
3. `read_trace_archive` 支持 v1 和 v2：v1 原样读取；v2 先展开 compact event 和 snapshot diff，再返回带完整 `TaskSnapshot` 的 `TraceArchive` 视图给 server/front-end。
4. compact 文件默认使用 minified JSON 写出；如需人工审阅，可通过配置打开 pretty 输出。

这种做法的收益来自两层：

- 行数：minified 输出 + 将数千个 delta 事件合并成数十个事件。
- 体积：重复 `messages` 用 keyframe/diff 编码，避免每轮模型调用保存完整历史。

## 事件合并设计

### reasoning delta 合并

对相邻、同一 `payload.node_id` 的 `model_call.reasoning_delta` 做 run 合并。合并后的事件仍使用 `model_call.reasoning_delta` 类型，`payload.delta` 为拼接后的文本，并增加 `_compact` 元数据：

```json
{
  "seq": 153,
  "timestamp": "2026-05-16T10:00:03Z",
  "type": "model_call.reasoning_delta",
  "payload": {
    "node_id": "model_1",
    "delta": "拼接后的完整 reasoning 片段",
    "_compact": {
      "kind": "event_run",
      "count": 53,
      "seq_start": 101,
      "seq_end": 153,
      "timestamp_start": "2026-05-16T10:00:01Z",
      "timestamp_end": "2026-05-16T10:00:03Z"
    }
  }
}
```

事件 envelope 的 `seq` 和 `timestamp` 使用原始 run 的最后一个事件，保证前端 `lastSeq` 单调前进。`seq_start/seq_end` 用于调试和后续精确回放。

读取 v2 时有两个可选策略：

- 默认：不展开 run，直接把合并事件交给 reducer。当前 reducer 会把 `delta` 追加到 reasoning 文本，最终状态正确。
- 精确回放模式：根据 `_compact` 元数据按原始数量拆回多个事件。第一阶段不启用，保留扩展点。

### output delta 合并

样例中 `model_output.delta` 的数量远大于 reasoning delta，因此同样应该压缩：

- `kind = "final_answer"`：相邻、同一 `node_id` 的 `content_delta` 直接拼接。
- `kind = "tool_calls"`：仅在同一 `node_id`、同一 `tool_call.index`、且 `tool_call_id/name` 不冲突时合并 `arguments_delta`；否则 flush 当前 run，避免把并行工具参数片段串错。

合并后的事件仍保持 `model_output.delta` 类型，增加同样的 `_compact` 元数据。

### 不跨边界合并

遇到以下情况必须 flush 当前 run：

- 事件类型变化。
- `node_id` 变化。
- `model_call.completed`、`model_output.completed`、`tool_call.started` 等生命周期边界。
- tool-call delta 的 index/id/name 变化。
- delta 文本为空但原事件有其他语义字段。

## request.messages diff 设计

`model_call.started.payload.request` 是另一类膨胀源。模型请求的 `messages` 通常是 append-only：上一轮完整历史 + assistant/tool 新消息。因此使用“关键帧 + 前缀引用 + 追加/替换 diff”，比通用文本 diff 更稳定。

### 编码结构

每个模型请求 snapshot 有两种形态：

1. Keyframe：保存完整 request snapshot。
2. Delta frame：引用一个 base request，并保存从 base 到 current 的结构化变更。

示例：

```json
{
  "encoding": "snapshot-diff/v1",
  "base_node_id": "model_1",
  "target_hash": "sha256:...",
  "base_hash": "sha256:...",
  "message_count": 12,
  "ops": [
    { "op": "retain_prefix", "count": 10 },
    {
      "op": "append",
      "messages": [
        { "role": "assistant", "content": "", "reasoning_content": "...", "tool_calls": [] },
        { "role": "tool", "content": "...", "tool_call_id": "call_1" }
      ]
    }
  ],
  "stats": {
    "full_bytes": 120000,
    "patch_bytes": 3600
  }
}
```

如果中间消息发生变化，使用 `replace_range`：

```json
{ "op": "replace_range", "start": 8, "delete": 2, "messages": [ ... ] }
```

如果 diff 不划算或过于复杂，直接写 keyframe。

### keyframe 策略

写完整帧的条件：

- 第一轮模型调用。
- 距离上一个 keyframe 超过 8 个 model calls。
- diff 后 JSON 字节数大于完整 snapshot 的 75%。
- 绝对节省小于 2KB。
- base hash 校验失败。
- request 的非 `messages` 字段发生大幅变化，例如 model/tools/thinking 配置变化。

这样可以避免为了小文件过度复杂化，也能保证局部损坏或实现 bug 时有恢复点。

### hash 和安全

- hash 基于脱敏后的 canonical JSON，避免把敏感值作为校验材料。
- diff 在 `JsonSnapshot::from_value` 脱敏之后执行，不能绕过 `api_key/token/password/secret` 规则。
- 展开时必须校验 `base_hash` 和 `target_hash`。失败时返回明确错误，不静默展示错误 trace。

## Archive schema

建议新增内部 v2 结构，不直接替换当前前端类型：

```rust
struct TraceArchiveV2 {
    schema_version: u32,
    exported_at: DateTime<Utc>,
    source: String,
    compression: TraceCompressionMeta,
    task: CompactTaskSnapshot,
}

struct TraceCompressionMeta {
    original_event_count: usize,
    compact_event_count: usize,
    event_runs: usize,
    snapshot_keyframes: usize,
    snapshot_delta_frames: usize,
    minified: bool,
}
```

`CompactTaskSnapshot.events` 可以混合保存 canonical event 和 compact event。`read_trace_archive` 对外仍返回 `TraceArchive { schema_version, exported_at, source, task: TaskSnapshot }`，其中 `task.events` 已展开为前端 reducer 能消费的完整视图，`schema_version` 保留源文件版本。前端类型需要把 `TraceArchive.schema_version` 从固定 `1` 放宽为 `1 | 2` 或 `number`，但 reducer 和页面主体不用理解 compact payload。

为了让前端知道文件被压缩过，可在后续小改中给 API response 增加可选 `compression` 字段；第一阶段不是必须。

## 可选替代方案

### 方案 A：只做 minified JSON

把 `serde_json::to_string_pretty` 改成 `serde_json::to_string`。

优点：实现极小，立即把行数压到 1 行，样例字节数约减少 15% 到 32%。

缺点：不减少事件数量，不减少重复 `messages`，文件仍可能很大；人工审阅变差；`TraceStore` 10,000 事件限制仍可能被 delta 打满。

结论：可作为 v2 compact 文件的默认序列化方式，但不应作为唯一方案。

### 方案 B：运行时 coalescing sink

在 `TraceStoreSink` 前增加 `TraceCompactingSink`，实时缓冲 delta，每 50 到 150ms 或遇到生命周期边界时 flush。

优点：同时减少内存事件数、SSE 事件数、归档文件行数，也能降低触发 10,000 event limit 的概率。

缺点：会改变实时 UI 的流式粒度；需要处理 flush 时机、任务失败、drop 时补 flush；测试面更大。

结论：适合作为第二阶段。在 archive-only 方案稳定后再做。

### 方案 C：外部压缩文件

输出 `.sparrow-trace.json.gz` 或旁路保存 gzip。

优点：体积收益明显，实现成熟。

缺点：不减少逻辑事件数和 JSON 行数；浏览器/API 要处理解压；用户直接打开不方便。

结论：可以作为长期的存储选项，不替代结构化压缩。

## 分阶段落地

### Phase 1：归档结构化压缩

1. 新增 `src/trace_compaction.rs`。
2. 实现 `compact_events`：合并 reasoning/output delta runs。
3. 实现 `compact_request_snapshots`：对 `model_call.started.payload.request` 做 keyframe/delta。
4. 新增 `TraceArchiveV2` 和 read/write 分发。
5. `write_trace_archive` 默认写 v2 minified JSON。
6. `read_trace_archive` 兼容 v1，并把 v2 展开成当前 `TraceArchive` 视图。

### Phase 2：前端显示压缩元信息

1. API response 增加可选 `compression`。
2. Trace 预览页显示原始事件数、压缩后事件数、keyframe/delta 数量。
3. 回放页默认按合并事件播放，后续可加“精确 token 回放”开关。

### Phase 3：实时事件合并

1. 增加 `TraceCompactingSink`。
2. 按时间窗口和生命周期边界 flush。
3. 将 `TraceStore` 事件上限从“原始事件数”转为“存储事件数”，并在失败 payload 里报告压缩统计。

## 测试计划

后端测试：

- `compact_events_merges_consecutive_reasoning_delta_for_same_model_call`。
- `compact_events_does_not_merge_across_node_or_lifecycle_boundary`。
- `compact_events_merges_final_answer_output_delta`。
- `compact_events_keeps_tool_call_argument_streams_separate_by_index`。
- `request_snapshot_delta_reconstructs_append_only_messages`。
- `request_snapshot_delta_falls_back_to_keyframe_when_patch_is_not_smaller`。
- `read_trace_archive_supports_v1_and_v2`。
- `v2_archive_round_trip_matches_original_final_trace_state`。

前端测试：

- 用原始事件和 compact-expanded 事件分别跑 `applyTraceSnapshot`，最终 `nodesById/finalAnswer/latestReasoningText` 一致。
- Trace archive/replay 页面继续能打开 v1 和 v2 文件。

回归指标：

- 对现有大样例，delta run 合并后事件数量应下降 80% 以上。
- 启用 request diff 后，含多轮完整 `messages` 的样例归档字节数应下降 30% 以上。
- v1 文件读取保持不变。

## 风险与缓解

- 风险：合并 delta 后精确逐 token 回放丢失节奏。
  缓解：保留 `_compact.seq_start/seq_end/timestamp_start/timestamp_end/count`，未来需要时可展开或插值播放。

- 风险：request diff 实现错误导致展示的 messages 不完整。
  缓解：使用 hash 校验；diff 不满足阈值就 keyframe；测试对比展开前后的最终 reducer 状态。

- 风险：schema v2 破坏旧前端。
  缓解：server 的 `read_trace_archive` 对外返回当前 shape；旧 v1 文件继续支持。

- 风险：minified 文件不便人工查看。
  缓解：增加配置保留 pretty 输出，或提供后续 `trace pretty-print` 命令。

## 推荐结论

优先实现“归档 v2 压缩，读取时展开”。它能直接解决本地 trace 文件过长的问题，同时把风险限制在文件读写层。事件合并应同时覆盖 `model_call.reasoning_delta` 和 `model_output.delta`，因为样例中 output delta 才是事件数量的最大来源。`messages` 不建议做通用文本 diff，而应做结构化 keyframe/delta：保留完整关键帧，后续帧优先用前缀引用和 append/replace ops；当差异过大时自动回退完整帧。
