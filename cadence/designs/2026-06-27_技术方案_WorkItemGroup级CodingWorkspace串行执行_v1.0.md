# WorkItemGroup 级 Coding Workspace 串行执行技术方案

> 文档类型：技术方案
>
> 版本：v1.0
>
> 日期：2026-06-27
>
> 适用范围：WorkItemPlan Final Compile 后的 WorkItemGroup 进入 Coding Workspace，并按 Work Item 串行执行 coding / testing / code review，最后进行整组 review 与 PR 提交。

## 背景

当前 WorkItemPlan 已经完成两阶段拆分、逐项 Draft 生成确认、Final Compile 与真实 Work Item 落库。Final Compile 会把 accepted Draft 投影成稳定的 `LifecycleWorkItemRecord`、`VerificationPlan`、`IssueWorkItemPlan.work_item_ids`、`IssueWorkItemPlan.dependency_graph`，并创建 Work Item 子 Workspace Session。

当前 Coding Workspace 仍以单个 `work_item_id` 为入口和执行边界：

- `CodingExecutionAttempt.work_item_id` 是 Coding Attempt 的核心关联字段。
- Coding Attempt 创建接口位于单个 Work Item 路径下。
- evaluation context、execution plan、handoff、shared worktree lock、final confirm 均按单个 Work Item 组织。
- 前端 Lifecycle Workbench 从单个 Work Item 卡片启动 Coding Workspace。

WorkItem 被拆分为 WorkItemGroup 后，用户进入 Coding Workspace 时期望带着整个 WorkItemGroup 进入，但每次实际编码只处理其中一个 Work Item。第一阶段只支持串行执行，后续可扩展到按依赖层并行。

## 目标

1. Coding Workspace 的产品入口从单个 Work Item 提升为 WorkItemGroup。
2. Group Coding Workspace 内部按 WorkItemPlan 顺序串行执行真实 Work Item。
3. 每个 Work Item 保留现有 coding、testing、code review、handoff 等节点。
4. 最后一个节点升级为整组 full review / submit PR / final confirm，而不是单个 Work Item 的局部确认。
5. WorkItemPlan / Outline / Draft 作为只读上下文增强与诊断来源，不作为 Coding 执行事实源。
6. 保留现有单 Work Item Coding Workspace 能力，避免破坏已有流程。

## 非目标

1. 不在第一阶段实现并行 Coding。
2. 不让 Coding 直接绑定 WorkItemDraftRecord 执行。
3. 不在 Coding 阶段自动修改 WorkItemPlan、Outline 或 Draft。
4. 不改变 WorkItemPlan Final Compile 的核心语义。
5. 不删除单 Work Item Coding Attempt 路径。

## 核心结论

采用“真实 Work Item 为执行事实源，WorkItemPlan / Draft 为只读上下文增强”的混合方案。

```text
执行事实源：Final Compile 后的真实 Work Item
排序依赖源：IssueWorkItemPlan.dependency_graph + WorkItem.sequence_hint / depends_on
上下文增强源：WorkItemPlan Outline、accepted Draft、compile transaction
异常诊断源：Draft 与真实 Work Item 的映射关系
```

Draft 与 Compile 后 Work Item 在业务含义上应该一致，但生命周期不同：

- Draft 是生成期候选事实，带有 `draft_id`、`generation_round_id`、`active`、`superseded`、`validator_findings` 等生成过程状态。
- Compile 后 Work Item 是执行期事实，带有稳定 `work_item_id`、`execution_status`、`handoff_summary_ref`、`completion_commit`、`worktree_path` 等执行状态。
- Coding 需要稳定执行状态和可追踪完成记录，因此应以 Compile 后 Work Item 为主。

## 概念模型

```text
IssueWorkItemPlan
  id = plan_id
  work_item_ids = [work_item_1, work_item_2, ...]
  dependency_graph = real work_item_id edges

Group Coding Workspace
  group_attempt_id
  plan_id / work_item_group_id
  current_work_item_id
  unit_runs[]

Coding Unit
  work_item_id
  execution_order
  status
  linked single-item coding state
  handoff_ref
  completion_commit
```

