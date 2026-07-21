# WorkItemGroup 级 Coding Workspace 串行执行 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Final Compile 后的 WorkItemGroup 可以作为 Coding Workspace 入口，并在一个 Group Coding Workspace 内按真实 Work Item 串行执行 coding / testing / code review，最后进行整组 review / PR / final confirm。

**Architecture:** 真实 `LifecycleWorkItemRecord` 是 Coding 执行事实源；`IssueWorkItemPlan`、Outline、accepted Draft 只作为只读上下文增强与异常诊断来源。新增 group-scope Coding Attempt 和 `CodingExecutionUnit`，第一阶段严格串行执行一个 active unit，保留现有单 Work Item Coding Workspace。

**Tech Stack:** Rust 1.95.0、Axum、serde、Cargo、Zustand、React、Vitest、Testing Library、pnpm。

## Global Constraints

- 文档来源：`cadence/designs/2026-06-27_技术方案_WorkItemGroup级CodingWorkspace串行执行_v1.0.md`。
- 计划文档必须存放在 `cadence/plans/`。
- 产品代码遵循 TDD：先写失败测试，再写实现，再跑验证。
- Cargo 命令禁止携带 `-j 1`。
- Rust 标准验证命令：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo check --locked`、`cargo test --locked`。
- 前端包管理使用 `pnpm`；验证可使用 `./node_modules/.bin/tsc --noEmit`、`./node_modules/.bin/vitest --run`、`./node_modules/.bin/vite build`。
- 第一阶段只支持串行执行，不实现并行 dependency layer。
- Coding 执行事实源固定为 Final Compile 后的真实 Work Item。
- WorkItemPlan / Outline / Draft 在 Coding 阶段只读，不由 Coding Workspace 自动改写。
- 现有单 Work Item Coding Workspace 必须保持兼容。
- 修改涉及 Workspace artifact、timeline/chat rebuild、artifact version 绑定时，必须评估 Story / Design / Work Item 三类 workspace 是否受影响。

---

## File Structure

### 后端模型与 Store

- Modify: `src/product/coding_models/execution.rs`
  - 新增 `CodingAttemptScope`。
  - 扩展 `CodingExecutionAttempt` 的 group 字段。
- Create: `src/product/coding_models/group.rs`
  - 定义 `CodingExecutionUnitStatus`、`CodingExecutionUnit`、`CodingGroupContext`。
- Modify: `src/product/coding_models/mod.rs`
  - 导出 `group` 模块。
- Modify: `src/product/coding_attempt_store/inputs.rs`
  - 新增 `CreateGroupCodingAttemptInput`、`CreateCodingExecutionUnitInput`。
- Create: `src/product/coding_attempt_store/group.rs`
  - group attempt 与 unit 持久化方法。
- Modify: `src/product/coding_attempt_store/mod.rs`
  - 注册 `group` 模块。
- Modify: `src/product/coding_attempt_store/paths.rs`
  - 增加 `coding_units_root`、`coding_unit_path`。
- Test: `src/product/coding_attempt_store/tests.rs`
  - 覆盖 scope serde、unit store、active unit 查询、旧 attempt 兼容。

### 后端 HTTP / WS 契约

- Modify: `src/web/app.rs`
  - 增加 group coding attempt 创建路由。
- Modify: `src/web/handlers/coding.rs`
  - 新增 `create_group_coding_attempt`。
  - 新增 `group_work_item_execution_order`、`work_items_for_confirmed_plan`、`ensure_no_active_group_or_item_attempt`。
- Modify: `src/web/handlers/dto.rs`
  - `coding_attempt_dto` 输出 group 字段。
  - 新增 `coding_execution_unit_dto`。
- Modify: `src/web/types.rs`
  - `CodingAttemptDto`、`CodingAttemptSnapshotResponse` 增加 group 字段。
- Modify: `src/web/coding_ws_handler/protocol.rs`
  - `CodingSessionState` 增加 group context 与 units。
- Modify: `src/web/coding_ws_handler/socket.rs`
  - snapshot 构建加载 group units。
- Modify: `src/web/coding_ws_handler/context.rs`
  - 当前 work item 解析从 `attempt.work_item_id` 抽为 helper。
- Modify: `src/web/coding_ws_handler/runner.rs`
  - group attempt 串行执行 current unit。
- Test: `tests/it_web/web_coding_attempt_api.rs`
  - 覆盖 group 创建、锁互斥、snapshot 字段。
- Test: `tests/it_web/web_coding_ws_handler.rs`
  - 覆盖 group WS snapshot 与串行调度。

### Coding Engine

- Modify: `src/product/coding_workspace_engine/types.rs`
  - 增加 group 相关错误码。
- Create: `src/product/coding_workspace_engine/group.rs`
  - group unit 生命周期：start、complete、advance、enter group final stages。
- Modify: `src/product/coding_workspace_engine/mod.rs`
  - 注册 `group` 模块。
- Modify: `src/product/coding_workspace_engine/handoffs.rs`
  - unit handoff 与 group final confirm 分离。
- Modify: `src/product/coding_workspace_engine/internal_pr_review.rs`
  - group internal PR review prompt 汇总所有 unit。
- Modify: `src/product/coding_workspace_engine/reports.rs`
  - group completion / review 报告数据。
- Test: `tests/it_product/product_coding_workspace_engine.rs`
  - 覆盖 unit 完成、切下一个、整组 final review。

### 前端

- Modify: `web/src/api/types/coding.ts`
  - 增加 `CodingAttemptScope`、`CodingExecutionUnit`、group snapshot 字段。
- Modify: `web/src/api/types/lifecycle.ts`
  - WorkItemGroup 入口需要识别 active group coding attempt。
- Modify: `web/src/api/client.ts`
  - 新增 `createGroupCodingAttempt(projectId, issueId, planId)`。
- Modify: `web/src/state/coding-workspace-store.ts`
  - 存储 group context、current unit、units。
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
  - WorkItemGroup 卡片启动 group coding attempt。
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbenchParts.tsx`
  - WorkItemGroup drawer 展示 group coding 操作。
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
  - Header / status bar 展示 group progress 与当前 Work Item。
- Create: `web/src/pages/CodingWorkspaceGroupProgress.tsx`
  - units 列表、当前 unit、blocked reason、完成进度。
- Test: `web/src/api/types.test.ts`
  - group DTO 类型契约。
- Test: `web/src/api/coding-attempts.test.ts`
  - group 创建 API URL。
- Test: `web/src/state/coding-workspace-store.test.ts`
  - snapshot group 字段恢复。
- Test: `web/src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx`
  - WorkItemGroup 入口创建 / 复用 group attempt。
- Test: `web/src/pages/CodingWorkspacePage.test.tsx`
  - group progress UI。

---

## Task 1: Coding Attempt Scope 与 Unit Store

