# per-repo-coding-execution Delta

## Purpose

Coding 在目标仓独立 worktree 执行（主 checkout 不动），shared worktree 键升级为 `(project, issue, repository)`，attempt 冻结 target 快照；mixed-target WorkItemGroup 按 target 分流为独立 target-attempt（每 `(plan, target)` 至多一个，单 target 场景零变化，分流契约以 `multi-target-group-coding` 为唯一来源）；交付为推分支 + 人工评审（ReviewRequest），自动 PR 后置。

## RENAMED Requirements

- FROM: `### Requirement: mixed-target group 一律拒绝（REQ-COD-04）`
- TO: `### Requirement: mixed-target group 按 target 分流（REQ-COD-04）`

## MODIFIED Requirements

### Requirement: mixed-target group 按 target 分流（REQ-COD-04）

系统 SHALL 使多成员 Issue 下创建 mixed-target WorkItemGroup 时按 target 分流：为每个目标仓库建立独立的 target-attempt（每 `(plan, target)` 至多一个），MUST NOT 以单一 attempt 承载多 target；「group 内存在多 target」本身 SHALL NOT 构成拒绝理由。分流 SHALL 在创建、恢复、replay 三处及全部路由消费面一致适用。无唯一 target 归属（units 无 `target_repository_id` 且 issue codebase selection focus 不唯一）SHALL 保持 fail-closed 拒绝并返回稳定错误码（TargetMissing 语义）；「group 内多 target」不再返回多目标歧义拒绝（TargetAmbiguous 在 group attempt 路由面退役，selection focus 面等其他消费点语义不变）。同 target 的 group 行为 SHALL 与分流引入前完全一致。分流的完整契约（per-`(plan, target)` 唯一性、单 target 零变化、per-target 冻结快照、手动推进、聚合视图、增殖审计）以 `multi-target-group-coding` capability 为唯一来源，本 requirement 只锁 per-repo-coding-execution 侧的分流义务与拒绝语义变更。

#### Scenario: 尝试 mixed-target group

- **WHEN** 用户对涉及多个目标仓库的 Work Item 建立同一 group
- **THEN** 系统为每个目标仓库分别建立独立 target-attempt（各自冻结该 target 快照、承载该 target 的 units），不再返回多目标歧义拒绝

#### Scenario: 无唯一 target 归属仍 fail-closed

- **WHEN** group 的 units 均无 `target_repository_id` 且 issue codebase selection focus 不唯一
- **THEN** 创建被拒绝并返回稳定错误码，不静默选择任一 target、不创建任何 attempt

#### Scenario: 同 target 的 group 行为零变化

- **WHEN** group 全部 units 属同一目标仓库
- **THEN** 建组、路由与执行行为与分流引入前完全一致：单 attempt、单快照、既有收敛路径不变
