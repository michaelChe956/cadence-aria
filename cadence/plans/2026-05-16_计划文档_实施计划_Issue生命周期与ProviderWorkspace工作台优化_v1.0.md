# Issue 生命周期与 Provider Workspace 工作台优化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the new Issue lifecycle workbench where Issue, Story Spec, Design Spec, and Work Item are first-class cards, and all provider-driven authoring, review, confirmation, and work item execution happen inside Provider Workspace dialogs.

**Architecture:** Add product lifecycle stores for Story Spec, Design Spec, Work Item, versions, workspace sessions, and provider review rounds under the existing `src/product` layer. Expose lifecycle and workspace-session APIs from `src/web`, then replace the current stage-based project workbench UI with a four-column `IssueLifecycleWorkbench` and `ProviderWorkspaceDialog`. Reuse existing runtime/provider/artifact/checkpoint/SSE capabilities through adapters instead of keeping the old execution workbench as the main UI.

**Tech Stack:** Rust 2024, Axum 0.8, serde JSON stores, Tokio, chrono, React 19, TypeScript, Vite, Vitest, Testing Library, Playwright, Tailwind CSS, lucide-react.

---

## Scope Check

This plan implements one integrated product slice. Backend lifecycle stores, lifecycle API, provider workspace sessions, and frontend four-column UI must ship together because Story, Design, and Work Item cards are persisted entities and the UI cannot be correct without the new API. The old execution workbench remains available only as compatibility surface for existing tests and runtime adapter work.

## File Structure

### Backend Product Layer

- Modify `src/product/models.rs`: add lifecycle enums and records for specs, versions, workspace sessions, provider review rounds, and project provider defaults.
- Modify `src/product/app_paths.rs`: add path helpers for issue lifecycle subdirectories.
- Create `src/product/lifecycle_store.rs`: persist Story Spec, Design Spec, Work Item, Spec Version, Workspace Session, Review Round, and Project defaults.
- Modify `src/product/issue_store.rs`: require `repo_id` when creating product issues for the new lifecycle path.
- Modify `src/product/mod.rs`: export `lifecycle_store`.

### Backend Web Layer

- Modify `src/web/types.rs`: add lifecycle DTOs and workspace session request/response types.
- Modify `src/web/handlers.rs`: add lifecycle handlers and workspace-session handlers.
- Modify `src/web/app.rs`: register lifecycle and workspace-session routes.
- Modify `src/web/events.rs`: add lifecycle event names for workspace session updates.

### Frontend API And State

- Modify `web/src/api/types.ts`: add lifecycle DTOs and workspace session DTOs.
- Modify `web/src/api/client.ts`: add lifecycle and workspace session API functions.
- Create `web/src/state/lifecycle-workbench-store.ts`: pure helpers for grouping, focus filtering, and stage unlocking.
- Create `web/src/state/lifecycle-workbench-store.test.ts`: unit tests for grouping and dependency gates.

### Frontend Components

- Create `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`: new four-column main workbench.
- Create `web/src/components/lifecycle/LifecycleColumn.tsx`: reusable column with stable card layout.
- Create `web/src/components/lifecycle/LifecycleCard.tsx`: issue, story, design, work item card renderer.
- Create `web/src/components/lifecycle/CreateLifecycleIssueDialog.tsx`: issue creation dialog that requires Repository.
- Create `web/src/components/workspace/ProviderWorkspaceDialog.tsx`: unified provider workspace dialog.
- Create `web/src/components/workspace/WorkspaceFlowRail.tsx`: dialog flow rail.
- Create `web/src/components/workspace/WorkspaceConversation.tsx`: provider conversation pane.
- Create `web/src/components/workspace/WorkspaceArtifactPane.tsx`: versions, review, and confirmation pane.
- Modify `web/src/components/project/ProjectManagementWorkbench.tsx`: replace main implementation with a compatibility wrapper that renders `IssueLifecycleWorkbench`.
- Modify `web/src/app-shell.tsx`: remove the old execution workbench as the default post-start target and route users back to lifecycle UI.

### Tests

- Create `tests/product_lifecycle_store.rs`.
- Create `tests/web_lifecycle_api.rs`.
- Create `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`.
- Create `web/src/components/workspace/ProviderWorkspaceDialog.test.tsx`.
- Create `web/e2e/issue-lifecycle-workspace.spec.ts`.

## Task 1: Product Lifecycle Models And Store

**Files:**
- Modify: `src/product/models.rs`
- Modify: `src/product/app_paths.rs`
- Create: `src/product/lifecycle_store.rs`
- Modify: `src/product/mod.rs`
- Test: `tests/product_lifecycle_store.rs`

- [ ] **Step 1: Write failing product lifecycle store tests**

Create `tests/product_lifecycle_store.rs`:

```rust
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::{
    AppendSpecVersionInput, CreateDesignSpecInput, CreateProjectProviderDefaultsInput,
    CreateStorySpecInput, CreateWorkItemInput, CreateWorkspaceSessionInput, LifecycleStore,
};
use cadence_aria::product::models::{
    DesignKind, LifecycleConfirmationStatus, ProviderName, WorkspaceSessionStatus,
    WorkspaceType,
};
use tempfile::tempdir;

#[test]
fn creates_story_design_work_item_and_versions_with_source_links() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "登录用户看到会话过期提示".to_string(),
        })
        .expect("story");
    assert_eq!(story.id, "story_spec_0001");
    assert_eq!(story.confirmation_status, LifecycleConfirmationStatus::Draft);

    let story_version = store
        .append_version(AppendSpecVersionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id.clone(),
            markdown: "# Story\n\n会话过期提示。".to_string(),
            provider_run_refs: vec!["run_story_0001".to_string()],
            review_refs: vec!["review_round_0001".to_string()],
            confirmed_by: None,
        })
        .expect("story version");
    assert_eq!(story_version.version, 1);

    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            story_spec_ids: vec![story.id.clone()],
            design_kind: DesignKind::Frontend,
            title: "会话过期前端设计".to_string(),
        })
        .expect("design");
    assert_eq!(design.story_spec_ids, vec![story.id.clone()]);

    let work_item = store
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec![story.id.clone()],
            design_spec_ids: vec![design.id.clone()],
            title: "实现会话过期提示".to_string(),
        })
        .expect("work item");
    assert_eq!(work_item.story_spec_ids, vec![story.id]);
    assert_eq!(work_item.design_spec_ids, vec![design.id]);
    assert_eq!(work_item.plan_status.as_str(), "not_started");
}

#[test]
fn persists_workspace_session_and_project_provider_defaults() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let defaults = store
        .upsert_project_provider_defaults(CreateProjectProviderDefaultsInput {
            project_id: "project_0001".to_string(),
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("defaults");
    assert_eq!(defaults.review_rounds, 2);

    let session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: false,
        })
        .expect("session");

    assert_eq!(session.id, "workspace_session_0001");
    assert_eq!(session.status, WorkspaceSessionStatus::Open);
    assert_eq!(store.list_workspace_sessions("project_0001", "issue_0001").unwrap().len(), 1);
}
```

- [ ] **Step 2: Run the failing test**

Run:

```bash
cargo test --locked --test product_lifecycle_store
```

Expected: FAIL with unresolved imports for `lifecycle_store`, `DesignKind`, `LifecycleConfirmationStatus`, `ProviderName`, and `WorkspaceType`.

- [ ] **Step 3: Add lifecycle models**

