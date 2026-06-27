# WorkItemPlan WP8：贯通验收与回归 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 验证 WorkItemPlan v1.5.0 新流程端到端可用，并确认 Story / Design / 普通 WorkItem Workspace 不受共享协议改造影响。

**Architecture:** 本 WP 以测试和收口为主。后端用 fake provider + WS 集成测试覆盖 Outline → mode select → serial/batch → compile。前端用 Vitest 覆盖主要 UI 状态。若发现生产缺陷，先写最小 failing test，再做窄修复；若缺陷超出当前 WP，新增修复计划。

**Tech Stack:** Rust 1.95.0、Cargo、Axum WS、React、Vitest、pnpm。

---

## 依赖

- WP0-WP7 全部完成。

## File Structure

| 文件 | 操作 | 职责 |
|---|---|---|
| `tests/it_web/web_work_item_plan_staged_flow.rs` | Create | 后端 WS 贯通测试 |
| `tests/it_web/web_workspace_recovery_consistency.rs` | Create | Story/Design/WorkItemPlan 恢复一致性 |
| `tests/it_web.rs` | Modify | 注册测试 |
| `web/src/pages/ChatWorkspacePage.test.tsx` | Modify | 前端贯通 UI 场景 |
| `web/src/state/workspace-ws-store.test.ts` | Modify | artifact history 恢复场景 |
| `cadence/reports/2026-06-22_进度报告_WorkItemPlan两阶段生成与逐项确认验证_v1.0.md` | Create | 验证报告 |

## Task 1：后端 serial flow 贯通

- [x] 写测试：`work_item_plan_serial_flow_outline_to_compile`
- [x] 流程：
  1. prepare WorkItemPlan workspace。
  2. WS start_generation。
  3. fake provider 返回 Outline。
  4. user accept Outline。
  5. reviewer 关闭时直接进入 generation mode。
  6. select serial。
  7. fake provider 逐个返回 draft。
  8. 每个 draft accept。
  9. final compile。
  10. human confirm。
- [x] 断言：
  - Draft 阶段没有真实 work_items / verification_plans / child sessions。
  - Compile 后 plan status confirmed，真实 work items、verification plans 和 child sessions 存在。
  - timeline 包含 outline/draft/compile 专属 node。

## Task 2：后端 batch flow 贯通

- [x] 写测试：`work_item_plan_batch_flow_with_validation_failed_then_rewrite`
- [x] 覆盖：
  - batch 每个 outline 一次 provider run。
  - local validation 失败自动重试一次。
  - 仍失败记录 `validation_failed_ids`。
  - 用户选择 rewrite_batch 后重新生成并成功 compile。

## Task 3：plan_reopen_required 与 downstream invalidation

- [x] 写测试：`plan_reopen_required_supersedes_drafts_and_reopens_outline`
- [x] 覆盖：
  - item reviewer 返回 `plan_reopen_required`。
  - active index `outline_state=revising`。
  - 目标和下游 draft superseded，历史 draft 可读取。
  - 旁路 draft 可复制为当前 round 新 draft，并重新 validator（由 WP5 `downgrade_to_serial_copies_unaffected_batch_drafts_and_revalidates` 覆盖）。

## Task 4：Compile recovery

- [x] 写测试：`compile_recovery_resumes_after_committed_marker`
- [x] 通过注入 store 写入失败或构造半提交 transaction 覆盖：
  - `plan_commit_state=not_started` 时允许 rollback。
  - `plan_commit_state=committed` 时只能 continue/human_triage。
  - continue 后不创建重复实体。

## Task 5：刷新恢复与 artifact history

- [x] 写测试：`session_state_restores_work_item_plan_staged_artifacts`
- [x] 断言 SessionState 包含：
  - stage + active_node 原子快照。
  - current outline。
  - draft records。
  - batch queue。
  - active outline id。
  - compile transaction/report。
  - artifact_versions MVP index。

## Task 6：三类 Workspace 回归

- [x] 写测试：
  - `story_workspace_review_sentinel_fallback_still_passes`
  - `design_workspace_artifact_history_still_loads_markdown`
  - `ordinary_work_item_workspace_review_unaffected`
- [x] 按项目规则说明影响：
  - Story/Design 复用 reviewer parser 和 artifact payload，需要测试。
  - 普通 WorkItem 若复用 `ReviewComplete` DTO，也需要测试。
  - Coding Workspace 如果使用独立 WS 类型，报告中说明不受本次 ChatWorkspace payload 改造影响。

## Task 7：前端贯通状态

- [x] 写 Vitest：
  - `renders outline then mode then serial draft confirm`
  - `renders batch queue and review findings`
  - `renders compile recovery actions by commit state`
  - `artifact history switches selected timeline draft`
- [x] 命令：
  - `pnpm -C web exec vitest --run src/pages/ChatWorkspacePage.test.tsx`
  - `pnpm -C web exec vitest --run src/state/workspace-ws-store.test.ts`

## Task 8：验证报告

- [x] 创建 `cadence/reports/2026-06-22_进度报告_WorkItemPlan两阶段生成与逐项确认验证_v1.0.md`。
- [x] 报告包含：
  - 后端定向测试结果。
  - 前端测试结果。
  - 标准验证命令结果。
  - Story/Design/WorkItem 三类回归影响说明。
  - 未覆盖风险与后续建议。

## 总验证

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
pnpm -C web test
pnpm -C web build
```

## 不做

- 不做 Playwright 浏览器 E2E，除非实现过程中发现 Vitest 无法覆盖关键交互。
- 不实现 P1/P2 artifact diff 增强层。

## Commit

```bash
git add tests/it_web.rs tests/it_web/web_work_item_plan_staged_flow.rs tests/it_web/web_workspace_recovery_consistency.rs web/src/pages/ChatWorkspacePage.test.tsx web/src/state/workspace-ws-store.test.ts cadence/reports/2026-06-22_进度报告_WorkItemPlan两阶段生成与逐项确认验证_v1.0.md
git commit -m "test(work-item-plan): verify staged flow end to end"
```
