# Aria 真实接入检查表

用于人工核对当前任务是否真实体现并正确使用了 `claude code`、`codex`、`OpenSpec`、`superpowers`。本检查表只读，不要求创建新任务。

## 0. 前置条件

- [ ] 已执行 `pnpm build`
- [ ] 仓库中存在待检查任务目录：`cadence/cache/aria/tasks/<task-id>/`
- [ ] 如需检查最新实现，建议先执行 `pnpm check && pnpm test`

## 1. 能力探测

命令：

```bash
node dist/src/index.js aria:doctor
```

通过标准：

- [ ] 输出包含 `claude_code`
- [ ] 输出包含 `codex`
- [ ] 输出包含 `OpenSpec`
- [ ] 输出包含 `superpowers`
- [ ] `claude_code` / `codex` 如需真实运行，`available` 应为 `true`

## 2. 前段工件

文件：

- `cadence/cache/aria/tasks/<task-id>/artifacts/spec-artifact.md`
- `cadence/cache/aria/tasks/<task-id>/artifacts/plan-brief.md`

通过标准：

- [ ] `spec-artifact.md` 包含 `producer: claude-code`
- [ ] `spec-artifact.md` 包含 `source_capabilities: [OpenSpec, superpowers]`
- [ ] `spec-artifact.md` 包含 `open_spec_evidence: provider=OpenSpec`
- [ ] `spec-artifact.md` 包含 `superpowers_evidence: provider=superpowers`
- [ ] `plan-brief.md` 包含 `producer: claude-code`
- [ ] `plan-brief.md` 包含 `source_capabilities: [OpenSpec, superpowers]`
- [ ] `plan-brief.md` 包含 `open_spec_evidence: provider=OpenSpec`
- [ ] `plan-brief.md` 包含 `superpowers_evidence: provider=superpowers`

## 3. Handoff 工件

文件：

- `cadence/cache/aria/tasks/<task-id>/artifacts/execution-context-bundle.yaml`
- `cadence/cache/aria/tasks/<task-id>/artifacts/dispatch-contract-exec-01.yaml`

通过标准：

- [ ] `execution-context-bundle.yaml` 包含 `source_capabilities`
- [ ] `execution-context-bundle.yaml` 同时列出 `OpenSpec` 与 `superpowers`
- [ ] `execution-context-bundle.yaml` 包含 `required_methods`
- [ ] `execution-context-bundle.yaml` 包含 `writing-plans`
- [ ] `execution-context-bundle.yaml` 包含 `test-driven-development`
- [ ] `execution-context-bundle.yaml` 包含 `verification-before-completion`
- [ ] `dispatch-contract-exec-01.yaml` 包含 `worker_cli: codex`
- [ ] `dispatch-contract-exec-01.yaml` 包含 `required_methods`
- [ ] `dispatch-contract-exec-01.yaml` 包含 `verification-before-completion`

## 4. 执行与 review/test 工件

文件：

- `cadence/cache/aria/tasks/<task-id>/artifacts/exec-result-exec-01.yaml`
- `cadence/cache/aria/tasks/<task-id>/artifacts/review-report.yaml`
- `cadence/cache/aria/tasks/<task-id>/artifacts/test-report.yaml`

通过标准：

- [ ] `exec-result-exec-01.yaml` 包含 `capabilities_used`
- [ ] `exec-result-exec-01.yaml` 包含 `- codex`
- [ ] `exec-result-exec-01.yaml` 包含 `openspec_refs_consumed`
- [ ] `exec-result-exec-01.yaml` 包含 `artifacts/spec-artifact.md`
- [ ] `exec-result-exec-01.yaml` 包含 `superpowers_refs_consumed`
- [ ] `exec-result-exec-01.yaml` 包含 `test-driven-development`
- [ ] `exec-result-exec-01.yaml` 包含 `verification-before-completion`
- [ ] `review-report.yaml` 包含 `producer: claude-code`
- [ ] `review-report.yaml` 的 `source_capabilities` 同时列出 `OpenSpec` 与 `superpowers`
- [ ] `review-report.yaml` 包含 `verdict: passed` 或明确失败原因
- [ ] `test-report.yaml` 包含 `producer: claude-code`
- [ ] `test-report.yaml` 的 `source_capabilities` 同时列出 `OpenSpec` 与 `superpowers`
- [ ] `test-report.yaml` 包含 `verdict: passed` 或明确失败原因

## 5. 最终状态

建议命令：

```bash
node dist/src/index.js aria:status --task-id <task-id>
node dist/src/index.js aria:result --task-id <task-id>
```

通过标准：

- [ ] 最终状态为 `verified`、`patching` 或 `blocked`
- [ ] 若状态为 `verified`，应同时存在 `review-report.yaml` 与 `test-report.yaml`
- [ ] 若状态为 `verified`，应同时满足 `review_status: passed` 与 `test_status: passed`

## 6. 快速结论

- [ ] 能力探测正确
- [ ] 前段来源证明正确
- [ ] Handoff 注入字段正确
- [ ] Exec 消费证据正确
- [ ] Review/Test 闭环正确
- [ ] 最终状态与报告一致
