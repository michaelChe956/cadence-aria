# work-item-group-coding-execution Specification

## Purpose

WorkItemGroup coding attempt 的执行编排：以依赖图为权威顺序来源的 unit 就绪门、共享 worktree 单 active unit 不变式、失败留在同一 attempt 内处理、plan 缺陷经 amendment 链恢复。

## Requirements

### Requirement: 依赖就绪门（REQ-GCE-01）

**适用范围**：仅适用于 `admission_kind = sc_advance`（经 `advance` 创建）的 WorkItemGroup coding attempt；该字段为 attempt 创建时写入的持久化辨识字段，依赖门与隔离测试仅以此为准，SHALL NOT 从 `flow_kind`、ID 命名或 `work_item_group_id` 是否存在推断

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

#### Scenario: legacy attempt 保持既有选择行为

- **WHEN** `admission_kind = legacy_group`（legacy 入口创建）的 attempt 推进 unit
- **THEN** `admission_kind = legacy_group`（legacy 入口创建）的 attempt SHALL 保持既有 order_index 选择行为，并有隔离回归测试锁死

#### Scenario: 依赖图为权威顺序来源

- **WHEN** 在适用范围内启动 unit
- **THEN** 在适用范围内，unit 启动 SHALL 以依赖图为权威顺序来源，`order_index` SHALL 仅作确定性排序与拓扑序 tie-breaker

#### Scenario: 依赖全部满足方可启动

- **WHEN** 判断某 unit 的全部直接依赖是否满足
- **THEN** 一个 unit 仅当其全部直接依赖满足以下条件后才允许启动：依赖 unit 已 Completed、对应 handoff 已发布、handoff revision 与当前 plan binding 匹配

#### Scenario: 依赖未就绪保持 Pending

- **WHEN** 存在依赖未就绪的 unit
- **THEN** 依赖未就绪的 unit SHALL 保持 Pending，SHALL NOT 被跳过执行

#### Scenario: 全部未就绪进入等待态

- **WHEN** 存在 pending unit 但全部未就绪
- **THEN** 存在 pending unit 但全部未就绪时 attempt SHALL 进入等待态而非失败

#### Scenario: 依赖图异常条件 fail-closed

- **WHEN** 依赖图存在未知依赖、环、自依赖或 handoff 与 binding 不匹配
- **THEN** 未知依赖、环、自依赖、handoff 与 binding 不匹配 SHALL fail-closed

#### Scenario: 共享 worktree 单 active unit

- **WHEN** 共享 worktree 下多个 unit 就绪
- **THEN** 共享 worktree 下同一时间 SHALL 至多一个 active unit

### Requirement: 失败留在同一 attempt 内（REQ-GCE-02）

coding 失败 SHALL 在同一 group attempt 内处理：provider 暂时失败 SHALL 沿用既有有界重试重试同一 unit；需要人工处理 SHALL 进入既有 blocked/人工门状态并在同一 group workspace 呈现；用户主动中止 SHALL 将 attempt 置 Aborted 并保留全部 units、日志、commit 与 durable 事件。任何失败路径 SHALL NOT 创建第二个 group attempt，SHALL NOT 从 SC 子 session 猜测执行位置。

#### Scenario: unit 失败重试不新建 attempt

- **WHEN** 某 unit 的 provider 运行失败且重试预算未耗尽
- **THEN** 系统在同一 attempt 内重试该 unit，attempt/unit 身份与 durable 事件保持连续

### Requirement: plan 缺陷走 amendment 链（REQ-GCE-03）

coding 阶段发现 plan 缺陷时，系统 SHALL 将当前 unit 标记 `BlockedByPlanDefect`、attempt 进入 `AwaitingPlanAmendment`（复用现有 `CodingAttemptStatus`，不新建状态），并写入 `PlanAmendmentContext` durable 关联记录（`plan_session_id`、`group_attempt_id`、触发 `unit_id`、触发 `finding_id`、`previous_plan_revision_id`、`resume_target`）

#### Scenario: plan 缺陷修订后回到 coding

- **WHEN** unit 因 plan 缺陷被标记 `BlockedByPlanDefect`，人工经 SC revision 路径产出通过校验的新 plan revision
- **THEN** attempt 更新 plan binding，按 resume target 恢复原 unit 的 Coding，不新建 attempt

