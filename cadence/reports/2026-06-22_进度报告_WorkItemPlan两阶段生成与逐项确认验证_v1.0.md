# WorkItemPlan 两阶段生成与逐项确认验证

## 文档信息

- 文档类型：进度报告
- 日期：2026-06-23
- 版本：v1.0
- worktree：`.worktrees/feat-b-0616`
- 分支：`feat-b-0616`
- 记录时间：2026-06-23 14:56 CST
- 对应方案：`cadence/designs/2026-06-22_技术方案_WorkItemPlan两阶段生成与逐项WorkItem确认流程_v1.5.0.md`
- 对应计划：`cadence/plans/2026-06-22_计划文档_实施计划_WorkItemPlan两阶段生成与逐项确认_WP8_贯通验收与回归_v1.0.md`

## 验证结论

WorkItemPlan v1.5.0 两阶段生成与逐项确认流程已完成贯通验证。后端覆盖 Outline 生成、模式选择、Serial/Batch Draft、逐项/整组 Review、plan reopen、FinalCompile 事务恢复、SessionState 恢复与三类既有 Workspace 回归。前端覆盖 staged artifact 展示、timeline draft 切换、serial draft confirm、batch queue/review findings 与 compile recovery action。

## 后端定向测试结果

- `cargo test --locked --test it_web work_item_plan_batch`：11 passed。
- `cargo test --locked --test it_web work_item_plan_compile`：7 passed。
- `cargo test --locked --test it_web work_item_plan_staged_flow`：3 passed。
- `cargo test --locked --test it_web plan_reopen_required_supersedes_drafts_and_reopens_outline`：1 passed。
- `cargo test --locked --test it_web web_workspace_recovery_consistency`：5 passed，1 ignored legacy。

覆盖重点：

- Serial flow：Outline accept 后选择 serial，逐项 draft accept，FinalCompile 后才写入真实 work items、verification plans 和 child sessions。
- Batch flow：按 outline 拓扑序生成 draft，local validation 失败自动重试一次，二次失败记录 `validation_failed_ids`，rewrite batch 后可重新生成并 compile。
- Plan reopen：item review 返回 `plan_reopen_required` 后重新打开 outline，目标及下游 draft 置为 superseded，历史 draft 仍可读取。
- Compile recovery：`not_started` 支持 rollback，`committed` 后只允许 continue/human_triage，continue 不重复创建实体。
- SessionState：恢复 stage、active node、current outline、draft records、batch queue、active outline id、compile transaction/report 与 artifact history MVP index。

## 前端测试结果

- `pnpm -C web exec vitest --run src/state/workspace-ws-store.test.ts src/hooks/useWorkspaceWs.test.tsx src/pages/ChatWorkspacePage.test.tsx`：141 passed。
- `pnpm -C web test`：40 test files passed，434 tests passed。

覆盖重点：

- `ChatWorkspacePage` 可展示 outline、mode select、serial draft confirm。
- Batch queue 展示 draft 状态、validation failure 与 review findings。
- Compile recovery action 按 commit state 展示。
- `workspace-ws-store` 可从 timeline/artifact history 切换 selected draft。

## 标准验证命令结果

- `cargo fmt --check`：passed。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：passed。
- `cargo check --locked`：passed。
- `cargo test --locked`：194 passed，0 failed，12 ignored；doc-tests 0 passed，0 failed。
- `pnpm -C web test`：40 passed，434 tests passed。
- `pnpm -C web build`：passed；Vite 仍提示单个 chunk 超过 500 kB，为既有 bundle size warning，不影响构建退出码。

## Workspace 回归影响

Story Workspace：

- 受影响点：复用 ChatWorkspace review sentinel 与 artifact payload 管线。
- 回归覆盖：`story_workspace_review_sentinel_fallback_still_passes`。
- 结论：review sentinel fallback 仍能恢复并展示 review decision，不受 WorkItemPlan staged payload 增量字段影响。

Design Workspace：

- 受影响点：复用 artifact history 与 Markdown artifact 展示。
- 回归覆盖：`design_workspace_artifact_history_still_loads_markdown`。
- 结论：Design artifact history 仍能加载 Markdown 当前快照，未被 WorkItemPlan staged artifact 分支破坏。

普通 WorkItem Workspace：

- 受影响点：复用 review decision DTO、human confirm 与 workspace session 恢复路径。
- 回归覆盖：`ordinary_work_item_workspace_review_unaffected`。
- 结论：普通 WorkItem 仍可通过 review 生成/确认流程，并产出 child WorkItem sessions。

Coding Workspace：

- Coding Workspace 使用独立 coding workspace WS 类型与 store，不复用本次 WorkItemPlan staged payload 字段；本轮通过前端既有 `CodingWorkspacePage` Vitest 与 Rust 全量测试间接确认未出现编译或测试回归。

## 未覆盖风险与后续建议

- 本轮按 WP8 明确范围未做 Playwright 浏览器 E2E；当前交互风险由后端 WS 集成测试和 Vitest 覆盖，后续若增加拖拽、复杂 timeline 定位或真实浏览器布局断言，应补浏览器级测试。
- `SessionState.artifact_versions` 当前保持 MVP index/summaries 契约，不存 full payload history；如后续需要跨版本直接恢复完整 staged payload，需要新增后端 payload 存储与兼容测试。
- 仍有 legacy full-candidate WorkItemPlan 测试处于 ignored 状态；这些旧流程已由两阶段 staged flow 替代，后续可在确认无回滚需求后清理或迁移命名，减少测试噪音。
- 前端生产构建存在 Vite chunk size warning；本次未做代码拆分，后续可单独规划 bundle split。

## 结论

WP8 贯通验收与回归目标已按计划完成，验证结果支持将 `feat-b-0616` 推送到远端作为 WorkItemPlan 两阶段生成与逐项确认流程的收口分支。
