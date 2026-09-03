# work-item-plan-advance Specification

## Purpose

`advance` 是把 durable Confirmed 的 workitem plan 推进到 coding 就绪的显式幂等编排接口：建立（或恢复）该 plan 唯一的 WorkItemGroup coding attempt 与其全部 durable 绑定，返回 group coding workspace 入口；它只负责"就绪"，不启动 coding provider。

## Requirements

### Requirement: advance 幂等编排（REQ-ADV-01）

系统 SHALL 提供 typed `advance` 入站命令（含客户端生成的 `command_id`）。前置校验（plan durable status 为 Confirmed、子 WorkItem session 已存在、无 active compile/revision）SHALL 仅适用于**首次** advance；幂等命中（同 `command_id` 或同 `plan_id`，见 REQ-ADV-02）SHALL 优先于前置校验。首次 advance 前置任一不满足 SHALL 返回 `advance_rejected` 且不产生任何副作用。前置满足后 SHALL 依序执行：持久化 `AdvanceRecord`（`command_id`、`plan_id`、不可变 `plan_revision_id`、状态）→ 执行 group 初始化 durable journal → 创建或恢复唯一的 WorkItemGroup coding attempt（其持久化字段 `admission_kind` SHALL 置为 `sc_advance`，作为依赖门适用范围与隔离测试的唯一判据；SHALL NOT 从 `flow_kind`、ID 命名或 `work_item_group_id` 是否存在推断），建立 plan binding、worktree lock 与按依赖拓扑序生成的 coding units → 置 `Ready` 并发出 `advance_completed`（含 `attempt_id` 与 workspace 入口引用）。`advance` SHALL NOT 启动 coding provider；coding 启动保持既有 `StartCoding` 语义。

#### Scenario: Confirmed plan 推进到 coding 就绪

- **WHEN** plan 已 durable Confirmed 且客户端发送 `advance { command_id }`
- **THEN** 系统建立该 plan 的唯一 group attempt 与全部 durable 绑定，返回 workspace 入口；此时无任何 coding provider 被启动

#### Scenario: 前置不满足零副作用拒绝

- **WHEN** plan 尚未 Confirmed 或存在 active compile/revision 时收到 `advance`
- **THEN** 系统返回 `advance_rejected`（含原因），不创建 AdvanceRecord、不改任何状态

### Requirement: advance 幂等与 attempt 唯一性（REQ-ADV-02）

同一 `command_id` 的重复 `advance` SHALL 返回同一 `AdvanceRecord` 与同一 `attempt_id`；同一 `plan_id` 以不同 `command_id` 重复 advance SHALL 命中唯一性约束并返回同一 group attempt，SHALL NOT 创建第二个 attempt。已存在 attempt 的状态矩阵：`Initializing`/`Ready`/`Running`/`AwaitingPlanAmendment`/`Completed` SHALL 返回该 attempt 的 durable 状态；`Failed`/`Aborted` SHALL 返回原 attempt 及其失败/中止原因，SHALL NOT 隐式创建第二个 attempt；显式 restart/retry 语义另行定义。初始化中断 SHALL 由 durable journal 恢复，恢复 SHALL NOT 重新分配另一套 attempt/unit。`advance_completed` 仅表示 group workspace 就绪，SHALL NOT 被解释为 coding 已完成。

#### Scenario: 同一命令键重发

- **WHEN** 客户端在收到 `advance_completed` 前以同一 `command_id` 重发 `advance`
- **THEN** 系统返回首次创建的 `AdvanceRecord` 与其 `attempt_id`，不重复初始化、不新建 attempt

#### Scenario: 不同命令键重复推进同一 plan

- **WHEN** 同一 `plan_id` 先后收到两个不同 `command_id` 的 `advance`
- **THEN** 两次均返回同一 `attempt_id`，group attempt、units、worktree lock 均只有一份

#### Scenario: 已有失败/中止 attempt 时不隐式重建

- **WHEN** 同一 `plan_id` 已存在 `Failed` 或 `Aborted` 状态的 attempt，客户端再次发送 `advance`（无论 `command_id`）
- **THEN** 系统返回该原 attempt 及其失败/中止原因，不创建第二个 attempt；重试需另立显式语义（不在本 change 范围）

#### Scenario: 初始化中断恢复

- **WHEN** advance 在 group 初始化 journal 执行中崩溃后恢复
- **THEN** 系统从 journal checkpoint 续跑到 Ready，attempt/unit 与中断前为同一套，事件前缀不变

### Requirement: SC 与 legacy 入口隔离（REQ-ADV-03）

单候选（SC）plan SHALL 仅经 `advance` 准入 group coding，SHALL NOT 使用 legacy 逐段 plan 决策协议进入 coding；legacy plan 的既有 group coding 入口与旧协议 SHALL 原样保留。两条入口若共享 group 初始化逻辑，则 SHALL 共享 attempt 唯一性、plan binding、worktree lock 与 replay 四道约束。`flow_kind` SHALL 保持不可变，不因入口不同而静默切换。在 REQ-WSC-07 退役门未全部满足前，旧协议 SHALL NOT 删除。

#### Scenario: SC plan 绕过 advance 直接进入 coding 被拒

- **WHEN** 对 SC plan 的子 session 直接调用 coding 启动入口而未经过 `advance`
- **THEN** 系统拒绝并提示先执行 advance；session 与 attempt 状态不变

#### Scenario: legacy 入口行为零变化

- **WHEN** legacy plan 经既有入口进入 group coding
- **THEN** 其行为与本 change 前完全一致：不经过 advance、不适用 REQ-GCE-01 依赖门、session `flow_kind` 保持原值不变
