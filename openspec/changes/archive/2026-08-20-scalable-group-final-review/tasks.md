## 1. 组就绪检查与状态迁移

- [x] 1.1 定义并持久化 Group Final Readiness snapshot，汇总所有 UnitRun 的 `start_commit..completion_commit` 完整提交区间、有序提交/diff 证据、独立审查的结论/发现/原始输出、handoff 与计划绑定的一致性结果。
- [x] 1.2 将新 Group attempt 在最后一个 Work Item 通过独立审查后的流转改为就绪检查和 FinalConfirm，不再启动 Internal Group Reviewer、shard 或 reduction Provider。
- [x] 1.3 为不完整就绪快照提供精确诊断与现有恢复/终止语义，确保其不能被人工确认伪装为完成；在 FinalConfirm 保留既有 completion binding、提交区间范围和共享 worktree 清洁性检查，不将其转化为新的组级 AI 门禁。

## 2. 人工最终确认界面

- [x] 2.1 在 Group Final 面板展示按 Work Item 汇总的完整提交区间和提交列表、独立审查结论/发现/原始输出证据、handoff、计划 revision 与就绪诊断；空观察区间必须明确展示。
- [x] 2.2 移除新 attempt 的组级 AI 进度、shard/reduction retry 与 Provider 失败操作，接入明确的人工 Final Confirm 与终止操作。
- [x] 2.3 保持 Plan Repair、Coder rework 和独立 Code Review 的既有入口与状态展示不变。

## 3. 兼容性与验证

- [x] 3.1 保持历史 shard/reduction 产物可读取；将可恢复的历史 Group attempt 映射到人工最终确认，身份不一致时失败关闭。
- [x] 3.2 覆盖完整组、缺少 completion commit/审查/handoff/计划绑定、首次提交加 Coder rework 提交、空观察区间、范围外残留的既有人工清理语义、经 Plan Repair 与历史恢复的后端回归场景。
- [x] 3.3 覆盖前端完整审查内容与提交区间展示、确认禁用条件、明确确认完成、既有终态检查失败提示和历史数据展示，并执行相关全量验证。