Modify `src/product/models.rs` by appending:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleConfirmationStatus {
    Draft,
    InReview,
    Confirmed,
    ChangeRequested,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignKind {
    Frontend,
    Backend,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderName {
    ClaudeCode,
    Codex,
    Fake,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceType {
    Story,
    Design,
    WorkItem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSessionStatus {
    Open,
    Running,
    WaitingForHuman,
    Confirmed,
    ChangeRequested,
    BlockedProviderUnavailable,
    Terminated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanStatus {
    NotStarted,
    Draft,
    Confirmed,
    ChangeRequested,
}

impl WorkItemPlanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkItemPlanStatus::NotStarted => "not_started",
            WorkItemPlanStatus::Draft => "draft",
            WorkItemPlanStatus::Confirmed => "confirmed",
            WorkItemPlanStatus::ChangeRequested => "change_requested",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StorySpecRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub title: String,
    pub current_version: Option<u32>,
    pub confirmation_status: LifecycleConfirmationStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DesignSpecRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_kind: DesignKind,
    pub title: String,
    pub current_version: Option<u32>,
    pub confirmation_status: LifecycleConfirmationStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LifecycleWorkItemRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub title: String,
    pub plan_status: WorkItemPlanStatus,
    pub execution_status: WorkItemStatus,
    pub worktree_path: Option<PathBuf>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SpecVersionRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub version: u32,
    pub markdown: String,
    pub provider_run_refs: Vec<String>,
    pub review_refs: Vec<String>,
    pub confirmed_by: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub workspace_type: WorkspaceType,
    pub status: WorkspaceSessionStatus,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
    pub messages: Vec<WorkspaceMessageRecord>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceMessageRecord {
    pub role: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderReviewRoundRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub session_id: String,
    pub round_index: u32,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_result: String,
    pub revision_result: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectProviderDefaultsRecord {
    pub project_id: String,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
    pub updated_at: String,
}
```

- [ ] **Step 4: Add lifecycle paths and store**

Modify `src/product/app_paths.rs`:

```rust
impl ProductAppPaths {
    pub fn issue_lifecycle_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.issue_root(project_id, issue_id)
    }

    pub fn project_provider_defaults_path(&self, project_id: &str) -> PathBuf {
        self.project_root(project_id).join("provider-defaults.json")
    }
}
```

Create `src/product/lifecycle_store.rs` with public inputs and methods used by the tests. Use `write_json`, `read_json`, `validate_relative_id`, `next_sequential_id`, and `chrono::Utc` following the style in `project_store.rs` and `issue_store.rs`.

Required method signatures:

```rust
pub struct LifecycleStore {
    paths: ProductAppPaths,
}

impl LifecycleStore {
    pub fn new(paths: ProductAppPaths) -> Self;
    pub fn create_story_spec(&self, input: CreateStorySpecInput) -> Result<StorySpecRecord, ProductStoreError>;
    pub fn list_story_specs(&self, project_id: &str, issue_id: &str) -> Result<Vec<StorySpecRecord>, ProductStoreError>;
    pub fn create_design_spec(&self, input: CreateDesignSpecInput) -> Result<DesignSpecRecord, ProductStoreError>;
    pub fn list_design_specs(&self, project_id: &str, issue_id: &str) -> Result<Vec<DesignSpecRecord>, ProductStoreError>;
    pub fn create_work_item(&self, input: CreateWorkItemInput) -> Result<LifecycleWorkItemRecord, ProductStoreError>;
    pub fn list_work_items(&self, project_id: &str, issue_id: &str) -> Result<Vec<LifecycleWorkItemRecord>, ProductStoreError>;
    pub fn append_version(&self, input: AppendSpecVersionInput) -> Result<SpecVersionRecord, ProductStoreError>;
    pub fn list_versions(&self, project_id: &str, issue_id: &str, entity_id: &str) -> Result<Vec<SpecVersionRecord>, ProductStoreError>;
    pub fn create_workspace_session(&self, input: CreateWorkspaceSessionInput) -> Result<WorkspaceSessionRecord, ProductStoreError>;
    pub fn list_workspace_sessions(&self, project_id: &str, issue_id: &str) -> Result<Vec<WorkspaceSessionRecord>, ProductStoreError>;
    pub fn upsert_project_provider_defaults(&self, input: CreateProjectProviderDefaultsInput) -> Result<ProjectProviderDefaultsRecord, ProductStoreError>;
}
```

Path conventions:

```rust
fn story_specs_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
    self.paths.issue_root(project_id, issue_id).join("story-specs")
}

fn design_specs_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
    self.paths.issue_root(project_id, issue_id).join("design-specs")
}

fn work_items_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
    self.paths.issue_root(project_id, issue_id).join("work-items")
}

fn versions_root(&self, project_id: &str, issue_id: &str, entity_id: &str) -> PathBuf {
    self.paths.issue_root(project_id, issue_id).join("versions").join(entity_id)
}

fn workspace_sessions_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
    self.paths.issue_root(project_id, issue_id).join("workspace-sessions")
}
```

Modify `src/product/mod.rs`:

```rust
pub mod lifecycle_store;
```

- [ ] **Step 5: Run store tests**

Run:

```bash
cargo test --locked --test product_lifecycle_store
```

Expected: PASS.

- [ ] **Step 6: Commit Task 1**

Run:

```bash
git add src/product/models.rs src/product/app_paths.rs src/product/lifecycle_store.rs src/product/mod.rs tests/product_lifecycle_store.rs
git commit -m "feat: add product lifecycle store"
```

## Task 2: Lifecycle API And Repository-Required Issue Creation

**Files:**
- Modify: `src/web/types.rs`
- Modify: `src/web/handlers.rs`
- Modify: `src/web/app.rs`
- Modify: `src/product/issue_store.rs`
- Test: `tests/web_lifecycle_api.rs`

- [ ] **Step 1: Write failing lifecycle API tests**

Create `tests/web_lifecycle_api.rs`:

```rust
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{json, Value};
use std::process::Command;
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn issue_creation_requires_repository_and_lifecycle_lists_cards() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    ).await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    ).await;

    let (status, missing_repo) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"Missing repo","description":null}),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(missing_repo["code"], "repository_required");

    let (status, issue) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({
            "title":"登录会话过期",
            "description":"需要结合前端代码提示用户重新登录",
            "repository_id":"repository_0001"
        }),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(issue["repo_id"], "repository_0001");

    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lifecycle["issue"]["issue_id"], "issue_0001");
    assert_eq!(lifecycle["story_specs"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["design_specs"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["work_items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn generate_endpoints_create_workspace_sessions_and_first_cards() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    request_json(app.clone(), Method::POST, "/api/projects", json!({"name":"Lifecycle","description":null})).await;
    request_json(app.clone(), Method::POST, "/api/projects/project_0001/repositories", json!({"name":"Repo","path":repo.path()})).await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    ).await;

    let (status, story_response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(story_response["story_specs"][0]["story_spec_id"], "story_spec_0001");
    assert_eq!(story_response["workspace_session"]["workspace_type"], "story");

    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lifecycle["story_specs"].as_array().unwrap().len(), 1);
}

async fn request_json(app: axum::Router, method: Method, uri: &str, body: Value) -> (StatusCode, Value) {
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
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

fn git_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("repo");
    let status = Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
    assert!(status.success());
    dir
}
```

- [ ] **Step 2: Run failing API tests**

Run:

```bash
cargo test --locked --test web_lifecycle_api
```

Expected: FAIL because `repository_id` is not accepted by `CreateProductIssueRequest`, lifecycle routes are not registered, and lifecycle response DTOs do not exist.

- [ ] **Step 3: Add web DTOs**

Modify `src/web/types.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateProductIssueRequest {
    pub title: String,
    pub description: Option<String>,
    pub change_id: Option<String>,
    pub repository_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueLifecycleResponse {
    pub issue: ProductIssueDto,
    pub story_specs: Vec<StorySpecDto>,
    pub design_specs: Vec<DesignSpecDto>,
    pub work_items: Vec<LifecycleWorkItemDto>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StorySpecDto {
    pub story_spec_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub title: String,
    pub current_version: Option<u32>,
    pub confirmation_status: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DesignSpecDto {
    pub design_spec_id: String,
    pub issue_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_kind: String,
    pub title: String,
    pub current_version: Option<u32>,
    pub confirmation_status: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LifecycleWorkItemDto {
    pub work_item_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub title: String,
    pub plan_status: String,
    pub execution_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GenerateStorySpecsRequest {
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GenerateStorySpecsResponse {
    pub story_specs: Vec<StorySpecDto>,
    pub workspace_session: WorkspaceSessionDto,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionDto {
    pub workspace_session_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub workspace_type: String,
    pub status: String,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
}
```

- [ ] **Step 4: Enforce repository on issue creation and add handlers**

Modify `src/web/handlers.rs`:

```rust
pub async fn create_product_issue(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<CreateProductIssueRequest>,
) -> ApiResult<Json<ProductIssueDto>> {
    let repository_id = request.repository_id.ok_or_else(|| {
        ApiError::validation("repository_required", "repository_id is required")
    })?;
    let _repository = find_repository(&product_app_paths(&state), &project_id, &repository_id)?;
    let store = IssueStore::new(product_app_paths(&state));
    let issue = store
        .create(CreateProductIssueInput {
            project_id,
            repo_id: Some(repository_id),
            title: request.title,
            description: request.description,
            change_id: request.change_id,
        })
        .map_err(product_store_api_error)?;
    Ok(Json(product_issue_dto(issue, None)))
}
```

Add handlers:

```rust
pub async fn issue_lifecycle(
    State(state): State<WebAppState>,
    Path(issue_id): Path<String>,
    Query(query): Query<GateResolveQuery>,
) -> ApiResult<Json<IssueLifecycleResponse>> {
    let project_id = query
        .project_id
        .ok_or_else(|| ApiError::validation("project_required", "project_id is required"))?;
    let app_paths = product_app_paths(&state);
    let issue = IssueStore::new(app_paths.clone())
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let lifecycle = LifecycleStore::new(app_paths);
    Ok(Json(IssueLifecycleResponse {
        issue: product_issue_dto_with_binding(&product_app_paths(&state), issue)?,
        story_specs: lifecycle
            .list_story_specs(&project_id, &issue_id)
            .map_err(product_store_api_error)?
            .into_iter()
            .map(story_spec_dto)
            .collect(),
        design_specs: lifecycle
            .list_design_specs(&project_id, &issue_id)
            .map_err(product_store_api_error)?
            .into_iter()
            .map(design_spec_dto)
            .collect(),
        work_items: lifecycle
            .list_work_items(&project_id, &issue_id)
            .map_err(product_store_api_error)?
            .into_iter()
            .map(lifecycle_work_item_dto)
            .collect(),
    }))
}

pub async fn generate_story_specs(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<GenerateStorySpecsRequest>,
) -> ApiResult<Json<GenerateStorySpecsResponse>> {
    let app_paths = product_app_paths(&state);
    let issue = IssueStore::new(app_paths.clone())
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let repository_id = issue
        .repo_id
        .clone()
        .ok_or_else(|| ApiError::validation("repository_required", "repository_id is required"))?;
    let lifecycle = LifecycleStore::new(app_paths);
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            repository_id,
            title: request.title,
        })
        .map_err(product_store_api_error)?;
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id,
            issue_id,
            entity_id: story.id.clone(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .map_err(product_store_api_error)?;
    Ok(Json(GenerateStorySpecsResponse {
        story_specs: vec![story_spec_dto(story)],
        workspace_session: workspace_session_dto(session),
    }))
}
```

Add DTO conversion helpers near existing conversion helpers:

```rust
fn story_spec_dto(record: StorySpecRecord) -> StorySpecDto {
    StorySpecDto {
        story_spec_id: record.id,
        issue_id: record.issue_id,
        repository_id: record.repository_id,
        title: record.title,
        current_version: record.current_version,
        confirmation_status: lifecycle_confirmation_status_text(&record.confirmation_status).to_string(),
    }
}
```

- [ ] **Step 5: Register lifecycle routes**

Modify `src/web/app.rs`:

```rust
.route("/api/issues/{issue_id}/lifecycle", get(handlers::issue_lifecycle))
.route(
    "/api/projects/{project_id}/issues/{issue_id}/story-specs:generate",
    post(handlers::generate_story_specs),
)
```

- [ ] **Step 6: Run API tests**

Run:

```bash
cargo test --locked --test web_lifecycle_api
```

Expected: PASS.

- [ ] **Step 7: Commit Task 2**

Run:

```bash
git add src/web/types.rs src/web/handlers.rs src/web/app.rs src/product/issue_store.rs tests/web_lifecycle_api.rs
git commit -m "feat: expose issue lifecycle api"
```

## Task 3: Workspace Session Actions And Provider Review Round Skeleton

**Files:**
- Modify: `src/product/lifecycle_store.rs`
- Modify: `src/web/types.rs`
- Modify: `src/web/handlers.rs`
- Modify: `src/web/app.rs`
- Test: `tests/web_lifecycle_api.rs`

- [ ] **Step 1: Add failing session action tests**

Append to `tests/web_lifecycle_api.rs`:

```rust
#[tokio::test]
async fn workspace_session_message_run_and_confirm_update_session_state() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    request_json(app.clone(), Method::POST, "/api/projects", json!({"name":"Lifecycle","description":null})).await;
    request_json(app.clone(), Method::POST, "/api/projects/project_0001/repositories", json!({"name":"Repo","path":repo.path()})).await;
    request_json(app.clone(), Method::POST, "/api/projects/project_0001/issues", json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"})).await;
    request_json(app.clone(), Method::POST, "/api/projects/project_0001/issues/issue_0001/story-specs:generate", json!({"title":"登录会话过期提示"})).await;

    let (status, message) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/message",
        json!({"role":"user","content":"请强调重新登录按钮"}),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(message["messages"][0]["content"], "请强调重新登录按钮");

    let (status, running) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/run-next",
        json!({}),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(running["status"], "waiting_for_human");

    let (status, confirmed) = request_json(
        app,
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(confirmed["status"], "confirmed");
}
```

- [ ] **Step 2: Run failing session action test**

Run:

```bash
cargo test --locked --test web_lifecycle_api workspace_session_message_run_and_confirm_update_session_state
```

Expected: FAIL because session action endpoints do not exist.

- [ ] **Step 3: Add store update methods**

In `src/product/lifecycle_store.rs`, add:

```rust
pub fn get_workspace_session(&self, session_id: &str) -> Result<WorkspaceSessionRecord, ProductStoreError>;
pub fn append_workspace_message(&self, session_id: &str, role: String, content: String) -> Result<WorkspaceSessionRecord, ProductStoreError>;
pub fn update_workspace_session_status(&self, session_id: &str, status: WorkspaceSessionStatus) -> Result<WorkspaceSessionRecord, ProductStoreError>;
```

Implementation rule: search all project issue `workspace-sessions/*.json` files for `session_id`. Reject path escapes with `validate_relative_id(session_id)`.

- [ ] **Step 4: Add request DTOs and handlers**

Add to `src/web/types.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionMessageRequest {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceSessionConfirmRequest {
    pub confirmed_by: String,
}
```

Add to `src/web/handlers.rs`:

```rust
pub async fn workspace_session_message(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<WorkspaceSessionMessageRequest>,
) -> ApiResult<Json<WorkspaceSessionDto>> {
    let session = LifecycleStore::new(product_app_paths(&state))
        .append_workspace_message(&session_id, request.role, request.content)
        .map_err(product_store_api_error)?;
    Ok(Json(workspace_session_dto(session)))
}

pub async fn workspace_session_run_next(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
) -> ApiResult<Json<WorkspaceSessionDto>> {
    let session = LifecycleStore::new(product_app_paths(&state))
        .update_workspace_session_status(&session_id, WorkspaceSessionStatus::WaitingForHuman)
        .map_err(product_store_api_error)?;
    Ok(Json(workspace_session_dto(session)))
}

pub async fn workspace_session_confirm(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(_request): Json<WorkspaceSessionConfirmRequest>,
) -> ApiResult<Json<WorkspaceSessionDto>> {
    let session = LifecycleStore::new(product_app_paths(&state))
        .update_workspace_session_status(&session_id, WorkspaceSessionStatus::Confirmed)
        .map_err(product_store_api_error)?;
    Ok(Json(workspace_session_dto(session)))
}
```

- [ ] **Step 5: Register workspace session routes**

Modify `src/web/app.rs`:

```rust
.route(
    "/api/workspace-sessions/{session_id}/message",
    post(handlers::workspace_session_message),
)
.route(
    "/api/workspace-sessions/{session_id}/run-next",
    post(handlers::workspace_session_run_next),
)
.route(
    "/api/workspace-sessions/{session_id}/confirm",
    post(handlers::workspace_session_confirm),
)
```

- [ ] **Step 6: Run lifecycle API tests**

Run:

```bash
cargo test --locked --test web_lifecycle_api
```

Expected: PASS.

- [ ] **Step 7: Commit Task 3**

Run:

```bash
git add src/product/lifecycle_store.rs src/web/types.rs src/web/handlers.rs src/web/app.rs tests/web_lifecycle_api.rs
git commit -m "feat: add provider workspace session actions"
```

## Task 4: Frontend API Types And Lifecycle Store Helpers

**Files:**
- Modify: `web/src/api/types.ts`
- Modify: `web/src/api/client.ts`
- Create: `web/src/state/lifecycle-workbench-store.ts`
- Create: `web/src/state/lifecycle-workbench-store.test.ts`

- [ ] **Step 1: Write failing frontend state tests**

Create `web/src/state/lifecycle-workbench-store.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import {
  groupLifecycleCards,
  lifecycleBlockedReason,
  visibleLifecycle,
} from "./lifecycle-workbench-store";
import type { IssueLifecycleResponse } from "../api/types";

const lifecycle: IssueLifecycleResponse = {
  issue: {
    issue_id: "issue_0001",
    project_id: "project_0001",
    repo_id: "repository_0001",
    workspace_id: null,
    task_id: null,
    session_id: null,
    title: "登录会话过期",
    description: "描述",
    change_id: "login-session-expired",
    phase: "clarification",
    status: "draft",
    active_binding_id: null,
    artifacts: [],
    created_at: "2026-05-16T00:00:00Z",
    updated_at: "2026-05-16T00:00:00Z",
  },
  story_specs: [
    {
      story_spec_id: "story_spec_0001",
      issue_id: "issue_0001",
      repository_id: "repository_0001",
      title: "会话过期提示",
      current_version: 1,
      confirmation_status: "confirmed",
    },
  ],
  design_specs: [
    {
      design_spec_id: "design_spec_0001",
      issue_id: "issue_0001",
      story_spec_ids: ["story_spec_0001"],
      design_kind: "frontend",
      title: "前端提示设计",
      current_version: 1,
      confirmation_status: "draft",
    },
  ],
  work_items: [],
};

describe("lifecycle workbench store", () => {
  it("groups lifecycle response into four columns", () => {
    const grouped = groupLifecycleCards([lifecycle]);
    expect(grouped.issue).toHaveLength(1);
    expect(grouped.story_spec).toHaveLength(1);
    expect(grouped.design_spec).toHaveLength(1);
    expect(grouped.work_item).toHaveLength(0);
  });

  it("filters cards by focused issue", () => {
    const grouped = visibleLifecycle(groupLifecycleCards([lifecycle]), "issue_0001");
    expect(grouped.story_spec[0].id).toBe("story_spec_0001");
    expect(grouped.design_spec[0].id).toBe("design_spec_0001");
  });

  it("blocks work item generation until design is confirmed", () => {
    expect(lifecycleBlockedReason("work_item", lifecycle)).toBe(
      "需要先确认至少一个 Design Spec",
    );
  });
});
```

- [ ] **Step 2: Run failing frontend state tests**

Run:

```bash
pnpm --dir web test -- src/state/lifecycle-workbench-store.test.ts
```

Expected: FAIL because `lifecycle-workbench-store.ts` and lifecycle types do not exist.

- [ ] **Step 3: Add frontend types**

Modify `web/src/api/types.ts`:

```ts
export type StorySpec = {
  story_spec_id: string;
  issue_id: string;
  repository_id: string;
  title: string;
  current_version: number | null;
  confirmation_status: "draft" | "in_review" | "confirmed" | "change_requested" | "blocked";
};

export type DesignSpec = {
  design_spec_id: string;
  issue_id: string;
  story_spec_ids: string[];
  design_kind: "frontend" | "backend";
  title: string;
  current_version: number | null;
  confirmation_status: "draft" | "in_review" | "confirmed" | "change_requested" | "blocked";
};

export type LifecycleWorkItem = {
  work_item_id: string;
  issue_id: string;
  repository_id: string;
  story_spec_ids: string[];
  design_spec_ids: string[];
  title: string;
  plan_status: "not_started" | "draft" | "confirmed" | "change_requested";
  execution_status: string;
};

export type IssueLifecycleResponse = {
  issue: ProductIssue;
  story_specs: StorySpec[];
  design_specs: DesignSpec[];
  work_items: LifecycleWorkItem[];
};

export type WorkspaceSession = {
  workspace_session_id: string;
  issue_id: string;
  entity_id: string;
  workspace_type: "story" | "design" | "work_item";
  status: string;
  review_rounds: number;
  superpowers_enabled: boolean;
  openspec_enabled: boolean;
};
```

- [ ] **Step 4: Add API client functions**

Modify `web/src/api/client.ts`:

```ts
export async function getIssueLifecycle(
  issueId: string,
  projectId: string,
): Promise<IssueLifecycleResponse> {
  return requestJson<IssueLifecycleResponse>(
    `/api/issues/${encodeURIComponent(issueId)}/lifecycle?project_id=${encodeURIComponent(projectId)}`,
  );
}

export async function generateStorySpecs(
  projectId: string,
  issueId: string,
  payload: { title: string },
): Promise<{ story_specs: StorySpec[]; workspace_session: WorkspaceSession }> {
  return requestJson(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/story-specs:generate`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export async function sendWorkspaceSessionMessage(
  sessionId: string,
  payload: { role: string; content: string },
): Promise<WorkspaceSession> {
  return requestJson(`/api/workspace-sessions/${encodeURIComponent(sessionId)}/message`, {
    method: "POST",
    body: JSON.stringify(payload),
  });
}
```

- [ ] **Step 5: Add lifecycle grouping helpers**

Create `web/src/state/lifecycle-workbench-store.ts`:

```ts
import type { DesignSpec, IssueLifecycleResponse, LifecycleWorkItem, ProductIssue, StorySpec } from "../api/types";

export type LifecycleCard =
  | { kind: "issue"; id: string; issueId: string; title: string; status: string; sourceIds: string[]; raw: ProductIssue }
  | { kind: "story_spec"; id: string; issueId: string; title: string; status: string; sourceIds: string[]; raw: StorySpec }
  | { kind: "design_spec"; id: string; issueId: string; title: string; status: string; sourceIds: string[]; raw: DesignSpec }
  | { kind: "work_item"; id: string; issueId: string; title: string; status: string; sourceIds: string[]; raw: LifecycleWorkItem };

export type LifecycleColumns = {
  issue: LifecycleCard[];
  story_spec: LifecycleCard[];
  design_spec: LifecycleCard[];
  work_item: LifecycleCard[];
};

export function groupLifecycleCards(lifecycles: IssueLifecycleResponse[]): LifecycleColumns {
  return lifecycles.reduce<LifecycleColumns>(
    (columns, lifecycle) => {
      columns.issue.push({
        kind: "issue",
        id: lifecycle.issue.issue_id,
        issueId: lifecycle.issue.issue_id,
        title: lifecycle.issue.title,
        status: lifecycle.issue.status,
        sourceIds: [],
        raw: lifecycle.issue,
      });
      lifecycle.story_specs.forEach((story) => {
        columns.story_spec.push({
          kind: "story_spec",
          id: story.story_spec_id,
          issueId: story.issue_id,
          title: story.title,
          status: story.confirmation_status,
          sourceIds: [story.issue_id],
          raw: story,
        });
      });
      lifecycle.design_specs.forEach((design) => {
        columns.design_spec.push({
          kind: "design_spec",
          id: design.design_spec_id,
          issueId: design.issue_id,
          title: design.title,
          status: design.confirmation_status,
          sourceIds: design.story_spec_ids,
          raw: design,
        });
      });
      lifecycle.work_items.forEach((item) => {
        columns.work_item.push({
          kind: "work_item",
          id: item.work_item_id,
          issueId: item.issue_id,
          title: item.title,
          status: item.execution_status,
          sourceIds: [...item.story_spec_ids, ...item.design_spec_ids],
          raw: item,
        });
      });
      return columns;
    },
    { issue: [], story_spec: [], design_spec: [], work_item: [] },
  );
}

export function visibleLifecycle(columns: LifecycleColumns, focusedIssueId: string | null): LifecycleColumns {
  if (!focusedIssueId) {
    return columns;
  }
  return {
    issue: columns.issue,
    story_spec: columns.story_spec.filter((card) => card.issueId === focusedIssueId),
    design_spec: columns.design_spec.filter((card) => card.issueId === focusedIssueId),
    work_item: columns.work_item.filter((card) => card.issueId === focusedIssueId),
  };
}

export function lifecycleBlockedReason(target: "design_spec" | "work_item" | "coding", lifecycle: IssueLifecycleResponse): string | null {
  if (target === "design_spec" && !lifecycle.story_specs.some((story) => story.confirmation_status === "confirmed")) {
    return "需要先确认至少一个 Story Spec";
  }
  if (target === "work_item" && !lifecycle.design_specs.some((design) => design.confirmation_status === "confirmed")) {
    return "需要先确认至少一个 Design Spec";
  }
  if (target === "coding" && !lifecycle.work_items.some((item) => item.plan_status === "confirmed")) {
    return "需要先确认 Work Item Plan";
  }
  return null;
}
```

- [ ] **Step 6: Run frontend state tests**

Run:

```bash
pnpm --dir web test -- src/state/lifecycle-workbench-store.test.ts
```

Expected: PASS.

- [ ] **Step 7: Commit Task 4**

Run:

```bash
git add web/src/api/types.ts web/src/api/client.ts web/src/state/lifecycle-workbench-store.ts web/src/state/lifecycle-workbench-store.test.ts
git commit -m "feat: add lifecycle frontend state"
```

## Task 5: Four-Column Issue Lifecycle Workbench

**Files:**
- Create: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
- Create: `web/src/components/lifecycle/LifecycleColumn.tsx`
- Create: `web/src/components/lifecycle/LifecycleCard.tsx`
- Create: `web/src/components/lifecycle/CreateLifecycleIssueDialog.tsx`
- Modify: `web/src/components/project/ProjectManagementWorkbench.tsx`
- Test: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`

- [ ] **Step 1: Write failing workbench component tests**

Create `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`:

```tsx
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";

describe("IssueLifecycleWorkbench", () => {
  it("renders four lifecycle columns and focuses derived cards by issue", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    expect(await screen.findByRole("region", { name: "Issue 列" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Story Spec 列" })).toHaveTextContent("会话过期提示");
    expect(screen.getByRole("region", { name: "Design Spec 列" })).toHaveTextContent("前端提示设计");
    expect(screen.getByRole("region", { name: "Work Item 列" })).toHaveTextContent("实现提示组件");

    await user.click(screen.getByRole("button", { name: "登录会话过期" }));
    expect(screen.getByRole("button", { name: "显示全部" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Story Spec 列" })).toHaveTextContent("会话过期提示");
  });

  it("requires repository when creating issue", async () => {
    vi.stubGlobal("fetch", lifecycleFetch());
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await screen.findByRole("region", { name: "Issue 列" });
    await user.click(screen.getByRole("button", { name: "新建 Issue" }));
    const dialog = screen.getByRole("dialog", { name: "新建 Issue" });
    await user.type(within(dialog).getByLabelText("Issue 标题"), "新增安全提示");
    await user.click(within(dialog).getByRole("button", { name: "创建 Issue" }));

    expect(within(dialog).getByText("请选择代码库")).toBeInTheDocument();
  });
});

function lifecycleFetch() {
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url === "/api/projects") {
      return jsonResponse({ projects: [{ project_id: "project_0001", name: "Aria", description: null, created_at: "2026-05-16T00:00:00Z", updated_at: "2026-05-16T00:00:00Z", last_opened_at: null }] });
    }
    if (url === "/api/projects/project_0001/repositories") {
      return jsonResponse({ repositories: [{ repository_id: "repository_0001", project_id: "project_0001", name: "Aria Repo", path: "/tmp/aria", repo_hash: "hash", runtime_root: "/tmp/aria/.aria/runtime", default_policy_preset: "manual-write", default_provider_mode: "fake", created_at: "2026-05-16T00:00:00Z", updated_at: "2026-05-16T00:00:00Z" }] });
    }
    if (url === "/api/projects/project_0001/issues") {
      return jsonResponse({ issues: [{ issue_id: "issue_0001", project_id: "project_0001", repo_id: "repository_0001", workspace_id: null, task_id: null, session_id: null, title: "登录会话过期", description: "描述", change_id: "login-session-expired", phase: "clarification", status: "draft", active_binding_id: null, artifacts: [], created_at: "2026-05-16T00:00:00Z", updated_at: "2026-05-16T00:00:00Z" }] });
    }
    if (url === "/api/issues/issue_0001/lifecycle?project_id=project_0001") {
      return jsonResponse({
        issue: { issue_id: "issue_0001", project_id: "project_0001", repo_id: "repository_0001", workspace_id: null, task_id: null, session_id: null, title: "登录会话过期", description: "描述", change_id: "login-session-expired", phase: "clarification", status: "draft", active_binding_id: null, artifacts: [], created_at: "2026-05-16T00:00:00Z", updated_at: "2026-05-16T00:00:00Z" },
        story_specs: [{ story_spec_id: "story_spec_0001", issue_id: "issue_0001", repository_id: "repository_0001", title: "会话过期提示", current_version: 1, confirmation_status: "confirmed" }],
        design_specs: [{ design_spec_id: "design_spec_0001", issue_id: "issue_0001", story_spec_ids: ["story_spec_0001"], design_kind: "frontend", title: "前端提示设计", current_version: 1, confirmation_status: "confirmed" }],
        work_items: [{ work_item_id: "work_item_0001", issue_id: "issue_0001", repository_id: "repository_0001", story_spec_ids: ["story_spec_0001"], design_spec_ids: ["design_spec_0001"], title: "实现提示组件", plan_status: "draft", execution_status: "planning" }],
      });
    }
    if (url === "/api/projects/project_0001/issues" && init?.method === "POST") {
      return jsonResponse({});
    }
    return jsonResponse({});
  });
}

function jsonResponse(body: unknown) {
  return Promise.resolve(new Response(JSON.stringify(body), { status: 200 }));
}
```

- [ ] **Step 2: Run failing component tests**

Run:

```bash
pnpm --dir web test -- src/components/lifecycle/IssueLifecycleWorkbench.test.tsx
```

Expected: FAIL because lifecycle components do not exist.

- [ ] **Step 3: Implement lifecycle columns and cards**

Create `web/src/components/lifecycle/LifecycleCard.tsx`:

```tsx
import { GitBranch, Layers3, ListChecks, ScrollText } from "lucide-react";
import type { LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";

export function LifecycleCard({
  card,
  selected,
  onSelect,
}: {
  card: LifecycleCardData;
  selected: boolean;
  onSelect: () => void;
}) {
  const Icon = card.kind === "issue" ? ListChecks : card.kind === "story_spec" ? ScrollText : card.kind === "design_spec" ? Layers3 : GitBranch;
  return (
    <button
      type="button"
      aria-label={card.title}
      aria-pressed={selected}
      onClick={onSelect}
      className={
        selected
          ? "w-full rounded-md border border-[var(--aria-primary)] bg-[var(--aria-panel)] p-3 text-left ring-2 ring-[var(--aria-primary)]"
          : "w-full rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 text-left transition-colors hover:bg-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
      }
    >
      <span className="flex min-w-0 items-start gap-2">
        <Icon className="mt-0.5 h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-semibold text-[var(--aria-ink)]">{card.title}</span>
          <span className="mt-1 flex flex-wrap gap-1.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
            <span>{card.id}</span>
            <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5">{card.status}</span>
          </span>
        </span>
      </span>
    </button>
  );
}
```

Create `web/src/components/lifecycle/LifecycleColumn.tsx`:

```tsx
import type { LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";
import { LifecycleCard } from "./LifecycleCard";

export function LifecycleColumn({
  title,
  ariaLabel,
  cards,
  selectedId,
  onSelect,
}: {
  title: string;
  ariaLabel: string;
  cards: LifecycleCardData[];
  selectedId: string | null;
  onSelect: (card: LifecycleCardData) => void;
}) {
  return (
    <section role="region" aria-label={ariaLabel} className="min-h-96 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2">
      <div className="mb-3 flex items-center justify-between gap-2">
        <h2 className="text-sm font-semibold text-[var(--aria-ink)]">{title}</h2>
        <span className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">{cards.length}</span>
      </div>
      <ul className="space-y-2">
        {cards.map((card) => (
          <li key={`${card.kind}:${card.id}`}>
            <LifecycleCard card={card} selected={selectedId === card.id} onSelect={() => onSelect(card)} />
          </li>
        ))}
      </ul>
    </section>
  );
}
```

- [ ] **Step 4: Implement IssueLifecycleWorkbench**

Create `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx` with these behaviors:

```tsx
import { Plus, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { getIssueLifecycle, listProductIssues, listProjects, listRepositories } from "../../api/client";
import type { IssueLifecycleResponse, Project, Repository } from "../../api/types";
import { groupLifecycleCards, visibleLifecycle, type LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";
import { WorkbenchSurface } from "../shell/WorkbenchSurface";
import { LifecycleColumn } from "./LifecycleColumn";
import { CreateLifecycleIssueDialog } from "./CreateLifecycleIssueDialog";

export function IssueLifecycleWorkbench() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [repositories, setRepositories] = useState<Repository[]>([]);
  const [lifecycles, setLifecycles] = useState<IssueLifecycleResponse[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [focusedIssueId, setFocusedIssueId] = useState<string | null>(null);
  const [selectedCardId, setSelectedCardId] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void refresh();
  }, []);

  async function refresh() {
    setError(null);
    const projectResponse = await listProjects();
    const projectId = selectedProjectId ?? projectResponse.projects[0]?.project_id ?? null;
    setProjects(projectResponse.projects);
    setSelectedProjectId(projectId);
    if (!projectId) {
      setLifecycles([]);
      return;
    }
    const [repositoryResponse, issueResponse] = await Promise.all([
      listRepositories(projectId),
      listProductIssues(projectId),
    ]);
    setRepositories(repositoryResponse.repositories ?? []);
    const lifecycleResponses = await Promise.all(
      (issueResponse.issues ?? []).map((issue) => getIssueLifecycle(issue.issue_id, projectId)),
    );
    setLifecycles(lifecycleResponses);
  }

  const columns = useMemo(() => visibleLifecycle(groupLifecycleCards(lifecycles), focusedIssueId), [lifecycles, focusedIssueId]);

  function handleSelectCard(card: LifecycleCardData) {
    setSelectedCardId(card.id);
    if (card.kind === "issue") {
      setFocusedIssueId(card.issueId);
    }
  }

  return (
    <WorkbenchSurface
      mainLabel="Issue 生命周期工作台"
      alert={error}
      header={
        <div className="flex min-w-0 flex-wrap items-center justify-between gap-3">
          <strong className="text-base font-semibold text-[var(--aria-ink)]">Aria Web</strong>
          <div className="flex flex-wrap items-center gap-2">
            {focusedIssueId ? (
              <button type="button" onClick={() => setFocusedIssueId(null)} className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold">
                显示全部
              </button>
            ) : null}
            <button type="button" onClick={() => void refresh()} className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-[var(--aria-line)]" title="刷新">
              <RefreshCw className="h-4 w-4" />
            </button>
            <button type="button" onClick={() => setDialogOpen(true)} className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white">
              <Plus className="mr-1 h-4 w-4" />
              新建 Issue
            </button>
          </div>
        </div>
      }
      main={
        <div className="grid min-h-[calc(100vh-6rem)] gap-3 overflow-auto rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 xl:grid-cols-4">
          <LifecycleColumn title="Issue" ariaLabel="Issue 列" cards={columns.issue} selectedId={selectedCardId} onSelect={handleSelectCard} />
          <LifecycleColumn title="Story Spec" ariaLabel="Story Spec 列" cards={columns.story_spec} selectedId={selectedCardId} onSelect={handleSelectCard} />
          <LifecycleColumn title="Design Spec" ariaLabel="Design Spec 列" cards={columns.design_spec} selectedId={selectedCardId} onSelect={handleSelectCard} />
          <LifecycleColumn title="Work Item" ariaLabel="Work Item 列" cards={columns.work_item} selectedId={selectedCardId} onSelect={handleSelectCard} />
        </div>
      }
    />
  );
}
```

- [ ] **Step 5: Add create issue dialog**

Create `web/src/components/lifecycle/CreateLifecycleIssueDialog.tsx` with a required Repository select and inline error text `请选择代码库` when submitted without `repository_id`.

- [ ] **Step 6: Run component tests**

Run:

```bash
pnpm --dir web test -- src/components/lifecycle/IssueLifecycleWorkbench.test.tsx
```

Expected: PASS.

- [ ] **Step 7: Commit Task 5**

Run:

```bash
git add web/src/components/lifecycle web/src/components/project/ProjectManagementWorkbench.tsx
git commit -m "feat: add issue lifecycle workbench"
```

## Task 6: Provider Workspace Dialog

**Files:**
- Create: `web/src/components/workspace/ProviderWorkspaceDialog.tsx`
- Create: `web/src/components/workspace/WorkspaceFlowRail.tsx`
- Create: `web/src/components/workspace/WorkspaceConversation.tsx`
- Create: `web/src/components/workspace/WorkspaceArtifactPane.tsx`
- Test: `web/src/components/workspace/ProviderWorkspaceDialog.test.tsx`

- [ ] **Step 1: Write failing dialog tests**

Create `web/src/components/workspace/ProviderWorkspaceDialog.test.tsx`:

```tsx
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ProviderWorkspaceDialog } from "./ProviderWorkspaceDialog";

describe("ProviderWorkspaceDialog", () => {
  it("shows flow rail conversation artifact pane and config overrides", async () => {
    const user = userEvent.setup();
    const onMessage = vi.fn();

    render(
      <ProviderWorkspaceDialog
        open
        title="Story Workspace"
        session={{
          workspace_session_id: "workspace_session_0001",
          issue_id: "issue_0001",
          entity_id: "story_spec_0001",
          workspace_type: "story",
          status: "waiting_for_human",
          review_rounds: 2,
          superpowers_enabled: true,
          openspec_enabled: true,
        }}
        onClose={vi.fn()}
        onMessage={onMessage}
        onRunNext={vi.fn()}
        onConfirm={vi.fn()}
        onRequestChange={vi.fn()}
      />,
    );

    expect(screen.getByRole("dialog", { name: "Story Workspace" })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Workspace 流程" })).toHaveTextContent("author draft");
    expect(screen.getByRole("region", { name: "Provider 对话" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Workspace 产物" })).toHaveTextContent("workspace_session_0001");
    expect(screen.getByText("review 2")).toBeInTheDocument();
    expect(screen.getByText("superpowers")).toBeInTheDocument();
    expect(screen.getByText("openspec")).toBeInTheDocument();

    await user.type(screen.getByLabelText("补充指令"), "请补充边界条件");
    await user.click(screen.getByRole("button", { name: "发送" }));
    expect(onMessage).toHaveBeenCalledWith("请补充边界条件");
  });
});
```

- [ ] **Step 2: Run failing dialog test**

Run:

```bash
pnpm --dir web test -- src/components/workspace/ProviderWorkspaceDialog.test.tsx
```

Expected: FAIL because dialog components do not exist.

- [ ] **Step 3: Implement dialog shell**

Create `web/src/components/workspace/ProviderWorkspaceDialog.tsx`:

```tsx
import type { WorkspaceSession } from "../../api/types";
import { WorkspaceArtifactPane } from "./WorkspaceArtifactPane";
import { WorkspaceConversation } from "./WorkspaceConversation";
import { WorkspaceFlowRail } from "./WorkspaceFlowRail";

export function ProviderWorkspaceDialog({
  open,
  title,
  session,
  onClose,
  onMessage,
  onRunNext,
  onConfirm,
  onRequestChange,
}: {
  open: boolean;
  title: string;
  session: WorkspaceSession;
  onClose: () => void;
  onMessage: (content: string) => void;
  onRunNext: () => void;
  onConfirm: () => void;
  onRequestChange: () => void;
}) {
  if (!open) {
    return null;
  }
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/30 px-4 py-6">
      <section role="dialog" aria-modal="true" aria-label={title} className="grid max-h-[90vh] w-full max-w-6xl grid-rows-[auto_minmax(0,1fr)] rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-xl">
        <header className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--aria-line)] px-4 py-3">
          <div>
            <h2 className="text-base font-semibold text-[var(--aria-ink)]">{title}</h2>
            <div className="mt-1 flex flex-wrap gap-1.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
              <span>{session.workspace_session_id}</span>
              <span>review {session.review_rounds}</span>
              {session.superpowers_enabled ? <span>superpowers</span> : null}
              {session.openspec_enabled ? <span>openspec</span> : null}
            </div>
          </div>
          <button type="button" onClick={onClose} className="h-8 rounded-md border border-[var(--aria-line)] px-3 text-sm font-semibold">关闭</button>
        </header>
        <div className="grid min-h-0 gap-0 overflow-hidden lg:grid-cols-[15rem_minmax(0,1fr)_20rem]">
          <WorkspaceFlowRail workspaceType={session.workspace_type} status={session.status} />
          <WorkspaceConversation onMessage={onMessage} onRunNext={onRunNext} onConfirm={onConfirm} onRequestChange={onRequestChange} />
          <WorkspaceArtifactPane session={session} />
        </div>
      </section>
    </div>
  );
}
```

- [ ] **Step 4: Implement flow, conversation, artifact panes**

Create `WorkspaceFlowRail.tsx`, `WorkspaceConversation.tsx`, and `WorkspaceArtifactPane.tsx` with stable regions:

```tsx
export function WorkspaceFlowRail({ workspaceType, status }: { workspaceType: string; status: string }) {
  const steps = workspaceType === "work_item"
    ? ["prepare context", "author plan", "confirm plan", "coding", "testing", "review", "final"]
    : ["prepare context", "author draft", "cross review", "revise", "human confirm"];
  return (
    <nav aria-label="Workspace 流程" className="overflow-auto border-r border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3">
      <ol className="space-y-2">
        {steps.map((step) => (
          <li key={step} className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-1.5 text-xs font-semibold text-[var(--aria-ink)]">{step}</li>
        ))}
      </ol>
      <p className="mt-3 font-mono text-[11px] text-[var(--aria-ink-muted)]">{status}</p>
    </nav>
  );
}
```

- [ ] **Step 5: Run dialog tests**

Run:

```bash
pnpm --dir web test -- src/components/workspace/ProviderWorkspaceDialog.test.tsx
```

Expected: PASS.

- [ ] **Step 6: Commit Task 6**

Run:

```bash
git add web/src/components/workspace/ProviderWorkspaceDialog.tsx web/src/components/workspace/WorkspaceFlowRail.tsx web/src/components/workspace/WorkspaceConversation.tsx web/src/components/workspace/WorkspaceArtifactPane.tsx web/src/components/workspace/ProviderWorkspaceDialog.test.tsx
git commit -m "feat: add provider workspace dialog"
```

## Task 7: Integration Flow And Old Execution UI Demotion

**Files:**
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
- Modify: `web/src/app-shell.tsx`
- Modify: `web/src/router.tsx`
- Test: `web/src/main.test.tsx`
- Test: `web/e2e/issue-lifecycle-workspace.spec.ts`

- [ ] **Step 1: Write failing route and E2E tests**

Modify `web/src/main.test.tsx` to assert the default screen contains `Issue 生命周期工作台` and does not contain the old execution workbench primary heading.

Create `web/e2e/issue-lifecycle-workspace.spec.ts`:

```ts
import { test, expect } from "@playwright/test";

test("issue lifecycle workspace is the default product flow", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("main", { name: "Issue 生命周期工作台" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Issue 列" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Story Spec 列" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Design Spec 列" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Work Item 列" })).toBeVisible();
});
```

- [ ] **Step 2: Run failing frontend integration tests**

Run:

```bash
pnpm --dir web test -- src/main.test.tsx
```

Expected: FAIL until routing uses the lifecycle workbench.

- [ ] **Step 3: Route lifecycle UI as default**

Modify `web/src/app-shell.tsx` and `web/src/router.tsx` so the default product view renders `IssueLifecycleWorkbench`. Keep old execution UI reachable only through an internal debug state or direct compatibility path used by tests.

Required behavior:

```tsx
<IssueLifecycleWorkbench />
```

is the default first screen.

- [ ] **Step 4: Wire card actions to ProviderWorkspaceDialog**

In `IssueLifecycleWorkbench.tsx`, when selected card kind is `story_spec`, `design_spec`, or `work_item`, open `ProviderWorkspaceDialog` with the current session returned by the generate endpoint or selected from lifecycle state.

Minimal state:

```tsx
const [workspaceSession, setWorkspaceSession] = useState<WorkspaceSession | null>(null);
```

Render:

```tsx
{workspaceSession ? (
  <ProviderWorkspaceDialog
    open
    title={`${workspaceSession.workspace_type} Workspace`}
    session={workspaceSession}
    onClose={() => setWorkspaceSession(null)}
    onMessage={(content) => void sendWorkspaceSessionMessage(workspaceSession.workspace_session_id, { role: "user", content })}
    onRunNext={() => void runWorkspaceSessionNext(workspaceSession.workspace_session_id)}
    onConfirm={() => void confirmWorkspaceSession(workspaceSession.workspace_session_id, { confirmed_by: "human" })}
    onRequestChange={() => void requestWorkspaceSessionChange(workspaceSession.workspace_session_id, { role: "user", content: "要求修改" })}
  />
) : null}
```

- [ ] **Step 5: Run frontend tests**

Run:

```bash
pnpm --dir web test -- src/main.test.tsx src/components/lifecycle/IssueLifecycleWorkbench.test.tsx src/components/workspace/ProviderWorkspaceDialog.test.tsx
```

Expected: PASS.

- [ ] **Step 6: Commit Task 7**

Run:

```bash
git add web/src/app-shell.tsx web/src/router.tsx web/src/components/lifecycle/IssueLifecycleWorkbench.tsx web/src/main.test.tsx web/e2e/issue-lifecycle-workspace.spec.ts
git commit -m "feat: make lifecycle workbench default"
```

## Task 8: Provider Runner Adapter For Workspace Sessions

**Files:**
- Create: `src/product/provider_workspace_runner.rs`
- Modify: `src/product/mod.rs`
- Modify: `src/product/lifecycle_store.rs`
- Modify: `src/web/handlers.rs`
- Test: `tests/provider_workspace_runner.rs`
- Test: `tests/web_lifecycle_api.rs`

- [ ] **Step 1: Write failing provider runner test**

Create `tests/provider_workspace_runner.rs`:

```rust
use cadence_aria::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::{
    CreateStorySpecInput, CreateWorkspaceSessionInput, LifecycleStore,
};
use cadence_aria::product::models::{ProviderName, WorkspaceSessionStatus, WorkspaceType};
use cadence_aria::product::provider_workspace_runner::{
    ProviderWorkspaceRunner, WorkspaceProviderRunInput,
};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn workspace_runner_calls_provider_and_records_version_and_review_round() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = LifecycleStore::new(paths.clone());
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "会话过期提示".to_string(),
        })
        .expect("story");
    let session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id.clone(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("session");

    let runner = ProviderWorkspaceRunner::new(paths);
    let output = runner
        .run_next(
            WorkspaceProviderRunInput {
                session_id: session.id.clone(),
                user_prompt: "生成 Story Spec".to_string(),
            },
            &RecordingProvider,
        )
        .expect("run next");

    assert_eq!(output.session.status, WorkspaceSessionStatus::WaitingForHuman);
    assert_eq!(output.version.version, 1);
    assert!(output.version.markdown.contains("Story Spec generated by provider"));
    assert_eq!(output.review_round.round_index, 1);
    assert_eq!(output.review_round.author_provider, ProviderName::Codex);
    assert_eq!(output.review_round.reviewer_provider, ProviderName::ClaudeCode);
}

struct RecordingProvider;

impl ProviderAdapter for RecordingProvider {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        assert!(input.prompt.contains("生成 Story Spec"));
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: "Story Spec generated by provider".to_string(),
            stderr: String::new(),
            structured_output: Some(json!({
                "markdown": "# Story Spec\n\nStory Spec generated by provider",
                "review_result": "review passed",
                "revision_result": "revision applied"
            })),
            files_modified: Vec::new(),
            duration_ms: 12,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}
```

- [ ] **Step 2: Run failing provider runner test**

Run:

```bash
cargo test --locked --test provider_workspace_runner
```

Expected: FAIL because `provider_workspace_runner` does not exist.

- [ ] **Step 3: Add provider workspace runner module**

Create `src/product/provider_workspace_runner.rs`:

```rust
use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::{
    AppendProviderReviewRoundInput, AppendSpecVersionInput, LifecycleStore,
};
use crate::product::models::{
    ProviderReviewRoundRecord, SpecVersionRecord, WorkspaceSessionRecord, WorkspaceSessionStatus,
};
use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceProviderRunInput {
    pub session_id: String,
    pub user_prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceProviderRunOutput {
    pub session: WorkspaceSessionRecord,
    pub version: SpecVersionRecord,
    pub review_round: ProviderReviewRoundRecord,
}

#[derive(Debug, Clone)]
pub struct ProviderWorkspaceRunner {
    paths: ProductAppPaths,
}

impl ProviderWorkspaceRunner {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn run_next(
        &self,
        input: WorkspaceProviderRunInput,
        provider: &dyn ProviderAdapter,
    ) -> Result<WorkspaceProviderRunOutput, ProviderAdapterError> {
        let store = LifecycleStore::new(self.paths.clone());
        let session = store
            .get_workspace_session(&input.session_id)
            .map_err(|error| ProviderAdapterError::incompatible_output(error.to_string(), "", ""))?;
        let adapter_input = AdapterInput {
            provider_type: ProviderType::Fake,
            role: AdapterRole::Planner,
            worktree_path: None,
            prompt: input.user_prompt,
            output_schema: "provider_workspace_markdown".to_string(),
            context_files: Vec::new(),
            timeout: 2400,
            max_retries: 0,
        };
        let adapter_output = provider.run(&adapter_input)?;
        let structured = adapter_output.structured_output.unwrap_or_default();
        let markdown = structured
            .get("markdown")
            .and_then(|value| value.as_str())
            .unwrap_or(adapter_output.stdout.as_str())
            .to_string();
        let review_result = structured
            .get("review_result")
            .and_then(|value| value.as_str())
            .unwrap_or("review completed")
            .to_string();
        let revision_result = structured
            .get("revision_result")
            .and_then(|value| value.as_str())
            .unwrap_or("revision completed")
            .to_string();
        let version = store
            .append_version(AppendSpecVersionInput {
                project_id: session.project_id.clone(),
                issue_id: session.issue_id.clone(),
                entity_id: session.entity_id.clone(),
                markdown,
                provider_run_refs: vec![format!("provider_run_{}", session.id)],
                review_refs: vec![format!("review_round_{}", session.id)],
                confirmed_by: None,
            })
            .map_err(|error| ProviderAdapterError::incompatible_output(error.to_string(), "", ""))?;
        let review_round = store
            .append_provider_review_round(AppendProviderReviewRoundInput {
                project_id: session.project_id.clone(),
                issue_id: session.issue_id.clone(),
                session_id: session.id.clone(),
                round_index: 1,
                author_provider: session.author_provider.clone(),
                reviewer_provider: session.reviewer_provider.clone(),
                review_result,
                revision_result,
            })
            .map_err(|error| ProviderAdapterError::incompatible_output(error.to_string(), "", ""))?;
        let session = store
            .update_workspace_session_status(&session.id, WorkspaceSessionStatus::WaitingForHuman)
            .map_err(|error| ProviderAdapterError::incompatible_output(error.to_string(), "", ""))?;
        Ok(WorkspaceProviderRunOutput {
            session,
            version,
            review_round,
        })
    }
}
```

Modify `src/product/mod.rs`:

```rust
pub mod provider_workspace_runner;
```

- [ ] **Step 4: Add review round append method to lifecycle store**

In `src/product/lifecycle_store.rs`, add input and method:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendProviderReviewRoundInput {
    pub project_id: String,
    pub issue_id: String,
    pub session_id: String,
    pub round_index: u32,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_result: String,
    pub revision_result: String,
}

pub fn append_provider_review_round(
    &self,
    input: AppendProviderReviewRoundInput,
) -> Result<ProviderReviewRoundRecord, ProductStoreError> {
    validate_relative_id(&input.project_id)?;
    validate_relative_id(&input.issue_id)?;
    validate_relative_id(&input.session_id)?;
    let root = self
        .paths
        .issue_root(&input.project_id, &input.issue_id)
        .join("provider-review-rounds");
    let existing_len = count_json_files(&root)?;
    let id = next_sequential_id("review_round", existing_len);
    let now = Utc::now().to_rfc3339();
    let record = ProviderReviewRoundRecord {
        id: id.clone(),
        project_id: input.project_id,
        issue_id: input.issue_id,
        session_id: input.session_id,
        round_index: input.round_index,
        author_provider: input.author_provider,
        reviewer_provider: input.reviewer_provider,
        review_result: input.review_result,
        revision_result: input.revision_result,
        created_at: now,
    };
    write_json(&root.join(format!("{id}.json")), &record)?;
    Ok(record)
}
```

- [ ] **Step 5: Wire workspace-session run-next handler to provider runner**

Modify `workspace_session_run_next` in `src/web/handlers.rs`:

```rust
pub async fn workspace_session_run_next(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
) -> ApiResult<Json<WorkspaceSessionDto>> {
    let runner = ProviderWorkspaceRunner::new(product_app_paths(&state));
    let provider = crate::cross_cutting::provider_adapter::FakeProviderAdapter;
    let output = runner
        .run_next(
            WorkspaceProviderRunInput {
                session_id,
                user_prompt: "run next provider workspace step".to_string(),
            },
            &provider,
        )
        .map_err(|error| {
            ApiError::runtime(
                "provider_workspace_run_failed",
                "provider workspace run failed",
                serde_json::json!({"details": error.details}),
            )
        })?;
    Ok(Json(workspace_session_dto(output.session)))
}
```

- [ ] **Step 6: Run provider runner tests**

Run:

```bash
cargo test --locked --test provider_workspace_runner
cargo test --locked --test web_lifecycle_api workspace_session_message_run_and_confirm_update_session_state
```

Expected: PASS.

- [ ] **Step 7: Commit Task 8**

Run:

```bash
git add src/product/provider_workspace_runner.rs src/product/mod.rs src/product/lifecycle_store.rs src/web/handlers.rs tests/provider_workspace_runner.rs tests/web_lifecycle_api.rs
git commit -m "feat: run provider workspace sessions"
```

## Task 9: Full Verification

**Files:**
- No source file edits unless verification exposes a defect.

- [ ] **Step 1: Run Rust format check**

Run:

```bash
cargo fmt --check
```

Expected: PASS.

- [ ] **Step 2: Run Rust clippy**

Run:

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: PASS.

- [ ] **Step 3: Run Rust tests**

Run:

```bash
cargo test --locked -j 1
```

Expected: PASS.

- [ ] **Step 4: Run frontend tests**

Run:

```bash
pnpm --dir web test
```

Expected: PASS.

- [ ] **Step 5: Run frontend build**

Run:

```bash
pnpm --dir web build
```

Expected: PASS.

- [ ] **Step 6: Stop on verification failure**

If any verification step fails, stop this task and create a focused follow-up task that names the failing command, the failing test or compiler error, and the exact files changed by the fix. Do not make a generic verification commit from this task.

## Self-Review Notes

- Spec coverage: the plan covers four-column UI, Repository-required Issue creation, first-class Story/Design/Work Item storage, Workspace Session, provider review configuration fields, Plan-first Work Item flow, and old execution UI demotion.
- Scope guard: Task 8 wires Workspace Session `run-next` through a `ProviderAdapter`, so the lifecycle can execute provider-backed generation while still using fake provider in deterministic tests.
- Verification coverage: Rust store/API tests cover persistence and lifecycle dependencies. Frontend unit tests cover four-column display, focus filtering, required Repository selection, and Workspace dialog layout. Full verification keeps existing runtime tests intact.
