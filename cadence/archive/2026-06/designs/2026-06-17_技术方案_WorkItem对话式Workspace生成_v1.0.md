# WorkItem 对话式 Workspace 生成技术方案

## 文档信息

- 文档类型：技术方案
- 创建日期：2026-06-17
- 版本：v1.0
- 目标分支：`feat-b-0616`
- 工作区：`.worktrees/feat-b-0616`
- 适用范围：从 Design Spec 经对话式 ChatWorkspacePage 生成一组 Work Item 的完整流程——新增 `WorkspaceType::WorkItemPlan`、prepare 阶段创建 Draft `IssueWorkItemPlan` 作为结构化参数来源、author + review + revert + revision 对话式状态机、结构化 candidate 产物承载、confirm 状态推进与子 session 建立
- 相关总览：`cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_拆分总览_v1.0.md`
- 取代：`cadence/plans/2026-06-17_实施计划_WorkItem对话式Workspace生成_方案B_v1.0.md`（方案 B v1.0，已废弃）
- 修订依据：无（首版，基于 2026-06-17 设计讨论结论：路线 B+ + 选项 A + 重做语义）

## 背景

`feat-b-0616` 分支的 P3 已实现 Work Item 生成的**弹窗式 REST 流程**：

- 入口：IssueLifecycleWorkbench 上选中 Design Spec → 点"生成下一阶段" → 弹窗选项 → 确认。
- 后端：`POST /work-items:generate`（`handlers.rs:512`）同步 `spawn_blocking` 调 Provider，一次性创建 `IssueWorkItemPlan` + 多个 `WorkItem` + `VerificationPlan` + 每个 WorkItem 的 workspace session。
- 组级确认/改请求走独立 REST：`POST /work-item-plans/{plan_id}/confirm`、`/change-request`。
- 成功后前端打开第一个 WorkItem 的 Drawer。

Story Spec / Design Spec 的生成则是对话式 Workspace（`ChatWorkspacePage` + WebSocket 状态机 + author/review/revision）。两者体验割裂。

## 目标

把 Work Item 生成流程对齐 Story/Design 的对话式体验：

- 从 Design Spec 进入 `ChatWorkspacePage`（新 `WorkspaceType::WorkItemPlan`）。
- `prepare` 创建空 Draft `IssueWorkItemPlan` + `WorkItemPlan` session，并注入上下文消息，不调 Provider。
- 用户点"开始生成" → author 阶段（`WorkItemSplitter`）**整组生成**候选计划。
- 候选以 LifecycleStore Draft 记录为事实来源，并同步为结构化 `artifact_version` candidate 供 AuthorConfirm 展示、恢复和 WS 推送。
- revert 批量触发 Revision，`WorkItemSplitter` 带 feedback **重做被 revert 的**、其余保留、DAG 自动重连。
- reviewer 对**整组** candidate 做语义审查（流式）。
- `HumanConfirm::Confirm` 调 `confirm_issue_work_item_plan` 推进已存在 Draft 记录，并建立每个 WorkItem 的子 Coding session。

## 非目标

- **不做**两阶段（大/小 Work Item）拆分。本方案是单层、单一粒度拆分，与 P3 现有 WorkItem 粒度一致。
- **不改** P4–P9 的共享 worktree、Coding 门禁、ExecutionPlan、Handoff 等已落地能力。
- **不做** Playwright 浏览器 E2E。
- **不做**多 Work Item 并行生成或真·流式逐个生成（author 是 `spawn_blocking` 整组返回）。

## 核心决策

### 1. 替代而非并存 P3 REST 流程

新对话式入口**完全取代**弹窗式 `/work-items:generate` 流程。前端入口全切 Workspace；3 条废弃 REST 路由直接删除（feat-b-0616 分支内变更、未合并 main，删除代价最小）。理由：并存会造成两套状态机与产物链路长期维护负担，且方案 C 已能覆盖 P3 REST 的全部能力。

### 2. 产物路线 B+：Lifecycle Draft 为事实来源，结构化 candidate 纳入 artifact payload

candidate（`plan + work_items + DAG + verification_plans + repository_profile + validator_findings`）在后端以 LifecycleStore 的 Draft `IssueWorkItemPlan`、Draft `LifecycleWorkItemRecord`、`VerificationPlan`、`RepositoryProfile` 作为事实来源；同时序列化为结构化 artifact payload，复用 Story/Design 已有的 artifact 版本管理、`ArtifactUpdate` 推送、恢复链路。**不新增** `WsOutMessage::WorkItemPlanCandidate` 变体（方案 B v1.0 的设想），改为把现有 markdown artifact 扩展为 `ArtifactPayload::Markdown | ArtifactPayload::WorkItemPlanCandidate` union。

理由：最大化复用 Story/Design 的状态机骨架；产物数据结构完整无损（相比"把拆分计划写成 Markdown 再解析"的路线 A，无解析损耗）；相比"纯结构化、不碰 artifact 概念"的路线 B，review/revision/version 链路可直接复用；同时 confirm 阶段可直接复用现有 `confirm_issue_work_item_plan` 对 Draft plan/work_items 的一致性检查。

### 2.1 prepare 阶段的唯一结构化来源：空 Draft `IssueWorkItemPlan`

`prepare_work_item_plan` 不调用 Provider，但必须创建一个空 Draft `IssueWorkItemPlan`：

