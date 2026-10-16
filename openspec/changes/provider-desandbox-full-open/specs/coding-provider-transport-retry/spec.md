# 规范增量：全开 Reviewer 的技术重试上下文

## MODIFIED Requirements

### Requirement: 自动重试使用适合角色的完整执行上下文

Coder 自动重试 MUST 在同一 worktree 使用新的完整执行上下文检查中断遗留改动。Work Item Code Reviewer 自动重试 MUST 在前轮资源停止、恢复与采信收口后，以新的全能力 B 档审查窗口重新读取基线，不再使用只读沙箱；违规或恢复不完整须人工处理，不因技术重试预算尚余绕过。Provider 特有恢复调用仍计入原两次自动重试预算。

#### Scenario: Coder 中断后保留部分 worktree 改动
- **WHEN** Coder 中断前已向共享 worktree 写入部分改动
- **THEN** 下一次自动重试 MUST 使用同一 worktree 完整上下文，要求先检查现有改动再继续，不调用 Reviewer 整树恢复

#### Scenario: Provider 特有恢复不突破预算
- **WHEN** 某 Provider 以新会话替代失效恢复会话
- **THEN** 该调用 MUST 占用自动重试预算，同触发仍最多首次加两次重试

#### Scenario: Reviewer 重试不能重用污染基线
- **WHEN** Reviewer 技术失败符合自动重试条件
- **THEN** 系统 MUST 先收口前轮窗口；无违规且恢复完整才可按原预算建新窗口、新基线和材料，任何违规事实保留并转人工，不沿用旧报告或把污染态作为下一轮基线
