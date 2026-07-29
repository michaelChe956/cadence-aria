# coding-attempt-deletion-cleanup Specification

## Purpose
TBD - created by archiving change cleanup-attempt-handoff-revisions. Update Purpose after archive.
## Requirements
### Requirement: 删除 attempt 时清理其已认领的 handoff revision

删除 coding attempt 时，系统 MUST 遍历该 attempt 的所有 coding unit，并对每个 `latest_handoff_revision_id` 非空的 unit，从 issue 级 work item lineage 中删除对应的 handoff revision。清理 MUST 在 attempt 记录被删除之前执行，以保证 unit 指针数据可读。

#### Scenario: 单 unit 已发布 handoff 的 attempt 被删除

- **WHEN** 一个 work item group attempt 的首个 unit 已完成并认领 handoff revision，用户删除该 attempt
- **THEN** 系统 MUST 从 lineage 中删除该 handoff revision，删除后 lineage 中不再存在该 handoff revision

#### Scenario: 多 unit 均已发布 handoff 的 attempt 被删除

- **WHEN** 一个 attempt 的多个 unit 均已完成并各自认领 handoff revision，用户删除该 attempt
- **THEN** 系统 MUST 删除全部这些 handoff revision

#### Scenario: 无 unit 认领 handoff 的 attempt 被删除

- **WHEN** 一个 attempt 的所有 unit 的 `latest_handoff_revision_id` 均为空，用户删除该 attempt
- **THEN** 系统 MUST 正常完成删除，且 MUST NOT 删除 lineage 中任何 handoff revision

### Requirement: 清理后可重建 attempt 并完成同一 work item

删除 attempt 并重新创建 attempt 后，重跑同一 work item MUST 能够正常发布 handoff revision 并完成该 unit，MUST NOT 因 handoff revision 重名而失败。

#### Scenario: 删除后重建并完成首个 work item

- **WHEN** 用户删除已完成首个 work item 的 attempt，重新创建 attempt 并重跑该 work item 至完成
- **THEN** 系统 MUST 成功发布该 unit 的 handoff revision，MUST NOT 返回 handoff revision 冲突错误

### Requirement: handoff revision 删除必须校验归属

删除 handoff revision 时，系统 MUST 校验目标 handoff revision 的 `logical_work_item_id` 与发起删除的 coding unit 的 `logical_work_item_id` 一致。不一致时 MUST NOT 删除该 handoff revision。

#### Scenario: 归属不符时拒绝删除

- **WHEN** 某 unit 的 `latest_handoff_revision_id` 指向的 handoff revision 的 `logical_work_item_id` 与该 unit 不一致
- **THEN** 系统 MUST NOT 删除该 handoff revision

### Requirement: 计划编译产物不受删除影响

attempt 删除的清理范围 MUST 限定为 handoff revision。系统 MUST NOT 删除 plan revision、work item revision、work item projection bundle、verification plan revision、dependency graph revision 或 plan validation report。

#### Scenario: 删除 attempt 后编译产物完好

- **WHEN** 用户删除一个已完成部分 work item 的 attempt
- **THEN** lineage 中的 plan revision、work item revision、projection bundle、verification plan revision 与 dependency graph revision MUST 全部保持存在且内容不变

### Requirement: handoff revision 发布语义保持不变

新增的删除能力 MUST NOT 改变 handoff revision 的发布路径、内容结构或不可变写入语义。已发布且未被删除的 handoff revision MUST NOT 被就地修改。

#### Scenario: 发布路径未被削弱

- **WHEN** group completion 发布某 unit 的 handoff revision
- **THEN** 发布仍 MUST 使用不可变写入，重复发布同一 ID 且内容不同时 MUST 继续失败关闭

