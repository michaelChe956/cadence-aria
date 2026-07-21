# WorkItem 对话式 Workspace 生成 WP3：后端 review 整组 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `WorkspaceEngine::build_review_input` 在 `WorkspaceType::WorkItemPlan` 分支下调新辅助函数 `build_work_item_plan_review_input`——从当前 Draft `IssueWorkItemPlan` 关联记录组装整组 candidate（plan + work_items + dependency_graph + exclusive_write_scopes + verification_plan_ref + validator_findings + repository_profile 裁剪字段），序列化为 reviewer 可读的中文 review 上下文，复用 `drive_review_session`（流式 Reviewer）+ `parse_review_verdict` + `review_gate_for` + `handle_review_decision`，使 WorkItemPlan 在 `AuthorDecision::Accept` 后走整组流式审查，产出 `ReviewDecision` 响应（continue / continue_with_context / human_intervene）。

**Architecture:** review 是 WorkItemPlan 唯一走流式 provider 的阶段（author/revision 非流式，已在 WP2b 落地）。reviewer 用 `AdapterRole::Reviewer`，经 `drive_review_session`（`workspace_engine.rs:1582`）调 `build_review_input` 构造 prompt。当前 `build_review_input`（`:2470`）对 `ArtifactPayload::WorkItemPlanCandidate` 变体返回空字符串（WP2a 临时处理）——WP3 在此分支调 `build_work_item_plan_review_input` 替代。新函数复用 WP2b 的 `build_work_item_plan_candidate_dto`（从 lifecycle 记录组装完整 `WorkItemPlanCandidateDto`），再按设计方案 :289 裁剪 token：`repository_profile` 只传 `confidence` + `detected_layers`；`WorkItem` 只传 reviewer 关心字段（`id`/`kind`/`title`/`depends_on`/`exclusive_write_scopes`/`verification_plan_ref`，不传 `meta`）；`dependency_graph` 全传；`validator_findings` 全传。verdict 解析、`review_gate`（RequiresRevision / UserConfirmAllowed / UserTriageRequired）、`ReviewDecisionResponse` 全复用，**不改** verdict 解析链路与 WS handler 的 `ReviewOnly` 分发。

**Tech Stack:** Rust 1.95.0、Cargo、tokio、serde。本 WP 不涉及前端，不改 WS 消息变体，不改 `WorkspaceStage`/`TimelineNodeType`/`AdapterRole`。

