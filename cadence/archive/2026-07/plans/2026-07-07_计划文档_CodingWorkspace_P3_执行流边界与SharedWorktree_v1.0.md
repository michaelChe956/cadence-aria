# CodingWorkspace P3 执行流边界与 Shared Worktree Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修正单 WorkItem 与 WorkItemGroup 的执行边界：单 WorkItem push 后完成，WorkItemGroup 只准备一次 worktree 并在全部 unit 完成后运行一次 GroupFinalReview。

**Architecture:** 本计划只改 runner / lifecycle / group progression 的执行流和对应测试，不处理 prompt 技术栈硬编码，也不处理自动返修配置 UI。先用测试锁定当前错误流转，再最小修改 runner。

**Tech Stack:** Rust backend, CodingWorkspace runner, CodingAttemptStore, group execution unit state, cargo focused tests.

---

## Scope

实现来源：

- `cadence/designs/2026-07-07_技术方案_CodingWorkspace流程精简补充Delta_v1.0.md` 第 7、8、9、12 节
- `cadence/designs/2026-07-07_技术方案_CodingWorkspace材料驱动Prompt协议_v1.0.md` 的 GroupFinalReview 边界

不做：

- 不实现 CodeReview 自动返修次数配置；这是 P2。
- 不重写 prompt 固定模板；这是 P1。
- 不做大规模历史数据兼容分支。

## Files

- Modify: `src/web/coding_ws_handler/runner.rs`
- Modify: `src/product/coding_workspace_engine/lifecycle.rs`
- Modify: `src/product/coding_workspace_engine/group.rs`
- Modify: `src/product/coding_workspace_engine/internal_pr_review.rs`
- Modify: `src/product/coding_models/execution.rs` only if stage semantics require naming guards
- Test: `src/product/coding_workspace_engine/tests/gate_rework.rs`
- Test: `src/product/coding_workspace_engine/tests.rs` or nearest runner/group test module
- Test: `tests/it_web/web_coding_attempt_api/*` only if public API behavior changes need integration coverage

## Task 1: 单 WorkItem 不运行 InternalPrReview 的 failing test

- [ ] 新增 runner-level 或 engine-level test：单 WorkItem CodeReview approve 后进入 ReviewRequest。
- [ ] 模拟 ReviewRequest push success。
- [ ] 断言最终 attempt status 为 completed。
- [ ] 断言未创建 `InternalPrReview` timeline node。
- [ ] 断言未创建 internal reviewer provider run。
- [ ] Run focused test:

```bash
cargo test --locked --lib coding_workspace_engine
```

Expected before implementation: 测试失败，当前 runner 在 ReviewRequest push 后仍可能进入 InternalPrReview。

## Task 2: 修改单 WorkItem approve 后流转

- [ ] 在 `execute_start_coding_flow` 的 ReviewRequest push success 后分支增加 scope 判断。
- [ ] 若 `current.scope == WorkItem`：
  - 调用完成 attempt 的现有方法或新增明确方法。
  - emit current session state。
  - return。
- [ ] 若 `current.scope == WorkItemGroup`：
  - 保持进入组级最终审查的路径。
- [ ] 删除或绕开单 WorkItem 的 `await_stage_gate(... InternalPrReview ...)`。
- [ ] 不引入隐藏 InternalPrReview 步骤。

## Task 3: WorkItemGroup 全部 unit 完成后才 GroupFinalReview

- [ ] 新增测试覆盖：
  - group 中第一个 unit CodeReview approve 后不执行 ReviewRequest。
  - 只推进到下一个 unit 或 handoff。
  - 全部 unit 完成后才执行 ReviewRequest commit/push。
  - push 成功后只运行一次 GroupFinalReview。
- [ ] 如果现有代码仍使用 `InternalPrReview` stage 名称，测试可以先断言行为，再在 P4 做展示命名清理。
- [ ] 确保 GroupFinalReview 的 prompt/caller 只在 group scope 可达。

## Task 4: WorkItemGroup shared worktree 只准备一次

- [ ] 新增测试：group attempt 首个 unit 完成 WorktreePrepare 后，`worktree_path` 已存在。
- [ ] 调用 `advance_to_next_group_unit` 或 runner 推进到下个 unit。
- [ ] 断言后续 unit 不再创建新的 WorktreePrepare timeline node。
- [ ] 断言 attempt 使用同一个 `worktree_path`。
- [ ] 修改点优先放在 runner `PrepareContext` 分支或 `start_attempt`：
  - 若 scope 是 WorkItemGroup。
  - 且 `attempt.worktree_path.is_some()`。
  - 且路径存在或 lifecycle record 指向 shared worktree。
  - 直接更新 stage 到 Coding，跳过 WorktreePrepare。
- [ ] 不依赖 git worktree 幂等来掩盖 timeline 重复。

## Task 5: GroupFinalReview caller 命名边界

- [ ] 后端如果仍使用 `execute_internal_pr_review_with_commands`，给调用层加清晰 wrapper：
  - `execute_group_final_review_with_commands`
  - 或在 runner 层只以 GroupFinalReview 语义命名变量和日志。
- [ ] 单 WorkItem scope 不调用该 wrapper。
- [ ] Group scope push 后调用一次。
- [ ] 若 parser 仍复用旧结构，确保 source stage 与 prompt 已在 P1 改为 `group_final_review`。

## Task 6: Verification

- [ ] Run focused backend tests:

```bash
cargo test --locked --lib coding_workspace_engine
cargo test --locked --lib gate_rework
```

- [ ] Run broader checks:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

- [ ] Run integration tests only if touched public web API contract:

```bash
cargo test --locked --test it_web web_coding_attempt_api
```

- [ ] Diff check:

```bash
git diff --stat
```

## Completion Criteria

- 单 WorkItem CodeReview approve -> ReviewRequest push -> Completed。
- 单 WorkItem 不创建 InternalPrReview stage gate、timeline node、provider run。
- WorkItemGroup 每个 unit 仍经过 Coding -> CodeReview。
- WorkItemGroup 全部 unit 完成后才 ReviewRequest commit/push。
- WorkItemGroup push 后只运行一次 GroupFinalReview。
- WorkItemGroup timeline 中 WorktreePrepare 只出现一次。
- 不重复实现 07-06 已完成流程。
