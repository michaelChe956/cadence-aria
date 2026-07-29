# work-item-runtime-projection Specification

## Purpose
TBD - created by archiving change fix-work-item-runtime-projection. Update Purpose after archive.
## Requirements
### Requirement: Schema v2 RuntimeBinding 只定位不可变 Revision
系统 SHALL 为每个 Schema v2 Work Item 子 Workspace 以及每个 Coding Attempt/Unit 的运行期读取保存或解析不可变 RuntimeBinding。该 Binding MUST 精确关联 Plan、PlanRevision、LogicalWorkItem、WorkItemRevision、WorkItemProjectionBundle 与 VerificationPlanRevision，并验证关联对象的 ID、版本和 Canonical Contract/Projection Hash 一致性。

RuntimeBinding SHALL 仅保存定位和完整性校验凭据；它 MUST NOT 保存可编辑的 Work Item 业务语义、可编辑 Projection 内容或独立执行状态。Canonical Contract SHALL 保持唯一业务权威，Human、Coder、Reviewer SHALL 保持唯一的三种派生 Projection。

#### Scenario: Final Compile 创建带 Binding 的子 Workspace
- **WHEN** Schema v2 Initial Final Compile 已发布一个逻辑 Work Item 的正式 Revision 与 ProjectionBundle
- **THEN** 系统 SHALL 在创建或复用该 Work Item 子 Workspace 时保存与该发布物一致的 RuntimeBinding
- **AND THEN** 系统 MUST NOT 创建旧 `LifecycleWorkItemRecord` 或旧 VerificationPlan 作为兼容数据

#### Scenario: Binding 与 Revision 关系不一致
- **WHEN** 运行期 Reader 发现 Binding 的 WorkItemRevision、ProjectionBundle、VerificationPlanRevision 或 Hash 不属于其绑定的 PlanRevision/LogicalWorkItem
- **THEN** 系统 SHALL 失败关闭并返回可诊断的 runtime binding 完整性错误
- **AND THEN** 系统 MUST NOT 回退读取旧 Work Item 数据或生成空的语义上下文

### Requirement: Work Item 运行期读取只使用 Revision Store
Schema v2 Work Item Workspace、Group Coding、Coding Provider Context、Evaluation、Tester、Completion Gate、Handoff 和生命周期派生视图 SHALL 通过 Revision Store Runtime Reader 读取 Work Item 语义。Reader MUST 根据 RuntimeBinding 与绑定的 Revision 解析角色所需数据。

Human Workspace SHALL 读取 Human Projection、Presentation Revision 与真实 Revision History；Coder SHALL 读取绑定的 Coder Projection；Reviewer SHALL 读取绑定的 Reviewer Projection。Tester、Evaluation、Gate 和 Handoff SHALL 读取 Canonical Contract、VerificationPlanRevision、DependencyGraphRevision 与 HandoffRevision 中的规范性数据，不得把 Human Projection 当作执行事实。

#### Scenario: 新 Group 在旧 Work Item 目录为空时初始化子 Workspace
- **WHEN** Schema v2 Group 的旧 `work-items/` 目录不存在且用户确认 Final Compile
- **THEN** 每个子 Workspace SHALL 使用其 RuntimeBinding 成功写入启动上下文并显示对应 Human Projection
- **AND THEN** 系统 SHALL 不返回 `work_item not found`

#### Scenario: Coder 与 Reviewer 使用同一 Contract 的不同 Projection
- **WHEN** 已绑定 Coding Unit 启动 Coder 或 Reviewer Provider
- **THEN** Coder SHALL 使用该 UnitRun 的 Coder Projection 和 Renderer 内容哈希
- **AND THEN** Reviewer SHALL 使用同一 WorkItemRevision 的 Reviewer Projection 和 Renderer 内容哈希
- **AND THEN** 系统 MUST NOT 按“当前最新 Plan”替换已绑定的 Projection

#### Scenario: Tester 与 Gate 消费规范性 Revision 数据
- **WHEN** 已绑定 Coding Unit 进入测试、评估或完成门禁
- **THEN** 系统 SHALL 从其绑定的 VerificationPlanRevision、Canonical Contract 和 HandoffRevision 解析验证、范围与依赖要求
- **AND THEN** 缺少或不一致的 Binding SHALL 使操作失败关闭