- `source_story_spec_ids` / `source_design_spec_ids` / `options` 来自 prepare 请求体。
- `work_item_ids` / `verification_plan_ids` / `dependency_graph` 初始为空。
- `session.workspace_type = WorkItemPlan`，`session.entity_id = plan.id`。

author/revision 阶段只从 `session.entity_id` 读取 plan，再从 plan 中读取 split options/spec ids。不要把这些结构化参数只塞进上下文消息，因为现有 `WorkspaceSessionRecord` 没有 metadata/context 字段，消息文本不能作为后续状态机的可靠数据源。

### 3. 不新增 Timeline 节点、不新增 WorkspaceStage、不新增 AdapterRole

- Timeline 复用 `PrepareContext / StartGeneration / AuthorRun / AuthorConfirm / ReviewerRun / ReviewDecision / Revision / HumanConfirm / Completed`。
- WorkspaceStage 8 阶段全复用。
- AdapterRole 用现有 `WorkItemSplitter`（author/revision）+ `Reviewer`（review）。

理由：现有状态机与节点体系已与 `WorkspaceType` 解耦，新类型自动适配；新增变体只会增加恢复链路与序列化的分支维护成本。

### 4. 唯一新增 WS 消息：`WsInMessage::RevertWorkItem`

AuthorConfirm 阶段标记单个 WorkItem 重做：`{ work_item_id, feedback, clear }`。这是整个设计里唯一真正新增的 WS 消息变体。其余 `StartGeneration / AuthorDecision / ReviewDecisionResponse / RequestRevision / HumanConfirm / Abort` 全复用。

### 5. provider 调用方式按阶段分化

- author / revision：`WorkItemSplitter`，经 `WorkItemSplitEngine::generate` 内部的 `spawn_blocking`（`work_item_split_engine.rs:227`）一次性返回结构化输出，**非流式**。
- review：`Reviewer`，流式（复用 `drive_review_session`，`workspace_engine.rs:1582`），reviewer 审查意见是文字，适合流式展示。

体验差异：author/revision 等待一次性返回 candidate；review 流式打字。这是产物形态决定的，合理。

### 6. review 粒度：整组一次审查

author 一次性生成完整一组 WorkItem 后，reviewer 对**整组 candidate** 做一次语义审查（不是逐个 WorkItem review）。对应"全部 work item 一起生成、一起审"。

### 7. 单 WorkItem 干预：选项 A（整组生成 + AuthorConfirm 逐个 revert）+ 重做语义

- author 整组生成（保 DAG 全局规划质量）。
- AuthorConfirm 阶段逐个展示，每个 WorkItem 可 `[revert]`（附反馈）。
- revert = **重做**：删掉被 revert 的 WorkItem，Revision 阶段 `WorkItemSplitter` 重生一个等位的新 WorkItem 顶替，整组数量不变。
- DAG 自动重连：其他 WorkItem 对原 id 的 `depends_on` 改指向新 id。

理由：work item 拆分本质是全局规划，一次性出整组 DAG 更合理；用户核心诉求"单个不好能 revert"用最小改动满足；真·逐个流式（方案 B v1.0 设想的 `WorkItemSplitRun` 流式）成本高且规划质量难保。

### 8. revert 批量触发

用户在 AuthorConfirm 可连续标记多个 revert（每个发 `RevertWorkItem`，candidate 内对应项标记 `reverted` + 存 feedback），点"重新生成被标记的 N 项"按钮触发**一次** Revision，`WorkItemSplitter` 一次性重做所有被标记的。理由：省 provider 调用、artifact version 不翻倍、用户可审完整组再决定。

### 9. confirm 后才建子 Coding session

draft 阶段会持久化 plan/work_items/verification_plans/repository_profile，但 work_items 均保持 `Draft` 状态，不建子 session。`HumanConfirm::Confirm` 时才：调 `confirm_issue_work_item_plan`（plan/work_items → `Confirmed`）+ 为每个 WorkItem 建 `WorkspaceType::WorkItem` 子 session。

理由：避免 draft 被废弃时留下一堆空 session；现有 P3 的 `persist_work_item_split_provider_output` 在生成时即建 session 的逻辑需拆分。

### 10. revision 用 WorkItemSplitter（不用 Orchestrator）

Revision 阶段产物格式必须与 author 一致（结构化 candidate），因此 revision 也用 `WorkItemSplitter` + feedback prompt，不用 Story/Design revision 的 `Orchestrator` 改写路径。`WorkItemSplitEngine` 新增 `generate_revision` 方法支持"后端保留 retained + provider 只生成 redo 项 + DAG repatch"。局部重做时不要求 provider 原样输出 retained，因为现有 split schema 的 `work_items` 没有 id 字段，后端必须作为 retained 的事实来源。

### 11. prepare 请求体复用现有 provider 配置解析

prepare 请求体：`title + story_spec_ids + design_spec_ids + 4 个 split 选项 + author_provider/reviewer_provider/review_rounds/superpowers_enabled/openspec_enabled`。复用现有 `provider_workspace_config`（`handlers.rs:3113`）解析。与现有 `GenerateWorkItemsRequest` 契约一致。

### 12. 删除 3 条废弃 REST 路由