**版本：** v1.0
**创建日期：** 2026-06-17
**目标分支：** `feat-b-0616`
**工作区：** `.worktrees/feat-b-0616`
**依赖总览：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_拆分总览_v1.0.md`（v1.1，WP3 章节）
**设计方案：** `cadence/designs/2026-06-17_技术方案_WorkItem对话式Workspace生成_v1.0.md`（第 287-292 行 review 阶段、第 223-237 行状态机映射、第 317-348 行 WS 协议）
**前置 WP：** WP1、WP2a、WP2b

---

## 全局约束（Global Constraints）

- **运行命令固定**：Rust 1.95.0；cargo 命令带 `--locked`；🔴 **禁止 `-j 1`**。
- **强制检查链**：`cargo fmt --check` + `cargo clippy --all-targets --all-features --locked -- -D warnings` + `cargo check --locked` + 定向测试，四者缺一不可。
- **TDD**：每个 Task 先写失败测试，再写实现。
- **写入范围严格**：只改「File Structure」声明的文件。本 WP 共享 `src/product/workspace_engine.rs`——须在 WP2b 之后串行执行。
- **行号是参考**：基于 `feat-b-0616` HEAD `8a2eee4`；实现时以 `grep -n` 实际定位为准。
- **不新增 WS 消息变体、不新增 TimelineNodeType、不新增 WorkspaceStage、不新增 AdapterRole**（设计方案 :68-73、:319-325）。reviewer 复用 `AdapterRole::Reviewer`。
- **review 粒度**：整组一次审查（设计方案 :287-291），不是逐个 WorkItem review。
- **review 是流式**（设计方案 :82-84）：复用 `drive_review_session` + `StreamChunk` + `ReviewComplete`，不走非流式 `WorkItemSplitEngine`。

---

## 前置交付摘要（来自 WP1 + WP2a + WP2b）

### 来自 WP1
- `WorkspaceType::WorkItemPlan` 变体（serde `"work_item_plan"`）。
- `prepare_work_item_plan` handler 创建空 Draft `IssueWorkItemPlan`（`work_item_ids`/`verification_plan_ids`/`dependency_graph` 为空）+ `WorkItemPlan` session（`entity_id = plan_id`）。
- `IssueWorkItemPlan` 字段（`models.rs:564-581`）：`id/project_id/issue_id/source_story_spec_ids/source_design_spec_ids/options/status/work_item_ids/repository_profile_ref/verification_plan_ids/dependency_graph/created_from_provider_run/validator_findings/review_summary/created_at/updated_at`。
- `WorkItemPlanCandidateDto` + 子 DTO（`WorkItemPlanDto`/`WorkItemCandidateDto`/`WorkItemCandidateMetaDto`/`WorkItemSplitOptionsDto`/`WorkItemDependencyEdgeDto`/`ValidatorFindingDto`/`VerificationPlanDto`/`RepositoryProfileDto`）已在 `workspace_ws_types.rs` 定义。

### 来自 WP2a
- `ArtifactPayload` enum 已挂载：`WsOutMessage::ArtifactUpdate { version, #[serde(flatten)] payload }`、`SessionState.artifact: Option<ArtifactPayload>`、`EngineEvent::ArtifactUpdate { version, payload }`、`WorkspaceSession.artifact: Option<ArtifactPayload>`、`ArtifactVersion { #[serde(flatten)] payload }`、`CheckpointRecord.artifact_snapshot: ArtifactPayload`。
- `build_review_input`（`:2470`）当前对 `ArtifactPayload::WorkItemPlanCandidate` 变体返回空字符串（WP2a 临时处理，`workspace_engine.rs` 内 `match &self.session.artifact { ... Some(ArtifactPayload::WorkItemPlanCandidate { .. }) => String::new() ... }`）——**WP3 替代此分支**。
- `update_artifact(&mut self, payload: ArtifactPayload)` 签名已切换为 union。

### 来自 WP2b
- WorkItemPlan author run 链路：`StartGeneration`（workspace_type=WorkItemPlan）→ `ProviderRunKind::WorkItemPlanAuthor` → `WorkItemSplitEngine::generate` → `engine.complete_work_item_plan_author`。
- `complete_work_item_plan_author(output: WorkItemSplitProviderOutput) -> Result<WorkItemPlanAuthorOutcome, String>`：validate → has_errors 计数重生（`AutoRevision`）/ warnings 随 candidate → `replace_issue_work_item_plan_candidate` → `build_work_item_plan_candidate_dto` → `update_artifact(ArtifactPayload::WorkItemPlanCandidate)` → `enter_author_confirm`。
- Draft candidate 已落盘：plan（Draft）+ work_items（Draft）+ verification_plans + repository_profile，**无子 WorkItem session**（confirm 时才建，WP5）。
- **`build_work_item_plan_candidate_dto(lifecycle, project_id, issue_id, plan_id) -> Result<WorkItemPlanCandidateDto, ProductStoreError>`**（free function 在 `workspace_engine.rs`）——从 lifecycle 记录组装完整 DTO（事实来源）。**WP3 复用此函数**，再从 DTO 裁剪生成 review prompt。
- `WorkItemPlanAuthorOutcome::{AuthorConfirm, AutoRevision { findings }, HumanConfirm { reason }}`——handler 按此分支；本 WP 不涉及。
- `workspace_type_title` 已加 `WorkItemPlan => "Work Item Plan"` 分支（WP2b Task 2 Step 2.5）。
- `workspace_requires_artifact_gate` 保持不含 WorkItemPlan（WorkItemPlan author 完成靠 `complete_work_item_plan_author` 显式调 `enter_author_confirm`，不靠 `content_has_complete_workspace_artifact`）。
- `ProviderRunContext` 已加 `provider_adapter: Arc<dyn ProviderAdapter + Send + Sync>` 字段（WP2b Task 3 Step 3.3）；`ProviderRunKind::WorkItemPlanAuthor` 已加（WP2b Task 3 Step 3.4）。**本 WP 不再改这些**。
- `session.artifact = Some(ArtifactPayload::WorkItemPlanCandidate { candidate })` 在 AuthorConfirm 阶段已就位（WP2b `complete_work_item_plan_author` 设置）——WP3 的 `build_work_item_plan_review_input` 可直接从 `session.entity_id` 读 lifecycle，或从 `session.artifact` 读 candidate DTO（推荐前者，lifecycle 是事实来源）。

### review 触发链路（已通用，WP3 不改 WS handler）
- `WsInMessage::AuthorDecision { decision: Accept }` → `handle_author_decision_from_handler`（`workspace_ws_handler.rs:811-847`）→ `engine.handle_author_decision(Accept)`（`workspace_engine.rs:2084-2133`）。
- `handle_author_decision` 的 Accept 分支（`:2095-2106`）：检查 `review_rounds > 0 && reviewer_provider.is_some()` → `start_review_or_skip`（`:3069-3106`）→ 进 `CrossReview` 阶段 + 建 `ReviewerRun` timeline 节点 → 返回 `AuthorDecisionOutcome::StartReview`。**与 workspace_type 无关**，WorkItemPlan 自动适配。
- WS handler 收 `StartReview` → `spawn_provider_run_from_handler(ProviderRunKind::ReviewOnly)`（`:822-828`）。
- `ProviderRunKind::ReviewOnly`（`:1297`）→ spawn task 内 `engine.drive_review_session(provider, command_rx)`（`:1427-1431`）。
- `drive_review_session`（`:1582-1618`）调 `build_review_input` 构造 prompt → `provider.start(input, cancel)` → `drive_reviewer_provider_session`。
- reviewer 输出经 `parse_review_verdict`（`:3450`）→ `review_gate_for`（`:3696`）→ 按gate 分支：`UserConfirmAllowed`/`UserTriageRequired` → `enter_human_confirm`；`RequiresRevision` → 进 `ReviewDecision` 阶段 + 发 `EngineEvent::ReviewDecisionRequired`（`:2051-2081`）。
- WS forwarder 转 `WsOutMessage::ReviewDecisionRequired`（`workspace_ws_handler.rs:357-361`）。
- `WsInMessage::ReviewDecisionResponse { decision, extra_context }` → `handle_review_decision_from_handler`（`:783-809`）→ `engine.handle_review_decision`（`:2135-2196`）→ `ReviewDecisionOutcome::{StartRevision, HumanConfirm}`。`StartRevision` → `spawn_provider_run_from_handler(ProviderRunKind::Revision)`（**review 触发的 revision 走 WP4 路径**）。

---

## 关键既有事实（避免重新探查）

所有行号基于 `feat-b-0616` HEAD `8a2eee4`，实现时用 `grep -n` 确认。

### `src/product/workspace_engine.rs`（8362 行）
- `WorkspaceEngine` struct（`:333-347`）：持有 `lifecycle_store: Option<LifecycleStore>`、`event_tx`、`session: WorkspaceSession`、`artifact_versions`、`cancel` 等。**不持有 provider_adapter**（靠参数注入）。
- `WorkspaceSession` struct（`:163-179`）：`workspace_type`/`stage`/`artifact: Option<ArtifactPayload>`（WP2a 后）/`author_provider`/`reviewer_provider`/`review_rounds`/`entity_id`/`project_id`/`issue_id`/`messages` 等。
- `build_review_input(&self) -> Result<StreamingProviderInput, String>`（`:2470-2530`）：
  - `:2477` 读 `self.session.artifact.clone()`（WP2a 后是 `Option<ArtifactPayload>`，当前对 `WorkItemPlanCandidate` 变体返回空字符串）。
  - `:2483-2517` 构造 reviewer prompt：中文说明 + Workspace 类型标题（`workspace_type_title`）+ 会话上下文消息 + artifact markdown + reviewer 契约（JSON verdict 格式）。
  - `:2519-2529` 返回 `StreamingProviderInput { provider_type, role: AdapterRole::Reviewer, prompt, working_dir, ... }`。
  - **WP3 改动点**：在函数开头加 `if self.session.workspace_type == WorkspaceType::WorkItemPlan { return self.build_work_item_plan_review_input(); }`，或重构为内部 match。
- `drive_review_session(&mut self, provider: Arc<dyn StreamingProviderAdapter>, command_rx: mpsc::Receiver<ProviderCommand>)`（`:1582-1618`）：调 `build_review_input` → `provider.start` → `drive_reviewer_provider_session`。**通用，WP3 不改**。
- `parse_review_verdict(output: &str) -> ReviewVerdict`（`:3450-3462`）：用 `extract_tail_json` + `parse_review_json` 解析 reviewer 输出尾部 JSON verdict。失败返回 `NeedsHuman` + `UserTriageRequired`。**纯函数，WP3 复用不改**。
- `review_gate_for(verdict, parsed_findings) -> ReviewGate`（`:3696-3722`）：按 findings severity 判定 `RequiresRevision` / `UserConfirmAllowed` / `UserTriageRequired`。**纯函数，WP3 复用不改**。
- `handle_review_decision(&mut self, decision: String, extra_context: Option<String>) -> Result<ReviewDecisionOutcome, String>`（`:2135-2196`）：处理 `continue`/`continue_with_context`/`human_intervene`。`continue`/`continue_with_context` → `ReviewDecisionOutcome::StartRevision`（进 Revision 阶段）；`human_intervene` → `ReviewDecisionOutcome::HumanConfirm`。**通用，WP3 不改**。
- `handle_author_decision(Accept)`（`:2084-2133`）：`review_enabled = review_rounds > 0 && reviewer_provider.is_some()`；`start_review_or_skip` → 若 `CrossReview` 返回 `StartReview`，否则 `HumanConfirm`。**通用，WorkItemPlan 自动适配**。
- `start_review_or_skip(&mut self)`（`:3069-3106`）：若 `review_rounds == 0 || reviewer_provider.is_none()` → `enter_human_confirm`；否则进 `CrossReview` + 建 `ReviewerRun` timeline 节点。**通用**。
- `enter_author_confirm(&mut self, summary: Option<String>)`（`:3108-3127`）：transition_stage(AuthorConfirm) + 建 AuthorConfirm timeline 节点 + session status = WaitingForHuman。WP2b 已让 WorkItemPlan 走此路径。
- `workspace_type_title(workspace_type: &WorkspaceType) -> &'static str`（`:3740-3746`）：WP2b 已加 `WorkItemPlan => "Work Item Plan"` 分支。
- `update_artifact(&mut self, payload: ArtifactPayload)`（`:2772`，WP2a 已改签名）：WP3 不调用（review 不产生新 artifact）。
- 顶部 `use`（`:1-33`）：已导入 `ArtifactPayload`（WP2a）、`WorkspaceType`、`LifecycleStore`、`ReviewVerdict`/`ReviewGate`/`ReviewVerdictType` 等。WP3 需补 `IssueWorkItemPlan`/`LifecycleWorkItemRecord`/`RepositoryProfile`/`RepositoryProfileConfidence`/`VerificationPlan`/`WorkItemSplitFinding`（部分可能已导入，`cargo check` 确认）。

### `src/product/lifecycle_store.rs`（2054 行）
- `get_issue_work_item_plan(project_id, issue_id, plan_id) -> Result<IssueWorkItemPlan, ProductStoreError>`（`:447-461`）。
- `list_work_items(project_id, issue_id) -> Result<Vec<LifecycleWorkItemRecord>, ProductStoreError>`（`:753` 附近）。
- `get_verification_plan(project_id, issue_id, plan_id) -> Result<VerificationPlan, ProductStoreError>`（`:727-742`）。
- `get_repository_profile(project_id, issue_id, profile_id) -> Result<RepositoryProfile, ProductStoreError>`（`:632-645`）。
- `ProductStoreError`（`json_store.rs:7-17`）。

### `src/product/models.rs`
- `IssueWorkItemPlan`（`:564-581`）：`id`/`project_id`/`issue_id`/`source_story_spec_ids`/`source_design_spec_ids`/`options: IssueWorkItemPlanOptions`/`status: IssueWorkItemPlanStatus`/`work_item_ids: Vec<String>`/`repository_profile_ref: Option<String>`/`verification_plan_ids: Vec<String>`/`dependency_graph: Vec<IssueWorkItemDependencyEdge>`/`created_from_provider_run`/`validator_findings: Vec<WorkItemSplitFinding>`/`review_summary`/`created_at`/`updated_at`。
- `IssueWorkItemDependencyEdge`（`:436-439`）：`{ from_work_item_id, to_work_item_id }`。
- `LifecycleWorkItemRecord`（`:357-398`）：`id`/`kind: WorkItemKind`/`title`/`depends_on: Vec<String>`/`exclusive_write_scopes: Vec<String>`/`verification_plan_ref: Option<String>` 等。reviewer 关心字段：`id`/`kind`/`title`/`depends_on`/`exclusive_write_scopes`/`verification_plan_ref`（不传 `meta`，meta 是 candidate DTO 的概念，lifecycle 记录无此字段——WP2b `build_work_item_plan_candidate_dto` 填 `meta: WorkItemCandidateMetaDto { reverted: false, revert_feedback: None }`，WP3 裁剪时不传 meta）。
- `RepositoryProfile`（`:467-485`）：`confidence: RepositoryProfileConfidence`（`:459-463`，`Low`/`Medium`/`High`）/`detected_layers: Vec<String>`/`languages`/`frameworks`/...。**reviewer 裁剪只传 `confidence` + `detected_layers`**。
- `WorkItemSplitFinding`（`:450-455`）：`{ severity, code, message, work_item_ids }`。**全传**。
- `WorkItemSplitFindingSeverity`（`:443-446`）：`Error`/`Warning`。

### `src/web/workspace_ws_handler.rs`（1553 行）
- `ProviderRunKind::ReviewOnly`（`:1297`）→ spawn task 内 `engine.drive_review_session(provider, command_rx)`（`:1427-1431`）。**通用，WP3 不改**。
- `handle_author_decision_from_handler`（`:811-847`）：`StartReview` → `spawn_provider_run_from_handler(ProviderRunKind::ReviewOnly)`。**通用**。
- `handle_review_decision_from_handler`（`:783-809`）：`StartRevision` → `spawn_provider_run_from_handler(ProviderRunKind::Revision)`。**WP4 会改 Revision 分发为 WorkItemPlanRevision**，WP3 不改。
- event forwarder（`:248-392`）：`EngineEvent::ReviewComplete` → `WsOutMessage::ReviewComplete`（`:340-348`）；`EngineEvent::ReviewDecisionRequired` → `WsOutMessage::ReviewDecisionRequired`（`:357-361`）；`EngineEvent::StreamChunk` → `WsOutMessage::StreamChunk`（`:251-256`）。**通用，WP3 不改**。

### `src/product/work_item_split_engine.rs`（747 行）
- `WorkItemSplitProviderOutput { repository_profile, plan, work_items, verification_plans }`（`:133-139`）。WP3 不调用此引擎（review 是流式 Reviewer，不是 WorkItemSplitter）。

### `tests/it_web/web_work_item_generation.rs`
- `MockSplitProviderAdapter`（`:15`，`ProviderAdapter` trait impl，非流式，WP2b 用）。
- `valid_split_output() -> Value`（`:33`）。
- `app_with_confirmed_story_and_design(output: Value) -> (axum::Router, tempfile::TempDir)`（`:416`）：建 app + project/repo/issue/story_spec/design_spec，注入 `MockSplitProviderAdapter`。WP3 测试复用此夹具（或其变体，补 prepare_work_item_plan + start_generation 进 AuthorConfirm）。

### `src/product/workspace_engine.rs` 的 `#[cfg(test)] mod tests`
- `ReviewVerdictStreamingProvider`（`:5757-5806`）：`StreamingProviderAdapter` impl，录制 prompt + 返回固定 verdict 字符串。**WP3 测试直接复用**。
- `FakeStreamingProvider`（`:4351` 引用）：通用 fake。
- `make_session(session_id)`（`:4375-4393`）：建 `WorkspaceType::Story` session。WP3 需建 `WorkspaceType::WorkItemPlan` session——参考此 helper 写 `make_work_item_plan_session`。
- `setup() -> (TempDir, Arc<CheckpointStore>)`（`:4369-4373`）。
- `empty_provider_commands() -> mpsc::Receiver<ProviderCommand>`（`:4395-4398`）。
- `drive_review_session_pass_enters_human_confirm`（`:5809-5880`）：参考此测试的 review 流程断言模式。

---

## File Structure

| 文件 | 操作 | 职责 / 本 WP 改动 |
|---|---|---|
| `src/product/workspace_engine.rs` | M | `build_review_input` 加 WorkItemPlan 分支（调 `build_work_item_plan_review_input`）；新增 `build_work_item_plan_review_input(&self) -> Result<StreamingProviderInput, String>` 辅助方法（从 `session.entity_id` 读 Draft plan + lifecycle 记录，复用 `build_work_item_plan_candidate_dto`，裁剪序列化为 review prompt，复用 reviewer 契约尾部）；顶部 `use` 补 `IssueWorkItemPlan`/`LifecycleWorkItemRecord`/`RepositoryProfile`/`RepositoryProfileConfidence`/`VerificationPlan`/`WorkItemSplitFinding`（以 `cargo check` 实际缺失为准） |
| `tests/it_web.rs` | M | 若新增 `web_work_item_plan_review.rs` mod，加 `#[path]` 注册 |
| `tests/it_web/web_work_item_plan_review.rs` | N | WP3 集成测试：prepare → start_generation → author → AuthorDecision::Accept → review → ReviewDecisionRequired → ReviewDecisionResponse（continue / continue_with_context / human_intervene） |

**不改：**
- ❌ `src/web/workspace_ws_handler.rs`（`ReviewOnly`/`ReviewDecisionResponse`/`AuthorDecision` 分发全通用，WorkItemPlan 自动适配）
- ❌ `src/web/workspace_ws_types.rs`（WP1/WP2a 已完成；不新增 WS 消息变体）
- ❌ `src/product/lifecycle_store.rs`（WP2b 已加 `replace_issue_work_item_plan_candidate`，WP3 只读不写）
- ❌ `src/product/work_item_split_engine.rs`（review 不用 WorkItemSplitter）
- ❌ `src/product/work_item_split_validator.rs`（review 不调 validate）
- ❌ `src/web/handlers.rs` / `app.rs` / `workspace_context.rs`（WP1 已完成）
- ❌ `src/product/models.rs`（不新增枚举/字段）
- ❌ 前端（WP6/WP7）

---

## Task 1：`build_work_item_plan_review_input` + `build_review_input` WorkItemPlan 分支

**目标**：在 `WorkspaceEngine` 上新增 `build_work_item_plan_review_input(&self) -> Result<StreamingProviderInput, String>`——从 `session.entity_id`（plan_id）+ `lifecycle_store` 读 Draft candidate 记录，复用 WP2b 的 `build_work_item_plan_candidate_dto` 组装完整 DTO，再按设计方案 :289 裁剪 token 序列化为中文 review prompt（`repository_profile` 只传 `confidence` + `detected_layers`；WorkItem 只传 `id`/`kind`/`title`/`depends_on`/`exclusive_write_scopes`/`verification_plan_ref`，不传 `meta`；`dependency_graph` 全传；`validator_findings` 全传），尾部追加与现有 `build_review_input` 相同的 reviewer JSON verdict 契约说明。`build_review_input` 在 `workspace_type == WorkItemPlan` 时调此新方法，替代 WP2a 的空字符串分支。

**Files:**
- Modify: `src/product/workspace_engine.rs`
- Test: `src/product/workspace_engine.rs` 的 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: WP2b 的 `build_work_item_plan_candidate_dto(lifecycle, project_id, issue_id, plan_id) -> Result<WorkItemPlanCandidateDto, ProductStoreError>`（free function）；`lifecycle_store.get_issue_work_item_plan`/`list_work_items`/`get_verification_plan`/`get_repository_profile`（经 `build_work_item_plan_candidate_dto` 间接调）；`session.entity_id`/`project_id`/`issue_id`/`reviewer_provider`/`repository_path`；`StreamingProviderInput`/`AdapterRole::Reviewer`/`provider_type_for_name`/`ProviderPermissionMode`/`DEFAULT_PROVIDER_TIMEOUT_SECS`（现有 `build_review_input` 已用）。
- Produces:
  - `WorkspaceEngine::build_work_item_plan_review_input(&self) -> Result<StreamingProviderInput, String>`（关联方法，`&self`，返回 Reviewer StreamingProviderInput）。
  - `build_review_input` 的 WorkItemPlan 分支路由（函数内 `if matches!(self.session.workspace_type, WorkspaceType::WorkItemPlan) { return self.build_work_item_plan_review_input(); }`）。

- [ ] **Step 1.1：写失败测试 —— build_work_item_plan_review_input 返回含裁剪 candidate 的 Reviewer 输入**

在 `src/product/workspace_engine.rs` 的 `#[cfg(test)] mod tests` 末尾追加。参考 `drive_review_session_pass_enters_human_confirm`（`:5809-5880`）的夹具模式 + WP2b 的 WorkItemPlan session 构造。

```rust
    #[test]
    fn build_work_item_plan_review_input_includes_trimmed_candidate_fields() {
        let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
            make_work_item_plan_engine_with_draft_candidate("sess_wip_review_prompt");
        // make_work_item_plan_engine_with_draft_candidate 见下方 helper：
        //   建 LifecycleStore + Draft IssueWorkItemPlan + 2 个 Draft LifecycleWorkItemRecord
        //   + 1 VerificationPlan + 1 RepositoryProfile（含 confidence=High, detected_layers=["backend","frontend"]）
        //   + WorkItemPlan session（entity_id=plan_id, reviewer_provider=Codex, review_rounds=1）
        //   + engine 已在 AuthorConfirm 阶段（session.artifact = WorkItemPlanCandidate）

        let input = engine
            .build_work_item_plan_review_input()
            .expect("review input");

        // provider_type 来自 reviewer_provider (Codex)
        assert_eq!(input.role, AdapterRole::Reviewer);
        // prompt 含 plan 概要
        assert!(input.prompt.contains("Work Item Plan"), "prompt 应含 workspace 类型标题");
        // prompt 含每个 work_item 的 reviewer 关心字段
        assert!(input.prompt.contains("work_item_0001"));
        assert!(input.prompt.contains("work_item_0002"));
        assert!(input.prompt.contains("depends_on"));
        assert!(input.prompt.contains("exclusive_write_scopes"));
        assert!(input.prompt.contains("verification_plan_ref"));
        // prompt 含 dependency_graph
        assert!(input.prompt.contains("dependency_graph"));
        // prompt 含 validator_findings（若有）
        // prompt 含 repository_profile 裁剪字段：confidence + detected_layers
        assert!(input.prompt.contains("high") || input.prompt.contains("High"), "prompt 应含 confidence");
        assert!(input.prompt.contains("backend"));
        // prompt 不应含 repository_profile 的非 reviewer 字段（如 languages/frameworks 详细列表）
        // （软断言：prompt 不含 "frameworks" heading——以实际裁剪实现为准）
        // prompt 含 reviewer JSON verdict 契约（复用现有 build_review_input 尾部）
        assert!(input.prompt.contains("\"verdict\":\"pass|revise|needs_human\""));
        assert!(input.prompt.contains("\"summary\""));
        assert!(input.prompt.contains("\"findings\""));
    }

    #[test]
    fn build_review_input_routes_work_item_plan_to_dedicated_helper() {
        // 同上夹具，但调 build_review_input（而非 build_work_item_plan_review_input）
        let (_tmp, _checkpoint_store, _lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_draft_candidate("sess_wip_review_route");

        let input = engine.build_review_input().expect("review input");

        assert_eq!(input.role, AdapterRole::Reviewer);
        // 验证走的是 WorkItemPlan 分支（prompt 含 candidate 字段，而非 Story/Design 的 artifact markdown）
        assert!(input.prompt.contains("work_item_0001"));
        assert!(!input.prompt.contains("当前已提取 Artifact Markdown"));
    }

    /// helper：构造 WorkItemPlan engine + Draft candidate 落盘 + session 在 AuthorConfirm。
    /// 参考 make_session（:4375）+ WP2b 的 complete_work_item_plan_author 测试夹具。
    fn make_work_item_plan_engine_with_draft_candidate(
        session_id: &str,
    ) -> (
        TempDir,
        Arc<CheckpointStore>,
        LifecycleStore,
        String,
        WorkspaceEngine,
    ) {
        let tmp = TempDir::new().unwrap();
        let app_paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let lifecycle = LifecycleStore::new(app_paths.clone());
        // 建 project/repo/issue/story_spec/design_spec（参考 app_with_confirmed_story_and_design :416 的 HTTP 流程，
        // 但这里是 engine 层测试，直接调 lifecycle.create_* 原语）
        // ... 建 repository/repository_profile/issue/story_spec/design_spec/plan/work_items/verification_plan ...
        let plan_id = "plan_0001".to_string();
        // ... 构造 Draft IssueWorkItemPlan + 2 work_items + 1 verification_plan + 1 repository_profile ...
        // ... 建 WorkItemPlan session（entity_id=plan_id, reviewer_provider=Codex, review_rounds=1）...
        let checkpoint_store = Arc::new(CheckpointStore::new(tmp.path().to_path_buf()));
        let (event_tx, _event_rx) = mpsc::channel(64);
        let session = WorkspaceSession {
            session_id: session_id.to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: plan_id.clone(),
            workspace_type: WorkspaceType::WorkItemPlan,
            stage: WorkspaceStage::AuthorConfirm,
            messages: Vec::new(),
            artifact: Some(ArtifactPayload::WorkItemPlanCandidate {
                candidate: build_work_item_plan_candidate_dto(
                    &lifecycle, "project_0001", "issue_0001", &plan_id,
                ).expect("candidate dto"),
            }),
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: Some(ProviderName::Codex),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            provider_conversations: Vec::new(),
            repository_path: None,
        };
        let engine = WorkspaceEngine::new(checkpoint_store.clone(), event_tx, session);
        // engine.lifecycle_store 需 Some(lifecycle)——参考 new_persistent（:470）或 WP2b 测试夹具的 engine 构造方式
        (tmp, checkpoint_store, lifecycle, plan_id, engine)
    }
```

> 实现者注意：
> 1. `make_work_item_plan_engine_with_draft_candidate` 是本 Task 最繁的夹具——建 LifecycleStore + 一整套 Draft 记录 + WorkItemPlan session + engine。参考 WP2b 的 `complete_work_item_plan_author_pushes_candidate_and_enters_author_confirm` 测试夹具（WP2b plan Task 2 Step 2.1）。若 WP2b 已抽了类似 helper，直接复用。
> 2. `WorkspaceEngine::new(checkpoint_store, event_tx, session)` 的签名以实际为准——`grep -n "fn new\b" src/product/workspace_engine.rs`。若 `new` 不接 `lifecycle_store`，需用 `new_persistent`（`:470`）或手动设 `engine.lifecycle_store = Some(lifecycle)`（若字段 pub）。参考 WP2b 测试夹具的 engine 构造方式。
> 3. `build_work_item_plan_candidate_dto` 是 WP2b 的 free function——若 WP2b 把它放在 `workspace_engine.rs` 顶层，test mod 可直接调（`use super::*` 已覆盖）。若 WP2b 放在别处，调整 `use`。
> 4. `ProductAppPaths`/`LifecycleStore::new` 的构造以实际为准——`grep -n "fn new" src/product/lifecycle_store.rs src/product/app_paths.rs`。
> 5. `RepositoryProfileConfidence::High` 的 serde 值是 `"high"`——prompt 里可断言 `"high"`（serde rename_all snake_case）或 `format!("{:?}", High).to_lowercase()`。以实际裁剪实现为准。

- [ ] **Step 1.2：运行测试，确认失败**

Run: `cargo test --locked --lib build_work_item_plan_review_input build_review_input_routes_work_item_plan`
Expected: 编译失败——`build_work_item_plan_review_input` 未定义；`make_work_item_plan_engine_with_draft_candidate` 未定义。

- [ ] **Step 1.3：实现 `build_work_item_plan_review_input`**

在 `src/product/workspace_engine.rs`，紧邻 `build_review_input`（`:2470`）之后（`:2530` 附近，`build_revision_input` 之前）插入新关联方法。先 `grep -n "fn build_review_input\|fn build_revision_input" src/product/workspace_engine.rs` 确认插入点。

```rust
    fn build_work_item_plan_review_input(&self) -> Result<StreamingProviderInput, String> {
        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable for work_item_plan review".to_string())?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let candidate = build_work_item_plan_candidate_dto(lifecycle, &project_id, &issue_id, &plan_id)
            .map_err(|error| format!("build work_item_plan candidate dto failed: {error}"))?;

        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };
        let provider = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);

        let mut prompt = String::new();
        prompt.push_str("请作为 reviewer 审核当前 WorkItemPlan 候选（整组 WorkItem 拆分计划）。\n\n");
        prompt.push_str(&format!(
            "Workspace 类型: {}\n",
            workspace_type_title(&self.session.workspace_type)
        ));
        prompt.push_str("会话上下文:\n");
        for msg in &self.session.messages {
            if matches!(msg.role.as_str(), "assistant" | "provider") {
                continue;
            }
            prompt.push_str(&format!("[{}]: {}\n", msg.role, msg.content));
        }
        self.append_missing_context_notes_to_prompt(&mut prompt);

        // === Plan 概要 ===
        prompt.push_str("\n## 待审核候选\n\n");
        prompt.push_str(&format!("### Plan\n- id: {}\n- status: {}\n", candidate.plan.id, candidate.plan.status));
        prompt.push_str(&format!(
            "- options: include_integration_tests={}, include_e2e_tests={}, force_frontend_backend_split={}, require_execution_plan_confirm={}\n",
            candidate.plan.options.include_integration_tests,
            candidate.plan.options.include_e2e_tests,
            candidate.plan.options.force_frontend_backend_split,
            candidate.plan.options.require_execution_plan_confirm,
        ));

        // === WorkItem 列表（裁剪：只传 reviewer 关心字段，不传 meta） ===
        prompt.push_str("\n### WorkItems\n");
        for wi in &candidate.work_items {
            prompt.push_str(&format!(
                "\n- id: {}\n  kind: {}\n  title: {}\n  depends_on: [{}]\n  exclusive_write_scopes: [{}]\n  verification_plan_ref: {}\n",
                wi.id,
                wi.kind,
                wi.title,
                wi.depends_on.join(", "),
                wi.exclusive_write_scopes.join(", "),
                wi.verification_plan_ref.as_deref().unwrap_or("(none)"),
            ));
        }

        // === Dependency Graph（全传） ===
        prompt.push_str("\n### Dependency Graph\n");
        if candidate.plan.dependency_graph.is_empty() {
            prompt.push_str("(empty)\n");
        } else {
            for edge in &candidate.plan.dependency_graph {
                prompt.push_str(&format!(
                    "- {} -> {}\n",
                    edge.from_work_item_id, edge.to_work_item_id
                ));
            }
        }

        // === Validator Findings（全传） ===
        prompt.push_str("\n### Validator Findings\n");
        if candidate.validator_findings.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for finding in &candidate.validator_findings {
                prompt.push_str(&format!(
                    "- [{}] {}: {} (work_items: [{}])\n",
                    finding.severity,
                    finding.code,
                    finding.message,
                    finding.work_item_ids.join(", "),
                ));
            }
        }

        // === Repository Profile（裁剪：只传 confidence + detected_layers） ===
        prompt.push_str("\n### Repository Profile (trimmed)\n");
        if let Some(rp) = &candidate.repository_profile {
            prompt.push_str(&format!(
                "- confidence: {}\n- detected_layers: [{}]\n",
                rp.confidence,
                rp.detected_layers.join(", "),
            ));
        } else {
            prompt.push_str("(none)\n");
        }

        // === Verification Plans（摘要，不展开 commands） ===
        prompt.push_str("\n### Verification Plans (summary)\n");
        if candidate.verification_plans.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for vp in &candidate.verification_plans {
                prompt.push_str(&format!(
                    "- id: {} | work_item_id: {} | scope: {} | commands: {} | manual_checks: {}\n",
                    vp.id,
                    vp.work_item_id,
                    vp.scope,
                    vp.commands.len(),
                    vp.manual_checks.len(),
                ));
            }
        }

        // === Reviewer 契约（与 build_review_input 尾部一致） ===
        prompt.push_str(
            "\n\n审核边界说明：本候选是 WorkItemPlan 整组拆分计划，请从以下维度评估：\
             1) 拆分粒度合理性（是否过粗或过细）；\
             2) 依赖完整性（DAG 是否无环、depends_on 指向存在的 work_item）；\
             3) 写入范围互斥（exclusive_write_scopes 之间无重叠）；\
             4) 跨端拆分恰当性（前端/后端/全栈划分是否合理）；\
             5) 验证计划覆盖度（每个 work_item 的 verification_plan_ref 是否存在、scope 是否匹配）。\
             不要因为 verification_plans 摘要未展开 commands 判定返修；只审核上述五个维度。\n",
        );
        prompt.push_str(
            "\n\n请输出审核意见，并在末尾附加 JSON 代码块：\n\
             - 只有影响下一阶段可用性的 finding 才能标记为 `blocking`、`must_fix` 或 `strong_recommend_fix`。\n\
             - 风格、措辞、文档美化、未来扩展、非必要补充只能标记为 `suggestion`、`minor` 或 `optional`。\n\
             - 没有强返修 finding 时，必须允许用户确认当前版本，不要为了普通建议使用强返修。\n\
             - 如果输出 `verdict=revise`，必须给出至少一个结构化 finding；否则系统会进入人工裁决而不是自动返修。\n\
             - 第二轮及后续 review 只复核上一轮强返修项是否关闭；除非 revision 新引入真正阻塞问题，不得重新发散普通建议。\n\
             - `pass`：产物可进入最终人工确认。\n\
             - `revise`：仅当存在 blocking/must_fix/strong_recommend_fix finding。\n\
             - `needs_human`：没有明确可自动返修内容，需要用户做产品/范围判断。\n\
             ```json\n\
             {\"verdict\":\"pass|revise|needs_human\",\"summary\":\"一句话摘要\",\"findings\":[{\"severity\":\"blocking|must_fix|strong_recommend_fix|suggestion|minor|optional\",\"message\":\"问题描述\",\"evidence\":\"当前产物中的具体证据\",\"impact\":\"为什么影响或不影响下一阶段\",\"required_action\":\"需要作者执行的最小动作\"}]}\n\
             ```\n",
        );

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }
```

> 实现者注意：
> 1. `candidate.repository_profile` 的字段名以 WP1 实际定义的 `RepositoryProfileDto` 为准——`grep -n "struct RepositoryProfileDto" src/web/workspace_ws_types.rs`。若 `RepositoryProfileDto` 的 `confidence` 字段是 `String`（serde 后），直接用；若是 enum，`format!("{:?}", ...).to_lowercase()`。`detected_layers` 应为 `Vec<String>`。
> 2. `candidate.verification_plans` 的字段名以 WP1 实际定义的 `VerificationPlanDto` 为准——`grep -n "struct VerificationPlanDto" src/web/workspace_ws_types.rs`。`id`/`work_item_id`/`scope`/`commands`/`manual_checks` 应存在；若字段名不同，按实际调整。
> 3. `candidate.work_items[i].verification_plan_ref` 类型是 `Option<String>`——用 `as_deref().unwrap_or("(none)")`。
> 4. reviewer 契约尾部与 `build_review_input`（`:2504-2517`）完全一致——直接复制，保证 verdict 解析（`parse_review_verdict`）能识别。**不要修改 JSON 格式**。
> 5. `provider_type_for_name`/`AdapterRole::Reviewer`/`ProviderPermissionMode::Supervised`/`DEFAULT_PROVIDER_TIMEOUT_SECS` 均在 `build_review_input` 中已用，`use` 已覆盖。
> 6. `build_work_item_plan_candidate_dto` 是 free function（WP2b），在 `impl WorkspaceEngine` 块外定义——关联方法内直接调（同模块可见）。

- [ ] **Step 1.4：`build_review_input` 加 WorkItemPlan 分支路由**

`src/product/workspace_engine.rs:2470-2477`（`build_review_input` 函数体开头），在读 `session.artifact` 之前加分支：

```rust
    fn build_review_input(&self) -> Result<StreamingProviderInput, String> {
        if matches!(self.session.workspace_type, WorkspaceType::WorkItemPlan) {
            return self.build_work_item_plan_review_input();
        }

        let working_dir = match &self.session.repository_path {
            // ... 现有代码不变 ...
        };
        // ... 现有 build_review_input 主体不变 ...
    }
