# Bash 智能审批与低风险策略缓存设计方案

日期：2026-05-17
状态：草案

## 背景

当前 `runBashCommand` 是默认关闭、仅 CLI Agent 暴露的本地命令执行工具。启用后，`src/bash_runner.rs` 会在每次执行前展示 `cwd`、`timeout` 和完整 `command`，并要求用户输入 `y` 或 `yes`。这保证了安全边界清晰，但在一轮复杂任务中会产生频繁审批，尤其是模型连续执行 `pwd`、`ls`、`rg`、`git status`、`cargo check` 这类低风险命令时。

用户希望：

- 低风险 bash 命令可以默认同意，减少一轮任务中反复确认的打断。
- 风险判断采用“本地规则优先，灰区再问模型”的方式。
- 低风险策略可以跨会话记住，而不是只在当前任务中生效。
- 删除文件、重置仓库、执行远端脚本等高危任务仍然必须被识别并拦截或要求确认。

## 目标

1. 在保留现有 Bash 工具安全边界的前提下，减少低风险命令的人工审批次数。
2. 引入可审计、可撤销、可过期的跨会话低风险审批策略缓存。
3. 本地规则永远优先于模型判断：明显高危或禁止命令不能被模型降级为低风险。
4. 模型只处理本地规则无法明确判断的灰区命令，并且只能生成低风险策略候选。
5. 保持 CLI-only 行为：Server 模式和浏览器 API 继续移除交互式 Bash 工具。
6. 所有自动批准、人工批准、拒绝和策略命中都能在日志或 trace 中被审计。

非目标：

- 不把 `runBashCommand` 变成 OS 级沙盒。当前 cwd root 校验仍不能阻止命令显式访问系统绝对路径。
- 不让模型直接决定执行高危命令。模型分类只是风险辅助信号，不是最终授权源。
- 不在第一阶段为所有 shell 语法实现完整 Bash AST。无法可靠解析的命令应保守升级为需要确认。
- 不把高危命令写入持久自动批准策略。

## 推荐方案

新增 Bash 智能审批层，执行顺序为：

```text
runBashCommand
  -> validate_command / resolve_cwd / resolve_timeout_ms
  -> BashRiskAssessor.classify(command, cwd, timeout)
  -> BashApprovalPolicyStore.find_matching_low_risk_policy(...)
  -> low 或 cache_hit: 自动批准并记录审计事件
  -> medium 或 uncertain: 请求用户确认，可选择记住相似低风险策略
  -> high: 强提示后请求用户确认，不允许持久自动批准
  -> blocked: 直接拒绝
  -> spawn bash
```

核心原则：

- **本地 hard rules 先跑且每次都跑**。即使策略缓存命中，也要重新检查 `rm`、`sudo`、`git reset --hard`、`curl | sh`、敏感路径写入等高危信号。
- **缓存只缓存低风险“命令形状”**。缓存命中代表这类命令可自动批准，不代表原始 bash 字符串永远可信。
- **模型只判断灰区**。本地规则返回 `low`、`high` 或 `blocked` 时不调用模型；只有 `uncertain` 或 `medium_candidate` 才调用模型。
- **策略带 cwd scope 和 TTL**。同一个 `git status` 可以跨会话自动批准，但默认只在相同 workspace root 或更窄路径下生效，并且会过期。

## 风险等级

### Low

可自动批准，并可生成持久策略：

- 只读目录和文本查看：`pwd`、`ls`、`find` 只读形式、`cat`、`head`、`tail`、`sed -n`。
- 只读搜索：`rg`、`grep`、`wc`。
- 只读 Git：`git status`、`git diff`、`git log`、`git show`、`git branch --show-current`。
- 只读项目信息：`cargo metadata`、`cargo check`、`cargo test`、`npm test`、`pnpm test`、`pnpm lint`。这些命令可能写构建缓存，但不应修改源文件或用户数据，可归为低风险或中低风险；第一阶段建议把测试/检查类命令设为 `low_with_cache_writes`，默认可自动批准但在审计原因中标明会写缓存。

### Medium

需要用户确认，确认时可选择是否记住更窄策略：

