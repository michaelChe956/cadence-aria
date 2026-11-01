## Why

每个 Work Item 已经过 Coder 自测和独立 Reviewer 审查后，再对整组执行 Provider 型 Group Final Review 会增加第三层 AI 判断、分片/归约状态与不可靠的结构化输出边界。实际失败表明，归约 Provider 在没有足够完整 finding 上下文时会猜测路由字段，造成 `reduction_output_invalid`；这一层的复杂度没有与其价值相称。

## What Changes

- **BREAKING**：新 Group Coding Attempt 不再启动 shard、reduction 或任何 Internal Group Reviewer Provider 调用。
- 在所有 Work Item 的 Coder、独立 Code Reviewer、completion commit、handoff 与计划绑定完成后，服务端生成客观的组就绪检查结果。
- 将组级最终步骤改为人工 Final Confirm：用户查看各 Work Item 的提交、审查结论、验证证据、handoff 和就绪检查后决定完成或终止。
- 保留 Coder/Reviewer 发现 Plan Defect 后进入现有 Plan Repair 的能力；人工最终确认不新增 Work Item 自动返修或 Plan Repair 入口。
- 对已经持久化 Group Final Review 产物的历史 attempt 保持可读取和可恢复，不再为其启动新的 shard/reduction Provider 运行。
- 新 attempt 的 Group Final Readiness 依赖 `coder-owned-commits-and-provider-retries` 定义的 UnitRun `start_commit..completion_commit` 证据区间；这两个 change 必须按该依赖顺序实施与验证。

## Capabilities

### New Capabilities

无。

### Modified Capabilities

- `group-final-review-triage`: 组级最终审查从 Provider 评审与人工分诊改为客观就绪检查和人工最终确认。
- `coding-workspace-completion`: Group attempt 的完成前提从 internal PR review 通过改为组就绪检查通过和人工最终确认。

## Impact

- 后端：Group Coding runner、完成条件、Group Final 数据投影、门禁动作与历史状态恢复。
- 前端：移除自动组级 AI 审查进度和 retry 操作，展示就绪检查与人工最终确认所需的每项证据。
- 将删除或停止使用 shard/reduction Provider prompt、产物和重试路径；历史持久化数据不迁移、不删除。
- 单项 Coder、Code Reviewer、Coder rework 与 Plan Repair 的既有语义保持不变。