```

> ⚠️ WP2a 在 `build_review_input` 内对 `session.artifact` 的 `WorkItemPlanCandidate` 变体返回空字符串——这是 Story/Design 分支内对 `session.artifact` 的 match 处理。WP3 的 WorkItemPlan 分支在函数开头提前 return，不会走到那个 match。**WP2a 的空字符串分支可保留**（作为 Story/Design 误收 WorkItemPlanCandidate 的兜底，实际不会触发），也可在 WP3 实现时清理——**建议保留**，避免越界改 WP2a 已测试的代码。`grep -n "WorkItemPlanCandidate" src/product/workspace_engine.rs` 确认 WP2a 的空字符串分支位置。

- [ ] **Step 1.5：顶部 `use` 补全**

`src/product/workspace_engine.rs:1-33`，补 `IssueWorkItemPlan`/`LifecycleWorkItemRecord`/`RepositoryProfile`/`RepositoryProfileConfidence`/`VerificationPlan`/`WorkItemSplitFinding`（`cargo check` 确认哪些实际缺失）。这些在 `use crate::product::models::{...}` 块内（`:20-25`）补。

> 若 `build_work_item_plan_review_input` 只经 `build_work_item_plan_candidate_dto` 间接读 lifecycle 记录（不直接 `use` 这些类型），则**无需补**——`cargo check` 会告诉你。优先写实现再根据 `cargo check` 补 `use`，避免无用导入（clippy 会警告）。

- [ ] **Step 1.6：运行 Task 1 测试 + 收口**

Run:
```
cargo test --locked --lib build_work_item_plan_review_input
cargo test --locked --lib build_review_input_routes_work_item_plan
cargo test --locked --lib workspace_engine
cargo check --locked
```
Expected:
- 两个新测试 PASS。
- 现有 `workspace_engine` 测试全绿（Story/Design/WorkItem 的 `build_review_input` 行为未变——WorkItemPlan 分支提前 return，其他 workspace_type 走原逻辑）。
- `cargo check` 全绿。

> 若现有 `drive_review_session_pass_enters_human_confirm`（`:5809`）等 Story 测试失败，说明 WorkItemPlan 分支误伤 Story——检查 `matches!(self.session.workspace_type, WorkspaceType::WorkItemPlan)` 条件是否正确（应为 WorkItemPlan 专属，Story/Design/WorkItem 不匹配）。

- [ ] **Step 1.7：提交**

```bash
git add src/product/workspace_engine.rs
git commit -m "feat(WP3): build_work_item_plan_review_input + build_review_input WorkItemPlan 分支"
```

---

## Task 2：review WS 触发与 verdict 决策（集成测试）

**目标**：验证 WorkItemPlan 的 review 流程在 WS handler 层正确触发 `ProviderRunKind::ReviewOnly` → `drive_review_session` → `build_work_item_plan_review_input`，reviewer 输出 verdict 后正确进入 `ReviewDecision` 阶段并发 `WsOutMessage::ReviewDecisionRequired`，前端发 `ReviewDecisionResponse`（continue / continue_with_context / human_intervene）后 `handle_review_decision` 正确返回 outcome（`StartRevision` / `HumanConfirm`）。**本 WP 边界：只验证到 `ReviewDecisionResponse` 响应返回，不验证完整 revision 重做**（review 触发的 revision 走 WP4 的 `WorkItemPlanRevision` 路径，WP4 才实现；完整 revision 重做在 WP8 贯通测试验证）。

**Files:**
- Modify: `tests/it_web.rs`（若新增 `web_work_item_plan_review.rs` mod，加 `#[path]` 注册）
- Test: `tests/it_web/web_work_item_plan_review.rs`（新增）

