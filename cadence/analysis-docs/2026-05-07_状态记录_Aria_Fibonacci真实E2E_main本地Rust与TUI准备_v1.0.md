# 2026-05-07 Aria Fibonacci 真实 E2E main 本地 Rust 执行记录与 TUI 准备

## 文档信息

- 文档类型：状态记录
- 目标用途：记录一次真实 provider E2E 执行过程，并为后续 TUI 界面设计准备状态、事件、报告与阻塞诊断样本。
- Aria 仓库：`/Users/michaelche/Documents/git-folder/github-folder/cadence-aria`
- Aria 分支：`main`
- 目标工作区：`/Users/michaelche/Documents/git-folder/github-folder/naruto`
- change-id：`aria-fibonacci-square`
- task-id：`task_0001`
- providers：`real`
- 日志时间：运行日志使用 UTC 时间；本机时区为 Asia/Shanghai。

## 执行目标

在 `main` 分支上使用本地 Rust 环境，再做一次 Fibonacci 平方和案例的真实 E2E 测试。

用户请求内容：

```text
用 JavaScript 写一个函数 fibonacciSquareSum(n)，实现斐波那契平方和公式：F(1)^2 + ... + F(n)^2 = F(n) * F(n+1)。要求包含源码和基础测试。
```

本次用户额外要求：

- 使用本地 Rust 环境执行，不使用 Docker。
- 旧 E2E 产物直接删除，保证目标工作区干净重跑。

## 执行环境

本次确认使用本机 Rust 工具链：

```text
rustc 1.95.0 (59807616e 2026-04-14)
cargo 1.95.0 (f2d3ce0bd 2026-03-21)
```

执行前确认 Aria 仓库当前分支：

```text
main
```

## 执行前清理

按用户“直接删掉”指令，删除目标工作区中的旧 E2E 产物：

```text
/Users/michaelche/Documents/git-folder/github-folder/naruto/.aria
/Users/michaelche/Documents/git-folder/github-folder/naruto/openspec/changes/aria-fibonacci-square
/Users/michaelche/Documents/git-folder/github-folder/naruto/src
/Users/michaelche/Documents/git-folder/github-folder/naruto/tests
```

清理目的：

- 避免沿用上次真实 E2E 的 `.aria` runtime 状态。
- 避免旧 OpenSpec change、源码和测试影响本轮 provider 输出。
- 让 TUI 后续可以区分“干净启动”和“继续已有任务”两种模式。

## 主命令

本次使用本地 cargo 执行：

```bash
/Users/michaelche/.cargo/bin/cargo run --locked -- task run \
  --workspace /Users/michaelche/Documents/git-folder/github-folder/naruto \
  --request "用 JavaScript 写一个函数 fibonacciSquareSum(n)，实现斐波那契平方和公式：F(1)^2 + ... + F(n)^2 = F(n) * F(n+1)。要求包含源码和基础测试。" \
  --change-id aria-fibonacci-square \
  --providers real \
  --timeout 2400 \
  --report json \
  --non-interactive
```

主命令最终 stdout：

```json
{
  "blocked_report_path": "/Users/michaelche/Documents/git-folder/github-folder/naruto/.aria/runtime/tasks/task_0001/reports/blocked-report.json",
  "change_id": "aria-fibonacci-square",
  "final_summary_path": null,
  "openspec_change_dir": "/Users/michaelche/Documents/git-folder/github-folder/naruto/openspec/changes/aria-fibonacci-square",
  "provider_run_refs": [
    "prun_task_0001_n04",
    "prun_task_0001_n05",
    "prun_task_0001_n06",
    "prun_task_0001_n07",
    "prun_task_0001_n08",
    "prun_task_0001_n09",
    "prun_task_0001_n08",
    "prun_task_0001_n10",
    "prun_task_0001_n11",
    "prun_task_0001_n12",
    "run_n16_0001",
    "run_n17_0001",
    "run_n18_0001",
    "run_n16_0001",
    "run_n17_0001",
    "run_n18_0001",
    "run_n16_0001",
    "run_n17_0001",
    "run_n18_0001",
    "run_n16_0001",
    "run_n17_0001",
    "run_n18_0001",
    "run_n16_0001",
    "run_n17_0001",
    "run_n18_0001",
    "run_n16_0001",
    "run_n17_0001",
    "run_n19_0001",
    "run_n17_0001",
    "run_n19_0001",
    "run_n17_0001",
    "run_n19_0001",
    "run_n17_0001"
  ],
  "report_path": "/Users/michaelche/Documents/git-folder/github-folder/naruto/.aria/runtime/tasks/task_0001/reports/final-report.json",
  "status": "blocked_by_gate",
  "task_id": "task_0001",
  "testing_report_path": "/Users/michaelche/Documents/git-folder/github-folder/naruto/.aria/runtime/tasks/task_0001/reports/testing-report.json"
}
```

