# WorkItem 对话式 Workspace 生成 WP2b：后端 author 生成 + Draft candidate 持久化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** WorkItemPlan 在 `StartGeneration` 后走 dedicated 非流式 author run：从 `session.entity_id` 读取 Draft plan → 组装 `GenerateWorkItemsRequest` 兼容输入 → `WorkItemSplitEngine::generate` → `WorkItemSplitValidator::validate` → `LifecycleStore::replace_issue_work_item_plan_candidate` 持久化 Draft candidate → 组装 `WorkItemPlanCandidateDto` → `update_artifact(ArtifactPayload::WorkItemPlanCandidate)` → 推 `ArtifactUpdate` → 进 AuthorConfirm。validate 失败最小自动重生（连续超阈值进 HumanConfirm）。

**Architecture:** author run 不走 `drive_provider_session` 流式路径。按设计方案 :260-265 选项 2：WS handler 新增 `ProviderRunKind::WorkItemPlanAuthor`，用 `state.provider_adapter`（`Arc<dyn ProviderAdapter>`，非流式）构造 `WorkItemSplitEngine`，调 `engine.generate` 拿 `WorkItemSplitProviderOutput`，再调 `WorkspaceEngine` 的新方法 `complete_work_item_plan_author` 完成 validate → replace candidate → 组装 DTO → `update_artifact` → `enter_author_confirm`。LifecycleStore 新增 `replace_issue_work_item_plan_candidate`（替换 Draft plan 关联的 work_items/verification_plans/repository_profile）+ `delete_verification_plan`/`delete_repository_profile` helper。Draft candidate 是事实来源，artifact payload 是镜像。

**Tech Stack:** Rust 1.95.0、Cargo、Axum、tokio（`spawn_blocking` 已在 `WorkItemSplitEngine::generate` 内）、serde。本 WP 不涉及前端。

