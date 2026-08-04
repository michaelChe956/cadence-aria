# work-item-runtime-projection Specification

## MODIFIED Requirements

### Requirement: Work Item 运行期读取只使用 Revision Store

Schema v2 Work Item Workspace、Group Coding、Coding Provider Context、Evaluation、Tester、Completion Gate、Handoff 和生命周期派生视图 SHALL 通过 Revision Store Runtime Reader 读取 Work Item 语义。Reader MUST 根据 RuntimeBinding 与绑定的 Revision 解析角色所需数据。

Human Workspace SHALL 读取 Human Projection、Presentation Revision 与真实 Revision History；Coder SHALL 读取绑定的 Coder Projection；单项 Coding Unit 的 Reviewer Provider SHALL 读取绑定的 Reviewer Projection。Tester、Evaluation、Gate 和 Handoff SHALL 读取 Canonical Contract、VerificationPlanRevision、DependencyGraphRevision 与 HandoffRevision 中的规范性数据，不得把 Human Projection 当作执行事实。

Group Final Review 的组级材料编译是本 requirement 的例外：它 SHALL 从权威 Binding 解析的规范性 Revision 数据编译组级材料，MUST NOT 要求为组级材料绑定 per-unit Reviewer Projection 执行上下文，其执行身份 SHALL 由组级材料快照的 schema 版本、compiler 版本与内容哈希承载。该例外 MUST NOT 改变单项 Coding Unit 的 Reviewer Provider 绑定要求。

#### Scenario: 新 Group 在旧 Work Item 目录为空时初始化子 Workspace
- **WHEN** Schema v2 Group 的旧 `work-items/` 目录不存在且用户确认 Final Compile
- **THEN** 每个子 Workspace SHALL 使用其 RuntimeBinding 成功写入启动上下文并显示对应 Human Projection
- **AND THEN** 系统 SHALL 不返回 `work_item not found`

#### Scenario: Coder 与单项 Reviewer 使用同一 Contract 的不同 Projection
- **WHEN** 已绑定 Coding Unit 启动 Coder Provider 或单项 Coding Unit 的 Reviewer Provider
- **THEN** Coder SHALL 使用该 UnitRun 的 Coder Projection 和 Renderer 内容哈希
- **AND THEN** 单项 Reviewer SHALL 使用同一 WorkItemRevision 的 Reviewer Projection 和 Renderer 内容哈希
- **AND THEN** 系统 MUST NOT 按“当前最新 Plan”替换已绑定的 Projection

#### Scenario: Group Final Review 使用组级材料快照身份
- **WHEN** Group Final Review 编译组级材料并启动分片或归约 Provider
- **THEN** 系统 SHALL 从权威 Binding 解析的规范性 Revision 数据编译材料
- **AND THEN** 系统 MUST NOT 为组级材料绑定或补写 per-unit Reviewer 执行上下文哈希
- **AND THEN** 该次组级审查的执行身份 SHALL 由组级材料快照的 schema 版本、compiler 版本与内容哈希标识

#### Scenario: Tester 与 Gate 消费规范性 Revision 数据
- **WHEN** 已绑定 Coding Unit 进入测试、评估或完成门禁
- **THEN** 系统 SHALL 从其绑定的 VerificationPlanRevision、Canonical Contract 和 HandoffRevision 解析验证、范围与依赖要求
- **AND THEN** 缺少或不一致的 Binding SHALL 使操作失败关闭
