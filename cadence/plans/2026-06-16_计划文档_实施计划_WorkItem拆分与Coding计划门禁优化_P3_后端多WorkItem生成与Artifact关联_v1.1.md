# WorkItem 拆分 P3 后端多 WorkItem 生成与 Artifact 关联 Implementation Plan

> **文档版本：** v1.1
>
> **v1.1 修订摘要：** 依据设计评审对照真实源码修订：(1) 新增"任务 0：修正全部 legacy `create_work_item` 调用点"，列出全仓 12+ 处调用文件并建议 `CreateWorkItemInput` 实现 `Default` 以缩小改动面；(2) `GenerateWorkItemsResponse` 改为兼容方案——保留旧字段 `workspace_session`（单数，取主 session）并新增 `workspace_sessions`（复数），维持"不改前端"边界且不破坏 `web_lifecycle_api.rs:350` 的既有断言；(3) 新增显式步骤：从 `web_lifecycle_api.rs` 移植 `request_json` 并定义 `app_with_confirmed_story_and_design` 两个 helper 到新测试文件；(4) 新增计划拆分点说明与 P1/P2 强依赖提示。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 `generate_work_items` 从单 Work Item 创建升级为 Issue Work Item Set 创建，确保每个 Work Item 都拥有独立 workspace session 与 artifact versions，并把 P2 validator 接入生成期校验。

**Architecture:** 后端 handler 接收用户拆分选项，构造内部 `IssueWorkItemPlan` 和多个 `LifecycleWorkItemRecord` 草稿，通过 `WorkItemSplitValidator` 后再持久化。第一版保持 fake/provider 输出可控：不在本计划实现真实 provider 拆分 run，只建立可验证的后端生成契约和 artifact/session 关联。

**Tech Stack:** Rust 1.95.0、Axum、Serde JSON、LifecycleStore、Cargo integration tests、TDD。

---

## 前置交付摘要

执行本计划前确认 P2 已交付：

- `IssueWorkItemPlan`、`IssueWorkItemPlanOptions`、`IssueWorkItemDependencyEdge`、`WorkItemSplitFinding` 已在 `src/product/models.rs` 中定义。
- `WorkItemSplitValidator::validate()` 会返回 `WorkItemSplitValidationReport`，并覆盖 DAG、scope、预算、跨端、Integration/E2E 与 traceability 校验。
- P2 不创建真实 Work Item，因此本计划负责持久化和 HTTP response 契约。

> **🔴 强依赖提示：** 本计划强依赖 P1/P2 产物——`LifecycleWorkItemRecord` 的新增字段（P1）、`IssueWorkItemPlan` 与 `WorkItemSplitValidator`（P2）。执行前必须确认 P1、P2 已合并到当前分支，否则任务 1/3/4 引用的类型与校验器不存在，无法编译。

## 计划拆分点说明

本计划范围偏大（types + store + handler 多 item 生成 + validator 接入 + workspace context）。若单个 session 放不下，按以下拆分点分两段提交：

- **第一段（types + store 字段）**：任务 0（修正 legacy 调用点）、任务 1（HTTP types）、任务 2（`CreateWorkItemInput` 新字段持久化）。提交后保证全仓编译与既有测试通过。
- **第二段（handler 多 item 生成 + validator 接入）**：任务 3（deterministic split builder + workspace context）、任务 4（validator 前置校验与无半成品回归）。

## 计划大小边界

本计划只做 `generate_work_items` 生成链路：

- 不实现 Issue 共享 worktree。
- 不修改 `create_coding_attempt` 启动门禁。
- 不修改 Coding Workspace engine。
- 不修改前端。
- 不写 Playwright E2E。

如果实现需要改 `src/product/coding_workspace_engine.rs` 或 `web/**`，停止并留给 P5/P7。

## 文件结构

