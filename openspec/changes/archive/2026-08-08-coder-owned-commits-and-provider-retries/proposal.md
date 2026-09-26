## Why

组级 Coding Attempt 在 Work Item 完成时由 Aria 对共享 worktree 执行自动暂存与提交。该行为会把其他 Work Item、生成物或 Coder 已决定不提交的改动一并纳入提交，已导致目标仓库的 `node_modules` 被错误提交。与此同时，Coder 或 Work Item Reviewer 的 Provider 因中断、超时或上游 5xx 失败时会立即进入人工门禁，用户需要为瞬时故障频繁手动恢复。

## What Changes

- 将 Work Item 完成提交的职责交给 Coder：Coder 按当前 Work Item 的规范性 `write_policy` 精确暂存并创建提交；Aria 不再自动 `git add` 或 `git commit`，只读取 UnitRun 的起始提交和 Coder 完成后的当前 `HEAD`，以二者构成该 Work Item 的提交证据区间。
- 在 Coder 与 Work Item Code Reviewer 的 Provider 调用外层增加有界技术失败重试：首次调用之外最多自动重试 2 次，耗尽后才进入既有人工处理。
- 将自动重试的运行、原始输出、失败原因和 UI 时间线持久化为独立可审计记录；自动重试不计入 Coder rework 次数。
- 明确技术失败、正常审查结论、结构化输出错误与 Plan Repair 的边界，确保 Provider 重试、Coder rework 和 Plan Repair 按优先级顺序协作。

## Capabilities

### New Capabilities

- `coder-owned-work-item-commit`: Coder 按 Work Item 写入策略精确提交，Aria 仅记录该提交。
- `coding-provider-transport-retry`: Coder 与 Work Item Code Reviewer 的技术失败自动重试、审计与耗尽后人工恢复。

### Modified Capabilities

- `coding-workspace-completion`: Group Work Item completion commit 的来源由 Aria 自动提交改为 Coder 已创建的 `HEAD`。

## Impact

- 后端：`coding_workspace_engine` 的完成提交、既有提交范围事实、流式 Provider 执行、role run/timeline 持久化和失败门禁；对应 Git service 调用收敛为只读 Git 事实查询。
- Prompt：Coder 运行期上下文增加按 `write_policy` 精确暂存、提交与自检的责任声明，不引入硬编码目录黑名单。
- 前端：显示自动重试进度和每次角色运行；最终失败仍沿用人工重试或终止入口。
- 不修改 Coder rework 的上限语义，不删除 Plan Repair，不对 Plan Repair Workspace Provider、内部组级 Reviewer 或用户主动取消启用本次自动重试。
