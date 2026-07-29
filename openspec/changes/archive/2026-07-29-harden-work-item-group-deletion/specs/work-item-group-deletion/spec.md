## ADDED Requirements

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

### Requirement: work item group 删除必须清理全部产物且无残留

通过门禁后，系统 MUST 删除该 group 的全部产物。每个删除步骤 MUST 把产物不存在视为成功，MUST NOT 因某项产物缺失、worktree 目录缺失或 revision 半残而中断。

需要清理的产物为：plan 记录、WorkItemPlan 类型 session、该 group 各 work item 的 WorkItem 类型 session 及其 timeline、plan store 的 drafts/compiles/outlines、revisions 整目录、revision-publications 整目录、issue shared worktree 记录、coding attempt 残留 lock。

#### Scenario: 完整 group 删除后无残留

- **WHEN** 一个无 coding attempt 的完整 work item group 被删除（全部产物均已存在）
- **THEN** 删除完成后，该 plan 的 revisions 目录、revision-publications 目录、plan store drafts/compiles/outlines 目录、issue shared worktree 文件、各 WorkItem session 与 timeline MUST 全部不存在

#### Scenario: 半残 group 仍能删除且无残留

- **WHEN** 一个 work item group 处于半残状态——部分 WorkItem session 已缺失、worktree 目录已被删除、coding attempt json 已删除但残留 lock 文件——用户请求删除
- **THEN** 系统 MUST 成功完成删除，MUST NOT 因缺失的 session 或 worktree 报错；删除完成后该 plan 的全部产物 MUST 不存在

#### Scenario: 缺失 worktree 不阻断删除

- **WHEN** 删除一个 worktree 目录已不存在的 group
- **THEN** 系统 MUST NOT 因 worktree 解析失败而拒绝或中断删除

#### Scenario: WorkItem session 清理不依赖 revision 完整

- **WHEN** group 的 plan revision 或 lineage 处于半残，`work_item_bindings` 不可靠
- **THEN** 系统 MUST 仍能通过扫描 WorkItem session 自身的 `plan_id` 字段定位并删除该 group 的全部 WorkItem session，MUST NOT 因 bindings 数量不匹配而拒绝

### Requirement: work item group 删除不得误伤其他数据

删除 MUST 只影响该 group 的产物，MUST NOT 删除 issue 本身、issue 的 story/design spec、spec 版本历史、仓库注册，也不得影响同 issue 下其他 plan 或其他 issue 的任何数据。

#### Scenario: 删除保留 issue 与 spec

- **WHEN** 一个 work item group 被成功删除
- **THEN** issue 记录、story spec、design spec、spec 版本历史、仓库初始化记录 MUST 全部仍然存在

#### Scenario: 删除不影响其他 plan

- **WHEN** 一个 issue 下存在多个 work item plan，删除其中一个
- **THEN** 其他 plan 的全部产物 MUST 不受影响

### Requirement: 删除失败必须给出可定位的错误细节

当删除路径产生 `ProductStoreError` 且未被精确映射时，系统 MUST 在错误响应的 details 中带出错误的 `kind` 与 `id`（或等价定位信息），MUST NOT 返回空 details。

#### Scenario: 完整性校验类错误带 kind 与 id

- **WHEN** 删除路径产生 `IdentityMismatch` 类错误（如 runtime binding 校验）
- **THEN** 响应 details MUST 含该错误的 kind 与 id，使前端能定位是哪类校验、哪个对象
