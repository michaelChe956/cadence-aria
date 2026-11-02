# 规范增量：全开 Reviewer 的技术重试上下文

## MODIFIED Requirements

### Requirement: 自动重试使用适合角色的完整执行上下文


Coder 的自动重试 MUST 在同一 worktree 上以新的完整执行上下文启动，使其能够检查先前中断调用可能留下的改动；Work Item Code Reviewer 的自动重试 MUST 使用新的只读审查会话重新读取当前 worktree。任何 Provider 特有的恢复调用也 MUST 计入上述两次自动重试预算。

#### Scenario: Coder 中断后保留部分 worktree 改动

- **WHEN** Coder 在中断前已向共享 worktree 写入部分改动
- **THEN** 下一次自动重试 MUST 在同一 worktree 上以完整上下文启动，要求 Coder 先检查现有改动后再继续

#### Scenario: Provider 特有恢复不突破预算

- **WHEN** 某个 Provider 需要以新会话替代失效恢复会话
- **THEN** 该新会话调用 MUST 占用自动重试预算，且同一触发最多仍为首次调用加两次自动重试

#### Scenario: Reviewer 重试不能重用污染基线
- **WHEN** Reviewer 技术失败符合自动重试条件
- **THEN** 系统 MUST 先收口前轮窗口；无违规且恢复完整才可按原预算建新窗口、新基线和材料，任何违规事实保留并转人工，不沿用旧报告或把污染态作为下一轮基线
