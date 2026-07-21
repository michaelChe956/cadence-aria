# WorkItemPlan WP7：前端两阶段 Workspace 交互 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 前端支持 WorkItemPlan Outline / Draft / Batch / Compile artifact union、专属 node type 路由、生成模式选择、逐项确认、整组确认、compile recovery 和 MVP artifact 历史。

**Architecture:** `workspace-ws-store.ts` 从 `workItemPlanCandidate` 单字段迁移到 discriminated union；`ChatWorkspacePage.tsx` 以 `active_node.node_type` 而不是单纯 `stage` 渲染 WorkItemPlan 专属操作区。右侧 Artifact 面板实现 MVP：Outline 最新只读、Draft 状态列表、Compile report key-value/JSON pretty-print，不做完整结构化 diff 增强层。

**Tech Stack:** React、TypeScript、Zustand、Vitest、Testing Library、pnpm、lucide-react。

---

## 依赖

- 后端 WP3 后可以先做类型与 mode select UI。
- 完整 UI 依赖 WP6 的 compile/recovery payload。

## File Structure

| 文件 | 操作 | 职责 |
|---|---|---|
| `web/src/api/types.ts` | Modify | 新增 WS 输入/输出与 artifact union 类型 |
| `web/src/state/workspace-ws-store.ts` | Modify | union 状态、artifact history index、active node fallback |
| `web/src/hooks/useWorkspaceWs.ts` | Modify | 新 WS 消息发送与接收分发 |
| `web/src/pages/ChatWorkspacePage.tsx` | Modify | WorkItemPlan 专属面板路由 |
| `web/src/components/workspace/WorkItemPlanStagedPanel.tsx` | Create | Outline/Draft/Batch/Compile UI 壳 |
| `web/src/components/workspace/WorkItemPlanArtifactPanel.tsx` | Create | MVP artifact 历史展示 |
| `web/src/components/workspace/WorkItemPlanCandidatePanel.tsx` | Modify | legacy candidate 只作为 final candidate fallback |
| `web/src/state/workspace-ws-store.test.ts` | Modify | store 测试 |
| `web/src/hooks/useWorkspaceWs.test.tsx` | Modify | WS 消息测试 |
| `web/src/pages/ChatWorkspacePage.test.tsx` | Modify | 页面交互测试 |

## Task 1：TypeScript 类型与 store union

- [x] 写失败测试：`pnpm -C web exec vitest --run src/state/workspace-ws-store.test.ts`
  - `stores work item plan outline payload`
  - `stores draft payload without clearing artifact history`
  - `stores compile report payload`
  - `unknown node type keeps loading fallback`
- [x] `api/types.ts` 新增：
  - `WorkItemPlanOutlineCandidatePayload`
  - `WorkItemPlanContextBlockerPayload`
  - `WorkItemDraftCandidatePayload`
  - `WorkItemBatchStatePayload`
  - `WorkItemPlanCompileReportPayload`
  - `WorkItemPlanArtifactPayload` union。
- [x] `WorkspaceArtifact` 替换候选单字段，保留 legacy `{ candidate }`。
- [x] store 保存：
  - [x] `workItemPlanArtifact: WorkItemPlanArtifactPayload | null`
  - [x] `workItemPlanArtifactVersions: WorkItemPlanArtifactVersion[]`
  - [x] legacy `workItemPlanCandidate` 可保留过渡。

## Task 2：WS 发送与接收

- [x] 写失败测试：`pnpm -C web exec vitest --run src/hooks/useWorkspaceWs.test.tsx`
- [x] 新增发送函数：
  - `sendSelectWorkItemGenerationMode(mode)`
  - `sendRequestOutlineRevision(feedback?)`
  - `sendWorkItemDraftDecision(outlineId, decision, feedback?)`
  - `sendWorkItemBatchDecision(decision, feedback?, firstAffectedOutlineId?)`
  - `sendWorkItemPlanCompileRecoveryAction(action, reason?)`
