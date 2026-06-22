# WorkItemPlan WP0：Reviewer Sentinel 与 Review 契约基础 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 先把 reviewer 输出从 markdown JSON fence 迁移到 nonce sentinel structured block，并为 WorkItemPlan 专属 review verdict 建立兼容 DTO、解析和引用校验基础。

**Architecture:** `workspace_engine.rs` 目前的 `extract_tail_json` 只解析 markdown fence，且 `ReviewVerdictType` 仅支持 `pass/revise/needs_human`。本 WP 新增 reviewer 用 nonce sentinel parser（带 8 字符 nonce，优先解析 sentinel，fallback 旧 fence），并让共享 provider adapter 兼容带 nonce 的 `<ARIA_STRUCTURED_OUTPUT>` 标签，避免 WorkItemPlan author prompt 改造后无法提取 structured output。内部 `ReviewVerdict` / `EngineEvent::ReviewComplete` 与 WS DTO 的 `ReviewComplete` 同步携带可选 `WorkItemPlanReviewComplete`，但不改变 Story/Design/普通 WorkItem 的通用 verdict 行为。

**Tech Stack:** Rust 1.95.0、serde、Cargo。

---

## 前置状态

- `src/product/work_item_split_engine.rs` 的 author prompt 已要求 `<ARIA_STRUCTURED_OUTPUT>`，但没有 nonce。
- `src/cross_cutting/provider_adapter.rs` 的 `parse_last_structured_output` 只识别精确 `<ARIA_STRUCTURED_OUTPUT>` 起始标签，不能解析 `<ARIA_STRUCTURED_OUTPUT nonce="...">`。
- `src/product/workspace_engine.rs` 的 reviewer 解析在 `parse_review_verdict` / `extract_tail_json` / `parse_review_json`，只支持 markdown fence。
- `src/product/workspace_engine.rs` 的 `ReviewVerdict` 与 `EngineEvent::ReviewComplete` 没有可选 WorkItemPlan 专属 review extension，事件转发链路无法带出新增字段。
- `src/web/workspace_ws_types.rs` 的 `WsOutMessage::ReviewComplete` 没有 `work_item_plan_review` 字段。
- 本 WP 不实现 WorkItemPlan Outline/Item/Batch 状态机，只铺协议和解析基础。

## File Structure

| 文件 | 操作 | 职责 |
|---|---|---|
| `src/cross_cutting/provider_adapter.rs` | Modify | 共享 structured output parser 兼容 `<ARIA_STRUCTURED_OUTPUT nonce="...">`，无 nonce 旧输出继续可解析 |
| `src/product/models.rs` | Modify | 若 provider run prompt 需要保存 nonce，新增轻量 helper 类型不放这里；原则上本 WP不改业务模型 |
| `src/product/workspace_engine.rs` | Modify | 新增 sentinel parser、review prompt sentinel 要求、fallback 旧 fence、单测 |
| `src/product/work_item_split_engine.rs` | Modify | author/revision prompt 注入 nonce 版本 sentinel；解析兼容由 `provider_adapter.rs` 承担 |
| `src/web/workspace_ws_types.rs` | Modify | 新增 `WorkItemPlanReviewComplete` 及 enum；`ReviewComplete` 出站消息加可选字段；serde 测试 |
| `src/web/workspace_ws_handler.rs` | Modify | 转发 `EngineEvent::ReviewComplete` 时保留可选 `work_item_plan_review` |

## Task 1：通用 nonce sentinel parser

**目标：** 解析最后一个 nonce 匹配的 sentinel block；无 nonce 或不匹配时视作普通文本；保留 markdown fence fallback。

- [ ] 写失败测试：`cargo test --locked --lib sentinel_parser`
  - `extract_structured_json_prefers_last_matching_nonce_block`
  - `extract_structured_json_ignores_nonce_mismatch`
  - `extract_structured_json_falls_back_to_markdown_fence`
  - `extract_structured_json_treats_non_nonce_sentinel_as_text`
- [ ] 在 `src/product/workspace_engine.rs` 新增私有函数：
  - `fn extract_structured_json(output: &str) -> Option<(String, String)>`
  - `fn extract_nonce_sentinel_json(output: &str) -> Option<(String, String)>`
  - `fn extract_markdown_fence_json(output: &str) -> Option<(String, String)>`
- [ ] 将 `parse_review_verdict` 从 `extract_tail_json` 切到 `extract_structured_json`。
- [ ] 保留旧 `extract_tail_json` 测试语义，但重命名为 markdown fallback 测试。
- [ ] 运行：`cargo test --locked --lib workspace_engine::tests::extract_structured_json`

## Task 1b：共享 provider adapter 支持 nonce sentinel

**目标：** WorkItemPlan author/revision prompt 一旦改为 `<ARIA_STRUCTURED_OUTPUT nonce="...">`，`ProviderAdapter` 仍能从 stdout 提取 structured output；旧无 nonce sentinel 保持兼容。

- [ ] 写失败测试：`cargo test --locked --test it_provider provider_adapter_parses_nonce_structured_output_sentinel`
- [ ] 修改 `src/cross_cutting/provider_adapter.rs` 的 `parse_last_structured_output`：
  - 起始标签允许 `<ARIA_STRUCTURED_OUTPUT>` 与 `<ARIA_STRUCTURED_OUTPUT nonce="8char">`。
  - 结束标签允许 `</ARIA_STRUCTURED_OUTPUT>` 与 `</ARIA_STRUCTURED_OUTPUT nonce="same">`。
  - stdout 中存在多个 block 时仍解析最后一个完整 block。
  - nonce 不匹配时返回 parse error，不回退到其他 block。