- 会安装依赖或触发网络下载：`pnpm install`、`npm install`、`cargo fetch`。
- 启动长期进程：`pnpm dev`、`cargo run -- --server`、`vite`。
- 会生成或格式化文件但通常可恢复：`cargo fmt`、`prettier --write`。
- 复杂管道、命令替换、重定向，且没有命中 hard high-risk rules。
- 本地解析器无法充分理解但模型认为低风险的命令。

### High

每次都要求用户确认，且不提供持久自动批准选项：

- 删除、覆盖、截断：`rm`、`rmdir`、`mv` 覆盖、`truncate`、`dd`。
- 权限和所有权修改：`chmod`、`chown`、`chgrp`。
- Git 破坏性操作：`git reset --hard`、`git clean`、`git checkout -- <path>`、`git restore --source`。
- 远端脚本执行：`curl ... | sh`、`wget ... | bash`、`bash <(curl ...)`。
- 权限提升：`sudo`、`su`。
- 写入敏感位置：shell profile、SSH 配置、Git 全局配置、系统目录、`~/.config` 中不属于当前项目的文件。
- 显式访问 allowed roots 之外的绝对路径，尤其是带写操作时。

### Blocked

直接拒绝执行：

- `rm -rf /`、`rm -rf ~`、删除系统关键路径。
- fork bomb 或明显资源耗尽攻击。
- 格式化磁盘、卸载卷、修改系统启动配置。
- 包含 NUL 字节或绕过现有 `validate_command` 的输入。
- 命令结构无法解析且同时命中多个高危信号。

## 组件设计

### BashRiskAssessor

新增模块 `src/bash_risk.rs`，对外提供：

```rust
pub struct BashRiskAssessor { ... }

pub struct BashRiskRequest {
    pub command: String,
    pub cwd: PathBuf,
    pub allowed_roots: Vec<PathBuf>,
    pub timeout_ms: u64,
}

pub struct BashRiskDecision {
    pub risk: BashRiskLevel,
    pub confidence: f32,
    pub reason: String,
    pub signals: Vec<BashRiskSignal>,
    pub policy_candidate: Option<BashApprovalPolicyMatcher>,
}
```

职责：

- 规范化命令：去除无意义空白，保留引号语义，提取第一个命令、argv、管道、重定向、控制操作符。
- 运行本地 allow/deny 规则。
- 对灰区命令调用模型分类器。
- 输出用于用户提示、审计和策略候选的结构化结果。

第一阶段解析策略保持保守：

- 支持常见 argv 解析、管道、`&&`、`||`、`;`、重定向的粗粒度识别。
- 遇到命令替换、process substitution、heredoc、多层 quoting 等复杂结构时，标记 `complex_shell_syntax`，至少升为 `medium`。
- 如果复杂结构同时包含危险程序或写重定向，升为 `high` 或 `blocked`。

### LocalRuleClassifier

本地规则分三层：

1. **Blocked rules**：系统破坏、fork bomb、明显危险路径，直接拒绝。
2. **High-risk rules**：删除、覆盖、权限提升、远端脚本执行、Git 破坏性操作，必须确认。
3. **Low-risk rules**：只读命令、只读 Git、测试检查类命令，可自动批准并生成策略候选。

规则要返回命中的 signal，而不只是布尔值，例如：

```json
{
  "kind": "dangerous_program",
  "value": "rm",
  "severity": "high"
}
```

这样 CLI 提示和 trace 能解释为什么要确认或拒绝。

### ModelRiskClassifier

灰区时调用现有配置的 `DeepSeekClient`，但使用一个不带 tools 的轻量 chat completion 请求。输入只包含必要上下文：

- 原始 command。
- canonical cwd。
- allowed roots。
- 本地解析摘要：programs、argv、operators、redirections、paths。
- 已命中的低/中/高风险 signals。

模型必须只返回 JSON：

```json
{
  "risk": "low",
  "confidence": 0.91,
  "reason": "Runs cargo tests in the current workspace and does not delete or overwrite user files.",
  "policy_candidate": {
    "kind": "argv_prefix",
    "program": "cargo",
    "args": ["test"]
  }
}
```

接受模型低风险结论的条件：

- `risk == "low"`。
- `confidence >= SPARROW_BASH_MODEL_LOW_RISK_THRESHOLD`，默认 `0.85`。
- 本地规则没有 `high` 或 `blocked` signal。
- `policy_candidate` 不能比本地解析出的命令范围更宽。