**Files:**
- Modify: `src/product/coding_models/execution.rs`
- Create: `src/product/coding_models/group.rs`
- Modify: `src/product/coding_models/mod.rs`
- Modify: `src/product/coding_attempt_store/inputs.rs`
- Create: `src/product/coding_attempt_store/group.rs`
- Modify: `src/product/coding_attempt_store/mod.rs`
- Modify: `src/product/coding_attempt_store/paths.rs`
- Test: `src/product/coding_attempt_store/tests.rs`

**Interfaces:**
- Produces:
  - `CodingAttemptScope::{WorkItem, WorkItemGroup}`
  - `CodingExecutionUnitStatus`
  - `CodingExecutionUnit`
  - `CreateGroupCodingAttemptInput`
  - `CreateCodingExecutionUnitInput`
  - `CodingAttemptStore::create_group_attempt`
  - `CodingAttemptStore::create_coding_unit`
  - `CodingAttemptStore::list_coding_units`
  - `CodingAttemptStore::get_active_coding_unit`
  - `CodingAttemptStore::update_coding_unit_status`

- [ ] **Step 1: Write failing model/store tests**

Add these tests to `src/product/coding_attempt_store/tests.rs`:

```rust
#[test]
fn legacy_attempt_without_scope_deserializes_as_work_item_scope() {
    let json = serde_json::json!({
        "id": "coding_attempt_0001",
        "project_id": "project_0001",
        "issue_id": "issue_0001",
        "work_item_id": "work_item_0001",
        "attempt_no": 1,
        "status": "created",
        "stage": "prepare_context",
        "base_branch": "main",
        "branch_name": "aria/issues/issue_0001",
        "worktree_path": null,
        "provider_config_snapshot": { "author": "codex", "reviewer": "codex", "review_rounds": 1 },
        "rework_count": 0,
        "max_auto_rework": 2,
        "head_commit": null,
        "pushed_remote": null,
        "review_request_id": null,
        "provider_conversations": [],
        "created_at": "2026-06-27T00:00:00Z",
        "updated_at": "2026-06-27T00:00:00Z",
        "completed_at": null
    });

    let attempt: CodingExecutionAttempt = serde_json::from_value(json).expect("attempt");

    assert_eq!(attempt.scope, CodingAttemptScope::WorkItem);
    assert_eq!(attempt.current_work_item_id.as_deref(), Some("work_item_0001"));
    assert!(attempt.work_item_group_id.is_none());
}

#[test]
fn creates_group_attempt_and_units_with_single_active_unit() {
    let (_tmp, store, _attempt) = setup();

    let group_attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect("group attempt");

    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: group_attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("unit 1");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: group_attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            work_item_id: "work_item_0002".to_string(),
            order_index: 1,
            status: CodingExecutionUnitStatus::Pending,
        })
        .expect("unit 2");

    let units = store
        .list_coding_units("project_0001", "issue_0001", &group_attempt.id)
        .expect("units");
    let active = store
        .get_active_coding_unit("project_0001", "issue_0001", &group_attempt.id)
        .expect("active lookup")
        .expect("active");

    assert_eq!(group_attempt.scope, CodingAttemptScope::WorkItemGroup);
    assert_eq!(group_attempt.work_item_group_id.as_deref(), Some("work_item_plan_0001"));
    assert_eq!(units.len(), 2);
    assert_eq!(active.work_item_id, "work_item_0001");
}
```

- [ ] **Step 2: Run model/store tests to verify they fail**

Run:

```bash
cargo test --locked --lib coding_attempt_store
```

Expected: FAIL with missing `CodingAttemptScope`, `CreateGroupCodingAttemptInput`, `CodingExecutionUnitStatus`, or unit store methods.

- [ ] **Step 3: Implement model types**

In `src/product/coding_models/execution.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CodingAttemptScope {
    #[default]
    WorkItem,
    WorkItemGroup,
}
```

Extend `CodingExecutionAttempt`:

```rust
#[serde(default)]
pub scope: CodingAttemptScope,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub work_item_group_id: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub current_work_item_id: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub active_unit_id: Option<String>,
```

When constructing a legacy single-item attempt in `create_attempt`, set:

```rust
scope: CodingAttemptScope::WorkItem,
work_item_group_id: None,
current_work_item_id: Some(input.work_item_id.clone()),
active_unit_id: None,
```

- [ ] **Step 4: Implement group model module**

Create `src/product/coding_models/group.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingExecutionUnitStatus {
    Pending,
    Running,
    WaitingForHuman,
    Completed,
    Failed,
    Blocked,
    Skipped,
}

impl CodingExecutionUnitStatus {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Running | Self::WaitingForHuman | Self::Blocked)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingExecutionUnit {
    pub id: String,
    pub attempt_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub work_item_id: String,
    pub order_index: u32,
    pub status: CodingExecutionUnitStatus,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub handoff_ref: Option<String>,
    pub completion_commit: Option<String>,
    pub summary: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
```

Update `src/product/coding_models/mod.rs`:

```rust
pub mod group;
```

- [ ] **Step 5: Implement store inputs and paths**

In `src/product/coding_attempt_store/inputs.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateGroupCodingAttemptInput {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub current_work_item_id: String,
    pub base_branch: String,
    pub branch_name: String,
    pub worktree_path: Option<PathBuf>,
    pub provider_config_snapshot: ProviderConfigSnapshot,
    pub max_auto_rework: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCodingExecutionUnitInput {
    pub attempt_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub work_item_id: String,
    pub order_index: u32,
    pub status: CodingExecutionUnitStatus,
}
```

In `src/product/coding_attempt_store/paths.rs`, add:

```rust
pub(crate) fn coding_units_root(
    &self,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> PathBuf {
    self.attempt_dir(project_id, issue_id, attempt_id).join("units")
}

pub(crate) fn coding_unit_path(
    &self,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
    unit_id: &str,
) -> PathBuf {
    self.coding_units_root(project_id, issue_id, attempt_id)
        .join(format!("{unit_id}.json"))
}
```

- [ ] **Step 6: Implement unit store methods**

Create `src/product/coding_attempt_store/group.rs` with these public method signatures:

```rust
impl super::CodingAttemptStore {
    pub fn create_group_attempt(
        &self,
        input: CreateGroupCodingAttemptInput,
    ) -> Result<CodingExecutionAttempt, ProductStoreError>;

    pub fn create_coding_unit(
        &self,
        input: CreateCodingExecutionUnitInput,
    ) -> Result<CodingExecutionUnit, ProductStoreError>;

    pub fn list_coding_units(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingExecutionUnit>, ProductStoreError>;

    pub fn get_active_coding_unit(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<CodingExecutionUnit>, ProductStoreError>;

    pub fn update_coding_unit_status(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        unit_id: &str,
        status: CodingExecutionUnitStatus,
        summary: Option<String>,
    ) -> Result<CodingExecutionUnit, ProductStoreError>;
}
```

Implementation details:

