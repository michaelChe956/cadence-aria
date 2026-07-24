# Work Item Draft Prompt 最小验证实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` task-by-task. Steps use checkbox syntax.

**Goal:** 保留 Draft 校验失败可见性与 findings 保真，仅以临时 Claude Code Prompt 试运行验证首次通过率，不在产品中实现评估平台。

**Architecture:** 产品代码只保留既有 Prompt、Validator、一次有界修复和 Work Item Draft 失败反馈。撤回 `work_item_draft_evaluation`、`work-item-draft-eval` CLI、30 场景语料及报告模型。真实 Prompt 试运行是操作者明确授权的一次性流程，不写入 `src/`、不进入 CI、不提交原始输入或输出。

**Tech Stack:** Rust 2024、现有 Claude Code Provider、既有 Work Item Draft Prompt 与本地 Validator。

## 全局约束

- 不回退无效 Draft 不可接受门禁、失败告警、`validator_findings` 服务端合并或有效 Draft 的现有确认动作。
- 不新增评估模块、CLI、持久化报告、版本控制 30 场景语料或默认 CI 步骤。
- Prompt 成功仅指首次 Provider 输出通过既有本地 Validator；自动修复不计入成功率。
- 仅使用 Claude Code；一至两个脱敏案例各执行 20 次。单案例至少 19/20；双案例合计至少 38/40 后停止自动调优并交由人工验证。
- 未达标时仅改变 Prompt 文案一个变量；不得放宽 Validator、Schema 或接受门禁。

---

## Task 1：撤回超出范围的正式评估功能

**Files:**

- Delete by reverting commits: `91ba6d0 feat: add explicit draft quality evaluation`、`21ff5e5 fix: harden draft evaluation reports`
- Preserve: Task 1–4 的 Prompt/Validator/反馈实现与测试

- [x] 先确认未提交的 Task 5 文件只属于评估模块，再恢复它们到当前 `HEAD`，不得触碰 `.superpowers/sdd/progress.md`、pycache、其他 OpenSpec change 或用户文件。
- [x] 按新到旧顺序 revert `21ff5e5`、`91ba6d0`，用 `git diff --check` 确认无空白错误。
- [x] 运行 `rg "work-item-draft-eval|work_item_draft_evaluation" src tests web`，确认评估运行时代码与 CLI 已不存在；保留普通 Draft Prompt、Validator 和反馈路径。
- [x] 运行 `cargo fmt --check` 与 `cargo check --locked`；提交 `revert: remove draft evaluation runtime`。

## Task 2：执行临时 Claude Code Prompt 试运行

**Files:**

- Runtime only: `/tmp/aria-draft-prompt-validation/`（不提交）
- Do not modify: `src/`、`web/`、`tests/`、CLI

- [ ] 在调用前由操作者指定或确认两个脱敏案例；案例不得包含客户数据、真实 issue 标题、绝对路径或完整历史对话。
- [ ] 每个案例调用 Claude Code 20 次；每次仅记录首次输出是否通过既有本地 Validator、失败码和调用序号，不保存 Prompt 或模型原文。
- [ ] 若单案例，要求至少 19/20；若双案例，要求合计至少 38/40。自动修复结果单独记录但不计入首次通过率。
- [ ] 达标后停止自动化操作并向操作者报告聚合结果，进入人工验证；未达标时只修改一个 Prompt 文案变量，先运行现有确定性测试，再对相同案例重跑。

## Task 3：交付前验证

- [ ] 运行与本次保留功能相关的 Rust/前端定向回归；记录 Task 4 已知的无关 `web_coding_attempt_api` 404 与完整 Rust/Clippy 的既有阻塞，不将其归因于本次回退。
- [ ] 更新 OpenSpec task 状态和本计划，仅在 Task 1 的 revert 与 Task 2 的新鲜人工授权证据齐全时勾选。
