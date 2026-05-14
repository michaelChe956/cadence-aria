# Task And Workspace Workbench Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a local task management workbench where users create issues, register code workspaces, choose a workspace on Start, and then enter the existing Aria execution workbench.

**Architecture:** Keep the existing Aria execution runtime as the workspace-scoped engine. Add local JSON registries under `app_root/.aria/runtime/web/` for issue and workspace metadata, then route Start into the existing `WebRuntime` using the selected workspace path. Split the frontend into a default task management view and a reusable execution shell view.

**Tech Stack:** Rust 2024, Axum 0.8, serde JSON registries, tempfile/tower tests, React, TypeScript, Vite, Vitest, Testing Library, Tailwind CSS.

---

## Scope Check

The design covers one integrated MVP, not independent subsystems. Backend registry/API work and frontend workbench changes must ship together because the Start flow crosses both. The plan keeps this as one implementation sequence but commits after each working slice.

## File Structure

- Create `src/web/workspace_registry.rs`: local workspace model, JSON persistence, path validation, default workspace bootstrap.
- Create `src/web/issue_registry.rs`: local issue model, JSON persistence, Start state transitions, task/workspace lookup.
- Modify `src/web/types.rs`: API DTOs for workspaces, issues, and Start.
- Modify `src/web/state.rs`: store `app_root` and registry access instead of treating the launch workspace as the only execution workspace.
- Modify `src/web/handlers.rs`: add workspace/issue handlers and make task/projection/file endpoints workspace-aware.
- Modify `src/web/app.rs`: register new API routes.
- Modify `src/web/runtime.rs`: add helper entry points that operate on a selected workspace path.
- Modify `src/web/mod.rs`: export new modules.
- Modify `web/src/api/types.ts`: frontend API types.
- Modify `web/src/api/client.ts`: frontend API functions for workspaces, issues, Start, and workspace-aware existing calls.
- Create `web/src/components/workspace/WorkspaceManager.tsx`: local workspace list and create form.
- Create `web/src/components/task/TaskManagementWorkbench.tsx`: issue create/list/start UI.
- Create `web/src/components/task/StartIssueDialog.tsx`: workspace selection for Start.
- Modify `web/src/app-shell.tsx`: extract or parameterize the existing execution UI so it can run from `workspace_id + task_id`.
- Modify `web/src/router.tsx`: default to task management view, allow in-memory transition to execution view.
- Modify tests in `web/src/main.test.tsx`, `web/src/api/client.test.ts`, and focused component tests.

## Task 1: Backend Workspace Registry

**Files:**
- Create: `src/web/workspace_registry.rs`
- Modify: `src/web/mod.rs`

- [ ] **Step 1: Write failing workspace registry tests**

Add module tests at the bottom of `src/web/workspace_registry.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::tempdir;

    fn git_repo() -> tempfile::TempDir {
        let dir = tempdir().expect("tempdir");
        let status = Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .status()
            .expect("git init");
        assert!(status.success());
        dir
    }

    #[test]
    fn creates_lists_and_persists_workspaces() {
        let app = tempdir().expect("tempdir");
        let repo = git_repo();
        let registry = WorkspaceRegistry::new(app.path().to_path_buf());

        let created = registry
            .create(CreateWorkspaceInput {
                name: "Aria".to_string(),
                path: repo.path().to_path_buf(),
                default_policy_preset: None,
                default_provider_mode: None,
            })
            .expect("create workspace");

        assert_eq!(created.workspace_id, "workspace_0001");
        assert_eq!(created.name, "Aria");
        assert_eq!(created.default_policy_preset, "manual-write");
        assert_eq!(created.default_provider_mode, "fake");

        let reloaded = WorkspaceRegistry::new(app.path().to_path_buf())
            .list()
            .expect("list workspaces");
        assert_eq!(reloaded, vec![created]);
    }

    #[test]
    fn rejects_non_git_workspace_path() {
        let app = tempdir().expect("tempdir");
        let not_git = tempdir().expect("tempdir");
        fs::write(not_git.path().join("README.md"), "not a repo").expect("write file");
        let registry = WorkspaceRegistry::new(app.path().to_path_buf());

        let error = registry
            .create(CreateWorkspaceInput {
                name: "Not Git".to_string(),
                path: not_git.path().to_path_buf(),
                default_policy_preset: None,
                default_provider_mode: None,
            })
            .expect_err("non git repo should fail");

        assert_eq!(error.code(), "workspace_path_not_git_repo");
    }

    #[test]
    fn bootstraps_app_root_as_default_workspace_when_git_repo() {
        let app = git_repo();
        let registry = WorkspaceRegistry::new(app.path().to_path_buf());

        let workspaces = registry.ensure_default_workspace().expect("default workspace");

        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].workspace_id, "workspace_0001");
        assert_eq!(workspaces[0].path, app.path());
    }
}
```