- `create_group_attempt` validates all relative ids, rejects another active group attempt for the same `plan_id`, writes `CodingExecutionAttempt { scope: WorkItemGroup, work_item_id: input.current_work_item_id.clone(), work_item_group_id: Some(input.plan_id.clone()), current_work_item_id: Some(input.current_work_item_id), active_unit_id: None, stage: PrepareContext, status: Created, ... }`, and writes role provider config exactly like `create_attempt`.
- `create_coding_unit` validates ids, creates `coding_unit_NNNN`, writes the JSON file under `coding-attempts/{attempt_id}/units/`, and sets `started_at` only when initial status is `Running`.
- Use `next_sequential_id("coding_unit", count_json_files(&root)?)`.
- Validate `project_id`、`issue_id`、`attempt_id`、`plan_id`、`work_item_id` with existing relative id validators.
- `list_coding_units` sorts by `(order_index, id.clone())`.
- `get_active_coding_unit` returns error `active_coding_unit_ambiguous` if more than one active unit exists.
- `update_coding_unit_status` sets `started_at` when moving to `Running` and currently empty; sets `completed_at` when moving to `Completed`、`Failed`、`Skipped`.

Register module in `src/product/coding_attempt_store/mod.rs`:

```rust
mod group;
```

- [ ] **Step 7: Run model/store tests**

Run:

```bash
cargo test --locked --lib coding_attempt_store
```

Expected: PASS for new model/store tests.

- [ ] **Step 8: Commit**

```bash
git add src/product/coding_models src/product/coding_attempt_store
git commit -m "feat(coding): add group attempt scope and units"
```

---

## Task 2: Group Coding Attempt 创建 API

**Files:**
- Modify: `src/web/app.rs`
- Modify: `src/web/handlers/coding.rs`
- Modify: `src/web/handlers/dto.rs`
- Modify: `src/web/types.rs`
- Test: `tests/it_web/web_coding_attempt_api.rs`

**Interfaces:**
- Consumes:
  - `CodingAttemptStore::create_group_attempt`
  - `CodingAttemptStore::create_coding_unit`
- Produces:
  - `POST /api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}/coding-attempts`
  - `CodingAttemptDto.attempt_scope`
  - `CodingAttemptDto.work_item_group_id`
  - `CodingAttemptDto.current_work_item_id`

- [ ] **Step 1: Write failing API tests**

Add to `tests/it_web/web_coding_attempt_api.rs`:

```rust
#[tokio::test]
async fn creates_group_coding_attempt_from_confirmed_work_item_plan() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["attempt_scope"], "work_item_group");
    assert_eq!(body["work_item_group_id"], "work_item_plan_0001");
    assert_eq!(body["current_work_item_id"], "work_item_0001");
    assert_eq!(body["branch_name"], "aria/issues/issue_0001");
}

#[tokio::test]
async fn rejects_group_coding_attempt_for_unconfirmed_plan() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_draft_work_item_plan_group(app.clone(), repo.path()).await;

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "work_item_plan_not_confirmed");
}

#[tokio::test]
async fn rejects_group_coding_attempt_when_single_item_attempt_holds_issue_lock() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;

    let (single_status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(single_status, StatusCode::OK);

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "issue_worktree_active");
}
```

Add helper `bootstrap_confirmed_work_item_plan_group` in the same test module. It must create:

- one repository;
- one confirmed `IssueWorkItemPlan` with `work_item_ids=["work_item_0001","work_item_0002"]`;
- two confirmed `LifecycleWorkItemRecord` values with `work_item_set_id=Some("work_item_plan_0001")`;
- `work_item_0002.depends_on=["work_item_0001"]`;
- matching verification plans.

- [ ] **Step 2: Run API tests to verify they fail**

Run:

```bash
cargo test --locked --test it_web creates_group_coding_attempt_from_confirmed_work_item_plan
cargo test --locked --test it_web rejects_group_coding_attempt_for_unconfirmed_plan
cargo test --locked --test it_web rejects_group_coding_attempt_when_single_item_attempt_holds_issue_lock
```

Expected: FAIL because the route and handler are not implemented.

- [ ] **Step 3: Add DTO fields**

In `src/web/types.rs`, extend `CodingAttemptDto`:

```rust
pub attempt_scope: String,
pub work_item_group_id: Option<String>,
pub current_work_item_id: Option<String>,
pub active_unit_id: Option<String>,
```

In `src/web/handlers/dto.rs`, update `coding_attempt_dto`:

```rust
attempt_scope: coding_attempt_scope_text(&attempt.scope).to_string(),
work_item_group_id: attempt.work_item_group_id.clone(),
current_work_item_id: attempt.current_work_item_id.clone(),
active_unit_id: attempt.active_unit_id.clone(),
```

Add:

```rust
pub(crate) fn coding_attempt_scope_text(scope: &CodingAttemptScope) -> &'static str {
    match scope {
        CodingAttemptScope::WorkItem => "work_item",
        CodingAttemptScope::WorkItemGroup => "work_item_group",
    }
}
```

- [ ] **Step 4: Implement route**

In `src/web/app.rs`, add before single Work Item coding route or near WorkItemPlan routes:

```rust
.route(
    "/api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}/coding-attempts",
    post(handlers::create_group_coding_attempt),
)
```

- [ ] **Step 5: Implement handler helper ordering**

In `src/web/handlers/coding.rs`, add:

```rust
pub(crate) fn group_work_item_execution_order(
    plan: &IssueWorkItemPlanRecord,
    work_items: &[LifecycleWorkItemRecord],
) -> Result<Vec<LifecycleWorkItemRecord>, ApiError> {
    let mut selected = plan
        .work_item_ids
        .iter()
        .map(|id| {
            work_items
                .iter()
                .find(|item| &item.id == id)
                .cloned()
                .ok_or_else(|| ApiError::runtime("work_item_not_found", "plan work item not found", json!({ "work_item_id": id })))
        })
        .collect::<Result<Vec<_>, _>>()?;
    selected.sort_by_key(|item| item.sequence_hint.unwrap_or(u32::MAX));
    Ok(selected)
}
```

If `sequence_hint` is missing, preserve `plan.work_item_ids` order by using the original index as secondary sort key.

- [ ] **Step 6: Implement create_group_coding_attempt**

In `src/web/handlers/coding.rs`, implement:

```rust
pub async fn create_group_coding_attempt(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, plan_id)): Path<(String, String, String)>,
) -> ApiResult<Json<CodingAttemptDto>> {
    let app_paths = product_app_paths(&state);
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let plan = lifecycle
        .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?;
    if plan.status != IssueWorkItemPlanStatus::Confirmed {
        return Err(ApiError::validation(
            "work_item_plan_not_confirmed",
            "work item plan must be confirmed before group coding",
        ));
    }
    let all_work_items = lifecycle
        .list_work_items(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let ordered = group_work_item_execution_order(&plan, &all_work_items)?;
    if ordered.is_empty() {
        return Err(ApiError::validation(
            "work_item_group_empty",
            "work item group has no compiled work items",
        ));
    }
    if let Some(mismatched) = ordered
        .iter()
        .find(|item| item.work_item_set_id.as_deref() != Some(plan_id.as_str()))
    {
        return Err(ApiError::validation_with_details(
            "work_item_group_mismatch",
            "compiled work item does not belong to the selected group",
            json!({ "work_item_id": mismatched.id }),
        ));
    }
    let current_work_item = ordered.first().expect("checked non-empty");
    let repository = find_repository(&app_paths, &project_id, &current_work_item.repository_id)?;
    if !is_git_repo(&repository.path) {
        return Err(ApiError::validation(
            "repository_path_not_git_repo",
            "repository path must point to a git work tree",
        ));
    }
    let branch_name = format!("aria/issues/{issue_id}");
    let base_branch = current_git_branch(&repository.path).unwrap_or_else(|| "HEAD".to_string());
    let shared_worktree_path = repository
        .path
        .join(".worktrees")
        .join("aria-issues")
        .join(&issue_id);
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            repository_id: repository.id.clone(),
            branch_name: branch_name.clone(),
            worktree_path: shared_worktree_path,
            base_branch: base_branch.clone(),
        })
        .map_err(product_store_api_error)?;
    let _lock = lifecycle
        .try_acquire_issue_worktree_lock(&project_id, &issue_id, &current_work_item.id)
        .map_err(product_store_api_error)?;
    let provider_config_snapshot = coding_provider_config_snapshot(
        &lifecycle,
        current_work_item,
        &repository.default_provider_mode,
        &*state.provider_availability,
    )?;
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let attempt = coding_store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            plan_id: plan_id.clone(),
            current_work_item_id: current_work_item.id.clone(),
            base_branch,
            branch_name,
            worktree_path: None,
            provider_config_snapshot,
            max_auto_rework: 2,
        })
        .map_err(product_store_api_error)?;
    for (index, item) in ordered.iter().enumerate() {
        coding_store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: attempt.id.clone(),
                project_id: project_id.clone(),
                issue_id: issue_id.clone(),
                plan_id: plan_id.clone(),
                work_item_id: item.id.clone(),
                order_index: index as u32,
                status: if index == 0 {
                    CodingExecutionUnitStatus::Running
                } else {
                    CodingExecutionUnitStatus::Pending
                },
            })
            .map_err(product_store_api_error)?;
    }
    Ok(Json(coding_attempt_dto(&attempt)))
}
```

If any step after acquiring the issue worktree lock returns an error, release the lock with `lifecycle.release_issue_worktree_lock(&project_id, &issue_id, &current_work_item.id)` before returning.

- [ ] **Step 7: Run API tests**

Run:

```bash
cargo test --locked --test it_web creates_group_coding_attempt_from_confirmed_work_item_plan
cargo test --locked --test it_web rejects_group_coding_attempt_for_unconfirmed_plan
cargo test --locked --test it_web rejects_group_coding_attempt_when_single_item_attempt_holds_issue_lock
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/web/app.rs src/web/handlers/coding.rs src/web/handlers/dto.rs src/web/types.rs tests/it_web/web_coding_attempt_api.rs
git commit -m "feat(coding): add group coding attempt api"
```

---

## Task 3: HTTP / WS Snapshot Group Context

**Files:**
- Modify: `src/web/types.rs`
- Modify: `src/web/coding_ws_handler/protocol.rs`
- Modify: `src/web/coding_ws_handler/socket.rs`
- Modify: `web/src/api/types/coding.ts`
- Modify: `web/src/state/coding-workspace-store.ts`
- Test: `tests/it_web/web_coding_ws_handler.rs`
- Test: `web/src/state/coding-workspace-store.test.ts`
- Test: `web/src/api/types.test.ts`

**Interfaces:**
- Consumes:
  - `CodingAttemptStore::list_coding_units`
- Produces:
  - `CodingExecutionUnitDto`
  - `CodingAttemptSnapshotResponse.units`
  - `CodingWsOutMessage::CodingSessionState.units`
  - frontend `CodingWorkspaceState.units`

- [ ] **Step 1: Write failing backend snapshot test**

Add to `tests/it_web/web_coding_ws_handler.rs`:

```rust
#[tokio::test]
async fn coding_ws_session_state_includes_group_units() {
    let root = tempdir().expect("root");
    let app = build_group_coding_app(root.path()).await;
    let attempt_id = create_group_coding_attempt_fixture(&app).await;

    let messages = connect_coding_ws_and_collect_initial_messages(&app, &attempt_id).await;
    let state = messages
        .iter()
        .find(|message| message["type"] == "coding_session_state")
        .expect("session state");

    assert_eq!(state["attempt_scope"], "work_item_group");
    assert_eq!(state["work_item_group_id"], "work_item_plan_0001");
    assert_eq!(state["current_work_item_id"], "work_item_0001");
    assert_eq!(state["units"].as_array().expect("units").len(), 2);
    assert_eq!(state["units"][0]["status"], "running");
    assert_eq!(state["units"][1]["status"], "pending");
}
```

- [ ] **Step 2: Write failing frontend store test**

Add to `web/src/state/coding-workspace-store.test.ts`:

```ts
it("restores group context from coding session snapshot", () => {
  const store = useCodingWorkspaceStore.getState();

  store.setSessionState({
    type: "coding_session_state",
    attempt_id: "coding_attempt_0001",
    attempt_scope: "work_item_group",
    work_item_group_id: "work_item_plan_0001",
    current_work_item_id: "work_item_0001",
    active_unit_id: "coding_unit_0001",
    units: [
      {
        unit_id: "coding_unit_0001",
        work_item_id: "work_item_0001",
        order_index: 0,
        status: "running",
        summary: null,
        handoff_ref: null,
        completion_commit: null,
      },
      {
        unit_id: "coding_unit_0002",
        work_item_id: "work_item_0002",
        order_index: 1,
        status: "pending",
        summary: null,
        handoff_ref: null,
        completion_commit: null,
      },
    ],
    status: "running",
    stage: "coding",
    branch_name: "aria/issues/issue_0001",
    base_branch: "main",
    worktree_path: null,
    rework_count: 0,
    max_auto_rework: 2,
    head_commit: null,
    pushed_remote: null,
    provider_config_snapshot: { author: "codex", reviewer: "codex", review_rounds: 1 },
    role_provider_config_snapshot: roleSnapshot(),
    chat_entries: [],
    timeline_nodes: [],
    active_node_id: null,
    testing_report: null,
    code_review_reports: [],
    review_request: null,
    internal_pr_review: null,
    pending_gates: [],
    pending_choices: [],
    latest_analyst_decision: null,
    role_runs: [],
    work_item_execution_plan: null,
    work_item_handoff: null,
    require_execution_plan_confirm: false,
    verification_commands: [],
    work_item_markdown: null,
  });

  expect(useCodingWorkspaceStore.getState().attemptScope).toBe("work_item_group");
  expect(useCodingWorkspaceStore.getState().units).toHaveLength(2);
  expect(useCodingWorkspaceStore.getState().currentWorkItemId).toBe("work_item_0001");
});
```

