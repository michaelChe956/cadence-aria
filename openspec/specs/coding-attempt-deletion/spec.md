# coding-attempt-deletion Specification

## Purpose
TBD - created by archiving change harden-coding-attempt-deletion. Update Purpose after archive.
## Requirements
### Requirement: 删除 coding attempt 必须按条件清理 shared-worktree（REQ-DEL-01）
系统 SHALL 使逻辑代码库场景下删除 coding attempt 时按 `(project, issue, repository)` 判定 shared-worktree 是否仍被同仓库其他 attempt 使用；仅当同仓无其他使用者时才清理，不得因异仓 attempt 存在而误删或误留。

#### Scenario: 按仓库范围判定 shared-worktree 清理
- **WHEN** 逻辑代码库场景下删除 coding attempt
- **THEN** 系统 SHALL 按 `(project, issue, repository)` 判定 shared-worktree 是否仍被同仓库其他 attempt 使用，仅同仓无使用者时清理

### Requirement: 删除 coding attempt 必须清理该 attempt 的残留 lock

删除 attempt 时 MUST 清理该 attempt 在 coding-attempts 下遗留的 lock：该 attempt 的 `.coding_attempt_<id>.json.lock`、（group scope 时）`.group-initialization-arbitration.lock`、以及 `work-item-attempt-locks/` 下该 attempt 各 work_item 对应的 lock。

#### Scenario: 清理该 attempt 的残留 lock

- **WHEN** 删除一个 attempt（group scope，含多个 work_item unit）
- **THEN** 该 attempt 的 `.coding_attempt_<id>.json.lock`、`.group-initialization-arbitration.lock`、以及其各 work_item 的 `work-item-attempt-locks/<wi>.lock` MUST 被删除

#### Scenario: 不误删其他 attempt 的 work_item lock

- **WHEN** 同 issue 存在另一个 attempt，其 work_item lock 在 `work-item-attempt-locks/` 中
- **THEN** 删除本 attempt 时 MUST NOT 删除其他 attempt work_item 的 lock（按 work_item 精确删，不整目录）

### Requirement: worktree 缺失不得阻断 coding attempt 删除

删除 attempt 时的 worktree 清理（移除 worktree、prune、删分支）MUST 容错：worktree 目录不存在时 MUST 跳过 git 回滚并视为成功，MUST NOT 因 worktree 已被删除而中断整个 attempt 删除。

#### Scenario: worktree 已删除时 attempt 仍能删除

- **WHEN** 删除一个 attempt，其 worktree 目录已被手动删除
- **THEN** 删除 MUST 成功完成，MUST NOT 因 worktree 缺失报错

### Requirement: 删除 coding attempt 不得误伤同 issue 其他数据（REQ-DEL-02）
系统 SHALL 使删除同一 Issue 下某一仓库的 attempt 时保留其他仓库的 shared-worktree、锁与数据，删除范围限定于该 target repository。

#### Scenario: 异仓 attempt 互不影响
- **WHEN** 删除同一 Issue 下某一仓库的 attempt
- **THEN** 系统 SHALL 保留其他仓库的 shared-worktree、锁与数据，删除范围限定于该 target repository