建议新增两个显式概念：

1. `CodingAttemptScope`
   - `work_item`：现有单 Work Item Coding Attempt。
   - `work_item_group`：新的 Group Coding Workspace。

2. `CodingExecutionUnit`
   - 表示 Group Coding Workspace 内部的一个真实 Work Item 执行单元。
   - 每个 unit 复用现有 Coding 阶段语义。
   - unit 完成后写 handoff 与 commit 信息，但不立即做最终 PR。

## 后端设计

### CodingAttempt 模型扩展

现有 `CodingExecutionAttempt` 保持兼容，新增可选 group 字段：

```text
scope: work_item | work_item_group
work_item_id: string
work_item_group_id: Option<String>
plan_id: Option<String>
current_work_item_id: Option<String>
active_unit_id: Option<String>
```

兼容规则：

- 旧 attempt 没有 `scope` 时按 `work_item` 处理。
- `scope=work_item` 时，`work_item_id` 仍是执行对象。
- `scope=work_item_group` 时：
  - `work_item_group_id` / `plan_id` 指向 `IssueWorkItemPlan.id`。
  - `work_item_id` 可保留为当前 active work item 的兼容字段，或在 DTO 层标记为当前 item。
  - 新逻辑优先读取 `current_work_item_id`。

### CodingExecutionUnit

新增持久化记录，用于 group attempt 内部串行调度：

```text
id
attempt_id
project_id
issue_id
plan_id
work_item_id
order_index
status: pending | running | waiting_for_human | completed | failed | blocked | skipped
started_at
completed_at
handoff_ref
completion_commit
summary
```

存储位置建议挂在 group attempt 目录下：

```text
coding-attempts/{attempt_id}/units/{unit_id}.json
```

这样不会污染现有单 Work Item attempt 文件结构。

### 创建 Group Coding Workspace

新增 API：

```text
POST /api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}/coding-attempts
```

创建流程：

1. 加载 `IssueWorkItemPlan`。
2. 要求 plan 状态为 `confirmed`。
3. 加载 `plan.work_item_ids` 对应真实 Work Item。
4. 校验这些 Work Item 都属于同一个 `work_item_set_id == plan_id`。
5. 根据 `IssueWorkItemPlan.dependency_graph` 和 `sequence_hint` 计算串行顺序。
6. 创建 `scope=work_item_group` 的 Coding Attempt。
7. 为每个 Work Item 创建 `CodingExecutionUnit`。
8. 选择第一个可执行 unit 作为 `current_work_item_id`。
9. 准备 Issue 共享 worktree 和 active lock。

第一阶段的 lock 规则保持保守：

- 同一 Issue 同一时间只允许一个 active group coding attempt。
- group attempt 内部一次只允许一个 running unit。
- 不允许单 Work Item attempt 与 group attempt 同时占用同一 Issue shared worktree。

### 串行调度

串行调度规则：

1. 按拓扑序执行 Work Item。
2. 若依赖未完成，当前 unit 保持 pending。
3. 若依赖完成但缺少 handoff，进入 blocked gate。
4. 当前 unit 完成后，调度下一个 pending 且依赖满足的 unit。
5. 所有 unit 完成后进入 group-level final stages。

第一阶段可简化为严格顺序：

```text
unit[0] completed -> unit[1] running -> unit[2] running -> group full review
```

即使 dependency graph 允许并行，也暂时不并行执行。

### 复用现有 Coding 节点

每个 unit 继续使用现有阶段：

```text
prepare_context
worktree_prepare
coding
testing
code_review
rework
```

区别在于：

- `review_request`
- `internal_pr_review`
- `final_confirm`

这三段从单 item 终点移动到 group 终点。

单个 unit 的 code review 通过后：

1. 生成 unit handoff。
2. 记录 completion commit / diff summary。
3. 更新真实 Work Item execution status。
4. 释放当前 unit，切到下一个 unit。

整个 group 的最后阶段：

```text
all units completed
  -> review_request
  -> internal_pr_review
  -> final_confirm
```

