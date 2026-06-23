# WorkItemPlan WP5：自动 Batch 生成确认与整组 Review Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现自动模式：系统按拓扑顺序串行生成全部 draft，不逐项等待用户确认；完成后进入整组确认和整组 reviewer。

**Architecture:** Batch 模式复用 WP4 的单 item prompt 和 local validator，但调度层没有逐项人工 gate。每个 item local validation 失败时自动重试一次；仍失败则标记 draft `validation_failed` 并继续生成后续 item。用户接受整组后，reviewer 开启则整组 review，通过后进入 final compile。

**Tech Stack:** Rust 1.95.0、Cargo、Axum WS、serde。

---

## 依赖

- WP4 完成，单 item prompt/local validator/Draft store 可用。

## File Structure

| 文件 | 操作 | 职责 |
|---|---|---|
| `src/product/workspace_engine.rs` | Modify | batch 调度、确认、review 路由 |
| `src/product/work_item_plan_store.rs` | Modify | batch record 状态、validation_failed_ids |
| `src/web/workspace_ws_types.rs` | Modify | Batch payload、batch decision message、node type |
| `src/web/workspace_ws_handler.rs` | Modify | `work_item_batch_decision` handler |
| `tests/it_web/web_work_item_plan_batch.rs` | Create | WS 集成测试 |
| `tests/it_web.rs` | Modify | 注册测试模块 |

## Task 1：Batch record 与队列状态

- [x] 写失败测试：
  - `batch_mode_creates_batch_record_for_current_round`
  - `batch_queue_uses_outline_topological_order`
- [x] 新增 node type：
  - `work_item_batch_run`
  - `work_item_batch_confirm`
  - `work_item_batch_review`
- [x] `WorkItemBatchRecord.status` 流转：
  - `generating` → `completed` → `review_pending` → `review_done`
- [x] Batch payload 包含 queue、draft_records、batch_status、failure_summary。

## Task 2：自动串行生成全部

- [x] 写失败测试：
  - `batch_generation_invokes_one_provider_run_per_outline`
  - `batch_item_n_plus_one_uses_previous_batch_drafts_as_context`
  - `batch_generation_does_not_enter_item_confirm`
- [x] 按拓扑序逐个调用 WP4 单 item prompt。
- [x] 生成 item N+1 时，前序上下文来自当前 batch 中已生成并被调度器接收的 draft records，不是 accepted records。
- [x] 不逐项跑 reviewer。

## Task 3：Local validator 失败自动重试一次

- [x] 写失败测试：
  - `batch_local_validation_failure_retries_once`
  - `batch_local_validation_second_failure_marks_validation_failed_and_continues`
  - `batch_confirm_payload_highlights_validation_failed_items`
- [x] 第一次失败：同 outline 重试一次，prompt 携带 validator findings。
- [x] 第二次失败：`WorkItemDraftRecord.status=validation_failed`，`WorkItemBatchRecord.validation_failed_ids` 追加该 draft，继续下一个 outline。
- [x] 不因单 item validation failed 中断 batch。

## Task 4：整组确认

- [x] 写失败测试：
  - `batch_confirm_accept_all_marks_all_valid_drafts_accepted`
  - `batch_confirm_rewrite_batch_supersedes_current_batch_drafts`
  - `batch_confirm_downgrade_to_serial_requires_strict_validator_failure_flag`（WP6 strict validator 失败入口补齐）
- [x] 新增 `WsInMessage::WorkItemBatchDecision { decision, feedback, first_affected_outline_id }`。
- [x] `accept_all`：仅所有 draft 没有 error 或用户明确接受 warning 时生效；validation_failed 存在时进入 human triage 或要求 rewrite/downgrade。
- [x] `rewrite_batch`：当前 batch drafts 全部 superseded，重新跑 batch。
- [x] `pause`：进入 human confirm。
- [x] `downgrade_to_serial`：仅 WP6 strict validator item 级失败后的失败摘要界面允许（基础入口已补；完整 draft 迁移规则仍在 Task 6）。

## Task 5：整组 reviewer

- [x] 写失败测试：
  - `batch_accept_enters_batch_review_when_reviewer_enabled`
  - `batch_accept_skips_review_when_reviewer_disabled`
  - `batch_review_pass_enters_final_compile`（WP6 final compile 节点补齐）
  - `batch_review_revise_batch_returns_batch_confirm`
  - `batch_review_plan_reopen_supersedes_drafts_and_sets_outline_revising`
- [x] reviewer prompt 审核整组，不允许单 item rewrite。
- [x] `pass`：进入 WP6 final compile。
- [x] `revise_batch`：回 `work_item_batch_confirm`，展示 findings。
- [x] `plan_reopen_required`：按 WP4 invalidation 规则标记 draft，进入 Outline 返修或 human triage。

## Task 6：自动模式降级为串行模式

- [x] 写失败测试：
  - `downgrade_to_serial_copies_unaffected_batch_drafts_and_revalidates`
  - `downgrade_to_serial_starts_from_first_affected_outline`
- [x] 未受影响 outline 的 batch drafts 复制为新 serial draft，并重新跑 local validator/review。
- [x] 受影响 outline 及之后按串行重新生成。

## 验证

```bash
cargo test --locked --test it_web work_item_plan_batch
cargo test --locked --lib workspace_engine
cargo test --locked --test it_product work_item_plan_store
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

## 不做

- 不实现并行 dependency layer。
- 不实现 final compile transaction。
- 不做前端 UI。

## Commit

```bash
git add src/product/workspace_engine.rs src/product/work_item_plan_store.rs src/web/workspace_ws_types.rs src/web/workspace_ws_handler.rs tests/it_web.rs tests/it_web/web_work_item_plan_batch.rs
git commit -m "feat(work-item-plan): add batch draft generation flow"
```