**版本：** v1.0
**创建日期：** 2026-06-17
**目标分支：** `feat-b-0616`
**工作区：** `.worktrees/feat-b-0616`
**依赖总览：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_拆分总览_v1.0.md`（v1.1，WP2b 章节）
**设计方案：** `cadence/designs/2026-06-17_技术方案_WorkItem对话式Workspace生成_v1.0.md`（第 240-272 行 author 阶段、第 192-200 行 replace 接口）
**前置 WP：** WP1、WP2a

---

## 全局约束（Global Constraints）

- **运行命令固定**：Rust 1.95.0；cargo 命令带 `--locked`；🔴 **禁止 `-j 1`**。
- **强制检查链**：`cargo fmt --check` + `cargo clippy --all-targets --all-features --locked -- -D warnings` + `cargo check --locked` + 定向测试，四者缺一不可。
- **TDD**：每个 Task 先写失败测试，再写实现。
- **写入范围严格**：只改「File Structure」声明的文件。本 WP 共享 `workspace_engine.rs` / `workspace_ws_handler.rs` / `lifecycle_store.rs`——须在 WP2a 之后串行执行。
- **行号是参考**：基于 `feat-b-0616` HEAD `8a2eee4`；实现时以 `grep -n` 实际定位为准。

---

## 前置交付摘要（来自 WP1 + WP2a）

### 来自 WP1
- `WorkspaceType::WorkItemPlan` 变体（serde `"work_item_plan"`）。
- `prepare_work_item_plan` handler 创建空 Draft `IssueWorkItemPlan`（`work_item_ids`/`verification_plan_ids`/`dependency_graph` 为空）+ `WorkItemPlan` session（`entity_id = plan_id`）。
- `IssueWorkItemPlan` 字段：`id/project_id/issue_id/source_story_spec_ids/source_design_spec_ids/options: IssueWorkItemPlanOptions/status/work_item_ids/repository_profile_ref/verification_plan_ids/dependency_graph/created_from_provider_run/validator_findings/review_summary/created_at/updated_at`。
- `IssueWorkItemPlanOptions`：4 个布尔字段（`include_integration_tests`/`include_e2e_tests`/`force_frontend_backend_split`/`require_execution_plan_confirm`）。
- `WorkItemPlanCandidateDto` + 子 DTO（`WorkItemPlanDto`/`WorkItemCandidateDto`/`WorkItemCandidateMetaDto`/`WorkItemSplitOptionsDto`/`WorkItemDependencyEdgeDto`/`ValidatorFindingDto` + 可能的 `VerificationPlanDto`/`RepositoryProfileDto`）已在 `workspace_ws_types.rs` 定义。

### 来自 WP2a
- `ArtifactPayload` enum 已挂载：`WsOutMessage::ArtifactUpdate { version, #[serde(flatten)] payload: ArtifactPayload }`、`SessionState.artifact: Option<ArtifactPayload>`、`EngineEvent::ArtifactUpdate { version, payload }`、`WorkspaceSession.artifact: Option<ArtifactPayload>`、`ArtifactVersion { #[serde(flatten)] payload }`、`CheckpointRecord.artifact_snapshot: ArtifactPayload`。
- `update_artifact(&mut self, payload: ArtifactPayload)` 签名——WP2b 传 `ArtifactPayload::WorkItemPlanCandidate { candidate }` 推送 candidate。
- `build_review_input` / `build_revision_input` 对 `WorkItemPlanCandidate` 变体当前返回空字符串——WP3 会新增 `build_work_item_plan_review_input` 替代。
- `product` 模块已依赖 `web` 模块（`lifecycle_store.rs:22` 导入 `workspace_ws_types`），`ArtifactPayload`/`WorkItemPlanCandidateDto` 可在 `lifecycle_store.rs`/`workspace_engine.rs` 直接导入使用。

---

## 关键既有事实（避免重新探查）

所有行号基于 `feat-b-0616` HEAD `8a2eee4`，实现时用 `grep -n` 确认。

### `src/product/work_item_split_engine.rs`（747 行）
- `WorkItemSplitEngine { provider_adapter: Arc<dyn ProviderAdapter + Send + Sync> }`，`new(adapter)`（:141-149）。
- `generate(&self, request: &GenerateWorkItemsRequest, lifecycle: &LifecycleStore, issue: &IssueRecord, repository: &RepositoryRecord, author_provider: ProviderName) -> ApiResult<WorkItemSplitProviderOutput>`（:151-265）。内部 :227 `spawn_blocking` 调 provider（3 小时超时，`max_retries: 1`）。返回 `WorkItemSplitProviderOutput { repository_profile, plan, work_items, verification_plans }`（:133-139）。**generate 本身只存 provider run（`save_work_item_split_provider_run`），不持久化 plan/work_items**。
- `build_split_prompt` / `parse_provider_output` / `WORK_ITEM_SPLIT_OUTPUT_SCHEMA`（:21-131）/ `summarize_repository_structure` 均为内部函数，WP2b 不改。
- `GenerateWorkItemsRequest`（`src/web/types.rs:564-579`）字段：`title/story_spec_ids/design_spec_ids/include_integration_tests: Option<bool>/include_e2e_tests/force_frontend_backend_split/require_execution_plan_confirm/author_provider/reviewer_provider/review_rounds/superpowers_enabled/openspec_enabled`。

### `src/product/work_item_split_validator.rs`（629 行）
- `WorkItemSplitValidator::validate(plan: &IssueWorkItemPlan, work_items: &[LifecycleWorkItemRecord], repository_profile: Option<&RepositoryProfile>, verification_plans: &[VerificationPlan]) -> WorkItemSplitValidationReport`（:26-45）。
- `WorkItemSplitValidationReport { findings: Vec<WorkItemSplitFinding> }`，`has_errors()`（:10-21）。**无 `has_warnings()`**。
- `WorkItemSplitFinding { severity: WorkItemSplitFindingSeverity(Error/Warning), code: String, message: String, work_item_ids: Vec<String> }`（models.rs:441-455）。

### `src/product/lifecycle_store.rs`（2054 行）
- `create_issue_work_item_plan(input: CreateIssueWorkItemPlanInput) -> Result<IssueWorkItemPlan, _>`（:407-445）：`write_json` 覆盖式（不检查存在性），`created_at`/`updated_at` = now，`review_summary` 强制 None。
- `create_work_item` / `create_verification_plan` / `create_repository_profile`：均 `write_json` 覆盖式，无存在性检查。
- `delete_work_item(project_id, issue_id, work_item_id) -> Result<(), _>`（:763-786）：调 `delete_required_file` + 删关联 WorkItem session。
- `list_work_items` / `list_verification_plans` / `list_repository_profiles` / `get_issue_work_item_plan` / `list_issue_work_item_plans`：现有 pub fn。
- **无** `delete_verification_plan` / `delete_repository_profile` / `delete_issue_work_item_plan` / `update_issue_work_item_plan`（局部字段更新）——WP2b 新增。
- `delete_required_file(path, kind, id)` / `remove_file_if_exists(path)` / `path_is_regular_file(path)`：现有 private helper（:1801-1847）。
- `ProductStoreError::NotFound { kind: &'static str, id: String }`（json_store.rs:7-17）。
- 路径 root helpers：`work_items_root`/`issue_work_item_plans_root`/`repository_profiles_root`/`verification_plans_root`（:1571-1624）。

### `src/product/workspace_engine.rs`（8362 行）
- `WorkspaceEngine` struct（:333-347）：**不持有 provider_adapter**（靠参数注入）。持有 `lifecycle_store: Option<LifecycleStore>`、`event_tx`、`session: WorkspaceSession`、`artifact_versions`、`cancel: CancellationToken` 等。
- `enter_author_confirm(&mut self, summary: Option<String>)`（:3108-3127）：transition_stage(AuthorConfirm) + 建 AuthorConfirm timeline 节点 + session status = WaitingForHuman。
- `workspace_requires_artifact_gate(&self) -> bool`（:2888-2893）：当前 `matches!(workspace_type, Story | Design)`。WorkItemPlan **不进 markdown gate**（author 完成靠 engine run 返回成功，不靠 `content_has_complete_workspace_artifact`）。
- `workspace_type_title(workspace_type: &WorkspaceType) -> &'static str`（:3740-3746）：需加 WorkItemPlan 分支。
- `update_artifact(&mut self, payload: ArtifactPayload)`（WP2a 已改签名）：WP2b 传 `WorkItemPlanCandidate`。
- `handle_confirm`（:2683-2731）的 `match workspace_type`（:2694）：WorkItemPlan 分支在 WP5 加。
- `transition_stage` / `create_timeline_node` / `append_completed_timeline_event` / `complete_active_node`：现有方法，WP2b 复用。
- 文件顶部 `use`（:1-33）：已导入 `ArtifactPayload`（WP2a）、`WorkspaceType`、`LifecycleStore` 等。WP2b 需补 `WorkItemSplitEngine`、`WorkItemSplitValidator`、`IssueWorkItemPlan`、`GenerateWorkItemsRequest` 等导入。

### `src/web/workspace_ws_handler.rs`（1553 行）
- `ProviderRunContext { provider_registry, engine, current_run, workspace_runs, session_id, next_run_id }`（:773-781，`#[derive(Clone)]`）——WP2b 加 `provider_adapter: Arc<dyn ProviderAdapter + Send + Sync>` 字段。
- `run_context` 构造（:396-403）——补 `provider_adapter: state.provider_adapter.clone()`。
- `ProviderRunKind { Author { content }, AuthorChoiceFollowup { content }, Revision, ReviewOnly }`（:1293-1298）——WP2b 加 `WorkItemPlanAuthor` 变体。
- `spawn_provider_run_from_handler(run_context, run_kind)`（:1347-1470）：解构 `ProviderRunContext`，:1362-1374 选 provider，:1408-1432 spawn task 内 match run_kind 分发。WP2b 两处 match 加 `WorkItemPlanAuthor` 分支。
- `StartGeneration` 处理（:677-707）：调 `engine.start_generation` + `spawn_provider_run_from_handler(ProviderRunKind::Author { content: "" })`。WP2b 按 `workspace_type == WorkItemPlan` 路由到 `ProviderRunKind::WorkItemPlanAuthor`。
- event forwarder（:248-392）：WP2a 已适配 `EngineEvent::ArtifactUpdate { version, payload }`。WP2b 不改。
- `state.provider_adapter: Arc<dyn ProviderAdapter + Send + Sync>`（state.rs:91）——构造 `WorkItemSplitEngine` 的依赖。
- 文件顶部 `use`（:1-34）：未导入 `ProviderAdapter`/`WorkItemSplitEngine`——WP2b 补。

### `src/web/handlers.rs` 的 `persist_work_item_split_provider_output`（:589-703）
顺序：create_repository_profile → 循环 create_verification_plan → create_issue_work_item_plan → 循环 create_work_item（plan_status: Draft）→ 循环 create_workspace_session（WorkItem）。**WP2b 的 `replace_issue_work_item_plan_candidate` 参考此顺序，但"替换"语义：先删旧关联记录，再 create 新的，不建子 session（confirm 时才建，WP5）。**

---

## File Structure

| 文件 | 操作 | 职责 / 本 WP 改动 |
|---|---|---|
| `src/product/lifecycle_store.rs` | M | 新增 `replace_issue_work_item_plan_candidate`（删旧 Draft work_items/verification_plans/repository_profile + create 新的 + 更新 plan 引用）；新增 `delete_verification_plan` / `delete_repository_profile` helper；新增 `update_issue_work_item_plan`（局部字段更新，保留 created_at/review_summary） |
| `src/product/workspace_engine.rs` | M | 新增 `complete_work_item_plan_author(&mut self, output: WorkItemSplitProviderOutput) -> Result<(), String>`：validate → replace candidate → 组装 DTO → `update_artifact(WorkItemPlanCandidate)` → `enter_author_confirm`；`workspace_requires_artifact_gate` 保持不含 WorkItemPlan；`workspace_type_title` 加 WorkItemPlan 分支；validate 失败最小自动重生（计数，超阈值进 HumanConfirm） |
| `src/web/workspace_ws_handler.rs` | M | `ProviderRunKind` 加 `WorkItemPlanAuthor`；`ProviderRunContext` 加 `provider_adapter` 字段；`run_context` 构造补 `state.provider_adapter.clone()`；`StartGeneration` 按 workspace_type 路由；`spawn_provider_run_from_handler` 两处 match 加 `WorkItemPlanAuthor` 分支（构造 `WorkItemSplitEngine` + 调 `engine.generate` + `engine.complete_work_item_plan_author`） |
| `tests/it_product/product_lifecycle_store.rs` | M | `replace_issue_work_item_plan_candidate` 单测 |
| `tests/it_web/web_work_item_generation.rs` 或新增 `web_work_item_plan_author.rs` | M/N | author run 集成测试 |
| `tests/it_web.rs` | M | 若新增 `web_work_item_plan_author.rs` mod，加 `#[path]` 注册 |

**不改：**
- ❌ `src/product/work_item_split_engine.rs`（只调用 `generate`，不改内核）
- ❌ `src/product/work_item_split_validator.rs`（只调用 `validate`）
- ❌ `src/web/workspace_ws_types.rs`（WP1/WP2a 已完成）
- ❌ `src/web/handlers.rs` / `app.rs` / `workspace_context.rs`（WP1 已完成；WP5 才删废弃路由）
- ❌ 前端（WP6/WP7）

---

## Task 1：LifecycleStore —— `replace_issue_work_item_plan_candidate` + delete helpers + plan 局部更新

**目标**：实现 Draft candidate 的替换语义：删旧关联记录（work_items/verification_plans/repository_profile）+ create 新的 + 更新 plan 引用（`work_item_ids`/`verification_plan_ids`/`repository_profile_ref`/`dependency_graph`/`created_from_provider_run`/`validator_findings`），保留 plan 的 `created_at`/`review_summary`。前置校验：plan 必须 `Draft`，不得替换 `Confirmed` plan。

**Files:**
- Modify: `src/product/lifecycle_store.rs`
- Test: `tests/it_product/product_lifecycle_store.rs`

**Interfaces:**
- Consumes: 现有 `create_work_item`/`create_verification_plan`/`create_repository_profile`/`get_issue_work_item_plan`/`list_work_items`/`delete_work_item`/`delete_required_file`/`remove_file_if_exists`。
- Produces:
  - `replace_issue_work_item_plan_candidate(project_id, issue_id, plan_id, output: &WorkItemSplitProviderOutput, validator_findings: Vec<WorkItemSplitFinding>) -> Result<WorkItemPlanCandidateSnapshot, ProductStoreError>`
  - `delete_verification_plan(project_id, issue_id, plan_id) -> Result<(), _>`
  - `delete_repository_profile(project_id, issue_id, profile_id) -> Result<(), _>`
  - `update_issue_work_item_plan(project_id, issue_id, plan_id, update: IssueWorkItemPlanUpdate) -> Result<IssueWorkItemPlan, _>`

- [ ] **Step 1.1：写失败测试 —— replace 替换 Draft candidate 记录**

在 `tests/it_product/product_lifecycle_store.rs` 末尾追加。参考现有 `confirm_issue_work_item_plan_marks_work_items_confirmed`（:763-912）的夹具模式。

```rust
#[test]
fn replace_issue_work_item_plan_candidate_swaps_draft_work_items_and_updates_plan() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    // 建旧 candidate：plan + 2 个 work_item + 1 verification_plan + 1 repository_profile
    let plan = store.create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
        id: None, project_id: "project_0001".into(), issue_id: "issue_0001".into(),
        source_story_spec_ids: vec!["story_spec_0001".into()],
        source_design_spec_ids: vec!["design_spec_0001".into()],
        options: IssueWorkItemPlanOptions { include_integration_tests: true, include_e2e_tests: false, force_frontend_backend_split: false, require_execution_plan_confirm: false },
        status: IssueWorkItemPlanStatus::Draft,
        work_item_ids: vec!["work_item_0001".into(), "work_item_0002".into()],
        repository_profile_ref: Some("repository_profile_0001".into()),
        verification_plan_ids: vec!["verification_plan_0001".into()],
        dependency_graph: vec![IssueWorkItemDependencyEdge { from_work_item_id: "work_item_0001".into(), to_work_item_id: "work_item_0002".into() }],
        created_from_provider_run: None, validator_findings: vec![],
    }).expect("plan");
    // ... 建旧 work_item_0001/0002、verification_plan_0001、repository_profile_0001（省略，照现有夹具） ...

    // 构造新 provider output（2 个新 work_item，新 verification_plan，新 profile）。
    // output.plan.id 可与 prepare 创建的 plan.id 不同；replace 必须忽略 output.plan.id，
    // 以传入的 plan.id / session.entity_id 为事实来源。
    let new_output = /* 构造 WorkItemSplitProviderOutput，work_items 含 2 项新 id，output.plan.id = "issue_work_item_plan_9999" */;

    let snapshot = store.replace_issue_work_item_plan_candidate(
        "project_0001", "issue_0001", &plan.id, &new_output, vec![]
    ).expect("replace");

    // 旧 work_item/verification_plan/repository_profile 被删除
    let work_items = store.list_work_items("project_0001", "issue_0001").unwrap();
    assert!(work_items.iter().all(|wi| wi.id != "work_item_0001" && wi.id != "work_item_0002"));
    // 新 work_item 存在
    assert_eq!(snapshot.work_item_ids.len(), 2);
    // plan 引用更新
    let plan_after = store.get_issue_work_item_plan("project_0001", "issue_0001", &plan.id).unwrap();
    assert_eq!(plan_after.work_item_ids, snapshot.work_item_ids);
    assert_eq!(plan_after.verification_plan_ids, snapshot.verification_plan_ids);
    assert_eq!(plan_after.repository_profile_ref.as_deref(), Some(&snapshot.repository_profile_id));
    // plan 仍 Draft，created_at 保留
    assert_eq!(plan_after.status, IssueWorkItemPlanStatus::Draft);
    assert_eq!(plan_after.id, plan.id);
    assert!(store.list_issue_work_item_plans("project_0001", "issue_0001").unwrap()
        .iter()
        .all(|p| p.id != "issue_work_item_plan_9999"));
    assert_eq!(plan_after.created_at, plan.created_at);
}

#[test]
fn replace_issue_work_item_plan_candidate_rejects_confirmed_plan() {
    // ... 建 Confirmed plan ...
    let result = store.replace_issue_work_item_plan_candidate(...);
    assert!(result.is_err());
    // 错误信息含 "not_draft" 或 plan 状态相关
}
```

> 实现者注意：`WorkItemSplitProviderOutput` 构造较繁（含 `RepositoryProfile`/`IssueWorkItemPlan`/`Vec<LifecycleWorkItemRecord>`/`Vec<VerificationPlan>`），参考 `tests/it_web/web_work_item_generation.rs` 的 `valid_split_output()`（:416-503 附近）构造逻辑，或抽一个 test helper。`IssueWorkItemDependencyEdge`/`IssueWorkItemPlanOptions`/`IssueWorkItemPlanStatus` 需在 test mod `use`。

- [ ] **Step 1.2：运行测试，确认失败**

Run: `cargo test --locked --test it_product replace_issue_work_item_plan_candidate`
Expected: 编译失败——`replace_issue_work_item_plan_candidate` 未定义。

- [ ] **Step 1.3：新增 `delete_verification_plan` / `delete_repository_profile` helper**

在 `src/product/lifecycle_store.rs`，参考 `delete_work_item`（:763-786）与 `delete_story_spec`（:255）模式。先 `grep -n "fn delete_work_item\|fn delete_required_file\|fn verification_plans_root\|fn repository_profiles_root" src/product/lifecycle_store.rs` 定位插入点与路径 helper。

```rust
pub fn delete_verification_plan(
    &self,
    project_id: &str,
    issue_id: &str,
    verification_plan_id: &str,
) -> Result<(), ProductStoreError> {
    let path = self
        .verification_plans_root(project_id, issue_id)
        .join(format!("{verification_plan_id}.json"));
    delete_required_file(&path, "verification_plan", verification_plan_id)
}

pub fn delete_repository_profile(
    &self,
    project_id: &str,
    issue_id: &str,
    repository_profile_id: &str,
) -> Result<(), ProductStoreError> {
    let path = self
        .repository_profiles_root(project_id, issue_id)
        .join(format!("{repository_profile_id}.json"));
    delete_required_file(&path, "repository_profile", repository_profile_id)
}
```

> `delete_required_file(path, kind, id)` 签名以实际为准（:1811-1823）——可能是 `delete_required_file(path: &Path, kind: &str, id: &str)`。照搬现有 `delete_work_item` 的调用方式。

- [ ] **Step 1.4：新增 `update_issue_work_item_plan`（局部字段更新）**

参考 `confirm_issue_work_item_plan`（:473-529）的 read-modify-write 模式。定义 `IssueWorkItemPlanUpdate` 输入结构（只含待更新字段）。

```rust
#[derive(Debug, Clone)]
pub struct IssueWorkItemPlanUpdate {
    pub work_item_ids: Vec<String>,
    pub verification_plan_ids: Vec<String>,
    pub repository_profile_ref: Option<String>,
    pub dependency_graph: Vec<IssueWorkItemDependencyEdge>,
    pub created_from_provider_run: Option<String>,
    pub validator_findings: Vec<WorkItemSplitFinding>,
}

pub fn update_issue_work_item_plan(
    &self,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    update: IssueWorkItemPlanUpdate,
) -> Result<IssueWorkItemPlan, ProductStoreError> {
    let mut plan = self.get_issue_work_item_plan(project_id, issue_id, plan_id)?;
    plan.work_item_ids = update.work_item_ids;
    plan.verification_plan_ids = update.verification_plan_ids;
    plan.repository_profile_ref = update.repository_profile_ref;
    plan.dependency_graph = update.dependency_graph;
    plan.created_from_provider_run = update.created_from_provider_run;
    plan.validator_findings = update.validator_findings;
    plan.updated_at = Utc::now().to_rfc3339();
    // 保留 created_at / review_summary / status / source_*/options 不变
    let path = self
        .issue_work_item_plans_root(project_id, issue_id)
        .join(format!("{plan_id}.json"));
    write_json(&path, &plan)?;
    Ok(plan)
}
```

> `IssueWorkItemPlanUpdate` 放在 `lifecycle_store.rs` 的 Input 结构体区（:33-195）。`IssueWorkItemDependencyEdge`/`WorkItemSplitFinding` 顶部 `use` 已导入（:1-30）。

- [ ] **Step 1.5：实现 `replace_issue_work_item_plan_candidate`**

```rust
pub struct WorkItemPlanCandidateSnapshot {
    pub plan_id: String,
    pub work_item_ids: Vec<String>,
    pub verification_plan_ids: Vec<String>,
    pub repository_profile_id: String,
}

pub fn replace_issue_work_item_plan_candidate(
    &self,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    output: &WorkItemSplitProviderOutput,
    validator_findings: Vec<WorkItemSplitFinding>,
) -> Result<WorkItemPlanCandidateSnapshot, ProductStoreError> {
    let existing = self.get_issue_work_item_plan(project_id, issue_id, plan_id)?;
    if existing.status != IssueWorkItemPlanStatus::Draft {
        return Err(ProductStoreError::Io(format!(
            "issue_work_item_plan_not_draft: {plan_id} status={:?}",
            existing.status
        )));
    }

    // 1. 删旧关联记录（work_items / verification_plans / repository_profile）
    for old_wi_id in &existing.work_item_ids {
        // 用 remove_file_if_exists 容忍已删除；不调 delete_work_item（会连带删 WorkItem session，
        // 但 Draft candidate 阶段无子 session——安全起见仍用 remove_file_if_exists 只删 work_item 文件）
        let path = self.work_items_root(project_id, issue_id).join(format!("{old_wi_id}.json"));
        let _ = remove_file_if_exists(&path);
    }
    for old_vp_id in &existing.verification_plan_ids {
        let _ = self.delete_verification_plan(project_id, issue_id, old_vp_id);
    }
    if let Some(old_profile_id) = &existing.repository_profile_ref {
        let _ = self.delete_repository_profile(project_id, issue_id, old_profile_id);
    }

    // 2. create 新的 repository_profile / verification_plans / work_items（复用 provider output 的 id）
    self.create_repository_profile(CreateRepositoryProfileInput {
        id: Some(output.repository_profile.id.clone()),
        project_id: project_id.into(), issue_id: issue_id.into(),
        repository_id: output.repository_profile.repository_id.clone(),
        // ... 其余字段从 output.repository_profile 拷贝 ...
    })?;
    for vp in &output.verification_plans {
        self.create_verification_plan(CreateVerificationPlanInput {
            id: Some(vp.id.clone()),
            // ... 从 vp 拷贝 ...
        })?;
    }
    for wi in &output.work_items {
        self.create_work_item(CreateWorkItemInput {
            id: Some(wi.id.clone()),
            project_id: project_id.into(), issue_id: issue_id.into(),
            // ... 从 wi 拷贝，plan_status: WorkItemPlanStatus::Draft ...
            plan_status: WorkItemPlanStatus::Draft,
            ..Default::default()
        })?;
    }

    // 3. 更新 plan 引用（保留 created_at/review_summary/status=Draft）
    //    注意：忽略 output.plan.id；prepare 阶段创建的 plan_id / session.entity_id 是唯一事实来源。
    let new_wi_ids: Vec<String> = output.work_items.iter().map(|wi| wi.id.clone()).collect();
    let new_vp_ids: Vec<String> = output.verification_plans.iter().map(|vp| vp.id.clone()).collect();
    let new_profile_id = output.repository_profile.id.clone();
    let new_graph = output.plan.dependency_graph.clone();
    let provider_run_ref = output.plan.created_from_provider_run.clone();
    self.update_issue_work_item_plan(project_id, issue_id, plan_id, IssueWorkItemPlanUpdate {
        work_item_ids: new_wi_ids.clone(),
        verification_plan_ids: new_vp_ids.clone(),
        repository_profile_ref: Some(new_profile_id.clone()),
        dependency_graph: new_graph,
        created_from_provider_run: provider_run_ref,
        validator_findings,
    })?;

    Ok(WorkItemPlanCandidateSnapshot {
        plan_id: plan_id.to_string(),
        work_item_ids: new_wi_ids,
        verification_plan_ids: new_vp_ids,
        repository_profile_id: new_profile_id,
    })
}
```

> 实现者注意：
> 1. `CreateRepositoryProfileInput`/`CreateVerificationPlanInput`/`CreateWorkItemInput` 的字段以 `lifecycle_store.rs:42-139` 实际定义为准——从 `output.repository_profile`/`output.verification_plans[i]`/`output.work_items[i]` 逐字段拷贝。参考 `persist_work_item_split_provider_output`（handlers.rs:589-703）的拷贝方式。
> 2. **不建子 WorkItem session**（confirm 时才建，WP5）——这是 Draft candidate 与 P3 REST 流程的关键区别。
> 3. `WorkItemSplitProviderOutput` 与 `WorkItemPlanCandidateSnapshot` 需在 `lifecycle_store.rs` 顶部 `use`（从 `crate::product::work_item_split_engine` 导入 `WorkItemSplitProviderOutput`；`WorkItemPlanCandidateSnapshot` 定义在本文件）。`WorkItemPlanCandidateSnapshot` 也可放 `models.rs`——以简洁为准，本 plan 建议放 `lifecycle_store.rs`（仅 store 内部 + engine 消费）。
> 4. 删旧 work_item 用 `remove_file_if_exists` 而非 `delete_work_item`——后者会调 `delete_workspace_sessions_for_entity` 删 WorkItem session，但 Draft candidate 阶段无子 session，两者等价；用 `remove_file_if_exists` 更直白表达"只清 work_item 文件"。

- [ ] **Step 1.6：运行 Task 1 测试 + 收口**

Run:
```
cargo test --locked --test it_product replace_issue_work_item_plan_candidate
cargo test --locked --test it_product product_lifecycle_store
cargo check --locked
```
Expected: 两个新测试 PASS；现有 lifecycle_store 测试全绿（delete helper 是纯新增）；`cargo check` 全绿。

- [ ] **Step 1.7：提交**

```bash
git add src/product/lifecycle_store.rs tests/it_product/product_lifecycle_store.rs
git commit -m "feat(WP2b): LifecycleStore replace_issue_work_item_plan_candidate + delete helpers"
```

---

## Task 2：WorkspaceEngine —— WorkItemPlan author run 完成 + DTO 组装 + 自动重生

**目标**：新增 `complete_work_item_plan_author` 方法：接收 `WorkItemSplitProviderOutput` → `WorkItemSplitValidator::validate` → 按 findings 分支（has_errors 计数重生 / warnings 随 candidate 推送）→ `replace_issue_work_item_plan_candidate` → 组装 `WorkItemPlanCandidateDto` → `update_artifact(ArtifactPayload::WorkItemPlanCandidate)` → `enter_author_confirm`。`workspace_type_title` 加 WorkItemPlan 分支。validate 失败最小自动重生（本 Task 实现重生计数 + 超阈值进 HumanConfirm 的骨架，实际重生由 handler 层重新调 `WorkItemSplitEngine::generate` 触发——见 Task 3）。

**Files:**
- Modify: `src/product/workspace_engine.rs`
- Test: `src/product/workspace_engine.rs` 的 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: Task 1 的 `replace_issue_work_item_plan_candidate`；`WorkItemSplitValidator::validate`；WP2a 的 `update_artifact(payload: ArtifactPayload)`；WP1 的 `WorkItemPlanCandidateDto` 及子 DTO；`lifecycle.get_issue_work_item_plan`/`list_work_items`/`get_verification_plan`/`get_repository_profile`。
- Produces: `WorkspaceEngine::complete_work_item_plan_author(&mut self, output: WorkItemSplitProviderOutput) -> Result<WorkItemPlanAuthorOutcome, String>`；`build_work_item_plan_candidate_dto(lifecycle, plan_id) -> Result<WorkItemPlanCandidateDto, ProductStoreError>`（free function 或 engine 关联函数）。

- [ ] **Step 2.1：写失败测试 —— complete_work_item_plan_author 推送 candidate 并进 AuthorConfirm**

在 `src/product/workspace_engine.rs` 的 `#[cfg(test)] mod tests` 末尾追加。参考现有 engine 测试夹具（构造 LifecycleStore + Draft plan + session + engine）。

```rust
#[tokio::test]
async fn complete_work_item_plan_author_pushes_candidate_and_enters_author_confirm() {
    let (lifecycle, plan_id, mut engine) = /* 构造 WorkItemPlan session 的 engine 夹具 */;
    let output = /* WorkItemSplitProviderOutput，2 个 work_item */;

    let outcome = engine.complete_work_item_plan_author(output).await.expect("author");
    assert!(matches!(outcome, WorkItemPlanAuthorOutcome::AuthorConfirm));

    // session.artifact 是 WorkItemPlanCandidate payload
    let artifact = engine.session().artifact.as_ref().expect("artifact");
    assert!(matches!(artifact, ArtifactPayload::WorkItemPlanCandidate { .. }));
    // stage = AuthorConfirm
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
    // candidate 含 2 个 work_item
    if let ArtifactPayload::WorkItemPlanCandidate { candidate } = artifact {
        assert_eq!(candidate.work_items.len(), 2);
    }
}
```

> 实现者注意：engine 夹具构造参考现有 `new_persistent` 测试（:470-524 附近）。需要建 repository/issue/story/design/Draft plan/WorkItemPlan session。`WorkItemSplitProviderOutput` 构造参考 Task 1 测试。`WorkItemPlanAuthorOutcome` enum 在本 Task 定义（`AuthorConfirm` / `AutoRevision { findings }` / `HumanConfirm { reason }` 变体）。

- [ ] **Step 2.2：运行测试，确认失败**

Run: `cargo test --locked --lib complete_work_item_plan_author`
Expected: 编译失败——`complete_work_item_plan_author` 未定义。

- [ ] **Step 2.3：定义 `WorkItemPlanAuthorOutcome` enum + `build_work_item_plan_candidate_dto`**

在 `src/product/workspace_engine.rs` 顶部（enum 区，邻近 `WorkspaceStage` :113）定义：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkItemPlanAuthorOutcome {
    /// validate 通过（warnings 随 candidate 推送），进 AuthorConfirm。
    AuthorConfirm,
    /// validate 有 errors，需 handler 层重新调 WorkItemSplitEngine::generate 重生。
    /// findings 作为 revision feedback 注入重生 prompt。
    AutoRevision { findings: Vec<WorkItemSplitFinding> },
    /// 连续重生超阈值（3 次）仍 has_errors，交用户决策。
    HumanConfirm { reason: String },
}
```

新增 `build_work_item_plan_candidate_dto`（free function，从 lifecycle 记录组装 DTO）：

```rust
fn build_work_item_plan_candidate_dto(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> Result<WorkItemPlanCandidateDto, ProductStoreError> {
    let plan = lifecycle.get_issue_work_item_plan(project_id, issue_id, plan_id)?;
    let work_items = lifecycle.list_work_items(project_id, issue_id)?;
    let plan_work_items: Vec<&LifecycleWorkItemRecord> = work_items
        .iter()
        .filter(|wi| plan.work_item_ids.contains(&wi.id))
        .collect();
    let verification_plans: Vec<VerificationPlanDto> = plan.verification_plan_ids
        .iter()
        .filter_map(|vp_id| lifecycle.get_verification_plan(project_id, issue_id, vp_id).ok())
        .map(|vp| VerificationPlanDto::from(&vp))  // 或手写字段映射
        .collect();
    let repository_profile = plan.repository_profile_ref
        .as_ref()
        .and_then(|rid| lifecycle.get_repository_profile(project_id, issue_id, rid).ok())
        .map(|rp| RepositoryProfileDto::from(&rp));
    let work_item_dtos: Vec<WorkItemCandidateDto> = plan_work_items
        .iter()
        .map(|wi| WorkItemCandidateDto {
            id: wi.id.clone(),
            kind: format!("{:?}", wi.kind).to_lowercase(),
            title: wi.title.clone(),
            depends_on: wi.depends_on.clone(),
            exclusive_write_scopes: wi.exclusive_write_scopes.clone(),
            verification_plan_ref: wi.verification_plan_ref.clone(),
            meta: WorkItemCandidateMetaDto { reverted: false, revert_feedback: None },
        })
        .collect();
    Ok(WorkItemPlanCandidateDto {
        plan: WorkItemPlanDto {
            id: plan.id.clone(),
            status: format!("{:?}", plan.status).to_lowercase(),
            options: WorkItemSplitOptionsDto {
                include_integration_tests: plan.options.include_integration_tests,
                include_e2e_tests: plan.options.include_e2e_tests,
                force_frontend_backend_split: plan.options.force_frontend_backend_split,
                require_execution_plan_confirm: plan.options.require_execution_plan_confirm,
            },
            dependency_graph: plan.dependency_graph.iter().map(|e| WorkItemDependencyEdgeDto {
                from_work_item_id: e.from_work_item_id.clone(),
                to_work_item_id: e.to_work_item_id.clone(),
            }).collect(),
        },
        work_items: work_item_dtos,
        verification_plans,
        repository_profile,
        validator_findings: plan.validator_findings.iter().map(|f| ValidatorFindingDto {
            severity: format!("{:?}", f.severity).to_lowercase(),
            code: f.code.clone(),
            message: f.message.clone(),
            work_item_ids: f.work_item_ids.clone(),
        }).collect(),
    })
}
```

> 实现者注意：
> 1. `VerificationPlanDto`/`RepositoryProfileDto` 的字段映射以 WP1 实际定义为准——若 WP1 定义了 `From<&VerificationPlan>`，用之；否则手写字段映射。先 `grep -n "struct VerificationPlanDto\|struct RepositoryProfileDto" src/web/workspace_ws_types.rs src/web/types.rs` 确认定义位置与字段。
> 2. `WorkItemKind`/`IssueWorkItemPlanStatus` 的 `format!("{:?}", ...).to_lowercase()` 与 serde `snake_case` 一致——但更稳妥用 `as_str()` 若有。先 `grep -n "fn as_str" src/product/models.rs` 确认。
> 3. `LifecycleWorkItemRecord`/`VerificationPlan`/`RepositoryProfile`/`WorkItemSplitFinding` 需在 engine.rs 顶部 `use`（部分已导入，补齐）。

- [ ] **Step 2.4：实现 `complete_work_item_plan_author`**

```rust
impl WorkspaceEngine {
    pub async fn complete_work_item_plan_author(
        &mut self,
        output: WorkItemSplitProviderOutput,
    ) -> Result<WorkItemPlanAuthorOutcome, String> {
        let lifecycle = self.lifecycle_store.clone().ok_or("lifecycle_store unavailable")?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();

        // 1. validate
        let report = WorkItemSplitValidator::validate(
            &output.plan,
            &output.work_items,
            Some(&output.repository_profile),
            &output.verification_plans,
        );
        let findings = report.findings.clone();

        if report.has_errors() {
            // 自动重生：本方法不直接重生（重生需 WorkItemSplitEngine，由 handler 层驱动）。
            // 返回 AutoRevision，handler 层据此重新 spawn WorkItemPlanAuthor run。
            // 计数：engine 内维护 self.work_item_plan_author_retry_count（新增字段，u32）。
            self.work_item_plan_author_retry_count += 1;
            if self.work_item_plan_author_retry_count >= 3 {
                self.enter_human_confirm_for_work_item_plan_author_failure(&findings).await;
                return Ok(WorkItemPlanAuthorOutcome::HumanConfirm {
                    reason: "validate 连续 3 次失败".to_string(),
                });
            }
            // 把 findings 写入 plan.validator_findings（供重生 prompt 用），但暂不进 AuthorConfirm
            let _ = lifecycle.replace_issue_work_item_plan_candidate(
                &project_id, &issue_id, &plan_id, &output, findings.clone(),
            );
            return Ok(WorkItemPlanAuthorOutcome::AutoRevision { findings });
        }

        // 2. validate 通过（warnings only 或无 findings）→ replace candidate（findings 写入 plan）
        let _ = lifecycle.replace_issue_work_item_plan_candidate(
            &project_id, &issue_id, &plan_id, &output, findings.clone(),
        ).map_err(|e| format!("replace candidate failed: {e}"))?;

        // 3. 组装 DTO + update_artifact
        let candidate = build_work_item_plan_candidate_dto(&lifecycle, &project_id, &issue_id, &plan_id)
            .map_err(|e| format!("build candidate dto failed: {e}"))?;
        self.update_artifact(ArtifactPayload::WorkItemPlanCandidate { candidate }).await;

        // 4. 进 AuthorConfirm
        self.enter_author_confirm(Some("WorkItemPlan 候选已生成，等待确认".to_string())).await;

        // 重置计数
        self.work_item_plan_author_retry_count = 0;
        Ok(WorkItemPlanAuthorOutcome::AuthorConfirm)
    }

    async fn enter_human_confirm_for_work_item_plan_author_failure(
        &mut self,
        _findings: &[WorkItemSplitFinding],
    ) {
        // 进 HumanConfirm 阶段，交用户决策（RequestChange 触发重生 / Terminate 废弃）
        self.transition_stage(WorkspaceStage::HumanConfirm).await;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            );
        }
        let _ = self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::HumanConfirm,
            agent: None,
            stage: WorkspaceStage::HumanConfirm,
            round: None,
            title: "WorkItemPlan validate 连续失败".to_string(),
            summary: Some("author 多次重生仍 validate 失败，需人工介入".to_string()),
            status: TimelineNodeStatus::Active,
        }).await;
    }
}
```

> 实现者注意：
> 1. `WorkspaceEngine` struct（:333-347）新增字段 `work_item_plan_author_retry_count: u32`，`Default::default()` 或 `new_persistent`/`new` 初始化为 0。
> 2. `TimelineNodeDraft`/`TimelineNodeType::HumanConfirm`/`TimelineNodeStatus::Active` 以现有定义为准（`grep -n "struct TimelineNodeDraft\|enum TimelineNodeType" src/`）。
> 3. `WorkItemSplitValidator`/`WorkItemSplitFinding`/`ArtifactPayload`/`WorkItemPlanCandidateDto` 等需在 engine.rs 顶部 `use` 补齐。
> 4. `WorkItemSplitProviderOutput` 从 `crate::product::work_item_split_engine` 导入。

- [ ] **Step 2.5：`workspace_type_title` 加 WorkItemPlan 分支**

`src/product/workspace_engine.rs:3740-3746`：

```rust
fn workspace_type_title(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "Story Spec",
        WorkspaceType::Design => "Design Spec",
        WorkspaceType::WorkItem => "Work Item",
        WorkspaceType::WorkItemPlan => "Work Item Plan",
    }
}
```

- [ ] **Step 2.6：`workspace_requires_artifact_gate` 保持不含 WorkItemPlan**

确认 `:2888-2893` 当前 `matches!(workspace_type, Story | Design)`——WorkItemPlan 不在列表，返回 false，不进 markdown gate。**本步不改代码**，仅确认 WorkItemPlan author 完成靠 `complete_work_item_plan_author` 显式调 `enter_author_confirm`，不靠 `content_has_complete_workspace_artifact`。

- [ ] **Step 2.7：运行 Task 2 测试 + 收口**

Run:
```
cargo test --locked --lib complete_work_item_plan_author
cargo test --locked --lib workspace_engine
cargo check --locked
```
Expected: 新测试 PASS；现有 engine 测试全绿；`cargo check` 全绿。

> 若 `cargo check` 报其他文件有非穷尽 match（因 `WorkspaceType::WorkItemPlan` 在 WP1 已加，本 WP 不应新增此类错误——WP1 已处理），属 WP1 遗漏，回 WP1 修。

- [ ] **Step 2.8：提交**

```bash
git add src/product/workspace_engine.rs
git commit -m "feat(WP2b): WorkspaceEngine complete_work_item_plan_author + candidate DTO 组装"
```

---

## Task 3：WS handler 接入 —— `WorkItemPlanAuthor` run kind + StartGeneration 路由

**目标**：handler 在 `StartGeneration` 且 `workspace_type == WorkItemPlan` 时启动 `ProviderRunKind::WorkItemPlanAuthor`（而非 `ProviderRunKind::Author`）；该分支用 `state.provider_adapter` 构造 `WorkItemSplitEngine`，调 `generate` + `engine.complete_work_item_plan_author`，按 `WorkItemPlanAuthorOutcome` 处理（AuthorConfirm 完成 / AutoRevision 重新 generate / HumanConfirm 等待）。

**Files:**
- Modify: `src/web/workspace_ws_handler.rs`（`ProviderRunKind`、`ProviderRunContext`、`run_context` 构造、`StartGeneration` 路由、`spawn_provider_run_from_handler` 两处 match）
- Test: `tests/it_web/web_work_item_generation.rs` 或新增 `tests/it_web/web_work_item_plan_author.rs`

**Interfaces:**
- Consumes: Task 2 的 `complete_work_item_plan_author` + `WorkItemPlanAuthorOutcome`；`WorkItemSplitEngine::generate`；`state.provider_adapter`；`lifecycle.get_issue_work_item_plan`（读 Draft plan 的 source ids/options 组装 `GenerateWorkItemsRequest`）。
- Produces: WorkItemPlan author run 的 WS 触发链路。

- [ ] **Step 3.1：写失败测试 —— StartGeneration 触发 WorkItemPlan author run 返回 candidate artifact**

在 `tests/it_web/web_work_item_plan_author.rs`（新增）或 `web_work_item_generation.rs` 末尾追加。复用 `app_with_confirmed_story_and_design` 夹具。

```rust
#[tokio::test]
async fn work_item_plan_start_generation_returns_candidate_artifact() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;
    // 1. prepare 创建 Draft plan + WorkItemPlan session
    let (_, prepare_resp) = request_json(app.clone(), Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({ "title":"登录拆分", "story_spec_ids":["story_spec_0001"], "design_spec_ids":["design_spec_0001"],
                "include_integration_tests":true, "include_e2e_tests":false,
                "force_frontend_backend_split":true, "require_execution_plan_confirm":false, "review_rounds":1 })).await;
    let session_id = prepare_resp["workspace_session"]["workspace_session_id"].as_str().unwrap().to_string();

    // 2. 连 WS，发 StartGeneration，收 ArtifactUpdate（candidate payload）
    let ws = connect_ws(&app, &session_id).await;  // 本 WP 新增共享 WS 连接 helper
    ws.send(json!({ "type":"start_generation", "provider_config":{/* minimal */}, "reviewer_enabled":false }).to_string()).await;

    // 收 SessionState（workspace_type=work_item_plan）→ 收 ArtifactUpdate（candidate）
    let messages = recv_ws_messages(&ws, timeout).await;
    let artifact_update = messages.iter().find(|m| m["type"] == "artifact_update").expect("artifact_update");
    assert!(artifact_update["candidate"]["work_items"].is_array());
    assert!(artifact_update["candidate"]["work_items"].as_array().unwrap().len() >= 1);
    // 收 StageChange → author_confirm
    let stage = messages.iter().find(|m| m["type"] == "stage_change").expect("stage_change");
    assert_eq!(stage["stage"], "author_confirm");
}

