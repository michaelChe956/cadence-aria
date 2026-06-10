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
- 必须启用 deterministic controlled provider 验收路径；真实 Provider 输出不可控，只能作为补充证据。
- 如果 P1-P3 尚未补齐 Testing/TestPlan controlled fixture，本阶段必须先阻塞并记录缺口，不得用真实 Provider 结果替代该回归。

## 文件结构

- Verify: Rust workspace
- Verify: `web/`
- Create: `cadence/reports/2026-06-10_进度报告_CodingWorkspaceProvider驱动测试审查与恢复机制验证_v1.0.md`

## Task 1: 静态与单元验证

- [x] **Step 1: Rust fmt**

```bash
cargo fmt --check
```

Expected: PASS。

- [x] **Step 2: Rust clippy**

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: PASS。

- [x] **Step 3: Rust check**

```bash
cargo check --locked
```

Expected: PASS。

- [x] **Step 4: Rust tests**

```bash
cargo test --locked
```

Expected: PASS。

- [x] **Step 5: Frontend tests**

```bash
pnpm -C web test
```

Expected: PASS。

- [x] **Step 6: Frontend build**

```bash
pnpm -C web build
```

Expected: PASS。

## Task 2: 启动服务并检查健康状态

- [x] **Step 1: 启动 Aria dev 服务**

使用仓库现有启动方式启动后端和前端。若当前服务已运行，先确认它们来自 `.worktrees/bugfix_test_branch`。

Expected:

- 后端监听 `http://127.0.0.1:4317`
- 前端监听 `http://127.0.0.1:5173`

- [x] **Step 2: 健康检查**

```bash
curl --noproxy '*' -sS http://127.0.0.1:4317/api/health
curl --noproxy '*' -sS -I http://127.0.0.1:5173/
curl --noproxy '*' -sS http://127.0.0.1:5173/api/health
```

Expected:

- API health 返回 `{"status":"ok"}`。
- 前端返回 200 或 304。

## Task 3: Controlled provider 可复现验收

- [x] **Step 1: 以 E2E test controls 模式启动服务**

后端必须带环境变量：

```bash
ARIA_E2E_TEST_CONTROLS=1 cargo watch -w src -w Cargo.toml -w Cargo.lock -x "run --locked -- web --workspace . --host 127.0.0.1 --port 4317"
```

前端：

```bash
pnpm dev --port 5173
```

Expected:

- `/api/test/...` 路由可用。
- health 检查仍返回 `{"status":"ok"}`。

- [x] **Step 2: 验证 controlled TestPlan happy path**

使用 controlled provider fixture 固定输出：

- `plan_tests` 输出两个 required steps：`unit`、`api_smoke`。
- `execute_test_plan` 对两个 step 都返回 passed evidence。
- `TestingReport.overall_status == passed`。
- `TestingReport.plan_id` 非空。
- `TestingReport.steps` 包含 `unit`、`api_smoke`。
- `unplanned_commands` 为空或不影响 required step。

如果当前 test controls 还没有 Testing/TestPlan fixture endpoint，应先回到 P2 增补，例如新增：

```text
POST /api/test/coding-attempts/{attempt_id}/testing-fixture
```

请求体至少包含：

```json
{
  "plan_output": {
    "summary": "controlled unit and API smoke",
    "steps": [
      {
        "id": "unit",
        "title": "Unit tests",
        "intent": "prove unit behavior",
        "required": true,
        "tool": "run_command",
        "risk_level": "low",
        "command_or_tool_input": {"command": ["true"]},
        "evidence_expectation": "exit 0"
      },
      {
        "id": "api_smoke",
        "title": "API smoke",
        "intent": "prove API health",
        "required": true,
        "tool": "run_command",
        "risk_level": "low",
        "command_or_tool_input": {"command": ["true"]},
        "evidence_expectation": "exit 0"
      }
    ]
  },
  "step_results": [
    {"step_id": "unit", "status": "passed"},
    {"step_id": "api_smoke", "status": "passed"}
  ]
}
```

- [x] **Step 3: 验证 missing required step blocked gate**

使用 controlled provider fixture 固定输出：

- TestPlan required steps：`unit`、`security`。
- execute 阶段只返回 `unit` 的 passed evidence。

Expected:

- `TestingReport.overall_status == blocked`。
- `missing_required_steps == ["security"]`。
- 前端 pending gate 显示 `reason_code`、`evidence_refs`、`raw_provider_output_ref`。
- gate actions 包含 `retry_test_plan`、`rerun_missing_steps`、`provide_context`、`manual_continue`、`abort`。
- 点击 `manual_continue` 时如果没有填写原因，应显示错误或被后端拒绝。

