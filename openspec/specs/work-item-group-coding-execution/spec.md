# work-item-group-coding-execution Specification

## Purpose

WorkItemGroup coding attempt 的执行编排：以依赖图为权威顺序来源的 unit 就绪门、共享 worktree 单 active unit 不变式、失败留在同一 attempt 内处理、plan 缺陷经 amendment 链恢复。

## Requirements

### Requirement: 依赖就绪门（REQ-GCE-01）

**适用范围**：仅适用于 `admission_kind = sc_advance`（经 `advance` 创建）的 WorkItemGroup coding attempt；该字段为 attempt 创建时写入的持久化辨识字段，依赖门与隔离测试仅以此为准，SHALL NOT 从 `flow_kind`、ID 命名或 `work_item_group_id` 是否存在推断。`admission_kind = legacy_group`（legacy 入口创建）的 attempt SHALL 保持既有 order_index 选择行为，并有隔离回归测试锁死。

在适用范围内，unit 启动 SHALL 以依赖图为权威顺序来源，`order_index` SHALL 仅作确定性排序与拓扑序 tie-breaker。一个 unit 仅当其全部直接依赖满足以下条件后才允许启动：依赖 unit 已 Completed、对应 handoff 已发布、handoff revision 与当前 plan binding 匹配。依赖未就绪的 unit SHALL 保持 Pending，SHALL NOT 被跳过执行；存在 pending unit 但全部未就绪时 attempt SHALL 进入等待态而非失败。未知依赖、环、自依赖、handoff 与 binding 不匹配 SHALL fail-closed。共享 worktree 下同一时间 SHALL 至多一个 active unit。

#### Scenario: 依赖未就绪不跳过

- **WHEN** unit B 依赖 unit A，A 尚未 Completed 或 handoff 未发布
- **THEN** B 保持 Pending，调度器不得启动 B；当 A 完成并发布匹配 handoff 后 B 方可启动

#### Scenario: 全部 pending 未就绪进入等待态

- **WHEN** 存在 pending units 但全部直接依赖未就绪
- **THEN** attempt 进入等待态并持久化原因，不失败、不选择任何 unit

#### Scenario: handoff 与 plan binding 不匹配 fail-closed

- **WHEN** 依赖 unit 已 Completed 但其 handoff revision 与当前 plan binding 不匹配
- **THEN** 系统 fail-closed 并持久化原因，不启动依赖方 unit

#### Scenario: 依赖图异常 fail-closed

- **WHEN** units 之间存在环、未知依赖引用或自依赖
- **THEN** 系统在启动前 fail-closed 并持久化原因，不选择任何 unit 执行

#### Scenario: legacy attempt 不适用依赖门

- **WHEN** legacy 入口创建的 group attempt 推进 unit
- **THEN** 其 unit 选择行为与本 change 前完全一致（order_index 顺序），依赖就绪门不介入

### Requirement: 失败留在同一 attempt 内（REQ-GCE-02）

coding 失败 SHALL 在同一 group attempt 内处理：provider 暂时失败 SHALL 沿用既有有界重试重试同一 unit；需要人工处理 SHALL 进入既有 blocked/人工门状态并在同一 group workspace 呈现；用户主动中止 SHALL 将 attempt 置 Aborted 并保留全部 units、日志、commit 与 durable 事件。任何失败路径 SHALL NOT 创建第二个 group attempt，SHALL NOT 从 SC 子 session 猜测执行位置。

#### Scenario: unit 失败重试不新建 attempt

- **WHEN** 某 unit 的 provider 运行失败且重试预算未耗尽
- **THEN** 系统在同一 attempt 内重试该 unit，attempt/unit 身份与 durable 事件保持连续

### Requirement: plan 缺陷走 amendment 链（REQ-GCE-03）

