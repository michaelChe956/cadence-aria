# WorkItemPlan 两阶段生成与逐项确认拆分总览 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 `cadence/designs/2026-06-22_技术方案_WorkItemPlan两阶段生成与逐项WorkItem确认流程_v1.5.0.md` 拆为多个单 session 可完成、可独立验证、单文件小于 50k 的实施计划。

**Architecture:** 先落跨 Workspace reviewer sentinel 兼容与 WorkItemPlan Draft/Outline/Compile 事实来源，再把现有一次性 candidate 流程迁移为 Outline → Draft → Compile。后端按数据层、Outline 流、review/mode、串行 Draft、自动 Batch、Final Compile 严格串行推进；前端在协议稳定后接入 discriminated artifact union、node type 路由和 MVP artifact 历史。

**Tech Stack:** Rust 1.95.0、Cargo、Axum、tokio、serde、React、TypeScript、Zustand、Vitest、pnpm。

---

## 当前进度摸底

- worktree：`.worktrees/feat-b-0616`
- 分支：`feat-b-0616`
- 远端同步：`git pull --ff-only` 已是最新；`git push origin feat-b-0616` 显示 `Everything up-to-date`
- 工作区：干净
- 最近提交：`07afbaf doc:review design`
- 方案状态：v1.5.0 已修复 3P0 + 5P1 + 7P2，当前缺实施计划拆解

现有实现仍是 2026-06-17 一次性 WorkItemPlan candidate 路线：

| 区域 | 当前事实 | v1.5.0 需要变化 |
|---|---|---|
| `src/product/work_item_split_engine.rs` | prompt 一次性输出 `repository_profile`、完整 `work_items`、完整 `verification_plans`；author 已用无 nonce sentinel | 改为 Outline prompt、单 item prompt、batch 调度 prompt；新流程不要求 `repository_profile` |
| `src/product/workspace_engine.rs` | `complete_work_item_plan_author` 生成后立即 full validator，失败自动返修，成功进入整组 AuthorConfirm | 改为 Outline 轻量校验、Draft 局部校验、最终 Compile 前 strict validator；废弃黑盒自动返修主流程 |
| `src/product/lifecycle_store.rs` | `replace_issue_work_item_plan_candidate` 在 Draft 阶段直接写真实 `work_items/`、`verification_plans/`、`repository_profiles/` | Draft 阶段只写 immutable `WorkItemDraftRecord` 与 active index；真实实体只在 Final Compile transaction 中写入 |
| `src/web/workspace_ws_types.rs` | `ArtifactPayload` 只有 Markdown / `WorkItemPlanCandidate`；`TimelineNodeType` 无 WorkItemPlan 专属 node；`ReviewComplete` 只有通用 verdict | 新增 Outline/Draft/Batch/Compile payload；新增 WorkItemPlan 专属 timeline node type；`ReviewComplete` 加可选 `work_item_plan_review` |
| `web/src/state/workspace-ws-store.ts` | 只保留 `workItemPlanCandidate` 单字段 | 改为 WorkItemPlan artifact union + history index |
| `web/src/components/workspace/WorkItemPlanCandidatePanel.tsx` | 只展示整组 candidate、revert、确认计划 | 改为 Outline 确认、生成模式选择、逐项 Draft、Batch 队列、Compile Recovery、MVP 历史视图 |

## 拆分原则

- 单个计划文件必须小于 50k 字节；实现者读取该计划、相关代码、写测试和修改时不应触发上下文压缩。
- 后端共享文件必须严格串行，尤其是 `src/product/workspace_engine.rs`、`src/web/workspace_ws_handler.rs`、`src/web/workspace_ws_types.rs`、`src/product/lifecycle_store.rs`。
- 每个 WP 只改声明的文件；发现需要越界时先更新总览或新增 WP，不在当前 WP 临时扩范围。
- TDD：每个行为先写失败测试，再写最小实现，再跑定向测试。
- Rust 命令遵守 `cadence/project-rules/build-test-commands.md`：禁止 `-j 1`，定向单测优先 `cargo test --locked --lib <过滤名>`，全量验证用标准四命令。
- 前端只用 `pnpm`；不使用 `npm` / `yarn`。
- Workspace 产物链路涉及 Story / Design / Work Item / WorkItemPlan 共享逻辑时，必须按 `workspace-artifact-bug-triage.md` 做三类产物影响说明与回归测试。

## WP 列表