- [x] `artifact_update` 分发按 union tag 或字段存在性识别，不再只判断 `candidate`。
- [x] `session_state` 中 stage 与 active node 一次性落 store；active node 未就绪时保持 loading。

## Task 3：WorkItemPlanStagedPanel

- [x] 写失败测试：`pnpm -C web exec vitest --run src/pages/ChatWorkspacePage.test.tsx`
  - `generation mode node shows serial batch revision buttons`
  - `draft confirm hides accept when validation failed`
  - `batch confirm shows accept all rewrite pause`
  - `compile recovery hides abort rollback after committed marker`
- [x] 新组件根据 `activeNode.node_type` 渲染：
  - [x] `work_item_plan_outline_confirm`：接受 Outline / 重写 Outline。
  - [x] `work_item_generation_mode`：逐个生成 / 自动生成 / 返回 Outline 返修。
  - [x] `work_item_draft_confirm`：接受 / 重写 / 暂停；校验失败隐藏接受。
  - [x] `work_item_batch_confirm`：接受全部 / 整组重写 / 暂停；严格校验失败时显示降级串行。
  - [x] `work_item_plan_compile_recovery`：继续 / 放弃 / 转人工，按 `plan_commit_state` 动态启用。
- [x] 不在 UI 内描述快捷键或实现细节。

## Task 4：Artifact MVP 面板

- [x] 写失败测试：
  - [x] `artifact panel lists outline draft batch compile groups`
  - [x] `timeline selection shows historical draft_readonly`
  - [x] `compile report renders_key_values`
- [x] MVP 范围：
  - [x] Outline 最新版本只读展示。
  - [x] Draft 显示 outline、status、accept 状态与写入范围。
  - [x] Batch snapshot 展示 queue 和 failure summary。
  - [x] Compile report 用 key-value 展示关键字段。
  - [x] 历史 draft_readonly 选择。
- [x] 对比 tab 可以先只对 compile report 做 before/after 文本展示；Outline/Draft diff 显示“本阶段未实现结构化对比”。

## Task 5：Unknown node type 与 protocol version fallback

- [x] 写失败测试：
  - [x] `unknown_work_item_plan_node_type_renders_processing_card`
  - [x] `upgrade_required_message_sets_protocol_error`
- [x] 未知 node type 显示通用“系统处理中...”卡片，展示 node_type 原始值。
- [x] 后端若推 `upgrade_required`，前端显示 protocol error alert。
- [x] 不缓存上一 active node 去匹配新 stage。

## Task 6：旧 CandidatePanel 兼容

- [x] 写失败测试：`legacy_candidate_payload_still_renders_existing_panel`
- [x] `WorkItemPlanCandidatePanel` 仅用于 legacy final candidate 或旧 session。
- [x] 新两阶段 payload 默认走 `WorkItemPlanStagedPanel` / `WorkItemPlanArtifactPanel`。

## 验证

```bash
pnpm -C web exec vitest --run src/state/workspace-ws-store.test.ts
pnpm -C web exec vitest --run src/hooks/useWorkspaceWs.test.tsx
pnpm -C web exec vitest --run src/pages/ChatWorkspacePage.test.tsx
pnpm -C web test
pnpm -C web build
```

## 不做

- 不做 P1/P2 完整结构化 diff。
- 不改后端。
- 不启动浏览器 E2E；贯通在 WP8。

## Commit

```bash
git add web/src/api/types.ts web/src/state/workspace-ws-store.ts web/src/hooks/useWorkspaceWs.ts web/src/pages/ChatWorkspacePage.tsx web/src/components/workspace/WorkItemPlanStagedPanel.tsx web/src/components/workspace/WorkItemPlanArtifactPanel.tsx web/src/components/workspace/WorkItemPlanCandidatePanel.tsx web/src/state/workspace-ws-store.test.ts web/src/hooks/useWorkspaceWs.test.tsx web/src/pages/ChatWorkspacePage.test.tsx
git commit -m "feat(web): add staged WorkItemPlan workspace UI"
```
