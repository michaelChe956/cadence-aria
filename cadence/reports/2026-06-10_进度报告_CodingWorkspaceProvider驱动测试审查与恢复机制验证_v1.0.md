# CodingWorkspaceProvider 驱动测试审查与恢复机制验证报告

## 基本信息

- 验证日期：2026-06-10
- worktree：`.worktrees/bugfix_test_branch`
- 分支：`bugfix_test_branch`
- HEAD：`acff077`
- 服务地址：验证时后端 `http://127.0.0.1:4317`，前端 `http://127.0.0.1:5173`
- 测试 Project：`project_0002`
- 测试 Repository：`repository_0001`
- 测试 Issue：`issue_0001`
- 测试 Work Item：`work_item_0001`
- 临时目标仓库：`/tmp/aria-e2e-repo.7x4jhr`

## 验证结论

P4 controlled provider 验收已覆盖 Tester 两段式 TestPlan、required step blocked gate、review alias/malformed recovery、blocked gate reconnect 幂等。过程中发现并修复两处 test controls 问题：

- `execute_test_plan` prompt 同时包含 `plan_tests` 文本时，fixture 误返回 TestPlan，导致 step results 丢失。
- 同一 attempt 只能保存一个 review fixture，Analyst 会先消费 CodeReviewer fixture，导致无法稳定验证 Code Review recovery。

真实浏览器页面点击启动 Coding attempt 未执行，P4 Task 4 Step 2 保持未完成；本轮使用 WebSocket driver 直接覆盖同一后端 pipeline。

## Controlled E2E 证据

### Happy Path

- attempt：`coding_attempt_0005`
- TestPlan：`test_plan_0001`
- report：`testing_report_0001`
- 结果：`overall_status=passed`
- steps：`unit`、`api_smoke`
- missing/skipped：均为空
- raw output：`provider-raw/testing/execute_test_plan_0001.txt`

### Missing Required Step

- attempt：`coding_attempt_0006`
- plan steps：`unit`、required `security`
- execute results：仅返回 `unit`
- report：`overall_status=blocked`
- missing：`["security"]`
- blocked gate：`coding_blocked_gate_0001`
- reason：`missing_required_steps`
- actions：`retry_test_plan`、`rerun_missing_steps`、`provide_context`、`manual_continue`、`abort`
- reconnect：重连后 `pending_gates` 恢复同一个 `coding_blocked_gate_0001`，未新增重复 gate/report

### Review Alias

- attempt：`coding_attempt_0008`
- Analyst fixture：`{"verdict":"no_issue"}`
- CodeReviewer fixture：`request_changes`，finding 只包含 `file`、`description`、`recommendation`
- report：`code_review_0001`
- 结果：finding 被保留并补齐为 `severity=warning`、`source_stage=code_review`
- raw output：`provider-raw/code_review/code_review_0001.txt`

### Review Malformed

- attempt：`coding_attempt_0009`
- Analyst fixture：`{"verdict":"no_issue"}`
- CodeReviewer fixture：`not json at all`
- report：`code_review_0001`
- verdict：`blocked`
- blocked gate：`coding_blocked_gate_0001`
- reason：`review_blocked`
- actions：`retry_review`、`send_raw_output_to_analyst`、`provide_context`、`manual_continue`、`abort`
- raw output：`provider-raw/code_review/code_review_0001.txt`

## Prompt Contract 证据

- `cargo test --locked --lib prompt` 通过。
- 覆盖 `tester_plan_prompt_requires_openspec_superpowers_and_step_bound_tools`。
- 覆盖 `rework_and_internal_review_prompts_require_openspec_and_superpowers`。
- 源码中 Tester prompt 包含 `[openspec_contract]`、`[superpowers_contract]`、Story Spec、Design Spec、Work Item 与 `step_id` 约束。
- Analyst/Internal Reviewer prompt 包含 EvaluationContextPack 与 contract。

## 修复验证

- `cargo test --locked --lib testing_fixture_fake_provider_emits_plan_and_step_results`：先 RED，修复后通过。
- `cargo test --locked --lib review_fixture_provider_consumes_queued_outputs_in_order`：先 RED，修复后通过。
- `cargo test --locked --lib fixture`：9 个 fixture 相关测试通过。
- `cargo test --locked --lib prompt`：12 个 prompt 相关测试通过。

## 最终验证命令

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过。
- `cargo check --locked`：通过。
- `cargo test --locked`：通过。
- `pnpm -C web test`：38 个测试文件、326 个测试通过。
- `pnpm -C web build`：通过；保留 Vite chunk size warning。

## 未覆盖项与风险

- 未通过浏览器页面手工点击启动 Coding attempt；P4 Task 4 Step 2 未勾选。
- 未接入真实外部 Provider 运行完整代码修改、测试、审查，只使用 test controls 和 fake provider 覆盖恢复机制。
- Prompt contract 在本轮以源码构造和单元测试验证，未作为 E2E raw prompt artifact 落盘核查。
