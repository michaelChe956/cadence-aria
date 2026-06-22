# WorkItemPlan WP2：Outline 生成与上下文门禁 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 WorkItemPlan author 第一阶段改为只生成 `WorkItemPlanOutline`，在生成前做 Design context heading 门禁，生成后做 Outline 轻量校验；当上下文不足时进入 `work_item_plan_context_blocker` 节点。

**Architecture:** 复用现有 WorkItemPlan workspace prepare/session，但 `StartGeneration` 不再调用一次性 `WorkItemSplitEngine::build_generate_invocation`。新增 Outline prompt builder 与 parser，输出写入 timeline artifact payload 和 `WorkItemPlanStore` 的 outline/context index；不写 Draft record，也不写真实 WorkItem。

**Tech Stack:** Rust 1.95.0、Cargo、Axum WS、serde。

---

## 依赖

- WP0：sentinel parser 可用。
- WP1：`WorkItemPlanOutline`、`OutlineContextIndex`、`WorkItemPlanStore` 可用。

## File Structure

| 文件 | 操作 | 职责 |
|---|---|---|
| `src/product/work_item_split_engine.rs` | Modify | 新增 Outline invocation/prompt/parser；旧 full split prompt 保留 legacy |
| `src/product/workspace_engine.rs` | Modify | WorkItemPlan `StartGeneration` 进入 outline run；完成后路由 confirm/blocker |
| `src/product/work_item_split_validator.rs` | Modify | 新增 Outline 轻量 validator |
| `src/web/workspace_ws_types.rs` | Modify | 新增 `WorkItemPlanOutlineCandidate`、`WorkItemPlanContextBlocker` artifact payload |
| `src/web/workspace_ws_handler.rs` | Modify | WorkItemPlan author run 调用新 Outline flow |
| `tests/it_web/web_work_item_plan_outline.rs` | Create | WS/engine 集成测试 |
| `tests/it_web.rs` | Modify | 注册测试模块 |

## Task 1：Design context capabilities 与 gaps

- [ ] 写失败测试：
  - `design_context_capabilities_detects_required_sections`
  - `legacy_design_spec_gaps_are_injected_without_blocking`
- [ ] 在 `workspace_engine.rs` 或新 helper 中实现：
  - `extract_design_context_capabilities(markdown) -> DesignContextCapabilities`
  - `design_context_gaps(capabilities) -> Vec<String>`
- [ ] 章节识别支持中文和英文近似标题：
  - 架构概览 / Architecture
  - 模块划分 / Modules
  - 技术选型与测试框架 / Tech Stack / Test Strategy
  - 关键目录结构 / Key Paths
  - 外部依赖、运行方式与验证约束 / Dependencies / Verification
- [ ] 旧 Design spec 缺章节不阻断 Outline author；gaps 写入 Outline prompt 和 `outline_context_index.json`。

## Task 2：Outline prompt 与 parser

- [ ] 写失败测试：
  - `outline_author_prompt_forbids_full_work_items_and_repository_profile`
  - `outline_parser_accepts_valid_sentinel_json`
  - `outline_parser_rejects_verification_plan_or_work_item_id`
- [ ] 新增 `WorkItemSplitEngine::build_outline_invocation(...)`：
  - 输入 issue、story specs、design specs、repository path、repository structure summary、context gaps、context resolutions、user options。
  - 输出 prompt、provider type、worktree path、sentinel nonce。
- [ ] prompt 必须包含：
  - 只能输出 Outline，不得输出完整 Work Item、VerificationPlan、repository_profile、parallel_groups。
  - 不得修改仓库文件，不得创建计划文档。
  - 如果无法补齐模块边界或测试策略，输出 `context_blockers[]`。
- [ ] 新增 `parse_work_item_plan_outline_output(value) -> OutlineAuthorOutput`。

## Task 3：Outline 轻量 validator

- [ ] 写失败测试：
  - `outline_validator_rejects_duplicate_outline_ids`
  - `outline_validator_rejects_missing_dependency`
  - `outline_validator_rejects_dependency_cycle`
  - `outline_validator_requires_traceability_and_write_scopes`
  - `outline_validator_detects_direct_scope_conflict`
- [ ] 新增 `WorkItemPlanOutlineValidator::validate(outline) -> WorkItemSplitValidationReport`。
- [ ] 校验范围只包含 Outline 级规则；不得运行 full `WorkItemSplitValidator::validate`。
- [ ] findings 复用 `WorkItemSplitFinding`，`work_item_ids` 中放 `outline_id`。

## Task 4：Engine 路由到 Outline confirm 或 context blocker

- [ ] 写失败测试：
  - `work_item_plan_start_generation_creates_outline_run_node`
  - `valid_outline_enters_outline_confirm`
  - `context_blockers_enter_context_blocker_node`
  - `outline_validation_failure_auto_retries_then_human_blocker`
- [ ] 新增 timeline node type：
  - `work_item_plan_outline_run`
  - `work_item_plan_outline_confirm`
  - `work_item_plan_context_blocker`
- [ ] `complete_work_item_plan_outline_author` 行为：
  - 有 `context_blockers[]`：写 `WorkItemPlanContextBlocker` payload，stage=`human_confirm`，active node=`work_item_plan_context_blocker`。
  - Outline validator 通过：写 `WorkItemPlanOutlineCandidate` payload，stage=`author_confirm`，active node=`work_item_plan_outline_confirm`。
  - Outline validator 失败：最多自动重跑 1 次；仍失败进入 blocker。
- [ ] 不生成 `generation_round_id`；round 在 Outline 确认后由 WP3 创建。

## Task 5：context blocker resolution 持久化

- [ ] 写失败测试：
  - `context_blocker_human_resolution_appends_outline_context_index`
  - `next_outline_prompt_includes_previous_resolution`
- [ ] 在 `work_item_plan_context_blocker` 节点：
  - 用户补充上下文后创建 `context_blocker_resolution` artifact ref。
  - 追加到 `outline_context_index.json`。
  - 重新进入 Outline author run。
- [ ] `human_confirm` 在该节点不得把 `confirm` 解释为继续生成。

## 验证

```bash
cargo test --locked --test it_web work_item_plan_outline
cargo test --locked --lib work_item_split_engine
cargo test --locked --lib work_item_split_validator
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

## 不做

- 不实现 Outline reviewer。
- 不实现 generation mode select。
- 不生成 WorkItemDraftRecord。
- 不改前端专属 UI，只保证 WS payload 可序列化。

## Commit

```bash
git add src/product/work_item_split_engine.rs src/product/workspace_engine.rs src/product/work_item_split_validator.rs src/web/workspace_ws_types.rs src/web/workspace_ws_handler.rs tests/it_web.rs tests/it_web/web_work_item_plan_outline.rs
git commit -m "feat(work-item-plan): generate outline with context gate"
```