- Modify: `src/web/types.rs`
  - 扩展 `GenerateWorkItemsRequest`，新增拆分选项。
  - 扩展 `GenerateWorkItemsResponse`：**保留旧字段 `workspace_session`（单数，兼容）**，新增 `workspace_sessions`（复数）、`work_item_plan` 与 `validator_findings`。
  - 扩展 `LifecycleWorkItemDto`，透出 P1/P2 新增字段。
- Modify: `src/web/handlers.rs`
  - `generate_work_items` 创建多个 Work Item。
  - 构造 `IssueWorkItemPlan` 并调用 `WorkItemSplitValidator`。
  - 返回每个 Work Item 对应的 workspace session（复数），同时把主 session 写入旧的单数字段。
- Modify: `src/product/lifecycle_store.rs`
  - 扩展 `CreateWorkItemInput` 支持 P1 新字段。
  - 增加批量创建辅助函数或保持 handler 顺序创建但保证失败前不产生半成品。
- Modify: `src/web/workspace_context.rs`
  - Work Item workspace system context 纳入 kind、依赖、写入范围、预算和验证命令摘要。
  - **同时修正本文件中的 legacy `create_work_item` 调用点（见任务 0）。**
- Modify: `tests/it_web.rs`
  - 引入 `web_work_item_generation`。
- Create: `tests/it_web/web_work_item_generation.rs`
  - 覆盖多 Work Item 生成、validator 拦截和 session/artifact 关联。
- Modify: `tests/it_product/product_lifecycle_store.rs`
  - 覆盖 `CreateWorkItemInput` 新字段持久化。
- Modify（任务 0，legacy 调用点）：`src/web/handlers.rs`、`src/web/workspace_context.rs`、`src/web/test_controls.rs`、`src/product/coding_evaluation_context.rs`、`src/product/lifecycle_store.rs`（self-test）、`tests/it_web/web_coding_ws_handler.rs`、`tests/it_web/web_coding_attempt_api.rs`、`tests/it_product/product_coding_workspace_engine.rs`、`tests/it_product/product_lifecycle_store.rs`
  - 为所有 legacy `create_work_item(CreateWorkItemInput{...})` 调用点补默认字段值（详见任务 0）。

## 任务 0：修正全部 legacy `create_work_item` 调用点

> **🔴 阻塞前置：** 给 `CreateWorkItemInput` 增加必填字段后，全仓约 12+ 处既有 `create_work_item(CreateWorkItemInput{...})` 调用点会编译失败。本任务负责把它们全部补齐，必须先于（或紧随）任务 2 的字段扩展完成，否则全仓无法编译。

**降风险建议（强烈推荐）：** 为 `CreateWorkItemInput` 新增字段实现 `Default`（或对整个结构体 `#[derive(Default)]`，注意 `WorkItemKind` 需有 `Default`），legacy 调用点即可用 `..Default::default()` 收尾，把改动面从"每处补 9 个字段"缩小为"每处补一行"。

**已确认需要同步修改的调用点文件清单（源码 + 测试）：**

- `src/web/handlers.rs:524`
- `src/web/workspace_context.rs:779`
- `src/web/test_controls.rs:484`
- `src/product/coding_evaluation_context.rs:641 / 758 / 811`（3 处）
- `src/product/lifecycle_store.rs:1329`（store 内 self-test）
- `tests/it_web/web_coding_ws_handler.rs:2369 / 2416 / 2508 / 2573 / 2644 / 2703 / 2804 / 3981`（8 处）
- `tests/it_web/web_coding_attempt_api.rs:702`
- `tests/it_product/product_coding_workspace_engine.rs:4813 / 5001`（2 处）
- `tests/it_product/product_lifecycle_store.rs:64 / 83`（2 处，旧用例）

- [ ] **步骤 1：先确认调用点全集**

运行（确认无遗漏后再动手）：

```bash
grep -rn "create_work_item(CreateWorkItemInput\|create_work_item(crate::product::lifecycle_store::CreateWorkItemInput" src/ tests/
```

- [ ] **步骤 2：实现 `Default` 并逐点补默认值**