**Interfaces:**
- Consumes: Task 1 的 `build_work_item_plan_review_input`；WP2b 的 `complete_work_item_plan_author`（让 engine 进 AuthorConfirm）；WP2b 的 `ProviderRunKind::WorkItemPlanAuthor`（prepare → start_generation → author）；现有 WS handler 的 `ReviewOnly`/`ReviewDecisionResponse` 分发（不改）；现有 `ReviewVerdictStreamingProvider`（engine 测试夹具，`:5757`，可作为 WS 层 reviewer provider 的 mock）。
- Produces: WP3 集成测试，验证 `review_returns_verdict_for_whole_candidate` + `work_item_plan_review_returns_decision_response`（建议总览 WP3 的 `work_item_plan_review_revision_loop` 调整为此名——见 Self-Review）。

- [ ] **Step 2.1：写失败测试 —— review_returns_verdict_for_whole_candidate**

在 `tests/it_web/web_work_item_plan_review.rs`（新增）。复用 `app_with_confirmed_story_and_design`（`web_work_item_generation.rs:416`）+ WP2b 的 WorkItemPlan author 链路（prepare → start_generation → AuthorConfirm），再触发 `AuthorDecision::Accept` 进入 review。

```rust
use super::*;
use crate::it_web::web_work_item_generation::app_with_confirmed_story_and_design;
use crate::it_web::web_work_item_generation::valid_split_output;
// 其他 use 以实际测试模块惯例为准

#[tokio::test]
async fn review_returns_verdict_for_whole_candidate() {
    // 1. 建带 fake split provider 的 app + Draft plan + WorkItemPlan session（在 AuthorConfirm 阶段）
    //    复用 WP2b 的 prepare → start_generation 链路（若 WP2b 已有 helper，直接调）
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;
    let session_id = prepare_work_item_plan_and_author_to_confirm(&app).await;
    // prepare_work_item_plan_and_author_to_confirm 见下方 helper：
    //   POST /api/projects/.../work-item-plans:prepare → 连 WS → send StartGeneration
    //   → 收 ArtifactUpdate(candidate) → 收 StageChange(author_confirm)
    //   返回 session_id

    // 2. 连 WS，发 AuthorDecision::Accept → 进 CrossReview → 收 ReviewComplete（含 verdict）
    let ws = connect_ws(&app, &session_id).await;
    ws.send(json!({ "type": "author_decision", "decision": "accept" }).to_string()).await;

    let messages = recv_ws_messages(&ws, timeout).await;
    // 收 StageChange → cross_review
    let stage_cross = messages.iter().find(|m| m["type"] == "stage_change" && m["stage"] == "cross_review")
        .expect("stage_change cross_review");
    // 收 StreamChunk（reviewer 流式意见）——至少一条
    let _stream = messages.iter().find(|m| m["type"] == "stream_chunk").expect("stream_chunk");
    // 收 ReviewComplete（含 verdict）
    let review_complete = messages.iter().find(|m| m["type"] == "review_complete").expect("review_complete");
    assert!(review_complete["verdict"].is_string());
    assert!(review_complete["summary"].is_string());

    // 3. 若 verdict 是 revise（RequiresRevision）→ 收 ReviewDecisionRequired
    //    若 verdict 是 pass（UserConfirmAllowed）→ 收 StageChange → human_confirm
    let verdict = review_complete["verdict"].as_str().unwrap();
    if verdict == "revise" {
        let decision_required = messages.iter().find(|m| m["type"] == "review_decision_required")
            .expect("review_decision_required for revise verdict");
        assert!(decision_required["options"].is_array());
        assert!(decision_required["options"].as_array().unwrap().contains(&json!("continue")));
        assert!(decision_required["options"].as_array().unwrap().contains(&json!("continue_with_context")));
        assert!(decision_required["options"].as_array().unwrap().contains(&json!("human_intervene")));
    } else {
        let stage_human = messages.iter().find(|m| m["type"] == "stage_change" && m["stage"] == "human_confirm")
            .expect("stage_change human_confirm for pass verdict");
    }
}

#[tokio::test]
async fn work_item_plan_review_returns_decision_response() {
    // 1. 同上，进 ReviewDecision 阶段（verdict=revise）
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;
    let session_id = prepare_work_item_plan_and_author_to_confirm(&app).await;
    let ws = connect_ws(&app, &session_id).await;
    ws.send(json!({ "type": "author_decision", "decision": "accept" }).to_string()).await;
    let _messages = recv_ws_messages(&ws, timeout).await;
    // 等到收到 ReviewDecisionRequired（verdict=revise 路径）
    // ... 收到 review_decision_required ...

    // 2. 发 ReviewDecisionResponse { decision: "human_intervene" } → 收 StageChange → human_confirm
    ws.send(json!({ "type": "review_decision_response", "decision": "human_intervene", "extra_context": null }).to_string()).await;
    let messages = recv_ws_messages(&ws, timeout).await;
    let stage_human = messages.iter().find(|m| m["type"] == "stage_change" && m["stage"] == "human_confirm")
        .expect("stage_change human_confirm after human_intervene");

    // 3. 另一场景：发 ReviewDecisionResponse { decision: "continue" } → 收 StageChange → revision
    //    （本 WP 边界：只验证 stage 进入 revision，不验证 revision 执行——revision 执行是 WP4）
    //    注意：WP4 未实现时，Revision 分发会走普通 drive_revision_session（Story/Design 路径），
    //    对 WorkItemPlan 会失败（build_revision_input 对 WorkItemPlanCandidate 返回空 artifact）。
    //    因此本测试只断言 stage_change → revision，不断言 revision 成功执行。
    // ... 另起 session 测试 continue 路径 ...
}
```

