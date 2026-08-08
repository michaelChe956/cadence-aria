# work-item-group-deletion Specification

## Purpose

（主规范沿用）为逻辑代码库场景扩展：mixed-target group 已被一律拒绝，删除与清理按 `(project, issue, repository)` 键执行，不误伤异仓产物。

## MODIFIED Requirements

### Requirement: work item group 删除必须清理全部产物且无残留（REQ-GRP-01）
系统 SHALL 使逻辑代码库场景下删除 work item group 时按 `(project, issue, repository)` 键清理 shared-worktree、锁、journal 与产物；同 Issue 异仓产物不得被连带删除。

#### Scenario: 按仓库键清理 group 产物
- **WHEN** 逻辑代码库场景下删除 work item group
- **THEN** 系统 SHALL 按 `(project, issue, repository)` 键清理该 group 产物；同 Issue 异仓产物不得被连带删除

### Requirement: 删除失败必须给出可定位的错误细节（REQ-GRP-02）
系统 SHALL 使逻辑代码库场景下遇到 mixed-target group（本 change 已一律拒绝创建）时返回稳定错误码并标注涉及的 target repository 集合。

#### Scenario: mixed-target group 错误码
- **WHEN** 逻辑代码库场景下遇到 mixed-target group
- **THEN** 删除/查询 SHALL 返回稳定错误码，标注涉及的 target repository 集合
