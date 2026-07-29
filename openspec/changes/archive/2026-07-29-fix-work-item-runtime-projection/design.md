## Context

原三投影设计规定：Canonical Contract 是唯一权威；Human、Coder、Reviewer Projection 是由同一 Contract 编译出的不可变派生快照；Schema v2 不实现旧 Artifact Reader、迁移、双读、双写或兼容 DTO。

当前 Final Compile 已符合写入侧：它发布 PlanRevision、WorkItemRevision、VerificationPlanRevision 和 ProjectionBundle，也刻意不创建旧 `LifecycleWorkItemRecord`。但运行期读取侧未同步切换：子 Workspace Context、Group Coding 创建、Coding Context、Tester、Evaluation、Gate、部分生命周期 API 仍从旧 `work-items/` 目录读取。这个目录在合法 Schema v2 Group 中为空，造成写入成功、读取失败的断链。

本 Change 仅迁移代码的运行期 Reader，不迁移任何历史业务数据。它不增加第四种 Projection，也不改变 Work Item 的拆分策略、Canonical Contract Schema 或 Provider 输出契约。

## Goals / Non-Goals

**Goals:**

- 使每个新 v2 Work Item 子 Workspace 和 Coding Unit 都有不可变、可校验的 Revision Binding。
- 让运行期所有 Work Item 语义读取来自 Revision Store；Human、Coder、Reviewer 分别消费正确的已绑定 Projection。
- 让 Final Compile 的成功边界包含子 Workspace 的 Binding 与可初始化上下文，避免“已确认后再报 not found”。
- 保持 Plan Repair、CodingAttemptPlanBinding、UnitRun 与 HandoffRevision 的已有不可变版本语义，并使其可从初始 v2 Group 实际进入。
- 以没有旧 Work Item 记录的端到端测试证明全链路。

**Non-Goals:**

- 不迁移、回填、读取、转换或删除历史 `.aria` Work Item、Coding Attempt 或 Workspace Session 业务数据。
- 不创建或双写 `LifecycleWorkItemRecord`、旧 VerificationPlan 或旧执行状态作为 v2 兼容投影。
- 不将 RuntimeBinding 作为第四种 Projection、可编辑缓存或业务事实源。
- 不修改 Canonical Contract 的业务语义、三投影编译算法、Provider 输出协议或 Work Item 前后端拆分策略。

## Decisions

### 1. RuntimeBinding 只保存不可变引用，不复制业务语义

每个 v2 Work Item 子 Workspace 保存 `plan_id`、`plan_revision_id`、`logical_work_item_id`、`work_item_revision_id`、`projection_bundle_id` 和 `verification_plan_revision_id`，以及用于校验关联关系的稳定哈希/版本凭据。

Binding 只能定位并验证 Revision Store 中的对象。它不得存储可编辑的目标、范围、依赖、验证规则或 Projection 内容；Canonical Contract 仍是唯一业务权威。选择显式 Binding 而非运行时按“当前 active revision”查询，是为了在 Plan Repair 后仍可复现已有 Workspace/UnitRun 实际使用的版本。

备选方案：在运行期按当前 active PlanRevision 延迟解析，因修订会改变已执行会话的语义而拒绝；补写旧 Work Item 记录，因违反 Schema Cutover 的无双写边界而拒绝。

### 2. 统一 Revision Store Runtime Reader 负责解析与失败关闭

建立一个限定在 v2 路径使用的 Reader：它先验证 Binding 与 PlanRevision 的 Work Item Binding、WorkItemRevision、ProjectionBundle、VerificationPlanRevision、Canonical Contract Hash 的双向关系，再提供角色化视图。

- Workspace/UI 取得 Human Projection、HumanPresentationRevision 和 Revision History。
- Coder 取得 Coder Projection；Reviewer 取得 Reviewer Projection；两者必须保留已绑定 Bundle 与 Renderer Content Hash。
- Tester、Evaluation、Gate 与 Handoff 从 Canonical Contract、VerificationPlanRevision、DependencyGraphRevision 和 HandoffRevision 读取规范性数据，不能以 Human Projection 作为执行依据。