> 实现者注意：
> 1. **WS 连接 helper**（`connect_ws`/`recv_ws_messages`）：复用 WP2b Task 3 新增的共享 helper；若 helper 还不完整，先在测试基础设施中补齐，再写本测试。Task 1 已覆盖 engine 层 prompt 构造，本 Task 2 的增量必须覆盖 WS 层 AuthorDecision → ReviewOnly run → ReviewComplete/ReviewDecisionResponse。
> 2. **`prepare_work_item_plan_and_author_to_confirm` helper**：本 Task 最繁的夹具。需先 POST `/api/projects/.../work-item-plans:prepare`（WP1 路由）→ 连 WS → `sendStartGeneration`（WP2b `ProviderRunKind::WorkItemPlanAuthor`）→ 收 `ArtifactUpdate(candidate)` → 收 `StageChange(author_confirm)`。若 WP2b 测试已有类似 helper，直接复用（`grep -rn "prepare_work_item_plan\|work_item_plan_start_generation" tests/it_web/`）。
> 3. **reviewer provider mock**：WS 层 reviewer 走 `provider_registry.get(reviewer_provider)`（`workspace_ws_handler.rs:1368-1372`）——需要 app 注入了 reviewer provider（如 FakeStreamingProvider 或能返回固定 verdict 的 mock）。参考 `app_with_confirmed_story_and_design` 的 `with_provider_adapter`（`web_work_item_generation.rs:431`）——但那是非流式 split adapter；reviewer 需要流式。**先 `grep -rn "with_streaming_provider\|with_reviewer\|provider_registry\|FakeStreaming" tests/it_web/`** 看 WS 测试如何注入流式 reviewer。若无现成，可能需在 `WebAppState::new` 时注入 `provider_registry` 含 FakeStreamingProvider（`ProviderName::Codex` 或 `Fake`）。
> 4. **测试边界**：`work_item_plan_review_returns_decision_response` 的 `continue` 路径只断言 `stage_change → revision`，不断言 revision 执行成功——WP4 未实现时 `drive_revision_session` 对 WorkItemPlan 会失败（`build_revision_input` 读 `session.artifact` 的 `WorkItemPlanCandidate` 变体返回空字符串，prompt 为空），但 stage 转换在 `handle_review_decision` 内已完成（`:2167-2181`）——测试只收 `StageChange` 消息即可。若 WS handler 在 revision spawn 失败后发 `Error` 消息，测试应容忍（不 fail on Error，只断言 stage_change 先到）。
> 5. **总览 WP3 验证条目调整建议**：总览 v1.1 WP3 章节列了 `work_item_plan_review_revision_loop`——此名暗示验证完整 revision 重做循环，但 WP3 边界只到 `ReviewDecisionResponse` 响应。建议调整为 `work_item_plan_review_returns_decision_response`（本 plan 已用此名）。**不要自行修改总览文件**——由主控决定。本 plan 在 Self-Review 标注此调整建议。

