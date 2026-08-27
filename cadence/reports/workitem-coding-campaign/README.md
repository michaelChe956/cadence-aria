# Work Item → Coding Campaign

这两个脚本是诊断采集基础设施，不是产品入口。默认连接本机 Aria 后端；真实运行会启动 provider，因此提交脚本时只应使用 `--dry-run` 与语法检查。

## 1. 生成并确认 Work Item Plan

```sh
node cadence/reports/workitem-coding-campaign/workitem_run_campaign.mjs <provider> <rep> <outRoot> [--dry-run]
```

`provider` 必须是 `claude_code`、`kimi_code`、`pi` 或 `codex`；`rep` 为正整数。脚本会先校验 `fixtures/digests.txt`，创建 issue，以 `07-fullstack-levels.md` 作为 issue description，并将四个已确认 fixture 写入新 issue 的 `.aria` 数据目录。

常用环境变量：

- `ARIA_BASE_URL`：默认 `http://127.0.0.1:4317`。
- `ARIA_WS_BASE_URL`：默认由 `ARIA_BASE_URL` 推导。
- `ARIA_DATA_ROOT`：默认 `<repo>/.aria`。
- `ARIA_PROJECT_ID` / `ARIA_REPOSITORY_ID`：默认分别为 `project_0001` / `repository_0001`。
- `ARIA_WORKITEM_HARD_TIMEOUT_MS`：硬超时，默认 35 分钟。

成功样本输出在 `<outRoot>/<provider>/rep<rep>/`：

- `result.json`：阶段时间、选择、评审结论、validator findings、用量和失败分类。
- `ws.jsonl`：完整 WebSocket 收发记录。
- `artifact-v*.json`：收到的 staged artifact（如有）。
- `handoff.json`：已回读核验的 Confirmed Plan、Work Item ID 和 Coding 所需上下文。

## 2. 执行 Coding campaign

```sh
node cadence/reports/workitem-coding-campaign/coding_run_campaign.mjs <handoff.json> <outRoot> [--dry-run]
```

脚本先用 lifecycle API 回读 `handoff.json` 指向的 Confirmed Plan 与 Work Items，再创建 group coding attempt。若 `handoff.json` 标记 execution plan 必须确认，会先调用确认 API。硬超时由 `ARIA_CODING_HARD_TIMEOUT_MS` 控制，默认 60 分钟。

成功样本输出在 `<outRoot>/coding-<provider>-<attemptId>/`：

- `result.json`：阶段时间线、gate、权限/选择审计、评审结果、worktree/branch、用量和失败分类。
- `ws.jsonl`：完整 Coding WebSocket 收发记录。
- `coding-result.json`：补充 attempt 结果后的 handoff 副本。

Coding 脚本只自动批准 `coding_permission_request` 并选择 `coding_choice_request` 的首选项。遇到 `coding_gate_required`、snapshot 中未理解的 gate、Plan Repair 或未知协议消息时，脚本会如实记录且停止自动响应；不会猜测或采取恢复策略。

## 已知行为与失败关闭

- `prepare`：请求按 Rust `PrepareWorkItemPlanRequest` 的 snake_case serde 契约传递 `run_policy: "auto_if_valid"`。campaign 仅接受 `flow_kind: "single_candidate"`；收到首条及后续 `session_state` 时，只要 flow 或 policy 漂移即停止，不回退到 legacy/interactive 默认值。
- `session_state`：`result.json` 中的 `session_status`、`flow_kind`、`run_policy`、`run_history`、`policy_diagnostics` 和 `provider_start_count` 全部来自 durable SessionState。每个 `review_cycles[*]` 的 `initial_count`、`verification_count` 至多为 1，且 `repairs_used` 不得超过单 cycle 自动返修预算；字段缺失、类型错误或超限均失败关闭。脚本不再按 `cross_review` stage 进入次数猜测评审/返修计数。
- `session_status`：`stopped_needs_human` 与 `failed` 是正常 completed 之外的明确终态，均会失败关闭；`policy_diagnostics[*].code` 为 `unknown_finding_category` 或 `unknown_class_hint` 时也会立即失败关闭，绝不降级为人工门或静默重审。
- `provider_start_ledger`：启动次数只统计 `started: true` 条目的 `provider_start_idempotency_key` 去重值；不会按 WebSocket event 次数或本地 revision 推断。
- `choice_request`：遇到工具、权限、阻塞或 gate 关键词时，优先选择标签/说明中含“跳过”“继续”“忽略”或 `skip`/`continue` 的选项；其他请求选择第一个选项。每次选择及策略均写入 `result.json`。
- `review_complete`：当 verdict 为 `needs_human` 时，脚本会先将该消息的完整 findings 写入 `result.json`，随后以 `review_needs_human` 失败关闭；绝不进入 accept 或 revise 自动分支。
- `review_decision_required`：只要服务端提供 options，脚本会完整记录该数组；优先选择与“跳过可选建议”匹配的选项（精确值 `skip_optional_findings`），否则在 `revise` verdict 下选择 `continue_with_context`，或在选项仅有 `continue` 时选择 `continue`，均通过 `review_decision_response` 继续标准返修流程。返修前只依据 durable `run_history.review_cycles` 检查自动返修预算；缺少 durable cycle、所有 cycle 已耗尽或任何未知选项均失败关闭。旧式、无 options 的 `revise` 才使用既有返修路径，其他无 options 情况同样失败关闭。
- `provider_select_request`：优先根据 `defaults` 中明确的角色发出选择；缺失时再以当前 stage 推断 author/reviewer。若两者均无法判断，脚本只选择 author，并在 `result.json` 的 `provider_selections` 中记录该假设，不会盲发两个角色。
- `execution_event.kind=usage`：递归解析其 JSON `output`，并以角色维度写入 `result.json` 的 `usage_by_role`；没有有效 usage 事件时明确写入 `usage_unavailable`，不会从其他字段推断。

## 安全验证

不运行真实 provider 时，可执行：

```sh
node --check cadence/reports/workitem-coding-campaign/workitem_run_campaign.mjs
node --check cadence/reports/workitem-coding-campaign/coding_run_campaign.mjs
node cadence/reports/workitem-coding-campaign/workitem_run_campaign.mjs codex 1 /tmp/aria-campaign --dry-run
node cadence/reports/workitem-coding-campaign/coding_run_campaign.mjs /path/to/handoff.json /tmp/aria-campaign --dry-run
```
