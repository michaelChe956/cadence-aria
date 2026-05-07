# Aria TUI 工作台与可回退交互 Runtime 设计

## 文档信息

- 文档类型：技术方案
- 版本：v1.0
- 日期：2026-05-07
- 适用仓库：`cadence-aria`
- 目标命令：`aria tui`
- 相关背景文档：`cadence/analysis-docs/2026-05-07_状态记录_Aria_Fibonacci真实E2E_main本地Rust与TUI准备_v1.0.md`
- 参考项目：`/Users/michaelche/Documents/git-folder/github-folder/vibe-kanban`

## 背景

当前 Aria CLI 已具备 `daemon status/run`、`repl` 与 `task run` 入口。真实 E2E 执行会在目标 workspace 下生成 `.aria/runtime/tasks/<task_id>/`、OpenSpec change、provider run record、report、业务源码和测试文件。

Fibonacci 真实 E2E 案例暴露了 CLI 输出不足的问题：整体状态为 `blocked_by_gate`，但业务代码和测试实际通过，最终阻塞来自归档 worktask 的 write scope contract 问题。TUI 需要把任务整体状态、节点输入输出、文档沉淀物、测试结果、gate 诊断和 provider 交互过程分开展示。

用户明确选择：

- 第一版形态为 Rust 原生终端 TUI。
- 后续会做桌面端应用，因此核心状态模型和操作接口不能绑死在 TUI 渲染层。
- 回退语义采用类似 vibe-kanban 的“恢复到该点”：恢复 Git worktree 与 runtime 状态，并把目标点之后的历史标记为 dropped。
- 入口模式同时支持浏览已有任务与新建逐节点执行。

## 目标

1. 新增 `aria tui [--workspace PATH] [--task-id ID]`，提供 Rust 原生终端工作台。
2. 清晰展示每个节点的输入、输出、使用过程、provider run、报告和文档沉淀物。
3. Claude Code 和 Codex 节点执行前提供类似 Codex 的交互输入框，用户确认后才执行，确认策略可配置。
4. 支持按用户确认轮次和 provider 节点回退。默认回退到上一轮，恢复到对应 checkpoint 并隐藏之后历史。
5. TUI 只作为第一个客户端，底层抽象可被未来桌面端复用。
6. 保持现有 `aria task run --non-interactive` 行为兼容。

## 非目标

- 第一版不做 HTTP 服务或桌面应用壳。
- 第一版不做多用户协作。
- 第一版不承诺回退仓库外部副作用。
- 第一版不重写 OpenSpec 正文格式。
- 第一版不把图形化 diff 做到桌面 IDE 级别；TUI 先提供文本 diff 与文件列表。

## 总体方案

采用“核心运行模型 + TUI 渲染层分离”的方案。

TUI 不直接散读 `.aria/runtime`、不直接调用 Claude/Codex、也不把回退逻辑写在界面层。新增 Interactive Runtime Core，负责 workspace/session/turn/checkpoint/projection 模型、节点确认策略、执行控制和回退恢复。`aria tui` 只消费核心投影并调用操作 API。

分层如下：

| 层级 | 职责 | 后续桌面端复用性 |
|------|------|------------------|
| 交互客户端层 | Ratatui/Crossterm 渲染、焦点、快捷键、输入框、布局状态 | 不复用或少量复用 |
| Interactive Runtime Core | 状态投影、执行控制、确认策略、checkpoint、rollback | 完整复用 |
| 现有 Aria Runtime | ProviderAdapter、OpenSpec、artifact/report、task orchestration | 保留并拆出 step runner |

## 模块边界

### `tui`

终端 UI 模块，负责：

- 工作台布局与 tabs。
- 终端事件循环。
- 键盘快捷键和焦点管理。
- 多行确认输入框。
- 调用 `interactive` 模块提供的操作接口。

该模块不直接执行 provider，不直接修改 Git，不直接写 runtime checkpoint。

### `interactive`

新增交互运行核心模块，负责：

