# Spec Delta

## MODIFIED Requirements

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
