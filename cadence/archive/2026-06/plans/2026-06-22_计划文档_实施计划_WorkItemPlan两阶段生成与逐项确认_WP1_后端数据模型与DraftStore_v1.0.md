# WorkItemPlan WP1：后端数据模型与 Draft Store Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立 v1.5.0 的 Outline、Draft、Batch、Compile Transaction、Outline Context Index 数据模型与专用 store，保证 Draft 阶段拥有独立事实来源，且不写真实 `work_items/` / `verification_plans/` / child sessions。

**Architecture:** 新建 `src/product/work_item_plan_store.rs` 承担 WorkItemPlan 两阶段流程的文件读写；复用 `json_store::write_json` 的 temp + atomic rename 能力。`LifecycleStore` 保留真实实体 CRUD，后续 Final Compile 才调用它写真实 WorkItem。模型放 `src/product/models.rs`，DTO 暂不暴露给前端复杂 UI。

**Tech Stack:** Rust 1.95.0、serde、Cargo。

---

## File Structure

| 文件 | 操作 | 职责 |
|---|---|---|
| `src/product/models.rs` | Modify | 新增 Outline/Draft/Batch/Compile/ContextIndex 结构体与 enum |
| `src/product/work_item_plan_store.rs` | Create | 新增专用 store：draft record、active index、compile transaction、outline context index |
| `src/product/mod.rs` | Modify | 导出 `work_item_plan_store` |
| `src/product/work_item_split_validator.rs` | Modify | 只提取可复用纯函数签名或新增 outline 级 helper 的类型入口，不改最终 validator 行为 |
| `tests/it_product/product_work_item_plan_store.rs` | Create | store 集成测试 |
| `tests/it_product.rs` | Modify | 注册测试模块 |

## Task 1：新增模型

- [ ] 写失败测试：`cargo test --locked --test it_product work_item_plan_models_roundtrip`
- [ ] 在 `models.rs` 新增：
  - `WorkItemPlanOutline`
  - `WorkItemOutline`
  - `WorkItemDraftCandidate`
  - `WorkItemDraftRecord`
  - `WorkItemDraftStatus::{Draft, Accepted, Superseded, ValidationFailed}`
  - `WorkItemGenerationMode::{Serial, Batch}`
  - `WorkItemDraftSupersedeReason::{DirectRewrite, AncestorRewritten, OutlineRevised}`
  - `WorkItemPlanDraftActiveIndex`
  - `WorkItemBatchRecord`
  - `WorkItemBatchStatus::{Generating, Completed, ReviewPending, ReviewDone}`
  - `WorkItemPlanCompileTransaction`
  - `WorkItemPlanCompileStatus::{Preparing, Validating, Committing, Committed, Failed, RecoveryRequired}`
  - `WorkItemPlanCommitState::{NotStarted, Committed}`
  - `OutlineContextIndex`
  - `DesignContextCapabilities`
  - `OutlineContextBlockerResolution`
- [ ] 所有新增类型使用 `#[serde(rename_all = "snake_case")]`；可选字段用 `#[serde(default, skip_serializing_if = "Option::is_none")]`。
- [ ] `previous_plan_snapshot` 在 `WorkItemPlanCompileTransaction` 中必须是非空字段，类型为 `IssueWorkItemPlan`。

## Task 2：新增 WorkItemPlanStore 路径与 CRUD

- [ ] 写失败测试：
  - `draft_store_writes_immutable_records_under_round_dir`
  - `active_index_tracks_current_round_and_batches`
  - `compile_transaction_roundtrips_with_previous_plan_snapshot`
  - `outline_context_index_uses_atomic_write`
- [ ] 创建 `WorkItemPlanStore::new(app_paths: ProductAppPaths) -> Self`。
- [ ] 路径约定：
  - Draft record：`.aria/projects/<project>/issues/<issue>/work_item_plan_drafts/<plan>/<round>/<draft>.json`
  - Active index：`.aria/projects/<project>/issues/<issue>/work_item_plan_drafts/<plan>/active_index.json`
  - Compile transaction：`.aria/projects/<project>/issues/<issue>/work_item_plan_compiles/<plan>/<compile>.json`
  - Outline context index：`.aria/projects/<project>/issues/<issue>/work_item_plan_outlines/<plan>/outline_context_index.json`
- [ ] 方法：
  - `put_draft_record(record)`
  - `get_draft_record(project, issue, plan, round, draft)`
  - `list_draft_records(project, issue, plan)`
  - `load_active_index(project, issue, plan)`
  - `save_active_index(index)`
  - `put_compile_transaction(tx)`
  - `get_compile_transaction(project, issue, plan, compile)`
  - `load_outline_context_index(project, issue, plan)`
  - `save_outline_context_index(index)`
- [ ] 所有 id 使用 `validate_relative_id`；路径逃逸测试必须覆盖 `../bad`。

## Task 3：Active index 行为

- [ ] 写失败测试：
  - `accepting_new_draft_supersedes_previous_active_for_outline`
  - `copying_draft_creates_new_draft_id_and_records_source`
  - `batch_id_sequence_is_scoped_to_generation_round`
- [ ] 新增 helper：
  - `next_generation_round_id(index) -> String`
  - `next_draft_id(index) -> String`
  - `next_batch_id(index, now) -> String`
  - `mark_draft_active(index, outline_id, draft_id, status)`
  - `mark_downstream_superseded(index, outline_ids, reason)`
- [ ] `outline_state` 合法值只允许 `confirmed/revising`。
- [ ] 串行模式 draft `batch_id = None`；batch 模式 draft `batch_id = Some(batch_id)`。

## Task 4：Outline context index 限流

- [ ] 写失败测试：
  - `outline_context_index_keeps_at_most_20_resolutions`
  - `outline_context_index_summarizes_when_estimated_tokens_exceed_threshold`
- [ ] `OutlineContextBlockerResolution` 包含：
  - `blocker_node_id`
  - `resolution_node_id`
  - `resolution_artifact_ref`
  - `estimated_tokens`
  - `created_at`
- [ ] 超过 20 条时合并最早记录为 summary record；总 token 超过 8000 时也触发合并。

## Task 5：确保旧 candidate 写真实实体路径未被新 store 调用

- [ ] 写失败测试：`draft_store_does_not_create_real_work_items_or_verification_plans`
- [ ] 测试中写入 draft record 后断言：
  - `LifecycleStore::list_work_items(project, issue)` 为空。
  - `LifecycleStore::list_verification_plans(project, issue)` 为空。
  - `LifecycleStore::list_workspace_sessions(project, issue)` 不新增 WorkItem child session。

## 验证

```bash
cargo test --locked --test it_product work_item_plan_store
cargo test --locked --lib models
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

## 不做

- 不改 `replace_issue_work_item_plan_candidate`；旧一次性流程暂时保留。
- 不实现 Outline/Draft provider 调度。
- 不实现 Final Compile 写真实实体。
- 不改前端。

## Commit

```bash
git add src/product/models.rs src/product/work_item_plan_store.rs src/product/mod.rs tests/it_product.rs tests/it_product/product_work_item_plan_store.rs
git commit -m "feat(work-item-plan): add two-stage draft store models"
```