- `WorkspaceProjection` 构建。
- `TaskSession`、`InteractionTurn`、`NodeRun`、`RuntimeCheckpoint` 管理。
- policy preset 解析。
- 暂停、确认、继续、取消、回退操作。
- 将事件和索引持久化到 `.aria/runtime/tasks/<task_id>/`。

### `task_run`

保留当前非交互入口，并逐步把 orchestration 拆成可暂停的 step runner：

- `task run --non-interactive` 使用 `non-interactive` policy。
- TUI 使用同一 step runner，但在 provider 节点前由 `interactive` 决定是否暂停确认。

### `cross_cutting`

复用并扩展既有横切能力：

- provider adapter 和 provider run record。
- runtime event log。
- artifact validate。
- git/worktree/checkpoint 能力。

## 数据模型

### `WorkspaceProjection`

TUI 和未来桌面端的主要读取结构。建议字段：

| 字段 | 说明 |
|------|------|
| `workspace_root` | 目标 workspace 路径 |
| `active_task_id` | 当前 task |
| `sessions` | task 下的交互 session |
| `active_session_id` | 当前 session |
| `overview` | task/change/phase/status/git/provider/policy 摘要 |
| `timeline` | 节点和 turn 的时间线 |
| `artifact_index` | 文档、报告、源码、测试、JSON artifact 索引 |
| `diagnostics` | blocked/gate/provider/validation 诊断 |
| `available_actions` | 当前状态允许的操作 |

### `TaskSession`

表示同一 task 下的一条 agent 交互线。第一版可以默认只有一个 session，但结构上保留多 session 能力：

| 字段 | 说明 |
|------|------|
| `session_id` | session 标识 |
| `task_id` | 所属 task |
| `created_at` | 创建时间 |
| `status` | idle/running/blocked/completed |
| `turn_ids` | 交互轮次 |
| `active_turn_id` | 当前轮次 |

### `InteractionTurn`

每次用户确认执行 Claude Code/Codex 节点前创建一个 turn：

| 字段 | 说明 |
|------|------|
| `turn_id` | 轮次标识 |
| `session_id` | 所属 session |
| `node_id` | 对应节点 |
| `provider_type` | Claude Code 或 Codex |
| `prompt_snapshot` | 最终发送前的 prompt |
| `input_summary` | canonical inputs、context files、scope 摘要 |
| `checkpoint_before` | 执行前 checkpoint |
| `provider_run_id` | provider run |
| `output_artifact_refs` | 输出产物 |
| `changed_files` | 修改文件 |
| `status` | pending/running/completed/failed/dropped |

用户不满意时，回退到该 turn 的 `checkpoint_before`。

### `NodeRun`

节点运行记录，比 turn 更底层。自动执行节点也有 NodeRun：

| 字段 | 说明 |
|------|------|
| `node_run_id` | 节点运行标识 |
| `node_id` | N04/N05/N16 等 |
| `turn_id` | 若由用户确认触发则关联 turn |
| `provider_run_id` | provider run |
| `input_refs` | 输入 artifact/projection/context refs |
| `output_schema` | 输出 schema |
| `artifact_refs` | 输出产物 |
| `status` | started/completed/failed/blocked/dropped |
| `duration_ms` | 耗时 |
| `diagnostic_refs` | 诊断信息 |

### `RuntimeCheckpoint`

每个可回退点记录：

| 字段 | 说明 |
|------|------|
| `checkpoint_id` | checkpoint 标识 |
| `task_id` | 所属 task |
| `session_id` | 所属 session |
| `turn_id` | 关联 turn |
| `git_head` | 执行前 HEAD |
| `dirty_summary` | 执行前 dirty 状态摘要 |
| `state_snapshot_ref` | runtime state 快照 |
| `projection_snapshot_ref` | projection 快照 |
| `artifact_boundary` | artifact index 边界 |
| `provider_run_boundary` | provider run 边界 |
| `node_run_boundary` | node run 边界 |
| `created_at` | 创建时间 |

回退后，将目标 checkpoint 之后的 NodeRun、InteractionTurn、ProviderRunRecord、ArtifactIndexEntry 标记为 `dropped=true`。

### `ArtifactIndexEntry`

统一索引输入输出和文档沉淀物：