#[tokio::test]
async fn work_item_plan_author_persists_draft_candidate_records_without_child_sessions() {
    // ... 同上 prepare + start_generation ...
    // 断言：lifecycle.list_work_items 非空（candidate work_items 已落盘）
    // 断言：无 WorkspaceType::WorkItem 子 session（confirm 前不建）
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(_repo.path().join(".aria")));
    let sessions = lifecycle.list_workspace_sessions("project_0001", "issue_0001").unwrap();
    assert!(sessions.iter().all(|s| s.workspace_type != WorkspaceType::WorkItem));
    let work_items = lifecycle.list_work_items("project_0001", "issue_0001").unwrap();
    assert!(!work_items.is_empty());
    // work_items 均 Draft
    assert!(work_items.iter().all(|wi| wi.plan_status == WorkItemPlanStatus::Draft));
}
```

> 实现者注意：
> 1. 本 WP 必须新增可被 WP3/WP4/WP8 复用的 WS 测试 helper（建议放在 `tests/it_web/web_work_item_plan_author.rs` 或抽到 `tests/it_web/workspace_ws_test_support.rs`）：`connect_ws(app, session_id)`、`recv_ws_messages_with_timeout(ws, timeout)`、`recv_until_stage(ws, stage)`。参考现有 `tests/it_web/web_coding_ws_handler.rs` 的 `tokio_tungstenite::connect_async` / `recv_json_value` 模式；不要假设仓库已有通用 helper。
> 2. `provider_config` 最小值参考现有 StartGeneration 测试夹具。
> 3. `valid_split_output()` 让 `MockSplitProviderAdapter` 返回有效 split JSON，`WorkItemSplitEngine::generate` 会成功 parse。
> 4. 不再允许把 WP2b 的 WS 路由覆盖整体 fallback 到 store+engine 层：Task 2 已覆盖 engine/store，Task 3 的价值是证明 `StartGeneration` WS 路由没有走普通 streaming author。若 helper 编写超出当前 Task，先补 helper，再写本测试。

- [ ] **Step 3.2：运行测试，确认失败**

Run: `cargo test --locked --test it_web work_item_plan_start_generation`
Expected: 失败——StartGeneration 走普通 `ProviderRunKind::Author`，不会产生 candidate artifact。

- [ ] **Step 3.3：`ProviderRunContext` 加 `provider_adapter` 字段**

`src/web/workspace_ws_handler.rs:773-781`：

```rust
#[derive(Clone)]
struct ProviderRunContext {
    provider_registry: Arc<ProviderRegistry>,
    engine: Arc<Mutex<WorkspaceEngine>>,
    current_run: Arc<Mutex<Option<WorkspaceActiveRun>>>,
    workspace_runs: WorkspaceRunRegistry,
    session_id: String,
    next_run_id: Arc<Mutex<u64>>,
    provider_adapter: Arc<dyn ProviderAdapter + Send + Sync>,
}
```

`run_context` 构造（:396-403）补：

```rust
    let run_context = ProviderRunContext {
        provider_registry: state.provider_registry.clone(),
        engine: engine.clone(),
        current_run: current_run.clone(),
        workspace_runs: state.workspace_runs.clone(),
        session_id: session_id.clone(),
        next_run_id: next_run_id.clone(),
        provider_adapter: state.provider_adapter.clone(),
    };