- [ ] 保留现有 `parser_accepts_fenced_json_inside_structured_output_sentinel` 兼容行为。

## Task 2：Review DTO 扩展

**目标：** 在 WS DTO 中添加 WorkItemPlan 专属 review 子结构，不修改通用 `ReviewVerdictType` enum。

- [ ] 写失败测试：`cargo test --locked --lib workspace_ws_types work_item_plan_review_complete_roundtrips`
- [ ] 在 `src/web/workspace_ws_types.rs` 新增：
  - `WorkItemPlanReviewVerdict::{Pass, Revise, ReviseBatch, NeedsHuman, PlanReopenRequired}`
  - `WorkItemPlanReviewScope::{Outline, Item, Batch}`
  - `WorkItemPlanReviewAction::{Continue, ReviseOutline, ReviseCurrentItem, ReviseBatch, HumanTriage}`
  - `WorkItemPlanReviewGate::{RequiresCurrentItemRevision, RequiresBatchRevision, RequiresPlanReopen}`
  - `WorkItemPlanReviewAffectedItem { outline_index, target_outline_id }`
  - `WorkItemPlanReviewComplete`
    - 必含：`verdict`、`review_scope`、`generation_round_id`、`review_action`、`gates`
    - 可选/默认：`target_outline_id`、`draft_id`、`batch_id`、`affects_items`、`warnings`
- [ ] 修改 `WsOutMessage::ReviewComplete`，增加：
  - `#[serde(default, skip_serializing_if = "Option::is_none")] work_item_plan_review: Option<WorkItemPlanReviewComplete>`
- [ ] 修改 `src/product/workspace_engine.rs` 中内部 `ReviewVerdict` 与 `EngineEvent::ReviewComplete`，增加同名可选字段，并在 `complete_review` 持久化 JSON 与事件发送时传递该字段。
- [ ] 修改 `src/web/workspace_ws_handler.rs` 的 `EngineEvent::ReviewComplete` 转换，保证 `work_item_plan_review` 原样转发到 `WsOutMessage::ReviewComplete`。
- [ ] 确保旧 JSON 无该字段可反序列化。
- [ ] 运行：`cargo test --locked --lib workspace_ws_types`

## Task 3：WorkItemPlan reviewer 结构化解析与引用校验

**目标：** 支持解析 sentinel block 内的 WorkItemPlan reviewer JSON，并对 `target_outline_id` / `affects_items` 做防幻觉校验。

- [ ] 写失败测试：`cargo test --locked --lib workspace_engine work_item_plan_review`
  - `work_item_plan_review_revise_batch_maps_to_needs_human_generic_verdict_with_extension`
  - `work_item_plan_review_invalid_target_outline_id_downgrades_to_needs_human`
  - `work_item_plan_review_drops_invalid_affects_items_below_threshold`
  - `work_item_plan_review_invalid_affects_items_over_half_downgrades`
- [ ] 在 `workspace_engine.rs` 增加解析 helper：
  - `parse_work_item_plan_review_json(json, comments, valid_outline_ids, scope) -> ReviewVerdict`
  - 返回通用 `ReviewVerdictType` 时只用 `pass/revise/needs_human`，专属语义放入 extension。
- [ ] 若当前 engine 还没有 Outline/Draft 上下文，函数先作为纯函数落地，后续 WP3/WP4/WP5 在调用点传入 `valid_outline_ids`。
- [ ] 无效引用处理：
  - 静默移除无效 affects item。
  - warnings 写入 summary 或 extension metadata。
  - 超过 50% 无效则保留原文，降级 `needs_human`。

## Task 4：Reviewer prompt 使用 sentinel，保留旧 fence fallback

**目标：** 新 prompt 统一要求 nonce sentinel；旧 reviewer 输出 markdown fence 仍可解析一个兼容期。

- [ ] 写失败测试：`cargo test --locked --lib workspace_engine reviewer_prompt_requires_nonce_sentinel`
- [ ] 更新 `build_review_input` 或当前 reviewer prompt 构造处：
  - 注入随机 8 字符 nonce。
  - 明确最终 JSON 必须位于 `<ARIA_STRUCTURED_OUTPUT nonce="...">...</ARIA_STRUCTURED_OUTPUT nonce="...">`。
  - 明确不得使用 markdown fence。
- [ ] 不要求本 WP 删除旧 fence parser。

## 验证

```bash
cargo test --locked --test it_provider provider_adapter
cargo test --locked --lib workspace_ws_types
cargo test --locked --lib workspace_engine
cargo test --locked --lib work_item_split_engine
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

## 不做

- 不实现 Outline reviewer / item reviewer / batch reviewer 的状态机跳转。
- 不实现 `plan_reopen_required` 的 Draft store 失效。
- 不改前端 renderer；前端在 WP7 消费新增字段。

## Commit

```bash
git add src/cross_cutting/provider_adapter.rs src/product/workspace_engine.rs src/product/work_item_split_engine.rs src/web/workspace_ws_types.rs src/web/workspace_ws_handler.rs
git commit -m "feat(workspace): add sentinel review contract foundation"
```