| 字段 | 说明 |
|------|------|
| `artifact_ref` | 产物引用 |
| `artifact_kind` | spec/design/plan/report/source/test 等 |
| `producer_node` | 生产节点 |
| `path` | 文件路径 |
| `summary` | 简要说明 |
| `status` | active/superseded/dropped/candidate |
| `content_type` | markdown/json/source/test/log |
| `traceability_refs` | 追踪引用 |

## 存储布局

沿用 `.aria/runtime/tasks/<task_id>/`，新增交互索引：

```text
.aria/runtime/tasks/<task_id>/
  state.json
  projection.json
  sessions/
    <session_id>.json
  turns/
    <turn_id>.json
  node-runs/
    <node_run_id>.json
  checkpoints/
    <checkpoint_id>.json
  artifacts/
  reports/
  provider-runs/
  logs/
```

`projection.json` 是可重建缓存。若损坏，可从 sessions、turns、node-runs、artifacts、reports 和 provider-runs 重建。

## 执行流程

`aria tui` 支持两种模式：

| 模式 | 说明 |
|------|------|
| browse | 默认打开最近或指定 task，浏览已有运行结果和诊断 |
| run | 新建任务或继续未完成任务，进入逐节点执行模式 |

逐节点执行流程：

1. 用户输入 request、workspace、change-id、provider、policy preset。
2. `ExecutionController` 解析下一节点。
3. 根据 policy 判断是否暂停确认。
4. 需要确认时，创建 `InteractionTurn` 和 `RuntimeCheckpoint`。
5. TUI 底部输入框展示 prompt、input summary、output schema、allowed write scope。
6. 用户可编辑补充指令并确认执行。
7. Provider 执行后写入 provider run、artifact/report、node event、node run。
8. 刷新 `WorkspaceProjection`。
9. 若进入 gate 或 blocked 状态，Diagnostics Panel 分类展示原因和建议。

## Policy Preset

| Preset | 行为 |
|--------|------|
| `manual-all` | 所有 provider 节点都暂停确认 |
| `manual-write` | 默认 preset。写代码、写文档、改状态的节点暂停确认；review/testing 自动 |
| `auto-review` | 规划和编码确认；review/testing/final summary 自动 |
| `non-interactive` | 接近当前 CLI 行为，不暂停确认 |

用户可对单个节点临时 override。

## 回退流程

回退语义采用“恢复到该点”：

1. 用户选择“上一轮”或指定 turn/provider run。
2. TUI 展示回退确认框：
   - 将恢复到哪个 checkpoint。
   - 将标记 dropped 的 turn/node/provider/artifact 数量。
   - 当前 Git dirty 状态。
   - 是否会执行 hard reset。
3. 若 checkpoint 缺失或 dirty 状态不安全，阻止静默回退，要求用户显式确认。
4. 执行 Git reset 到 checkpoint 的 `git_head`。
5. 恢复 runtime state/projection 边界。
6. 标记后续历史为 `dropped=true`。
7. 回到底部输入框，允许用户修改 prompt 并重新执行。

第一版回退承诺只覆盖目标 workspace 内 Git worktree 与 Aria runtime 索引。Provider 在仓库外产生的副作用不纳入回退。

## TUI 信息架构

采用工作台主导布局。

### 顶部状态栏

展示：

- workspace
- task_id
- change_id
- phase
- status
- policy preset
- provider mode
- git branch/head/dirty

### 主视图 tabs

| Tab | 内容 |
|-----|------|
| Overview | task/change/workspace/provider/policy/phase/status/current_worktask/最近诊断 |
| Timeline | 节点、turn、provider run、重复节点、rework 次数、dropped 标记 |
| IO | canonical inputs、context files、allowed write scope、prompt、schema、outputs |
| Artifacts | OpenSpec、Aria artifacts、reports、source、tests、logs |
| Changes | 当前 worktree diff、turn changed_files、checkpoint 前后差异 |

### 底部 Action Bar

Claude Code 和 Codex 节点显示多行确认输入框。支持：

- 编辑 prompt。
- 确认执行。
- 暂停或继续自动执行。
- 回退上一轮。
- 切换 policy preset。
- 查看当前节点输入摘要。