### Evaluation Context

当前 evaluation context 从 `attempt.work_item_id` 读取真实 Work Item 与 artifact。Group 模式下需要扩展为：

```text
current_work_item = current_work_item_id 对应真实 Work Item
group_context = plan + sibling work items + dependency handoffs
draft_context = accepted Draft / Outline 的只读补充
```

读取优先级：

1. 当前真实 Work Item 字段。
2. 当前真实 Work Item 对应的 Work Item Workspace artifact。
3. 当前 Work Item 的 VerificationPlan。
4. 依赖 Work Item 的 handoff。
5. WorkItemPlan Outline / accepted Draft 的原始拆分上下文。

如果 1-4 足够，则不需要读取 Draft。只有在缺少上下文、需要解释拆分意图、或诊断不一致时才读取 Draft。

### Plan / Draft 映射

Final Compile 时已经存在从 outline 到 real work item 的映射：

```text
outline_id -> work_item_id
outline_id -> verification_plan_id
draft_id -> outline_id
```

建议把这个映射作为 group coding context 的只读辅助索引暴露出来：

```text
work_item_id -> outline_id
work_item_id -> draft_id
work_item_id -> source_draft_ref
```

这可以来自 compile transaction 或按 active draft records 反查。第一阶段只需要读取，不需要改写。

### 不一致处理

如果真实 Work Item 与 Draft / Plan 不一致，处理原则如下：

1. Coding 执行仍以真实 Work Item 为准。
2. Draft / Plan 只用于提示差异，不覆盖真实 Work Item。
3. 严重不一致进入人工 gate。
4. gate 中展示：
   - real work_item_id
   - plan_id
   - outline_id
   - draft_id
   - 差异摘要
5. 用户可以选择终止、继续按真实 Work Item 执行，或回到 WorkItemPlan 重新生成。

## 前端设计

### Lifecycle Workbench

WorkItemGroup 卡片增加“进入 Coding Workspace”主操作。

行为：

1. 如果该 group 已有 active group coding attempt，直接进入。
2. 如果没有，则调用新的 group coding attempt API。
3. 单个 Work Item 卡片继续保留单 item coding 入口，用于兼容和特殊场景。

### CodingWorkspacePage

页面仍然以 `attemptId` 路由进入：

```text
/workbench/coding/{attemptId}
```

Snapshot 增加 group context：

```text
attempt_scope
plan_id
current_work_item_id
units[]
group_status
```

页面布局建议：

- Header 显示 WorkItemGroup / Plan ID 和当前 Work Item。
- 左侧 timeline 增加 unit 分组，当前 unit 展开显示 coding/testing/review 节点。
- 中间 chat 仍展示当前节点对话。
- 结果面板增加 group summary 和 unit handoff 列表。
- Final review 阶段展示整组 diff、所有 handoff、测试摘要、code review 摘要。

第一阶段不需要复杂甘特图或并行视图，只需要清楚表达：

```text
Group Progress: 1 / N completed
Current: work_item_xxx
Next: work_item_yyy
Blocked: dependency / handoff / gate reason
```

## WebSocket / Snapshot 契约

现有 Coding WS 可继续按 attempt 连接：

```text
GET /ws/coding-attempts/{attempt_id}
```

新增或扩展消息：

```text
coding_group_snapshot
coding_unit_started
coding_unit_completed
coding_unit_blocked
coding_group_review_started
coding_group_review_completed
```

也可以第一阶段先不新增独立消息，而是在现有 snapshot 中附带 group fields，由前端按 snapshot 重建状态。事件消息后续再细化。

## 状态流转

```text
Group Attempt Created
  -> Unit Prepare Context
  -> Unit Worktree Prepare
  -> Unit Coding
  -> Unit Testing
  -> Unit Code Review
  -> Unit Handoff
  -> Next Unit
  -> Group Review Request
  -> Group Internal PR Review
  -> Group Final Confirm
  -> Completed
```

异常状态：