#### Scenario: amendment 回合挂在原 plan session

- **WHEN** attempt 处于 `AwaitingPlanAmendment` 且人在原 plan session 的对话门（amendment 上下文）发送 `human_gate_feedback`
- **THEN** 回合记录与预算扣减归属该 plan session（接续其 `manual_repairs_remaining`），group attempt 侧不产生新预算账；不创建第二个人工门实例；断线重连由原 plan session 恢复 amendment 上下文与 in-flight turn

#### Scenario: amendment 批准后恢复原 attempt

- **WHEN** amendment 修订产出通过校验的新 plan revision 且人 approve
- **THEN** attempt 的 plan binding 更新为新 `plan_revision_id`（原 `previous_plan_revision_id` 保留在 `PlanAmendmentContext`），coding attempt 保持同一实例并按 `resume_target` 恢复，不新建 attempt、不重新进入首次 approve 的 compile 链；新 revision 与原 attempt 不兼容时持久化失败并进入人工处置，不静默切换路径

#### Scenario: 原 plan session 保持唯一人工门

- **WHEN** plan 缺陷进入 amendment 流程
- **THEN** 原 plan session 保持唯一人工门宿主，不创建第二个人工门

#### Scenario: amendment 回合重开原 plan session 人工门

- **WHEN** amendment 期间产生人工反馈回合
- **THEN** amendment 期间的人工反馈回合以 amendment 上下文重开原 SC plan session 的对话式人工门，预算接续该 session 快照的 `manual_repairs_remaining`，`HumanGateTurn` 归属该 plan session

#### Scenario: 修订通过后更新 binding 并恢复

- **WHEN** 人工反馈触发 SC revision provider 并产出新 plan revision
- **THEN** 人工反馈触发 SC revision provider（REQ-CG-03 同源），新候选经 SC compiler 与 canonical validator 形成新 plan revision 后，系统 SHALL 更新 group attempt 的 plan binding 并按既有 resume target（Reexecute/Revalidate/AwaitHandoff）从原 attempt 恢复 Coding

#### Scenario: approve 沿用修订校验链

- **WHEN** amendment 修订通过后 approve
- **THEN** amendment approve 沿用「修订校验通过→新 plan revision→binding 更新」链，不复用首次 approve 的 compile 入口

#### Scenario: 新 revision 不兼容时 fail-closed

- **WHEN** 新 plan revision 与当前 attempt 无法安全兼容
- **THEN** 新 plan revision 与当前 attempt 无法安全兼容时 SHALL durable fail-closed，SHALL NOT 静默切换到另一 plan 或 legacy coding 路径

#### Scenario: 断线恢复由原 plan session 负责

- **WHEN** amendment 期间发生断线恢复
- **THEN** amendment 期间断线恢复由原 plan session 负责

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

### Requirement: 换代后的 compile child 精确绑定（REQ-C1-CHILD-01）

compile child/session 的选择 SHALL 同时匹配当前 plan 身份、plan revision、work item revision 与当前 enrollment binding version/身份；仅按 work item 类型、实体编号或“最新 session”猜测 SHALL 不被接受。用户明确换代/重新绑定后，系统 SHALL 创建属于新绑定版本的新 child；旧 child 与旧 binding SHALL 保持只读可查，不清空、不覆盖、不迁移其历史事实。

#### Scenario: 同 Work Item 编号跨代创建新 child

- **WHEN** 同一 Work Item 编号在旧代失败后，用户明确选择新 plan/session/source/target/provider 并提交换代
- **THEN** compile 为新绑定版本创建新 child 并成功匹配当前 plan/revision，旧 child 仍可按旧绑定查询且不被改写

#### Scenario: 旧 child 不冒充当前代

- **WHEN** 当前代 compile 查找时仅存在旧 plan/revision/binding 的 child
- **THEN** 系统拒绝复用旧 child，返回需要绑定/重建的等待操作，不清理旧 binding，不把旧成功回执写入当前代

#### Scenario: 同代重放不创建第二 child

- **WHEN** 当前绑定版本、plan/revision/work item 身份均相同的 compile 命令以相同或重复 command_id 重放
- **THEN** 系统命中同一 child 和 durable 结果，不创建第二 child，不修改旧历史

### Requirement: 返修指令与上下文注记单次消费（REQ-GCE-C2-INSTR）