- [ ] **Step 2.2：运行测试，确认失败**

Run: `cargo test --locked --test it_web review_returns_verdict_for_whole_candidate work_item_plan_review_returns_decision_response`
Expected: 失败——WS helper 或 reviewer provider mock 未就位，或 WorkItemPlan 的 AuthorDecision::Accept 仍没有路由到 reviewer 流式 run。本 Task 2 主要是 WS 层验证。

> Task 1 的 engine 层测试已覆盖 review prompt 构造，Task 2 的核心增量是 **WS 层端到端**：AuthorDecision::Accept → ReviewOnly run kind → drive_review_session → ReviewComplete → ReviewDecisionRequired → ReviewDecisionResponse → stage 转换。不要用 engine 层组合测试替代本 Task；若失败，补 WS helper 或 reviewer provider mock。

- [ ] **Step 2.3：补 helper（若用 WS 集成测试）**

在 `tests/it_web/web_work_item_plan_review.rs` 或 `tests/it_web/web_work_item_generation.rs` 补：
- `prepare_work_item_plan_and_author_to_confirm(app) -> String`（返回 session_id）：POST prepare → 连 WS → send StartGeneration → 收 ArtifactUpdate + StageChange(author_confirm)。
- 若需流式 reviewer mock，补 `MockReviewerStreamingProvider`（参考 engine 测试的 `ReviewVerdictStreamingProvider`，`:5757`，返回固定 verdict 字符串）。注入方式以 `WebAppState` 的 `provider_registry` API 为准——`grep -rn "provider_registry\|with_provider\|ProviderRegistry" src/web/state.rs tests/it_web/`。