```

> 顶部 `use`（:1-34）补：`use crate::cross_cutting::provider_adapter::ProviderAdapter;`

- [ ] **Step 3.4：`ProviderRunKind` 加 `WorkItemPlanAuthor` 变体**

`src/web/workspace_ws_handler.rs:1293-1298`：

```rust
enum ProviderRunKind {
    Author { content: String },
    AuthorChoiceFollowup { content: String },
    Revision,
    ReviewOnly,
    WorkItemPlanAuthor,
}
```

- [ ] **Step 3.5：`StartGeneration` 按 workspace_type 路由**

`src/web/workspace_ws_handler.rs:677-707`，在 `engine.start_generation(...)` 成功后，根据 workspace_type 选 run kind：

```rust
            WsInMessage::StartGeneration { provider_config, reviewer_enabled } => {
                let result = {
                    let mut engine = engine.lock().await;
                    engine.start_generation(provider_config, reviewer_enabled).await
                };
                match result {
                    Ok((_node, locked)) => {
                        let _ = send_json_outbound(&outbound_tx, &locked).await;
                        // 按 workspace_type 路由 run kind
                        let run_kind = {
                            let engine = engine.lock().await;
                            if engine.session().workspace_type == WorkspaceType::WorkItemPlan {
                                ProviderRunKind::WorkItemPlanAuthor
                            } else {
                                ProviderRunKind::Author { content: String::new() }
                            }
                        };
                        if let Err(message) = spawn_provider_run_from_handler(
                            run_context.clone(),
                            run_kind,
                        ).await {
                            let err = WsOutMessage::Error { message };
                            let _ = send_json_outbound(&outbound_tx, &err).await;
                        }
                    }
                    Err(message) => {
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(&outbound_tx, &err).await;
                    }
                }
            }