### Requirement: Group Coding 从 PlanRevision 物化运行事实
系统 SHALL 从已绑定 PlanRevision、DependencyGraphRevision、PlanProjectionBundle 和 Issue/Repository 元数据建立 Group Coding Attempt 的成员、顺序、依赖、仓库与初始 Unit。系统 MUST NOT 以前置 `LifecycleStore::list_work_items` 结果作为 Schema v2 Group 创建或完整性校验的条件。

#### Scenario: 新确认 Group 创建 Coding Attempt
- **WHEN** 用户对一个已确认的 Schema v2 Work Item Plan 创建 Group Coding Attempt
- **THEN** 系统 SHALL 在旧 Work Item 记录为空时根据 active PlanRevision 成功创建 Plan Binding 与 Coding Units
- **AND THEN** 每个 Unit SHALL 指向该 PlanRevision 指定的 WorkItemRevision

#### Scenario: Plan Repair 发布新 Revision
- **WHEN** Plan Amendment 通过既有确认流程发布新的 PlanRevision
- **THEN** 系统 SHALL 仅通过 Amendment 事务创建或更新后续 Unit 的 Binding
- **AND THEN** 既有 Workspace、completed UnitRun 和 HandoffRevision MUST 保持其原始绑定，不得自动漂移到新 Revision

### Requirement: Final Compile 成功包含子 Workspace 运行期就绪
系统 SHALL 仅在不可变 Revision 发布、所有子 Workspace RuntimeBinding 已确保、每个子 Workspace 的 Revision Store 启动上下文已成功持久化且恢复 Journal 已记录后，才对外报告 Initial Final Compile/Plan 确认成功。

#### Scenario: 所有子 Workspace 运行期就绪
- **WHEN** Initial Final Compile 的全部 Work Item Revision、Binding 和子 Workspace Context 都成功准备
- **THEN** 系统 SHALL 将 Compile Journal 标记为 committed 并返回已确认 Plan 及全部子 Workspace

#### Scenario: 子 Workspace 初始化阶段失败
- **WHEN** 任一子 Workspace 的 Binding 或 Context 初始化失败
- **THEN** 系统 SHALL 保留可恢复的 Journal 与已发布的不可变 Revision
- **AND THEN** 系统 MUST NOT 对客户端发送成功确认或将该 Compile 标记为 committed

#### Scenario: 恢复同一 Final Compile
- **WHEN** 系统从同一 Initial Final Compile 的部分完成 Journal 恢复
- **THEN** 系统 SHALL 仅复用身份一致的 Revision、Binding 和子 Workspace
- **AND THEN** 系统 MUST NOT 创建重复 Session、修改发布 Revision 或创建旧 Work Item 兼容记录

### Requirement: Schema Cutover 禁止 Legacy Work Item 兼容
系统 SHALL 保持 Schema v2 的一次性 Cutover：不得为新 Group 回填、迁移、双读、双写、fallback 或兼容旧 `LifecycleWorkItemRecord`、旧 VerificationPlan 和旧 Work Item 执行状态。

#### Scenario: 运行期 Reader 缺失 Schema v2 Binding
- **WHEN** v2 运行期入口收到没有有效 RuntimeBinding 的 Work Item、Workspace 或 Coding Attempt
- **THEN** 系统 SHALL 返回明确的 v2 runtime binding/schema 错误
- **AND THEN** 系统 MUST NOT 读取或创建旧业务数据来继续操作

#### Scenario: 读取历史业务数据
- **WHEN** `.aria` 业务数据不是支持的 Schema v2 或不具备 v2 Binding
- **THEN** 系统 SHALL 按 Schema Cutover 规则拒绝该数据
- **AND THEN** 系统 MUST NOT 自动迁移、回填、删除或修改该历史数据

### Requirement: 共享 Workspace 协议保持三类型一致
系统对 Workspace Session Binding、上下文创建或恢复的共享修改 SHALL 覆盖 Story、Design、Work Item 三种 Workspace Type。Story 与 Design MUST 继续使用各自产物数据路径，不得被要求具备 Work Item RuntimeBinding。

#### Scenario: Story 和 Design 共享链路不依赖 Work Item Binding
- **WHEN** 系统创建或恢复 Story 或 Design Workspace
- **THEN** 系统 SHALL 按各自现有实体路径完成上下文与恢复
- **AND THEN** 系统 MUST NOT 读取、创建或要求 Work Item RuntimeBinding

