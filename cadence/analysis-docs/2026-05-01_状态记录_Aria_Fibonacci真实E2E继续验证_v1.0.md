# 2026-05-01 Aria Fibonacci 真实 E2E 继续验证状态

## 当前验证目标

- Aria worktree：`/Users/michaelche/Documents/git-folder/github-folder/cadence-aria/.worktrees/task-run-e2e-entry`
- 分支：`aria/task-run-e2e-entry`
- 目标工作区：`/Users/michaelche/Documents/git-folder/github-folder/naruto`
- change-id：`aria-fibonacci-square`
- 请求：
  `用 JavaScript 写一个函数 fibonacciSquareSum(n)，实现斐波那契平方和公式：F(1)^2 + ... + F(n)^2 = F(n) * F(n+1)。要求包含源码和基础测试。`

## 进程状态

- 当前没有残留进程：
  - `target/debug/aria task run`
  - `claude -p --permission-mode dontAsk`
  - `codex exec`
- 最新真实 E2E 已自然停止在第二次 N08 provider 调用失败。

## 今天新增修复

1. `SpecProjection` 支持真实 provider 的普通编号功能需求列表。
   - 失败现象：N05 生成的 `## 功能需求` 使用普通 `1. ...` 编号列表，无 `REQ/FR` 显式 ID，导致 `projection compile failed: empty projection payload SpecProjection`。
   - 修复：`compile_spec_projection()` 在功能需求 entry 为空时，使用已有 `synthetic_entries_from_section_tree(..., "req-")` 合成 `req-001` 等 ID。
   - 回归测试：`spec_projection_synthesizes_functional_requirements_from_real_provider_numbered_list`。

2. N09 设计修订 prompt 现在内联 `SpecProjection`。
   - 失败风险：N09 之前只拿当前 design 和 N08 review findings，未拿完整 spec 约束，真实修订把 JavaScript 需求漂移成 Python 伪代码和 `ValueError`。
   - 修复：`run_design_revision()` 新增 `spec_projection` 输入，并在 N09 canonical input summary / adapter input 中包含 `spec_projection_ref` 与 `spec_projection_payload`。
   - 回归测试：`fake_provider_routes_revise_review_to_n09_and_back_to_n08` 断言 N09 prompt 包含 `[spec_projection_payload]` 与 `success_criteria`。

## 最新真实 E2E 节点进展

- N04 clarification：完成。
- N05 spec：完成；已越过普通编号功能需求列表导致的 SpecProjection 空 payload 问题。
- N06 spec gate：完成，decision pass。
- N07 design：完成，但首次输出跑题，生成了看板 / REST / AIService 等与 fibonacci 无关的设计。
- N08 design review：完成，正确给出 fail，并指出设计主题完全不匹配、缺失核心算法设计、测试设计不对应。
- N09 design revision：完成，生成了回到 fibonacci 主题的修订设计，但仍存在语言漂移：使用 Python 伪代码而不是 JavaScript。
- 第二次 N08 design review：provider 执行失败，未获得模型评审结果。

## 最新阻塞

第二次 N08 的 Codex provider stderr 显示额度不足：

```text
unexpected status 403 Forbidden: 预扣费额度失败, 用户剩余额度: ＄0.000946, 需要预扣费额度: ＄0.011546
```

因此当前无法继续真实 provider E2E。该失败不是代码崩溃，也不是 projection 编译失败。

## Naruto 当前现场

保留以下真实 E2E 产物，便于继续检查：

- `/Users/michaelche/Documents/git-folder/github-folder/naruto/.aria/`
- `/Users/michaelche/Documents/git-folder/github-folder/naruto/openspec/changes/aria-fibonacci-square/`
- 关键日志：
  - `.aria/runtime/tasks/task_0001/logs/node-events.jsonl`
  - `.aria/runtime/tasks/task_0001/provider-runs/prun_task_0001_n08.json`
  - `.aria/runtime/provider-streams/codex-44878-stderr.log`
- 关键产物：
  - `.aria/runtime/tasks/task_0001/artifacts/spec/art_spec_task_0001_0001_v0001.md`
  - `.aria/runtime/tasks/task_0001/artifacts/design/art_design_task_0001_0001_v0001.md`
  - `.aria/runtime/tasks/task_0001/artifacts/design_revision_record/art_design_revision_record_task_0001_0001_v0001.json`

## 已通过验证

- `/Users/michaelche/.cargo/bin/cargo test --test spec_projection`：20 passed
- `/Users/michaelche/.cargo/bin/cargo test --test planning_chain_fake_provider`：11 passed
- `/Users/michaelche/.cargo/bin/cargo test --test execution_chain_fake_provider`：12 passed
- `/Users/michaelche/.cargo/bin/cargo check --locked`：passed

## 继续建议

1. 解决 Codex provider 额度后，先清理 `naruto` 真实 E2E 产物：

```bash
rm -rf /Users/michaelche/Documents/git-folder/github-folder/naruto/.aria \
       /Users/michaelche/Documents/git-folder/github-folder/naruto/openspec/changes/aria-fibonacci-square \
       /Users/michaelche/Documents/git-folder/github-folder/naruto/src \
       /Users/michaelche/Documents/git-folder/github-folder/naruto/tests
```

2. 重新运行真实 E2E：

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

3. 若真实 E2E 通过，再跑完整门禁：

```bash
/Users/michaelche/.cargo/bin/cargo fmt --check
/Users/michaelche/.cargo/bin/cargo check --locked
/Users/michaelche/.cargo/bin/cargo test --locked -j 1
```

