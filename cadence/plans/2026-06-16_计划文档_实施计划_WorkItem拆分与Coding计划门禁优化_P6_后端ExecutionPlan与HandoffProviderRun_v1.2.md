# WorkItem 拆分 P6 后端 WorkItemExecutionPlan 与 Handoff Provider Run Implementation Plan

> **文档版本：** v1.2
>
> **v1.1 修订摘要：** 预设拆分点（execution plan 任务 1-3 / handoff 与 diff scope 任务 4-5 分两段提交）；修正最终验证过滤名为实际测试函数子串；纠正 `CodingWsOutMessage::CodingSessionState` 实际定义在 `src/web/coding_ws_handler.rs`（非 `workspace_ws_types.rs`）；补全 `completion_commit`/`handoff_summary_ref`/`execution_plan_status` 字段来源（P3/P4）并明确 `completion_commit` 取 `head_commit`；明确 diff scope 与 handoff 校验必须在 P5 的状态更新/锁释放之前执行；将 diff scope completion gate 纳入任务 4，复用 `validate_write_path` 并增加越界阻断测试。
>
> **v1.2 修订摘要（架构评审修复）：** 1) 新增任务 4B：抽取 `run_completion_gates` helper，强制 `handle_final_confirm` 与 `complete_attempt_after_final_rework` 共用同一完成门禁；2) `ProviderAdapter` 注入类型统一为 `Arc<dyn ProviderAdapter + Send + Sync>`；3) handoff provider run 必须包裹在 `tokio::task::spawn_blocking` 中；4) 新增 `AdapterRole::Handoff` exhaustive match 检查；5) `head_commit` 为 `None` 时返回 `completion_commit_missing`。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Coding 前生成内部 `WorkItemExecutionPlan`，默认展示但不阻塞；Work Item 完成前执行 diff scope 校验并运行额外 provider handoff run，越界 diff 或缺 handoff 都不允许完成或解锁依赖项。

**Architecture:** Execution plan 与 handoff 都存 Aria 内部数据。`WorkItemExecutionPlan` 作为 Coding prompt 的结构化来源，只有 `require_execution_plan_confirm=true` 时阻塞；diff scope gate 在 final confirm 临界路径内复用 `validate_write_path` 阻断越界改动；`WorkItemHandoff` 在 review/final confirm 前由额外 provider run 或 fake provider 摘要生成，并成为 Work Item 完成门禁。

**Tech Stack:** Rust 1.95.0、CodingAttemptStore、CodingWorkspaceEngine、Provider adapter、Serde JSON、Cargo integration tests、TDD。

---

## 前置交付摘要

执行本计划前确认 P5 已交付：

- `create_coding_attempt` 已检查依赖、handoff 可读性和 active lock。
- Coding attempt 已使用 `aria/issues/{issue_id}` branch 与 `.worktrees/aria-issues/{issue_id}` worktree。
- `handle_final_confirm()` 只有在 shared worktree clean gate 通过后才释放 active lock 并记录 completed Work Item；dirty 时保持锁并进入人工 gate。
- P3 已保存 provider 输出的 `VerificationPlan`，Work Item 通过 `verification_plan_ref` 关联它。

> **字段来源说明（v1.1 新增）：** 本计划写入的 `LifecycleWorkItemRecord` 字段及其上游来源如下，实现前必须确认这些字段已由上游落地并逐字对齐：
>
> - `execution_plan_status`：字段由 **P3/P4** 引入到 `LifecycleWorkItemRecord`；本计划任务 3 在 execution plan confirm/change-request 时更新它。
> - `handoff_summary_ref`：字段由 **P3/P4** 引入；P5 在 Coding 启动门禁中读取，本计划任务 4 在完成时写入。
> - `completion_commit`：字段由 **P3/P4** 引入。`CodingExecutionAttempt` 已有 `head_commit` 字段，本计划**约定 `completion_commit` 取自 attempt 的 `head_commit`**（即 final confirm 时 attempt 的 HEAD commit），而非新引入提交来源。`WorkItemHandoff.commit_sha` 亦应与该 `head_commit` 一致。
> - `verification_plan_ref`：字段由 **P3** 引入，指向 provider 输出的 `VerificationPlan`。本计划只读取并展示该计划，不按 kind 或当前仓库技术栈生成命令。

## 计划大小边界

本计划只做后端 execution plan、diff scope completion gate 与 handoff：