- [ ] **Step 3: Run snapshot tests to verify they fail**

Run:

```bash
cargo test --locked --test it_web coding_ws_session_state_includes_group_units
cd web && ./node_modules/.bin/vitest --run src/state/coding-workspace-store.test.ts
```

Expected: FAIL because snapshot fields are absent.

- [ ] **Step 4: Add backend DTO and protocol fields**

In `src/web/types.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CodingExecutionUnitDto {
    pub unit_id: String,
    pub work_item_id: String,
    pub order_index: u32,
    pub status: String,
    pub summary: Option<String>,
    pub handoff_ref: Option<String>,
    pub completion_commit: Option<String>,
}
```

Extend `CodingAttemptSnapshotResponse`:

```rust
pub attempt_scope: String,
pub work_item_group_id: Option<String>,
pub current_work_item_id: Option<String>,
pub active_unit_id: Option<String>,
#[serde(default)]
pub units: Vec<CodingExecutionUnitDto>,
```

Extend `CodingWsOutMessage::CodingSessionState` in `src/web/coding_ws_handler/protocol.rs` with the same fields.

- [ ] **Step 5: Build snapshot units**

In `src/web/coding_ws_handler/socket.rs`, when creating `CodingSessionState`:

```rust
let units = coding_store
    .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
    .unwrap_or_default()
    .into_iter()
    .map(coding_execution_unit_dto)
    .collect::<Vec<_>>();
```

Use an empty vector for single-item attempts.

- [ ] **Step 6: Add frontend types and store fields**

In `web/src/api/types/coding.ts`:

```ts
export type CodingAttemptScope = "work_item" | "work_item_group";

export type CodingExecutionUnitStatus =
  | "pending"
  | "running"
  | "waiting_for_human"
  | "completed"
  | "failed"
  | "blocked"
  | "skipped";

export type CodingExecutionUnit = {
  unit_id: string;
  work_item_id: string;
  order_index: number;
  status: CodingExecutionUnitStatus;
  summary: string | null;
  handoff_ref: string | null;
  completion_commit: string | null;
};
```

Add to `CodingAttempt` and `CodingWsOutMessage`:

```ts
attempt_scope: CodingAttemptScope;
work_item_group_id: string | null;
current_work_item_id: string | null;
active_unit_id: string | null;
units: CodingExecutionUnit[];
```

In `web/src/state/coding-workspace-store.ts`, add state:

```ts
attemptScope: CodingAttemptScope | null;
workItemGroupId: string | null;
currentWorkItemId: string | null;
activeUnitId: string | null;
units: CodingExecutionUnit[];
```

Set them from snapshot in `setSessionState`.

- [ ] **Step 7: Run snapshot tests**

Run:

```bash
cargo test --locked --test it_web coding_ws_session_state_includes_group_units
cd web && ./node_modules/.bin/vitest --run src/state/coding-workspace-store.test.ts src/api/types.test.ts
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/web/types.rs src/web/coding_ws_handler web/src/api/types/coding.ts web/src/state/coding-workspace-store.ts tests/it_web/web_coding_ws_handler.rs web/src/state/coding-workspace-store.test.ts web/src/api/types.test.ts
git commit -m "feat(coding): expose group context in coding snapshots"
```

---

## Task 4: Current Work Item Context 与 Plan/Draft 只读增强

**Files:**
- Modify: `src/web/coding_ws_handler/context.rs`
- Modify: `src/product/coding_evaluation_context/builder.rs`
- Modify: `src/product/coding_evaluation_context/mod.rs`
- Test: `src/product/coding_evaluation_context/tests.rs`
- Test: `tests/it_web/web_coding_ws_handler.rs`

**Interfaces:**
- Produces:
  - `current_work_item_id_for_attempt(attempt: &CodingExecutionAttempt) -> &str`
  - `CodingGroupContextPack`
  - context warnings: `group_draft_context_loaded`、`group_draft_context_unavailable`、`group_plan_mapping_mismatch`

- [ ] **Step 1: Write failing context tests**

Add to `src/product/coding_evaluation_context/tests.rs`:

```rust
#[test]
fn group_attempt_uses_current_work_item_as_execution_context() {
    let (_tmp, paths, attempt) = group_attempt_with_two_work_items();

    let pack = build_evaluation_context_pack(
        paths,
        &attempt,
        EvaluationContextRole::Coder,
    )
    .expect("context pack");

    assert_eq!(pack.work_item.artifact_id, "work_item_0001");
    assert_eq!(pack.group_context.as_ref().expect("group").plan_id, "work_item_plan_0001");
    assert_eq!(
        pack.group_context.as_ref().expect("group").sibling_work_item_ids,
        vec!["work_item_0001".to_string(), "work_item_0002".to_string()]
    );
}

#[test]
fn group_context_warns_when_current_work_item_is_not_in_plan() {
    let (_tmp, paths, mut attempt) = group_attempt_with_two_work_items();
    attempt.current_work_item_id = Some("work_item_outside".to_string());

    let pack = build_evaluation_context_pack(
        paths,
        &attempt,
        EvaluationContextRole::Coder,
    )
    .expect("context pack");

    assert!(pack.context_warnings.contains(&"group_plan_mapping_mismatch".to_string()));
}
```

- [ ] **Step 2: Run context tests to verify they fail**

Run:

```bash
cargo test --locked --lib coding_evaluation_context
```

Expected: FAIL because group context types and current work item helper do not exist.

- [ ] **Step 3: Add current work item helper**

In `src/web/coding_ws_handler/context.rs`, add:

```rust
pub(crate) fn current_work_item_id_for_attempt(attempt: &CodingExecutionAttempt) -> &str {
    attempt
        .current_work_item_id
        .as_deref()
        .unwrap_or(&attempt.work_item_id)
}
```

Replace direct reads of `attempt.work_item_id` in:

- `coding_execution_context`
- `ensure_work_item_execution_plan_confirmed`
- `repository_path_for_attempt`
- `test_specs_for_attempt` if it resolves Work Item-specific metadata.

- [ ] **Step 4: Add group context model**

In `src/product/coding_evaluation_context/mod.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CodingGroupContextPack {
    pub plan_id: String,
    pub current_work_item_id: String,
    pub sibling_work_item_ids: Vec<String>,
    pub dependency_handoff_refs: Vec<String>,
    pub source_outline_id: Option<String>,
    pub source_draft_id: Option<String>,
}
```

Extend `EvaluationContextPack`:

```rust
pub group_context: Option<CodingGroupContextPack>,
```

Single-item attempts set `group_context: None`.

- [ ] **Step 5: Build group context in evaluation builder**

In `src/product/coding_evaluation_context/builder.rs`:

- Resolve `current_work_item_id` via `attempt.current_work_item_id.as_deref().unwrap_or(&attempt.work_item_id)`.
- If `attempt.scope == CodingAttemptScope::WorkItemGroup`, load `IssueWorkItemPlan` from `attempt.work_item_group_id`.
- Fill `sibling_work_item_ids` from `plan.work_item_ids`.
- Fill `dependency_handoff_refs` from current Work Item `required_handoff_from` records that have `handoff_summary_ref`.
- If current work item not in `plan.work_item_ids`, push `group_plan_mapping_mismatch`.
- Read source draft mapping only when available from compile transaction; if unavailable, keep `source_outline_id=None` and `source_draft_id=None` with warning `group_draft_context_unavailable`.

- [ ] **Step 6: Run context tests**

Run:

```bash
cargo test --locked --lib coding_evaluation_context
cargo test --locked --test it_web coding_ws_session_state_includes_group_units
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/web/coding_ws_handler/context.rs src/product/coding_evaluation_context src/product/coding_evaluation_context/tests.rs tests/it_web/web_coding_ws_handler.rs
git commit -m "feat(coding): resolve current work item for group attempts"
```

---

## Task 5: 串行 Unit Runner 与 Unit Handoff

**Files:**
- Create: `src/product/coding_workspace_engine/group.rs`
- Modify: `src/product/coding_workspace_engine/mod.rs`
- Modify: `src/product/coding_workspace_engine/handoffs.rs`
- Modify: `src/web/coding_ws_handler/runner.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`
- Test: `tests/it_web/web_coding_ws_handler.rs`

**Interfaces:**
- Consumes:
  - `CodingAttemptScope::WorkItemGroup`
  - `CodingAttemptStore::get_active_coding_unit`
  - `CodingAttemptStore::update_coding_unit_status`
- Produces:
  - `CodingWorkspaceEngine::complete_current_group_unit`
  - `CodingWorkspaceEngine::advance_to_next_group_unit`
  - `CodingWorkspaceEngine::group_attempt_ready_for_final_review`

- [ ] **Step 1: Write failing product tests**

Add to `tests/it_product/product_coding_workspace_engine.rs`:

```rust
#[tokio::test]
async fn completing_group_unit_marks_current_unit_completed_and_next_running() {
    let (paths, store, engine, attempt) = group_engine_with_two_units();

    let updated = engine
        .complete_current_group_unit(&attempt, Some("unit handoff saved".to_string()))
        .await
        .expect("complete unit");

    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units");
    assert_eq!(updated.current_work_item_id.as_deref(), Some("work_item_0002"));
    assert_eq!(units[0].status, CodingExecutionUnitStatus::Completed);
    assert_eq!(units[1].status, CodingExecutionUnitStatus::Running);
    assert!(paths.app_root().exists());
}

#[tokio::test]
async fn completing_last_group_unit_enters_review_request_stage() {
    let (_paths, store, engine, attempt) = group_engine_with_last_running_unit();

    let updated = engine
        .complete_current_group_unit(&attempt, Some("last unit done".to_string()))
        .await
        .expect("complete last unit");

    assert_eq!(updated.stage, CodingExecutionStage::ReviewRequest);
    assert!(engine
        .group_attempt_ready_for_final_review(&updated)
        .expect("ready"));
    assert!(store
        .list_coding_units(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("units")
        .iter()
        .all(|unit| unit.status == CodingExecutionUnitStatus::Completed));
}
```

- [ ] **Step 2: Run product tests to verify they fail**

Run:

```bash
cargo test --locked --test it_product completing_group_unit_marks_current_unit_completed_and_next_running
cargo test --locked --test it_product completing_last_group_unit_enters_review_request_stage
```

Expected: FAIL because group engine methods are missing.

- [ ] **Step 3: Implement group engine module**

Create `src/product/coding_workspace_engine/group.rs`:

```rust
impl CodingWorkspaceEngine {
    pub async fn complete_current_group_unit(
        &self,
        attempt: &CodingExecutionAttempt,
        summary: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Ok(attempt.clone());
        }
        let active = self
            .store
            .get_active_coding_unit(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .ok_or_else(|| CodingWorkspaceEngineError::WorkItemHandoffMissing(attempt.id.clone()))?;
        self.store.update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &active.id,
            CodingExecutionUnitStatus::Completed,
            summary,
        )?;
        self.advance_to_next_group_unit(attempt).await
    }

    pub async fn advance_to_next_group_unit(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let units = self
            .store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        if let Some(next) = units
            .iter()
            .filter(|unit| unit.status == CodingExecutionUnitStatus::Pending)
            .min_by_key(|unit| unit.order_index)
        {
            self.store.update_coding_unit_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &next.id,
                CodingExecutionUnitStatus::Running,
                Some("进入下一个 Work Item".to_string()),
            )?;
            let mut updated = attempt.clone();
            updated.current_work_item_id = Some(next.work_item_id.clone());
            updated.active_unit_id = Some(next.id.clone());
            updated.stage = CodingExecutionStage::PrepareContext;
            updated.status = CodingAttemptStatus::Running;
            updated.updated_at = Utc::now().to_rfc3339();
            self.store.save_coding_attempt(&updated)?;
            return Ok(updated);
        }
        let updated = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )?;
        Ok(updated)
    }

    pub fn group_attempt_ready_for_final_review(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<bool, CodingWorkspaceEngineError> {
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Ok(false);
        }
        Ok(self
            .store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .iter()
            .all(|unit| unit.status == CodingExecutionUnitStatus::Completed))
    }
}
```

Register `mod group;` in `src/product/coding_workspace_engine/mod.rs`.

- [ ] **Step 4: Separate unit handoff from group final confirm**

In `src/product/coding_workspace_engine/handoffs.rs`:

- `generate_and_save_work_item_handoff_if_missing` must use current Work Item id for group attempts.
- Add:

```rust
pub async fn complete_group_unit_after_code_review(
    &self,
    attempt: &CodingExecutionAttempt,
) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
    self.generate_and_save_work_item_handoff_if_missing(attempt).await?;
    self.complete_current_group_unit(attempt, Some("当前 Work Item 已完成".to_string())).await
}
```

Single-item `handle_final_confirm` remains unchanged for `scope=work_item`.

- [ ] **Step 5: Modify runner after code review**

In `src/web/coding_ws_handler/runner.rs`, after unit-level code review rework loop reaches `CodingExecutionStage::ReviewRequest`, branch by scope:

```rust
if current.scope == CodingAttemptScope::WorkItemGroup {
    current = engine.complete_group_unit_after_code_review(&current).await?;
    emit_current_session_state(event_tx, coding_store, &current).await?;
    if current.stage == CodingExecutionStage::PrepareContext {
        continue 'pipeline;
    }
    if current.stage == CodingExecutionStage::ReviewRequest {
        let review_request = engine
            .execute_review_request(&current, "origin", "feat: implement work item group")
            .await?;
        current = coding_store.get_attempt(&current.project_id, &current.issue_id, &current.id)?;
        if review_request.push_status != crate::product::coding_models::PushStatus::Pushed {
            return emit_current_session_state(event_tx, coding_store, &current).await;
        }
    }
}
```