模型返回无效 JSON、超时、低置信度或冲突判断时，降级为 `medium` 并询问用户。

### BashApprovalPolicyStore

新增持久策略文件：

```text
~/.sparrow_agent/bash_approval_policies.json
```

Unix 下文件权限设为 `0600`。结构示例：

```json
{
  "schema_version": 1,
  "policies": [
    {
      "id": "policy_01...",
      "matcher": {
        "kind": "argv_prefix",
        "program": "git",
        "args": ["status"]
      },
      "cwd_scope": "/Users/yankaizhi/RustProjects/sparrow_agent",
      "risk": "low",
      "source": "local_rule",
      "confidence": 1.0,
      "reason": "Read-only git status command.",
      "created_at": "2026-05-17T00:00:00Z",
      "expires_at": "2026-08-15T00:00:00Z",
      "hit_count": 12,
      "last_hit_at": "2026-05-17T12:30:00Z"
    }
  ]
}
```

Matcher 类型：

- `exact_normalized_command`：最窄，适合复杂但被用户明确记住的命令。
- `argv_exact`：程序和参数完全匹配，允许 cwd scope 内复用。
- `argv_prefix`：程序和前缀参数匹配，例如 `git status`。
- `tool_family`：只用于内置本地规则明确安全的族，如 `rg` 只读搜索；模型不能创建 broad `tool_family` 策略。

缓存命中条件：

- 策略未过期。
- 当前 cwd 在 `cwd_scope` 内。
- matcher 匹配当前规范化命令。
- 重新运行本地 hard rules 后没有 high/blocked signal。

## CLI 用户体验

低风险自动批准时，终端打印简短审计行：

```text
bash approval> auto-approved low-risk command by policy policy_01...: git status
```

需要确认时，根据风险显示不同提示。

中风险：

```text
Sparrow wants to run bash command:
  risk: medium
  reason: Starts a long-running dev server.
  cwd: /path/to/workspace
  timeout: 30000 ms
  command:
    pnpm dev

Approve? [y] once / [a] approve similar low-risk policy / [n] deny
```

只有当当前决策包含 `policy_candidate` 且最终风险可降为低风险时，才显示 `[a]`。否则只显示 `[y/N]`。

高风险：

```text
Sparrow wants to run high-risk bash command:
  reason: Deletes files with rm.
  command:
    rm -rf target

Approve once? [y/N]
```

Blocked：

```text
bash approval> blocked command: attempts to delete a protected system path
```

## 配置

新增配置字段：

```rust
pub enum BashApprovalMode {
    AlwaysPrompt,
    Smart,
    NeverPrompt,
}

pub struct BashConfig {
    ...
    pub approval_mode: BashApprovalMode,
    pub approval_policy_path: PathBuf,
    pub approval_policy_ttl_days: u64,
    pub model_low_risk_threshold: f32,
}
```

环境变量：

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `SPARROW_BASH_APPROVAL_MODE` | `smart` | `always`、`smart`、`never`。`smart` 是启用 Bash 工具后的默认体验：低风险自动批准，高风险仍确认或拒绝；需要恢复旧行为时设置为 `always`。 |
| `SPARROW_BASH_APPROVAL_POLICY_PATH` | `~/.sparrow_agent/bash_approval_policies.json` | 持久策略文件位置。 |
| `SPARROW_BASH_APPROVAL_POLICY_TTL_DAYS` | `90` | 低风险策略默认过期时间。 |
| `SPARROW_BASH_MODEL_LOW_RISK_THRESHOLD` | `0.85` | 模型低风险判断的最低置信度。 |

兼容策略：

- 现有 `require_confirmation: bool` 可在实现中迁移为 `approval_mode`：`true` 对应 `Smart`，`false` 对应 `NeverPrompt`。需要逐条确认的用户通过 `SPARROW_BASH_APPROVAL_MODE=always` 显式选择旧体验。
- 实现时不再新增第二个布尔开关；统一使用 `BashApprovalMode` 表达逐条确认、智能审批和不提示三种模式，避免配置语义继续分裂。

## 数据流