`/work-items:generate`、`/work-item-plans/{plan_id}/confirm`、`/work-item-plans/{plan_id}/change-request` 路由及对应 handler 直接删除。其底层逻辑（`persist_work_item_split_provider_output` 的持久化、`confirm_issue_work_item_plan` 的状态推进）作为内核保留，迁入新 WS 流程。

## 总体流程

```
[IssueLifecycleWorkbench: Design Spec → "生成下一阶段"]
   │ POST /work-item-plans:prepare
   ▼
[prepare_work_item_plan handler]
   │ 建空 Draft IssueWorkItemPlan + WorkspaceSession(WorkItemPlan)
   │ session.entity_id = plan_id，注入上下文消息
   │ 返回 workspace_session_id
   ▼
[ChatWorkspacePage] 连接 WS → 收 SessionState(WorkItemPlan)
   │ 用户点"开始生成" → sendStartGeneration
   ▼
[WorkItemPlan 非流式 author run]
   │ spawn_blocking(WorkItemSplitEngine::generate) → parse → validate
   │ 持久化/替换 Draft candidate 记录
   │ 同步 artifact_version(candidate) → 推 ArtifactUpdate
   │ 进 AuthorConfirm
   ▼
[ChatWorkspacePage: WorkItemPlanCandidatePanel]
   │ 用户逐个审视，对不满意的发 RevertWorkItem{work_item_id, feedback}
   │ （可连续标记多个）
   │
   │ ┌─ 点"重新生成被标记的" → RequestRevision ───────────────────┐
   │ │                                                          ▼
   │ │  [Revision: WorkItemSplitter + feedback 局部重做] → 新 version → 回 AuthorConfirm
   │ │
   │ └─ 点"确认计划" → AuthorDecision::Accept
   │     │ 若 review_rounds>0 且有 reviewer → 进 CrossReview（流式审整组）
   │     │   ├─ ReviewDecision: continue → HumanConfirm
   │     │   └─ ReviewDecision: continue_with_context / RequiresRevision → Revision
   │     ▼
   │  HumanConfirm::Confirm
   ▼
[confirm_issue_work_item_plan + 为每个 WorkItem 建子 Coding session]
   │ Completed
   ▼
[回到 IssueLifecycleWorkbench，WorkItem 列展示，可进入各自 Coding Workspace]
```

## 数据模型

### WorkItemPlanCandidate（Lifecycle Draft + artifact payload）

WorkItemPlan 下的 candidate 有两个层次：

1. **事实来源**：LifecycleStore 中的 Draft `IssueWorkItemPlan`、Draft `LifecycleWorkItemRecord`、`VerificationPlan`、`RepositoryProfile`。
2. **展示/恢复镜像**：artifact payload 中的 `WorkItemPlanCandidateDto`，由事实来源组装，随 `ArtifactUpdate` 和 `SessionState.artifact` 推给前端。

`WorkItemPlanCandidateDto` 结构与 `WorkItemSplitProviderOutput`（`work_item_split_engine.rs:133-139`）对齐：

```json
{
  "plan": { "id": "...", "status": "draft", "options": {...}, "dependency_graph": [...] },
  "work_items": [
    { "id": "...", "kind": "...", "title": "...", "depends_on": [...],
      "exclusive_write_scopes": [...], "verification_plan_ref": "...",
      "meta": { "reverted": false, "revert_feedback": null } }
  ],
  "verification_plans": [ ... ],
  "repository_profile": { "confidence": "high", "languages": [...], ... },
  "validator_findings": [ { "severity": "warning", "code": "...", "message": "...", "work_item_ids": [...] } ]
}
```

- 每次 author/revision 生成 → 先替换当前 Draft candidate 记录，再写入新 `artifact_version`（version 号递增）。
- `work_items[i].meta.reverted` + `revert_feedback`：AuthorConfirm 阶段的 revert 标记态（标记不产生新 version，只写回当前 `ArtifactVersion.payload` 的 candidate meta，并同步 `session.artifact` 后推同 version `ArtifactUpdate`；重连从当前 artifact version 恢复，不丢标记）。
- `validator_findings` 随 candidate 一起存与推送（warning 不阻断，error 触发自动 Revision，见"author 阶段"）。

### Draft candidate 替换接口

新增 `LifecycleStore::replace_issue_work_item_plan_candidate`（命名实现时可调整，但语义必须保持）：

- 输入：`plan_id`、新的 `WorkItemSplitProviderOutput`、validator findings。
- 前置：目标 `IssueWorkItemPlan.status == Draft`；不得替换 `Confirmed` plan；不得删除已有 `WorkspaceType::WorkItem` 子 session 的记录。
- 行为：删除或解除引用旧 Draft work_items / verification_plans / repository_profile；创建新的 Draft work_items、verification_plans、repository_profile；更新同一个 plan 的 `work_item_ids`、`verification_plan_ids`、`repository_profile_ref`、`dependency_graph`、`created_from_provider_run`、`validator_findings`。
- 输出：当前完整 `WorkItemPlanCandidateDto`。

这样 revision 能生成新 candidate，又不需要创建新的 plan，也不会让 confirm 阶段面对 artifact-only 数据。

### ArtifactUpdate payload 扩展

现有 `WsOutMessage::ArtifactUpdate { version, markdown, diff }` 中 `markdown` 是必填 `String`。本方案必须把后端/前端 artifact 协议升级为 union，而不是只在消息上补一个孤立字段：