- 不修改 Product Workbench Work Item DAG UI。
- 不修改 Coding Prepare 前端展示。
- 不写 Playwright E2E。
- 不改变 P5 已建立的 shared worktree branch/path 规则。
- 不按 `WorkItemKind`、文件路径或当前仓库内容硬编码 `cargo`、`pnpm`、Vitest、Playwright 等目标项目验证命令。

如果需要前端展示字段，后端只扩展 DTO/WS payload；UI 渲染留给 P8。

> **拆分点预设（v1.1 新增）：** 本计划偏大（5 任务 / 多个后端文件 / 两个新模型 + 两条新路由 + 改动约 89KB 的 `coding_ws_handler.rs`）。为降低单次提交风险，**至少拆为两段提交**：
>
> - **段一「execution plan」（任务 1-3）**：`WorkItemExecutionPlan` 模型与 store、attempt 启动时生成、snapshot/WS 暴露、可配置 confirm 门禁。完成后独立提交并通过该段的最终验证。
> - **段二「handoff-diff-scope」（任务 4-5）**：`WorkItemHandoff` 模型与 store、diff scope completion gate、完成门禁、handoff 生成 helper。在段一合并基础上提交。
>
> 两段各自保持可编译、可测试、可独立 review。下方「提交」章节给出对应的两条提交命令。

## 文件结构

- Modify: `src/product/coding_models.rs`
  - 新增 `WorkItemExecutionPlan`、`WorkItemHandoff`、状态和输入结构。
- Modify: `src/product/coding_attempt_store.rs`
  - 增加 execution plan 与 handoff 存取 API。
- Modify: `src/product/coding_workspace_engine.rs`
  - Prepare/Coding prompt 注入 execution plan。
  - final confirm 前检查 diff scope 和 handoff。
  - 增加 handoff provider run 调用，解析输出并保存 `WorkItemHandoff`。
- Modify: `src/product/coding_workspace_engine.rs` 构造函数
  - 注入 `Arc<dyn ProviderAdapter + Send + Sync>`，用于 handoff provider run。
  - **Provider adapter 来源：** 复用 P3 在 `WebAppState` 中新增的 `provider_adapter` 字段，通过 `CodingWorkspaceEngine::new(..., provider_adapter, ...)` 传入。
- Create: `src/product/handoff_provider.rs`（或内联于 coding_workspace_engine.rs）
  - 定义 handoff provider 的 prompt template 与输出 schema。
  - 将 `AdapterOutput` 解析为 `WorkItemHandoff`。
- Modify: `src/cross_cutting/worktree.rs`
  - 必要时将禁止运行时路径判断提升为 `pub(crate)` 以复用既有安全规则，不另写路径安全逻辑。
- Modify: `src/web/types.rs`
  - `CodingAttemptSnapshotResponse` 包含 `work_item_execution_plan` 和 `work_item_handoff`。
- Modify: `src/web/workspace_ws_types.rs`
  - 若 WS payload 的 plan/handoff 子结构（如 DTO 字段类型）需新增，在此补充类型定义。
  - **（v1.1 勘误）`CodingWsOutMessage::CodingSessionState` 枚举与变体实际定义在 `src/web/coding_ws_handler.rs`（约 `:1976`），不在 `workspace_ws_types.rs`。为该变体新增 `work_item_execution_plan` / `work_item_handoff` 字段的改动请在 `coding_ws_handler.rs` 完成（见下一条）。**
