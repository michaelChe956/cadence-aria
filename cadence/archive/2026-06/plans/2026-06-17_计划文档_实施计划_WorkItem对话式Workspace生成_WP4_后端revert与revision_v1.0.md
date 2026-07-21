# WorkItem 对话式 Workspace 生成 WP4：后端 revert + revision 局部重做 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 `WsInMessage::RevertWorkItem` 标记处理（candidate meta 更新 + 写回当前 `ArtifactVersion.payload` + 推同 version `ArtifactUpdate`）、批量触发 dedicated 非流式 WorkItemPlan Revision（`ProviderRunKind::WorkItemPlanRevision`）、`WorkItemSplitEngine::generate_revision`（retained + redo_specs，局部重做时 provider 只输出 redo 项）与 `repatch_dependencies` DAG 重连；revision 输出通过 `replace_issue_work_item_plan_candidate` 替换 Draft candidate 再写 artifact payload；review 触发的整组 revision 也走本 WP。最后迁移 WP2b 的 `AutoRevision` 路径到 `generate_revision`，对齐 design 第 269 行语义。

**Architecture:** revert 标记是"在当前 artifact_version 上改 `work_items[i].meta` + 持久化该 version payload + 推同 version 的 `ArtifactUpdate`"，不产生新 version、不调 provider。触发 Revision 走 dedicated 非流式 run：WS handler 在 `RequestRevision`（WorkItemPlan 下）启动 `ProviderRunKind::WorkItemPlanRevision`，用 `state.provider_adapter` 构造 `WorkItemSplitEngine`，调新 `generate_revision(retained, redo_specs)`。局部重做时 retained 由后端沿用，provider 只返回 redo 项，后端分配新 id、合并 retained+redo、执行 `repatch_dependencies`；整组 review/AutoRevision（retained/redo 均空）退化为现有完整 split 输出解析。随后调 `WorkspaceEngine::complete_work_item_plan_revision` 完成 replace candidate → 组装 DTO → `update_artifact(WorkItemPlanCandidate)` → 回 AuthorConfirm。`generate_revision` 是 `WorkItemSplitEngine` 的新方法，不改 `generate` 主体；`repatch_dependencies` 是纯函数，负责把被重做 WorkItem 的旧 id 在 `dependency_graph` 与其他 WorkItem 的 `depends_on` 中改指向新 id。

**Tech Stack:** Rust 1.95.0、Cargo、Axum、tokio（`spawn_blocking` 在 `WorkItemSplitEngine` 内）、serde。本 WP 不涉及前端。