- [ ] **Step 2.4：运行 Task 2 测试 + 收口**

Run:
```
cargo test --locked --test it_web review_returns_verdict_for_whole_candidate
cargo test --locked --test it_web work_item_plan_review_returns_decision_response
cargo test --locked --test it_web web_work_item_generation
cargo check --locked
```
Expected:
- 两个新 WS 层测试 PASS。
- 现有 `web_work_item_generation`（P3 REST 流程 + WP2b author 链路）仍全绿。
- `cargo check` 全绿。

> 若 WS helper 缺能力，先补 WP2b 共享 helper；不要把本 Task 的 WS 路由验证延后到 WP8。

- [ ] **Step 2.5：提交**

```bash
git add tests/it_web.rs tests/it_web/web_work_item_plan_review.rs
git commit -m "test(WP3): WorkItemPlan review 整组 verdict + ReviewDecisionResponse 集成测试"
```

---

## Task 3：WP3 收口验证（全量回归 + 交付摘要供 WP4 用）

**目标**：跑完整验证链，确保 review 整组未破坏 Story/Design/WorkItem 既有 review 流程；WorkItemPlan prepare → author → review 链路通；交付 WP4 所需的前置摘要。

**Files:** 无新增改动；仅运行验证命令。

- [ ] **Step 3.1：全量验证链**

Run（依次，任一失败即停并修复）:
```
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked --lib workspace_engine
cargo test --locked --test it_web
```
Expected: 全绿。

> `cargo test --locked --test it_web` 全量跑 web 集成测试，覆盖 Story/Design/WorkItem/WorkItemPlan prepare/author/review 的 HTTP + WS 流程。是 WP3 最大的回归保障——Story/Design 的 `build_review_input` 行为未变（WorkItemPlan 分支提前 return，其他 workspace_type 走原逻辑）。

- [ ] **Step 3.2：确认 WP1/WP2a/WP2b 成果未破坏**

Run:
```
cargo test --locked --test it_web prepare_work_item_plan_creates_draft_plan_and_session_without_generating
cargo test --locked --test it_web work_item_plan_start_generation_returns_candidate_artifact
cargo test --locked --lib workspace_ws_types
```
Expected: PASS（WP1 prepare 仍工作；WP2b author run 仍工作；WP2a union 类型 serde 往返正常）。

- [ ] **Step 3.3：交付摘要（供 WP4 前置交付摘要使用）**

commit 后，把以下内容写入 WP4 plan 的「前置交付摘要」章节：

- WorkItemPlan review 链路：`AuthorDecision::Accept`（AuthorConfirm 阶段）→ `handle_author_decision` 返回 `StartReview` → WS handler `spawn_provider_run_from_handler(ProviderRunKind::ReviewOnly)` → `engine.drive_review_session` → `build_review_input` WorkItemPlan 分支 → `build_work_item_plan_review_input`（从 lifecycle 组装裁剪 candidate prompt）→ reviewer 流式输出 → `parse_review_verdict` → `review_gate_for` → 按gate 进 `ReviewDecision` 或 `HumanConfirm`。
- `build_work_item_plan_review_input`（`workspace_engine.rs`，Task 1 新增）：复用 WP2b 的 `build_work_item_plan_candidate_dto` 组装完整 DTO，再裁剪（repository_profile 只传 confidence + detected_layers；WorkItem 只传 id/kind/title/depends_on/exclusive_write_scopes/verification_plan_ref；dependency_graph 全传；validator_findings 全传；verification_plans 只传摘要）。
- `build_review_input` WorkItemPlan 分支（Task 1 Step 1.4）：函数开头 `if matches!(self.session.workspace_type, WorkspaceType::WorkItemPlan) { return self.build_work_item_plan_review_input(); }`。
- verdict 解析 / `review_gate` / `handle_review_decision` / `ReviewDecisionResponse` 全复用，**WP3 未改**。
- review 触发的 Revision：`handle_review_decision("continue"|"continue_with_context", ...)` 返回 `StartRevision` → WS handler `spawn_provider_run_from_handler(ProviderRunKind::Revision)`。**当前 Revision 分发走普通 `drive_revision_session`（Story/Design 路径），对 WorkItemPlan 会失败**（`build_revision_input` 读 `session.artifact` 的 `WorkItemPlanCandidate` 变体返回空字符串，prompt 为空）——**WP4 需改 Revision 分发为 `ProviderRunKind::WorkItemPlanRevision`**，调 `WorkItemSplitEngine::generate_revision` 而非 `drive_revision_session`。
- **WP4 待办**：
  - `WsInMessage::RevertWorkItem` 标记处理（candidate meta 更新 + 推 ArtifactUpdate）。
  - `RequestRevision`/`ReviewDecisionResponse::continue` 在 WorkItemPlan 下启动 `ProviderRunKind::WorkItemPlanRevision`（非流式 `WorkItemSplitEngine::generate_revision`）。
  - `WorkItemSplitEngine::generate_revision(retained, redo_specs)` + `repatch_dependencies` DAG 重连。
  - `build_revision_input` 对 WorkItemPlan 分支：WP4 可改为返回 feedback context（供 `WorkItemSplitter` prompt 用），或直接在 `WorkItemPlanRevision` run kind 内构造 prompt（不经 `build_revision_input`）——以 WP4 plan 为准。
