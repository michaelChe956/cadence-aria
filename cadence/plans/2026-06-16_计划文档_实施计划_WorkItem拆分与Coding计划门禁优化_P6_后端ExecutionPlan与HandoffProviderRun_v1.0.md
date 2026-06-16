# WorkItem 拆分 P6 后端 WorkItemExecutionPlan 与 Handoff Provider Run Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Coding 前生成内部 `WorkItemExecutionPlan`，默认展示但不阻塞；Work Item 完成前运行额外 provider handoff run，缺 handoff 不允许完成或解锁依赖项。

**Architecture:** Execution plan 与 handoff 都存 Aria 内部数据。`WorkItemExecutionPlan` 作为 Coding prompt 的结构化来源，只有 `require_execution_plan_confirm=true` 时阻塞；`WorkItemHandoff` 在 review/final confirm 前由额外 provider run 或 fake provider 摘要生成，并成为 Work Item 完成门禁。

**Tech Stack:** Rust 1.95.0、CodingAttemptStore、CodingWorkspaceEngine、Provider adapter、Serde JSON、Cargo integration tests、TDD。

---

## 前置交付摘要

执行本计划前确认 P5 已交付：

- `create_coding_attempt` 已检查依赖、handoff 可读性和 active lock。
- Coding attempt 已使用 `aria/issues/{issue_id}` branch 与 `.worktrees/aria-issues/{issue_id}` worktree。
- `handle_final_confirm()` 会释放 active lock 并记录 completed Work Item。

## 计划大小边界

本计划只做后端 execution plan 与 handoff：

- 不修改 Product Workbench Work Item DAG UI。
- 不修改 Coding Prepare 前端展示。
- 不写 Playwright E2E。
- 不改变 P5 已建立的 shared worktree branch/path 规则。

如果需要前端展示字段，后端只扩展 DTO/WS payload；UI 渲染留给 P8。

## 文件结构

- Modify: `src/product/coding_models.rs`
  - 新增 `WorkItemExecutionPlan`、`WorkItemHandoff`、状态和输入结构。
- Modify: `src/product/coding_attempt_store.rs`
  - 增加 execution plan 与 handoff 存取 API。
- Modify: `src/product/coding_workspace_engine.rs`
  - Prepare/Coding prompt 注入 execution plan。
  - final confirm 前检查 handoff。
  - 增加 handoff generation step/helper。
- Modify: `src/web/types.rs`
  - `CodingAttemptSnapshotResponse` 包含 `work_item_execution_plan` 和 `work_item_handoff`。
- Modify: `src/web/workspace_ws_types.rs`
  - `CodingWsOutMessage::CodingSessionState` 包含 `work_item_execution_plan` 和 `work_item_handoff`。
- Modify: `src/web/coding_ws_handler.rs`
  - Coding WS 初始 state 和进入 Coder 前门禁读取 execution plan。
- Modify: `src/web/handlers.rs`
  - snapshot 返回新增字段。
  - 新增 execution plan confirm/change request API。
- Modify: `tests/it_product/product_coding_attempt_store.rs`
- Modify: `tests/it_product/product_coding_workspace_engine.rs`
- Modify: `tests/it_web/web_coding_attempt_api.rs`
- Modify: `tests/it_web/web_coding_ws_handler.rs`

## 任务 1：Persist WorkItemExecutionPlan

**文件：**

- Modify: `src/product/coding_models.rs`
- Modify: `src/product/coding_attempt_store.rs`
- Modify: `tests/it_product/product_coding_attempt_store.rs`

- [ ] **步骤 1：编写失败态 store tests**

Append to `tests/it_product/product_coding_attempt_store.rs`:

```rust
#[test]
fn saves_and_loads_work_item_execution_plan() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let plan = WorkItemExecutionPlan {
        id: "work_item_execution_plan_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        status: WorkItemExecutionPlanStatus::Draft,
        goal: "实现后端 API".to_string(),
        allowed_write_scopes: vec!["src/product/**".to_string()],
        forbidden_write_scopes: vec!["web/**".to_string()],
        dependency_handoffs: Vec::new(),
        story_refs: vec!["story_spec_0001".to_string()],
        design_refs: vec!["design_spec_0001".to_string()],
        openspec_refs: vec!["REQ-001".to_string()],
        superpowers_contract: "use superpowers:test-driven-development".to_string(),
        tdd_contract: "先写失败测试，再写实现".to_string(),
        verification_commands: vec!["cargo test --locked --test it_product backend_api".to_string()],
        risk_notes: Vec::new(),
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    };

    store
        .save_work_item_execution_plan(&plan)
        .expect("save execution plan");

    let loaded = store
        .get_work_item_execution_plan("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("load execution plan")
        .expect("plan exists");
    assert_eq!(loaded.goal, "实现后端 API");
    assert_eq!(loaded.status, WorkItemExecutionPlanStatus::Draft);
}
```

- [ ] **步骤 2：运行 test 并确认失败**

运行:

```bash
cargo test --locked --test it_product saves_and_loads_work_item_execution_plan
```

预期：编译失败，因为 model/store APIs do not exist.

- [ ] **步骤 3：添加 models and store methods**

在 `src/product/coding_models.rs`, add `WorkItemExecutionPlanStatus`, `WorkItemExecutionPlan`, and `WorkItemDependencyHandoffRef`.

在 `src/product/coding_attempt_store.rs`, persist under:

```text
projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/work-item-execution-plan.json
```

Add:

```rust
pub fn save_work_item_execution_plan(&self, plan: &WorkItemExecutionPlan) -> Result<(), ProductStoreError>
pub fn get_work_item_execution_plan(&self, project_id: &str, issue_id: &str, attempt_id: &str) -> Result<Option<WorkItemExecutionPlan>, ProductStoreError>
pub fn update_work_item_execution_plan_status(&self, project_id: &str, issue_id: &str, attempt_id: &str, status: WorkItemExecutionPlanStatus) -> Result<WorkItemExecutionPlan, ProductStoreError>
```

- [ ] **步骤 4：运行 store test 并确认通过**

重新运行步骤 2 的命令。

预期：通过。

## 任务 2：Generate Execution Plan On Coding Attempt Start

**文件：**

- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `src/web/handlers.rs`
- Modify: `tests/it_web/web_coding_attempt_api.rs`

- [ ] **步骤 1：编写失败态 snapshot test**

Append to `tests/it_web/web_coding_attempt_api.rs`:

```rust
#[tokio::test]
async fn coding_attempt_snapshot_includes_generated_work_item_execution_plan() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;

    let (_status, attempt) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(attempt["attempt_id"], "coding_attempt_0001");

    let (status, snapshot) = request_json(
        app,
        Method::GET,
        "/api/coding-attempts/coding_attempt_0001",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        snapshot["work_item_execution_plan"]["work_item_id"],
        "work_item_0001"
    );
    assert_eq!(snapshot["work_item_execution_plan"]["status"], "draft");
    assert!(
        snapshot["work_item_execution_plan"]["verification_commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command.as_str().unwrap().contains("cargo"))
    );
}
```

- [ ] **步骤 2：运行 snapshot test 并确认失败**

运行:

```bash
cargo test --locked --test it_web coding_attempt_snapshot_includes_generated_work_item_execution_plan
```

预期：snapshot 缺少 `work_item_execution_plan`.

- [ ] **步骤 3：创建 execution plan during attempt creation**

在 `CodingAttemptStore::create_attempt()`, build and save a draft execution plan using:

- Work Item title as `goal`.
- Work Item `exclusive_write_scopes` and `forbidden_write_scopes`.
- `required_handoff_from` as dependency refs.
- Story/Design IDs as refs.
- Verification commands from Work Item kind:
  - backend/integration: include cargo commands.
  - frontend/e2e: include pnpm/vitest or Playwright commands.