```

> 顶部 `use` 补 `WorkspaceType`（从 `crate::product::models`）。

- [ ] **Step 3.6：`spawn_provider_run_from_handler` 两处 match 加 `WorkItemPlanAuthor` 分支**

`src/web/workspace_ws_handler.rs`：

1. **provider 选择 match**（:1362-1374）：`WorkItemPlanAuthor` 不走 `provider_registry`（流式），用 `provider_adapter`。但此 match 的目的是选 `provider_name`（用于 `provider_registry.get`）——`WorkItemPlanAuthor` 不需要 `provider_for_run`，可提前跳过。改为：

```rust
    let provider_name = {
        let engine = engine.lock().await;
        match &run_kind {
            ProviderRunKind::Author { .. }
            | ProviderRunKind::AuthorChoiceFollowup { .. }
            | ProviderRunKind::Revision => engine.session().author_provider.clone(),
            ProviderRunKind::ReviewOnly => engine
                .session()
                .reviewer_provider
                .clone()
                .unwrap_or(ProviderName::Codex),
            ProviderRunKind::WorkItemPlanAuthor => {
                // 不使用 provider_registry 的流式 adapter；run 分支用 provider_adapter
                // 此处返回一个占位 name（不会被使用，因 run 分支不取 provider_for_run）
                engine.session().author_provider.clone()
            }
        }
    };
    let provider_for_run = if matches!(run_kind, ProviderRunKind::WorkItemPlanAuthor) {
        None
    } else {
        let Some(p) = provider_registry.get(&provider_name) else {
            return Err(format!("provider unavailable: {provider_name:?}"));
        };
        Some(p)
    };
