# Task 4.5 验收报告：prompt/scope 合并门禁与重连

## Status

- 已完成 Task 4.5 的测试合并门禁；未修改 production 文件，前置实现已满足契约，依裁决不人为制造失败。
- 本次仅新增或强化 brief 指定的两个测试文件中的断言；不调用真实 Provider。

## 覆盖与裁决

- Prompt 的 C layer 必备内容、B layer 移除、JSON/Markdown schema 排除以及质量/硬字节上限，已有 `prompt_contract.rs` 的既有细分测试覆盖；新增 `single_item_prompt_merge_gate_keeps_c_layer_and_excludes_b_layer_schema` 作为集中回归门。
- Initial/Verification reviewer calibration（`must_fix` 仅限机械漏网硬错误或明确自相矛盾、完备度为 advisory）由两个 prompt scope 测试明确锁定。
- 新增 scope 的 serde JSON 往返、Lifecycle JSON durable 读取以及 `WorkspaceEngine::new_persistent` 重建后 digest 不漂移测试。
- 新增 Initial scope 落入 Verification、Verification scope 落入 Initial 时均 `ProtocolViolation` 且 durable `Failed` 的 fail-closed 测试；该路径仅在 `SingleCandidate` flow_kind 下进入 scope policy dispatch。
- 4.4 已覆盖 Verification scope 的缺失报告、ref/IR/hash/version/digest 错误、parser 错误及 original/new fingerprint 路由；本任务不放宽任何校验。

## 验证记录

| 命令 | 结果 | 摘要 |
| --- | --- | --- |
| `cargo test --locked --lib work_item_split_engine::tests::prompt_contract -- --list` | 通过 | 23 项，brief 过滤名有效。 |
| `cargo test --locked --lib workspace_engine::tests::single_candidate_prompt -- --list` | 通过 | 14 项，brief 过滤名有效。 |
| `cargo test --locked --lib work_item_split_engine::tests::prompt_contract` | 通过 | 23 passed，0 failed。 |
| `cargo test --locked --lib workspace_engine::tests::single_candidate_prompt` | 通过 | 14 passed，0 failed。 |
| `cargo fmt --check` | 通过 | 格式检查通过。 |
| `cargo check --locked` | 通过 | 当前树编译通过。 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 通过 | 无 warning。 |
| `cargo test --locked` | 通过 | 400 passed、44 integration passed、2 doc-tests passed；12 ignored。 |
| `git diff --check` | 通过 | 无空白错误。 |

## 并行工作树说明

全量测试时共享 worktree 另有未暂存的 `src/product/workspace_engine/tests.rs` 和 `src/product/workspace_engine/tests/single_candidate_flow_dispatch.rs` 改动；二者非本任务所写，不纳入本任务提交。本任务仅按精确路径暂存下列报告与两个 brief 指定测试文件。

## Provider 提醒

本次变更涉及 Work Item Draft Prompt 契约测试。建议操作者按 `cadence/designs/2026-07-24_技术方案_WorkItemDraftPrompt测试基线_v1.0.md` 执行 Case A 与 Case B 各 10 个有效首次输出的 Claude Code 验证；未获明确授权，未调用 Provider。

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "仅更新 Task 4.5 brief 指定的 prompt_contract.rs 与 single_candidate_prompt.rs 测试，并新增报告；生产代码、字节上限和 scope 校验均未放宽。"
    }
  ],
  "changedFiles": [
    "src/product/work_item_split_engine/tests/prompt_contract.rs",
    "src/product/workspace_engine/tests/single_candidate_prompt.rs",
    ".superpowers/sdd/2026-08-27_计划文档_WorkItem阶段2单候选C′MVP_v1.0/task-4.5-report.md"
  ],
  "testsAddedOrUpdated": [
    "src/product/work_item_split_engine/tests/prompt_contract.rs - C/B layer、schema 排除和双重 prompt byte 上限合并门",
    "src/product/workspace_engine/tests/single_candidate_prompt.rs - reviewer calibration、scope JSON durable roundtrip/reconnect、Initial/Verification phase fail-closed"
  ],
  "commandsRun": [
    {
      "command": "cargo test --locked --lib work_item_split_engine::tests::prompt_contract -- --list",
      "result": "passed",
      "summary": "匹配 23 项。"
    },
    {
      "command": "cargo test --locked --lib workspace_engine::tests::single_candidate_prompt -- --list",
      "result": "passed",
      "summary": "匹配 14 项。"
    },
    {
      "command": "cargo test --locked --lib work_item_split_engine::tests::prompt_contract",
      "result": "passed",
      "summary": "23 passed，0 failed。"
    },
    {
      "command": "cargo test --locked --lib workspace_engine::tests::single_candidate_prompt",
      "result": "passed",
      "summary": "14 passed，0 failed。"
    },
    {
      "command": "cargo fmt --check",
      "result": "passed",
      "summary": "格式检查通过。"
    },
    {
      "command": "cargo check --locked",
      "result": "passed",
      "summary": "编译通过。"
    },
    {
      "command": "cargo clippy --all-targets --all-features --locked -- -D warnings",
      "result": "passed",
      "summary": "无 warning。"
    },
    {
      "command": "cargo test --locked",
      "result": "passed",
      "summary": "400 passed、44 integration passed、2 doc-tests passed；12 ignored。"
    },
    {
      "command": "git diff --check",
      "result": "passed",
      "summary": "无空白错误。"
    }
  ],
  "validationOutput": [
    "Prompt merge gate 与既有细分契约测试完整通过。",
    "scope 的 serde JSON、durable Lifecycle JSON、engine reconnect 后 digest 稳定。",
    "Initial/Verification scope phase 互换均 fail-closed 并 durable Failed。",
    "全量 cargo test --locked 通过。"
  ],
  "residualRisks": [
    "未调用真实 Provider；需操作者授权后执行 Case A、Case B 各 10 次有效首次 Claude Code 输出验证。",
    "共享 worktree 存在非本任务的未暂存 tests.rs 与 single_candidate_flow_dispatch.rs 改动，已排除在本任务提交外。"
  ],
  "noStagedFiles": true,
  "diffSummary": "增加 prompt C/B layer/schema/byte-limit 合并门，并补足 reviewer calibration、scope durable JSON/reconnect 与 phase fail-closed 回归覆盖。",
  "reviewFindings": [
    "no blockers"
  ],
  "manualNotes": "前置任务已实现生产接线，按本任务裁决不人为制造失败；所有新增测试首次执行即通过。"
}
```