# WorkItemPlan WP3：Outline 确认 Review 与生成模式选择 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 Outline 确认、Outline reviewer、`work_item_generation_mode` 节点和专用 WS 输入校验，使用户确认 Outline 后可选择逐项生成或自动生成。

**Architecture:** `WorkspaceStage::AuthorConfirm` 继续复用，但前后端必须以 `active_node.node_type` 区分 `work_item_plan_outline_confirm` 与 `work_item_generation_mode`。Outline 确认后生成新的 `generation_round_id`，active index `outline_state=confirmed`，reviewer 开启时先走 `work_item_plan_outline_review`，通过后进入 mode select。

**Tech Stack:** Rust 1.95.0、Cargo、Axum WS、serde。

---

## 依赖

- WP0：review sentinel 与 `WorkItemPlanReviewComplete` 可用。
- WP1：active index 与 generation round helper 可用。
- WP2：Outline author / payload / validator 可用。

## File Structure

| 文件 | 操作 | 职责 |
|---|---|---|
| `src/product/workspace_engine.rs` | Modify | Outline confirm/review/mode 状态流转 |
| `src/web/workspace_ws_handler.rs` | Modify | 新增 WS 输入消息路由与阶段合法性校验 |
| `src/web/workspace_ws_types.rs` | Modify | 新增 `select_work_item_generation_mode`、`request_outline_revision` 输入和 node type |
| `src/product/work_item_plan_store.rs` | No code change | 复用 WP1 已有 active index 读写与 round helper |
| `tests/it_web/web_work_item_plan_mode.rs` | Create | WS 集成测试 |
| `tests/it_web.rs` | Modify | 注册测试模块 |

## Task 1：WS 输入与 node type

- [x] 写失败测试：`cargo test --locked --lib work_item_plan_mode_messages_roundtrip`
- [x] `TimelineNodeType` 新增：
  - `WorkItemPlanOutlineReview`
  - `WorkItemGenerationMode`
- [x] `WsInMessage` 新增：
  - `SelectWorkItemGenerationMode { mode: WorkItemGenerationModeDto }`
  - `RequestOutlineRevision { feedback: Option<String> }`
- [x] `WorkItemGenerationModeDto::{Serial, Batch}` 使用 snake_case。

## Task 2：Outline 确认后创建 generation round

- [x] 写失败测试：
  - `accept_outline_creates_generation_round_and_active_index`
  - `author_decision_is_rejected_on_generation_mode_node`
  - `request_revision_on_outline_confirm_returns_to_outline_run_without_round`
- [x] 在 active node=`work_item_plan_outline_confirm` 且 stage=`author_confirm` 时：
  - `author_decision accept`：确认当前 Outline，创建 `generation_round_id`，写 active index `outline_state=confirmed`。
  - `author_decision reject` 或 `request_revision`：回 Outline revision，不生成 round。
- [x] 在 active node=`work_item_generation_mode` 时拒绝通用 `author_decision`，返回 protocol error。

## Task 3：Outline reviewer

- [x] 写失败测试：
  - `outline_accept_enters_outline_review_when_reviewer_enabled`
  - `outline_accept_skips_review_when_reviewer_disabled`
  - `outline_review_pass_enters_generation_mode`
  - `outline_review_revise_returns_to_outline_revision`
- [x] 新增 `begin_work_item_plan_outline_review_run`。
- [x] prompt 审核范围只允许 Outline 级问题，不得要求完整 verification plan。
- [x] review 输出：
  - `pass`：进入 `work_item_generation_mode`。
  - `revise`：回 Outline revision，active index `outline_state=revising`。
  - `needs_human`：进入 `human_confirm`。
- [x] 旧通用 reviewer 输出可 fallback，但 WorkItemPlan reviewer 优先解析 `work_item_plan_review` extension。

## Task 4：generation mode select 节点

- [x] 写失败测试：
  - `select_serial_mode_enters_first_item_run`
  - `select_batch_mode_enters_batch_run`
  - `request_outline_revision_on_mode_node_sets_outline_revising`
  - `select_mode_rejected_outside_generation_mode_node`
- [x] 创建 `work_item_generation_mode` timeline node：
  - stage=`author_confirm`
  - payload 包含 confirmed outline、current_generation_round_id、selected mode 可空。
- [x] `select_work_item_generation_mode` 仅在该 node 生效。
- [x] `request_outline_revision` 仅在该 node 生效，并进入 Outline revision，旧 draft records 处理由 WP4/WP5 的失效逻辑承接。

## Task 5：SessionState 恢复 mode 节点

- [x] 写失败测试：`session_state_restores_generation_mode_node_with_outline_payload`
- [x] `SessionState` 必须带回：
  - stage
  - active_node_id
  - active node detail
  - current outline artifact
  - current_generation_round_id
- [x] 初始化 snapshot 中 stage 和 active_node 一次性推送，不先推 stage 再补 node。

## 验证

```bash
cargo test --locked --test it_web work_item_plan_mode
cargo test --locked --lib workspace_ws_types
cargo test --locked --lib workspace_engine
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

## 不做

- 不实现单 item Draft 生成。
- 不实现 batch 调度。
- 不实现 final compile。
- 前端只会看到新 payload，UI 在 WP7。

## Commit

```bash
git add src/product/workspace_engine.rs src/web/workspace_ws_handler.rs src/web/workspace_ws_types.rs tests/it_web.rs tests/it_web/web_work_item_plan_mode.rs cadence/plans/2026-06-22_计划文档_实施计划_WorkItemPlan两阶段生成与逐项确认_WP3_Outline确认Review与模式选择_v1.0.md
git commit -m "feat(work-item-plan): add outline review and mode selection"
```
