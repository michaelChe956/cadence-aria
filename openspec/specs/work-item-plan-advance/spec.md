# work-item-plan-advance Specification

## Purpose

`advance` 是把 durable Confirmed 的 workitem plan 推进到 coding 就绪的显式幂等编排接口：建立（或恢复）该 plan 的 WorkItemGroup coding attempt 集与全部 durable 绑定——单一 target 场景为唯一 attempt（行为零变化），多 target 场景按 target 分流为每 `(plan, target)` 恰一个 target-attempt（分流契约以 `multi-target-group-coding` 为唯一来源）——返回 group coding workspace 入口。它只负责"就绪"（Ready 即止），不启动 coding provider：coding provider 启动的唯一入口是显式 `StartCoding` 入站命令，SC admission 的 target-attempt 在经 advance 置 Ready 之前收到 StartCoding 会被 `SC_CODING_REQUIRES_ADVANCE` fail-closed 拒绝（per-attempt 适用）；显式 opt-in auto 启动通道为条件 defer 项（触发条件=autopilot/驾驶舱真实 auto 需求，红线五条以 REQ-ADV-05 为唯一来源）。

## Requirements

### Requirement: advance 幂等编排（REQ-ADV-01）

系统 SHALL 提供 typed `advance` 入站命令（含客户端生成的 `command_id`）。前置校验（plan durable status 为 Confirmed、子 WorkItem session 已存在、无 active compile/revision）SHALL 仅适用于**首次** advance；幂等命中（同 `command_id` 或同 `plan_id`，见 REQ-ADV-02）SHALL 优先于前置校验。首次 advance 前置任一不满足 SHALL 返回 `advance_rejected` 且不产生任何副作用。前置满足后 SHALL 依序执行：持久化 `AdvanceRecord`（`command_id`、`plan_id`、不可变 `plan_revision_id`、状态；每 plan 恰一条，与 target-attempt 的绑定关系按 additive 方式承载——单 target 场景 `attempt_id` 单值承载语义不变）→ 执行 group 初始化 durable journal → 创建或恢复该 plan 的 coding attempt 集：单一 target（含无 target 且 selection focus 唯一）SHALL 建立唯一 WorkItemGroup coding attempt（行为与本变更前完全一致）；多 target SHALL 按 target 分流为每 `(plan, target)` 恰一个 target-attempt，各 attempt 的持久化字段 `admission_kind` SHALL 置为 `sc_advance`（作为依赖门适用范围与隔离测试的唯一判据；SHALL NOT 从 `flow_kind`、ID 命名或 `work_item_group_id` 是否存在推断），各自建立 plan binding、worktree lock 与按依赖拓扑序生成的该 target coding units → 置 `Ready` 并发出 `advance_completed`（含 target-attempt 引用——单 target 为单 `attempt_id`，多 target 为 target-attempt 引用集，与 workspace 入口引用）。`advance` SHALL NOT 启动 coding provider；coding 启动保持既有 `StartCoding` 语义（per-attempt 适用）。

#### Scenario: Confirmed plan 推进到 coding 就绪

- **WHEN** plan 已 durable Confirmed 且客户端发送 `advance { command_id }`
- **THEN** 系统建立该 plan 的 coding attempt 集与全部 durable 绑定，返回 workspace 入口；此时无任何 coding provider 被启动

#### Scenario: 多 target plan 分流推进

- **WHEN** plan 的 units 涉及多个目标仓库且客户端发送 `advance { command_id }`
- **THEN** 系统按 target 分流为每 `(plan, target)` 恰一个 target-attempt（各持该 target 冻结快照），`advance_completed` 载荷含 target-attempt 引用集

#### Scenario: 前置不满足零副作用拒绝

- **WHEN** plan 尚未 Confirmed 或存在 active compile/revision 时收到 `advance`
- **THEN** 系统返回 `advance_rejected`（含原因），不创建 AdvanceRecord、不改任何状态