## 节点进展摘要

本轮真实 E2E 已通过规划链主路径，并进入执行链：

| 节点 | 结果 | 说明 |
|------|------|------|
| N04 | completed | clarification 完成 |
| N05 | completed | spec 完成 |
| N06 | completed | spec gate 完成 |
| N07 | completed | design 完成 |
| N08 | completed | design review 完成 |
| N09 | completed | design revision 完成 |
| N08 | completed | 二次 design review 完成 |
| N10 | completed | readiness check 完成 |
| N11 | completed | plan 完成 |
| N12 | completed | dispatch package 完成 |
| N16 | completed，多次 | coding worktask 可返回，本次未复现 N16 timeout |
| N17 | completed，多次 | testing report 节点可返回，但在 wt-006 上报告失败 |
| N18 | completed，多次 | code review report 节点可返回 |
| N19 | completed，多次 | rework 节点可返回，但未解除 wt-006 阻塞 |

关键观察：

- 之前关注的 N16 timeout 本轮未复现。
- Fibonacci 源码与测试已经生成，并通过独立 Node 测试。
- E2E 最终没有 completed，而是因归档 worktask 的门禁失败进入 `blocked_by_gate`。

## 生成的业务产物

本轮在目标工作区生成：

```text
/Users/michaelche/Documents/git-folder/github-folder/naruto/src/fibonacciSquareSum.js
/Users/michaelche/Documents/git-folder/github-folder/naruto/tests/fibonacciSquareSum.test.js
```

实现摘要：

- `fibonacciSquareSum(n)` 使用滚动 Fibonacci 计算 `F(n) * F(n + 1)`。
- CommonJS 导出 `{ fibonacciSquareSum }`。
- JSDoc 中记录 `1 <= n <= 39` 的安全整数范围。

测试摘要：

- 覆盖 `n = 1, 2, 3, 5, 10, 20`。
- 覆盖 JSDoc 安全范围说明。
- provider 执行过程中曾完成 `prettier`、`eslint`、`node --test`、coverage 与 `node --check` 等验证。

## 独立人工验证结果

主 E2E 返回后，在目标工作区重新执行 Node 测试：

```bash
node --test tests/fibonacciSquareSum.test.js
```

结果：

```text
tests 7
pass 7
fail 0
duration_ms 114.700253
```

覆盖率命令：

```bash
node --test --experimental-test-coverage --test-coverage-lines=80 --test-coverage-functions=80 --test-coverage-branches=80 tests/*.test.js
```

结果：

```text
tests 7
pass 7
fail 0
src/fibonacciSquareSum.js line 100.00 branch 100.00 funcs 100.00
all files line 100.00 branch 100.00 funcs 100.00
```

结论：Fibonacci 实现与测试本身通过。E2E 阻塞不是业务代码失败。

## 最终状态与报告

最终 `state.json`：

```json
{
  "change_id": "aria-fibonacci-square",
  "current_worktask": "work_wt_006",
  "openspec_bootstrap_status": "bootstrapped",
  "phase": "blocked_by_gate",
  "task_id": "task_0001"
}
```

`final-report.json`：

```json
{
  "blocked_report_path": "/Users/michaelche/Documents/git-folder/github-folder/naruto/.aria/runtime/tasks/task_0001/reports/blocked-report.json",
  "change_id": "aria-fibonacci-square",
  "status": "blocked_by_gate",
  "task_id": "task_0001"
}
```

`blocked-report.json`：

```json
{
  "next_node": "X08",
  "reason": "rework_limit_exceeded",
  "status": "blocked_by_gate",
  "task_id": "task_0001"
}
```

当前 reports 目录关键文件：

```text
/Users/michaelche/Documents/git-folder/github-folder/naruto/.aria/runtime/tasks/task_0001/reports/blocked-report.json
/Users/michaelche/Documents/git-folder/github-folder/naruto/.aria/runtime/tasks/task_0001/reports/final-report.json
/Users/michaelche/Documents/git-folder/github-folder/naruto/.aria/runtime/tasks/task_0001/reports/testing-report.json
```

## 阻塞原因

阻塞 worktask：`work_wt_006`

`work_wt_006` 的目标是归档：

- 设计说明到 `cadence/designs/`
- 测试报告到 `cadence/reports/`

实际问题：

- 归档任务需要写 `cadence/`。
- 但 dispatch 或 node contract 给到该任务的允许写入范围没有包含 `cadence/`。
- 最新 testing report 中该节点的约束甚至表现为 `allowed_write_scope=[]`，即不允许写任何文件。
- 因此 provider 无法在不违反 `do_not_write_outside_allowed_scope` 的前提下完成归档。
- N17/N19 反复返工后达到 rework limit，最终进入 `blocked_by_gate`。

testing report 中的关键失败描述：