The group path must not call `execute_review_request` until all units are completed.

- [ ] **Step 6: Run runner tests**

Run:

```bash
cargo test --locked --test it_product completing_group_unit_marks_current_unit_completed_and_next_running
cargo test --locked --test it_product completing_last_group_unit_enters_review_request_stage
cargo test --locked --test it_web web_coding_ws_handler
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/product/coding_workspace_engine src/web/coding_ws_handler/runner.rs tests/it_product/product_coding_workspace_engine.rs tests/it_web/web_coding_ws_handler.rs
git commit -m "feat(coding): run group units serially"
```

---

## Task 6: 整组 Review Request / Internal PR Review / Final Confirm

**Files:**
- Modify: `src/product/coding_workspace_engine/internal_pr_review.rs`
- Modify: `src/product/coding_workspace_engine/handoffs.rs`
- Modify: `src/product/coding_workspace_engine/reports.rs`
- Modify: `src/web/coding_ws_handler/runner.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`
- Test: `tests/it_web/web_coding_ws_handler.rs`

**Interfaces:**
- Produces:
  - group-level review request uses all completed units.
  - group-level internal PR review prompt includes all unit handoffs.
  - group final confirm marks group attempt completed and keeps all Work Items completed.

- [ ] **Step 1: Write failing final review tests**

Add to `tests/it_product/product_coding_workspace_engine.rs`:

```rust
#[tokio::test]
async fn group_internal_review_prompt_includes_all_unit_handoffs() {
    let (_paths, _store, engine, attempt) = completed_group_attempt_with_handoffs();

    let prompt = engine
        .build_group_internal_pr_review_prompt_for_test(&attempt)
        .await
        .expect("prompt");

    assert!(prompt.contains("work_item_0001"));
    assert!(prompt.contains("work_item_0002"));
    assert!(prompt.contains("handoff summary for backend"));
    assert!(prompt.contains("handoff summary for frontend"));
}

#[tokio::test]
async fn group_final_confirm_completes_attempt_after_all_units_completed() {
    let (_paths, store, engine, attempt) = group_attempt_waiting_for_final_confirm();

    let updated = engine
        .handle_final_confirm(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("final confirm");

    assert_eq!(updated.status, CodingAttemptStatus::Completed);
    assert_eq!(updated.scope, CodingAttemptScope::WorkItemGroup);
    assert!(store
        .list_coding_units(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("units")
        .iter()
        .all(|unit| unit.status == CodingExecutionUnitStatus::Completed));
}
```

- [ ] **Step 2: Run final review tests to verify they fail**

Run:

```bash
cargo test --locked --test it_product group_internal_review_prompt_includes_all_unit_handoffs
cargo test --locked --test it_product group_final_confirm_completes_attempt_after_all_units_completed
```

Expected: FAIL because group final review prompt and confirm semantics are not implemented.

- [ ] **Step 3: Build group internal review prompt**

In `src/product/coding_workspace_engine/internal_pr_review.rs`, add:

```rust
async fn build_group_internal_pr_review_prompt(
    &self,
    attempt: &CodingExecutionAttempt,
    review_request: &ReviewRequest,
    worktree_path: &Path,
    retry_diagnostic: Option<&str>,
) -> Result<String, CodingWorkspaceEngineError> {
    let units = self.store.list_coding_units(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let handoffs = units
        .iter()
        .map(|unit| {
            self.store
                .get_work_item_handoff(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .map(|handoff| (unit, handoff))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!(
        "你是整组 PR 的最终 reviewer。\n\nReview Request: {}\nWorktree: {}\nUnits:\n{}\nRetry Diagnostic:\n{}",
        review_request.id,
        worktree_path.display(),
        format_group_unit_handoff_section(&handoffs),
        retry_diagnostic.unwrap_or("无")
    ))
}
```

`format_group_unit_handoff_section` must include work item id、unit status、completion commit、handoff summary、tests run、risk notes.

In `execute_internal_pr_review_with_commands`, choose prompt builder:

```rust
let prompt = if attempt.scope == CodingAttemptScope::WorkItemGroup {
    self.build_group_internal_pr_review_prompt(&attempt, &review_request, worktree_path, retry_diagnostic.as_deref()).await?
} else {
    self.build_internal_pr_review_prompt(&attempt, &review_request, worktree_path, retry_diagnostic.as_deref()).await?
};
```

- [ ] **Step 4: Guard group final confirm**

In `handle_final_confirm`:

```rust
if current.scope == CodingAttemptScope::WorkItemGroup
    && !self.group_attempt_ready_for_final_review(&current)?
{
    return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(attempt_id.to_string()));
}
```

For group attempts, do not update only `updated.work_item_id`; instead:

- keep completed unit work item statuses as already completed;
- set group attempt status to completed;
- release shared worktree lock using `current.current_work_item_id` if it is still the holder;
- complete active final confirm timeline node.

- [ ] **Step 5: Run final review tests**

Run:

```bash
cargo test --locked --test it_product group_internal_review_prompt_includes_all_unit_handoffs
cargo test --locked --test it_product group_final_confirm_completes_attempt_after_all_units_completed
cargo test --locked --test it_web web_coding_ws_handler
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/product/coding_workspace_engine src/web/coding_ws_handler/runner.rs tests/it_product/product_coding_workspace_engine.rs tests/it_web/web_coding_ws_handler.rs
git commit -m "feat(coding): add group final review flow"
```

---

## Task 7: Frontend Group Coding 入口与进度展示

**Files:**
- Modify: `web/src/api/client.ts`
- Modify: `web/src/api/types/coding.ts`
- Modify: `web/src/api/types/lifecycle.ts`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbenchParts.tsx`
- Create: `web/src/pages/CodingWorkspaceGroupProgress.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Test: `web/src/api/coding-attempts.test.ts`
- Test: `web/src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx`
- Test: `web/src/pages/CodingWorkspacePage.test.tsx`

**Interfaces:**
- Consumes:
  - `POST /api/projects/{projectId}/issues/{issueId}/work-item-plans/{planId}/coding-attempts`
  - `CodingWorkspaceState.units`
- Produces:
  - `createGroupCodingAttempt(projectId, issueId, planId)`
  - WorkItemGroup drawer action.
  - `CodingWorkspaceGroupProgress`.

- [ ] **Step 1: Write failing API client test**

Add to `web/src/api/coding-attempts.test.ts`:

```ts
it("creates group coding attempts from work item plan route", async () => {
  await createGroupCodingAttempt("project/with space", "issue/with space", "plan/1");

  expect(fetchMock).toHaveBeenCalledWith(
    "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/work-item-plans/plan%2F1/coding-attempts",
    expect.objectContaining({ method: "POST" }),
  );
});
```

- [ ] **Step 2: Write failing page test**

Add to `web/src/pages/CodingWorkspacePage.test.tsx`:

```tsx
it("shows group progress and current work item for group attempts", async () => {
  mockCodingSessionState({
    attempt_scope: "work_item_group",
    work_item_group_id: "work_item_plan_0001",
    current_work_item_id: "work_item_0001",
    active_unit_id: "coding_unit_0001",
    units: [
      {
        unit_id: "coding_unit_0001",
        work_item_id: "work_item_0001",
        order_index: 0,
        status: "running",
        summary: null,
        handoff_ref: null,
        completion_commit: null,
      },
      {
        unit_id: "coding_unit_0002",
        work_item_id: "work_item_0002",
        order_index: 1,
        status: "pending",
        summary: null,
        handoff_ref: null,
        completion_commit: null,
      },
    ],
  });

  render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

  expect(await screen.findByText("WorkItemGroup")).toBeInTheDocument();
  expect(screen.getByText("1 / 2")).toBeInTheDocument();
  expect(screen.getByText("work_item_0001")).toBeInTheDocument();
});
```

- [ ] **Step 3: Run frontend tests to verify they fail**

Run:

```bash
cd web && ./node_modules/.bin/vitest --run src/api/coding-attempts.test.ts src/pages/CodingWorkspacePage.test.tsx
```

Expected: FAIL because group API and UI are not implemented.

- [ ] **Step 4: Implement API client**

In `web/src/api/client.ts`:

```ts
export function createGroupCodingAttempt(
  projectId: string,
  issueId: string,
  planId: string,
): Promise<CodingAttempt> {
  return requestJson<CodingAttempt>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/work-item-plans/${encodeURIComponent(planId)}/coding-attempts`,
    {
      method: "POST",
      body: JSON.stringify({}),
    },
  );
}
```

- [ ] **Step 5: Implement group progress component**

Create `web/src/pages/CodingWorkspaceGroupProgress.tsx`:

```tsx
import type { CodingExecutionUnit } from "../api/types";

export function CodingWorkspaceGroupProgress({
  planId,
  currentWorkItemId,
  units,
}: {
  planId: string | null;
  currentWorkItemId: string | null;
  units: CodingExecutionUnit[];
}) {
  if (!planId || units.length === 0) return null;
  const completed = units.filter((unit) => unit.status === "completed").length;
  return (
    <section className="flex min-h-10 shrink-0 items-center justify-between gap-3 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-2 text-xs">
      <div className="min-w-0">
        <div className="font-semibold text-[var(--aria-ink)]">WorkItemGroup</div>
        <div className="truncate font-mono text-[var(--aria-ink-muted)]">{planId}</div>
      </div>
      <div className="shrink-0 text-[var(--aria-ink-muted)]">{completed + 1} / {units.length}</div>
      <div className="min-w-0 truncate font-mono text-[var(--aria-ink-muted)]">
        {currentWorkItemId ?? "未选择 Work Item"}
      </div>
    </section>
  );
}
```

- [ ] **Step 6: Wire page header**

In `web/src/pages/CodingWorkspacePage.tsx`, import and render `CodingWorkspaceGroupProgress` above `CodingProviderConfigPanel` or directly below the header:

```tsx
<CodingWorkspaceGroupProgress
  planId={store.workItemGroupId}
  currentWorkItemId={store.currentWorkItemId}
  units={store.units}
/>
```

Keep the existing single-item header unchanged when `store.attemptScope !== "work_item_group"`.

- [ ] **Step 7: Wire Lifecycle WorkItemGroup action**

In `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`, update `handleOpenCodingWorkspaceFromDrawer`:

```ts
if (card.kind === "work_item_group") {
  if (card.raw.latest_group_attempt) {
    onOpenCodingWorkspace(card.raw.latest_group_attempt.attempt_id);
    return;
  }
  const attempt = await createGroupCodingAttempt(selectedProjectId, card.issueId, card.id);
  await refresh(selectedProjectId);
  onOpenCodingWorkspace(attempt.attempt_id);
  return;
}
```

If lifecycle response does not yet provide `latest_group_attempt`, find an active attempt from `lifecycle.coding_attempts` where `attempt.work_item_group_id === card.id`.

- [ ] **Step 8: Run frontend tests**

Run:

```bash
cd web && ./node_modules/.bin/vitest --run src/api/coding-attempts.test.ts src/state/coding-workspace-store.test.ts src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx src/pages/CodingWorkspacePage.test.tsx
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add web/src/api web/src/state web/src/components/lifecycle web/src/pages
git commit -m "feat(coding): add group coding workspace UI"
```

---

## Task 8: 回归验证与收口

**Files:**
- Test only; no product source changes unless earlier tests expose defects.

**Interfaces:**
- Confirms:
  - 单 Work Item Coding Workspace remains compatible.
  - WorkItemPlan Final Compile remains compatible.
  - Story / Design / Work Item Workspace shared artifact chain remains compatible.

- [ ] **Step 1: Run focused backend regression**

Run:

```bash
cargo test --locked --test it_web web_coding_attempt_api
cargo test --locked --test it_web web_coding_ws_handler
cargo test --locked --test it_web web_work_item_plan_compile
cargo test --locked --test it_web web_workspace_recovery_consistency
cargo test --locked --test it_product product_coding_workspace_engine
cargo test --locked --lib coding_attempt_store
cargo test --locked --lib coding_evaluation_context
```

Expected: all listed commands PASS.

- [ ] **Step 2: Run frontend focused regression**

Run:

```bash
cd web && ./node_modules/.bin/vitest --run src/api/coding-attempts.test.ts src/state/coding-workspace-store.test.ts src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx src/pages/CodingWorkspacePage.test.tsx src/pages/CodingWorkspacePage.execution-plan.test.tsx src/pages/CodingWorkspacePage.gates.test.tsx src/pages/CodingWorkspacePage.reports.test.tsx
```

Expected: all listed Vitest files PASS.

- [ ] **Step 3: Run full required checks**

Run from repository root:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

Run from `web/`:

```bash
./node_modules/.bin/tsc --noEmit
./node_modules/.bin/vitest --run
./node_modules/.bin/tsc -b
./node_modules/.bin/vite build
```

Expected: all commands exit 0. `vite build` may print chunk size warnings; warnings are acceptable if exit code is 0.

- [ ] **Step 4: Inspect final diff**

Run:

```bash
git status --short
git diff --check
```

Expected:

- `git diff --check` exits 0.
- `git status --short` contains only intended source, test, and plan files.

- [ ] **Step 5: Commit final verification notes if docs changed**

If implementation added or updated plan/report docs, commit them:

```bash
git add cadence/plans cadence/reports
git commit -m "docs(coding): record group coding workspace verification"
```

If no docs changed, skip this commit.

## Execution Notes

- Do not use `cargo test --locked -j 1`.
- Do not run Docker as the default Rust verification path.
- Keep each task as an independently reviewable commit.
- If a task exposes a shared Workspace artifact bug, apply the project triage rule: evaluate Story、Design、Work Item workspace chains before reporting completion.
- If frontend text or layout changes are needed beyond this plan, keep the first viewport work-focused and dense; do not add a marketing-style page.
