# WorkItemPlan WP4：串行 Draft 生成确认与逐项 Review Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现严格串行模式：按 Outline 拓扑顺序逐个生成 `WorkItemDraftRecord`，局部 validator 通过后允许用户接受，reviewer 开启时逐项 review 通过才进入下一个 item。

**Architecture:** 串行模式复用 WP1 Draft store，不写真实 WorkItem。每次只对当前 outline invoke provider，prompt 携带已 accepted 前序 draft 的摘要和直接依赖完整内容。`WorkItemDraftLocalValidator` 把当前 draft 投影为临时真实结构，调用可定位到 item 的 validator 子规则。

**Tech Stack:** Rust 1.95.0、Cargo、Axum WS、serde。

---

## 依赖

- WP0-WP3 全部完成。

## File Structure

| 文件 | 操作 | 职责 |
|---|---|---|
| `src/product/work_item_split_engine.rs` | Modify | 单 item draft prompt/parser |
| `src/product/work_item_split_validator.rs` | Modify | `WorkItemDraftLocalValidator` |
| `src/product/work_item_plan_store.rs` | Modify | draft 创建、accept、supersede/downstream invalidation |
| `src/product/workspace_engine.rs` | Modify | serial 调度、item confirm、item review 路由 |
| `src/web/workspace_ws_types.rs` | Modify | Draft artifact payload、draft decision message、node type |
| `src/web/workspace_ws_handler.rs` | Modify | `work_item_draft_decision` handler |
| `tests/it_web/web_work_item_plan_serial.rs` | Create | WS 集成测试 |
| `tests/it_web.rs` | Modify | 注册测试模块 |

## Task 1：拓扑序与当前 outline 游标

- [x] 写失败测试：
  - `serial_mode_starts_first_outline_by_topological_order`
  - `work_item_plan_outline_topological_order_rejects_cycles`
- [x] 在确认后的 Outline dependency graph 上计算拓扑序。
- [x] active index 或 session payload 保存当前 `active_outline_id`。
- [x] 若防御性发现环，进入 human confirm，不进入 Draft。

## Task 2：单 item prompt/parser

- [x] 写失败测试：
  - `single_item_prompt_contains_accepted_previous_context`
  - `single_item_prompt_forbids_work_item_id_and_outline_changes`
  - `single_item_parser_rejects_multiple_work_items`
  - `single_item_parser_rejects_backend_status_fields`
- [x] 新增 `build_work_item_draft_invocation(...)`。
- [x] 输入必须包含：
  - 完整已确认 Outline。
  - 当前 `WorkItemOutline`。
  - serial mode。
  - 直接依赖 draft 完整内容。
  - 其他已 accepted draft 摘要。
  - 用户/reviewer feedback。
- [x] parser 输出 `WorkItemDraftCandidate`，不得接受 `status/generated_from_node_id/accepted_at/work_item_id`。

## Task 3：WorkItemDraftLocalValidator

- [x] 写失败测试：
  - `local_validator_allows_valid_single_draft`
  - `local_validator_blocks_missing_write_scope`
  - `local_validator_blocks_required_gate_missing`
  - `local_validator_blocks_scope_conflict_with_direct_dependency`
- [x] 新增 `WorkItemDraftLocalValidator::validate(current, accepted_dependencies, outline)`.
- [x] 检查范围（WP4 Draft schema 可定位范围）：
  - traceability refs：由 Outline validator 覆盖，Draft local validator 校验 draft 与当前 outline 绑定。
  - write scope 必填。
  - context budget 上限：当前 `WorkItemDraftCandidate` schema 暂无独立 budget 字段，保留到 WP6 full validator / compile 前校验。
  - verification plan 内部合法性。
  - required gates。
  - command cwd/safety/source：当前 WP4 覆盖 required gate 引用；完整命令投影校验保留到 WP6 full validator。
  - 与直接依赖的 scope/handoff 一致性。
- [x] 不检查全 plan dependency graph 一致性；Final Compile 仍会跑 full validator。

## Task 4：Draft run 与 confirm 节点

- [x] 写失败测试：
  - `serial_item_run_writes_draft_record_not_real_work_item`
  - `local_validation_success_enters_draft_confirm_with_accept`
  - `local_validation_failure_enters_draft_confirm_without_accept`
  - `draft_accept_marks_record_accepted`
  - `draft_rewrite_supersedes_old_draft_and_regenerates_current_outline`
- [x] 新增 node type：
  - `work_item_draft_run`
  - `work_item_draft_confirm`
- [x] 新增 `WsInMessage::WorkItemDraftDecision { outline_id, decision, feedback }`。
- [x] `accept` 只在 local validator 无 error 时允许；否则 protocol error。
- [x] `rewrite` 创建新 draft，旧 draft 标记 `superseded`、`active=false`。
- [x] `pause` 进入 human confirm 或 paused node，不能自动继续。

## Task 5：逐项 reviewer

- [x] 写失败测试：
  - `accepted_draft_enters_item_review_when_reviewer_enabled`
  - `item_review_pass_starts_next_outline`
  - `item_review_revise_rewrites_only_current_item`
  - `item_review_plan_reopen_marks_outline_revising`
  - `item_review_revise_affecting_previous_item_downgrades_to_needs_human`
- [x] 新增 node type `work_item_draft_review`。
- [x] reviewer prompt 明确：`revise` 只能修改当前 outline；要改前序必须 `plan_reopen_required`。
- [x] `pass`：若还有下一个 outline，进入下一个 `work_item_draft_run`；否则进入 final compile 前的确认入口（WP6 接管）。
- [x] `plan_reopen_required`：active index `outline_state=revising`，触发 downstream invalidation 基础。

## Task 6：Downstream invalidation 与复用入口基础

- [x] 写失败测试：
  - `direct_rewrite_supersedes_target_and_downstream`
  - `ancestor_rewritten_draft_can_be_copied_and_revalidated`
  - `direct_rewrite_cannot_opt_out`
- [x] 使用 Outline dependency graph 计算可达下游。
- [x] 目标 item `supersede_reason=direct_rewrite`，下游 `ancestor_rewritten`。
- [x] 复制旧 draft 时必须生成新 `draft_id`、记录 `copied_from_draft_id`、重新跑 local validator 的入口基础。

## 验证

```bash
cargo test --locked --test it_web work_item_plan_serial
cargo test --locked --lib work_item_split_engine
cargo test --locked --lib work_item_split_validator
cargo test --locked --test it_product work_item_plan_store
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

## 不做

- 不实现 batch 模式。
- 不实现 final compile 写真实实体。
- 不做前端专属 UI。

## Commit

```bash
git add cadence/plans/2026-06-22_计划文档_实施计划_WorkItemPlan两阶段生成与逐项确认_WP4_串行Draft生成确认与逐项Review_v1.0.md src/product/models.rs src/product/work_item_split_engine.rs src/product/work_item_split_validator.rs src/product/work_item_plan_store.rs src/product/workspace_engine.rs src/web/workspace_ws_types.rs src/web/workspace_ws_handler.rs tests/it_product/product_work_item_plan_store.rs tests/it_web.rs tests/it_web/web_work_item_plan_serial.rs
git commit -m "feat(work-item-plan): add serial draft generation flow"
```