1. `LocalToolProvider` 解析 `runBashCommand` 参数，调用 `BashRunner::run`。
2. `BashRunner` 完成命令长度、NUL、cwd、timeout 校验。
3. `BashRunner` 调用 `BashApprovalGate::decide`。
4. `BashApprovalGate` 调用 `BashRiskAssessor` 获取风险等级。
5. 如果本地规则不是 blocked/high，查询 `BashApprovalPolicyStore`。
6. cache hit 或 low-risk rule 命中时自动批准。
7. medium/high 时调用 CLI prompt。
8. 用户选择 `a` 时，将 policy candidate 写入 store。
9. 审批通过后执行 `/bin/bash --noprofile --norc -c command`。
10. `BashCommandOutput` 增加可选审批元数据，序列化回 tool result。

建议新增输出字段：

```rust
pub struct BashCommandOutput {
    ...
    pub approval: Option<BashApprovalSummary>,
}

pub struct BashApprovalSummary {
    pub mode: String,
    pub risk: String,
    pub approved_by: String,
    pub policy_id: Option<String>,
    pub reason: String,
}
```

这能让模型理解命令是自动批准、人工批准还是被策略命中，也方便 trace 展示。

## 错误处理

- 策略文件不存在：创建空 store。
- 策略文件 JSON 损坏：重命名为 `.corrupt.<timestamp>`，创建空 store，并提示用户。
- 策略文件无法写入：不阻止本次命令执行，但不保存策略，并打印警告。
- 模型分类失败：降级为人工确认。
- 模型返回高风险但本地规则低风险：取更高风险等级，要求确认。
- 本地规则高风险但模型低风险：本地规则胜出，要求确认。
- cache hit 后 hard rules 发现高风险：忽略缓存并要求确认或拒绝。

## 安全审计

每次审批决策都应记录：

- command 原文和 normalized form。
- cwd。
- risk level。
- decision source：`local_rule`、`model`、`policy_cache`、`user_once`、`user_policy`、`blocked`。
- matched signals。
- policy id 和 matcher。

在 CLI 模式先输出简短文本；在 traced loop 中，后续可把审批摘要放进 `tool_call.completed.output_metadata` 或 bash tool result 内。第一阶段不需要新增前端 UI，但数据结构应为后续展示留出空间。

## 测试计划

单元测试：

- `BashRiskAssessor` 将 `pwd`、`ls`、`rg foo src`、`git status` 判为 low。
- `rm -rf target` 判为 high；`rm -rf /` 判为 blocked。
- `git reset --hard`、`git clean -fd` 判为 high。
- `curl https://example.com/install.sh | sh` 判为 high。
- 含复杂 shell 结构但无危险程序的命令判为 medium。
- 本地 high signal 不能被 mock model low response 降级。

策略 store 测试：

- 新建空策略文件。
- 写入策略时权限为 `0600`。
- 过期策略不命中。
- cwd scope 外不命中。
- cache hit 后 hard rules 仍会拦截高风险变体。

集成测试：

- low-risk 命令在 smart mode 下无需 prompt 即可执行。
- medium 命令在用户拒绝时返回 `Denied`。
- 用户选择记住策略后，下一次相同命令跨 runner 实例自动批准。
- `AlwaysPrompt` 保持旧行为。
- `NeverPrompt` 仍执行 blocked hard rules，不能绕过直接拒绝。

## 迁移与文档

README 的 Bash 安全边界需要更新：

- 说明 `smart` 审批模式。
- 说明策略文件位置、TTL、删除/编辑方式。
- 说明高危命令不会被持久自动批准。
- 强调该能力不是 OS 沙盒，allowed roots 仍主要约束 cwd。

建议提供一个后续 CLI 管理入口：

```bash
cargo run -- bash-policies list
cargo run -- bash-policies revoke policy_01...
cargo run -- bash-policies clear-expired
```

第一阶段可以先不实现管理命令，但策略文件必须是可读 JSON，用户可以手动删除策略。

## 实施顺序建议

1. 新增风险等级、规则分类器和测试，不接入执行路径。
2. 新增策略 store 和 matcher 测试。
3. 在 `BashRunner` 中接入 `BashApprovalGate`，保留旧 prompt 路径兼容。
4. 接入模型灰区分类器，使用 mock 测试覆盖失败和冲突路径。
5. 扩展 `BashCommandOutput` 审批摘要。
6. 更新 README 和 Bash 工具 contract tests。