- Modify: `src/web/coding_ws_handler.rs`
  - `CodingWsOutMessage::CodingSessionState`（约 `:1976`）新增 `work_item_execution_plan` 和 `work_item_handoff` 字段。
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
        verification_plan_ref: Some("verification_plan_work_item_0001".to_string()),
        verification_summary: Some("provider supplied required gate verify_backend_unit".to_string()),
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
    assert_eq!(
        snapshot["work_item_execution_plan"]["verification_plan_ref"],
        "verification_plan_work_item_0001"
    );
    assert_eq!(
        snapshot["work_item_execution_plan"]["verification_summary"],
        "provider supplied required gate verify_backend_unit"
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
- Verification plan from Work Item `verification_plan_ref`:
  - load the provider-supplied `VerificationPlan` saved by P3.
  - copy only `verification_plan_ref` and a concise `verification_summary` into `WorkItemExecutionPlan`.
  - do not synthesize `cargo`, `pnpm`, Vitest, Playwright, or any other command from Work Item kind.
  - if no valid `VerificationPlan` exists, block with `verification_plan_missing` or enter provider repair/manual gate.

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

- [ ] **步骤 6：编写 provider verification plan 防硬编码回归测试**

追加:

```rust
#[tokio::test]
async fn execution_plan_uses_provider_verification_plan_without_kind_defaults() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_backend_work_item_with_verification_plan(
        app.clone(),
        repo.path(),
        verification_plan_with_command("custom-verify --target backend-api"),
    )
    .await;

    let (_status, attempt) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    let (status, snapshot) = request_json(
        app,
        Method::GET,
        "/api/coding-attempts/coding_attempt_0001",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let plan = &snapshot["work_item_execution_plan"];
    assert_eq!(plan["verification_plan_ref"], "verification_plan_work_item_0001");
    assert!(!plan.to_string().contains("cargo test"));
    assert!(!plan.to_string().contains("pnpm"));
    assert!(plan.to_string().contains("custom-verify"));
}
```

运行:

```bash
cargo test --locked --test it_web execution_plan_uses_provider_verification_plan_without_kind_defaults
```

预期：通过后才能认为 P6 没有按 kind 注入默认命令。

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
- Modify: `src/cross_cutting/worktree.rs`
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
        tests_run: vec!["provider gate verify_session_api passed".to_string()],
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

Before setting attempt/work item completed in `handle_final_confirm()`, require:

- saved `WorkItemHandoff`
- required verification gate results for the Work Item's `VerificationPlan`
- shared worktree clean gate from P5 remains true at release time

After success, update `LifecycleWorkItemRecord.handoff_summary_ref` and `completion_commit` from the handoff/review request（`completion_commit` 取 attempt 的 `head_commit`，见前置交付摘要）。

> **🔴 执行顺序约束（v1.2 修订）：** P5 已让 `handle_final_confirm()` 承担「置 Completed + clean-gate 释放 active lock + 记录 last_completed」。本步骤插入的 handoff、verification gate 校验**必须位于 P5 的状态更新与锁释放之前**：即进入函数后先校验 diff scope、verification gates、handoff 和 shared worktree clean，缺失则提前返回阻断，之后才执行 P5 的 Completed 落库与锁释放。否则会出现「Work Item 已置 Completed、锁已释放，但 handoff 或 required verification 缺失」的不一致状态，且锁释放后同 Issue 下一个 Work Item 可能已抢占，无法回滚。

- [ ] **步骤 5A：编写 required verification gate 失败测试**

追加:

```rust
#[tokio::test]
async fn final_confirm_requires_required_verification_gate_result() {
    let root = tempdir().expect("root");
    let (store, attempt) =
        final_confirm_attempt_with_verification_plan(root.path(), "work_item_0001");
    store
        .save_work_item_handoff(&handoff_for_attempt(&attempt))
        .expect("save handoff");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let error = engine
        .handle_final_confirm("project_0001", "issue_0001", &attempt.id)
        .await
        .expect_err("missing required verification blocks completion");

    assert!(format!("{error}").contains("verification_gate_result_missing"));
}
```

实现要求：

- 从 Work Item `verification_plan_ref` 读取 `VerificationPlan.required_gates`。
- 从 attempt testing report / manual gate record / verification result store 读取执行结果。
- required gate 缺失、失败且未人工接受时，返回 `verification_gate_result_missing` 或 `verification_gate_failed`。
- manual accepted 必须记录操作者、时间、说明和关联 gate ID，供 handoff provider 输入消费。

## 任务 4B：抽取 completion gate helper 并覆盖 `complete_attempt_after_final_rework`

> **v1.2 新增任务：** 架构评审发现 `complete_attempt_after_final_rework` 直接置 `Completed` 并绕过 `handle_final_confirm` 的门禁。本任务抽取统一 `run_completion_gates` helper，要求 `handle_final_confirm` 与 `complete_attempt_after_final_rework` 共用同一套完成门禁。

**文件：**

- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **步骤 1：抽取 `run_completion_gates` helper**

在 `src/product/coding_workspace_engine.rs` 新增：

```rust
async fn run_completion_gates(
    &self,
    attempt: &CodingExecutionAttempt,
) -> Result<CompletionGateReport, CodingWorkspaceEngineError>
```

运行顺序（任一失败提前返回，不得修改任何状态）：

1. **diff-scope gate**：复用 `validate_write_path`，检查 attempt 的 changed files 是否全部落在 `LifecycleWorkItemRecord.exclusive_write_scopes` 内且未命中 `forbidden_write_scopes`。
2. **required verification gate result**：检查 `VerificationPlan.required_gates` 是否全部通过或被人工接受。
3. **handoff existence**：检查是否已保存 `WorkItemHandoff`。
4. **shared worktree clean gate**：调用 `ensure_issue_shared_worktree_clean(...)`，dirty 时返回错误。

额外约束：

- 若 `attempt.head_commit` 为 `None`，直接返回 `CodingWorkspaceEngineError::completion_commit_missing`（diff gate 无法确定基线）。
- helper 内部只做读取与校验，不修改 attempt、Work Item 或 shared worktree 状态。

- [ ] **步骤 2：`handle_final_confirm` 复用 helper**

在 `handle_final_confirm()` 函数最开头调用 `self.run_completion_gates(&attempt).await?`；只有成功后才继续执行 Completed 落库、`mark_issue_worktree_completed_item` 与 active lock 释放。

- [ ] **步骤 3：`complete_attempt_after_final_rework` 复用 helper**

在 `complete_attempt_after_final_rework()` 函数最开头调用 `self.run_completion_gates(&attempt).await?`；只有成功后才允许：

- 将 attempt/Work Item 置为 `Completed`。
- 调用 `mark_issue_worktree_completed_item(...)` 释放 active lock。
- 若 helper 失败（包括 diff scope、verification、handoff、dirty worktree 或 `head_commit` 缺失），不得置 Completed，不得释放锁。

- [ ] **步骤 4：新增回归测试**

追加到 `tests/it_product/product_coding_workspace_engine.rs`：

```rust
#[tokio::test]
async fn complete_attempt_after_final_rework_requires_handoff_and_diff_scope() {
    let root = tempdir().expect("root");
    let (store, attempt) =
        complete_attempt_after_final_rework_fixture(root.path(), "work_item_0001");
    // fixture 中已设置越界 changed files，且未保存 handoff
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let error = engine
        .complete_attempt_after_final_rework("project_0001", "issue_0001", &attempt.id)
        .await
        .expect_err("missing completion gates blocks auto-complete");

    assert!(
        format!("{error}").contains("work_item_handoff_missing")
            || format!("{error}").contains("work_item_diff_scope_violation")
            || format!("{error}").contains("completion_commit_missing")
    );
}
```

- [ ] **步骤 5：运行回归测试并确认失败/通过**

运行:

```bash
cargo test --locked --test it_product complete_attempt_after_final_rework_requires_handoff_and_diff_scope
```

预期：先失败（helper 尚未接入），接入 helper 后通过。

- [ ] **步骤 6：编写失败态 diff scope gate test（v1.1 阻塞修复）**

Append to `tests/it_product/product_coding_workspace_engine.rs`:

```rust
#[tokio::test]
async fn final_confirm_rejects_diff_outside_work_item_write_scope() {
    let root = tempdir().expect("root");
    let (store, attempt) =
        final_confirm_attempt_with_changed_files(root.path(), "work_item_0001", vec!["web/src/App.tsx"]);
    store
        .save_work_item_handoff(&handoff_for_attempt(&attempt))
        .expect("save handoff");
    store
        .set_work_item_write_scopes(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            vec!["src/product/**".to_string()],
            vec!["web/**".to_string()],
        )
        .expect("write scopes");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let error = engine
        .handle_final_confirm("project_0001", "issue_0001", &attempt.id)
        .await
        .expect_err("out-of-scope diff blocks completion");

    assert!(format!("{error}").contains("work_item_diff_scope_violation"));
}
```

预期：测试先失败，因为 `handle_final_confirm()` 尚未在完成前校验改动文件范围。

- [ ] **步骤 7：实现 diff scope completion gate（v1.1 阻塞修复）**

在 `handle_final_confirm()` 中，顺序必须是：

1. 读取当前 attempt 对应的 `LifecycleWorkItemRecord`。
2. 读取 attempt 的 changed files / diff files 清单；若当前代码只持久化 diff summary，需要在本步骤补一个内部 helper，从 review request、testing report 或 Git status 结果中得到相对路径列表，测试 helper 可直接提供该列表。
3. 对每个相对路径调用 `crate::cross_cutting::worktree::validate_write_path(worktree_root, &work_item.exclusive_write_scopes, path, true)`。
4. 若路径匹配任一 `work_item.forbidden_write_scopes`，返回 `work_item_diff_scope_violation`。
5. 任一路径不在允许范围或命中禁止范围时，必须在 verification/handoff 校验、Completed 落库、active lock 释放之前提前返回错误。
6. diff scope 校验通过后，校验 `VerificationPlan.required_gates` 的执行结果或 manual accepted gate。
7. required verification 校验通过后，继续执行 handoff 校验和 P5 的 clean-gate 完成/解锁逻辑。

若需要在 `cross_cutting/worktree.rs` 中复用禁止运行时路径判断，可将私有 `is_forbidden_runtime_path` 提升为 `pub(crate)`；不要另写一套路径安全规则。

运行:

```bash
cargo test --locked --test it_product final_confirm_rejects_diff_outside_work_item_write_scope
```

预期：测试通过，越界改动不会标记 Work Item completed，也不会释放同 Issue active lock。

- [ ] **步骤 8：运行 handoff and diff scope tests 并确认通过**

运行:

```bash
cargo test --locked --test it_product saves_and_loads_work_item_handoff
cargo test --locked --test it_product final_confirm_requires_work_item_handoff
cargo test --locked --test it_product final_confirm_requires_required_verification_gate_result
cargo test --locked --test it_product final_confirm_rejects_diff_outside_work_item_write_scope
```

预期：三条测试都通过。

## 任务 5：Generate Handoff From Extra Provider Run

**文件：**

- Modify: `src/product/coding_workspace_engine.rs`
- Create: `src/product/handoff_provider.rs`（若文件拆分；否则内联实现）
- Modify: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **步骤 1：构造支持 handoff provider run 的 engine**

修改 `CodingWorkspaceEngine::new` 签名（或新增 `with_provider` 构造器），注入 provider adapter：

```rust
pub fn new(
    store: CodingAttemptStore,
    git_service: GitWorkspaceService,
    provider: Arc<dyn ProviderAdapter + Send + Sync>,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
) -> Self
```

所有现有调用点（tests 与生产代码）需要同步更新；测试使用 `Arc::new(FakeProviderAdapter::handoff(summary))` 这类 mock。

- [ ] **步骤 2：编写失败态 handoff provider run test**

追加:

```rust
#[tokio::test]
async fn generates_handoff_from_extra_provider_run_before_final_confirm() {
    let root = tempdir().expect("root");
    let (store, attempt) = attempt_with_review_request_and_testing_report(root.path());
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let provider = Arc::new(FakeProviderAdapter::with_structured_output(
        serde_json::json!({
            "summary": "后端 API 已完成，前端可调用 /api/session",
            "files_changed": ["src/web/handlers.rs"],
            "diff_summary": "新增 session API",
            "tests_run": ["provider gate verify_session_api passed"],
            "test_result_summary": "全部通过",
            "api_or_contract_changes": ["GET /api/session"],
            "next_work_item_notes": ["前端处理 401"]
        }),
    ));
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), provider, tx);

    let handoff = engine
        .generate_work_item_handoff(&attempt)
        .await
        .expect("generate handoff");

    assert_eq!(handoff.work_item_id, "work_item_0001");
    assert!(handoff.summary.contains("/api/session"));
    assert!(!handoff.tests_run.is_empty());
    assert!(handoff.provider_run_ref.is_some());
}
```

- [ ] **步骤 3：运行 handoff generation test 并确认失败**

运行:

```bash
cargo test --locked --test it_product generates_handoff_from_extra_provider_run_before_final_confirm
```

预期：构造器签名、provider run 调用、`generate_work_item_handoff` 方法均缺失，编译失败。

- [ ] **步骤 4：实现 handoff provider run**

在 `src/product/handoff_provider.rs`（或 `coding_workspace_engine.rs` 内）实现：

1. `HandoffProviderInput`：聚合以下上下文
   - Work Item 目标与范围（`exclusive_write_scopes`、`forbidden_write_scopes`）
   - diff summary 与 changed files 清单
   - `VerificationPlan` execution result summary and manual gate decisions
   - review report / review request summary
   - commit/head（attempt.head_commit）
   - API 或契约变化摘要
2. 构造 `AdapterInput`：
   - `provider_type`：沿用当前 Coding Workspace 的 author provider（或专用 handoff provider，第一版复用 author provider）
   - `role`：`AdapterRole::Handoff`（若不存在则新增）或复用 `AdapterRole::Analyst`
   - `prompt`：包含上述输入的 prompt template
   - `output_schema`：定义 `WorkItemHandoff` 核心字段的 JSON schema
   - `worktree_path`：attempt worktree path
   - **v1.2 提醒：** 新增 `AdapterRole::Handoff` 后，必须全仓搜索对 `AdapterRole` 的 exhaustive `match`，为现有 match 增加新变体或补 `unknown`/wildcard arm，避免编译失败。
3. 调用 `provider.run(&input)`，得到 `AdapterOutput`。**必须**用 `tokio::task::spawn_blocking` 包裹同步 `provider.run(&input)` 调用，避免阻塞 tokio worker。
4. 解析 `structured_output` 为 `WorkItemHandoff`；若解析失败，返回 `work_item_handoff_provider_output_invalid`。
5. 保存 provider run record（复用 `provider_run_record_from_output`）到 `.aria` 目录，并将 ref 写入 `WorkItemHandoff.provider_run_ref`。

不要 consume next Work Item context budget while generating handoff.

- [ ] **步骤 5：在 final confirm 路径调用 handoff provider run**

在 `handle_final_confirm()` 中：

1. 若 handoff 尚未生成，调用 `engine.generate_work_item_handoff(&attempt).await`。
2. 将生成的 `WorkItemHandoff` 通过 `CodingAttemptStore::save_work_item_handoff` 持久化。
3. 更新 `LifecycleWorkItemRecord.handoff_summary_ref` 和 `completion_commit`。
4. 调用任务 4B 抽取的 `self.run_completion_gates(&attempt).await?`；只有通过后才继续 P5 的 Completed 落库与 active lock 释放。
5. 在 `complete_attempt_after_final_rework()` 中同样先保证 handoff 已生成，再调用 `run_completion_gates` 校验，成功后置 Completed 并释放锁。

- [ ] **步骤 6：运行 handoff generation test 并确认通过**

重新运行步骤 3 的命令。

预期：通过。

## 最终验证

> **v1.1 修正：** 原过滤名 `work_item_execution_plan` / `work_item_handoff` / `execution_plan` 作为 `cargo test` 子串与实际测试函数名不一致或匹配过宽。下面已改为能命中本计划全部新增用例、且子串语义明确的过滤器。

运行:

```bash
cargo test --locked --test it_product saves_and_loads_work_item_execution_plan
cargo test --locked --test it_product saves_and_loads_work_item_handoff
cargo test --locked --test it_product final_confirm_requires_work_item_handoff
cargo test --locked --test it_product final_confirm_requires_required_verification_gate_result
cargo test --locked --test it_product final_confirm_rejects_diff_outside_work_item_write_scope
cargo test --locked --test it_product generates_handoff_from_extra_provider_run_before_final_confirm
cargo test --locked --test it_web execution_plan_uses_provider_verification_plan_without_kind_defaults
cargo test --locked --test it_web coding_attempt_snapshot_includes_generated_work_item_execution_plan
cargo test --locked --test it_web coding_ws_blocks_coder_stage_when_execution_plan_requires_confirmation
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

预期:

- Execution plan tests pass.
- Handoff tests pass.
- Diff scope completion gate test passes.
- Formatting, clippy and check pass.

## Diff Scope 校验说明（v1.1 修复）

本计划在任务 4 步骤 6-8 中落地 diff scope completion gate。实现必须复用 `src/cross_cutting/worktree.rs` 的 `validate_write_path`，并在 handoff 校验、Completed 落库、active lock 释放之前阻断越界改动。该门禁已在本计划内处理。

## 提交

> v1.1 拆分为两段提交（见「计划大小边界」）。

段一「execution plan」（任务 1-3 完成后）:

```bash
git add src/product/coding_models.rs src/product/coding_attempt_store.rs src/product/coding_workspace_engine.rs src/web/types.rs src/web/handlers.rs src/web/coding_ws_handler.rs tests/it_product/product_coding_attempt_store.rs tests/it_web/web_coding_attempt_api.rs
git commit -m "feat: add work item execution plan and configurable confirm gate"
```

段二「handoff」（任务 4-5 完成后）:

```bash
git add src/product/coding_models.rs src/product/coding_attempt_store.rs src/product/coding_workspace_engine.rs src/product/handoff_provider.rs tests/it_product/product_coding_attempt_store.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: require work item handoff before completion"
```