- [x] **Step 4: 验证 review schema alias / malformed recovery**

使用现有或扩展后的 review fixture：

```bash
curl --noproxy '*' -sS -X POST \
  http://127.0.0.1:4317/api/test/workspace-sessions/<attempt_id>/review-fixture \
  -H 'content-type: application/json' \
  -d '{"verdict":"request_changes","summary":"fixture summary","comments":"review fixture"}'
```

若现有 review fixture 不能输出 findings alias/缺字段场景，应在 P2 增补 controlled review fixture，使其能输出：

```json
{
  "verdict": "request_changes",
  "summary": "schema alias case",
  "findings": [
    {
      "file": "src/example.rs",
      "description": "missing validation",
      "recommendation": "add validation"
    }
  ]
}
```

Expected:

- parser 保留 finding，不因缺 `severity` / `source_stage` 丢弃。
- 完全 malformed JSON 时 verdict 为 `blocked`。
- raw output 落盘。
- 前端显示 `retry_review`、`send_raw_output_to_analyst`、`provide_context`、`manual_continue`、`abort`。

- [x] **Step 5: 验证 reconnect 幂等**

在 missing required step 或 review blocked gate 打开后：

1. 刷新浏览器。
2. 重新进入同一 Coding Workspace attempt。
3. 再次触发同一 gate action。

Expected:

- pending gate 仍能恢复显示。
- 同一 gate 不重复出现。
- 同一 gate response 不重复启动 runner。
- 已 resolved gate 不会重新显示。

## Task 4: 真实 Coding Workspace attempt 验收

- [x] **Step 1: 选择测试 Work Item**

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

- [x] **Step 3: 验证 Tester 两段式**

检查 Testing 节点：

- Tester prompt 包含 `[openspec_contract]`。
- Tester prompt 包含 `[superpowers_contract]`。
- Tester prompt 包含 Story Spec、Design Spec、Work Item 上下文。
- Tester 先输出 TestPlan。
- TestPlan steps 由 Provider 根据工作项判断，不是 Aria 硬编码 pnpm/cargo。
- execute 阶段 tool call 带 `step_id`。
- TestingReport 显示 `plan_id`、steps、evidence refs。

- [x] **Step 4: 验证 required step gate**

如果真实场景自然出现 missing required step：

- TestingReport overall status 为 `blocked`。
- 前端 pending gate 显示 reason、evidence、raw output。
- gate actions 包含 `retry_test_plan` 或 `rerun_missing_steps`。

如果真实场景全部通过：

- 使用 Task 3 的 controlled provider 缺 step 回归，制造 required step 未执行。
- 验证同样的 blocked gate 展示与恢复动作。

- [x] **Step 5: 验证 Code Review recovery**

使用真实 Provider 或 controlled provider 触发 reviewer 输出 schema alias/缺字段场景。

Expected:

- parser 保留可恢复 findings。
- raw output 落盘。
- 完全无法解析时 report verdict 为 `blocked`。
- 前端显示 `retry_review`、`send_raw_output_to_analyst`、`provide_context`、`manual_continue`、`abort`。

- [x] **Step 6: 验证 Analyst/Internal Reviewer contract**

检查 Analyst 和 Internal Reviewer prompt：

- 包含 `[openspec_contract]`。
- 包含 `[superpowers_contract]`。
- 包含 EvaluationContextPack。
- 不把 schema 异常误当 request_changes。

## Task 5: 记录验收报告

- [x] **Step 1: 创建报告**

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
- controlled provider fixture 配置与结果。
- Testing TestPlan 证据。
- Review raw output/gate 证据。
- Analyst/Internal Reviewer prompt contract 证据。
- reconnect 幂等验证证据。
- 未覆盖项和后续风险。

- [ ] **Step 2: 提交报告**

```bash
git add cadence/reports/2026-06-10_进度报告_CodingWorkspaceProvider驱动测试审查与恢复机制验证_v1.0.md
git commit -m "docs: record coding QA recovery verification"
```

## 阶段验收标准

- 标准 Rust 命令全部 PASS。
- 标准前端命令全部 PASS。
- controlled provider happy path、missing required step、malformed review、reconnect 幂等全部可复现通过。
- 真实或 controlled attempt 能证明 Testing 不再只靠固定命令宣称通过。
- Reviewer schema 异常不再导致前端无恢复动作地卡住。
- OpenSpec/Superpowers 在四个角色中都有 prompt 证据。