不要 block attempt creation when `require_execution_plan_confirm=false`.

- [ ] **步骤 4：扩展 snapshot response**

在 `src/web/types.rs`, add:

```rust
pub work_item_execution_plan: Option<WorkItemExecutionPlan>,
pub work_item_handoff: Option<WorkItemHandoff>,
```

to `CodingAttemptSnapshotResponse`.

Update `get_coding_attempt()` to load both optional records.

- [ ] **步骤 5：运行 snapshot test 并确认通过**

重新运行步骤 2 的命令。

预期：通过。

## 任务 3：Execution Plan Confirmation Gate Is Configurable

**文件：**

- Modify: `src/web/handlers.rs`
- Modify: `tests/it_web/web_coding_attempt_api.rs`

- [ ] **步骤 1：编写失败态 gate tests**

追加:

```rust
#[tokio::test]
async fn coding_ws_blocks_coder_stage_when_execution_plan_requires_confirmation() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_requiring_execution_plan_confirm(app.clone(), repo.path()).await;

    let (status, attempt) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let ws_error = start_coding_ws_until_error(app, attempt["attempt_id"].as_str().unwrap()).await;

    assert_eq!(ws_error["code"], "work_item_execution_plan_not_confirmed");
}
```

- [ ] **步骤 2：运行 gate test 并确认失败**

运行:

```bash
cargo test --locked --test it_web coding_ws_blocks_coder_stage_when_execution_plan_requires_confirmation
```

预期：当前 flow does not enforce configurable gate.

- [ ] **步骤 3：实现 confirm/change request API**

添加 routes:

```text
POST /api/coding-attempts/{attempt_id}/execution-plan/confirm
POST /api/coding-attempts/{attempt_id}/execution-plan/change-request
```

The confirm route sets `WorkItemExecutionPlanStatus::Confirmed` and updates the linked `LifecycleWorkItemRecord.execution_plan_status`.

The change-request route sets `ChangeRequested` and stores a risk note or user note if included in payload.

- [ ] **步骤 4：Enforce gate at Coder start**

Enforce the configurable gate when websocket runner transitions from Prepare/WorktreePrepare into Coding. Attempt creation remains allowed because it is the point where the draft execution plan is generated and exposed to the user.

Rule:

- `require_execution_plan_confirm=false`: draft plan is allowed.
- `require_execution_plan_confirm=true`: status must be `Confirmed` before Coder provider run.

- [ ] **步骤 5：运行 gate tests 并确认通过**

运行:

```bash
cargo test --locked --test it_web execution_plan
cargo test --locked --test it_product execution_plan
```

预期：可配置门禁按预期生效。

## 任务 4：Persist WorkItemHandoff And Require It Before Completion

**文件：**

- Modify: `src/product/coding_models.rs`
- Modify: `src/product/coding_attempt_store.rs`
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `tests/it_product/product_coding_attempt_store.rs`
- Modify: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **步骤 1：编写失败态 handoff store test**

Append to `tests/it_product/product_coding_attempt_store.rs`:

```rust
#[test]
fn saves_and_loads_work_item_handoff() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let handoff = WorkItemHandoff {
        id: "work_item_handoff_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        provider_run_ref: Some("provider-raw/handoff/work_item_0001.txt".to_string()),
        summary: "后端 API 已完成，前端可调用 /api/session".to_string(),
        files_changed: vec!["src/web/handlers.rs".to_string()],
        commit_sha: Some("abc123".to_string()),
        diff_summary: "新增 session API".to_string(),
        tests_run: vec!["cargo test --locked --test it_web session_api".to_string()],
        test_result_summary: "全部通过".to_string(),
        review_summary: Some("无阻塞问题".to_string()),
        api_or_contract_changes: vec!["GET /api/session".to_string()],
        open_risks: Vec::new(),
        next_work_item_notes: vec!["前端处理 401".to_string()],
        created_at: "2026-06-16T00:00:00Z".to_string(),
    };

    store.save_work_item_handoff(&handoff).expect("save handoff");

    let loaded = store
        .get_work_item_handoff("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("load handoff")
        .expect("handoff exists");
    assert_eq!(loaded.summary, handoff.summary);
}
```