- `ArtifactPayload::Markdown { markdown, diff }`：Story/Design 使用。
- `ArtifactPayload::WorkItemPlanCandidate { candidate }`：WorkItemPlan 使用。
- `EngineEvent::ArtifactUpdate`、`WsOutMessage::ArtifactUpdate`、前端 `workspace-ws-store` / `useWorkspaceWs` 必须同步接受 union。
- 为兼容现有前端代码，JSON 可保留 `markdown?: string | null`、`diff?: string | null`、`candidate?: WorkItemPlanCandidateDto` 字段形态，但 TypeScript/Rust 类型上必须表达互斥语义，避免前端继续无条件读取 `markdown.length`。

### SessionState.artifact 扩展

`SessionState.artifact` 同样扩展为 union（markdown 或 candidate），保证断连重连后前端能恢复候选展示。`artifact_version_summaries` 可以继续只保存 version/source/review 摘要；完整 candidate 由当前 Draft lifecycle 记录组装或从 artifact payload 加载。

### WorkspaceType 枚举

```rust
pub enum WorkspaceType { Story, Design, WorkItem, WorkItemPlan }  // 新增 WorkItemPlan
```

序列化为 `snake_case`：`"work_item_plan"`。

## 状态机行为映射（WorkItemPlan 走 8 阶段）

| 阶段 | WorkItemPlan 具体行为 | 复用 / 新增 |
|---|---|---|
| **PrepareContext** | `prepare` 建空 Draft `IssueWorkItemPlan` + `WorkItemPlan` session，`session.entity_id = plan_id`，注入上下文消息（Design/Story Spec 摘要 + 仓库结构 + 4 个 split 选项 + provider 配置），不调 provider | 新 handler + `workspace_context` 分支 |
| **Running / AuthorRun** | 用户点"开始生成" → WS handler 启动 WorkItemPlan 非流式 run → `WorkItemSplitEngine::generate` → `parse_provider_output` → `Validator::validate` → 替换 Draft candidate 记录 → 写 artifact payload → 推 `ArtifactUpdate` → 进 AuthorConfirm | engine + WS handler 新分支（非流式） |
| **AuthorConfirm** | 逐个展示 WorkItem（DAG + 范围 + 验证计划 + findings），每个可 `[revert]`（附反馈）；用户 Accept 或标记 revert | 复用阶段；前端新面板 |
| **CrossReview / ReviewerRun** | reviewer **流式**审整组 candidate（新 `build_work_item_plan_review_input`）→ 输出 verdict | 新 review prompt；verdict/gate/decision 复用 |
| **ReviewDecision** | continue / continue_with_context / human_intervene | 复用 |
| **Revision** | `WorkItemSplitter` + feedback 重做（被 revert 的 WorkItem + review findings）→ 替换 Draft candidate → 新 `artifact_version` → 回 AuthorConfirm | engine + WS handler 新分支（局部重做） |
| **HumanConfirm** | Confirm = `confirm_issue_work_item_plan` + 建子 session；RequestChange = 回 Revision；Terminate = 废弃 | `handle_confirm` 加分支 |
| **Completed** | plan Confirmed，关联 WorkItem 可被 Coding | 复用 |

`WorkspaceSessionStatus` 与 `WorkspaceStage` 的映射（`workspace_stage_for_status` / `workspace_status_for_stage`）类型中性，新类型自动适配。

## 关键机制设计

### author 阶段与候选生成

**调用链**：

```
start_generation（WorkItemPlan 分支）
  → 从 session.entity_id 读取 Draft IssueWorkItemPlan，取 split options + spec_ids
  → WS handler / WorkItemPlan run kind 持有 provider_adapter
  → WorkItemSplitEngine::generate(...)            // 复用，role=WorkItemSplitter，内部 spawn_blocking
  → parse_provider_output                          // 复用纯函数
  → Validator::validate(plan, work_items, profile, verification_plans)  // 复用
  → 按 findings 分支：
       has_errors     → 自动进 Revision（见下文）
       warnings only  → findings 随 candidate 推送，进 AuthorConfirm
  → replace_issue_work_item_plan_candidate（Draft plan + Draft work_items，不建子 session）
  → 写 candidate payload
  → 推 ArtifactUpdate(candidate) + TimelineNode(AuthorRun, Completed)
  → enter_author_confirm
```

**Provider 调用方式**：`WorkItemSplitEngine::generate` 内部已 `spawn_blocking`（`work_item_split_engine.rs:227`），不得再走 `drive_provider_session` 的流式 provider 路径。现有 `WorkspaceEngine` 不持有 `provider_adapter`，因此实现时必须二选一并在 WP2 落定：

1. 给 `WorkspaceEngine` 注入 WorkItemSplit 依赖，`start_generation` / dedicated method 内部执行非流式 run。
2. 在 `workspace_ws_handler.rs` 新增 `ProviderRunKind::WorkItemPlanAuthor`，由 handler 使用 `state.provider_adapter` 构造 `WorkItemSplitEngine`，再调用 engine 的持久化/阶段推进方法。

推荐 2：改动边界更贴近现有 P3 REST 生成入口，也避免把通用 Chat Workspace engine 直接绑定非流式 provider adapter。取消语义：用户中途断连/Abort 通过 `CancellationToken` 标记丢弃结果；已发起的 `spawn_blocking` provider 调用允许跑完（结果丢弃），不阻塞 engine。