```text
未发现归档到 cadence/designs/ 与 cadence/reports/ 的文件。
node_contract.allowed_write_scope=[]，本节点不得写入任何文件。
当前节点不能在不违反 do_not_write_outside_allowed_scope 的前提下完成 wt-006。
```

这说明当前阻塞更接近 dispatch routing / write-scope contract 问题，而不是 provider 能力、Fibonacci 代码质量或测试质量问题。

## TUI 界面准备记录

本次执行暴露了 TUI 需要优先呈现的几类信息。

### 任务总览区域

建议展示：

- `task_id`
- `change_id`
- 目标 workspace
- 当前分支
- provider 模式：`real` 或 `fake`
- timeout
- 当前 phase：例如 `execution`、`blocked_by_gate`
- 当前 worktask：例如 `work_wt_006`
- 最终 status：例如 `blocked_by_gate`

本轮样本值：

```text
task_id=task_0001
change_id=aria-fibonacci-square
phase=blocked_by_gate
current_worktask=work_wt_006
status=blocked_by_gate
reason=rework_limit_exceeded
```

### 节点时间线

TUI 应从 `logs/node-events.jsonl` 流式展示节点进入和退出事件：

- `node_enter`
- `node_exit`
- `node_id`
- `status`
- `duration_ms`
- `provider_run_id`
- `context_package_ref`
- `output_schema`

本次特别需要展示 repeated node pattern：

```text
N16 -> N17 -> N18 -> N16 -> N17 -> N18 -> ... -> N19 -> N17 -> N19 -> N17
```

这类重复模式在纯日志中难以识别，TUI 应提供：

- 重复节点高亮。
- rework 次数计数。
- 最近一次失败原因摘要。
- 当前是否接近 rework limit。

### Worktask 详情面板

本次阻塞发生在 `work_wt_006`，TUI 需要能展示：

- worktask 标题和目标。
- acceptance target，例如 `ac-003`。
- allowed write scope。
- node contract。
- provider 实际尝试写入的路径。
- testing report 的 failures。
- remaining risks。

本轮最有价值的诊断字段：

```text
worktask_id=work_wt_006
acceptance_target=ac-003
allowed_write_scope=[]
expected_archive_paths=cadence/designs/, cadence/reports/
status=failed
```

### Gate / Blocked 视图

当最终状态是 `blocked_by_gate` 时，TUI 不应只显示“失败”，而应区分：

- 业务实现失败。
- 测试失败。
- provider 超时。
- provider 额度/鉴权失败。
- contract 或 write scope 阻塞。
- rework limit exceeded。

本轮应归类为：

```text
category=contract_write_scope_blocked
status=blocked_by_gate
reason=rework_limit_exceeded
next_node=X08
```

### 报告与产物导航

TUI 应提供一键打开或查看：

- `final-report.json`
- `blocked-report.json`
- `testing-report.json`
- `node-events.jsonl`
- provider run refs
- 生成源码
- 生成测试
- OpenSpec change dir

本轮关键路径：

```text
.aria/runtime/tasks/task_0001/reports/final-report.json
.aria/runtime/tasks/task_0001/reports/blocked-report.json
.aria/runtime/tasks/task_0001/reports/testing-report.json
.aria/runtime/tasks/task_0001/logs/node-events.jsonl
src/fibonacciSquareSum.js
tests/fibonacciSquareSum.test.js
openspec/changes/aria-fibonacci-square
```

### 验证结果面板

TUI 需要把 E2E 状态和产物验证状态分开展示。

本轮如果只显示 overall failed，会误导用户以为 Fibonacci 实现失败。更准确的展示是：

```text
E2E overall: blocked_by_gate
Business code: generated
Unit tests: passed
Coverage gate: passed
Archive worktask: failed
Root cause: cadence/ write scope missing
```

### 操作建议

TUI 可以在检测到类似本轮阻塞时给出自动诊断建议：

1. 检查 dispatch package 中 `work_wt_006.allowed_write_scope` 是否包含 `cadence/designs/` 与 `cadence/reports/`。
2. 检查后续 N17/N19 节点是否继承了正确 write scope。
3. 若归档由专门节点完成，确认该节点 contract 允许写 `cadence/`。
4. 避免把业务代码测试通过与归档失败混为同一个失败原因。

## 后续排查建议

1. 优先定位 `work_wt_006` 的生成逻辑，确认 N12 dispatch package 为什么给归档任务不完整写入范围。
2. 检查执行链中 rework 节点是否重建或覆盖了 `allowed_write_scope`，尤其是从 `src/`、`tests/` 变成 `[]` 的路径。
3. 为归档类 worktask 增加回归测试：当任务要求写 `cadence/designs/` 与 `cadence/reports/` 时，dispatch package 与 node contract 必须包含这些路径。
4. TUI 第一版可先围绕本轮样本实现：任务总览、节点时间线、当前 worktask、报告查看、阻塞诊断五个区域。