为 `CreateWorkItemInput` 新字段实现 `Default`，对上述每个 legacy 调用点追加：

```rust
work_item_set_id: None,
kind: WorkItemKind::Other,
sequence_hint: None,
depends_on: Vec::new(),
exclusive_write_scopes: Vec::new(),
forbidden_write_scopes: Vec::new(),
context_budget: WorkItemContextBudget::default(),
required_handoff_from: Vec::new(),
require_execution_plan_confirm: false,
```

或在实现 `Default` 后用 `..Default::default()` 收尾。

- [ ] **步骤 3：全仓编译确认**

```bash
cargo check --locked --all-targets
```

预期：所有 legacy 调用点编译通过。



## 任务 1：Extend HTTP Types For Split Options And Work Item DTO

**文件：**

- Modify: `src/web/types.rs`
- Modify: `tests/it_web.rs`
- Create: `tests/it_web/web_work_item_generation.rs`

- [ ] **步骤 1：编写失败态 response contract test**

> **🔴 测试 helper 前置：** 本任务的测试用到 `request_json(...)` 与 `app_with_confirmed_story_and_design().await` 两个 helper，但全 `tests/` 中**不存在** `app_with_confirmed_story_and_design`，`request_json` 也分散在各测试文件中各自定义。必须在新测试文件里显式定义这两个 helper：
>
> - 从 `tests/it_web/web_lifecycle_api.rs:1159` 移植 `request_json`，签名返回 `(StatusCode, Value)`。
> - 参考 `web_lifecycle_api.rs` 中"创建 project/issue + 生成并 confirm story/design spec"的既有流程，封装出 `app_with_confirmed_story_and_design()`，返回 `(axum::Router, TempRepo)`，内部完成：建仓、建 project、建 issue、生成并 confirm `story_spec_0001` 与 `design_spec_0001`。

创建 `tests/it_web/web_work_item_generation.rs`:

```rust
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn generate_work_items_accepts_split_options_and_returns_plan_metadata() {
    let (app, _repo) = app_with_confirmed_story_and_design().await;

    let (status, response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "登录会话拆分实现",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["work_item_plan"]["status"], "draft");
    assert_eq!(response["work_item_plan"]["options"]["include_integration_tests"], true);
    assert_eq!(response["work_items"].as_array().unwrap().len(), 3);
    assert_eq!(response["workspace_sessions"].as_array().unwrap().len(), 3);
    // 兼容断言：旧的单数字段保留，指向主 session（首个 work item）。
    assert_eq!(response["workspace_session"]["entity_id"], "work_item_0001");
    assert!(response["validator_findings"].as_array().unwrap().is_empty());
}

// 移植自 tests/it_web/web_lifecycle_api.rs:1159，返回 (StatusCode, Value)。
async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json")
    };
    (status, value)
}

// 新增 helper：建仓 + project + issue，并生成并 confirm story/design spec。
// 参考 web_lifecycle_api.rs 中既有的生成/confirm 流程拼装。
async fn app_with_confirmed_story_and_design() -> (axum::Router, tempfile::TempDir) {
    // 1. tempdir + WebRuntime + build_web_router
    // 2. POST 创建 project_0001 / issue_0001
    // 3. 生成并 confirm story_spec_0001、design_spec_0001
    // 返回 (app, repo_tempdir)
    todo!("移植 web_lifecycle_api.rs 的 confirmed story/design 流程")
}
```

在 `tests/it_web.rs`, add:

```rust
#[path = "it_web/web_work_item_generation.rs"]
mod web_work_item_generation;
```

- [ ] **步骤 2：运行 contract test 并确认失败**

运行:

```bash
cargo test --locked --test it_web generate_work_items_accepts_split_options_and_returns_plan_metadata
```

预期：反序列化或断言失败，因为 current response has one `workspace_session` and no split fields.

- [ ] **步骤 3：扩展 web types**

在 `src/web/types.rs`:

```rust
pub struct GenerateWorkItemsRequest {
    pub title: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub include_integration_tests: Option<bool>,
    pub include_e2e_tests: Option<bool>,
    pub force_frontend_backend_split: Option<bool>,
    pub require_execution_plan_confirm: Option<bool>,
    pub author_provider: Option<String>,
    pub reviewer_provider: Option<String>,
    pub review_rounds: Option<u32>,
    pub superpowers_enabled: Option<bool>,
    pub openspec_enabled: Option<bool>,
}
```

替换 `GenerateWorkItemsResponse` with（**兼容方案：保留旧的单数 `workspace_session` 字段，新增复数 `workspace_sessions`**）:

```rust
pub struct GenerateWorkItemsResponse {
    pub work_items: Vec<LifecycleWorkItemDto>,
    /// 兼容字段：保留旧的单数 session，取首个/主 session。
    /// 维持"不改前端"边界，且不破坏 web_lifecycle_api.rs:350 的既有断言。
    pub workspace_session: WorkspaceSessionDto,
    /// 新增：每个 Work Item 对应一个 session。
    pub workspace_sessions: Vec<WorkspaceSessionDto>,
    pub work_item_plan: IssueWorkItemPlan,
    pub validator_findings: Vec<WorkItemSplitFinding>,
}
```

> **🔴 兼容性约束：** 旧字段 `workspace_session` 不得删除或改名。handler 构造响应时，将首个（主）Work Item 的 session 同时写入 `workspace_session`（单数）与 `workspace_sessions[0]`。本计划维持"不改前端"边界，既有后端测试 `tests/it_web/web_lifecycle_api.rs:350` 的 `workspace_session` 断言必须继续通过。

添加 the necessary imports from `crate::product::models`.

- [ ] **步骤 4：运行 contract test 并确认编译错误收敛到预期位置**

Run the command from Step 2.

预期：编译错误现在指向 `handlers.rs` response construction and TypeScript is not involved in this backend test.

## 任务 2：Persist Multiple Work Items And Workspace Sessions

**文件：**

- Modify: `src/product/lifecycle_store.rs`
- Modify: `tests/it_product/product_lifecycle_store.rs`

- [ ] **步骤 1：编写失败态 store test**

Append to `tests/it_product/product_lifecycle_store.rs`:

```rust
#[test]
fn create_work_item_persists_split_fields() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let work_item = store
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "后端 API".to_string(),
            work_item_set_id: Some("work_item_set_0001".to_string()),
            kind: WorkItemKind::Backend,
            sequence_hint: Some(10),
            depends_on: Vec::new(),
            exclusive_write_scopes: vec!["src/product/**".to_string()],
            forbidden_write_scopes: vec!["web/**".to_string()],
            context_budget: WorkItemContextBudget::default(),
            required_handoff_from: Vec::new(),
            require_execution_plan_confirm: false,
        })
        .expect("work item");

    assert_eq!(work_item.work_item_set_id.as_deref(), Some("work_item_set_0001"));
    assert_eq!(work_item.kind, WorkItemKind::Backend);
    assert_eq!(work_item.exclusive_write_scopes, vec!["src/product/**"]);
}
```

- [ ] **步骤 2：运行 store test 并确认失败**

运行:

```bash
cargo test --locked --test it_product create_work_item_persists_split_fields
```

预期：在 `CreateWorkItemInput` has the split fields.

- [ ] **步骤 3：扩展 `CreateWorkItemInput` and creation defaults**

添加 the new fields to `CreateWorkItemInput`. When legacy callers do not have split metadata, update those call sites to pass:

```rust
work_item_set_id: None,
kind: WorkItemKind::Other,
sequence_hint: None,
depends_on: Vec::new(),
exclusive_write_scopes: Vec::new(),
forbidden_write_scopes: Vec::new(),
context_budget: WorkItemContextBudget::default(),
required_handoff_from: Vec::new(),
require_execution_plan_confirm: false,
```

`create_work_item()` must copy these fields into `LifecycleWorkItemRecord` and initialize:

```rust
execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
handoff_summary_ref: None,
completion_commit: None,
completion_diff_summary_ref: None,
```

- [ ] **步骤 4：运行 store test 并确认通过**

重新运行步骤 2 的命令。

预期：测试通过。

## 任务 3：Implement Deterministic First-Version Split Generation

**文件：**

- Modify: `src/web/handlers.rs`
- Modify: `src/web/workspace_context.rs`
- Modify: `tests/it_web/web_work_item_generation.rs`

- [ ] **步骤 1：编写失败态 multi Work Item generation test**

Extend the web test to assert deterministic titles and dependencies:

```rust
#[tokio::test]
async fn generate_work_items_creates_backend_frontend_and_integration_items_with_sessions() {
    let (app, _repo) = app_with_confirmed_story_and_design().await;

    let (status, response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "登录会话拆分实现",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let items = response["work_items"].as_array().unwrap();
    assert_eq!(items[0]["kind"], "backend");
    assert_eq!(items[1]["kind"], "frontend");
    assert_eq!(items[2]["kind"], "integration");
    assert_eq!(items[1]["depends_on"], json!(["work_item_0001"]));
    assert_eq!(items[2]["depends_on"], json!(["work_item_0001", "work_item_0002"]));
    assert_eq!(response["workspace_sessions"][0]["entity_id"], "work_item_0001");
    assert_eq!(response["workspace_sessions"][1]["entity_id"], "work_item_0002");
    assert_eq!(response["workspace_sessions"][2]["entity_id"], "work_item_0003");
}
```

- [ ] **步骤 2：运行 test 并确认失败**

运行:

```bash
cargo test --locked --test it_web generate_work_items_creates_backend_frontend_and_integration_items_with_sessions
```

预期：当前 handler returns one work item and one session.

- [ ] **步骤 3：实现 deterministic split builder**

Inside `src/web/handlers.rs`, add a helper near `generate_work_items`:

```rust
fn build_initial_work_item_specs(
    request: &GenerateWorkItemsRequest,
) -> Vec<InitialWorkItemSpec> {
    let mut specs = vec![
        InitialWorkItemSpec::backend(&request.title),
        InitialWorkItemSpec::frontend(&request.title, vec!["work_item_0001".to_string()]),
    ];
    if request.include_integration_tests.unwrap_or(false) {
        specs.push(InitialWorkItemSpec::integration(
            &request.title,
            vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
        ));
    }
    if request.include_e2e_tests.unwrap_or(false) {
        specs.push(InitialWorkItemSpec::e2e(
            &request.title,
            vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
        ));
    }
    specs
}
```

使用 exact write scopes:

- Backend: `src/**`, forbidden `web/**`
- Frontend: `web/src/**`, forbidden `src/**`
- Integration: `src/**`, `web/src/**`
- E2E: `web/e2e/**`

在 creating records, construct `IssueWorkItemPlan`, run `WorkItemSplitValidator::validate()`, and return validation errors with code `work_item_split_invalid` before returning success.

- [ ] **步骤 4：Include split context in workspace messages**

Update `ensure_workspace_context_message()` or the Work Item context builder in `src/web/workspace_context.rs` so each Work Item session mentions:

- Work Item kind.
- Allowed write scopes.
- Forbidden write scopes.
- Dependencies.
- Required handoff sources.
- Superpowers/TDD/verification requirements.

- [ ] **步骤 5：运行 generation tests 并确认通过**

运行:

```bash
cargo test --locked --test it_web generate_work_items_accepts_split_options_and_returns_plan_metadata
cargo test --locked --test it_web generate_work_items_creates_backend_frontend_and_integration_items_with_sessions
```

预期：两条测试都通过。

## 任务 4：Validator Blocks Invalid Generation Without Half-Created Work Items

**文件：**

- Modify: `src/web/handlers.rs`
- Modify: `tests/it_web/web_work_item_generation.rs`

- [ ] **步骤 1：抽取 candidate validation helper**