```

> `provider_for_run: Option<...>`，后续流式分支解构时用 `provider_for_run.expect("...")`（非 WorkItemPlanAuthor 分支必为 Some）。

2. **run 分发 match**（:1411-1432）：加 `WorkItemPlanAuthor` 分支。该分支构造 `WorkItemSplitEngine`，调 `generate`，再调 `engine.complete_work_item_plan_author`，按 outcome 处理：

```rust
            ProviderRunKind::WorkItemPlanAuthor => {
                // 1. 从 session.entity_id 读 Draft plan，组装 GenerateWorkItemsRequest
                let request = {
                    let engine = engine.lock().await;
                    build_work_item_plan_generate_request(&engine, &lifecycle_for_run)
                        .map_err(|e| format!("build request failed: {e}"))?
                };
                // 2. 构造 WorkItemSplitEngine，调 generate
                let split_engine = WorkItemSplitEngine::new(provider_adapter_for_run.clone());
                let repository = workspace_repository_for_session(&app_paths_for_run, &lifecycle_for_run, &session_record_for_run)
                    .map_err(|e| format!("load repository failed: {e}"))?;
                let issue = IssueStore::new(app_paths_for_run.clone()).get(&request_project_id, &request_issue_id)
                    .map_err(|e| format!("load issue failed: {e}"))?;
                let author_provider = {
                    let engine = engine.lock().await;
                    engine.session().author_provider.clone()
                };
                let output = split_engine.generate(&request, &lifecycle_for_run, &issue, &repository, author_provider).await
                    .map_err(|e| format!("split generate failed: {e}"))?;
                // 3. engine.complete_work_item_plan_author（含 validate/replace/artifact/enter_author_confirm）
                let outcome = {
                    let mut engine = engine.lock().await;
                    engine.complete_work_item_plan_author(output).await
                        .map_err(|e| format!("complete author failed: {e}"))?
                };
                // 4. 按 outcome 处理
                match outcome {
                    WorkItemPlanAuthorOutcome::AuthorConfirm => { /* 完成，stage 已进 AuthorConfirm */ }
                    WorkItemPlanAuthorOutcome::AutoRevision { findings: _ } => {
                        // 重新 spawn WorkItemPlanAuthor run（重生）
                        // 注意：需重新构造 run_context，避免 move 后不可用
                        drop(engine);
                        let _ = spawn_provider_run_from_handler(run_context_clone, ProviderRunKind::WorkItemPlanAuthor).await;
                    }
                    WorkItemPlanAuthorOutcome::HumanConfirm { reason: _ } => { /* stage 已进 HumanConfirm，等用户 */ }
                }
            }
