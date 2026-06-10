# CodingWorkspace Provider QA P4 真实 E2E 验收实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用真实 Coding Workspace attempt 验证 provider-driven TestPlan、Review recovery gate、OpenSpec/Superpowers 契约和前端展示闭环。

**Architecture:** 先运行后端/前端标准验证命令，再启动工作台服务，最后用真实 Provider 跑一次覆盖 Testing 与 Code Review 的端到端场景，并保存验收记录。

**Tech Stack:** Cargo、pnpm、Aria dev service、Coding Workspace Web UI、真实 Provider、浏览器检查。

---

## 依赖与边界

- 必须先完成 P1、P2、P3。
- 必须在 `.worktrees/bugfix_test_branch` 执行。
- 不使用 Docker 作为本地 Rust 验证路径。
- 禁止 `cargo test -j 1`。

## 文件结构

- Verify: Rust workspace
- Verify: `web/`
- Create: `cadence/reports/2026-06-10_进度报告_CodingWorkspaceProvider驱动测试审查与恢复机制验证_v1.0.md`

## Task 1: 静态与单元验证

- [ ] **Step 1: Rust fmt**

```bash
cargo fmt --check
```

Expected: PASS。

- [ ] **Step 2: Rust clippy**

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: PASS。

- [ ] **Step 3: Rust check**

```bash
cargo check --locked
```

Expected: PASS。

- [ ] **Step 4: Rust tests**

```bash
cargo test --locked
```

Expected: PASS。

- [ ] **Step 5: Frontend tests**

```bash
pnpm -C web test
```

Expected: PASS。

- [ ] **Step 6: Frontend build**

```bash
pnpm -C web build
```

Expected: PASS。

## Task 2: 启动服务并检查健康状态

- [ ] **Step 1: 启动 Aria dev 服务**

使用仓库现有启动方式启动后端和前端。若当前服务已运行，先确认它们来自 `.worktrees/bugfix_test_branch`。

Expected:

- 后端监听 `http://127.0.0.1:4317`
- 前端监听 `http://127.0.0.1:5173`

- [ ] **Step 2: 健康检查**

```bash
curl --noproxy '*' -sS http://127.0.0.1:4317/api/health
curl --noproxy '*' -sS -I http://127.0.0.1:5173/
curl --noproxy '*' -sS http://127.0.0.1:5173/api/health
```

Expected:

- API health 返回 healthy。
- 前端返回 200 或 304。

## Task 3: 真实 Coding Workspace attempt 验收

- [ ] **Step 1: 选择测试 Work Item**

选择一个能触发代码修改、Testing、Code Review 的真实 Work Item。记录：

- project id
- issue id
- work item id
- branch name
- provider config

- [ ] **Step 2: 启动 attempt**

在前端 Coding Workspace 页面启动 attempt。

Expected:

- timeline 进入 Coding。
- stage gate 可正常确认。
- Provider prompt 可看到对应 role。

- [ ] **Step 3: 验证 Tester 两段式**

检查 Testing 节点：

- Tester prompt 包含 `[openspec_contract]`。
- Tester prompt 包含 `[superpowers_contract]`。
- Tester prompt 包含 Story Spec、Design Spec、Work Item 上下文。
- Tester 先输出 TestPlan。
- TestPlan steps 由 Provider 根据工作项判断，不是 Aria 硬编码 pnpm/cargo。
- execute 阶段 tool call 带 `step_id`。
- TestingReport 显示 `plan_id`、steps、evidence refs。

- [ ] **Step 4: 验证 required step gate**

如果真实场景自然出现 missing required step：

- TestingReport overall status 为 `blocked`。
- 前端 pending gate 显示 reason、evidence、raw output。
- gate actions 包含 `retry_test_plan` 或 `rerun_missing_steps`。

如果真实场景全部通过：

- 用 fake/controlled provider 补充一次缺 step 回归，制造 required step 未执行。
- 验证同样的 blocked gate 展示与恢复动作。

- [ ] **Step 5: 验证 Code Review recovery**

使用真实 Provider 或 controlled provider 触发 reviewer 输出 schema alias/缺字段场景。

Expected:

- parser 保留可恢复 findings。
- raw output 落盘。
- 完全无法解析时 report verdict 为 `blocked`。
- 前端显示 `retry_review`、`send_raw_output_to_analyst`、`provide_context`、`manual_continue`、`abort`。

- [ ] **Step 6: 验证 Analyst/Internal Reviewer contract**

检查 Analyst 和 Internal Reviewer prompt：

- 包含 `[openspec_contract]`。
- 包含 `[superpowers_contract]`。
- 包含 EvaluationContextPack。
- 不把 schema 异常误当 request_changes。

## Task 4: 记录验收报告

- [ ] **Step 1: 创建报告**

创建：

```text
cadence/reports/2026-06-10_进度报告_CodingWorkspaceProvider驱动测试审查与恢复机制验证_v1.0.md
```

报告必须包含：

- 验证日期。
- worktree 路径和分支。
- 提交 hash。
- Rust/前端命令结果。
- 服务地址。
- attempt id。
- Testing TestPlan 证据。
- Review raw output/gate 证据。
- Analyst/Internal Reviewer prompt contract 证据。
- 未覆盖项和后续风险。

- [ ] **Step 2: 提交报告**

```bash
git add cadence/reports/2026-06-10_进度报告_CodingWorkspaceProvider驱动测试审查与恢复机制验证_v1.0.md
git commit -m "docs: record coding QA recovery verification"
```

## 阶段验收标准

- 标准 Rust 命令全部 PASS。
- 标准前端命令全部 PASS。
- 真实或 controlled attempt 能证明 Testing 不再只靠固定命令宣称通过。
- Reviewer schema 异常不再导致前端无恢复动作地卡住。
- OpenSpec/Superpowers 在四个角色中都有 prompt 证据。