| WP | 文件 | 目标 | 依赖 | 是否可并行 |
|---|---|---|---|---|
| WP0 | `..._WP0_ReviewerSentinel与Review契约基础_v1.0.md` | reviewer sentinel nonce/fallback、`WorkItemPlanReviewComplete` DTO、引用校验基础 | 无 | 可先做，后续 review WP 依赖 |
| WP1 | `..._WP1_后端数据模型与DraftStore_v1.0.md` | Outline/Draft/Batch/Compile 数据模型与专用 store | 无 | 与 WP0 可并行，但推荐先 WP0 |
| WP2 | `..._WP2_Outline生成与上下文门禁_v1.0.md` | Design context gate、Outline author、轻量 validator、context blocker/index | WP0、WP1 | 否 |
| WP3 | `..._WP3_Outline确认Review与模式选择_v1.0.md` | Outline confirm/review、generation mode node、专用 WS 输入校验 | WP2 | 否 |
| WP4 | `..._WP4_串行Draft生成确认与逐项Review_v1.0.md` | serial 单 item Draft 生成、局部 validator、逐项确认/review、downstream invalidation | WP3 | 否 |
| WP5 | `..._WP5_自动Batch生成确认与整组Review_v1.0.md` | batch 串行自动生成、validation_failed 处理、整组确认/review、降级串行 | WP4 | 否 |
| WP6 | `..._WP6_FinalCompile事务与恢复_v1.0.md` | compile transaction、拓扑排序、strict validator、幂等提交、recovery node | WP4、WP5 | 否 |
| WP7 | `..._WP7_前端WorkItemPlan两阶段Workspace_v1.0.md` | artifact union、node type 路由、Outline/Draft/Batch/Compile UI、未知 node fallback | WP3 起可做骨架，完整依赖 WP6 |
| WP8 | `..._WP8_贯通验收与回归_v1.0.md` | 后端 WS + 前端 Vitest 贯通、三类 Workspace 回归、计划收口报告 | WP0-WP7 | 否 |

## 串行约束

以下文件会被多个 WP 修改，必须按 WP 顺序推进：

| 文件 | 涉及 WP |
|---|---|
| `src/product/workspace_engine.rs` | WP0、WP2、WP3、WP4、WP5、WP6 |
| `src/web/workspace_ws_handler.rs` | WP3、WP4、WP5、WP6 |
| `src/web/workspace_ws_types.rs` | WP0、WP2、WP3、WP4、WP5、WP6、WP7 |
| `src/product/models.rs` | WP0、WP1 |
| `src/product/lifecycle_store.rs` | WP1、WP6 |
| `src/product/work_item_split_engine.rs` | WP0、WP2、WP4、WP5 |
| `src/product/work_item_split_validator.rs` | WP1、WP2、WP4、WP6 |
| `web/src/state/workspace-ws-store.ts` | WP7 |
| `web/src/hooks/useWorkspaceWs.ts` | WP7 |
| `web/src/pages/ChatWorkspacePage.tsx` | WP7 |

## 推荐执行顺序

1. WP0：先把 reviewer 输出解析与协议扩展从 WorkItemPlan 主流程中解耦，保留 markdown fence fallback。
2. WP1：落数据模型和专用 store，确保 Draft 阶段可不写真实 work item。
3. WP2：实现 Outline author + context blocker，仍不进入 Draft。
4. WP3：实现 Outline 确认、review 和 mode select，使用户能在 Outline 后选择 serial/batch。
5. WP4：实现 serial 单 item 路径，优先打通最小正确路径。
6. WP5：实现 batch 路径，复用 WP4 的 per-item prompt/local validator。
7. WP6：实现 final compile，使 Draft 真正物化为现有 `IssueWorkItemPlan` / `LifecycleWorkItemRecord` / `VerificationPlan` / child session。
8. WP7：前端完整接入；可以在 WP3 后先建类型和 fallback，WP6 后补 compile/recovery UI。
9. WP8：贯通验收，修正计划边界之外暴露的问题另起修复计划。

## 总体验证命令

每个 WP 使用自己的定向测试；全部完成后执行：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
pnpm -C web test
pnpm -C web build
```

## 完成标准

- WorkItemPlan 新流程不再在 Draft 阶段写真实 WorkItem / VerificationPlan / child session。
- Outline 确认后支持逐项生成和自动 batch 两条路径。
- reviewer 开关与 Story / Design / WorkItem 行为一致；WorkItemPlan 不做例外。
- `plan_reopen_required` 与 strict validator item/plan 级失败能按方案维护 Draft store 与 active index。
- Final Compile transaction 满足 marker 先后顺序、幂等续跑和 recovery node。
- 刷新恢复能还原 current outline、draft records、batch queue、active outline、compile transaction 和 artifact history MVP。
- 前端未知 node type 有降级 UI；`stage` 与 `active_node` 未就绪时不猜 UI。
- 每个计划文件大小小于 50k。
