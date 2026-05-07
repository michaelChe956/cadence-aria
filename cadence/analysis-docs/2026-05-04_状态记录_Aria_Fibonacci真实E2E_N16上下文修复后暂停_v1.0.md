# 2026-05-04 Aria Fibonacci 真实 E2E 暂停状态

## 当前暂停点

- Aria worktree：`/Users/michaelche/Documents/git-folder/github-folder/cadence-aria/.worktrees/task-run-e2e-entry`
- 分支：`aria/task-run-e2e-entry`
- 目标工作区：`/Users/michaelche/Documents/git-folder/github-folder/naruto`
- change-id：`aria-fibonacci-square`
- 请求：
  `用 JavaScript 写一个函数 fibonacciSquareSum(n)，实现斐波那契平方和公式：F(1)^2 + ... + F(n)^2 = F(n) * F(n+1)。要求包含源码和基础测试。`

当前已经清理 Naruto 上一轮真实 E2E 产物，但尚未启动下一轮真实 E2E。

已确认以下路径不存在：

- `/Users/michaelche/Documents/git-folder/github-folder/naruto/.aria`
- `/Users/michaelche/Documents/git-folder/github-folder/naruto/openspec/changes/aria-fibonacci-square`
- `/Users/michaelche/Documents/git-folder/github-folder/naruto/src`
- `/Users/michaelche/Documents/git-folder/github-folder/naruto/tests`

## 进程状态

- 没有残留 `target/debug/aria task run` 进程。
- 没有残留 `claude -p --permission-mode dontAsk` provider 进程。
- 没有残留 `codex exec` provider 进程。
- `pgrep -af codex` 只显示当前 Codex 会话自身：
  - `node /usr/local/bin/codex`
  - `.../codex/codex`

## 本轮真实 E2E 结果

本轮清理 Naruto 后重跑真实 provider E2E，流程推进到 N16：

- N04 clarification：completed
- N05 spec：completed
- N06 spec gate：completed
- N07 design：completed
- N08 design review：首次 fail 后进入 N09
- N09 design revision：completed
- N08 design review：第二次 pass
- N10 readiness：completed
- N11 plan：completed
- N12 dispatch：completed
- N16 coding：failed

最终 CLI JSON：

```json
{
  "status": "blocked_by_gate",
  "blocked_report_path": "/Users/michaelche/Documents/git-folder/github-folder/naruto/.aria/runtime/tasks/task_0001/reports/blocked-report.json",
  "testing_report_path": null
}
```

blocked report：

```json
{
  "status": "blocked_by_gate",
  "reason": "provider_timeout",
  "next_node": "X08",
  "task_id": "task_0001"
}
```

N16 provider 证据：

- `run_n16_0001/run.json` 记录 `error_code = provider_timeout`，`timeout_status = hard_timeout_killed`，`duration_ms = 2400025`。
- `codex-29826-stdout.log` 为 0 字节。
- `codex-29826-stderr.log` 只有 Codex 会话头与输入 prompt，没有模型完成内容。
- N16 未创建 `src/` 或 `tests/`。

结论：这次阻断不是测试失败，也不是 projection 编译失败；真实 provider 在 N16 coding 阶段没有产出。

## 本轮新增修复

### 1. N16/N17/N18 prompt 补充真实执行上下文

根因方向：执行链给真实 provider 的 execution prompt 只有极简摘要，例如 `worktask work_wt_001`，`projection_summary` 也只是固定文本 `spec/design/plan projection summary`。N16 没有看到 active work package 描述、验收目标、路由写范围、完整 plan projection；N18 也缺少前序 coding/testing 报告细节。

修复文件：

- `src/runtime_units/coding.rs`

主要变化：

- `builder_input()` 不再只传 `worktask <id>` 摘要。
- 新增 `canonical_inputs_for_node()`，把以下内容序列化进 prompt 可见上下文：
  - `artifact_refs`
  - `prior_artifacts`
  - `risk_registry_ref`
  - `acceptance_targets`
  - `active_work_package`
  - `worktask_routing`
  - `worktree_path`
- `projection_summary` 现在包含 `projection_refs` 与 `plan_projection` JSON。
- N17/N18 能通过 `prior_artifacts` 看到前序 coding/testing 报告。

这是针对真实 provider 上下文不足的修复；下一轮真实 E2E 尚未验证。

### 2. 执行链回归测试

修复文件：

- `tests/execution_chain_fake_provider.rs`

新增测试：

- `coding_prompt_includes_worktask_plan_and_routing_context_for_real_providers`
  - 先红灯：N16 prompt 不含 work package 描述。
  - 修复后绿灯。
- `review_prompt_includes_prior_coding_and_testing_reports_for_real_providers`
  - 先红灯：N18 prompt 不含前序 coding report。
  - 修复后绿灯。

## 已通过验证

本轮修复后的验证结果：

```bash
/Users/michaelche/.cargo/bin/cargo test --test execution_chain_fake_provider prompt_includes
```

- 2 passed

```bash
/Users/michaelche/.cargo/bin/cargo test --test execution_chain_fake_provider
```

- 14 passed

```bash
/Users/michaelche/.cargo/bin/cargo fmt --check
```

- passed

```bash
/Users/michaelche/.cargo/bin/cargo check --locked
```

- passed

```bash
/Users/michaelche/.cargo/bin/cargo test --locked -j 1
```

- passed

## 还未做

- 尚未用修复后的代码重跑真实 E2E。
- Naruto 已清理干净，下一步可以直接启动真实 E2E。

## 晚上继续时的第一步

在 Aria worktree 中运行：

```bash
cd /Users/michaelche/Documents/git-folder/github-folder/cadence-aria/.worktrees/task-run-e2e-entry
/Users/michaelche/.cargo/bin/cargo run --locked -- task run \
  --workspace /Users/michaelche/Documents/git-folder/github-folder/naruto \
  --request "用 JavaScript 写一个函数 fibonacciSquareSum(n)，实现斐波那契平方和公式：F(1)^2 + ... + F(n)^2 = F(n) * F(n+1)。要求包含源码和基础测试。" \
  --change-id aria-fibonacci-square \
  --providers real \
  --timeout 2400 \
  --report json \
  --non-interactive
```

重点观察：

- N16 是否不再 `provider_timeout`。
- 是否生成 `src/` 与 `tests/`。
- N17 是否产出 `testing-report.json`。
- N18 是否再次出现空 stdout / parse error / timeout。

结束后读取：

- `/Users/michaelche/Documents/git-folder/github-folder/naruto/.aria/runtime/tasks/task_0001/reports/final-report.json`
- `/Users/michaelche/Documents/git-folder/github-folder/naruto/.aria/runtime/tasks/task_0001/reports/blocked-report.json`，如存在
- `/Users/michaelche/Documents/git-folder/github-folder/naruto/.aria/runtime/tasks/task_0001/reports/testing-report.json`，如存在

## 注意事项

- 当前 worktree 有大量既有未提交修改，不要回退不属于本轮工作的内容。
- 本轮明确涉及的修改是：
  - `src/runtime_units/coding.rs`
  - `tests/execution_chain_fake_provider.rs`
  - 本状态记录文档
- 之前已完成的 design projection 修复仍保留：
  - `src/cross_cutting/artifact_projection.rs`
  - `tests/design_projection.rs`
- Naruto 已清理，不需要晚上再次清理，除非中途又启动过真实 E2E。
