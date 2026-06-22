# WorkItemPlan WP6：Final Compile 事务与恢复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 accepted Draft 编译为现有真实 `IssueWorkItemPlan`、`LifecycleWorkItemRecord[]`、`VerificationPlan[]` 和 child WorkItem sessions，并用 `WorkItemPlanCompileTransaction` 保证幂等与恢复。

**Architecture:** Final Compile 是唯一写真实实体的位置。进入 compile 后先创建 transaction，内存中分配真实 id、拓扑排序、运行 strict validator；validator 通过后进入 committing，按拓扑序创建实体，先写 committed marker，再更新 Plan 指针，最后写 compile report。

**Tech Stack:** Rust 1.95.0、Cargo、serde、Axum WS。

---

## 依赖

- WP4：serial accepted drafts。
- WP5：batch accepted drafts。

## File Structure

| 文件 | 操作 | 职责 |
|---|---|---|
| `src/product/work_item_plan_store.rs` | Modify | compile transaction CRUD 与报告 |
| `src/product/lifecycle_store.rs` | Modify | 必要的幂等 create/delete helper；避免旧 replace candidate 路径复用 |
| `src/product/workspace_engine.rs` | Modify | compile 状态机、strict validator 分流、recovery action |
| `src/product/work_item_split_validator.rs` | Modify | finding code → remediation scope helper |
| `src/web/workspace_ws_types.rs` | Modify | compile report payload、recovery action input、node type |
| `src/web/workspace_ws_handler.rs` | Modify | `work_item_plan_compile_recovery_action` handler |
| `tests/it_web/web_work_item_plan_compile.rs` | Create | WS 集成测试 |
| `tests/it_product/product_work_item_plan_compile.rs` | Create | store/事务集成测试 |
| `tests/it_web.rs` / `tests/it_product.rs` | Modify | 注册测试 |

## Task 1：Compile transaction 创建与前置校验

- [ ] 写失败测试：
  - `final_compile_creates_transaction_with_previous_plan_snapshot`
  - `final_compile_rejects_non_accepted_active_drafts`
  - `final_compile_rejects_superseded_drafts`
- [ ] 进入 `work_item_plan_compile` node 后创建 transaction：
  - `status=preparing`
  - `plan_commit_state=not_started`
  - `previous_plan_snapshot` 为当前完整 `IssueWorkItemPlan`
  - `active_draft_ids` 只取 active index 中 accepted 且未 superseded 的 draft。

## Task 2：内存映射、拓扑排序和 strict validator

- [ ] 写失败测试：
  - `compile_assigns_stable_ids_without_writing_entities_before_validation`
  - `compile_detects_dependency_cycle_before_commit`
  - `compile_runs_full_validator_before_real_writes`
- [ ] 分配 `outline_id -> work_item_id` 与 `outline_id -> verification_plan_id`。
- [ ] 将 Draft 投影为内存 `LifecycleWorkItemRecord[]`、`VerificationPlan[]`、`IssueWorkItemPlan`。
- [ ] 对 `depends_on_outline_ids` 拓扑排序；发现环则 transaction `failed`，进入 recovery/human node。
- [ ] strict validator 通过前不得写真实 `work_items/`、`verification_plans/`、`issue_work_item_plans/` 或 child session。

## Task 3：幂等提交顺序

- [ ] 写失败测试：
  - `committing_reuses_created_ids_on_retry`
  - `commit_marker_is_written_before_plan_file_update`
  - `committed_marker_retry_updates_plan_idempotently`
- [ ] `status=committing` 时写入 id mapping、topological order、step cursor。
- [ ] 按拓扑序创建 WorkItem / VerificationPlan，每步先检查 transaction `created_*_ids`。
- [ ] 创建 child workspace sessions 幂等。
- [ ] 先写 `plan_commit_state=committed` + `committed_at`。
- [ ] 再更新 `IssueWorkItemPlan.work_item_ids`、`verification_plan_ids`、`dependency_graph`、`status`、findings。
- [ ] 最后写 `status=committed` 和 compile report artifact。

## Task 4：Strict validator remediation 分流

- [ ] 写失败测试：
  - `strict_validator_item_failure_triggers_serial_downstream_invalidation`
  - `strict_validator_item_failure_in_batch_returns_batch_confirm`
  - `strict_validator_plan_failure_sets_outline_revising`
  - `repository_profile_missing_is_not_emitted_for_new_flow`
- [ ] 新增 `finding_remediation_scope(code, work_item_ids) -> PlanLevel | ItemLevel | Warning`。
- [ ] 串行模式 item 级失败：定位 outline，执行 downstream invalidation，从目标 item 重新生成。
- [ ] batch 模式 item 级失败：回 `work_item_batch_confirm`，只允许整组重写、暂停、转人工或用户明确 downgrade。
- [ ] plan 级失败：回 Outline 返修或 human triage。
- [ ] 新流程调用 validator 时 `repository_profile_ref=None` 且 `repository_profile=None`。

## Task 5：Recovery node 与 action

- [ ] 写失败测试：
  - `recovery_continue_resumes_from_step_cursor`
  - `abort_and_rollback_allowed_only_before_plan_commit`
  - `abort_and_rollback_restores_previous_plan_snapshot`
  - `committed_plan_state_forces_continue_or_human_triage`
- [ ] 新增 node type `work_item_plan_compile_recovery`。
- [ ] 新增 `WsInMessage::WorkItemPlanCompileRecoveryAction { action, reason }`。
- [ ] action：
  - `continue`：按 step cursor 幂等继续。
  - `abort_and_rollback`：仅 `plan_commit_state=not_started` 允许；恢复 previous plan snapshot 并清理 `created_*_ids`。
  - `human_triage`：锁定 plan，进入 `human_confirm`。

## Task 6：最终完成节点

- [ ] 写失败测试：
  - `compile_committed_enters_human_confirm_with_child_session_ids`
  - `human_confirm_after_compile_marks_workspace_completed`
- [ ] compile committed 后进入最终 `human_confirm`，展示 compiled plan summary 和 child session ids。
- [ ] 用户 confirm 后 stage=`completed`。

## 验证

```bash
cargo test --locked --test it_product work_item_plan_compile
cargo test --locked --test it_web work_item_plan_compile
cargo test --locked --lib work_item_split_validator
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

## 不做

- 不实现结构化 diff 的增强/体验层；只产出 MVP compile report payload。
- 不删除旧一次性 candidate 兼容 DTO。

## Commit

```bash
git add src/product/work_item_plan_store.rs src/product/lifecycle_store.rs src/product/workspace_engine.rs src/product/work_item_split_validator.rs src/web/workspace_ws_types.rs src/web/workspace_ws_handler.rs tests/it_web.rs tests/it_product.rs tests/it_web/web_work_item_plan_compile.rs tests/it_product/product_work_item_plan_compile.rs
git commit -m "feat(work-item-plan): add final compile transaction"
```
