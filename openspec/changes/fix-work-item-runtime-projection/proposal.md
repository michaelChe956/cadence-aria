## Why

Work Item Plan 的 Schema v2 Final Compile 已将 Canonical Contract、Revision 与三种 Projection 正确发布到 Revision Store，但子 Workspace、Group Coding 和其后的测试/门禁仍读取已被 v2 废弃的 `LifecycleWorkItemRecord`。新 Group 因而在正式内容已落盘后失败，或在后续运行期得到空上下文。

现在必须完成运行期代码读取端的迁移，使 Schema v2 的 Revision Store 从 Final Compile 一直到 Coding、Testing、Review 与 Handoff 都能形成闭环；旧业务数据不在支持范围内。

## What Changes

- 建立只含 Revision 标识与完整性凭据的 `RuntimeBinding`，使 Work Item 子 Workspace 和 Coding Attempt 精确绑定 PlanRevision、WorkItemRevision、ProjectionBundle 与 VerificationPlanRevision。
- 以 Revision Store Runtime Reader 替换 v2 路径中对 `LifecycleWorkItemRecord`、旧 VerificationPlan 和旧 Work Item 执行状态的读取；**BREAKING**：v2 运行期不再兼容或回退旧 `work-items/` 数据。
- 使 Final Compile、子 Workspace 初始化、Group Coding、Provider 上下文、Tester/Evaluation、Completion Gate、Handoff 与生命周期展示从同一 Binding 解析各自需要的权威或派生数据。
- 将三投影不变量扩展到运行期：Human 只用于展示，Coder 与 Reviewer 分别由同一 Canonical Contract 的已绑定快照渲染；任何 Binding/哈希不一致必须失败关闭。
- 扩展端到端测试，以“旧 Work Item 目录不存在”为固定前提，验证从 Final Compile 到 Group Coding 的完整路径；不回填、不迁移、不修改历史 `.aria` 业务数据。

## Capabilities

### New Capabilities

- `work-item-runtime-projection`: Schema v2 Work Item 的运行期 Binding、Revision Store Reader 与三投影消费边界。

### Modified Capabilities

无。

## Impact

- 后端：Final Compile、Workspace Session、Workspace Context、Revision Store、Coding Attempt Store、Coding/Test/Review/Gate/Handoff、生命周期 API。
- 前端：生命周期 Work Item 派生视图、Work Item Workspace Projection/History 展示，以及 Coding Workspace 的已绑定运行数据消费。
- API 行为：v2 Work Item 不再由旧 Lifecycle Work Item API 作为事实源；历史旧 `.aria` 数据继续按 Schema Cutover 规则拒绝加载，不提供迁移或兼容路径。