**上下文参数来源**：prepare 请求体字段必须存入 Draft `IssueWorkItemPlan`。author 阶段根据 `session.entity_id` 读取 plan，把 plan 的 source ids/options 组装回 `GenerateWorkItemsRequest` 兼容结构传给 `WorkItemSplitEngine::generate`。

**validate 失败处理（WS 语义，无 HTTP 422）**：`has_errors` 时不进 AuthorConfirm；candidate 仍替换 Draft 记录并写入 artifact payload（带 error findings）后推 `ArtifactUpdate`；自动进入 Revision，prompt 带 validator error findings 让 `WorkItemSplitter` 立即修正 → 新 version → 再判 AuthorConfirm；连续失败超阈值（如 3 次）→ 进 `HumanConfirm` 交用户决策。

**system_prompt_for(WorkItemPlan) 的定位**：与 Story/Design 不同——author 阶段实际调 `WorkItemSplitEngine::generate`（自带 `build_split_prompt` 构建完整 prompt）。因此 `workspace_context::system_prompt_for(WorkItemPlan)` 的职责是**注入 session 的"上下文消息"**（向用户展示本 Workspace 任务背景、约束、产物结构），**不直接作为 provider 的 system prompt**。

### AuthorConfirm 与逐个 revert

**展示**：前端 `WorkItemPlanCandidatePanel`——WorkItem 列表（标题 / kind / 写入范围 / 预算 / 验证计划摘要）、DAG 图、Repository Profile、validator findings（warning）。每个 WorkItem 一个卡片，带 `[revert]` 按钮（可弹反馈输入框）。

**revert 标记（批量）**：

- 用户点某 WorkItem 的 `[revert]` → 发 `WsInMessage::RevertWorkItem { work_item_id, feedback, clear: false }`。
- 后端把该 WorkItem 在**当前 artifact_version** 标记为 `reverted`（`work_items[i].meta.reverted = true` + 存 feedback），写回当前 `ArtifactVersion.payload`，同步 `session.artifact`，再推 `ArtifactUpdate`（同 version，仅 meta 变化）。不得只改内存态。
- 用户可连续标记多个；可取消（发 `RevertWorkItem` 带 `clear: true`）。

**触发 Revision**：用户点"重新生成被标记的 N 项" → 发 `WsInMessage::RequestRevision { feedback }`（复用现有消息）。后端收集所有 `reverted` WorkItem 及 feedback → 进 Revision 阶段。

**重做语义**：Revision 阶段 `WorkItemSplitter` 的 prompt 约束——保留未标记的 WorkItem 由后端直接沿用（id + 内容固定不变），provider 只输出被 revert 的等位新项；整组数量不变。新生成 WorkItem 的 id 仍由后端基于 `next_sequential_id` 分配。DAG 自动重连：后端根据 `redo_specs` 顺序建立 old→new 映射，`repatch_dependencies` 重写 `dependency_graph` 与 retained WorkItem 的 `depends_on`，把对原 id 的引用改为新 id。

### Review 阶段（整组）

**review prompt**：新增 `build_work_item_plan_review_input`——从当前 Draft plan 关联记录组装 candidate（plan + work_items + dependency_graph + exclusive_write_scopes + verification_plans + repository_profile），再序列化为 review 上下文（裁剪 token：repository_profile 只传 confidence + detected_layers，WorkItem 只传 reviewer 关心字段）。reviewer 契约：评估拆分粒度合理性、依赖完整性、写入范围互斥、跨端拆分恰当性、验证计划覆盖度，输出 JSON verdict。

**复用**：reviewer 用 `AdapterRole::Reviewer` + 流式（`drive_review_session`）。verdict 解析（`parse_review_verdict`）、`review_gate`（`RequiresRevision` / `CanConfirm`）、`ReviewDecisionResponse`（continue / continue_with_context / human_intervene）全复用。review 触发的 Revision 走"Revision 阶段"（review findings 作为 feedback 注入）。

### Revision 阶段（局部重做）

Revision 有三个来源，统一走 `WorkItemSplitter` + feedback：

| Revision 触发来源 | feedback 内容 | 重做范围 |
|---|---|---|
| AuthorConfirm 的 revert 批量 | 各 WorkItem 的 revert feedback | 只重做被 revert 的（保留其余） |
| Review 的 `RequiresRevision` | review findings + summary + 用户 `extra_context` | 整组可调（允许微调非保留项） |
| validate 失败自动触发 | validator error findings | 整组修正 |

- revision 先替换当前 Draft candidate 记录，再产生新 `artifact_version` → 回 **AuthorConfirm**（不是直接进 review，让用户再看改完的候选）。
- `WorkItemSplitEngine` 新增 `generate_revision(..., retained: &[LifecycleWorkItemRecord], redo_specs: &[RedoSpec])`：局部重做时 prompt 注入 retained 摘要作为不可改上下文，并要求 provider 只输出 redo 项（数量与 `redo_specs` 一致）；后端把 retained 原记录与 redo 输出合并成完整 candidate，再执行 `repatch_dependencies`。整组 review/AutoRevision（`retained`/`redo_specs` 均空）退化为普通整组 split，直接解析完整 provider 输出。

