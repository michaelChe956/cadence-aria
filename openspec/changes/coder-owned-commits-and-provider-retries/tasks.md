## 1. Coder-owned Work Item 提交

- [ ] 1.1 更新 Coder 运行期上下文与提示词，要求按 `write_policy` 精确暂存、提交并报告 Git 证据，同时禁止全量暂存和范围外清理。
- [ ] 1.2 将组 Work Item completion 改为只读取并持久化 Coder 已创建的 terminal HEAD，并以 UnitRun `start_commit..completion_commit` 作为既有范围、diff 和人工证据的事实来源，保留完成提交恢复的幂等性。
- [ ] 1.3 覆盖范围外未跟踪内容、策略允许的生成文件、首次提交加 rework 提交、空观察区间、Coder 已提交和旧完成记录恢复等回归场景。

## 2. Provider 技术失败重试

- [ ] 2.1 定义跨 Coder 与 Work Item Code Reviewer 共用的技术失败分类、两次自动重试预算与用户取消边界，明确 Provider 执行超时可重试、权限/选择等待超时不可重试。
- [ ] 2.2 重构流式调用失败分流，使自动重试耗尽前不创建人工门禁，并为每次调用持久化独立 role run、retry-cycle、事件和原始输出；移除或接管现有内嵌 fresh retry，避免绕过预算。
- [ ] 2.3 为 Coder 重试注入同一 worktree 的完整新上下文，为 Reviewer 重试注入新的只读审查上下文，并使 Provider 特有的新会话恢复计入共同预算。
- [ ] 2.4 在自动预算耗尽后接入既有人工 Coder/Reviewer 重试入口，并记录新的用户授权周期与上一个耗尽周期的关联，包括 Coder 的人工 retry。

## 3. UI、兼容性与验收

- [ ] 3.1 在 Coding Workspace 显示自动重试进度、调用序号、角色运行历史和耗尽后的人工处置状态。
- [ ] 3.2 覆盖 Coder/Reviewer 的成功重试、连续三次技术失败、内嵌 fresh recovery 计数、权限等待超时、取消、结构化输出无效、Coder rework 与 Plan Repair 边界。
- [ ] 3.3 运行定向与全量 Rust/前端验证，确认历史 completion commit 与既有人工门禁兼容。
