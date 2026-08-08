# coding-attempt-deletion Specification

## Purpose

（主规范沿用）为逻辑代码库场景扩展：shared worktree 与删除判定升级为 `(project, issue, repository)` 键，删除不误伤异仓产物。

## MODIFIED Requirements

### Requirement: 删除 coding attempt 必须按条件清理 shared-worktree
系统 SHALL 使逻辑代码库场景下删除 coding attempt 时按 `(project, issue, repository)` 判定 shared-worktree 是否仍被同仓库其他 attempt 使用；仅当同仓无其他使用者时才清理，不得因异仓 attempt 存在而误删或误留。

#### Scenario: 按仓库范围判定 shared-worktree 清理
- **WHEN** 逻辑代码库场景下删除 coding attempt
- **THEN** 系统 SHALL 按 `(project, issue, repository)` 判定 shared-worktree 是否仍被同仓库其他 attempt 使用，仅同仓无使用者时清理

### Requirement: 删除 coding attempt 不得误伤同 issue 其他数据
系统 SHALL 使删除同一 Issue 下某一仓库的 attempt 时保留其他仓库的 shared-worktree、锁与数据，删除范围限定于该 target repository。

#### Scenario: 异仓 attempt 互不影响
- **WHEN** 删除同一 Issue 下某一仓库的 attempt
- **THEN** 系统 SHALL 保留其他仓库的 shared-worktree、锁与数据，删除范围限定于该 target repository