```

> ⚠️ 实现者注意（关键复杂点）：
> 1. **`run_context` move 问题**：`spawn_provider_run_from_handler` 消费 `run_context`（by value）。`WorkItemPlanAuthor` 分支内若要重新 spawn（AutoRevision），需要 `run_context.clone()`——`ProviderRunContext` 已 `#[derive(Clone)]`，在 spawn task 闭包前 `let run_context_clone = run_context.clone();`。
> 2. **依赖获取**：spawn task 闭包（:1408 `tokio::spawn(async move { ... })`）内需要 `lifecycle`/`app_paths`/`session_record`/`provider_adapter`。这些不在 `ProviderRunContext` 里。两种方案：
>    - **方案 A（推荐）**：给 `ProviderRunContext` 加 `lifecycle: LifecycleStore` / `app_paths: ProductAppPaths` / `session_record: WorkspaceSessionRecord` 字段（或合并成一个 `WorkItemPlanRunDeps`）。
>    - **方案 B**：在 `WorkItemPlanAuthor` 分支内重新从 `app_paths` 构造 `LifecycleStore`/`IssueStore`——但闭包没有 `app_paths`。
>    - **选 A**：`ProviderRunContext` 加 `app_paths: ProductAppPaths` + `session_record: WorkspaceSessionRecord`（或 `entity_id`/`project_id`/`issue_id` 足够），`lifecycle` 可从 `app_paths` 在闭包内 `LifecycleStore::new(app_paths.clone())` 构造。`run_context` 构造处（:396-403）补这些字段。
> 3. **`build_work_item_plan_generate_request`**：新 helper，从 `session.entity_id` 读 Draft `IssueWorkItemPlan`，把 `source_story_spec_ids`/`source_design_spec_ids`/`options` 组装成 `GenerateWorkItemsRequest`（`title` 用 plan.title 或固定 "WorkItemPlan author"；provider 配置从 session 读）。放 `workspace_ws_handler.rs` 或 `workspace_engine.rs`。先 `grep -n "struct GenerateWorkItemsRequest" src/web/types.rs` 确认字段。
> 4. **`provider_adapter_for_run`**：从 `run_context.provider_adapter.clone()` 取（解构 `ProviderRunContext` 时拿到）。
> 5. **outcome 重新 spawn 的循环风险**：AutoRevision 重新 spawn 会再跑 `generate` + `complete_work_item_plan_author`，若一直 has_errors 会反复 spawn 直到 3 次进 HumanConfirm。`complete_work_item_plan_author` 内 `work_item_plan_author_retry_count` 计数保证最终收敛。但**计数在 engine 内存**，若 spawn 之间 engine 被 abort/重建会丢失——本 WP 接受此风险（abort 时丢弃 run，用户可重新 start）。
> 6. **`engine` 锁**：`WorkItemPlanAuthor` 分支内多次 `engine.lock()`（读 session、调 complete、读 outcome）。注意不要跨 await 持锁——每次 lock 用完即 drop。上面代码已分段 lock。

- [ ] **Step 3.7：顶部 `use` 补全**

`src/web/workspace_ws_handler.rs:1-34` 补：
```rust
use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::product::issue_store::IssueStore;
use crate::product::models::WorkspaceType;
use crate::product::work_item_split_engine::WorkItemSplitEngine;
use crate::product::workspace_engine::WorkItemPlanAuthorOutcome;
use crate::product::app_paths::ProductAppPaths;
use crate::product::models::WorkspaceSessionRecord;
```
> 以实际缺失为准——`cargo check` 会报缺哪些。

- [ ] **Step 3.8：运行 Task 3 测试 + 收口**

Run:
```
cargo test --locked --test it_web work_item_plan_start_generation
cargo test --locked --test it_web work_item_plan_author
cargo test --locked --test it_web web_work_item_generation
cargo check --locked
```
Expected: 新 WS 集成测试 PASS；现有 `web_work_item_generation`（P3 REST 流程）仍全绿（本 WP 不删路由）；`cargo check` 全绿。

> 本 Task 必须保留 WS 集成测试：若 helper 不完整，先按 Step 3.1 补共享 helper，再运行本测试。store/engine 层测试已由 Task 2 覆盖，不能替代这里的 WS 路由验证。

- [ ] **Step 3.9：提交**

```bash
git add src/web/workspace_ws_handler.rs tests/it_web/
git commit -m "feat(WP2b): WS handler WorkItemPlanAuthor run + StartGeneration 路由"
```

---

## Task 4：WP2b 收口验证（全量回归）

**目标**：跑完整验证链，确保 author run 未破坏 Story/Design/WorkItem 既有流程；WorkItemPlan prepare→author 链路通。