- [ ] **Step 2: Run the failing tests**

Run: `cargo test --locked workspace_registry`

Expected: FAIL because `src/web/workspace_registry.rs` and the types do not exist.

- [ ] **Step 3: Implement workspace registry**

Create `src/web/workspace_registry.rs` with these public shapes and behavior:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{code}: {message}")]
pub struct WorkspaceRegistryError {
    code: &'static str,
    message: String,
}

impl WorkspaceRegistryError {
    pub fn code(&self) -> &'static str {
        self.code
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceRecord {
    pub workspace_id: String,
    pub name: String,
    pub path: PathBuf,
    pub default_policy_preset: String,
    pub default_provider_mode: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateWorkspaceInput {
    pub name: String,
    pub path: PathBuf,
    pub default_policy_preset: Option<String>,
    pub default_provider_mode: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceRegistry {
    app_root: PathBuf,
}

impl WorkspaceRegistry {
    pub fn new(app_root: PathBuf) -> Self {
        Self { app_root }
    }

    pub fn list(&self) -> Result<Vec<WorkspaceRecord>, WorkspaceRegistryError> {
        let path = self.path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&path).map_err(io_error)?;
        serde_json::from_reader(file).map_err(json_error)
    }

    pub fn create(
        &self,
        input: CreateWorkspaceInput,
    ) -> Result<WorkspaceRecord, WorkspaceRegistryError> {
        let canonical_path = validate_workspace_path(&input.path)?;
        let mut records = self.list()?;
        let now = Utc::now();
        let record = WorkspaceRecord {
            workspace_id: format!("workspace_{:04}", records.len() + 1),
            name: input.name.trim().to_string(),
            path: canonical_path,
            default_policy_preset: input
                .default_policy_preset
                .unwrap_or_else(|| "manual-write".to_string()),
            default_provider_mode: input.default_provider_mode.unwrap_or_else(|| "fake".to_string()),
            created_at: now,
            updated_at: now,
        };
        records.push(record.clone());
        self.write(&records)?;
        Ok(record)
    }

    pub fn get(&self, workspace_id: &str) -> Result<WorkspaceRecord, WorkspaceRegistryError> {
        self.list()?
            .into_iter()
            .find(|record| record.workspace_id == workspace_id)
            .ok_or_else(|| registry_error("workspace_not_found", workspace_id))
    }

    pub fn ensure_default_workspace(&self) -> Result<Vec<WorkspaceRecord>, WorkspaceRegistryError> {
        let existing = self.list()?;
        if !existing.is_empty() {
            return Ok(existing);
        }
        let name = self
            .app_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("workspace")
            .to_string();
        self.create(CreateWorkspaceInput {
            name,
            path: self.app_root.clone(),
            default_policy_preset: None,
            default_provider_mode: None,
        })?;
        self.list()
    }

    fn path(&self) -> PathBuf {
        self.app_root.join(".aria/runtime/web/workspaces.json")
    }

    fn write(&self, records: &[WorkspaceRecord]) -> Result<(), WorkspaceRegistryError> {
        let path = self.path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let file = fs::File::create(path).map_err(io_error)?;
        serde_json::to_writer_pretty(file, records).map_err(json_error)
    }
}

fn validate_workspace_path(path: &Path) -> Result<PathBuf, WorkspaceRegistryError> {
    if !path.exists() {
        return Err(registry_error("workspace_path_missing", &path.display().to_string()));
    }
    if !path.is_dir() {
        return Err(registry_error("workspace_path_not_directory", &path.display().to_string()));
    }
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(registry_error("workspace_path_not_git_repo", &path.display().to_string()));
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
}

fn registry_error(code: &'static str, value: &str) -> WorkspaceRegistryError {
    WorkspaceRegistryError {
        code,
        message: value.to_string(),
    }
}

fn io_error(error: std::io::Error) -> WorkspaceRegistryError {
    WorkspaceRegistryError {
        code: "workspace_registry_io",
        message: error.to_string(),
    }
}

fn json_error(error: serde_json::Error) -> WorkspaceRegistryError {
    WorkspaceRegistryError {
        code: "workspace_registry_json",
        message: error.to_string(),
    }
}
```

Modify `src/web/mod.rs`:

```rust
pub mod workspace_registry;
```

- [ ] **Step 4: Run tests**

Run: `cargo test --locked workspace_registry`

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```bash
git add src/web/workspace_registry.rs src/web/mod.rs
git commit -m "feat: add web workspace registry"
```

## Task 2: Backend Issue Registry And Start State

**Files:**
- Create: `src/web/issue_registry.rs`
- Modify: `src/web/mod.rs`

- [ ] **Step 1: Write failing issue registry tests**

Add tests to `src/web/issue_registry.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_lists_and_persists_issues() {
        let app = tempdir().expect("tempdir");
        let registry = IssueRegistry::new(app.path().to_path_buf());

        let issue = registry
            .create(CreateIssueInput {
                title: "Add workspace picker".to_string(),
                description: Some("Choose a code repo before start".to_string()),
                change_id: None,
            })
            .expect("create issue");

        assert_eq!(issue.issue_id, "issue_0001");
        assert_eq!(issue.status, IssueStatus::Draft);
        assert_eq!(issue.change_id, "add-workspace-picker");

        let reloaded = IssueRegistry::new(app.path().to_path_buf())
            .list()
            .expect("list issues");
        assert_eq!(reloaded, vec![issue]);
    }

    #[test]
    fn start_links_issue_to_workspace_and_task_once() {
        let app = tempdir().expect("tempdir");
        let registry = IssueRegistry::new(app.path().to_path_buf());
        let issue = registry
            .create(CreateIssueInput {
                title: "Run selected repo".to_string(),
                description: None,
                change_id: None,
            })
            .expect("create issue");

        let started = registry
            .mark_started(&issue.issue_id, "workspace_0001", "task_0001", "sess_task_0001")
            .expect("mark started");
        let started_again = registry
            .mark_started(&issue.issue_id, "workspace_9999", "task_9999", "sess_task_9999")
            .expect("mark started again");

        assert_eq!(started.workspace_id.as_deref(), Some("workspace_0001"));
        assert_eq!(started.task_id.as_deref(), Some("task_0001"));
        assert_eq!(started_again.workspace_id.as_deref(), Some("workspace_0001"));
        assert_eq!(started_again.task_id.as_deref(), Some("task_0001"));
    }

    #[test]
    fn finds_workspace_for_task() {
        let app = tempdir().expect("tempdir");
        let registry = IssueRegistry::new(app.path().to_path_buf());
        let issue = registry
            .create(CreateIssueInput {
                title: "Lookup task".to_string(),
                description: None,
                change_id: None,
            })
            .expect("create issue");
        registry
            .mark_started(&issue.issue_id, "workspace_0001", "task_0001", "sess_task_0001")
            .expect("start");

        let link = registry.find_by_task("task_0001").expect("find task");

        assert_eq!(link.issue_id, "issue_0001");
        assert_eq!(link.workspace_id, "workspace_0001");
    }
}
```

- [ ] **Step 2: Run failing tests**

Run: `cargo test --locked issue_registry`

Expected: FAIL because `IssueRegistry` does not exist.

- [ ] **Step 3: Implement issue registry**

Create `src/web/issue_registry.rs` with public model names used by the tests:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{code}: {message}")]
pub struct IssueRegistryError {
    code: &'static str,
    message: String,
}

impl IssueRegistryError {
    pub fn code(&self) -> &'static str {
        self.code
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    Draft,
    Started,
    Running,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueRecord {
    pub issue_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: IssueStatus,
    pub workspace_id: Option<String>,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub change_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateIssueInput {
    pub title: String,
    pub description: Option<String>,
    pub change_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskWorkspaceLink {
    pub issue_id: String,
    pub workspace_id: String,
}

#[derive(Debug, Clone)]
pub struct IssueRegistry {
    app_root: PathBuf,
}

impl IssueRegistry {
    pub fn new(app_root: PathBuf) -> Self {
        Self { app_root }
    }

    pub fn list(&self) -> Result<Vec<IssueRecord>, IssueRegistryError> {
        let path = self.path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(path).map_err(io_error)?;
        serde_json::from_reader(file).map_err(json_error)
    }

    pub fn create(&self, input: CreateIssueInput) -> Result<IssueRecord, IssueRegistryError> {
        let title = input.title.trim().to_string();
        if title.is_empty() {
            return Err(registry_error("issue_title_required", "title is required"));
        }
        let mut issues = self.list()?;
        let now = Utc::now();
        let issue = IssueRecord {
            issue_id: format!("issue_{:04}", issues.len() + 1),
            title: title.clone(),
            description: input.description.filter(|value| !value.trim().is_empty()),
            status: IssueStatus::Draft,
            workspace_id: None,
            task_id: None,
            session_id: None,
            change_id: input.change_id.unwrap_or_else(|| slugify(&title)),
            created_at: now,
            updated_at: now,
        };
        issues.push(issue.clone());
        self.write(&issues)?;
        Ok(issue)
    }

    pub fn mark_started(
        &self,
        issue_id: &str,
        workspace_id: &str,
        task_id: &str,
        session_id: &str,
    ) -> Result<IssueRecord, IssueRegistryError> {
        let mut issues = self.list()?;
        let issue = issues
            .iter_mut()
            .find(|issue| issue.issue_id == issue_id)
            .ok_or_else(|| registry_error("issue_not_found", issue_id))?;
        if issue.task_id.is_none() {
            issue.status = IssueStatus::Started;
            issue.workspace_id = Some(workspace_id.to_string());
            issue.task_id = Some(task_id.to_string());
            issue.session_id = Some(session_id.to_string());
            issue.updated_at = Utc::now();
        }
        let cloned = issue.clone();
        self.write(&issues)?;
        Ok(cloned)
    }

    pub fn find_by_task(&self, task_id: &str) -> Result<TaskWorkspaceLink, IssueRegistryError> {
        let issue = self
            .list()?
            .into_iter()
            .find(|issue| issue.task_id.as_deref() == Some(task_id))
            .ok_or_else(|| registry_error("task_workspace_not_found", task_id))?;
        Ok(TaskWorkspaceLink {
            issue_id: issue.issue_id,
            workspace_id: issue
                .workspace_id
                .ok_or_else(|| registry_error("issue_missing_workspace", task_id))?,
        })
    }

    fn path(&self) -> PathBuf {
        self.app_root.join(".aria/runtime/web/issues.json")
    }

    fn write(&self, issues: &[IssueRecord]) -> Result<(), IssueRegistryError> {
        let path = self.path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let file = fs::File::create(path).map_err(io_error)?;
        serde_json::to_writer_pretty(file, issues).map_err(json_error)
    }
}

fn slugify(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn registry_error(code: &'static str, value: &str) -> IssueRegistryError {
    IssueRegistryError {
        code,
        message: value.to_string(),
    }
}

fn io_error(error: std::io::Error) -> IssueRegistryError {
    IssueRegistryError {
        code: "issue_registry_io",
        message: error.to_string(),
    }
}

fn json_error(error: serde_json::Error) -> IssueRegistryError {
    IssueRegistryError {
        code: "issue_registry_json",
        message: error.to_string(),
    }
}
```

Modify `src/web/mod.rs`:

```rust
pub mod issue_registry;
```

- [ ] **Step 4: Run tests**

Run: `cargo test --locked issue_registry`

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```bash
git add src/web/issue_registry.rs src/web/mod.rs
git commit -m "feat: add web issue registry"
```

## Task 3: Backend Workspace And Issue API

**Files:**
- Modify: `src/web/types.rs`
- Modify: `src/web/handlers.rs`
- Modify: `src/web/app.rs`
- Modify: `src/web/state.rs`

- [ ] **Step 1: Write failing API tests**

Add tests to `src/web/handlers.rs` using `tower::ServiceExt`:

```rust
#[cfg(test)]
mod api_tests {
    use super::*;
    use crate::web::app::build_web_router;
    use crate::web::runtime::WebRuntime;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use std::process::Command;
    use tempfile::tempdir;
    use tower::ServiceExt;

    fn git_repo() -> tempfile::TempDir {
        let dir = tempdir().expect("tempdir");
        assert!(Command::new("git").args(["init"]).current_dir(dir.path()).status().unwrap().success());
        dir
    }

    #[tokio::test]
    async fn workspace_and_issue_start_api_round_trip() {
        let app_root = tempdir().expect("app root");
        let workspace = git_repo();
        let state = WebAppState::new(app_root.path().to_path_buf(), WebRuntime::new_fake(app_root.path().to_path_buf()));
        let app = build_web_router(state);

        let create_workspace = Request::builder()
            .method(Method::POST)
            .uri("/api/workspaces")
            .header("content-type", "application/json")
            .body(Body::from(format!(
                r#"{{"name":"Repo","path":"{}"}}"#,
                workspace.path().display()
            )))
            .unwrap();
        let response = app.clone().oneshot(create_workspace).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let create_issue = Request::builder()
            .method(Method::POST)
            .uri("/api/issues")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"title":"Implement picker","description":"Start with workspace"}"#))
            .unwrap();
        let response = app.clone().oneshot(create_issue).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let start_issue = Request::builder()
            .method(Method::POST)
            .uri("/api/issues/issue_0001/start")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"workspace_id":"workspace_0001"}"#))
            .unwrap();
        let response = app.oneshot(start_issue).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
```

- [ ] **Step 2: Run failing API test**

Run: `cargo test --locked workspace_and_issue_start_api_round_trip`

Expected: FAIL because routes and DTOs do not exist.

- [ ] **Step 3: Add DTOs**

Add to `src/web/types.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceListResponse {
    pub workspaces: Vec<WorkspaceDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceDto {
    pub workspace_id: String,
    pub name: String,
    pub path: String,
    pub default_policy_preset: String,
    pub default_provider_mode: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateWorkspaceRequest {
    pub name: String,
    pub path: String,
    pub default_policy_preset: Option<String>,
    pub default_provider_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueListResponse {
    pub issues: Vec<IssueDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueDto {
    pub issue_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub workspace_id: Option<String>,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub change_id: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateIssueRequest {
    pub title: String,
    pub description: Option<String>,
    pub change_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StartIssueRequest {
    pub workspace_id: String,
    pub policy_preset: Option<String>,
    pub provider_mode: Option<String>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StartIssueResponse {
    pub issue_id: String,
    pub workspace_id: String,
    pub task_id: String,
    pub session_id: String,
    pub status: String,
}
```

- [ ] **Step 4: Update app state**

Change `src/web/state.rs` so `workspace_root` becomes `app_root` while preserving constructor names:

```rust
#[derive(Clone)]
pub struct WebAppState {
    pub app_root: PathBuf,
    pub runtime: Arc<Mutex<WebRuntime>>,
    pub events: EventHub,
}
```

Update `new` and `with_events` to fill `app_root`.

- [ ] **Step 5: Implement handlers**

In `src/web/handlers.rs`, import registries and implement:

```rust
pub async fn list_workspaces(State(state): State<WebAppState>) -> ApiResult<Json<WorkspaceListResponse>> {
    let registry = WorkspaceRegistry::new(state.app_root.clone());
    let records = registry.ensure_default_workspace().map_err(crate::web::error::ApiError::from)?;
    Ok(Json(WorkspaceListResponse {
        workspaces: records.into_iter().map(workspace_dto).collect(),
    }))
}

pub async fn create_workspace(
    State(state): State<WebAppState>,
    Json(request): Json<CreateWorkspaceRequest>,
) -> ApiResult<Json<WorkspaceDto>> {
    let registry = WorkspaceRegistry::new(state.app_root.clone());
    let record = registry.create(CreateWorkspaceInput {
        name: request.name,
        path: request.path.into(),
        default_policy_preset: request.default_policy_preset,
        default_provider_mode: request.default_provider_mode,
    }).map_err(crate::web::error::ApiError::from)?;
    Ok(Json(workspace_dto(record)))
}

pub async fn list_issues(State(state): State<WebAppState>) -> ApiResult<Json<IssueListResponse>> {
    let registry = IssueRegistry::new(state.app_root.clone());
    Ok(Json(IssueListResponse {
        issues: registry.list().map_err(crate::web::error::ApiError::from)?.into_iter().map(issue_dto).collect(),
    }))
}

pub async fn create_issue(
    State(state): State<WebAppState>,
    Json(request): Json<CreateIssueRequest>,
) -> ApiResult<Json<IssueDto>> {
    let registry = IssueRegistry::new(state.app_root.clone());
    let issue = registry.create(CreateIssueInput {
        title: request.title,
        description: request.description,
        change_id: request.change_id,
    }).map_err(crate::web::error::ApiError::from)?;
    Ok(Json(issue_dto(issue)))
}
```

Implement `start_issue` by loading workspace, creating `WebRuntime::new_fake(workspace.path.clone())`, calling `create_task`, and recording the link in `IssueRegistry::mark_started`.

- [ ] **Step 6: Wire routes**

Add to `src/web/app.rs`:

```rust
.route(
    "/api/workspaces",
    get(handlers::list_workspaces).post(handlers::create_workspace),
)
.route(
    "/api/issues",
    get(handlers::list_issues).post(handlers::create_issue),
)
.route("/api/issues/{issue_id}/start", post(handlers::start_issue))
```

- [ ] **Step 7: Run API test**

Run: `cargo test --locked workspace_and_issue_start_api_round_trip`

Expected: PASS.

- [ ] **Step 8: Commit**

Run:

```bash
git add src/web/types.rs src/web/handlers.rs src/web/app.rs src/web/state.rs
git commit -m "feat: expose issue and workspace web APIs"
```

## Task 4: Workspace-Aware Execution APIs

**Files:**
- Modify: `src/web/handlers.rs`
- Modify: `src/web/runtime.rs`
- Modify: `src/web/types.rs`
- Modify: `web/src/api/client.ts` later consumes this, but do not edit frontend in this task.

- [ ] **Step 1: Write failing projection and task control tests**

Add a Rust API test that creates a workspace, starts an issue, then requests:

```rust
GET /api/projection?workspace_id=workspace_0001&task_id=task_0001
POST /api/tasks/task_0001/advance?workspace_id=workspace_0001
```

Assert both responses are `200 OK`.

- [ ] **Step 2: Run failing test**

Run: `cargo test --locked workspace_aware_execution`

Expected: FAIL because existing handlers ignore `workspace_id`.

- [ ] **Step 3: Add workspace query structs**

Extend query types in `src/web/handlers.rs`:

```rust
#[derive(Debug, Deserialize)]
pub struct ProjectionQuery {
    pub workspace_id: Option<String>,
    pub task_id: Option<String>,
    pub node_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceQuery {
    pub workspace_id: Option<String>,
}
```

- [ ] **Step 4: Add workspace resolver helper**

In `src/web/handlers.rs`:

```rust
fn resolve_workspace_root(
    app_root: &std::path::Path,
    workspace_id: Option<&str>,
    task_id: Option<&str>,
) -> ApiResult<std::path::PathBuf> {
    let workspace_registry = WorkspaceRegistry::new(app_root.to_path_buf());
    if let Some(workspace_id) = workspace_id {
        return Ok(workspace_registry
            .get(workspace_id)
            .map_err(crate::web::error::ApiError::from)?
            .path);
    }
    if let Some(task_id) = task_id {
        let link = IssueRegistry::new(app_root.to_path_buf())
            .find_by_task(task_id)
            .map_err(crate::web::error::ApiError::from)?;
        return Ok(workspace_registry
            .get(&link.workspace_id)
            .map_err(crate::web::error::ApiError::from)?
            .path);
    }
    Ok(app_root.to_path_buf())
}
```

- [ ] **Step 5: Update handlers to use selected workspace**

Change `projection`, `advance_task`, `confirm_task`, `stop_task`, `rollback_preview`, `rollback_task`, `artifact_content`, `file_content`, and `file_diff` to resolve `workspace_root` before calling `WebRuntime`.

- [ ] **Step 6: Run backend tests**

Run: `cargo test --locked workspace_aware_execution`

Expected: PASS.

- [ ] **Step 7: Commit**

Run:

```bash
git add src/web/handlers.rs src/web/runtime.rs src/web/types.rs
git commit -m "feat: make execution APIs workspace aware"
```

## Task 5: Frontend API Client

**Files:**
- Modify: `web/src/api/types.ts`
- Modify: `web/src/api/client.ts`
- Modify: `web/src/api/client.test.ts`

- [ ] **Step 1: Write failing client tests**

Add tests:

```ts
it("creates a workspace through the API", async () => {
  const fetchMock = vi.fn(async () => jsonResponse({
    workspace_id: "workspace_0001",
    name: "Repo",
    path: "/tmp/repo",
    default_policy_preset: "manual-write",
    default_provider_mode: "fake",
    created_at: "2026-05-14T00:00:00Z",
    updated_at: "2026-05-14T00:00:00Z",
  }));
  vi.stubGlobal("fetch", fetchMock);

  await createWorkspace({ name: "Repo", path: "/tmp/repo" });

  expect(fetchMock).toHaveBeenCalledWith("/api/workspaces", expect.objectContaining({
    method: "POST",
    body: JSON.stringify({ name: "Repo", path: "/tmp/repo" }),
  }));
});

it("starts an issue through the API", async () => {
  const fetchMock = vi.fn(async () => jsonResponse({
    issue_id: "issue_0001",
    workspace_id: "workspace_0001",
    task_id: "task_0001",
    session_id: "sess_task_0001",
    status: "started",
  }));
  vi.stubGlobal("fetch", fetchMock);

  await startIssue("issue_0001", { workspace_id: "workspace_0001" });

  expect(fetchMock).toHaveBeenCalledWith("/api/issues/issue_0001/start", expect.objectContaining({
    method: "POST",
  }));
});
```

- [ ] **Step 2: Run failing frontend client tests**

Run: `pnpm --dir web test -- client`

Expected: FAIL because API functions and types do not exist.

- [ ] **Step 3: Add frontend types**

Add `Workspace`, `Issue`, `CreateWorkspaceRequest`, `CreateIssueRequest`, `StartIssueRequest`, and `StartIssueResponse` to `web/src/api/types.ts` using the same snake_case field names as Rust DTOs.

- [ ] **Step 4: Add client functions**

Add to `web/src/api/client.ts`:

```ts
export function listWorkspaces(): Promise<{ workspaces: Workspace[] }> {
  return requestJson("/api/workspaces");
}

export function createWorkspace(payload: CreateWorkspaceRequest): Promise<Workspace> {
  return requestJson("/api/workspaces", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function listIssues(): Promise<{ issues: Issue[] }> {
  return requestJson("/api/issues");
}

export function createIssue(payload: CreateIssueRequest): Promise<Issue> {
  return requestJson("/api/issues", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function startIssue(
  issueId: string,
  payload: StartIssueRequest,
): Promise<StartIssueResponse> {
  return requestJson(`/api/issues/${encodeURIComponent(issueId)}/start`, {
    method: "POST",
    body: JSON.stringify(payload),
  });
}
```

- [ ] **Step 5: Add workspace-aware execution calls**

Update `getProjection`, `advanceTask`, `confirmTask`, `stopTask`, `rollbackPreview`, `rollbackTask`, `getArtifactContent`, `getFileContent`, and `getFileDiff` to accept `workspaceId?: string` and append `workspace_id` to query strings.

- [ ] **Step 6: Run frontend client tests**

Run: `pnpm --dir web test -- client`

Expected: PASS.

- [ ] **Step 7: Commit**

Run:

```bash
git add web/src/api/types.ts web/src/api/client.ts web/src/api/client.test.ts
git commit -m "feat: add issue and workspace web client APIs"
```

## Task 6: Task Management Workbench UI

**Files:**
- Create: `web/src/components/workspace/WorkspaceManager.tsx`
- Create: `web/src/components/task/StartIssueDialog.tsx`
- Create: `web/src/components/task/TaskManagementWorkbench.tsx`
- Modify: `web/src/main.test.tsx`

- [ ] **Step 1: Write failing default homepage test**

In `web/src/main.test.tsx`, change the first-screen test to expect task management:

```ts
it("renders task management as the default workbench", async () => {
  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url === "/api/workspaces") return jsonResponse({ workspaces: [] });
    if (url === "/api/issues") return jsonResponse({ issues: [] });
    return jsonResponse({});
  }));

  render(<AppShell />);

  expect(await screen.findByRole("heading", { name: "任务管理" })).toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "Workspace 空间" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "新建 issue" })).toBeDisabled();
});
```

- [ ] **Step 2: Run failing UI test**

Run: `pnpm --dir web test -- main`

Expected: FAIL because the default view is still the execution workbench.

- [ ] **Step 3: Implement WorkspaceManager**

Create a controlled component that renders workspace list and a create form. Use labels `workspace 名称` and `workspace 路径`; disable submit when either is empty; call `onCreateWorkspace`.

- [ ] **Step 4: Implement StartIssueDialog**

Create a dialog-like section with `role="dialog"` and label `选择 workspace` when an issue is being started. Render a `select` labelled `启动 workspace` and a button labelled `确认 Start`.

- [ ] **Step 5: Implement TaskManagementWorkbench**

Use `listWorkspaces`, `listIssues`, `createWorkspace`, `createIssue`, and `startIssue`. Keep local `busy` and `error` state. On Start success call:

```ts
onOpenExecution({
  issueId: response.issue_id,
  workspaceId: response.workspace_id,
  taskId: response.task_id,
});
```

- [ ] **Step 6: Run UI test**

Run: `pnpm --dir web test -- main`

Expected: PASS for default homepage test; older execution tests may still fail until Task 7.

- [ ] **Step 7: Commit**

Run:

```bash
git add web/src/components/workspace/WorkspaceManager.tsx web/src/components/task/StartIssueDialog.tsx web/src/components/task/TaskManagementWorkbench.tsx web/src/main.test.tsx
git commit -m "feat: add task management workbench UI"
```

## Task 7: Reuse Existing Execution Workbench After Start

**Files:**
- Modify: `web/src/app-shell.tsx`
- Modify: `web/src/router.tsx`
- Modify: `web/src/main.test.tsx`

- [ ] **Step 1: Write failing Start-to-execution test**

Add test:

```ts
it("starts an issue with a workspace and opens the execution workbench", async () => {
  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url === "/api/workspaces") return jsonResponse({ workspaces: [workspaceFixture()] });
    if (url === "/api/issues") return jsonResponse({ issues: [issueFixture()] });
    if (url === "/api/issues/issue_0001/start") {
      return jsonResponse({
        issue_id: "issue_0001",
        workspace_id: "workspace_0001",
        task_id: "task_0001",
        session_id: "sess_task_0001",
        status: "started",
      });
    }
    if (url === "/api/projection?task_id=task_0001&workspace_id=workspace_0001") {
      return jsonResponse(projection(null));
    }
    return jsonResponse({});
  }));

  render(<AppShell />);
  await userEvent.click(await screen.findByRole("button", { name: "Start" }));
  await userEvent.selectOptions(screen.getByLabelText("启动 workspace"), "workspace_0001");
  await userEvent.click(screen.getByRole("button", { name: "确认 Start" }));

  expect(await screen.findByRole("main", { name: "Aria workbench" })).toBeInTheDocument();
  expect(screen.getByText("issue_0001")).toBeInTheDocument();
  expect(screen.getByText("workspace_0001")).toBeInTheDocument();
});
```

- [ ] **Step 2: Run failing test**

Run: `pnpm --dir web test -- main`

Expected: FAIL because `AppShell` does not switch views.

- [ ] **Step 3: Parameterize execution calls**

In `app-shell.tsx`, introduce:

```ts
type ExecutionContext = {
  issueId: string;
  workspaceId: string;
  taskId: string;
};
```

Use `executionContext.workspaceId` in all execution API calls.

- [ ] **Step 4: Split view state**

Make `AppShell` render `TaskManagementWorkbench` when `executionContext === null`; render current workbench when `executionContext !== null`. Add a `返回任务管理` button that clears execution context.

- [ ] **Step 5: Load projection after Start**

On execution context creation, call:

```ts
const projection = await getProjection(taskId, undefined, workspaceId);
store.setProjection(projection);
```

- [ ] **Step 6: Update existing tests**

Existing tests that interact with execution workbench should first drive Start or render an extracted `WorkspaceExecutionShell` helper. Prefer driving Start so the behavior matches the product flow.

- [ ] **Step 7: Run frontend tests**

Run: `pnpm --dir web test -- main`

Expected: PASS.

- [ ] **Step 8: Commit**

Run:

```bash
git add web/src/app-shell.tsx web/src/router.tsx web/src/main.test.tsx
git commit -m "feat: open execution workbench from issue start"
```

## Task 8: Full Verification

**Files:**
- Modify tests only if verification reveals a real gap.

- [ ] **Step 1: Run Rust formatting**

Run: `cargo fmt --check`

Expected: PASS.

- [ ] **Step 2: Run Rust checks**

Run: `cargo check --locked`

Expected: PASS.

- [ ] **Step 3: Run Rust tests**

Run: `cargo test --locked -j 1`

Expected: PASS.

- [ ] **Step 4: Run frontend tests**

Run: `pnpm --dir web test`

Expected: PASS.

- [ ] **Step 5: Start dev service**

Run backend:

```bash
cargo watch -w src -w Cargo.toml -w Cargo.lock -x "run --locked -- web --workspace . --host 127.0.0.1 --port 4317"
```

Run frontend:

```bash
pnpm --dir web dev
```

Expected:

- Backend prints `aria web listening on http://127.0.0.1:4317`.
- Vite prints `Local: http://127.0.0.1:5173/`.

- [ ] **Step 6: Verify HTTP health**

Run:

```bash
curl --noproxy '*' -sS http://127.0.0.1:4317/api/health
curl --noproxy '*' -sS http://127.0.0.1:5173/api/health
```

Expected both return `{"status":"ok"}`.

- [ ] **Step 7: Browser smoke test**

Use the in-app browser at `http://127.0.0.1:5173/`:

- Confirm the default view is task management.
- Add the current repo as a workspace.
- Create an issue.
- Start the issue with that workspace.
- Confirm the existing execution workbench appears.

- [ ] **Step 8: Commit verification fixes**

Only if Task 8 required code changes:

```bash
git add src web
git commit -m "test: verify task and workspace workbench"
```

## Self-Review

- Spec coverage: Tasks 1-4 cover local persistence, workspace validation, issue Start, and workspace-aware execution APIs. Tasks 5-7 cover frontend default task management, workspace management, Start selection, and entry into the existing workbench. Task 8 covers verification.
- Placeholder scan: This plan contains concrete file paths, commands, expected outcomes, and code shapes for each implementation slice.
- Type consistency: Backend DTO names use snake_case fields matching frontend types. `workspace_id`, `issue_id`, and `task_id` are consistent across registries, APIs, and UI tests.
