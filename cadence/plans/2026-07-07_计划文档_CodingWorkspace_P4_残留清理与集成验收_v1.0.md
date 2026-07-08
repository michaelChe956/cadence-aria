# CodingWorkspace P4 残留清理与集成验收 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 清理新链路中仍可见的 Tester / Analyst / 单 WorkItem Internal Reviewer 残留，并做跨后端、前端、prompt 的最终验收。

**Architecture:** 本计划是收口计划，依赖 P1、P2、P3 完成。它不引入新功能，只删除或迁移仍污染新 Coding Workspace 语义的入口、文案、类型和测试夹具。

**Tech Stack:** Rust backend, TypeScript frontend, coding workspace reports/timeline/store, cargo and pnpm verification.

---

## Scope

实现来源：

- `cadence/designs/2026-07-07_技术方案_CodingWorkspace流程精简补充Delta_v1.0.md` 第 9、12 节
- `cadence/designs/2026-07-07_技术方案_CodingWorkspace材料驱动Prompt协议_v1.0.md` 的验收标准

前置条件：

- P1 Prompt 材料驱动已完成。
- P2 CodeReview 自动返修配置和人工 gate 已完成。
- P3 单 WorkItem / WorkItemGroup 执行流边界已完成。

不做：

- 不保留 legacy UI。
- 不新增 tester/testing/analyst 节点。
- 不按 07-06 全量重做。

## Files

- Modify: `web/src/components/coding-workspace/CodingProviderConfigPanel.tsx`
- Modify: `web/src/api/types/coding.ts`
- Modify: `web/src/state/coding-workspace-store.ts`
- Modify: report/timeline components under `web/src/pages` or `web/src/components/coding-workspace`
- Modify: Rust DTOs under `src/web/handlers/dto.rs` if old fields leak through new response semantics
- Modify: tests/fixtures that still assert Tester/Analyst/InternalPrReview in new flow

## Task 1: 残留入口审计

- [ ] Run searches:

```bash
rg -n "Tester|tester|Testing|testing|Analyst|analyst|Internal Reviewer|internal_reviewer|InternalPrReview|internal_pr_review" src web/src tests
```

- [ ] Classify each hit as:
  - old test fixture to delete or rewrite
  - historical data type still needed for deserialization
  - new flow visible UI/API/prompt that must be removed
  - group final review concept that should be renamed or wrapped
- [ ] Record the classification in the implementation report.
- [ ] Do not delete a shared type before checking all callers.

## Task 2: Provider config panel final cleanup

- [ ] Ensure `CodingProviderConfigPanel.tsx` displays only:
  - Coder
  - Code Reviewer
  - CodeReview 自动返修次数
  - GroupFinalReview only when WorkItemGroup mode needs it
- [ ] Ensure it does not display:
  - Tester Plan
  - Tester Execute
  - Analyst
  - Single WorkItem Internal Reviewer
- [ ] Remove hidden rows if they are no longer needed for new flow. If removing the type breaks broad generated API contracts, keep the storage field but do not expose a UI row.
- [ ] Keep labels dense and operational; no long explanatory text in UI.

## Task 3: API / frontend type cleanup

- [ ] In `web/src/api/types/coding.ts`, remove new-flow-facing fields or aliases that imply tester/analyst are configurable.
- [ ] If backend DTO still includes historical fields, isolate them as compatibility-only internal fields and do not surface them in store actions.
- [ ] Update `coding-workspace-store.ts` actions:
  - no tester/analyst selection actions in new flow
  - no single WorkItem internal reviewer selection action
  - preserve code reviewer auto rework count action from P2
- [ ] Update component props to match.

## Task 4: Timeline and report wording

- [ ] Single WorkItem report/timeline should show:
  - Coding
  - CodeReview
  - ReviewRequest
  - Completed
- [ ] Single WorkItem report/timeline should not show:
  - Testing
  - Analyst Rework
  - InternalPrReview
- [ ] WorkItemGroup report/timeline may show GroupFinalReview after ReviewRequest push.
- [ ] If backend stage enum still has `InternalPrReview`, frontend should map group-scope final review display to `GroupFinalReview` wording and avoid showing it for single WorkItem.

## Task 5: Test fixture cleanup

- [ ] Update Rust tests that assert old five-role or testing flow.
- [ ] Update frontend tests that expect Tester/Analyst/Internal Reviewer rows.
- [ ] Add tests for:
  - provider config panel hides Tester/Analyst.
  - single WorkItem config does not show Internal Reviewer.
  - GroupFinalReview wording appears only for WorkItemGroup final review.
  - auto rework count remains visible under Code Reviewer.
- [ ] Avoid broad snapshot updates that accept unrelated UI churn.

## Task 6: End-to-end manual acceptance checklist

- [ ] Start backend/frontend if needed.
- [ ] Create or use a single WorkItem coding attempt.
- [ ] Confirm provider config page shows Coder + Code Reviewer + auto rework count, not Tester/Analyst/Internal Reviewer.
- [ ] Confirm CodeReview approve path completes after ReviewRequest push.
- [ ] Create or use WorkItemGroup coding attempt.
- [ ] Confirm WorktreePrepare appears once.
- [ ] Confirm each unit gets Coding + CodeReview.
- [ ] Confirm GroupFinalReview appears once after all units and push.
- [ ] Force or fake `request_changes` if available:
  - first failures auto返修 until configured budget
  - after budget人工 gate appears
  - manual context enters coder delta prompt

## Task 7: Verification

- [ ] Backend:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

- [ ] Frontend:

```bash
pnpm -C web test
pnpm -C web tsc -b
```

- [ ] If Playwright/e2e coverage exists for Coding Workspace:

```bash
pnpm -C web test:e2e
```

- [ ] Diff check:

```bash
git diff --stat
```

## Completion Criteria

- 新链路 UI 不展示 Tester / Analyst / 单 WorkItem Internal Reviewer 配置。
- Single WorkItem 不展示或运行 InternalPrReview。
- WorkItemGroup 使用 GroupFinalReview 语义。
- Prompt 固定模板不包含技术栈硬编码。
- CodeReview 自动返修配置和人工 gate 行为可见并通过测试。
- WorkItemGroup WorktreePrepare timeline 只出现一次。
- 全量 Rust 与前端验证通过，或报告明确阻塞原因。
