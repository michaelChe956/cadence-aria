# 2026-04-30 Aria Fibonacci E2E 状态记录

## 当前验证目标

- 测试项目：`/Users/michaelche/Documents/git-folder/github-folder/naruto`
- Aria worktree：`/Users/michaelche/Documents/git-folder/github-folder/cadence-aria/.worktrees/task-run-e2e-entry`
- Aria 分支：`aria/task-run-e2e-entry`
- change-id：`aria-fibonacci-square`
- 请求：
  `用 JavaScript 写一个函数 fibonacciSquareSum(n)，实现斐波那契平方和公式：F(1)^2 + ... + F(n)^2 = F(n) * F(n+1)。要求包含源码和基础测试。`

## 进程状态

- 当前端到端验证已暂停。
- 已停止：
  - `aria task run` PID `65454`
  - 其子进程 `claude -p --permission-mode dontAsk ...` PID `66465`
- 复查时没有残留 `aria task run` / `claude -p --permission-mode dontAsk` / `codex exec` 进程。

## Naruto 当前现场

- `naruto` 工作区有未跟踪验证产物：
  - `.aria/`
  - `openspec/changes/`
- 当前保留这些产物，便于晚上继续检查。
- 第四次真实 E2E 已完成并落盘：
  - `N04` clarification
  - `N05` spec
  - `N06` spec gate
  - `SpecProjection`
- 第四次真实 E2E 暂停位置：
  - `N07` Claude 设计生成阶段
  - 暂停前尚未落 `artifacts/design/*`

## 今天已修复的真实 E2E 问题

1. `SpecProjection` 无法解析真实 Claude 输出的 `| ID | 描述 |` 表格。
   - 修复：把 `描述` 作为通用正文字段别名。
   - 同时修复 inline code 末尾反引号被误删的问题。
   - 回归测试：`spec_projection_accepts_requirement_table_description_header`

2. `N08 pass` 后 OpenSpec `design.md` 写回使用原始设计 Markdown，导致无显式 `[DEC-*]` / `[CMP-*]` 时 `design constraints are empty`。
   - 修复：基于 `DesignProjection` 生成 OpenSpec-friendly `design.md`，将合成 ID 写成 `[DEC-001]` / `[CMP-001]`。
   - 同时修复 tasks 写回只识别 `dd-`、不识别 `dec-` 的问题。
   - 回归测试：`planning_chain_writes_openspec_design_from_synthesized_projection_ids`

3. `DesignProjection` 无法从普通编号列表合成设计决策 ID。
   - 修复：`设计决策` section 在无显式 ID 且无可用表格时，使用列表项合成 `dec-*`。
   - 回归测试：`design_projection_synthesizes_decision_ids_from_numbered_decision_list`

## 已通过的验证

- `cargo test --test spec_projection`：19 passed
- `cargo test --test planning_chain_fake_provider`：7 passed
- `cargo test --test design_projection`：8 passed

之前同一轮也通过过：

- `cargo fmt --check`
- `cargo check --locked`
- `cargo test --locked -j 1`

注意：新增后续修复后，还需要晚上重新跑完整门禁。

## 当前 Aria 未提交改动范围

主要涉及：

- 真实 provider CLI/输出处理：`adapter_compatibility.rs`、`cli_adapter.rs`
- provider 输出解析和 prompt/context：`provider_adapter.rs`、`provider_context_builder.rs`、`prompt_template_registry.rs`
- 规划链稳健性：`clarification.rs`、`design_review.rs`、`design_revision.rs`、`plan_dispatch.rs`
- 投影与 OpenSpec 兼容：`artifact_projection.rs`、`artifact_validate.rs`、`openspec_constraints.rs`
- 回归测试：`spec_projection.rs`、`design_projection.rs`、`planning_chain_fake_provider.rs`、`cli_adapter_baseline.rs`、`context_builder.rs`、`openspec_bundle.rs`、`provider_adapter_baseline.rs`、`artifact_schema_min_fields.rs`
- 新增测试：`tests/clarification_record.rs`

## 晚上继续建议

1. 先确认没有残留进程：
   `ps -eo pid,ppid,stat,etime,command | rg "target/debug/aria task run|claude -p --permission-mode dontAsk|codex exec"`

2. 如需干净重跑，清理当前 naruto 验证产物：
   `rm -rf /Users/michaelche/Documents/git-folder/github-folder/naruto/.aria /Users/michaelche/Documents/git-folder/github-folder/naruto/openspec/changes/aria-fibonacci-square`

3. 重新运行真实 E2E：
   `/Users/michaelche/.cargo/bin/cargo run --locked -- task run --workspace /Users/michaelche/Documents/git-folder/github-folder/naruto --request "用 JavaScript 写一个函数 fibonacciSquareSum(n)，实现斐波那契平方和公式：F(1)^2 + ... + F(n)^2 = F(n) * F(n+1)。要求包含源码和基础测试。" --change-id aria-fibonacci-square --providers real --timeout 2400 --report json --non-interactive`

4. 若真实 E2E 通过，再跑完整门禁：
   - `cargo fmt --check`
   - `cargo check --locked`
   - `cargo test --locked -j 1`

