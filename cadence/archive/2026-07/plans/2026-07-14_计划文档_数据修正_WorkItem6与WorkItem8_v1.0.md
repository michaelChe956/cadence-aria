# Work Item 6 与 Work Item 8 数据修正 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在不改动业务代码和 Work Item 1–5 完成结果的前提下，使 Work Item 6 可独立完成 API 契约实现与既有测试迁移，并让 Work Item 8 正确消费该迁移结果。

**Architecture:** 只修改 `.aria/projects/project_0001/issues/issue_0001` 下的 Source Draft、Compiled Work Item 和 Verification Plan。Work Item 6 在 Web/API 边界处理 HOME、安全错误摘要、Fake runtime 测试 seam 和 `web_product_api.rs` 契约迁移；Work Item 8 仅记录迁移归属并在 Coding 前审计其他受影响测试。

**Tech Stack:** JSON、Cadence Aria Work Item Workspace 数据、Git。

## Global Constraints

- Work Item 3、5 已完成，不重新调度或修改其数据。
- Work Item 6 保持 `execution_status = pending`。
- Coding Attempt 保持 `running / prepare_context`。
- Work Item 6 不允许修改 `src/product/**`、`src/cross_cutting/**`。
- 禁止重新引入 `ProviderAvailabilityGate::new_test_mode`。
- 禁止 E2E、Playwright、Chrome/浏览器自动化和安装浏览器。
- 不修改业务 worktree `/home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001`。
- 不暂存或覆盖用户文件 `.superpowers/sdd/final-review-fix-report.md`。

---

### Task 1: 同步 Work Item 6 Source Draft、Compiled Work Item 与 Verification Plan

**Files:**

- Modify: `.aria/projects/project_0001/issues/issue_0001/work_item_plan_drafts/issue_work_item_plan_0001/round_002/draft_008.json`
- Modify: `.aria/projects/project_0001/issues/issue_0001/work-items/work_item_compile_20260712024139064_006.json`
- Modify: `.aria/projects/project_0001/issues/issue_0001/verification-plans/verification_plan_compile_20260712024139064_006.json`

**Interfaces:**

- Consumes: Work Item 2/5 已交付的共享 runner、gate、registry、RepositoryRegistrationCoordinator 和 CadenceSkillsManager 注入式构造器。
- Produces: Work Item 6 的一致范围、实现上下文和验证门禁；Work Item 8 可消费的 HTTP 201/envelope 契约与旧测试迁移结果。

- [ ] **Step 1: 修正 Work Item 6 实现上下文**

  在 Source Draft `candidate.implementation_context` 和 Compiled `planned_implementation_context` 中同步写明：

  - `product_resources.rs` 提供请求级可注入 HOME resolver，按 `HOME`、`USERPROFILE` 顺序读取，并拒绝空值和相对路径。
  - `web/error.rs` 只做 API 出口安全清理，不通过文本推断业务错误码。
  - Fake runtime 通过请求级健康源、Fake registry 或 registrar seam 隔离，不修改全局 gate 语义。
  - `tests/it_web/web_product_api.rs` 同步迁移 HTTP 201 和 `repository.*` JSON 路径。
  - 禁止 E2E、Playwright、Chrome/浏览器自动化和安装浏览器。

- [ ] **Step 2: 修正 Work Item 6 写入范围**

  在 Source Draft 和 Compiled 中把 `tests/it_web/web_product_api.rs` 加入 `exclusive_write_scopes`，删除与其冲突的整体 `tests/**` Forbidden Scope，保留其他精确 Forbidden Scope。

- [ ] **Step 3: 增加既有 API 测试迁移门禁**

  在 Source Draft 内嵌 Verification Plan 和独立 Verification Plan 中增加：

  ```text
  cargo test --locked --test it_web manages_workspace_repositories_and_keeps_issue_on_lifecycle_flow
  ```

  命令 ID 使用 `cmd_web_product_api_contract`，并加入 `required_gates`；其他 6 条 Cargo 命令保持不变。

- [ ] **Step 4: 校验 Work Item 6 三份数据一致**

  运行 `jq empty`，并用 `jq` 对比 Source Draft/Compiled 的 scopes、实现上下文关键约束和 Verification Plan command IDs。

### Task 2: 同步 Work Item 8 的旧测试迁移归属

**Files:**

- Modify: `.aria/projects/project_0001/issues/issue_0001/work_item_plan_drafts/issue_work_item_plan_0001/round_002/draft_010.json`
- Modify: `.aria/projects/project_0001/issues/issue_0001/work-items/work_item_compile_20260712024139064_008.json`

**Interfaces:**

- Consumes: Work Item 6 已迁移的 `tests/it_web/web_product_api.rs`。
- Produces: Work Item 8 Coding 前对其余 Repository POST 旧测试执行精确审计的约束。

- [ ] **Step 1: 替换旧 blocker 归属说明**

  在 Source Draft `candidate.implementation_context` 和 Compiled `planned_implementation_context` 中同步写明：

  - `tests/it_web/web_product_api.rs` 已由 Work Item 6 迁移，Work Item 8 不重复拥有。
  - Work Item 8 只验证迁移后的契约和新增贯通场景。
  - Coding 前重新检索其他旧 Repository POST 测试。
  - 若仍受影响，必须在 Coding 开始前把精确文件加入正式合法范围；不能 Coding 后再报告无人处理的 blocker。

- [ ] **Step 2: 保持 Work Item 8 scopes 不变**

  确认 `tests/it_web/web_product_api.rs` 未加入 Work Item 8 `exclusive_write_scopes`，且 `src/**`、`web/src/**`、`web/e2e/**` 仍为 Forbidden Scope。

- [ ] **Step 3: 校验 Source Draft 与 Compiled 一致**

  运行 `jq empty`，并检索两份数据中的迁移归属关键句。

### Task 3: 验证状态、提交并推送

**Files:**

- Verify only: `.aria/projects/project_0001/issues/issue_0001/**`
- Preserve: `/home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001/**`
- Preserve: `.superpowers/sdd/final-review-fix-report.md`

**Interfaces:**

- Consumes: Task 1、2 的修正结果。
- Produces: 可从页面继续 Work Item 6 Coding 的一致数据状态。

- [ ] **Step 1: 验证 JSON 与关键约束**

  对 5 个 JSON 运行 `jq empty`，确认无 `Playwright`、`Chrome`、浏览器自动化要求，并确认 Work Item 6 精确拥有 `web_product_api.rs`。

- [ ] **Step 2: 验证流程状态未被扰动**

  确认 Work Item 1–5 仍为 `completed`，Work Item 6 为 `pending`，Attempt 为 `running / prepare_context`，`rework_count = 0`，`provider_conversations = []`。

- [ ] **Step 3: 验证业务 worktree 未改变**

  确认业务 worktree HEAD 仍为 `640d63d78a316275b42e5bcab6969d7588e13d19` 且工作区干净。

- [ ] **Step 4: 检查并提交平台数据修正**

  运行 `git diff --check`，只暂存本计划、两份既有分析/设计文档以及 5 个 `.aria` JSON；不暂存 `.superpowers/sdd/final-review-fix-report.md`。提交信息使用：

  ```text
  fix: align work item 6 and 8 execution scopes
  ```

- [ ] **Step 5: 推送 `feat-b-0709`**

  推送到 `origin/feat-b-0709`，并再次核对本地与远端提交关系。
