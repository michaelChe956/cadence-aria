## 1. 运行期投影边界

- [ ] 1.1 定义并实现由已发布 Work Item revision 生成运行期 Work Item 记录的来源映射与幂等校验。
- [ ] 1.2 将初始 Final Compile 接入运行期投影，保证其在创建 Work Item 子 Workspace 前完成。

## 2. 确认门禁与失败语义

- [ ] 2.1 调整确认流程，使运行期投影、子 Workspace 创建和启动上下文初始化全部成功后才报告 Plan 已确认。
- [ ] 2.2 为来源不一致或上下文初始化失败提供可诊断的失败结果，避免错误的确认成功状态。

## 3. 回归验证

- [ ] 3.1 为新 Group 的 Initial Final Compile、重试幂等性和来源冲突添加后端回归测试。
- [ ] 3.2 为确认后的 Work Item 子 Workspace 启动上下文添加完整链路测试。
- [ ] 3.3 验证 Story 与 Design Workspace 的初始化路径保持不变，并确认历史已确认 Group 不被自动回填。