### Requirement: advance 幂等与 attempt 唯一性（REQ-ADV-02）

同一 `command_id` 的重复 `advance` SHALL 返回同一 `AdvanceRecord` 与同一 target-attempt 集（单 target 场景为同一 `attempt_id`）；同一 `plan_id` 以不同 `command_id` 重复 advance SHALL 命中唯一性约束并返回同一 target-attempt 集，SHALL NOT 为任何 `(plan, target)` 创建第二个 attempt。attempt 唯一性语义为 per-`(plan, target)`：对每 `(plan, target)`，已存在 target-attempt 的状态矩阵 `Initializing`/`Ready`/`Running`/`AwaitingPlanAmendment`/`Completed` SHALL 返回该 target-attempt 的 durable 状态；`Failed`/`Aborted` SHALL 返回原 target-attempt 及其失败/中止原因，SHALL NOT 隐式创建第二个 attempt；显式 restart/retry 语义另行定义。初始化中断 SHALL 由 durable journal 恢复，恢复 SHALL NOT 重新分配另一套 attempt/unit（每 `(plan, target)` 的 target-attempt 与中断前为同一套）。`advance_completed` 仅表示 group workspace 就绪，SHALL NOT 被解释为 coding 已完成。

#### Scenario: 同一命令键重发

- **WHEN** 客户端在收到 `advance_completed` 前以同一 `command_id` 重发 `advance`
- **THEN** 系统返回首次创建的 `AdvanceRecord` 与其 target-attempt 集（单 target 为同一 `attempt_id`），不重复初始化、不新建 attempt

#### Scenario: 不同命令键重复推进同一 plan

- **WHEN** 同一 `plan_id` 先后收到两个不同 `command_id` 的 `advance`
- **THEN** 两次均返回同一 target-attempt 集（每 `(plan, target)` 恰一个），target-attempts、units、worktree lock 均只有一份

#### Scenario: 已有失败/中止 attempt 时不隐式重建

- **WHEN** 某 `(plan, target)` 已存在 `Failed` 或 `Aborted` 状态的 target-attempt，客户端再次发送 `advance`（无论 `command_id`）
- **THEN** 系统返回该原 target-attempt 及其失败/中止原因，不为该 `(plan, target)` 创建第二个 attempt；重试需另立显式语义（不在本 change 范围）

#### Scenario: 初始化中断恢复

- **WHEN** advance 在 group 初始化 journal 执行中崩溃后恢复
- **THEN** 系统从 journal checkpoint 续跑到 Ready，每 `(plan, target)` 的 target-attempt/unit 与中断前为同一套，事件前缀不变

#### Scenario: 单 target 场景唯一性零变化

- **WHEN** plan 仅涉及单一 target 且重复 advance 或初始化中断恢复
- **THEN** attempt 唯一性、幂等与恢复行为与本变更前完全一致（唯一 attempt、同一 `attempt_id`）

### Requirement: SC 编译源草稿落盘（REQ-ADV-04）

SC 初始计划编译 SHALL 在任何 work item revision 可见之前，将编译输入中的 accepted draft records 持久化到草稿库，使 advance 的权威绑定解析可溯源；恢复重放 SHALL 幂等（同 draft_id 同内容覆盖写）。

#### Scenario: 真实链 SC confirm 后 advance 可解析溯源

- **WHEN** 单候选计划经真实批准链 Confirmed 后客户端调用 advance
- **THEN** 权威绑定解析从草稿库命中每个 work item revision 的 source draft，group initialization 通过，不因溯源缺失拒绝

#### Scenario: 编译恢复重放幂等

- **WHEN** SC 编译在提交段崩溃后经恢复重放
- **THEN** 草稿库中每个 draft record 恰一份且与首次落盘字节一致，无重复无冲突

### Requirement: SC 准入唯一入口与共享初始化约束（REQ-ADV-06）