coding 阶段发现 plan 缺陷时，系统 SHALL 将当前 unit 标记 `BlockedByPlanDefect`、attempt 进入 `AwaitingPlanAmendment`（复用现有 `CodingAttemptStatus`，不新建状态），并写入 `PlanAmendmentContext` durable 关联记录（`plan_session_id`、`group_attempt_id`、触发 `unit_id`、触发 `finding_id`、`previous_plan_revision_id`、`resume_target`）；原 plan session 保持唯一人工门宿主，不创建第二个人工门。amendment 期间的人工反馈回合以 amendment 上下文重开原 SC plan session 的对话式人工门，预算接续该 session 快照的 `manual_repairs_remaining`，`HumanGateTurn` 归属该 plan session。人工反馈触发 SC revision provider（REQ-CG-03 同源），新候选经 SC compiler 与 canonical validator 形成新 plan revision 后，系统 SHALL 更新 group attempt 的 plan binding 并按既有 resume target（Reexecute/Revalidate/AwaitHandoff）从原 attempt 恢复 Coding；amendment approve 沿用「修订校验通过→新 plan revision→binding 更新」链，不复用首次 approve 的 compile 入口。新 plan revision 与当前 attempt 无法安全兼容时 SHALL durable fail-closed，SHALL NOT 静默切换到另一 plan 或 legacy coding 路径。amendment 期间断线恢复由原 plan session 负责。

#### Scenario: plan 缺陷修订后回到 coding

- **WHEN** unit 因 plan 缺陷被标记 `BlockedByPlanDefect`，人工经 SC revision 路径产出通过校验的新 plan revision
- **THEN** attempt 更新 plan binding，按 resume target 恢复原 unit 的 Coding，不新建 attempt

#### Scenario: amendment 回合挂在原 plan session

- **WHEN** attempt 处于 `AwaitingPlanAmendment` 且人在原 plan session 的对话门（amendment 上下文）发送 `human_gate_feedback`
- **THEN** 回合记录与预算扣减归属该 plan session（接续其 `manual_repairs_remaining`），group attempt 侧不产生新预算账；不创建第二个人工门实例；断线重连由原 plan session 恢复 amendment 上下文与 in-flight turn

#### Scenario: amendment 批准后恢复原 attempt

- **WHEN** amendment 修订产出通过校验的新 plan revision 且人 approve
- **THEN** attempt 的 plan binding 更新为新 `plan_revision_id`（原 `previous_plan_revision_id` 保留在 `PlanAmendmentContext`），coding attempt 保持同一实例并按 `resume_target` 恢复，不新建 attempt、不重新进入首次 approve 的 compile 链；新 revision 与原 attempt 不兼容时持久化失败并进入人工处置，不静默切换路径

### Requirement: Work Item 进展与成果只读投影（REQ-GCE-04）

group coding workspace 是 coding 的唯一观察面；系统 SHALL 提供每个 `logical_work_item_id` 的只读投影，内容至少包括：unit 状态与当前 stage、当前/最终 commit、code review 结果、handoff 结果、失败/阻塞原因、plan revision binding、以及 group 聚合进度。该投影 SHALL 从 group attempt/unit 确定性派生，SHALL NOT 形成第二套状态机；SC 的 per-WI session 不得作为进展事实来源。验收只要求既有 group workspace 数据/API 能读取上述事实，前端 UI 不在本 change 范围。

#### Scenario: 每个 Work Item 的进展与成果可见

- **WHEN** 客户端读取 SC-linked group workspace 的数据面（既有 group attempt/unit API）
- **THEN** 每个 `logical_work_item_id` 可读到 unit 状态/当前 stage、当前与最终 commit、code review 结果、handoff 结果、失败或阻塞原因、plan revision binding，以及 group 聚合进度；数据均来自 group attempt/unit 派生，SC per-WI session 不出现在来源链中

### Requirement: 修订出版中窗崩溃恢复（REQ-GCE-05）

修订出版过程在任一 checkpoint 崩溃后，同 command 的重新确认 SHALL 从既有出版 journal 重放恢复（校验 confirmation 与 manifest 一致），不因重建身份不匹配而 fail-closed；既有 journal 与本次冲突时 SHALL 以显式冲突拒绝。

#### Scenario: 中窗崩溃后重确认恢复出版

- **WHEN** 修订出版在 work item revision 已发布、plan revision 未发布的中间窗口崩溃，客户端重启后重发同一确认命令
- **THEN** 系统发现既有出版 journal 即按重放分支校验并继续完成出版，group attempt 保持同一身份恢复
