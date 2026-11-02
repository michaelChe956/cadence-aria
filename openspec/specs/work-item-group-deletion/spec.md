# work-item-group-deletion Specification

## Purpose
规范 work item group 的删除边界：存在对应 coding attempt 时拒绝删除并提示先删除 coding workspace（不自动删除或中止该 attempt），删除时按 `(project, issue, repository)` 键清理该 group 的 shared-worktree、锁、journal 与产物且不连带删除同 issue 异仓数据，不误伤 issue 本身、spec 版本历史与其他 plan，失败时返回稳定错误码并标注涉及的 target repository 集合。

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

系统 SHALL 在 legacy 与 schema-v2 Plan/group 永久删除时物理清除该 Plan 拥有的主记录、全部会话及附属内容、source/IR/机械与结构报告/provenance、draft/compile/outline、revision/publication、真实归属生命周期 Work Item、关联 review/run、advance/初始化journal/retry及专属checkpoint与验证配置。共享工作树/锁按真实Plan/WI/target和`(project,issue,repository)`归属清理，不误伤其他Plan/异仓；共享review/run有存活外部引用时 MUST 拒删，不能删他人依赖或留下目标独占内容。Confirmed/曾发布不豁免，只留无内容事实。

#### Scenario: 按仓库键清理 group 产物
- **WHEN** 逻辑代码库场景下删除 work item group
- **THEN** 系统 SHALL 按 `(project, issue, repository)` 键清理该 group 产物；同 Issue 异仓产物不得被连带删除

#### Scenario: 两条 Plan 删除路径均清 durable source 闭包
- **WHEN** 无 coding attempt 的 legacy 或 schema-v2 Plan 通过全部门禁后被永久删除
- **THEN** 全部历史 source、IR、报告和 provenance 以及 compile/draft/outline/revision/publication MUST 不再存在，当前与旧 refs 均不得继续读到被删内容

#### Scenario: schema-v2 Work Item 按真实归属清除
- **WHEN** schema-v2 Plan 的编译结果还存在于 issue-wide 生命周期 Work Item 存储，或 Plan 摘要缺少旧所属项
- **THEN** 系统 MUST 清除真实绑定该 Plan/work_item_set 的记录及会话，另一 Plan 的同名逻辑 Work Item MUST 原样保留

#### Scenario: review 是 Plan 的组成部分
- **WHEN** Plan 的版本、会话或明确 run refs 关联多轮 review/provider-run 档案
- **THEN** 所有关联内容 MUST 随 Plan 清除，不独立保留 review，不整删 issue-wide review/run 目录

#### Scenario: 曾确认或曾发布内容同样物理删除
- **WHEN** Confirmed 或曾发布后回到 Draft 的 Plan 完成一次删除确认且无拒删阻挡
- **THEN** 系统 MUST 删除其确认版本、发布记录与全部审计内容，只保留一条无业务内容删除记录；上游 Story/Design 和其他 Plan 不受影响

#### Scenario: 共享工作树仍被其他 Plan 使用
- **WHEN** 目标 Plan 的仓库键工作树或锁资源仍属于同 Issue 另一存活 Plan
- **THEN** 系统 MUST 仅清目标 Plan 专属引用/资源，保留仍被其他 Plan 使用的共享资源，不整目录删除

#### Scenario: Plan 所属 advance 整条内容随删除清除
- **WHEN** 无对应coding attempt的Plan满足永久删除门禁，仍有advance record、初始化journal、独立retry或advance专属session checkpoint
- **THEN** 系统 MUST 按真实plan_id/advance_id/session归属清除全部对象，不删除其他Plan的advance及retry，不因根摘要缺失而漏清

#### Scenario: 删除终止 advance 重放与 Ready 放行
- **WHEN** 已永久删除的Plan身份再次收到同command_id、不同command_id的advance或retry-initialization，或者被旧advance Ready记录查询
- **THEN** 系统 MUST 优先依据删除事实拒绝该身份重放与初始化，Ready准入结果 MUST 为false，不返回陈旧workspace入口、不创建attempt/产物；未删除Plan的原幂等与Ready合同不变

#### Scenario: 共享业务审计内容被外部实体引用则拒删
- **WHEN** 待删Plan关联的review或provider-run内容仍被闭包外的存活实体引用
- **THEN** 系统 MUST 在任何清理前以`entity_referenced`拒删，列出实际引用者并提示先处理后置；完整Plan闭包保持不动。这不改变共享基础设施工作树仍被其他Plan使用时保留该资源的规则

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
