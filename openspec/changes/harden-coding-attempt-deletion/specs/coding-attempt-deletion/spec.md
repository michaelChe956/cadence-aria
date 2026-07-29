## ADDED Requirements

### Requirement: 删除 coding attempt 必须按条件清理 shared-worktree

删除一个 coding attempt 后，系统 MUST 检查该 issue 是否还有其他 attempt 记录。无其他 attempt 时 MUST 删除 `issue-shared-worktree.json` 及其 `.lock`；有其他 attempt 时 MUST 保留（它们仍在使用 shared-worktree）。

#### Scenario: 无其他 attempt 时清理 shared-worktree

- **WHEN** 删除一个 attempt 后，该 issue 不存在其他 attempt 记录
- **THEN** 系统 MUST 删除 `issue-shared-worktree.json` 与 `.issue-shared-worktree.json.lock`，且删除视为成功（NotFound=OK）

#### Scenario: 有其他 attempt 时保留 shared-worktree

- **WHEN** 删除一个 attempt 后，该 issue 仍存在其他 attempt 记录
- **THEN** 系统 MUST NOT 删除 `issue-shared-worktree.json`，其他 attempt 对 shared-worktree 的使用 MUST 不受影响

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

### Requirement: 删除 coding attempt 不得误伤同 issue 其他数据

删除 attempt MUST 只影响该 attempt 自身及其产物，MUST NOT 删除同 issue 其他 attempt 的数据、plan、revisions 或其他 attempt 的 worktree/分支。

#### Scenario: 删除一个 attempt 不影响其他 attempt

- **WHEN** 同 issue 有多个 attempt，删除其中一个
- **THEN** 其他 attempt 的记录、shared-worktree（因仍有 attempt）、各自的 worktree/分支 MUST 不受影响
