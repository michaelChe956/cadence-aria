## Context

handoff revision 是「上游 work item 完成后交给下游的契约成果」，记录提供的契约、能力、测试、变更文件与 completion commit。它存放在 issue 级 work item lineage（`work-item-revisions/<plan>/logical-work-items/<wi>/handoff-revisions/`），而非 attempt 目录，因此不随 attempt 删除而清理。

其 ID 由 unit run ID 派生（`handoff_revision_` + `coding_unit_run_0001`）。unit run 编号是 attempt 内相对编号，每个新 attempt 的首个 unit 首次 run 均为 `coding_unit_run_0001`，因此同一 work item 重跑时 ID 必然重名。

真实故障：attempt `45c6a931` 完成 `wi_format_duration_lib` 并发布 `handoff_revision_coding_unit_run_0001`（commit `8a765ac7`）。该 attempt 被删除后 handoff 成为孤儿。后续 attempt 重跑同一 work item，`preflight_existing_group_handoff` 发现同 ID 档案存在但 unit 指针为空且内容不匹配，判定冲突并返回 `group_completion_handoff_revision_conflict`，新 attempt 无法完成首个 work item。

关键实现事实：handoff revision 目前通过 `write_immutable` 写入（`work_item_revision_store/handoff.rs:7`），存储层没有任何删除能力。

## Goals / Non-Goals

**Goals:**

- 删除 attempt 时清理该 attempt 各 unit 已认领的 handoff revision，消除孤儿。
- 删除后重建 attempt 可正常完成同一 work item。
- 删除限定归属校验，避免误删其他 work item 的交接产物。
- 计划编译产物完全不受影响。

**Non-Goals:**

- 不改 handoff revision 的发布路径、内容结构与不可变写入语义。
- 不改 group completion 的 handoff 发布与 preflight 判定逻辑。
- 不改 handoff revision 的 ID 派生规则。
- 不引入跨 attempt 引用扫描。
- 不自动清理历史遗留孤儿。
- 不把 handoff 删除暴露为通用存储操作或对外 API。

## Decisions

### 清理范围按 unit 指针反查，不做跨 attempt 引用扫描

删除 attempt 时遍历其 coding unit，取每个 unit 的 `latest_handoff_revision_id` 作为清理目标。

理由：上游 handoff 的解析完全限定在同一 attempt 内。`authoritative_resolved_handoff_revision_ids`（`coding_workspace_engine/plan_defect.rs:415-455`）只列举本 attempt 的 units 作为依赖候选，用本 attempt 的 unit 指针取 handoff ID，并校验依赖 unit 的 status 与 completion commit 均属本 attempt；依赖 unit 不在本 attempt 时直接返回 `WorkItemHandoffMissing`。配合 `active_coding_attempt_exists` 的排他约束（同一 issue 同时只允许一个活跃 attempt），一个 work item group 的全部 work item 必在同一 attempt 内完成。因此 attempt 删除后其 handoff 必然无消费者，无需引用扫描。

备选方案 1：按 unit run ID 推导 handoff ID 后删除同名档案。放弃原因是可能误删 ID 撞名的其他 attempt 档案。

备选方案 2：删除前扫描全 lineage 的 `resolved_handoff_revision_ids` 引用。放弃原因是已确认无跨 attempt 消费路径，扫描属多余复杂度。

### 新增存储层删除能力，但不作为通用操作暴露

在 `WorkItemRevisionStore` 新增 handoff revision 删除方法。该方法只在 attempt 删除流程调用。

理由：lineage 的设计前提是已发布版本化记录不可篡改（plan revision、work item revision、handoff revision 均用不可变写入）。新增删除是对该约定的定向开口，必须限定使用面。

删除方法 MUST 校验目标 handoff revision 的 `logical_work_item_id` 与发起删除的 unit 一致，归属不符不删除。这防止 unit 指针异常时误删其他 work item 的交接产物。

### 清理时机在 attempt 记录删除之前

清理调用置于 `delete_coding_attempt`（`web/handlers/coding.rs`）中 `coding_store.delete_attempt` 之前。

理由：清理依赖读取 attempt 的 coding unit 指针，attempt 记录删除后这些数据不可读。

### 不改 ID 派生规则

handoff revision ID 继续由 unit run ID 派生，不引入 attempt 标识。

理由：本 change 通过删除时清理消除孤儿，重名的前提（孤儿存在）被消除后 ID 规则不再产生冲突。改 ID 规则会影响发布路径、preflight 判定与既有数据兼容性，范围远超当前问题。

## Risks / Trade-offs

- [不可变约定被开口，未来可能被误用于其他删除场景] → 删除方法只在 attempt 删除流程调用，附归属校验；不暴露为通用存储 API 或对外 HTTP 接口。
- [unit 指针异常时可能指向他人 handoff] → 删除前校验 `logical_work_item_id` 归属，不符则不删除。
- [清理失败可能中断 attempt 删除] → 清理属删除流程的一部分，失败应遵循既有删除流程的错误处理；不得在清理失败时静默继续并留下不一致状态。
- [历史遗留孤儿不被自动清理] → 通过手动清理处理（本次故障的孤儿已手动清除并留存备份）。
- [跨 attempt 消费假设若未来变化，清理会破坏交接语义] → 该假设由 `authoritative_resolved_handoff_revision_ids` 的同 attempt 限定与活跃 attempt 排他约束共同保证；若未来放开跨 attempt 接续，本清理语义须重新评估。

## Migration Plan

- 无数据结构变更，无存储迁移。
- 部署后需重启后端服务方可生效。
- 历史遗留孤儿 handoff revision 不自动清理；如遇同类冲突需手动清除对应档案。
- 回滚方式为回退提交并重启服务；回滚后恢复为不清理行为，不产生残留数据。

## Open Questions

无。设计边界已确认。