### Confirm 与子 session 建立

`HumanConfirm::Confirm` 在 WorkItemPlan 分支：

1. 根据 `session.entity_id` 找到 Draft `IssueWorkItemPlan`。
2. `LifecycleStore::confirm_issue_work_item_plan`（`lifecycle_store.rs:473`）—— plan.status `Draft→Confirmed`，关联 WorkItem `plan_status → Confirmed`。
3. **此时才建子 Coding session**：为每个 WorkItem 调 `create_workspace_session(WorkspaceType::WorkItem, entity_id=work_item.id)`，注入 WorkItem 上下文消息；若 session 已存在则跳过，保证重试幂等。
4. 从 `persist_work_item_split_provider_output`（`handlers.rs:589`）抽取的"建 work_items/verification_plans/repository_profile 并更新 plan"持久化逻辑已在 author/revision 生成时完成（candidate 落盘），confirm 只做 status promote + 建 session。

`RequestChange` → 进 Revision（用户补充反馈），不调 `request_issue_work_item_plan_change`（该 REST 将删除）。`Terminate` → session `Terminated`，draft candidate 保留可追溯但不 promote。

## WS 协议

### 复用（不改）

- `WsOutMessage`：`SessionState` / `ArtifactUpdate` / `TimelineNodeCreated` / `ProviderStatus` / `StreamChunk`（review 用）/ `ReviewComplete` / `ReviewDecisionRequired` / `StageChange` / `Error` 等复用既有消息语义；其中 `ArtifactUpdate` 和 `SessionState.artifact` 的 payload 类型扩展为 union。
- `WsInMessage`：`StartGeneration` / `AuthorDecision` / `ReviewDecisionResponse` / `RequestRevision` / `HumanConfirm` / `Abort` 等全部复用。
- `TimelineNodeType`：11 个变体全部复用。
- `WorkspaceStage`：8 阶段全部复用。

### 新增 `WsInMessage::RevertWorkItem`

```jsonc
{
  "type": "revert_work_item",
  "work_item_id": "work_item_xxx",
  "feedback": "拆得太粗，API 和数据模型应该分开",
  "clear": false
}
```

### `ArtifactUpdate` / `SessionState.artifact` 扩展（非新增消息变体，payload 改为 union）

```jsonc
{
  "type": "artifact_update",
  "version": 3,
  "markdown": null,
  "diff": null,
  "candidate": { "plan": {...}, "work_items": [...], "verification_plans": [...],
                 "repository_profile": {...}, "validator_findings": [...] }
}
```

## 后端设计

### WorkspaceType 枚举（`src/product/models.rs`）

加 `WorkItemPlan` 变体（当前 `enum WorkspaceType` 在 `:239`）。

### WorkspaceEngine 接入分支（`src/product/workspace_engine.rs`）

| 函数（当前行号，实现时以实际为准） | 改动 |
|---|---|
| `start_generation` (:746) / WS handler run 分发 | WorkItemPlan 分支：进入非流式 WorkItemSplit run，不走 `drive_provider_session` 流式 author |
| `complete_assistant_message` (:2436) / `content_has_complete_workspace_artifact` (:3963) | WorkItemPlan 完成判定 = `WorkItemSplitEngine` 返回成功 + parse + validate |
| `workspace_requires_artifact_gate` (:2888) | WorkItemPlan 纳入 gate |
| `enter_author_confirm` (:3108) | WorkItemPlan 下推进 candidate 展示 |
| `build_review_input` (:2470) | 加 WorkItemPlan 分支 → `build_work_item_plan_review_input` |
| revision 构建（`drive_revision_session` :1620 周边） | WorkItemPlan 分支：`WorkItemSplitter` + feedback，调 `generate_revision` |
| `handle_confirm`（含 `match workspace_type` :2694） | 加 WorkItemPlan 分支 → `confirm_issue_work_item_plan` + 建子 session |
| `workspace_type_title` (:3740) | 加 `WorkItemPlan => "Work Item Plan"` |
| revert 标记处理（新增） | 收 `RevertWorkItem` → 更新 candidate meta → 推 `ArtifactUpdate` |

### WS handler 接入（`src/web/workspace_ws_handler.rs`）

WorkItemPlan 不是普通流式 author run，必须改动 handler：

- `StartGeneration` 在 `workspace_type == WorkItemPlan` 时启动 `ProviderRunKind::WorkItemPlanAuthor`，不再启动 `ProviderRunKind::Author { content: "" }`。
- `RevertWorkItem` 加入 `is_message_valid_for_stage` 的 AuthorConfirm 白名单，并在 `message_type` 中返回 `revert_work_item`。
- `RequestRevision` 在 WorkItemPlan 下启动 `ProviderRunKind::WorkItemPlanRevision`，不走普通 `drive_revision_session`。
- event forwarder 支持 `EngineEvent::ArtifactUpdate` 的 artifact payload union。

### LifecycleStore 接入（`src/product/lifecycle_store.rs`）

新增窄接口支撑 draft candidate 生命周期：

- `create_issue_work_item_plan` 继续用于 prepare 阶段创建空 Draft plan。
- 新增 `replace_issue_work_item_plan_candidate`，负责 author/revision 后替换 Draft candidate。
- 新增或复用 helper 在 confirm 后为每个 confirmed WorkItem 创建 `WorkspaceType::WorkItem` 子 session，且幂等跳过已存在 session。