Coder 首次 spawn 与 rework 两条路径 SHALL 使用同一消费口径：系统 SHALL 先以待消费的 rework instruction 与 context note 渲染完整 prompt 与执行上下文，再在一次可重放的原子写入中认领这些指令、绑定渲染结果并标记消费，最后 spawn provider。实际发送给 provider 的 prompt SHALL 包含被消费的指令内容；以指令为空渲染后再标记消费的行为 MUST NOT 发生。标记消费 MUST NOT 早于完整 prompt 渲染完成。同一指令 SHALL 至多被一次 role run 消费；中断后重放同一次认领 SHALL 命中同一结果而非生成新的上下文 hash。已记录的执行上下文 hash MUST NOT 被覆盖为新含义。

#### Scenario: rework 实际 prompt 含新指令

- **WHEN** Code Review 要求返修并落地新的 rework instruction，系统启动返修 Coder
- **THEN** provider 收到的 prompt 包含该指令，指令被该次 role run 消费一次，执行上下文 hash 与实际 prompt 一致

#### Scenario: spawn 路径与 rework 路径口径一致

- **WHEN** 首次 spawn 时存在未消费的 rework instruction 或 context note
- **THEN** 渲染出的执行上下文与实际 prompt 均包含其完整内容，而不仅是摘要

#### Scenario: 认领后 spawn 前中断

- **WHEN** 指令已认领并绑定上下文，但在 provider spawn 前发生中断
- **THEN** 系统落地可操作等待项并通知用户；用户点击继续后以同一认领与同一上下文 hash 启动，不产生 hash 冲突，指令不被消费第二次

#### Scenario: 渲染失败不消费

- **WHEN** 完整 prompt 渲染失败
- **THEN** 指令保持未消费，attempt 进入可操作等待，用户重试时仍能读取该指令

### Requirement: 下游启动不依赖上游最终确认（REQ-GCE-06）

多仓 group 中某 Work Item 的下游 attempt 调度 SHALL 只以既有 unit/run 完成事实与 handoff 发布为就绪判据，MUST NOT 把上游 attempt 的人工最终确认（FinalConfirm）作为下游启动前置。plan 级延迟聚合确认引入后，中间仓 attempt 停留在等待人工最终确认时，下游依赖齐备的 attempt SHALL 照常自动启动；平台 MUST NOT 因存在待确认的上游 attempt 而向用户逐仓催点最终确认。

#### Scenario: 上游留待最终确认仍自动启动下游

- **WHEN** 双仓 plan 中上游 WI 的 unit run 已 Completed 且 handoff 已发布，但其 attempt 停留在 WaitingForHuman＋FinalConfirm
- **THEN** 下游 WI 的 attempt 被自动调度启动并进入 coding，无需用户先确认上游
- **AND** 平台不为上游的逐仓最终确认发送催点通知，聚合确认通知只在全部执行完成后出现

#### Scenario: 聚合确认不改变调度判据

- **WHEN** 引入 plan 级聚合最终确认后，某 plan 存在部分 attempt 已完成、部分仍在执行
- **THEN** scheduler 的完成集与就绪判定仍只由 unit Completed 与 handoff 事实驱动，读取路径与引入前一致

### Requirement: 上游完成时契约与能力登记（REQ-GCE-07）

Work Item 完成发布 handoff 时，若 handoff 的 provided_contract_refs 为空且该完成 run 绑定的冻结 output_contracts 声明了契约，系统 SHALL 从冻结 output_contracts 登记契约标识与对应能力到新发布的 handoff；refs 非空时 SHALL 保持既有取值不变；无 output 的非接口 WI SHALL 允许为空。系统 MUST NOT 把 coder 自由文本自报解析为权威契约。

#### Scenario: refs 为空时从冻结 output 登记

- **WHEN** 某 WI 完成 run 的 handoff refs 为空，而其冻结 output_contracts 声明了 CT-001 及能力
- **THEN** 新发布的 handoff 携带该 contract ref 与对应 capabilities，并沿用既有 hash/build/publish 链
- **AND** 下游读取该 handoff 时能取到契约与能力

#### Scenario: 非空 refs 与无 output 保持原行为

- **WHEN** 完成 run 的 handoff refs 非空，或该 WI 无 output_contracts
- **THEN** 登记结果与既有行为一致（分别保持原选择、允许为空），不产生新失败