任何缺失、逻辑 ID 混淆、Revision/Bundle/Hash 不一致都必须失败关闭，并报告所缺 Binding，而不是访问旧 Lifecycle Store 或静默降级为空上下文。

### 3. Final Compile 在成功对外可见前确保子 Workspace 可运行

Final Compile 的恢复 Journal 负责依次发布不可变 Revision、确保每个带 Binding 的子 Workspace、生成其 Revision Store 驱动的启动上下文、持久化可恢复游标，然后才报告 Compile/Plan 确认成功。失败时保留最后成功的不可变发布物和 Journal，但不得发送“确认成功”；重试只能复用同一编译与同一 Binding。

这不是把子 Workspace 内容写回 Lifecycle，而是在 Session 中持久化指向 Revision Store 的引用。Story 与 Design 继续使用自身既有实体路径；共享 Session 元数据变更必须验证三种 Workspace Type 的恢复行为。

### 4. Group Coding 从 PlanRevision 而非旧 Work Item 列表物化

创建 Group Coding Attempt 时，顺序、成员、依赖、当前 Unit、Repository 与验证绑定由 PlanRevision、DependencyGraphRevision、PlanProjectionBundle 和 Issue/Repository 元数据解析。CodingAttemptPlanBinding 与 CodingExecutionUnit 记录已绑定 Revision；UnitRun 继续记录已解析 HandoffRevision 和 Projection/Renderer Hash。

这保留现有 P5 的静态 Projection/Handoff 校验逻辑，但删除其创建和验证入口对旧 Work Item 列表的前置依赖。生命周期 UI/API 输出为 Revision Store 的派生读模型，不再要求旧 Lifecycle Work Item 事实存在。

### 5. 旧 Reader 在 v2 Work Item 路径中移除，而非增加 fallback

本 Change 的代码迁移范围包含 Workspace Context、Repository 解析、Group Coding、Coding WS Context、Evaluation、Tester、Completion Gate、运行期状态展示与删除/终止路径。凡 v2 Work Item 路径仍需要旧记录的 API，要么改为上述 Reader，要么明确从 v2 UI/路由下线；不得保留“先读 Revision、失败再读旧记录”或反向 fallback。

## Risks / Trade-offs

- [Reader 范围遗漏导致下一阶段才失败] → 对所有 `list_work_items` 运行期调用做可达性清单，并用新 Group 端到端测试穿透 Workspace、Coding、Tester、Gate 和 Handoff。
- [Binding 在 Repair 后指向错误 Revision] → Binding 不随 active revision 自动漂移；Plan Amendment 只能通过已有的 Plan Binding/UnitRun 事务创建新的 Binding。
- [Final Compile 中途失败形成部分对象] → 只发布不可变 Revision，Journal 记录子 Session/Context 阶段；重试验证身份并幂等继续，未完成前不发送成功确认。
- [共享 Session 元数据影响 Story/Design] → 为 Story、Design、Work Item 三种 Workspace Type 添加表驱动恢复与上下文回归测试。
- [旧端点仍被外部调用] → v2 输入遇到旧 Reader 路径时返回明确的 schema/runtime binding 错误，不尝试兼容旧业务数据。

## Migration Plan

这是代码读取端迁移，不是数据迁移。

1. 在 Schema v2 的新 Group 上发布 Binding 与 Revision Store Reader。
2. 以无旧 `work-items/` 目录的全新 `.aria` Workspace 验证完整流程。
3. 旧 Schema 或缺少 v2 Binding 的业务数据继续返回既有的 Schema Cutover/Binding 错误；不运行回填脚本、不提供自动迁移、不进行双写。
4. 若运行期 Reader 发现完整性不一致，停止该操作并保留诊断信息；回滚仅回滚代码，不修改已发布的不可变 Revision。

## Open Questions

无。用户已确认完整 Reader 范围、无历史数据迁移，以及 Canonical Contract + 三投影的权威边界。