### workspace_context 分支（`src/web/workspace_context.rs`）

约 10 处加 WorkItemPlan 分支：`workspace_entity_context` (:207)、`workspace_type_label` (:390)、`node_id_for` (:398)、`workspace_runtime_role` (:406)、`system_prompt_for` (:414)、`constraint_summary_for` (:428)、`workflow_discipline_for` (:449)、`output_schema_for` (:479)、`is_workspace_generation_brief` (:187)。

### WorkItemSplitEngine 扩展（`src/product/work_item_split_engine.rs`）

- 新增 `generate_revision(..., retained, redo_specs)` 方法（不改 `generate` 主体）；局部重做使用 redo-only provider schema/parser，整组修正可复用现有完整 split parser。
- 新增 `repatch_dependencies` 纯函数（DAG 重连）。
- 复用 `build_split_prompt` / `WORK_ITEM_SPLIT_OUTPUT_SCHEMA` / `parse_provider_output` / `summarize_repository_structure`；局部重做新增 revision-only prompt/schema/parser，避免要求现有 `ProviderWorkItem` 携带 id。

### prepare handler（`src/web/handlers.rs`）

新增 `prepare_work_item_plan`（参考 `generate_story_specs` :437）：建空 Draft `IssueWorkItemPlan` + session，注入上下文，返回 `workspace_session` 与 `plan_id`。`workspace_type_text` (:3063) 加分支。

### 可复用内核（来自 P3，不改或最小改）

| 内核 | 位置 | 复用方式 |
|---|---|---|
| `WorkItemSplitEngine::new/generate/build_split_prompt/parse_provider_output` | `work_item_split_engine.rs` | author 直接调；revision 扩展 |
| `WorkItemSplitValidator::validate` 及全部子检查 | `work_item_split_validator.rs` | generate 后立即调 |
| `LifecycleStore::confirm_issue_work_item_plan` | `lifecycle_store.rs:473` | 映射 `HumanConfirm::Confirm` |
| `provider_workspace_config` | `handlers.rs:3113` | prepare 请求体解析 |
| `LifecycleStore::create_*` / `list_*` | `lifecycle_store.rs` | 持久化与恢复原语 |
| Story/Design 的 8 阶段状态机 / WS 分发 / 恢复链路 | `workspace_engine.rs` / `workspace_ws_handler.rs` | 复用阶段语义；author/revision run 分发对 WorkItemPlan 特化 |

## 前端设计

### 入口改造（`web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`）

`handleGenerateNext(design_spec)` 与 `handleLaunchWorkspace("work_item")` 改为调 `prepareWorkItemPlan` 并打开 `ChatWorkspacePage`；移除 `setPendingWorkItemGenerate` 弹窗逻辑（对齐 `handleLaunchWorkspace("story"/"design")`）。

### ChatWorkspacePage 分支（`web/src/pages/ChatWorkspacePage.tsx`）

按 `workspace_type === "work_item_plan"` 分支：不渲染 Markdown Artifact Pane，改渲染 `WorkItemPlanCandidatePanel`；复用 Story/Design 的 review/confirm/revision 交互骨架。

### WorkItemPlanCandidatePanel（新增，`web/src/components/workspace/`）

展示 candidate：WorkItem 列表（标题/kind/范围/预算/验证摘要）、DAG 图、Repository Profile、findings；每个 WorkItem `[revert]` 按钮 + 反馈输入；底部"重新生成被标记的 N 项" / "确认计划" 按钮。

### WS store / API client

`workspace-ws-store.ts` / `useWorkspaceWs.ts`：处理 artifact payload union（markdown 或 candidate），维护 `workItemPlanCandidate` 状态；新增 `sendRevertWorkItem`；复用 `sendStartGeneration` / `sendAuthorDecision` / `sendReviewDecision` / `sendHumanConfirm`。
`api/client.ts` / `types.ts`：新增 `prepareWorkItemPlan` 函数 + `PrepareWorkItemPlanRequest/Response` + `WorkItemPlanCandidateDto` + `RevertWorkItem` 消息类型（参考 `generateStorySpecs` / `generateDesignSpecs`）。

> 前端具体行号在 WP6/WP7 子计划里基于实现时代码确认。

## 废弃项（直接删除）

| 删除项 | 位置 |
|---|---|
| 路由 `POST /work-items:generate` | `app.rs:80-81` |
| 路由 `POST /work-item-plans/{plan_id}/confirm` | `app.rs:84` |
| 路由 `POST /work-item-plans/{plan_id}/change-request` | `app.rs:88` |
| handler `generate_work_items` / `confirm_issue_work_item_plan` / `request_issue_work_item_plan_change` | `handlers.rs:512 / :845 / :860` |
| `build_generate_work_items_response` + 相关 DTO | `handlers.rs:705-843` |

> `validate_work_item_generation_candidates`、`persist_work_item_split_provider_output` 的**逻辑**保留（迁入新流程），仅删除 REST 包装。

## 测试策略

### Rust 单元测试