在 `src/web/handlers.rs`, extract candidate validation before persistence into a helper that can be unit-tested without adding test-only HTTP request fields:

```rust
fn validate_work_item_generation_candidates(
    plan: &IssueWorkItemPlan,
    candidates: &[LifecycleWorkItemRecord],
) -> Result<(), ApiError> {
    let report = WorkItemSplitValidator::validate(plan, candidates);
    if report.has_errors() {
        return Err(ApiError::validation_with_details(
            "work_item_split_invalid",
            "work item split plan did not pass validation",
            json!({ "validator_findings": report.findings }),
        ));
    }
    Ok(())
}
```

Call this helper before the first `lifecycle.create_work_item(...)` call.

- [ ] **步骤 2：编写失败态 invalid candidate unit test**

追加:

```rust
#[test]
fn validate_work_item_generation_candidates_rejects_required_e2e_when_e2e_item_is_missing() {
    let plan = issue_work_item_plan_for_test(
        vec!["work_item_0001", "work_item_0002"],
        IssueWorkItemPlanOptions {
            include_integration_tests: false,
            include_e2e_tests: true,
            force_frontend_backend_split: true,
            require_execution_plan_confirm: false,
        },
    );
    let candidates = vec![
        candidate_work_item("work_item_0001", WorkItemKind::Backend, vec![], vec!["src/**"]),
        candidate_work_item(
            "work_item_0002",
            WorkItemKind::Frontend,
            vec!["work_item_0001"],
            vec!["web/src/**"],
        ),
    ];

    let error = validate_work_item_generation_candidates(&plan, &candidates)
        .expect_err("missing e2e item should be rejected");

    assert_eq!(error.code(), "work_item_split_invalid");
}
```

- [ ] **步骤 3：运行 invalid candidate test 并确认失败**

运行:

```bash
cargo test --locked --test it_web validate_work_item_generation_candidates_rejects_required_e2e_when_e2e_item_is_missing
```

预期：失败，直到 validation happens before persistence.

- [ ] **步骤 4：校验 before persistence**

Build candidate `LifecycleWorkItemRecord` values in memory with predicted sequential IDs before writing JSON. Validate candidates. Persist only after `report.has_errors()` is false.

- [ ] **步骤 5：添加 HTTP no-half-created regression**

追加:

```rust
#[tokio::test]
async fn generate_work_items_rejects_invalid_confirmed_refs_without_half_created_records() {
    let (app, _repo) = app_with_confirmed_story_and_design().await;

    let (status, body) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "登录会话拆分实现",
            "story_spec_ids": ["story_spec_9999"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "story_spec_not_found");

    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lifecycle["work_items"].as_array().unwrap().len(), 0);
}
```

- [ ] **步骤 6：运行 invalid candidate and no-half-created tests 并确认通过**

运行:

```bash
cargo test --locked --test it_web validate_work_item_generation_candidates_rejects_required_e2e_when_e2e_item_is_missing
cargo test --locked --test it_web generate_work_items_rejects_invalid_confirmed_refs_without_half_created_records
```

预期：invalid candidate 返回 `work_item_split_invalid`; invalid HTTP refs return `400`, and lifecycle still has zero Work Items.

## 最终验证

运行:

```bash
cargo test --locked --test it_web generate_work_items
cargo test --locked --test it_product lifecycle_store
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

预期:

- Work Item generation tests pass.
- Lifecycle store tests pass.
- Formatting, clippy and check pass.

## 提交

```bash
git add src/web/types.rs src/web/handlers.rs src/product/lifecycle_store.rs src/web/workspace_context.rs tests/it_web.rs tests/it_web/web_work_item_generation.rs tests/it_product/product_lifecycle_store.rs
# 任务 0 修正的 legacy create_work_item 调用点
git add src/web/test_controls.rs src/product/coding_evaluation_context.rs tests/it_web/web_coding_ws_handler.rs tests/it_web/web_coding_attempt_api.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: generate split work items"
```