### 右侧 Diagnostics Panel

分类展示：

- provider error
- gate blocked
- validation failed
- checkpoint unsafe
- test failed
- code review blocking
- write scope/contract conflict

Fibonacci 样本应展示为：

```text
E2E overall: blocked_by_gate
Business code: generated
Unit tests: passed
Coverage gate: passed
Archive worktask: failed
Root cause: cadence/ write scope missing
Reason: rework_limit_exceeded
Next node: X08
```

## 错误处理

### `provider_error`

包括：

- provider 命令缺失
- 未登录或鉴权失败
- 权限拒绝
- 超时
- 结构化输出解析失败
- 非零 exit code

TUI 显示 provider、node、stderr 摘要、错误码、可重试性。

### `gate_blocked`

包括：

- `blocked_by_gate`
- `rework_limit_exceeded`
- write scope contract 阻塞
- policy/manual intervention hold

TUI 显示触发节点、下一节点、原因、建议操作。

### `validation_failed`

包括：

- artifact schema 校验失败
- phase1 profile 校验失败
- traceability 校验失败
- OpenSpec bundle 校验失败

TUI 显示具体 artifact、字段路径、校验消息。

### `checkpoint_unsafe`

包括：

- checkpoint 缺失
- 当前 worktree dirty 且未确认
- 目标 checkpoint 后有未纳入索引的文件
- Git reset 失败

此类错误必须弹确认或阻止操作，不允许静默 reset。

## 兼容性

现有入口保持兼容：

```text
aria task run --non-interactive
aria daemon status
aria daemon run
aria repl
```

新增入口：

```text
aria tui --workspace <path>
aria tui --workspace <path> --task-id <task_id>
```

后续可增加辅助命令：

```text
aria task status --workspace <path> --task-id <task_id> --json
aria task rollback --workspace <path> --task-id <task_id> --to <checkpoint_id>
```

第一版不要求暴露完整辅助命令，优先给 TUI core 调用。

## 测试策略

遵循 TDD。

### Projection 单元测试

使用 fixture runtime store 输入，断言：

- Overview 字段准确。
- Timeline 节点顺序和重复节点识别准确。
- IO 中输入输出和 schema 准确。
- Artifacts 索引 producer/status/path 准确。
- Diagnostics 能识别 Fibonacci `blocked_by_gate` 样本。

### Policy/Step Runner 测试

使用 fake provider 验证：

- `manual-all` 暂停所有 provider 节点。
- `manual-write` 暂停写入节点，自动执行 review/testing。
- `auto-review` 按定义暂停规划和编码。
- `non-interactive` 不暂停确认。

### Checkpoint/Rollback 测试

在临时 Git repo 中创建多个 turn，断言回退后：

- worktree HEAD 恢复到 checkpoint。
- runtime state 恢复到目标边界。
- 后续 turn/node/provider/artifact 标记 `dropped=true`。
- projection 刷新后不把 dropped 历史作为 active 状态。

### TUI Reducer 测试

不依赖真实终端，测试：

- tab 切换。
- 节点选择。
- 输入框编辑。
- 确认执行。
- 回退确认。
- 错误展示。
- policy 切换。

## 验收标准

1. `aria tui` 能浏览已有 task。
2. `aria tui` 能新建 task 并进入逐节点执行模式。
3. 每个节点的输入、输出、使用过程和文档沉淀物可查看。
4. Claude Code 和 Codex 节点执行前可编辑 prompt 并确认。
5. 用户可回退上一轮，回退后后续历史标记 dropped。
6. Fibonacci 样本能清晰区分业务测试通过与归档 gate 阻塞。
7. `aria task run --non-interactive` 行为不回归。

## 实施顺序建议

1. 先补 runtime fixture 与 projection 测试。
2. 实现 `WorkspaceProjection` 和 artifact/diagnostic 索引。
3. 实现 policy 与 step runner 暂停点。
4. 实现 checkpoint 与 rollback。
5. 实现 TUI reducer。
6. 接入 ratatui/crossterm 工作台布局。
7. 用 Fibonacci 样本做端到端验收。