- `workspace_context.rs` WorkItemPlan 各分支（prompt/schema/label/node_id/brief 识别）。
- `WorkItemSplitEngine::generate_revision` 的 redo-only 输出解析、retained + redo 后端合并、`repatch_dependencies` DAG 重连。
- `WorkspaceEngine` / `workspace_ws_handler` WorkItemPlan：非流式 author run 分发、candidate payload 推送、revert 标记、confirm 状态推进 + 子 session 建立。
- `LifecycleStore::replace_issue_work_item_plan_candidate`：只允许替换 Draft plan；替换 work_items / verification_plans / repository_profile 后 plan 引用一致。
- validate 失败自动 Revision、连续失败进 HumanConfirm。

### Rust 集成测试

复用 `MockSplitProviderAdapter` + Fake provider（`tests/it_web/`）：

- `prepare_work_item_plan_creates_draft_plan_and_session_without_generating`
- `work_item_plan_start_generation_returns_candidate_artifact`
- `work_item_plan_author_persists_draft_candidate_records_without_child_sessions`
- `revert_work_item_triggers_local_redo_in_revision`
- `revision_replaces_draft_candidate_without_touching_confirmed_records`
- `confirm_creates_child_work_item_sessions`
- `review_returns_verdict_for_whole_candidate`
- `delete_legacy_rest_routes_returns_404`

### 前端测试

- `IssueLifecycleWorkbench.test.tsx`：Design Spec 入口打开 Workspace（不再弹窗）。
- `ChatWorkspacePage.test.tsx`：`work_item_plan` 分支渲染 candidate 面板。
- `WorkItemPlanCandidatePanel.test.tsx`：列表/DAG/revert 标记/批量触发/确认。
- `workspace-ws-store.test.ts`：artifact payload union、candidate 消息收发、revert 发送。

### 一致性覆盖

- Story / Design / WorkItem / WorkItemPlan 四种 workspace type 的 timeline 恢复、chat entry 重建、provider conversation 恢复需要逐一评估；其中 Story/Design/WorkItemPlan 复用 ChatWorkspacePage，WorkItem 既有 Coding Workspace 链路若不受影响，测试说明必须写明原因。
- `SessionState.workspace_type = work_item_plan` 经 serde 往返不丢失 candidate。

## 风险与回退

| 风险 | 影响 | 缓解 |
|---|---|---|
| `WorkspaceEngine` 体积大（8000+ 行），新增分支易引入回归 | 高 | 严格 TDD；每次改动跑 `cargo test --locked` + `pnpm -C web test`；Story/Design 回归用例必须全绿 |
| WorkItemPlan author/revision 是非流式 provider run，现有 WS handler 默认启动流式 author | 高 | WP2 显式改 `workspace_ws_handler.rs`，新增 dedicated run kind；测试覆盖 StartGeneration 不走 `ProviderRunKind::Author` |
| Draft candidate 同时存在 LifecycleStore 与 artifact payload，可能出现双写不一致 | 高 | LifecycleStore Draft 记录是唯一事实来源；artifact payload 只由当前 Draft 记录组装；测试覆盖 revision 替换后 plan 引用与 payload 一致 |
| `WorkItemSplitEngine::generate_revision` 可能破坏现有 `parse_provider_output` | 中 | 作为独立方法，不改 `generate` 主体；局部重做使用 redo-only parser，整组修正才复用完整 parser；retained 合并 + DAG repatch 的纯函数单测覆盖 |
| `spawn_blocking` 在 WS 循环中无法中途取消 | 中 | 接受"已发起的调用跑完、结果丢弃"；CancellationToken 在结果返回前丢弃路径 |
| review prompt 序列化整组 candidate 可能超 token 上限 | 中 | 序列化裁剪（只保留 reviewer 关心字段） |
| 删除 REST 路由可能漏改前端残留调用 | 低 | 删除前全局搜 `work-items:generate` / `work-item-plans.*confirm` / `change-request`；前端测试覆盖入口 |
| revert 批量后 DAG 重连错误 | 中 | `repatch_dependencies` 纯函数 + 单测覆盖"A 被重做、B 依赖 A"；Validator 的 `validate_dependencies` 兜底 |

## 验收标准

- 从 Design Spec 点"生成下一阶段" → 进入 `ChatWorkspacePage`（不再弹窗）。
- 点"开始生成" → timeline 出现 `AuthorRun` 节点 → 一次性返回 candidate（WorkItem 列表 + DAG + RepositoryProfile + VerificationPlan + findings）。
- AuthorConfirm 阶段每个 WorkItem 可 `[revert]`（附反馈），支持连续标记多个；点"重新生成被标记的" → Revision 产出新 version，被 revert 的 WorkItem 被重做、其余保留、DAG 自动重连。
- 若 `review_rounds > 0`：reviewer 流式审整组，返回 verdict + review decision。
- `HumanConfirm::Confirm` → plan Confirmed + 为每个 WorkItem 建子 Coding session。
- `POST /work-items:generate`、`/work-item-plans/{id}/confirm`、`/change-request` 返回 404（已删除）。
- Story / Design / WorkItem / WorkItemPlan 四种 workspace type 的 timeline/chat/provider 恢复影响评估完成；受共享链路影响的类型回归全绿，不受影响的 WorkItem/Coding 链路给出排除说明。
- `cargo fmt --check` / `cargo clippy --all-targets --all-features --locked -- -D warnings` / `cargo test --locked` / `pnpm -C web test` / `pnpm -C web build` 全绿。
