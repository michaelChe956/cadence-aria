# work-item-group-deletion Specification

## Purpose
TBD - created by archiving change harden-work-item-group-deletion. Update Purpose after archive.
## Requirements
### Requirement: 存在 coding workspace 时拒绝删除 work item group

系统 MUST 在删除 work item group 前检查该 group 是否存在对应的 coding attempt。存在时 MUST 拒绝删除，MUST NOT 自动删除或中止该 coding attempt。拒绝响应 MUST 给出明确提示，要求用户先删除 coding workspace。

#### Scenario: 有 coding attempt 时拒绝删除

- **WHEN** 一个 work item group 存在对应的 coding attempt 记录，用户请求删除该 group
- **THEN** 系统 MUST 拒绝删除，响应 MUST 含 `coding_workspace_exists` 错误码与提示先删除 coding workspace 的信息，响应 details MUST 含 plan_id 与 attempt_id

#### Scenario: 拒绝时 group 与 attempt 均保持不动

- **WHEN** 因存在 coding attempt 而拒绝删除 group
- **THEN** 该 group 的 plan 记录、revisions、coding attempt 记录 MUST 全部仍然存在，MUST NOT 被部分删除

#### Scenario: coding attempt 不存在时放行

- **WHEN** 一个 work item group 没有对应的 coding attempt 记录（attempt json 不存在），即使存在残留的 attempt lock 文件
- **THEN** 系统 MUST 放行删除，MUST NOT 因残留 lock 而拒绝

### Requirement: work item group 删除必须清理全部产物且无残留（REQ-GRP-01）
系统 SHALL 使逻辑代码库场景下删除 work item group 时按 `(project, issue, repository)` 键清理 shared-worktree、锁、journal 与产物；同 Issue 异仓产物不得被连带删除。

#### Scenario: 按仓库键清理 group 产物
- **WHEN** 逻辑代码库场景下删除 work item group
- **THEN** 系统 SHALL 按 `(project, issue, repository)` 键清理该 group 产物；同 Issue 异仓产物不得被连带删除

### Requirement: work item group 删除不得误伤其他数据

删除 MUST 只影响该 group 的产物，MUST NOT 删除 issue 本身、issue 的 story/design spec、spec 版本历史、仓库注册，也不得影响同 issue 下其他 plan 或其他 issue 的任何数据。

#### Scenario: 删除保留 issue 与 spec

- **WHEN** 一个 work item group 被成功删除
- **THEN** issue 记录、story spec、design spec、spec 版本历史、仓库初始化记录 MUST 全部仍然存在

#### Scenario: 删除不影响其他 plan

- **WHEN** 一个 issue 下存在多个 work item plan，删除其中一个
- **THEN** 其他 plan 的全部产物 MUST 不受影响

### Requirement: 删除失败必须给出可定位的错误细节（REQ-GRP-02）
系统 SHALL 使逻辑代码库场景下遇到 mixed-target group（本 change 已一律拒绝创建）时返回稳定错误码并标注涉及的 target repository 集合。

#### Scenario: mixed-target group 错误码
- **WHEN** 逻辑代码库场景下遇到 mixed-target group
- **THEN** 删除/查询 SHALL 返回稳定错误码，标注涉及的 target repository 集合