单候选（SC）plan SHALL 仅经 `advance` 准入 group coding，系统 SHALL NOT 存在任何其他 coding 准入入口。旧协议（legacy 逐段 plan 决策协议及其 group coding 入口）已按 `legacy-protocol-retirement` 退役删除：legacy 入口不复存在，已删除消息类型收到时 SHALL 返回 protocol error 且零副作用。两条历史入口共享的 group 初始化逻辑 SHALL 保持 attempt 唯一性、plan binding、worktree lock 与 replay 四道约束；`flow_kind` 字段仅作为存量 durable 记录的只读历史值保留（新会话一律单候选流，处置细则以 `legacy-protocol-retirement` REQ-RET-03 为唯一来源）。

#### Scenario: SC plan 绕过 advance 直接进入 coding 被拒

- **WHEN** 对 SC plan 的子 session 直接调用 coding 启动入口而未经过 `advance`
- **THEN** 系统拒绝并提示先执行 advance；session 与 attempt 状态不变

#### Scenario: 已删除 legacy 入口协议错误拒绝

- **WHEN** 客户端以已删除的 legacy 逐段 plan 决策消息尝试进入 group coding
- **THEN** 系统返回 protocol error 且零副作用，不存在 legacy 准入路径

#### Scenario: flow_kind 仅作只读历史值

- **WHEN** 读取存量 durable 记录中带 legacy 取值的 `flow_kind` 字段
- **THEN** 系统以只读历史值呈现（不路由到 legacy 引擎、不报错），新会话创建一律单候选流

### Requirement: advance 到 Ready 即止与 coding 启动唯一入口（REQ-ADV-05）

`advance` 的完成态 SHALL 为 `Ready`（group workspace 就绪），advance 本身及 `advance_completed` 事件 SHALL NOT 启动任何 coding provider。coding provider 启动的唯一入口 SHALL 为显式 `StartCoding` 入站命令（人工触发）；advance 完成、事件编排、恢复重放或任何其他服务端动作 SHALL NOT 隐式启动、批量启动或随附启动 coding provider。SC admission 的 coding attempt 在经 advance 置 `Ready` 之前收到 `StartCoding` SHALL fail-closed 拒绝并返回错误码 `SC_CODING_REQUIRES_ADVANCE`，attempt 与 session 状态不变。显式 opt-in auto 启动通道为条件 defer 项（触发条件=autopilot/驾驶舱真实 auto 需求），本 requirement 不定义其实施；若未来立项 MUST 满足全部红线：opt-in 持久化 run_policy、默认 off、per-attempt 单发、绝不批量、不动唯一人工门，且 MUST 另行 change 显式定义，MUST NOT 以隐式或随 advance 形态落地。

#### Scenario: advance 完成不启动 provider

- **WHEN** advance 执行到完成态 `Ready` 并发出 `advance_completed`
- **THEN** 无任何 coding provider 被启动（显式 `StartCoding` 之前 attempt 保持就绪不运行）

#### Scenario: Ready 前 StartCoding 被守卫拒绝

- **WHEN** SC admission 的 attempt 尚未经 advance 置 `Ready` 时收到 `StartCoding`
- **THEN** 系统以错误码 `SC_CODING_REQUIRES_ADVANCE` fail-closed 拒绝，attempt 与 session 状态不变，无 provider 启动

#### Scenario: 编排动作不隐式启动 provider

- **WHEN** advance 完成、`advance_completed` 事件编排、恢复重放或任何服务端自动动作执行
- **THEN** coding provider 仅在显式 `StartCoding` 入站命令后启动，无隐式、批量或随附启动路径

#### Scenario: auto 通道红线条件 defer 登记

- **WHEN** 未来出现 autopilot/驾驶舱真实 auto 需求并拟启动 coding provider
- **THEN** 只能以另行 change 显式定义 opt-in auto 通道且满足全部红线五条（opt-in 持久化 run_policy、默认 off、per-attempt 单发、绝不批量、不动唯一人工门）；在此之前任何 auto 启动路径不存在