- [ ] **步骤 2：运行 handoff store test 并确认失败**

运行:

```bash
cargo test --locked --test it_product saves_and_loads_work_item_handoff
```

预期：模型或 store API 缺失导致失败。

- [ ] **步骤 3：添加 handoff model and store methods**

Persist under:

```text
projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/work-item-handoff.json
```

添加 `save_work_item_handoff()` and `get_work_item_handoff()`.

- [ ] **步骤 4：编写失败态 completion gate test**

Append to `tests/it_product/product_coding_workspace_engine.rs`:

```rust
#[tokio::test]
async fn final_confirm_requires_work_item_handoff() {
    let root = tempdir().expect("root");
    let (store, attempt) = final_confirm_attempt(ProductAppPaths::new(root.path().join(".aria")), "work_item_0001");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let error = engine
        .handle_final_confirm("project_0001", "issue_0001", &attempt.id)
        .await
        .expect_err("missing handoff blocks completion");

    assert!(format!("{error}").contains("work_item_handoff_missing"));
}
```

- [ ] **步骤 5：实现 completion gate**

Before setting attempt/work item completed in `handle_final_confirm()`, require a saved `WorkItemHandoff`. After success, update `LifecycleWorkItemRecord.handoff_summary_ref` and `completion_commit` from the handoff/review request.

- [ ] **步骤 6：运行 handoff tests 并确认通过**

运行:

```bash
cargo test --locked --test it_product saves_and_loads_work_item_handoff
cargo test --locked --test it_product final_confirm_requires_work_item_handoff
```

预期：两条测试都通过。

## 任务 5：Generate Handoff From Provider Or Fake Summary

**文件：**

- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **步骤 1：编写失败态 handoff generation test**

追加:

```rust
#[tokio::test]
async fn generates_handoff_from_review_and_test_summaries_before_final_confirm() {
    let root = tempdir().expect("root");
    let (store, attempt) = attempt_with_review_request_and_testing_report(root.path());
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let handoff = engine
        .generate_work_item_handoff(&attempt)
        .await
        .expect("generate handoff");

    assert_eq!(handoff.work_item_id, "work_item_0001");
    assert!(handoff.summary.contains("work_item_0001"));
    assert!(!handoff.tests_run.is_empty());
}
```

- [ ] **步骤 2：运行 generation test 并确认失败**

运行:

```bash
cargo test --locked --test it_product generates_handoff_from_review_and_test_summaries_before_final_confirm
```

预期：方法缺失导致失败。

- [ ] **步骤 3：实现 handoff generation helper**

第一版 uses a deterministic summary from persisted testing report, code review report, internal review and review request, and stores `provider_run_ref=None`. A later provider-backed handoff run can replace this helper without changing the persisted `WorkItemHandoff` contract.

不要 consume next Work Item context budget while generating handoff.

- [ ] **步骤 4：运行 generation test 并确认通过**

重新运行步骤 2 的命令。

预期：通过。

## 最终验证

运行:

```bash
cargo test --locked --test it_product work_item_execution_plan
cargo test --locked --test it_product work_item_handoff
cargo test --locked --test it_web execution_plan
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

预期:

- Execution plan tests pass.
- Handoff tests pass.
- Formatting, clippy and check pass.

## 提交

```bash
git add src/product/coding_models.rs src/product/coding_attempt_store.rs src/product/coding_workspace_engine.rs src/web/types.rs src/web/handlers.rs tests/it_product/product_coding_attempt_store.rs tests/it_product/product_coding_workspace_engine.rs tests/it_web/web_coding_attempt_api.rs
git commit -m "feat: add work item execution plan and handoff"
```