- unit coding 失败：该 unit 进入 failed / blocked，group attempt 等待人工处理。
- unit review 要求修改：回到该 unit 的 rework。
- unit handoff 生成失败：该 unit blocked，不进入下一个 unit。
- shared worktree dirty：保持 group lock，进入人工 gate。
- group full review 要求修改：根据 review 结果回到对应 unit rework；如果无法定位，进入 group-level human gate。

## 测试策略

后端测试：

1. group coding attempt 创建必须要求 confirmed WorkItemPlan。
2. group coding attempt 必须从 `plan.work_item_ids` 创建 units。
3. 串行调度必须按 dependency graph / sequence_hint。
4. 当前 unit 完成后自动进入下一个 unit。
5. 所有 units 完成后进入 group-level review_request / internal_pr_review / final_confirm。
6. 单 Work Item attempt 与 group attempt 不得同时占用同一 Issue shared worktree。
7. Plan / Draft 不一致时进入人工 gate，不覆盖真实 Work Item。

前端测试：

1. WorkItemGroup 卡片能创建并进入 group coding attempt。
2. 已有 active group attempt 时直接进入。
3. CodingWorkspacePage 能展示 group progress、当前 unit、unit 列表。
4. Unit 切换后 chat / timeline / reports 正确刷新。
5. Final review 阶段展示整组 handoff 与 review 摘要。

回归测试：

1. 现有单 Work Item Coding Workspace 不受影响。
2. Story / Design / Work Item Workspace 的 timeline、chat rebuild、artifact version 绑定不受影响。
3. WorkItemPlan Final Compile 不受影响。

## 分阶段实施建议

### Phase 1：后端模型与只读 group snapshot

- 扩展 Coding Attempt scope。
- 新增 CodingExecutionUnit store。
- 新增 group coding attempt 创建 API。
- Snapshot 返回 group context。
- 不改变 runner 执行逻辑。

### Phase 2：串行 runner 接入

- group attempt 内部选择 current unit。
- 复用现有 coding/testing/code review 阶段执行当前 Work Item。
- unit 完成后生成 handoff 并切换下一个 unit。

### Phase 3：整组 final review

- 所有 units 完成后进入 group-level review_request。
- internal PR review prompt 汇总全量 diff、所有 unit handoff、测试和 code review。
- final confirm 更新 group attempt 与所有 Work Item 状态。

### Phase 4：前端体验完善

- WorkItemGroup 卡片入口。
- CodingWorkspacePage group progress。
- Unit 切换、blocked gate、group final review 展示。

### Phase 5：并行扩展预留

- 将串行调度器抽象为 scheduler。
- 允许 dependency layer 内多个 unit 并行。
- shared worktree 并行前需要进一步设计写入范围隔离和 git commit 策略。

## 风险与约束

1. shared worktree 风险：第一阶段必须保持串行和单 active lock，避免多个 unit 同时写同一工作区。
2. final review 归因风险：整组 review 可能提出跨 item 修改，需要 review 输出能定位到 work_item_id；不能定位时进入人工 gate。
3. DTO 兼容风险：`CodingAttemptDto.work_item_id` 目前是必填，group 模式下需要保持兼容或新增字段。
4. 上下文膨胀风险：group context 不能把所有 Draft 原文无条件塞进 prompt，应按当前 unit 和依赖裁剪。
5. 状态重复风险：真实 Work Item execution status 与 unit status 需要明确同步规则，避免一个完成一个未完成。

## 推荐决策

1. 第一阶段只支持 Final Compile 后的真实 WorkItemGroup 进入 Coding。
2. Coding 执行事实源固定为真实 Work Item。
3. WorkItemPlan / Draft 只读参与上下文增强与异常诊断。
4. Group Coding Workspace 内部严格串行。
5. 现有单 Work Item Coding Workspace 保留。
6. 最终 PR / Internal PR Review / Final Confirm 升级为 group-level。

这套方案能最大化复用现有 Coding Workspace 运行时，同时避免把生成期 Draft 状态带入编码期。后续如果需要并行，可以在 `CodingExecutionUnit` 和 scheduler 上扩展，而不需要重写单 item coding 节点。
