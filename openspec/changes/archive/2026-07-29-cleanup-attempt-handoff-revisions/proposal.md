## Why

删除 coding attempt 时，该 attempt 已发布的 handoff revision 留存在 issue 级 lineage 中成为孤儿。下一次 attempt 重跑同一 work item 时，handoff revision ID 由 unit run ID 派生（`handoff_revision_` + `coding_unit_run_0001`）必然重名，group completion 的 preflight 校验判定冲突并失败关闭，导致新 attempt 无法完成第一个 work item。真实故障：孤儿 `handoff_revision_coding_unit_run_0001`（commit `8a765ac7`）阻塞后续 attempt 报 `group_completion_handoff_revision_conflict`。

## What Changes

- 删除 coding attempt 时，同步删除该 attempt 各 coding unit 已认领的 handoff revision，使 issue 级 lineage 不再残留该 attempt 的交接产物。
- 为 handoff revision 存储新增删除能力；该能力仅用于 attempt 删除流程，不作为通用存储操作暴露。
- 删除前校验 handoff revision 归属（`logical_work_item_id` 与 unit 匹配），归属不符时不删除。
- 不改变 handoff revision 的发布路径、内容结构与不可变写入语义。
- 不改变 group completion 的 handoff 发布与 preflight 判定逻辑。
- 不引入跨 attempt 引用扫描：上游 handoff 的解析限定在同一 attempt 内，attempt 删除后其 handoff 必然无消费者。
- 不自动清理历史遗留孤儿 handoff revision。

## Capabilities

### New Capabilities
- `coding-attempt-deletion-cleanup`: coding attempt 删除时对 issue 级 lineage 中该 attempt 交接产物的清理语义，包括清理范围、归属校验与失败处理。

### Modified Capabilities

（无。现有 specs 未覆盖 attempt 删除的 lineage 清理语义。）

## Impact

- `src/product/work_item_revision_store/handoff.rs`：新增 handoff revision 删除能力。
- `src/web/handlers/coding.rs`：attempt 删除流程中的清理调用。
- 受影响的用户可见行为：删除 attempt 后重建 attempt 可正常完成 work item，不再因 handoff 重名冲突失败。
- 不影响 plan revision、work item revision、projection bundle、verification plan revision 等计划编译产物。
- 不影响 group completion、plan repair、code review 既有判定。
