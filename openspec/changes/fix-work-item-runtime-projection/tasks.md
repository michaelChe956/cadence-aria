## 1. RuntimeBinding 与 Revision Store Reader

- [x] 1.1 定义 RuntimeBinding 的持久化、身份校验和 Schema v2 失败关闭语义，确保它不复制 Canonical Contract 或 Projection 业务内容。
- [x] 1.2 实现角色化 Revision Store Runtime Reader，分别提供 Workspace、Coder、Reviewer、Tester/Evaluation、Gate/Handoff 所需的绑定数据。
- [x] 1.3 删除 v2 可达路径对旧 Lifecycle Work Item、旧 VerificationPlan 和旧执行状态的 fallback 依赖。

## 2. Final Compile 与 Work Item Workspace

- [x] 2.1 将 Initial Final Compile Journal 扩展为发布 Revision 后确保子 Workspace Binding 与 Revision 驱动启动上下文的可恢复阶段。
- [x] 2.2 迁移 Work Item Workspace Context、Repository 解析、Projection/History 展示与恢复，使其只使用 RuntimeBinding 和 Revision Store。
- [x] 2.3 保持 Story、Design Workspace 的既有实体路径，并为共享 Session 修改补充三类型保护。

## 3. Group Coding 与执行期消费者

- [x] 3.1 从 PlanRevision、DependencyGraphRevision、PlanProjectionBundle 和 Issue/Repository 元数据创建及校验 Group Coding Attempt/Units。
- [x] 3.2 迁移 Coding WebSocket Context、Provider Execution Context、Evaluation、Tester、Completion Gate、终止/删除路径和生命周期派生视图。
- [x] 3.3 保证 Coder/Reviewer/Tester/Gate/Handoff 分别使用正确的绑定 Projection 或 Canonical/Verification/Handoff Revision，并保持 Plan Repair 的历史 Binding 不变。

## 4. 端到端验证与旧路径清除

- [x] 4.1 添加“无旧 Work Item 目录”的 Final Compile → Confirm → 子 Workspace Context → Group Coding Attempt 端到端回归测试。
- [x] 4.2 添加 Coder/Reviewer Projection、Tester/Gate/Handoff Revision Binding、Final Compile Journal 恢复和 Binding 不一致失败关闭测试。
- [x] 4.3 对 Story、Design、Work Item 三类 Workspace 的共享上下文/恢复协议添加回归测试，并验证 Schema v2 不发生 Legacy Work Item 读写。