**Files:** 无新增改动；仅运行验证命令。

- [ ] **Step 4.1：全量验证链**

Run（依次，任一失败即停并修复）:
```
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked --lib workspace_engine
cargo test --locked --test it_product product_lifecycle_store
cargo test --locked --test it_web
```
Expected: 全绿。

> `cargo test --locked --test it_web` 全量跑 web 集成测试，覆盖 Story/Design/WorkItem/WorkItemPlan prepare 的 HTTP + WS 流程。是 WP2b 最大的回归保障。

- [ ] **Step 4.2：确认 WP1/WP2a 成果未破坏**

Run:
```
cargo test --locked --test it_web prepare_work_item_plan_creates_draft_plan_and_session_without_generating
cargo test --locked --lib workspace_ws_types
```
Expected: PASS（WP1 prepare 仍工作；WP2a union 类型 serde 往返正常）。

- [ ] **Step 4.3：交付摘要（供 WP3 前置交付摘要使用）**

commit 后，把以下内容写入 WP3 plan 的「前置交付摘要」章节：

- WorkItemPlan author run 链路：`StartGeneration`（workspace_type=WorkItemPlan）→ `ProviderRunKind::WorkItemPlanAuthor` → `WorkItemSplitEngine::generate` → `engine.complete_work_item_plan_author`。
- `complete_work_item_plan_author` 流程：validate → has_errors 计数重生（AutoRevision）/ warnings 随 candidate → `replace_issue_work_item_plan_candidate` → `build_work_item_plan_candidate_dto` → `update_artifact(ArtifactPayload::WorkItemPlanCandidate)` → `enter_author_confirm`。
- Draft candidate 已落盘：plan（Draft）+ work_items（Draft）+ verification_plans + repository_profile，**无子 WorkItem session**（confirm 时才建，WP5）。
- `WorkItemPlanCandidateDto` 由 `build_work_item_plan_candidate_dto` 从 lifecycle 记录组装（事实来源）——WP3 的 `build_work_item_plan_review_input` 可复用此 DTO 或直接从 lifecycle 读取。
- `WorkItemPlanAuthorOutcome::{AuthorConfirm, AutoRevision, HumanConfirm}`——handler 按此分支；AutoRevision 重新 spawn WorkItemPlanAuthor run。
- **WP3 待办**：`build_review_input` 在 WorkItemPlan 分支调新 `build_work_item_plan_review_input`（从当前 Draft candidate 组装 review 上下文，裁剪 token）；reviewer 流式审整组。

---

## Self-Review（写完后的自查）

**1. Spec 覆盖**（对照总览 v1.1 WP2b 目标/写入范围/验证 + 设计方案 :240-272）：
- ✅ 从 `session.entity_id` 读取 Draft plan → 组装 `GenerateWorkItemsRequest` → Task 3 Step 3.6（`build_work_item_plan_generate_request`）
- ✅ `WorkItemSplitEngine::generate` → Task 3 Step 3.6
- ✅ `Validator::validate` → Task 2 Step 2.4
- ✅ `replace_issue_work_item_plan_candidate` 持久化 Draft candidate → Task 1
- ✅ 写 artifact payload → `update_artifact(WorkItemPlanCandidate)` → Task 2 Step 2.4
- ✅ 推 `ArtifactUpdate` → Task 2（`update_artifact` 内部发 `EngineEvent::ArtifactUpdate`，WP2a 已挂载）
- ✅ 进 AuthorConfirm → Task 2 Step 2.4（`enter_author_confirm`）
- ✅ StartGeneration 在 WorkItemPlan 下启动 dedicated non-streaming run → Task 3 Step 3.5
- ✅ 不启动 `ProviderRunKind::Author` → Task 3 Step 3.5（workspace_type 路由）
- ✅ `workspace_requires_artifact_gate` 保持不含 WorkItemPlan → Task 2 Step 2.6
- ✅ `workspace_type_title` 加分支 → Task 2 Step 2.5
- ✅ validate 失败自动 Revision 最小版本 → Task 2 Step 2.4（AutoRevision + 计数 + HumanConfirm）
- ✅ 不建子 session（confirm 前只存 candidate）→ Task 1 Step 1.5（replace 不建 session）+ Task 3 测试断言
- ✅ 验证命令链 → Task 4
- ✅ 不做项：未实现 review（WP3）、未实现 revert/revision 局部重做（WP4）、未实现 confirm（WP5）、未改 WorkItemSplitEngine 内核、未改前端——均在「不做」清单。

**2. Placeholder 扫描**：
- `build_work_item_plan_generate_request`（Task 3 Step 3.6）：给出职责描述但未给完整函数体——因 `GenerateWorkItemsRequest` 字段映射需实现时确认。给出 `grep` 定位指引，属可接受（不是「TBD」，是「字段映射以实际为准」）。**实现时应补完整字段映射**。
- `VerificationPlanDto`/`RepositoryProfileDto` 的 `From` impl（Task 2 Step 2.3）：给出 `map(|vp| VerificationPlanDto::from(&vp))` 但未定义 impl——标注「以 WP1 实际定义为准，若无 From 则手写字段映射」。可接受。
- WS 连接 helper（Task 3 Step 3.1）：已明确本 WP 新增共享 helper，后续 WP3/WP4/WP8 复用。属可接受。
- `WorkItemSplitProviderOutput` 构造（Task 1 Step 1.1）：参考 `valid_split_output()`，未完整展开——因测试夹具构造繁长，参考现有夹具是合理指引。

**3. 类型一致性**：
- `WorkItemPlanAuthorOutcome` 在 Task 2 定义，Task 3 handler 引用一致。
- `WorkItemPlanCandidateSnapshot` 在 Task 1 定义，Task 2 `replace_issue_work_item_plan_candidate` 返回值一致。
- `complete_work_item_plan_author(output: WorkItemSplitProviderOutput) -> Result<WorkItemPlanAuthorOutcome, String>` 签名在 Task 2 定义，Task 3 调用一致。
- `build_work_item_plan_candidate_dto` 返回 `WorkItemPlanCandidateDto`（WP1 定义），Task 2 用之填 `ArtifactPayload::WorkItemPlanCandidate`（WP2a 挂载）。

**4. 边界风险**：
- **`run_context` move + 重新 spawn**（Task 3 Step 3.6）：AutoRevision 重新 spawn 需 `run_context.clone()`。`ProviderRunContext` 已 `#[derive(Clone)]`，但加了 `provider_adapter: Arc<dyn ProviderAdapter>` 字段后 `Clone` 仍成立（`Arc` clone）。已标注。
- **依赖获取**（Task 3 Step 3.6）：spawn 闭包需 `lifecycle`/`app_paths`/`session_record`，方案 A（给 `ProviderRunContext` 加字段）会扩大 `ProviderRunContext` 体积，但最干净。已标注选 A。
- **engine 锁跨 await**（Task 3 Step 3.6）：多次 `engine.lock()` 分段，不跨 await 持锁。已标注。
- **AutoRevision 循环收敛**（Task 3 Step 3.6）：靠 `work_item_plan_author_retry_count` 计数保证 3 次后进 HumanConfirm。计数在 engine 内存，abort 后丢失——接受此风险（方案 :265 也接受"已发起的 spawn_blocking 跑完、结果丢弃"）。
- **WS 集成测试难度**（Task 3 Step 3.1）：本 WP 必须补共享 helper 并覆盖 `StartGeneration` WS 路由；不能整体延后 WP8，否则 WP3/WP4 的路由分支会缺共同测试基础。
- **`build_work_item_plan_generate_request` 字段映射**（Task 3 Step 3.6）：`GenerateWorkItemsRequest` 的 `title` 字段——plan 无 title，用固定字符串或 plan.id。实现时定。

---

## Execution Handoff

本 WP2b plan 已保存至 `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP2b_后端author生成_v1.0.md`。

**执行方式（待用户选择）：**

1. **Subagent-Driven（推荐）** — 每个 Task 派一个 fresh subagent 实现，Task 间做 two-stage review。
2. **Inline Execution** — 在当前 session 按 superpowers:executing-plans 批量执行，带 checkpoint 审查。

**完成后**：交付 WP2b 后，按同样标准继续 WP3（后端 review 整组，依赖 WP2b 的 candidate 落盘与 DTO 组装）。WP3 的「前置交付摘要」直接引用本 plan Task 4 Step 4.3 的产出。

**⚠️ 实现前注意**：Task 3 Step 3.6 是本 WP 最复杂的接入点（handler spawn 闭包依赖获取 + run_context move + outcome 重新 spawn）。建议执行者先完整读 `spawn_provider_run_from_handler`（:1347-1470）现有实现，再按 Step 3.6 改造。若 `ProviderRunContext` 加字段影响其他调用点（`grep -n "ProviderRunContext" src/`），一并适配。