**版本：** v1.0
**创建日期：** 2026-06-17
**目标分支：** `feat-b-0616`
**工作区：** `.worktrees/feat-b-0616`
**依赖总览：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_拆分总览_v1.0.md`（v1.1，WP4 章节）
**设计方案：** `cadence/designs/2026-06-17_技术方案_WorkItem对话式Workspace生成_v1.0.md`（第 274-305 行 AuthorConfirm 与 revert、第 294-305 行 Revision 阶段、第 326-335 行 RevertWorkItem 消息）
**前置 WP：** WP1、WP2a、WP2b、WP3

---

## 全局约束（Global Constraints）

- **运行命令固定**：Rust 1.95.0；cargo 命令带 `--locked`；🔴 **禁止 `-j 1`**。
- **强制检查链**：`cargo fmt --check` + `cargo clippy --all-targets --all-features --locked -- -D warnings` + `cargo check --locked` + 定向测试，四者缺一不可。
- **TDD**：每个 Task 先写失败测试，再写实现。
- **写入范围严格**：只改「File Structure」声明的文件。本 WP 共享 `workspace_engine.rs` / `workspace_ws_handler.rs` / `work_item_split_engine.rs` / `lifecycle_store.rs`——须在 WP2b、WP3 之后串行执行。
- **行号是参考**：基于 `feat-b-0616` HEAD `8a2eee4`；实现时以 `grep -n` 实际为准。

---

## 前置交付摘要（来自 WP1 + WP2a + WP2b + WP3）

### 来自 WP1
- `WsInMessage::RevertWorkItem { work_item_id: String, feedback: Option<String>, clear: bool }` 已定义（WP4 实现 handler 逻辑）。
- `WorkItemCandidateMetaDto { reverted: bool, revert_feedback: Option<String> }`、`WorkItemCandidateDto`（含 `meta` 字段）、`WorkItemPlanCandidateDto` 已定义。
- `WorkspaceType::WorkItemPlan` 变体已就位。

### 来自 WP2a
- `ArtifactPayload::WorkItemPlanCandidate { candidate: WorkItemPlanCandidateDto }` 已挂载：`update_artifact(&mut self, payload: ArtifactPayload)`、`EngineEvent::ArtifactUpdate { version, payload }`、`SessionState.artifact: Option<ArtifactPayload>`、`ArtifactVersion { #[serde(flatten)] payload }`。
- `update_artifact` 每次调用**递增 version**并设 `is_current`——WP4 的 revert 标记**不能调 `update_artifact`**（会产新 version），必须直接改 `session.artifact` 的 candidate meta 并手动推同 version 的 `EngineEvent::ArtifactUpdate`。

### 来自 WP2b
- `LifecycleStore::replace_issue_work_item_plan_candidate(project_id, issue_id, plan_id, output: &WorkItemSplitProviderOutput, validator_findings: Vec<WorkItemSplitFinding>) -> Result<WorkItemPlanCandidateSnapshot, ProductStoreError>`：替换 Draft plan 关联的 work_items/verification_plans/repository_profile + 更新 plan 引用；拒绝 Confirmed plan；不建子 session。
- `WorkspaceEngine::complete_work_item_plan_author(output: WorkItemSplitProviderOutput) -> Result<WorkItemPlanAuthorOutcome, String>`：validate → replace candidate → 组装 DTO → `update_artifact(WorkItemPlanCandidate)` → `enter_author_confirm`。
- `WorkItemPlanAuthorOutcome { AuthorConfirm, AutoRevision { findings }, HumanConfirm { reason } }`：WP2b 的 `AutoRevision` 让 handler 重新 spawn `WorkItemPlanAuthor`（无 feedback 重生）——**WP4 Task 4 迁移此分支**。
- `build_work_item_plan_candidate_dto(lifecycle, project_id, issue_id, plan_id) -> Result<WorkItemPlanCandidateDto, ProductStoreError>`（free function）。
- `ProviderRunKind::WorkItemPlanAuthor`、`ProviderRunContext` 已有 `provider_adapter: Arc<dyn ProviderAdapter + Send + Sync>` 字段、`StartGeneration` 按 `workspace_type == WorkItemPlan` 路由模式。
- `WorkspaceEngine` struct 有 `work_item_plan_author_retry_count: u32` 字段（WP2b 新增）。
- Draft candidate 已落盘：plan（Draft）+ work_items（Draft）+ verification_plans + repository_profile，**无子 WorkItem session**。

### 来自 WP3
- `build_work_item_plan_review_input` 已就位（`build_review_input` 的 WorkItemPlan 分支）。
- review `handle_review_decision("continue")` → `StartRevision`：在 WorkItemPlan 下必须路由到 `WorkItemPlanRevision`（WP4 Task 3 实现），不走普通 `drive_revision_session`。
- WP3 的 review revision loop 测试只到 `ReviewDecisionResponse`，完整 revision 重做由 WP4 实现（WP8 贯通）。

---

## 关键既有事实（避免重新探查）

所有行号基于 `feat-b-0616` HEAD `8a2eee4`，实现时用 `grep -n` 确认。

### `src/product/work_item_split_engine.rs`（747 行）
- `WorkItemSplitEngine { provider_adapter: Arc<dyn ProviderAdapter + Send + Sync> }`，`new(adapter)`（:141-149）。
- `generate(&self, request: &GenerateWorkItemsRequest, lifecycle: &LifecycleStore, issue: &IssueRecord, repository: &RepositoryRecord, author_provider: ProviderName) -> ApiResult<WorkItemSplitProviderOutput>`（:151-265），内部 :227 `spawn_blocking`。**generate 只存 provider run，不持久化 plan/work_items**。
- `WorkItemSplitProviderOutput { repository_profile, plan, work_items, verification_plans }`（:133-139）。
- `build_split_prompt` / `parse_provider_output` / `WORK_ITEM_SPLIT_OUTPUT_SCHEMA`（:21-131）/ `summarize_repository_structure` 均为内部函数。
- **无** `generate_revision` / `repatch_dependencies`——WP4 新增。

### `src/product/workspace_engine.rs`（8362 行）
- `WorkspaceEngine` struct（:333-347）：持有 `lifecycle_store: Option<LifecycleStore>`、`event_tx`、`session: WorkspaceSession`、`artifact_versions`、`cancel: CancellationToken`、`work_item_plan_author_retry_count: u32`（WP2b）。**不持有 provider_adapter**。
- `drive_revision_session`（:1620 附近）：Story/Design 的流式 revision 路径——**WorkItemPlan 不走此路径**。
- `build_revision_input`（:2550 附近）：Story/Design 喂 revision prompt；WP2a 对 WorkItemPlanCandidate 变体返回空字符串。
- `build_review_input`（:2470 附近）：WP3 已加 WorkItemPlan 分支调 `build_work_item_plan_review_input`。
- `handle_review_decision`：review verdict 后的决策分发，`continue` → `StartRevision`（触发 revision run）。
- `enter_author_confirm(&mut self, summary: Option<String>)`（:3108-3127）。
- `transition_stage` / `create_timeline_node` / `append_completed_timeline_event` / `complete_active_node` / `update_artifact`：现有方法。
- `complete_work_item_plan_author`（WP2b）：WP4 Task 4 修改其 `AutoRevision` 分支。
- `build_work_item_plan_candidate_dto`（WP2b free function）。
- 文件顶部 `use`（:1-33）：已导入 `ArtifactPayload`、`WorkItemSplitProviderOutput`（WP2b）、`WorkItemSplitValidator` 等。WP4 需补 `WorkItemSplitEngine`（若 Task 3 在 engine 层构造——实际由 handler 构造，engine 不需导入）、`RedoSpec`（本 WP 定义）等。

### `src/web/workspace_ws_handler.rs`（1553 行）
- `ProviderRunKind { Author { content }, AuthorChoiceFollowup { content }, Revision, ReviewOnly, WorkItemPlanAuthor }`（:1293-1298）——WP4 加 `WorkItemPlanRevision` 变体。
- `ProviderRunContext`（:773-781，`#[derive(Clone)]`）：已有 `provider_adapter` 字段（WP2b）、`app_paths`/`session_record`（WP2b 方案 A 加的）。WP4 复用。
- `spawn_provider_run_from_handler(run_context, run_kind)`（:1347-1470）：provider 选择 match :1362-1374，run 分发 match :1411-1432。WP4 两处 match 加 `WorkItemPlanRevision` 分支。
- `StartGeneration`（:677-707）：WP2b 已按 workspace_type 路由 `WorkItemPlanAuthor`。WP4 的 `RequestRevision` 路由参照此模式。
- `WsInMessage::RequestRevision` 处理：现有，触发 `ProviderRunKind::Revision`。WP4 按 workspace_type 路由 `WorkItemPlanRevision`。
- `WsInMessage::RevertWorkItem` 处理：**WP4 新增**（WP1 只定义变体，未实现分发）。
- `is_message_valid_for_stage` / `message_type`：阶段白名单与消息类型映射。WP4 把 `RevertWorkItem` 加入 AuthorConfirm 白名单。
- event forwarder（:248-392）：WP2a 已适配 `EngineEvent::ArtifactUpdate { version, payload }`。WP4 不改。
- 文件顶部 `use`（:1-34）：WP4 补 `WorkItemSplitEngine`、`WorkItemPlanRevisionOutcome`（若定义）等。

### `src/web/workspace_ws_types.rs`
- `WsInMessage::RevertWorkItem { work_item_id, feedback: Option<String>, clear: bool }`（WP1，:135-190 区间）。
- `WsInMessage::RequestRevision { feedback: Option<String> }`（现有）。
- `WorkItemCandidateMetaDto { reverted: bool, revert_feedback: Option<String> }`、`WorkItemPlanCandidateDto`（WP1）。

### `src/product/lifecycle_store.rs`（2054 行）
- `replace_issue_work_item_plan_candidate`（WP2b）：WP4 revision 直接复用。
- `get_issue_work_item_plan` / `list_work_items` / `get_work_item`：现有 pub fn。
- `WorkItemPlanCandidateSnapshot { plan_id, work_item_ids, verification_plan_ids, repository_profile_id }`（WP2b）。

### `src/product/models.rs`
- `IssueWorkItemPlan`、`LifecycleWorkItemRecord`（含 `id`/`kind`/`title`/`depends_on`/`exclusive_write_scopes`/`verification_plan_ref`/`plan_status`）、`IssueWorkItemDependencyEdge { from_work_item_id, to_work_item_id }`、`WorkItemSplitFinding`。

---

## File Structure

| 文件 | 操作 | 职责 / 本 WP 改动 |
|---|---|---|
| `src/product/work_item_split_engine.rs` | M | 新增 `generate_revision(&self, request, lifecycle, issue, repository, author_provider, retained: &[LifecycleWorkItemRecord], redo_specs: &[RedoSpec]) -> ApiResult<WorkItemSplitProviderOutput>`；新增 redo-only provider parser/merge helper；新增 `repatch_dependencies(graph, id_mapping) -> Vec<IssueWorkItemDependencyEdge>` 纯函数；定义 `RedoSpec { old_id, feedback }`。**不改 `generate` 主体** |
| `src/product/workspace_engine.rs` | M | 新增 `complete_work_item_plan_revision(&mut self, output: WorkItemSplitProviderOutput) -> Result<(), String>`（replace candidate → 组装 DTO → `update_artifact(WorkItemPlanCandidate)` → 回 AuthorConfirm）；新增 `apply_revert_mark(&mut self, work_item_id, feedback, clear) -> Result<(), String>`（改 candidate meta + 推同 version ArtifactUpdate）；Task 4 修改 `complete_work_item_plan_author` 的 `AutoRevision` 分支 |
| `src/web/workspace_ws_handler.rs` | M | `ProviderRunKind` 加 `WorkItemPlanRevision`；`WsInMessage::RevertWorkItem` 分发 + AuthorConfirm 白名单 + `message_type`；`RequestRevision` 按 workspace_type 路由 `WorkItemPlanRevision`；`handle_review_decision` 的 StartRevision 在 WorkItemPlan 下路由 `WorkItemPlanRevision`；`spawn_provider_run_from_handler` 两处 match 加 `WorkItemPlanRevision` 分支 |
| `src/product/lifecycle_store.rs` | （复用） | 复用 `replace_issue_work_item_plan_candidate`，**不改**（除非需读单个 work_item 的 helper） |
| `src/web/workspace_ws_types.rs` | （一般不改） | `RevertWorkItem` 变体 WP1 已定义；若需调整 feedback 必填性在此（一般不改） |
| `tests/it_product/product_work_item_split_engine.rs` | M | `generate_revision` + `repatch_dependencies` 单测 |
| `tests/it_web/web_work_item_generation.rs` 或新增 `tests/it_web/web_work_item_plan_revert.rs` | M/N | revert + revision 集成测试 |
| `tests/it_web.rs` | M | 若新增 mod，加 `#[path]` 注册 |

**不改：**
- ❌ `src/product/work_item_split_validator.rs`（只调用 `validate`）
- ❌ `src/web/handlers.rs` / `app.rs` / `workspace_context.rs`（WP1 已完成；WP5 才删废弃路由）
- ❌ 前端（WP6/WP7）

---

## Task 1：`WorkItemSplitEngine::generate_revision` + `repatch_dependencies`

**目标**：`generate_revision` 在 `WorkItemSplitEngine` 上新增方法。局部重做时 prompt 注入"保留项清单 + 重做项及反馈"，provider **只输出 redo 项**，后端把 retained 原记录与 redo 输出合并，再 `repatch_dependencies` 重连 DAG；整组 review/AutoRevision（`retained`/`redo_specs` 均空）复用现有完整 split 输出解析。`repatch_dependencies` 是纯函数。**不改 `generate` 主体**。

**Files:**
- Modify: `src/product/work_item_split_engine.rs`
- Test: `tests/it_product/product_work_item_split_engine.rs`

**Interfaces:**
- Consumes: `WorkItemSplitEngine::generate` 的内部 helper（`build_split_prompt`/`parse_provider_output`/`spawn_blocking` 调 provider 的模式）；`LifecycleWorkItemRecord`/`IssueWorkItemDependencyEdge`。局部重做新增 revision-only 输出结构，不能要求现有 `ProviderWorkItem` 携带 id。
- Produces:
  - `RedoSpec { old_id: String, feedback: String }`
  - `WorkItemSplitEngine::generate_revision(...) -> ApiResult<WorkItemSplitProviderOutput>`
  - `repatch_dependencies(graph: &[IssueWorkItemDependencyEdge], id_mapping: &HashMap<String, String>) -> Vec<IssueWorkItemDependencyEdge>`

- [ ] **Step 1.1：写失败测试 —— `repatch_dependencies` 重连依赖**

在 `tests/it_product/product_work_item_split_engine.rs` 末尾追加。`repatch_dependencies` 是纯函数，无需 provider。

```rust
    #[test]
    fn repatch_dependencies_reconnects_dependents() {
        use std::collections::HashMap;
        use crate::product::models::IssueWorkItemDependencyEdge;
        // 原 DAG: A→B, A→C, B→C
        let graph = vec![
            IssueWorkItemDependencyEdge { from_work_item_id: "work_item_0001".into(), to_work_item_id: "work_item_0002".into() },
            IssueWorkItemDependencyEdge { from_work_item_id: "work_item_0001".into(), to_work_item_id: "work_item_0003".into() },
            IssueWorkItemDependencyEdge { from_work_item_id: "work_item_0002".into(), to_work_item_id: "work_item_0003".into() },
        ];
        // A 被重做，新 id = work_item_0009
        let mut mapping = HashMap::new();
        mapping.insert("work_item_0001".to_string(), "work_item_0009".to_string());
        let repatched = repatch_dependencies(&graph, &mapping);
        // 所有 0001 引用改为 0009
        assert!(repatched.iter().all(|e| e.from_work_item_id != "work_item_0001" && e.to_work_item_id != "work_item_0001"));
        assert!(repatched.iter().any(|e| e.from_work_item_id == "work_item_0009" && e.to_work_item_id == "work_item_0002"));
        assert!(repatched.iter().any(|e| e.from_work_item_id == "work_item_0009" && e.to_work_item_id == "work_item_0003"));
        // B→C 不受影响
        assert!(repatched.iter().any(|e| e.from_work_item_id == "work_item_0002" && e.to_work_item_id == "work_item_0003"));
        // 数量不变
        assert_eq!(repatched.len(), 3);
    }
```

- [ ] **Step 1.2：写失败测试 —— `generate_revision` 保留未标记项 + 重做被 revert 项**

```rust
    #[tokio::test]
    async fn generate_revision_keeps_retained_and_redoes_marked() {
        let (_dir, lifecycle, issue, repository, engine) = split_engine_fixture().await;
        // retained: work_item_0001（保留），redo_specs: work_item_0002 要重做（feedback="拆得太粗"）
        let retained = vec![/* LifecycleWorkItemRecord id=work_item_0001 ... */];
        let redo_specs = vec![RedoSpec { old_id: "work_item_0002".into(), feedback: "拆得太粗".into() }];
        let request = /* GenerateWorkItemsRequest，与 generate 同构 */;

        let output = engine.generate_revision(&request, &lifecycle, &issue, &repository, ProviderName::ClaudeCode, &retained, &redo_specs).await.expect("revision");
        // 后端合并后，保留项 id 仍在
        assert!(output.work_items.iter().any(|wi| wi.id == "work_item_0001"));
        // 旧 redo id 不在（被重做）
        assert!(output.work_items.iter().all(|wi| wi.id != "work_item_0002"));
        // 整组数量 = retained.len() + redo_specs.len()
        assert_eq!(output.work_items.len(), retained.len() + redo_specs.len());
    }
```

> 实现者注意：`split_engine_fixture` 参考 `tests/it_product/product_work_item_split_engine.rs` 现有 `generate` 测试夹具（`grep -n "fn.*generate\|async fn" tests/it_product/product_work_item_split_engine.rs`）。`MockSplitProviderAdapter` 让 provider 返回 **redo-only** JSON（只包含被重做项，不包含 `work_item_0001` retained）；断言 `generate_revision` 后端把 retained 原样合并回 output。`LifecycleWorkItemRecord` 构造参考 `tests/it_web/web_work_item_generation.rs` 的 `valid_split_output()`。

- [ ] **Step 1.3：运行测试，确认失败**

Run: `cargo test --locked --test it_product generate_revision`
Expected: 编译失败——`generate_revision` / `repatch_dependencies` / `RedoSpec` 未定义。

- [ ] **Step 1.4：定义 `RedoSpec` + `repatch_dependencies`**

`src/product/work_item_split_engine.rs`，在 `WorkItemSplitProviderOutput`（:133-139）附近定义：

```rust
/// 被重做的 WorkItem 规格：旧 id + 用户反馈。
#[derive(Debug, Clone)]
pub struct RedoSpec {
    pub old_id: String,
    pub feedback: String,
}

/// DAG 重连：把 graph 中对旧 id 的引用改为新 id。
///
/// `id_mapping`: old_id → new_id。只重写映射中存在的 id，未映射的边原样保留。
pub fn repatch_dependencies(
    graph: &[IssueWorkItemDependencyEdge],
    id_mapping: &std::collections::HashMap<String, String>,
) -> Vec<IssueWorkItemDependencyEdge> {
    graph
        .iter()
        .map(|edge| IssueWorkItemDependencyEdge {
            from_work_item_id: id_mapping
                .get(&edge.from_work_item_id)
                .cloned()
                .unwrap_or_else(|| edge.from_work_item_id.clone()),
            to_work_item_id: id_mapping
                .get(&edge.to_work_item_id)
                .cloned()
                .unwrap_or_else(|| edge.to_work_item_id.clone()),
        })
        .collect()
}
```

> `IssueWorkItemDependencyEdge` 顶部 `use` 已导入（`grep -n "use crate::product::models" src/product/work_item_split_engine.rs` 确认，若无则补）。

- [ ] **Step 1.5：实现 `generate_revision`**

`src/product/work_item_split_engine.rs`，在 `generate`（:151-265）之后新增。复用 `generate` 的 provider 调用模式，但解析分两路：`retained.is_empty() && redo_specs.is_empty()` 时复用完整 `parse_provider_output`；局部重做时解析 revision-only redo 输出，再由后端合并 retained。

```rust
impl WorkItemSplitEngine {
    /// Revision：保留项 + redo-only 重做项 + DAG repatch。
    ///
    /// 局部重做时，prompt 注入"保留项清单（只作上下文，不允许重写）+ 重做项及反馈"，
    /// provider 只输出 redo 项。后端负责：
    /// 1. retained 原记录直接合并；
    /// 2. 为 redo 输出分配新 id / verification_plan id；
    /// 3. 用 redo_specs 顺序建立 old_id -> new_id 映射；
    /// 4. `repatch_dependencies` 把 dependency_graph 与 retained/redo 的 depends_on 中旧 id 改成新 id。
    ///
    /// retained/redo_specs 均空时表示整组 review/AutoRevision，退化为完整 split 输出解析。
    pub async fn generate_revision(
        &self,
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
        retained: &[LifecycleWorkItemRecord],
        redo_specs: &[RedoSpec],
    ) -> ApiResult<WorkItemSplitProviderOutput> {
        let prompt = build_revision_prompt(request, retained, redo_specs);
        let provider_output = self.invoke_provider(&prompt, author_provider).await?;
        let structured = provider_output.structured_output.ok_or_else(|| {
            ApiError::runtime("work_item_split_provider_output_invalid", "missing structured output", json!({}))
        })?;

        if retained.is_empty() && redo_specs.is_empty() {
            return parse_provider_output(
                lifecycle,
                request,
                issue,
                repository,
                provider_output.run_ref,
                &structured,
            );
        }

        let redo = parse_revision_redo_output(&structured)?;
        if redo.work_items.len() != redo_specs.len() || redo.verification_plans.len() != redo_specs.len() {
            return Err(ApiError::validation(
                "revision_redo_count_mismatch",
                &format!("redo_specs={} but provider returned work_items={} verification_plans={}", redo_specs.len(), redo.work_items.len(), redo.verification_plans.len()),
            ));
        }

        let mut id_mapping = std::collections::HashMap::new();
        let mut merged_work_items = retained.to_vec();
        let mut redo_work_items = materialize_redo_work_items(
            lifecycle,
            request,
            issue,
            repository,
            &provider_output.run_ref,
            redo.work_items,
            redo.verification_plans,
            redo_specs,
            &mut id_mapping,
        )?;
        merged_work_items.append(&mut redo_work_items);

        for wi in &mut merged_work_items {
            wi.depends_on = wi
                .depends_on
                .iter()
                .map(|dep| id_mapping.get(dep).cloned().unwrap_or_else(|| dep.clone()))
                .collect();
        }

        let old_graph = build_graph_from_work_items(&merged_work_items);
        let dependency_graph = repatch_dependencies(&old_graph, &id_mapping);
        build_revision_provider_output(
            request,
            issue,
            repository,
            provider_output.run_ref,
            merged_work_items,
            redo.repository_profile,
            dependency_graph,
        )
    }
}
```

> 实现者注意：
> 1. `invoke_provider` 应抽出 `generate` 内部 `spawn_blocking` 调 provider 的逻辑，并返回包含 `structured_output` 与 `run_ref` 的结果；不要把它简化成 raw string，否则无法复用现有 `parse_provider_output` 签名与 provider run 引用。
> 2. `build_revision_prompt(request, retained, redo_specs) -> String` 是新 helper，参考 `build_split_prompt` 风格。局部重做时明确要求 provider 输出 redo-only JSON（work_items/verification_plans 数量必须等于 `redo_specs.len()`，不包含 retained，不包含旧 id）；整组修正时要求完整 split JSON。
> 3. `parse_revision_redo_output` 是 revision-only parser，结构可复用 `ProviderWorkItem` / `ProviderVerificationPlan` / `ProviderRepositoryProfile`，但只解析 redo 项，不尝试读取 id。
> 4. `materialize_redo_work_items` 负责为 redo 项按现有 count 分配 `work_item_*` / `verification_plan_*` id，并填充 `LifecycleWorkItemRecord` / `VerificationPlan`；实现时可抽取 `parse_provider_output` 中的 id 分配与 record 构造逻辑，避免复制大块代码。
> 5. retained 不经过 provider 输出校验，因为 provider 不输出 retained；后端直接沿用 lifecycle 中的 `LifecycleWorkItemRecord`。这就是本计划选定策略，避免与现有 provider schema 冲突。

- [ ] **Step 1.6：运行 Task 1 测试 + 收口**

Run:
```
cargo test --locked --test it_product generate_revision
cargo test --locked --test it_product repatch_dependencies
cargo test --locked --test it_product product_work_item_split_engine
cargo check --locked
```
Expected: 新测试 PASS；现有 split engine 测试全绿（`generate` 主体未改）；`cargo check` 全绿。

- [ ] **Step 1.7：提交**

```bash
git add src/product/work_item_split_engine.rs tests/it_product/product_work_item_split_engine.rs
git commit -m "feat(WP4): WorkItemSplitEngine generate_revision + repatch_dependencies"
```

---

## Task 2：`RevertWorkItem` 标记处理（engine + WS handler）

**目标**：用户在 AuthorConfirm 阶段对单个 WorkItem 点 `[revert]` → WS handler 收 `RevertWorkItem { work_item_id, feedback, clear }` → 调 `engine.apply_revert_mark` → 改当前 artifact_version 的 candidate `work_items[i].meta.reverted` + 存 feedback → 写回当前 `ArtifactVersion.payload`（不新增 version）→ 推**同 version** 的 `ArtifactUpdate`。`clear: true` 取消标记。可连续标记多个。`RevertWorkItem` 加入 AuthorConfirm 阶段白名单。

**Files:**
- Modify: `src/product/workspace_engine.rs`（新 `apply_revert_mark`）
- Modify: `src/web/workspace_ws_handler.rs`（`RevertWorkItem` 分发 + 白名单 + `message_type`）
- Test: `tests/it_web/web_work_item_plan_revert.rs`（新增）或 `web_work_item_generation.rs` 末尾

**Interfaces:**
- Consumes: WP1 的 `WsInMessage::RevertWorkItem`；WP2a 的 `ArtifactPayload::WorkItemPlanCandidate`；`WorkItemCandidateMetaDto`。
- Produces: `WorkspaceEngine::apply_revert_mark(&mut self, work_item_id, feedback, clear) -> Result<(), String>`。

- [ ] **Step 2.1：写失败测试 —— revert 标记在 AuthorConfirm 有效，改 meta 不产新 version**

在 `tests/it_web/web_work_item_plan_revert.rs`（新增，并在 `tests/it_web.rs` 加 `#[path]` mod 注册）或 `web_work_item_generation.rs` 末尾。复用 `app_with_confirmed_story_and_design` + prepare + start_generation 夹具（WP2b 已建）。

```rust
#[tokio::test]
async fn revert_work_item_is_valid_in_author_confirm_only() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;
    let session_id = prepare_and_start_generation(&app).await; // helper: prepare + WS start_generation，返回 session_id 并推进到 author_confirm
    let ws = connect_ws(&app, &session_id).await;

    // AuthorConfirm 阶段发 RevertWorkItem → 应被接受（不返回 Error）
    ws.send(json!({"type":"revert_work_item","work_item_id":"work_item_0001","feedback":"拆得太粗","clear":false}).to_string()).await;
    let messages = recv_ws_messages(&ws, timeout).await;
    // 收到 ArtifactUpdate（同 version，candidate 的 work_item_0001.meta.reverted=true）
    let artifact = messages.iter().find(|m| m["type"] == "artifact_update").expect("artifact_update");
    let wi = artifact["candidate"]["work_items"].as_array().unwrap().iter()
        .find(|w| w["id"] == "work_item_0001").unwrap();
    assert_eq!(wi["meta"]["reverted"], true);
    assert_eq!(wi["meta"]["revert_feedback"], "拆得太粗");
    // version 不变（标记不产新 version）
    // （需先记录 start_generation 后的 version，断言 revert 后 version 相同）
}

#[tokio::test]
async fn revert_work_item_clear_removes_mark() {
    // ... 同上，先 revert 标记，再 clear:true，断言 meta.reverted=false, feedback=null
}
```

> 实现者注意：`prepare_and_start_generation`/`connect_ws`/`recv_ws_messages` helper 复用 WP2b Task 3 新增的共享 WS helper。若 helper 缺少 `recv_until_stage` / timeout 收集能力，先补 helper；本 Task 不 fallback 到 engine 层，因为需要覆盖 `RevertWorkItem` 的 WS 白名单与分发。

- [ ] **Step 2.2：运行测试，确认失败**

Run: `cargo test --locked --test it_web revert_work_item`
Expected: 失败——`RevertWorkItem` 无分发（WS handler 不认识该消息或返回 Error）、`apply_revert_mark` 未定义。

- [ ] **Step 2.3：实现 `apply_revert_mark`**

`src/product/workspace_engine.rs`，在 `complete_work_item_plan_author` 附近新增：

```rust
impl WorkspaceEngine {
    /// AuthorConfirm 阶段标记/取消标记单个 WorkItem 的 revert。
    ///
    /// **不产生新 artifact_version**：改 `session.artifact` 与当前 is_current
    /// `ArtifactVersion.payload` 的 candidate meta，再推同 version 的 `EngineEvent::ArtifactUpdate`。
    pub async fn apply_revert_mark(
        &mut self,
        work_item_id: &str,
        feedback: Option<String>,
        clear: bool,
    ) -> Result<(), String> {
        let payload = self
            .session
            .artifact
            .clone()
            .ok_or("no artifact to mark revert on")?;
        let mut candidate = match payload {
            ArtifactPayload::WorkItemPlanCandidate { candidate } => candidate,
            _ => return Err("artifact is not a WorkItemPlanCandidate".into()),
        };
        let wi = candidate
            .work_items
            .iter_mut()
            .find(|w| w.id == work_item_id)
            .ok_or_else(|| format!("work_item {} not in candidate", work_item_id))?;
        if clear {
            wi.meta.reverted = false;
            wi.meta.revert_feedback = None;
        } else {
            wi.meta.reverted = true;
            wi.meta.revert_feedback = feedback;
        }

        // 更新 session.artifact + 当前 ArtifactVersion.payload（不 push artifact_versions，version 不变）
        let current_version = self
            .artifact_versions
            .iter()
            .rev()
            .find(|v| v.is_current)
            .map(|v| v.version)
            .unwrap_or(1);
        let payload = ArtifactPayload::WorkItemPlanCandidate { candidate: candidate.clone() };
        self.session.artifact = Some(payload.clone());
        if let Some(version) = self.artifact_versions.iter_mut().rev().find(|v| v.is_current) {
            version.payload = payload.clone();
            self.persist_artifact_versions();
        }

        // 推同 version 的 ArtifactUpdate（前端据此刷新 candidate 展示）
        let _ = self
            .event_tx
            .send(EngineEvent::ArtifactUpdate {
                version: current_version,
                payload,
            })
            .await;
        Ok(())
    }
}
```

> 实现者注意：
> 1. **不调 `update_artifact`**——`update_artifact` 会 push 新 `ArtifactVersion` 并递增 version。revert 标记必须保持 version 不变：同步改 `session.artifact` 和当前 `artifact_versions[is_current].payload`，调用 `persist_artifact_versions()` 写回，再手动发同 version 的 `EngineEvent::ArtifactUpdate`。
> 2. `WorkItemCandidateDto.meta` 字段是 `WorkItemCandidateMetaDto`（WP1 定义），需 `use` 导入。
> 3. candidate meta 变化必须跨重连保留；恢复路径走 `new_persistent` 从当前 `ArtifactVersion.payload` 还原 `session.artifact`。不把 revert meta 写入 lifecycle work_item record，避免污染 Draft candidate 的事实记录；artifact payload 是展示/恢复镜像。

- [ ] **Step 2.4：WS handler `RevertWorkItem` 分发 + 白名单 + `message_type`**

`src/web/workspace_ws_handler.rs`：

1. **白名单**：`is_message_valid_for_stage`（`grep -n "fn is_message_valid_for_stage" src/web/workspace_ws_handler.rs`）把 `RevertWorkItem` 加入 `AuthorConfirm` 阶段白名单（与 `AuthorDecision` 同阶段）。
2. **`message_type`**：`grep -n "fn message_type\|WsInMessage::" src/web/workspace_ws_handler.rs`，在 `WsInMessage` → 消息类型字符串映射里加 `WsInMessage::RevertWorkItem { .. } => "revert_work_item"`。
3. **分发**：在 WS 消息分发 match（`grep -n "WsInMessage::StartGeneration\|WsInMessage::AuthorDecision" src/web/workspace_ws_handler.rs` 定位主 match）加分支：

```rust
            WsInMessage::RevertWorkItem { work_item_id, feedback, clear } => {
                let result = {
                    let mut engine = engine.lock().await;
                    engine.apply_revert_mark(&work_item_id, feedback, clear).await
                };
                if let Err(message) = result {
                    let err = WsOutMessage::Error { message };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                }
                // 成功时 apply_revert_mark 已发 EngineEvent::ArtifactUpdate，event forwarder 会推前端
            }
```

> 实现者注意：`RevertWorkItem` 只在 AuthorConfirm 阶段有效——`is_message_valid_for_stage` 拦截其他阶段的该消息。若 stage 不符，现有阶段校验逻辑会返回 Error（参考 `AuthorDecision` 的阶段校验）。

- [ ] **Step 2.5：运行 Task 2 测试 + 收口**

Run:
```
cargo test --locked --test it_web revert_work_item
cargo test --locked --test it_web web_work_item_generation
cargo check --locked
```
Expected: 新测试 PASS；现有测试全绿；`cargo check` 全绿。

- [ ] **Step 2.6：提交**

```bash
git add src/product/workspace_engine.rs src/web/workspace_ws_handler.rs tests/it_web/
git commit -m "feat(WP4): RevertWorkItem 标记处理（candidate meta + 同 version ArtifactUpdate）"
```

---

## Task 3：WorkItemPlan Revision run + RequestRevision 路由 + review 触发路由 + `complete_work_item_plan_revision`

**目标**：① `ProviderRunKind::WorkItemPlanRevision` 变体；② `RequestRevision` 在 WorkItemPlan 下路由到 `WorkItemPlanRevision`（不走普通 `Revision`）；③ `handle_review_decision` 的 `StartRevision` 在 WorkItemPlan 下也路由 `WorkItemPlanRevision`；④ `spawn_provider_run_from_handler` 两处 match 加 `WorkItemPlanRevision` 分支（构造 `WorkItemSplitEngine` + 调 `generate_revision` + 调 `engine.complete_work_item_plan_revision`）；⑤ `complete_work_item_plan_revision`：replace candidate → 组装 DTO → `update_artifact(WorkItemPlanCandidate)` → 回 AuthorConfirm。

**Files:**
- Modify: `src/product/workspace_engine.rs`（`complete_work_item_plan_revision`）
- Modify: `src/web/workspace_ws_handler.rs`（`ProviderRunKind`、`RequestRevision` 路由、review StartRevision 路由、`spawn_provider_run_from_handler` 两处 match）
- Test: `tests/it_web/web_work_item_plan_revert.rs`

**Interfaces:**
- Consumes: Task 1 的 `generate_revision` + `RedoSpec`；WP2b 的 `replace_issue_work_item_plan_candidate` + `build_work_item_plan_candidate_dto`；`lifecycle.list_work_items`（取 retained vs redo）。
- Produces: `WorkspaceEngine::complete_work_item_plan_revision(output) -> Result<(), String>`；WorkItemPlan revision 的 WS 触发链路。

- [ ] **Step 3.1：写失败测试 —— revert 批量触发 revision 重做被标记项、保留其余、DAG 重连**

```rust
#[tokio::test]
async fn revert_work_item_triggers_local_redo_in_revision() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;
    let session_id = prepare_and_start_generation(&app).await;
    let ws = connect_ws(&app, &session_id).await;
    // 标记 work_item_0001 revert
    ws.send(json!({"type":"revert_work_item","work_item_id":"work_item_0001","feedback":"拆得太粗","clear":false}).to_string()).await;
    // 触发 revision
    ws.send(json!({"type":"request_revision","feedback":"重做被标记的"}).to_string()).await;
    let messages = recv_ws_messages_with_timeout(&ws, timeout).await;
    // 收到新 ArtifactUpdate（version 递增），candidate 中 0001 不在，有新 id 顶替
    let artifact = messages.iter().filter(|m| m["type"] == "artifact_update").last().expect("artifact_update");
    assert!(artifact["candidate"]["work_items"].as_array().unwrap().iter().all(|w| w["id"] != "work_item_0001"));
    // 整组数量不变
    let count = artifact["candidate"]["work_items"].as_array().unwrap().len();
    // （需记录原 candidate 数量，断言 revision 后数量相同）
    // 回到 author_confirm
    let stage = messages.iter().find(|m| m["type"] == "stage_change").expect("stage_change");
    assert_eq!(stage["stage"], "author_confirm");
}

#[tokio::test]
async fn revision_replaces_draft_candidate_without_touching_confirmed_records() {
    // 建 Confirmed plan（另一个 issue/plan），revision 只针对当前 Draft plan，不影响 Confirmed
    // 断言 Confirmed plan 的 work_items 未被删
}
```

> 实现者注意：`MockSplitProviderAdapter` 需配置 revision 调用返回的 redo-only JSON（只返回被重做的新项；保留项由后端合并）。`recv_ws_messages_with_timeout` 收到多次 ArtifactUpdate（revert + revision），取最后一次。

- [ ] **Step 3.2：运行测试，确认失败**

Run: `cargo test --locked --test it_web revert_work_item_triggers_local_redo_in_revision`
Expected: 失败——`RequestRevision` 走普通 `Revision`（Story/Design 流式路径），WorkItemPlan revision 不产生 candidate。

- [ ] **Step 3.3：实现 `complete_work_item_plan_revision`**

`src/product/workspace_engine.rs`，在 `complete_work_item_plan_author` 附近新增：

```rust
impl WorkspaceEngine {
    /// WorkItemPlan Revision 完成：replace Draft candidate → 组装 DTO →
    /// `update_artifact(WorkItemPlanCandidate)`（新 version）→ 回 AuthorConfirm。
    pub async fn complete_work_item_plan_revision(
        &mut self,
        output: WorkItemSplitProviderOutput,
    ) -> Result<(), String> {
        let lifecycle = self.lifecycle_store.clone().ok_or("lifecycle_store unavailable")?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();

        // replace Draft candidate（validator_findings 为空，revision 不重新 validate；
        // 若需 validate，复用 WP2b 的 validate 逻辑——本 WP 先不 validate，由 WP4 Task 4 的
        // AutoRevision 路径处理 validate 失败重生）
        let _ = lifecycle
            .replace_issue_work_item_plan_candidate(&project_id, &issue_id, &plan_id, &output, Vec::new())
            .map_err(|e| format!("replace candidate failed: {e}"))?;

        // 组装 DTO + update_artifact（新 version）
        let candidate = build_work_item_plan_candidate_dto(&lifecycle, &project_id, &issue_id, &plan_id)
            .map_err(|e| format!("build candidate dto failed: {e}"))?;
        self.update_artifact(ArtifactPayload::WorkItemPlanCandidate { candidate }).await;

        // 回 AuthorConfirm
        self.enter_author_confirm(Some("WorkItemPlan 候选已重做，等待确认".to_string())).await;
        Ok(())
    }
}
```

> 实现者注意：revision 后**回 AuthorConfirm**（不直接进 review，让用户再看改完的候选，design :303）。`WorkItemSplitProviderOutput`/`ArtifactPayload`/`build_work_item_plan_candidate_dto` 顶部 `use` 已导入（WP2b）。

- [ ] **Step 3.4：`ProviderRunKind` 加 `WorkItemPlanRevision`**

`src/web/workspace_ws_handler.rs:1293-1298`：

```rust
enum ProviderRunKind {
    Author { content: String },
    AuthorChoiceFollowup { content: String },
    Revision,
    ReviewOnly,
    WorkItemPlanAuthor,
    WorkItemPlanRevision,
}
```

- [ ] **Step 3.5：`RequestRevision` 按 workspace_type 路由**

`src/web/workspace_ws_handler.rs`，`WsInMessage::RequestRevision` 处理（`grep -n "WsInMessage::RequestRevision" src/web/workspace_ws_handler.rs`）。参考 WP2b 的 `StartGeneration` 路由模式：

```rust
            WsInMessage::RequestRevision { feedback } => {
                // ... 现有 engine 进 Revision 阶段逻辑 ...
                let run_kind = {
                    let engine = engine.lock().await;
                    if engine.session().workspace_type == WorkspaceType::WorkItemPlan {
                        ProviderRunKind::WorkItemPlanRevision
                    } else {
                        ProviderRunKind::Revision
                    }
                };
                if let Err(message) = spawn_provider_run_from_handler(run_context.clone(), run_kind).await {
                    let _ = send_json_outbound(&outbound_tx, &WsOutMessage::Error { message }).await;
                }
            }
```

> `feedback` 在 WorkItemPlan 下作为 revision 的整体反馈（与各 WorkItem 的 revert feedback 一起喂 `generate_revision`）。`work_item_plan_author_retry_count` 重置（revision 是用户主动触发，重置计数）。

- [ ] **Step 3.6：review 触发的 StartRevision 路由**

`src/web/workspace_ws_handler.rs`，`handle_review_decision` 返回 `StartRevision` 后的 spawn 逻辑（`grep -n "StartRevision\|ProviderRunKind::Revision" src/web/workspace_ws_handler.rs`）。在 spawn 前按 workspace_type 路由：

```rust
                let run_kind = {
                    let engine = engine.lock().await;
                    if engine.session().workspace_type == WorkspaceType::WorkItemPlan {
                        ProviderRunKind::WorkItemPlanRevision
                    } else {
                        ProviderRunKind::Revision
                    }
                };
                spawn_provider_run_from_handler(run_context.clone(), run_kind).await
```

> review 触发的 revision 是整组可调（design :300），`redo_specs` 为空（无 revert 标记），`feedback` = review findings + summary + 用户 extra_context。Task 3.7 的 `WorkItemPlanRevision` 分支需处理"无 revert 标记的整组 revision"——`retained` 空、`redo_specs` 空，prompt 让 provider 输出完整 split JSON 并走现有 `parse_provider_output`。**这与 AuthorConfirm revert 批量的局部重做不同**：分支内据 `session.artifact` 的 candidate meta 判断有无 reverted 项——有则局部重做（retained=未标记，redo=已标记，provider 只输出 redo 项），无则整组微调（retained=空，redo=空，feedback=review findings）。实现时在分支内组装 `retained`/`redo_specs`。

- [ ] **Step 3.7：`spawn_provider_run_from_handler` 两处 match 加 `WorkItemPlanRevision` 分支**

`src/web/workspace_ws_handler.rs`：

1. **provider 选择 match**（:1362-1374）：`WorkItemPlanRevision` 同 `WorkItemPlanAuthor`——不取 `provider_for_run`，用 `provider_adapter`。参考 WP2b Task 3 Step 3.6 的 `WorkItemPlanAuthor` 处理：

```rust
            ProviderRunKind::WorkItemPlanRevision => {
                engine.session().author_provider.clone() // 占位 name，不使用 provider_for_run
            }
```
`provider_for_run` 对 `WorkItemPlanRevision` 也返回 `None`（与 `WorkItemPlanAuthor` 同）。

2. **run 分发 match**（:1411-1432）：加 `WorkItemPlanRevision` 分支。参考 WP2b 的 `WorkItemPlanAuthor` 分支结构（构造 `WorkItemSplitEngine` + 调 `generate`），改为调 `generate_revision`：

```rust
            ProviderRunKind::WorkItemPlanRevision => {
                // 1. 从 session.artifact 读当前 candidate，区分 retained vs redo
                let (retained, redo_specs, request) = {
                    let engine = engine.lock().await;
                    build_work_item_plan_revision_input(&engine, &lifecycle_for_run, &feedback_for_run)
                        .map_err(|e| format!("build revision input failed: {e}"))?
                };
                // 2. 构造 WorkItemSplitEngine，调 generate_revision
                let split_engine = WorkItemSplitEngine::new(provider_adapter_for_run.clone());
                let repository = /* 同 WorkItemPlanAuthor 分支取 repository */;
                let issue = /* 同取 issue */;
                let author_provider = /* 同取 author_provider */;
                let output = split_engine.generate_revision(&request, &lifecycle_for_run, &issue, &repository, author_provider, &retained, &redo_specs).await
                    .map_err(|e| format!("split generate_revision failed: {e}"))?;
                // 3. engine.complete_work_item_plan_revision
                let mut engine = engine.lock().await;
                engine.complete_work_item_plan_revision(output).await
                    .map_err(|e| format!("complete revision failed: {e}"))?;
            }
```

> ⚠️ 实现者注意（关键复杂点，参考 WP2b Task 3 Step 3.6）：
> 1. **`build_work_item_plan_revision_input(&engine, &lifecycle, &feedback) -> (Vec<LifecycleWorkItemRecord>, Vec<RedoSpec>, GenerateWorkItemsRequest)`**：新 helper。从 `session.artifact` 的 candidate 取 work_items：`meta.reverted==true` 的进 `redo_specs`（old_id + feedback），其余进 `retained`（从 lifecycle 读完整 `LifecycleWorkItemRecord`）。`request` 从 `session.entity_id` 读 Draft plan 组装（同 WP2b 的 `build_work_item_plan_generate_request`）。整组 review 触发的 revision（无 reverted 项）→ `retained` 空、`redo_specs` 空、feedback=review findings——`generate_revision` 需支持 `retained`/`redo_specs` 均空（整组微调）。**若 `generate_revision` 不支持均空，Task 1 的实现需兼容此情况**（prompt 退化为"整组微调"）。
> 2. **`run_context` move**：`ProviderRunContext` 已 `#[derive(Clone)]`（WP2b），分支内 `run_context.clone()`。
> 3. **依赖获取**：复用 WP2b 给 `ProviderRunContext` 加的 `app_paths`/`session_record` 字段（方案 A），闭包内 `LifecycleStore::new(app_paths.clone())` 构造。
> 4. **`feedback_for_run`**：`RequestRevision` 的 `feedback` 或 review findings。需在 spawn 前捕获到闭包外（`run_context` 不带 feedback，需额外捕获——可给 `ProviderRunKind::WorkItemPlanRevision` 加 `feedback: Option<String>` 字段，或通过 engine 的某个临时字段传递）。**建议给 `WorkItemPlanRevision` 加 `feedback: Option<String>` 字段**，`RequestRevision`/review 触发时传入，分支内解构拿到。
> 5. **engine 锁跨 await**：分段 lock，不跨 await 持锁。

- [ ] **Step 3.8：顶部 `use` 补全**

`src/web/workspace_ws_handler.rs:1-34` 补 `WorkItemSplitEngine`、`RedoSpec`（从 `crate::product::work_item_split_engine`）。`cargo check` 报缺哪些补哪些。

- [ ] **Step 3.9：运行 Task 3 测试 + 收口**

Run:
```
cargo test --locked --test it_web revert_work_item_triggers_local_redo_in_revision
cargo test --locked --test it_web revision_replaces_draft_candidate_without_touching_confirmed_records
cargo test --locked --test it_web web_work_item_generation
cargo check --locked
```
Expected: 新测试 PASS；现有测试全绿；`cargo check` 全绿。

- [ ] **Step 3.10：提交**

```bash
git add src/product/workspace_engine.rs src/web/workspace_ws_handler.rs tests/it_web/
git commit -m "feat(WP4): WorkItemPlanRevision run + RequestRevision/review 路由 + complete_work_item_plan_revision"
```

---

## Task 4：迁移 WP2b `AutoRevision` 到 `generate_revision`

**目标**：design 第 269 行要求 validate 失败"自动进入 Revision，prompt 带 validator error findings 让 WorkItemSplitter 立即修正"（用 `generate_revision` 带 feedback）。WP2b 的 `AutoRevision` 实现是"返回 `AutoRevision` 让 handler 重新 spawn `WorkItemPlanAuthor`（调 `generate` 无 feedback 重生）"。本 Task 把 `AutoRevision` 路径迁到 `WorkItemPlanRevision`（调 `generate_revision`，validator error findings 作 feedback），保持 design 最终语义。

**Files:**
- Modify: `src/product/workspace_engine.rs`（`complete_work_item_plan_author` 的 `has_errors` 分支）
- Modify: `src/web/workspace_ws_handler.rs`（`WorkItemPlanAuthor` 分支的 `AutoRevision` outcome 处理）

**Interfaces:**
- Consumes: Task 1 的 `generate_revision`；Task 3 的 `WorkItemPlanRevision`。
- Produces: validate 失败 → 进 Revision 阶段调 `generate_revision`（findings 作 feedback）→ 新 candidate → 再判 AuthorConfirm；连续失败超阈值进 HumanConfirm。

- [ ] **Step 4.1：写失败测试 —— validate 失败自动 revision 用 generate_revision 带 feedback**

在 `tests/it_web/web_work_item_plan_revert.rs` 或 `web_work_item_plan_author.rs`（WP2b）末尾：

```rust
#[tokio::test]
async fn work_item_plan_validate_errors_auto_revision_uses_generate_revision() {
    // 配置 MockSplitProviderAdapter：第一次 generate 返回有 error findings 的 candidate，
    // 第二次 generate_revision 返回修正后的 candidate。
    let (app, _repo) = app_with_confirmed_story_and_design(failing_then_passing_split_output()).await;
    let session_id = prepare_and_start_generation(&app).await;
    let ws = connect_ws(&app, &session_id).await;
    let messages = recv_ws_messages_with_timeout(&ws, timeout).await;
    // 最终进 author_confirm（validate 通过），candidate 是修正后的
    let stage = messages.iter().find(|m| m["type"] == "stage_change").expect("stage_change");
    assert_eq!(stage["stage"], "author_confirm");
    // MockSplitProviderAdapter 应被调用了 generate（首次）+ generate_revision（修正）
    // ——通过 mock 的调用计数断言
}
```

> 实现者注意：`failing_then_passing_split_output` 让 mock 第一次返回 validate 失败的 JSON，第二次（revision）返回通过的 JSON。需 mock 支持"按调用次数返回不同输出"——`grep -n "MockSplitProviderAdapter" tests/it_web/web_work_item_generation.rs` 看现有 mock 是否支持调用序列。若不支持，扩展 mock 或简化测试（只断言最终 stage=author_confirm + 不进 HumanConfirm）。

- [ ] **Step 4.2：运行测试，确认失败**

Run: `cargo test --locked --test it_web work_item_plan_validate_errors_auto_revision`
Expected: 失败——WP2b 的 `AutoRevision` 重新 spawn `WorkItemPlanAuthor`（调 `generate` 无 feedback），mock 第二次仍返回失败 → 进 HumanConfirm（或循环）。

- [ ] **Step 4.3：修改 `complete_work_item_plan_author` 的 `has_errors` 分支**

`src/product/workspace_engine.rs`，WP2b 的 `complete_work_item_plan_author` 内 `if report.has_errors()` 分支（WP2b Task 2 Step 2.4）。原逻辑：计数 + `replace_issue_work_item_plan_candidate`（存带 error 的 candidate）+ 返回 `AutoRevision { findings }`。改为：

```rust
        if report.has_errors() {
            self.work_item_plan_author_retry_count += 1;
            if self.work_item_plan_author_retry_count >= 3 {
                self.enter_human_confirm_for_work_item_plan_author_failure(&findings).await;
                return Ok(WorkItemPlanAuthorOutcome::HumanConfirm {
                    reason: "validate 连续 3 次失败".to_string(),
                });
            }
            // 把 findings 写入 plan.validator_findings（供 revision prompt 用）
            let _ = lifecycle.replace_issue_work_item_plan_candidate(
                &project_id, &issue_id, &plan_id, &output, findings.clone(),
            );
            // 迁移：不再返回 AutoRevision 让 handler 重新 generate；
            // 改为进 Revision 阶段，handler 据 AutoRevision 启动 WorkItemPlanRevision
            // （generate_revision 带 findings 作 feedback）。
            // 仍返回 AutoRevision { findings }，但 handler 的处理改为启动 WorkItemPlanRevision。
            return Ok(WorkItemPlanAuthorOutcome::AutoRevision { findings });
        }
```

> 说明：`AutoRevision` 变体语义不变（仍表示"validate 失败需重生"），但 **handler 的处理方式改变**——从"重新 spawn `WorkItemPlanAuthor`"改为"spawn `WorkItemPlanRevision`（findings 作 feedback）"。这样 `generate_revision` 带 feedback 修正，符合 design :269。

- [ ] **Step 4.4：修改 handler 的 `WorkItemPlanAuthor` 分支 outcome 处理**

`src/web/workspace_ws_handler.rs`，WP2b Task 3 Step 3.6 的 `WorkItemPlanAuthor` 分支内 `match outcome`（WP2b 原代码 `AutoRevision => 重新 spawn WorkItemPlanAuthor`）。改为：

```rust
                match outcome {
                    WorkItemPlanAuthorOutcome::AuthorConfirm => { /* 完成 */ }
                    WorkItemPlanAuthorOutcome::AutoRevision { findings } => {
                        // 迁移：启动 WorkItemPlanRevision（findings 作 feedback），而非重新 spawn WorkItemPlanAuthor
                        drop(engine);
                        let _ = spawn_provider_run_from_handler(
                            run_context_clone,
                            ProviderRunKind::WorkItemPlanRevision { feedback: Some(format_findings_as_feedback(&findings)) },
                        ).await;
                    }
                    WorkItemPlanAuthorOutcome::HumanConfirm { reason: _ } => { /* stage 已进 HumanConfirm */ }
                }
```

> `ProviderRunKind::WorkItemPlanRevision` 需加 `feedback: Option<String>` 字段（Task 3 Step 3.7 已建议）。`format_findings_as_feedback(&[WorkItemSplitFinding]) -> String` 把 validator error findings 格式化为 feedback 文本（severity + code + message + work_item_ids）。
>
> **`WorkItemPlanRevision` 的整组修正语义**：AutoRevision 触发时 `session.artifact` 的 candidate 无 `reverted` 标记（author 刚失败，未进 AuthorConfirm），`build_work_item_plan_revision_input` 返回 `retained` 空、`redo_specs` 空、feedback=findings——`generate_revision` 整组微调修正。Task 1 的 `generate_revision` 需支持 `retained`/`redo_specs` 均空。

- [ ] **Step 4.5：运行 Task 4 测试 + 收口**

Run:
```
cargo test --locked --test it_web work_item_plan_validate_errors_auto_revision
cargo test --locked --test it_web work_item_plan_author
cargo test --locked --test it_web web_work_item_generation
cargo check --locked
```
Expected: 新测试 PASS；WP2b 的 author 测试仍全绿（AutoRevision 语义未变，只是 handler 处理方式变）；`cargo check` 全绿。

> 若 WP2b 的 `work_item_plan_validate_errors_auto_revision` 测试（WP2b Task 4）断言了"重新 spawn WorkItemPlanAuthor"，需更新为"spawn WorkItemPlanRevision"。检查 WP2b 测试并适配。

- [ ] **Step 4.6：提交**

```bash
git add src/product/workspace_engine.rs src/web/workspace_ws_handler.rs tests/it_web/
git commit -m "refactor(WP4): 迁移 AutoRevision 到 generate_revision（对齐 design validate 失败语义）"
```

---

## Task 5：WP4 收口验证（全量回归）

**目标**：跑完整验证链，确保 revert/revision 未破坏 Story/Design/WorkItem 既有流程；WorkItemPlan revert→revision 链路通；AutoRevision 迁移正确。

**Files:** 无新增改动；仅运行验证命令。

- [ ] **Step 5.1：全量验证链**

Run（依次，任一失败即停并修复）:
```
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked --lib workspace_engine
cargo test --locked --test it_product product_work_item_split_engine
cargo test --locked --test it_product product_lifecycle_store
cargo test --locked --test it_web
```
Expected: 全绿。

> `cargo test --locked --test it_web` 全量覆盖 Story/Design/WorkItem/WorkItemPlan prepare/author/review/revert/revision 的 HTTP + WS 流程，是 WP4 最大的回归保障。

- [ ] **Step 5.2：确认 WP1/WP2a/WP2b/WP3 成果未破坏**

Run:
```
cargo test --locked --test it_web prepare_work_item_plan_creates_draft_plan_and_session_without_generating
cargo test --locked --test it_web work_item_plan_start_generation_returns_candidate_artifact
cargo test --locked --test it_web review_returns_verdict_for_whole_candidate
```
Expected: PASS。

- [ ] **Step 5.3：交付摘要（供 WP5 前置交付摘要使用）**

commit 后，把以下内容写入 WP5 plan 的「前置交付摘要」章节：

- `RevertWorkItem` 标记处理：AuthorConfirm 阶段，`engine.apply_revert_mark` 改 candidate meta + 写回当前 `ArtifactVersion.payload` + 推同 version `ArtifactUpdate`（不产新 version）；`clear:true` 取消；断线重连不丢标记。
- `WorkItemPlanRevision` run：`RequestRevision`（WorkItemPlan 下）或 review `StartRevision` 触发，`generate_revision(retained, redo_specs)` + `repatch_dependencies`，`complete_work_item_plan_revision`（replace candidate → 新 version artifact → 回 AuthorConfirm）。
- `generate_revision`：局部重做时 provider 只输出 redo 项，后端保留 retained、为 redo 分配新 id、建立 old→new 映射并 DAG repatch；支持 `retained`/`redo_specs` 均空（整组微调，用于 review/AutoRevision）。
- `AutoRevision` 已迁移：validate 失败 → `WorkItemPlanRevision`（findings 作 feedback）→ 修正；连续 3 次进 HumanConfirm。
- Draft candidate 仍无子 WorkItem session（confirm 时才建，WP5）。
- **WP5 待办**：`handle_confirm` WorkItemPlan 分支调 `confirm_issue_work_item_plan` + 幂等建子 session；删 3 条废弃 REST 路由。

---

## Self-Review（写完后的自查）

**1. Spec 覆盖**（对照总览 v1.1 WP4 目标/写入范围/验证 + 设计方案 :274-305、:326-335）：
- ✅ `RevertWorkItem` 标记处理（candidate meta + 写回当前 ArtifactVersion payload + 推 ArtifactUpdate 同 version）→ Task 2
- ✅ 批量触发 dedicated 非流式 WorkItemPlan Revision → Task 3
- ✅ `WorkItemSplitEngine::generate_revision`（retained + redo_specs）→ Task 1
- ✅ `repatch_dependencies` DAG 重连 → Task 1
- ✅ revision 通过 `replace_issue_work_item_plan_candidate` 替换 Draft candidate → Task 3 `complete_work_item_plan_revision`
- ✅ review 触发的整组 revision 走本 WP → Task 3 Step 3.6
- ✅ revision 回 AuthorConfirm（不直接进 review）→ Task 3 Step 3.3
- ✅ 重做语义（后端保留未标记 + provider redo-only 重做被 revert + DAG 重连 + 整组数量不变）→ Task 1/3
- ✅ `RequestRevision` 复用现有消息 → Task 3 Step 3.5
- ✅ AutoRevision 迁移到 generate_revision（对齐 design :269）→ Task 4
- ✅ 验证命令链 → Task 5
- ✅ 不做项：未实现 confirm（WP5）、未改前端——均在「不做」清单。

**2. Placeholder 扫描**：
- `build_revision_prompt`（Task 1 Step 1.5）：给出职责但未给完整 prompt 文本——因 prompt 需与 `build_split_prompt` 风格对齐，实现时参考 `build_split_prompt`（:21-131）。给出 `grep` 定位，属可接受。
- `build_work_item_plan_revision_input`（Task 3 Step 3.7）：给出签名与职责，实现时从 `session.artifact` 组装。属可接受指引。
- `format_findings_as_feedback`（Task 4 Step 4.4）：给出职责，实现简单。属可接受。
- `parse_revision_redo_output` / `materialize_redo_work_items` / `build_revision_provider_output`（Task 1 Step 1.5）：给出职责和数据边界，具体字段映射需从现有 `parse_provider_output` 抽取。属可接受。
- WS 测试 helper（Task 2/3）：复用 WP2b 共享 helper；若能力不足先补 helper。属可接受。

**3. 类型一致性**：
- `RedoSpec { old_id, feedback }` 在 Task 1 定义，Task 3 `build_work_item_plan_revision_input` 产出。
- `repatch_dependencies(graph, id_mapping) -> Vec<IssueWorkItemDependencyEdge>` 在 Task 1 定义，`generate_revision` 内调用。
- `WorkItemPlanAuthorOutcome::AutoRevision { findings }`（WP2b 定义）Task 4 复用，handler 处理方式改变。
- `ProviderRunKind::WorkItemPlanRevision { feedback: Option<String> }`（Task 3/4 定义），`RequestRevision`/review/AutoRevision 三处触发一致传 feedback。
- `complete_work_item_plan_revision(output: WorkItemSplitProviderOutput) -> Result<(), String>`（Task 3 定义），handler 调用一致。

**4. 边界风险**：
- **revert meta 持久化**（Task 2 Step 2.3）：已定为写回当前 `ArtifactVersion.payload`，不写 lifecycle work_item record。风险点变为"更新同 version 时漏调 `persist_artifact_versions` 导致重连丢失"；WP8 增加 `reconnect_preserves_revert_marks_from_current_artifact_version` 覆盖。
- **`generate_revision` 支持 retained/redo 均空**（Task 1/3/4）：review 触发与 AutoRevision 触发的 revision 无 reverted 标记，`retained`/`redo_specs` 均空，prompt 退化为整组微调。`generate_revision` 需兼容此情况——Task 1 Step 1.5 的 prompt 构建需处理空 retained/redo。**已标注，Task 1 实现时确保支持。**
- **`run_context` move + 重新 spawn**（Task 3 Step 3.7）：`ProviderRunContext` 已 `#[derive(Clone)]`（WP2b），`run_context.clone()`。`WorkItemPlanRevision` 加 `feedback` 字段后 `Clone` 仍成立。已标注。
- **engine 锁跨 await**（Task 3 Step 3.7）：分段 lock，不跨 await 持锁。已标注。
- **AutoRevision 循环收敛**（Task 4）：`work_item_plan_author_retry_count` 计数保证 3 次后进 HumanConfirm。迁移后 AutoRevision → WorkItemPlanRevision → `complete_work_item_plan_revision` 不再走 `complete_work_item_plan_author`（计数不递增）——**需确认计数在 revision 路径也生效**。Task 4 Step 4.3 的迁移保留计数在 `complete_work_item_plan_author` 的 `has_errors` 分支（revision 触发前计数），但 revision 本身若 validate 失败如何计数？**`complete_work_item_plan_revision`（Task 3 Step 3.3）当前不 validate、不计数**——若 revision 输出仍 validate 失败，无计数保护。**风险**：revision 失败循环。缓解：revision 后回 AuthorConfirm（不自动重做），用户可再触发 RequestRevision；AutoRevision 路径的计数在 author 阶段（首次生成）保护，revision 是用户/review 触发的显式动作，不自动循环。**已标注，实现时确认 revision 不引入自动循环。**
- **WP2b 测试适配**（Task 4 Step 4.5）：WP2b 的 `work_item_plan_validate_errors_auto_revision` 测试可能断言"重新 spawn WorkItemPlanAuthor"，迁移后需更新。已标注。
- **`ProviderRunKind::WorkItemPlanRevision` 加 feedback 字段**（Task 3/4）：影响 `spawn_provider_run_from_handler` 的 `run_kind` 解构。所有构造 `WorkItemPlanRevision` 的位置（RequestRevision/review/AutoRevision）需传 feedback。已标注。

---

## Execution Handoff

本 WP4 plan 已保存至 `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP4_后端revert与revision_v1.0.md`。

**执行方式（待用户选择）：**

1. **Subagent-Driven（推荐）** — 每个 Task 派一个 fresh subagent 实现，Task 间做 two-stage review。
2. **Inline Execution** — 在当前 session 按 superpowers:executing-plans 批量执行，带 checkpoint 审查。

**完成后**：交付 WP4 后，按同样标准继续 WP5（后端 confirm 落盘 + 子 session + 删废弃路由）。WP5 的「前置交付摘要」直接引用本 plan Task 5 Step 5.3 的产出。

**⚠️ 实现前注意**：
1. Task 1 的 `generate_revision` 需支持 `retained`/`redo_specs` 均空（整组微调），供 Task 3 的 review 触发与 Task 4 的 AutoRevision 触发使用。
2. Task 2 的 revert meta 持久化策略已定：写回当前 `ArtifactVersion.payload`，不新增 version，不写 lifecycle work_item record。
3. Task 3 Step 3.7 的 `WorkItemPlanRevision` 分支是本 WP 最复杂接入点（依赖获取 + run_context move + retained/redo 组装），建议执行者先完整读 WP2b Task 3 Step 3.6 的 `WorkItemPlanAuthor` 分支实现，再按本 plan 改造。
4. Task 4 修改 WP2b 已写代码（`complete_work_item_plan_author` + handler outcome 处理），需同步更新 WP2b 的相关测试。