- **WP3 边界明确**：review 触发的 revision 只验证到 `ReviewDecisionResponse` 响应返回 + stage 进入 revision，**不验证 revision 执行**（WP4 实现 `WorkItemPlanRevision` 后，WP8 贯通测试验证完整 revision 重做循环）。

---

## Self-Review（写完后的自查）

**1. Spec 覆盖**（对照总览 v1.1 WP3 目标/写入范围/验证 + 设计方案 :287-292、:223-237、:317-348）：
- ✅ `build_review_input` WorkItemPlan 分支调 `build_work_item_plan_review_input` → Task 1 Step 1.4
- ✅ 从当前 Draft plan 关联记录组装整组 candidate → Task 1 Step 1.3（复用 `build_work_item_plan_candidate_dto`）
- ✅ 序列化为 review 上下文（裁剪 token）→ Task 1 Step 1.3（repository_profile 只传 confidence + detected_layers；WorkItem 只传 reviewer 关心字段；dependency_graph 全传；validator_findings 全传）
- ✅ reviewer 流式（复用 `drive_review_session`）→ Task 2（WS 层 `ReviewOnly` → `drive_review_session`，通用路径，不改）
- ✅ 复用 verdict 解析 / `review_gate` / `ReviewDecisionResponse` → Task 1 Step 1.3（reviewer 契约尾部与 `build_review_input` 一致，`parse_review_verdict`/`review_gate_for`/`handle_review_decision` 不改）
- ✅ 验证命令链 → Task 3
- ✅ 不做项：未实现 revert/revision 局部重做（WP4）、未实现 confirm（WP5）、未改前端——均在「不做」清单。
- ✅ 不新增 WS 消息变体 / TimelineNodeType / WorkspaceStage / AdapterRole → File Structure「不改」清单 + 全局约束。
- ✅ review 粒度整组一次审查 → Task 1 Step 1.3（prompt 含全部 work_items，不逐个 review）。

**2. Placeholder 扫描**：
- `make_work_item_plan_engine_with_draft_candidate`（Task 1 Step 1.1）：给出职责描述 + 字段清单，未完整展开 lifecycle 记录构造——因 `IssueWorkItemPlan`/`LifecycleWorkItemRecord`/`VerificationPlan`/`RepositoryProfile` 构造涉及 10+ 字段，参考 WP2b 的 `complete_work_item_plan_author` 测试夹具（WP2b plan Task 2 Step 2.1）是合理指引。**实现时应补完整构造**——若 WP2b 已抽 helper，直接复用。
- `prepare_work_item_plan_and_author_to_confirm`（Task 2 Step 2.1）：给出职责描述，依赖 WP2b 共享 WS helper。可接受。
- `RepositoryProfileDto`/`VerificationPlanDto`/`WorkItemCandidateDto` 字段名（Task 1 Step 1.3）：给出 `grep` 定位指引，标注「以 WP1 实际定义为准」。可接受。
- `MockReviewerStreamingProvider`（Task 2 Step 2.3）：参考 engine 测试的 `ReviewVerdictStreamingProvider`（`:5757`），未完整展开——给出 `grep` 定位指引。可接受。
- reviewer 契约尾部 JSON（Task 1 Step 1.3）：完整复制 `build_review_input`（`:2504-2517`）的文本，非占位符。

**3. 类型一致性**：
- `build_work_item_plan_review_input(&self) -> Result<StreamingProviderInput, String>` 签名在 Task 1 定义，与 `build_review_input` 返回类型一致（`StreamingProviderInput`，`AdapterRole::Reviewer`）。
- `build_review_input` WorkItemPlan 分支提前 return `self.build_work_item_plan_review_input()`——类型一致。
- 复用 `build_work_item_plan_candidate_dto`（WP2b free function）返回 `WorkItemPlanCandidateDto`——Task 1 的 `candidate` 变量类型一致。
- `StreamingProviderInput` 字段（`provider_type`/`role`/`prompt`/`working_dir`/`workspace_session_id`/`resume_provider_session_id`/`permission_mode`/`env_vars`/`timeout_secs`）与 `build_review_input`（`:2519-2529`）一致。

**4. 边界风险**：
- **WP4 Revision 分发未实现**（Task 2 Step 2.1 测试边界）：`handle_review_decision("continue")` 返回 `StartRevision` → WS handler `spawn_provider_run_from_handler(ProviderRunKind::Revision)` → `drive_revision_session` → `build_revision_input` 对 WorkItemPlan 返回空 artifact（WP2a 临时处理）→ revision prompt 为空 → provider 调用失败。**WP3 测试只断言 stage 进入 revision，不断言 revision 成功**。已标注。WP4 需改 Revision 分发为 `WorkItemPlanRevision`。
- **reviewer provider 注入**（Task 2 Step 2.3）：WS 层 reviewer 走 `provider_registry.get(reviewer_provider)`——需 app 注入流式 reviewer mock。若 `app_with_confirmed_story_and_design` 只注入非流式 split adapter，需补流式 reviewer 注入；不能用 engine 层测试替代 WS 路由验证。已标注。
- **WP2a 空字符串分支保留**（Task 1 Step 1.4）：`build_review_input` 内对 `session.artifact` 的 `WorkItemPlanCandidate` 变体返回空字符串的 WP2a 分支，WP3 不清理（避免越界改 WP2a 已测试代码）。WorkItemPlan 分支在函数开头提前 return，不会走到那个 match——无功能影响。已标注。
- **裁剪策略与设计方案 :289 一致性**：设计方案要求「repository_profile 只传 confidence + detected_layers；WorkItem 只传 reviewer 关心字段（id/kind/title/depends_on/exclusive_write_scopes/verification_plan_ref，不传 meta）」。Task 1 Step 1.3 的裁剪实现与此一致——`Repository Profile (trimmed)` 只输出 confidence + detected_layers；WorkItems 列表只输出 id/kind/title/depends_on/exclusive_write_scopes/verification_plan_ref，不输出 meta。`dependency_graph` 全传、`validator_findings` 全传——与设计方案一致。
- **review prompt token 上限**（设计方案 :489 风险项）：裁剪后 prompt 仍可能超 token 上限（candidate 含大量 work_items 时）。本 WP 裁剪策略是设计方案 :289 明确要求的，不额外加 token 截断——若实际超限，后续 WP 或维护者加 work_items 分页或摘要。已标注。
- **总览 WP3 验证条目调整建议**：总览 v1.1 WP3 章节列了 `work_item_plan_review_revision_loop`——此名暗示验证完整 revision 重做循环，但 WP3 边界只到 `ReviewDecisionResponse` 响应（revision 执行是 WP4）。建议调整为 `work_item_plan_review_returns_decision_response`（本 plan 已用此名）。**不要自行修改总览文件**——由主控决定。本 plan 在 Task 2 Step 2.1 + Task 3 Step 3.3 标注此调整建议。
- **`append_missing_context_notes_to_prompt` 复用**（Task 1 Step 1.3）：`build_work_item_plan_review_input` 调 `self.append_missing_context_notes_to_prompt(&mut prompt)`（现有方法，`:2984-2994`）——把 prepare 阶段的 ContextNote 追加到 review prompt。与 Story/Design 的 `build_review_input` 一致行为。无风险。

---

## Execution Handoff

本 WP3 plan 已保存至 `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP3_后端review整组_v1.0.md`。

**执行方式（待用户选择）：**

1. **Subagent-Driven（推荐）** — 每个 Task 派一个 fresh subagent 实现，Task 间做 two-stage review。
2. **Inline Execution** — 在当前 session 按 superpowers:executing-plans 批量执行，带 checkpoint 审查。

**完成后**：交付 WP3 后，按同样标准继续 WP4（后端 revert + revision 局部重做，依赖 WP3 的 review 触发 revision 链路 + WP2b 的 candidate 落盘）。WP4 的「前置交付摘要」直接引用本 plan Task 3 Step 3.3 的产出。

**⚠️ 实现前注意**：
1. Task 1 的 `make_work_item_plan_engine_with_draft_candidate` 夹具是本 WP 最繁的构造——建议先 `grep -rn "build_work_item_plan_candidate_dto\|complete_work_item_plan_author" tests/ src/` 看 WP2b 是否已抽类似 helper 可复用。
2. Task 2 的 WS 集成测试依赖 WP2b 共享 helper；若 helper 不完整，先补 helper，再做本 Task。WS 端到端不延后到 WP8。
3. **WP4 边界依赖**：WP3 的 review 触发 revision 只到 `ReviewDecisionResponse` 响应，revision 执行由 WP4 实现。WP4 必须改 `Revision` 分发为 `WorkItemPlanRevision`（当前走 `drive_revision_session` 对 WorkItemPlan 会失败）——本 plan Task 3 Step 3.3 已明确交付此信息。
