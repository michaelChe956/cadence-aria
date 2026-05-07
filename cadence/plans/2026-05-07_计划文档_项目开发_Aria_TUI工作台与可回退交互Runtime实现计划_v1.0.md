# Aria TUI Workbench Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `aria tui` as a Rust terminal workbench backed by a reusable interactive runtime model with node IO visibility, configurable provider confirmation, and checkpoint-based rollback.

**Architecture:** Add an `interactive` core module for projections, sessions, turns, policy, checkpoints, and execution control. Add a thin `tui` module for terminal state and rendering. Keep `task run --non-interactive` compatible while extracting enough step-runner seams for TUI-controlled execution.

**Tech Stack:** Rust 1.95, serde/serde_json, chrono, tempfile test fixtures, existing ProviderAdapter/runtime store modules, planned Ratatui/Crossterm terminal UI dependencies.

---

## Scope And Sequencing

This is one product feature with several dependent subsystems. Implement it in vertical order:

1. Runtime projection and diagnostics first, so `aria tui` can browse existing tasks.
2. Policy, checkpoint, and controller next, so execution can pause and resume safely.
3. Step runner integration after the controller is tested with fake steps.
4. Terminal rendering last, using stable core APIs.

Do not rewrite current `task run --non-interactive` behavior. Add tests before each implementation change.

Use the project Rust/Docker rule when the host Rust toolchain is not explicitly confirmed:

```bash
docker run --rm -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/work/.cargo-home \
  -e CARGO_NET_RETRY=10 \
  -e CARGO_HTTP_TIMEOUT=120 \
  -v "$PWD":/work -w /work \
  rust:1-bookworm cargo test --locked -j 1
```

When committing from this plan, stage only the files listed in the task. The current workspace may contain unrelated staged or untracked files.

## File Structure

Create these focused modules:

| Path | Responsibility |
|------|----------------|
| `src/interactive/mod.rs` | Public module exports |
| `src/interactive/models.rs` | Serializable workspace/session/turn/node/checkpoint/artifact projection types |
| `src/interactive/store.rs` | Read/write `.aria/runtime/tasks/<task_id>/` interactive index files |
| `src/interactive/diagnostics.rs` | Classify provider/gate/validation/checkpoint diagnostics |
| `src/interactive/projection.rs` | Build `WorkspaceProjection` from runtime store files |
| `src/interactive/policy.rs` | Policy preset and per-node confirmation decisions |
| `src/interactive/checkpoint.rs` | Create and restore runtime/Git checkpoints |
| `src/interactive/controller.rs` | Pending provider step and confirmation/rollback orchestration |
| `src/task_run/step_runner.rs` | Incremental provider-node execution seam for TUI-controlled runs |
| `src/tui/mod.rs` | TUI module exports and entry point |
| `src/tui/state.rs` | UI reducer and view state, independent of terminal backend |
| `src/tui/render.rs` | Ratatui rendering functions |

Modify these existing files:

| Path | Change |
|------|--------|
| `src/lib.rs` | Export `interactive` and `tui` |
| `src/cli.rs` | Parse and route `aria tui` |
| `src/task_run/mod.rs` | Export `step_runner` |
| `Cargo.toml` | Add Ratatui/Crossterm dependencies when TUI rendering task starts |

Add tests:

| Path | Coverage |
|------|----------|
| `tests/interactive_store.rs` | Model serialization and store IO |
| `tests/interactive_projection.rs` | Projection and diagnostics from fixture runtime files |
| `tests/interactive_policy.rs` | Policy preset decisions |
| `tests/interactive_checkpoint.rs` | Checkpoint creation and rollback in temp Git repos |
| `tests/interactive_controller.rs` | Pending approval and rollback orchestration with fake steps |
| `tests/task_run_step_runner.rs` | Step runner compatibility seams |
| `tests/tui_state.rs` | TUI reducer behavior without real terminal |
| `tests/tui_cli.rs` | CLI parsing and browse launch routing |

---

### Task 1: Interactive Models And Store

**Files:**
- Create: `src/interactive/mod.rs`
- Create: `src/interactive/models.rs`
- Create: `src/interactive/store.rs`
- Modify: `src/lib.rs`
- Test: `tests/interactive_store.rs`

- [ ] **Step 1: Write failing model/store tests**

Create `tests/interactive_store.rs`:

```rust
use cadence_aria::interactive::models::{
    ArtifactIndexEntry, ArtifactStatus, ContentType, InteractionTurn, NodeRun,
    NodeRunStatus, TaskSession, TurnStatus, WorkspaceProjection,
};
use cadence_aria::interactive::store::InteractiveStore;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn interactive_store_round_trips_session_turn_node_and_projection() {
    let workspace = tempdir().expect("workspace");
    let store = InteractiveStore::new(workspace.path(), "task_0001");

    let session = TaskSession {
        session_id: "sess_task_0001".to_string(),
        task_id: "task_0001".to_string(),
        created_at: "2026-05-07T00:00:00Z".to_string(),
        status: "idle".to_string(),
        turn_ids: vec!["turn_0001".to_string()],
        active_turn_id: Some("turn_0001".to_string()),
    };
    store.write_session(&session).expect("write session");

    let turn = InteractionTurn {
        turn_id: "turn_0001".to_string(),
        session_id: "sess_task_0001".to_string(),
        node_id: "N16".to_string(),
        provider_type: "codex".to_string(),
        prompt_snapshot: "实现 fibonacciSquareSum".to_string(),
        input_summary: json!({"allowed_write_scope": ["src/", "tests/"]}),
        checkpoint_before: Some("ckpt_0001".to_string()),
        provider_run_id: Some("run_n16_0001".to_string()),
        output_artifact_refs: vec!["coding_report_work_wt_001_0001".to_string()],
        changed_files: vec!["src/fibonacciSquareSum.js".to_string()],
        status: TurnStatus::Completed,
        dropped: false,
        created_at: "2026-05-07T00:00:01Z".to_string(),
        updated_at: "2026-05-07T00:00:02Z".to_string(),
    };
    store.write_turn(&turn).expect("write turn");

    let node = NodeRun {
        node_run_id: "nrun_0001".to_string(),
        node_id: "N16".to_string(),
        turn_id: Some("turn_0001".to_string()),
        provider_run_id: Some("run_n16_0001".to_string()),
        input_refs: vec!["plan_projection_0001".to_string()],
        output_schema: Some("schema://aria/artifacts/coding_report/v1".to_string()),
        artifact_refs: vec!["coding_report_work_wt_001_0001".to_string()],
        status: NodeRunStatus::Completed,
        duration_ms: Some(42),
        diagnostic_refs: Vec::new(),
        dropped: false,
        created_at: "2026-05-07T00:00:01Z".to_string(),
        updated_at: "2026-05-07T00:00:02Z".to_string(),
    };
    store.write_node_run(&node).expect("write node");

    let projection = WorkspaceProjection {
        workspace_root: workspace.path().to_string_lossy().to_string(),
        active_task_id: Some("task_0001".to_string()),
        active_session_id: Some("sess_task_0001".to_string()),
        overview: json!({"phase": "execution", "status": "running"}),
        sessions: vec![session.clone()],
        timeline: vec![json!({"kind": "node", "node_id": "N16"})],
        artifact_index: vec![ArtifactIndexEntry {
            artifact_ref: "coding_report_work_wt_001_0001".to_string(),
            artifact_kind: "coding_report".to_string(),
            producer_node: Some("N16".to_string()),
            path: ".aria/runtime/tasks/task_0001/artifacts/execution/0000.json".to_string(),
            summary: "编码报告".to_string(),
            status: ArtifactStatus::Active,
            content_type: ContentType::Json,
            traceability_refs: Vec::new(),
            dropped: false,
        }],
        diagnostics: Vec::new(),
        available_actions: vec!["rollback_previous_turn".to_string()],
    };
    store.write_projection(&projection).expect("write projection");

    assert_eq!(store.read_session("sess_task_0001").expect("read session"), session);
    assert_eq!(store.read_turn("turn_0001").expect("read turn"), turn);
    assert_eq!(store.read_node_run("nrun_0001").expect("read node"), node);
    assert_eq!(store.read_projection().expect("read projection"), projection);
}
```

- [ ] **Step 2: Run the failing test**

Run:

```bash
cargo test --test interactive_store --locked
```

Expected: FAIL because `cadence_aria::interactive` does not exist.

- [ ] **Step 3: Add module exports**

Modify `src/lib.rs`:

```rust
pub mod cli;
pub mod cross_cutting;
pub mod daemon;
pub mod interactive;
pub mod protocol;
pub mod repl;
pub mod runtime_units;
pub mod task_run;
```

Create `src/interactive/mod.rs`:

```rust
pub mod diagnostics;
pub mod models;
pub mod store;
```

Create empty module files needed by the export:

```rust
// src/interactive/diagnostics.rs
```

- [ ] **Step 4: Add serializable models**

Create `src/interactive/models.rs`:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Dropped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRunStatus {
    Started,
    Completed,
    Failed,
    Blocked,
    Dropped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStatus {
    Active,
    Superseded,
    Candidate,
    Rejected,
    Dropped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    Markdown,
    Json,
    Source,
    Test,
    Log,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskSession {
    pub session_id: String,
    pub task_id: String,
    pub created_at: String,
    pub status: String,
    pub turn_ids: Vec<String>,
    pub active_turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct InteractionTurn {
    pub turn_id: String,
    pub session_id: String,
    pub node_id: String,
    pub provider_type: String,
    pub prompt_snapshot: String,
    pub input_summary: Value,
    pub checkpoint_before: Option<String>,
    pub provider_run_id: Option<String>,
    pub output_artifact_refs: Vec<String>,
    pub changed_files: Vec<String>,
    pub status: TurnStatus,
    pub dropped: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NodeRun {
    pub node_run_id: String,
    pub node_id: String,
    pub turn_id: Option<String>,
    pub provider_run_id: Option<String>,
    pub input_refs: Vec<String>,
    pub output_schema: Option<String>,
    pub artifact_refs: Vec<String>,
    pub status: NodeRunStatus,
    pub duration_ms: Option<u64>,
    pub diagnostic_refs: Vec<String>,
    pub dropped: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeCheckpoint {
    pub checkpoint_id: String,
    pub task_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub git_head: Option<String>,
    pub dirty_summary: Value,
    pub state_snapshot_ref: String,
    pub projection_snapshot_ref: String,
    pub artifact_boundary: usize,
    pub provider_run_boundary: usize,
    pub node_run_boundary: usize,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ArtifactIndexEntry {
    pub artifact_ref: String,
    pub artifact_kind: String,
    pub producer_node: Option<String>,
    pub path: String,
    pub summary: String,
    pub status: ArtifactStatus,
    pub content_type: ContentType,
    pub traceability_refs: Vec<String>,
    pub dropped: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceProjection {
    pub workspace_root: String,
    pub active_task_id: Option<String>,
    pub active_session_id: Option<String>,
    pub overview: Value,
    pub sessions: Vec<TaskSession>,
    pub timeline: Vec<Value>,
    pub artifact_index: Vec<ArtifactIndexEntry>,
    pub diagnostics: Vec<Value>,
    pub available_actions: Vec<String>,
}
```

- [ ] **Step 5: Add store IO**

Create `src/interactive/store.rs`:

```rust
use std::path::{Component, Path, PathBuf};

use serde::{Serialize, de::DeserializeOwned};

use crate::interactive::models::{InteractionTurn, NodeRun, TaskSession, WorkspaceProjection};
use crate::task_run::types::TaskRunError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractiveStore {
    workspace_root: PathBuf,
    task_id: String,
}

impl InteractiveStore {
    pub fn new(workspace_root: &Path, task_id: impl Into<String>) -> Self {
        Self {
            workspace_root: workspace_root.to_path_buf(),
            task_id: task_id.into(),
        }
    }

    pub fn task_root(&self) -> PathBuf {
        self.workspace_root
            .join(".aria/runtime/tasks")
            .join(&self.task_id)
    }

    pub fn write_session(&self, session: &TaskSession) -> Result<PathBuf, TaskRunError> {
        self.write_json(&format!("sessions/{}.json", session.session_id), session)
    }

    pub fn read_session(&self, session_id: &str) -> Result<TaskSession, TaskRunError> {
        validate_runtime_id(session_id)?;
        self.read_json(&format!("sessions/{session_id}.json"))
    }

    pub fn write_turn(&self, turn: &InteractionTurn) -> Result<PathBuf, TaskRunError> {
        self.write_json(&format!("turns/{}.json", turn.turn_id), turn)
    }

    pub fn read_turn(&self, turn_id: &str) -> Result<InteractionTurn, TaskRunError> {
        validate_runtime_id(turn_id)?;
        self.read_json(&format!("turns/{turn_id}.json"))
    }

    pub fn write_node_run(&self, node_run: &NodeRun) -> Result<PathBuf, TaskRunError> {
        self.write_json(&format!("node-runs/{}.json", node_run.node_run_id), node_run)
    }

    pub fn read_node_run(&self, node_run_id: &str) -> Result<NodeRun, TaskRunError> {
        validate_runtime_id(node_run_id)?;
        self.read_json(&format!("node-runs/{node_run_id}.json"))
    }

    pub fn write_projection(&self, projection: &WorkspaceProjection) -> Result<PathBuf, TaskRunError> {
        self.write_json("projection.json", projection)
    }

    pub fn read_projection(&self) -> Result<WorkspaceProjection, TaskRunError> {
        self.read_json("projection.json")
    }

    fn write_json<T: Serialize>(&self, relative_path: &str, value: &T) -> Result<PathBuf, TaskRunError> {
        validate_runtime_relative_path(relative_path)?;
        let path = self.task_root().join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                TaskRunError::new("interactive_store_io", format!("create {}: {error}", parent.display()))
            })?;
        }
        let bytes = serde_json::to_vec_pretty(value)
            .map_err(|error| TaskRunError::new("interactive_store_serialize", error.to_string()))?;
        std::fs::write(&path, bytes).map_err(|error| {
            TaskRunError::new("interactive_store_io", format!("write {}: {error}", path.display()))
        })?;
        Ok(path)
    }

    fn read_json<T: DeserializeOwned>(&self, relative_path: &str) -> Result<T, TaskRunError> {
        validate_runtime_relative_path(relative_path)?;
        let path = self.task_root().join(relative_path);
        let bytes = std::fs::read(&path).map_err(|error| {
            TaskRunError::new("interactive_store_io", format!("read {}: {error}", path.display()))
        })?;
        serde_json::from_slice(&bytes)
            .map_err(|error| TaskRunError::new("interactive_store_serialize", error.to_string()))
    }
}

fn validate_runtime_id(value: &str) -> Result<(), TaskRunError> {
    if value.is_empty()
        || value.contains('/')
        || value.contains('\\')
        || value.contains("..")
    {
        return Err(TaskRunError::new(
            "interactive_store_invalid_id",
            format!("invalid runtime id: {value}"),
        ));
    }
    Ok(())
}

fn validate_runtime_relative_path(relative_path: &str) -> Result<(), TaskRunError> {
    let path = Path::new(relative_path);
    if path.components().any(|component| {
        matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::ParentDir
        )
    }) {
        return Err(TaskRunError::new(
            "interactive_store_path_escape",
            format!("runtime store path escapes task root: {relative_path}"),
        ));
    }
    Ok(())
}
```

- [ ] **Step 6: Run test to verify it passes**

Run:

```bash
cargo test --test interactive_store --locked
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs src/interactive/mod.rs src/interactive/models.rs src/interactive/store.rs tests/interactive_store.rs
git commit -m "feat: add interactive runtime store models"
```

---

### Task 2: Runtime Projection And Artifact Index

**Files:**
- Create: `src/interactive/projection.rs`
- Modify: `src/interactive/mod.rs`
- Test: `tests/interactive_projection.rs`

- [ ] **Step 1: Write failing projection test**

Create `tests/interactive_projection.rs`:

```rust
use cadence_aria::interactive::models::{ArtifactStatus, ContentType};
use cadence_aria::interactive::projection::build_workspace_projection;
use serde_json::json;
use std::fs;
use tempfile::tempdir;

#[test]
fn projection_reads_state_reports_events_provider_runs_and_artifacts() {
    let workspace = tempdir().expect("workspace");
    let task_root = workspace.path().join(".aria/runtime/tasks/task_0001");
    fs::create_dir_all(task_root.join("reports")).expect("reports dir");
    fs::create_dir_all(task_root.join("logs")).expect("logs dir");
    fs::create_dir_all(task_root.join("provider-runs/run_n16_0001")).expect("provider run dir");
    fs::create_dir_all(task_root.join("artifacts/execution")).expect("artifacts dir");

    fs::write(
        task_root.join("state.json"),
        serde_json::to_vec_pretty(&json!({
            "task_id": "task_0001",
            "change_id": "aria-fibonacci-square",
            "phase": "blocked_by_gate",
            "current_worktask": "work_wt_006",
            "openspec_bootstrap_status": "bootstrapped"
        }))
        .expect("state json"),
    )
    .expect("write state");
    fs::write(
        task_root.join("reports/final-report.json"),
        serde_json::to_vec_pretty(&json!({
            "task_id": "task_0001",
            "change_id": "aria-fibonacci-square",
            "status": "blocked_by_gate",
            "blocked_report_path": task_root.join("reports/blocked-report.json")
        }))
        .expect("final json"),
    )
    .expect("write final");
    fs::write(
        task_root.join("reports/blocked-report.json"),
        serde_json::to_vec_pretty(&json!({
            "task_id": "task_0001",
            "status": "blocked_by_gate",
            "reason": "rework_limit_exceeded",
            "next_node": "X08"
        }))
        .expect("blocked json"),
    )
    .expect("write blocked");
    fs::write(
        task_root.join("reports/testing-report.json"),
        serde_json::to_vec_pretty(&json!({
            "artifact_kind": "testing_report",
            "artifact_ref": "testing_report_work_wt_006_0001",
            "tests_passed": false,
            "failures": ["node_contract.allowed_write_scope=[]"]
        }))
        .expect("testing json"),
    )
    .expect("write testing");
    fs::write(
        task_root.join("logs/node-events.jsonl"),
        concat!(
            "{\"event_kind\":\"node_enter\",\"task_id\":\"task_0001\",\"node_id\":\"N16\",\"status\":\"started\",\"details\":{\"provider_run_id\":\"run_n16_0001\",\"output_schema\":\"schema://aria/artifacts/coding_report/v1\"}}\n",
            "{\"event_kind\":\"node_exit\",\"task_id\":\"task_0001\",\"node_id\":\"N16\",\"status\":\"completed\",\"details\":{\"provider_run_id\":\"run_n16_0001\",\"duration_ms\":42}}\n"
        ),
    )
    .expect("write events");
    fs::write(
        task_root.join("provider-runs/run_n16_0001/run.json"),
        serde_json::to_vec_pretty(&json!({
            "provider_run_id": "run_n16_0001",
            "node_id": "N16",
            "provider_type": "codex",
            "status": "completed",
            "duration_ms": 42,
            "files_modified": ["src/fibonacciSquareSum.js", "tests/fibonacciSquareSum.test.js"]
        }))
        .expect("run json"),
    )
    .expect("write provider run");
    fs::write(
        task_root.join("artifacts/execution/0000.json"),
        serde_json::to_vec_pretty(&json!({
            "artifact_kind": "coding_report",
            "artifact_ref": "coding_report_work_wt_006_0001",
            "worktask_id": "work_wt_006",
            "files_modified": ["src/fibonacciSquareSum.js"]
        }))
        .expect("artifact json"),
    )
    .expect("write artifact");

    let projection = build_workspace_projection(workspace.path(), Some("task_0001"))
        .expect("build projection");

    assert_eq!(projection.active_task_id.as_deref(), Some("task_0001"));
    assert_eq!(projection.overview["phase"], "blocked_by_gate");
    assert_eq!(projection.overview["change_id"], "aria-fibonacci-square");
    assert!(projection.timeline.iter().any(|entry| {
        entry["node_id"] == "N16" && entry["status"] == "completed"
    }));
    assert!(projection.artifact_index.iter().any(|entry| {
        entry.artifact_ref == "coding_report_work_wt_006_0001"
            && entry.status == ArtifactStatus::Active
            && entry.content_type == ContentType::Json
    }));
}
```

- [ ] **Step 2: Run the failing test**

Run:

```bash
cargo test --test interactive_projection --locked
```

Expected: FAIL because `interactive::projection` does not exist.

- [ ] **Step 3: Export projection module**

Modify `src/interactive/mod.rs`:

```rust
pub mod diagnostics;
pub mod models;
pub mod projection;
pub mod store;
```

- [ ] **Step 4: Implement projection builder**

Create `src/interactive/projection.rs`:

```rust
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::interactive::models::{
    ArtifactIndexEntry, ArtifactStatus, ContentType, WorkspaceProjection,
};
use crate::task_run::types::TaskRunError;

pub fn build_workspace_projection(
    workspace_root: &Path,
    task_id: Option<&str>,
) -> Result<WorkspaceProjection, TaskRunError> {
    let task_id = task_id
        .map(ToOwned::to_owned)
        .or_else(|| latest_task_id(workspace_root))
        .ok_or_else(|| TaskRunError::new("interactive_task_missing", "no task id available"))?;
    let task_root = workspace_root.join(".aria/runtime/tasks").join(&task_id);
    let state = read_json_optional(&task_root.join("state.json"))?.unwrap_or_else(|| json!({}));
    let final_report =
        read_json_optional(&task_root.join("reports/final-report.json"))?.unwrap_or_else(|| json!({}));
    let timeline = read_node_events(&task_root.join("logs/node-events.jsonl"))?;
    let artifacts = read_artifact_index(&task_root)?;

    let overview = json!({
        "task_id": state.get("task_id").cloned().unwrap_or_else(|| json!(task_id)),
        "change_id": state.get("change_id").cloned().unwrap_or(Value::Null),
        "phase": state.get("phase").cloned().unwrap_or(Value::Null),
        "current_worktask": state.get("current_worktask").cloned().unwrap_or(Value::Null),
        "status": final_report.get("status")
            .cloned()
            .or_else(|| state.get("phase").cloned())
            .unwrap_or(Value::Null),
        "workspace": workspace_root.to_string_lossy(),
    });

    Ok(WorkspaceProjection {
        workspace_root: workspace_root.to_string_lossy().to_string(),
        active_task_id: Some(task_id),
        active_session_id: None,
        overview,
        sessions: Vec::new(),
        timeline,
        artifact_index: artifacts,
        diagnostics: Vec::new(),
        available_actions: vec!["refresh".to_string()],
    })
}

fn latest_task_id(workspace_root: &Path) -> Option<String> {
    let tasks_dir = workspace_root.join(".aria/runtime/tasks");
    let mut entries = std::fs::read_dir(tasks_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    entries.sort();
    entries.pop()
}

fn read_json_optional(path: &Path) -> Result<Option<Value>, TaskRunError> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|error| {
        TaskRunError::new("interactive_projection_io", format!("read {}: {error}", path.display()))
    })?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| TaskRunError::new("interactive_projection_json", error.to_string()))
}

fn read_node_events(path: &Path) -> Result<Vec<Value>, TaskRunError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path).map_err(|error| {
        TaskRunError::new("interactive_projection_io", format!("read {}: {error}", path.display()))
    })?;
    let mut timeline = Vec::new();
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        let event = serde_json::from_str::<Value>(line)
            .map_err(|error| TaskRunError::new("interactive_projection_jsonl", error.to_string()))?;
        let details = event.get("details").cloned().unwrap_or_else(|| json!({}));
        timeline.push(json!({
            "kind": "node",
            "event_kind": event.get("event_kind").cloned().unwrap_or(Value::Null),
            "task_id": event.get("task_id").cloned().unwrap_or(Value::Null),
            "node_id": event.get("node_id").cloned().unwrap_or(Value::Null),
            "status": event.get("status").cloned().unwrap_or(Value::Null),
            "provider_run_id": details.get("provider_run_id").cloned().unwrap_or(Value::Null),
            "duration_ms": details.get("duration_ms").cloned().unwrap_or(Value::Null),
            "output_schema": details.get("output_schema").cloned().unwrap_or(Value::Null),
        }));
    }
    Ok(timeline)
}

fn read_artifact_index(task_root: &Path) -> Result<Vec<ArtifactIndexEntry>, TaskRunError> {
    let mut entries = Vec::new();
    let artifacts_dir = task_root.join("artifacts");
    collect_artifact_entries(task_root, &artifacts_dir, &mut entries)?;
    let reports_dir = task_root.join("reports");
    collect_artifact_entries(task_root, &reports_dir, &mut entries)?;
    Ok(entries)
}

fn collect_artifact_entries(
    task_root: &Path,
    path: &Path,
    entries: &mut Vec<ArtifactIndexEntry>,
) -> Result<(), TaskRunError> {
    if !path.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(path).map_err(|error| {
        TaskRunError::new("interactive_projection_io", format!("read {}: {error}", path.display()))
    })? {
        let entry = entry.map_err(|error| {
            TaskRunError::new("interactive_projection_io", error.to_string())
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_artifact_entries(task_root, &path, entries)?;
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let value = read_json_optional(&path)?.unwrap_or_else(|| json!({}));
        let relative = path
            .strip_prefix(task_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        let artifact_kind = value
            .get("artifact_kind")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| infer_artifact_kind(&path));
        let artifact_ref = value
            .get("artifact_ref")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| relative.clone());
        entries.push(ArtifactIndexEntry {
            artifact_ref,
            artifact_kind,
            producer_node: None,
            path: relative,
            summary: path.file_name().and_then(|name| name.to_str()).unwrap_or("artifact").to_string(),
            status: ArtifactStatus::Active,
            content_type: content_type_for_path(&path),
            traceability_refs: string_array_at(&value, &["_aria", "traceability_refs"]),
            dropped: false,
        });
    }
    Ok(())
}

fn infer_artifact_kind(path: &Path) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("artifact")
        .replace('-', "_")
}

fn content_type_for_path(path: &Path) -> ContentType {
    match path.extension().and_then(|value| value.to_str()) {
        Some("md") => ContentType::Markdown,
        Some("json") => ContentType::Json,
        Some("js") | Some("ts") | Some("rs") => ContentType::Source,
        Some("log") | Some("jsonl") => ContentType::Log,
        _ => ContentType::Unknown,
    }
}

fn string_array_at(value: &Value, path: &[&str]) -> Vec<String> {
    let mut cursor = value;
    for part in path {
        let Some(next) = cursor.get(part) else {
            return Vec::new();
        };
        cursor = next;
    }
    cursor
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}
```

- [ ] **Step 5: Run projection test**

Run:

```bash
cargo test --test interactive_projection --locked
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/interactive/mod.rs src/interactive/projection.rs tests/interactive_projection.rs
git commit -m "feat: build interactive workspace projection"
```

---

### Task 3: Diagnostics Classification

**Files:**
- Modify: `src/interactive/diagnostics.rs`
- Modify: `src/interactive/projection.rs`
- Test: `tests/interactive_projection.rs`

- [ ] **Step 1: Add failing diagnostics assertion**

Append to `tests/interactive_projection.rs`:

```rust
#[test]
fn projection_classifies_write_scope_gate_blocked_fibonacci_shape() {
    let workspace = tempdir().expect("workspace");
    let task_root = workspace.path().join(".aria/runtime/tasks/task_0001");
    fs::create_dir_all(task_root.join("reports")).expect("reports dir");
    fs::write(
        task_root.join("state.json"),
        serde_json::to_vec_pretty(&json!({
            "task_id": "task_0001",
            "change_id": "aria-fibonacci-square",
            "phase": "blocked_by_gate",
            "current_worktask": "work_wt_006"
        }))
        .expect("state json"),
    )
    .expect("write state");
    fs::write(
        task_root.join("reports/blocked-report.json"),
        serde_json::to_vec_pretty(&json!({
            "status": "blocked_by_gate",
            "reason": "rework_limit_exceeded",
            "next_node": "X08"
        }))
        .expect("blocked json"),
    )
    .expect("write blocked");
    fs::write(
        task_root.join("reports/testing-report.json"),
        serde_json::to_vec_pretty(&json!({
            "artifact_kind": "testing_report",
            "tests_passed": false,
            "failures": [
                "未发现归档到 cadence/designs/ 与 cadence/reports/ 的文件。",
                "node_contract.allowed_write_scope=[]，本节点不得写入任何文件。"
            ]
        }))
        .expect("testing json"),
    )
    .expect("write testing");

    let projection = build_workspace_projection(workspace.path(), Some("task_0001"))
        .expect("build projection");

    let diagnostic = projection
        .diagnostics
        .iter()
        .find(|entry| entry["category"] == "contract_write_scope_blocked")
        .expect("write scope diagnostic");
    assert_eq!(diagnostic["severity"], "blocking");
    assert_eq!(diagnostic["reason"], "rework_limit_exceeded");
    assert_eq!(diagnostic["next_node"], "X08");
}
```

- [ ] **Step 2: Run failing diagnostics test**

Run:

```bash
cargo test --test interactive_projection projection_classifies_write_scope_gate_blocked_fibonacci_shape --locked
```

Expected: FAIL because projection diagnostics are empty.

- [ ] **Step 3: Implement diagnostics classifier**

Replace `src/interactive/diagnostics.rs` with:

```rust
use std::path::Path;

use serde_json::{Value, json};

use crate::task_run::types::TaskRunError;

pub fn classify_task_diagnostics(task_root: &Path, state: &Value) -> Result<Vec<Value>, TaskRunError> {
    let blocked = read_json_optional(&task_root.join("reports/blocked-report.json"))?;
    let testing = read_json_optional(&task_root.join("reports/testing-report.json"))?;
    let mut diagnostics = Vec::new();

    if let Some(blocked) = blocked {
        let reason = blocked
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("blocked_by_gate");
        let next_node = blocked.get("next_node").and_then(Value::as_str);
        let testing_text = testing
            .as_ref()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let category = if testing_text.contains("allowed_write_scope=[]")
            || testing_text.contains("cadence/designs")
            || testing_text.contains("cadence/reports")
        {
            "contract_write_scope_blocked"
        } else {
            "gate_blocked"
        };
        diagnostics.push(json!({
            "category": category,
            "severity": "blocking",
            "status": "blocked_by_gate",
            "reason": reason,
            "next_node": next_node,
            "task_id": state.get("task_id").cloned().unwrap_or(Value::Null),
            "current_worktask": state.get("current_worktask").cloned().unwrap_or(Value::Null),
        }));
    }

    Ok(diagnostics)
}

fn read_json_optional(path: &Path) -> Result<Option<Value>, TaskRunError> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|error| {
        TaskRunError::new("interactive_diagnostics_io", format!("read {}: {error}", path.display()))
    })?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| TaskRunError::new("interactive_diagnostics_json", error.to_string()))
}
```

- [ ] **Step 4: Attach diagnostics to projection**

In `src/interactive/projection.rs`, add the import:

```rust
use crate::interactive::diagnostics::classify_task_diagnostics;
```

Before constructing `WorkspaceProjection`, add:

```rust
let diagnostics = classify_task_diagnostics(&task_root, &state)?;
```

Set the projection field:

```rust
diagnostics,
```

- [ ] **Step 5: Run projection tests**

Run:

```bash
cargo test --test interactive_projection --locked
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/interactive/diagnostics.rs src/interactive/projection.rs tests/interactive_projection.rs
git commit -m "feat: classify interactive task diagnostics"
```

---

### Task 4: Policy Preset Decisions

**Files:**
- Create: `src/interactive/policy.rs`
- Modify: `src/interactive/mod.rs`
- Test: `tests/interactive_policy.rs`

- [ ] **Step 1: Write failing policy tests**

Create `tests/interactive_policy.rs`:

```rust
use cadence_aria::interactive::policy::{
    ConfirmationDecision, NodeWriteClass, PolicyPreset, ProviderNodeMeta,
};

#[test]
fn manual_all_pauses_every_provider_node() {
    let meta = ProviderNodeMeta::new("N17", "codex", NodeWriteClass::ReadOnly);
    assert_eq!(
        PolicyPreset::ManualAll.decision_for(&meta),
        ConfirmationDecision::PauseForConfirmation
    );
}

#[test]
fn manual_write_pauses_write_nodes_and_runs_readonly_nodes() {
    let write_node = ProviderNodeMeta::new("N16", "codex", NodeWriteClass::WritesWorkspace);
    let review_node = ProviderNodeMeta::new("N18", "codex", NodeWriteClass::ReadOnly);
    assert_eq!(
        PolicyPreset::ManualWrite.decision_for(&write_node),
        ConfirmationDecision::PauseForConfirmation
    );
    assert_eq!(
        PolicyPreset::ManualWrite.decision_for(&review_node),
        ConfirmationDecision::RunAutomatically
    );
}

#[test]
fn auto_review_pauses_planning_and_coding_but_runs_review_and_testing() {
    let planning = ProviderNodeMeta::new("N11", "claude_code", NodeWriteClass::WritesRuntime);
    let coding = ProviderNodeMeta::new("N16", "codex", NodeWriteClass::WritesWorkspace);
    let testing = ProviderNodeMeta::new("N17", "codex", NodeWriteClass::ReadOnly);
    assert_eq!(
        PolicyPreset::AutoReview.decision_for(&planning),
        ConfirmationDecision::PauseForConfirmation
    );
    assert_eq!(
        PolicyPreset::AutoReview.decision_for(&coding),
        ConfirmationDecision::PauseForConfirmation
    );
    assert_eq!(
        PolicyPreset::AutoReview.decision_for(&testing),
        ConfirmationDecision::RunAutomatically
    );
}

#[test]
fn non_interactive_never_pauses() {
    let write_node = ProviderNodeMeta::new("N16", "codex", NodeWriteClass::WritesWorkspace);
    assert_eq!(
        PolicyPreset::NonInteractive.decision_for(&write_node),
        ConfirmationDecision::RunAutomatically
    );
}
```

- [ ] **Step 2: Run failing policy tests**

Run:

```bash
cargo test --test interactive_policy --locked
```

Expected: FAIL because `interactive::policy` does not exist.

- [ ] **Step 3: Export policy module**

Modify `src/interactive/mod.rs`:

```rust
pub mod diagnostics;
pub mod models;
pub mod policy;
pub mod projection;
pub mod store;
```

- [ ] **Step 4: Implement policy**

Create `src/interactive/policy.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyPreset {
    ManualAll,
    ManualWrite,
    AutoReview,
    NonInteractive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationDecision {
    PauseForConfirmation,
    RunAutomatically,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeWriteClass {
    ReadOnly,
    WritesRuntime,
    WritesWorkspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderNodeMeta {
    pub node_id: String,
    pub provider_type: String,
    pub write_class: NodeWriteClass,
}

impl ProviderNodeMeta {
    pub fn new(
        node_id: impl Into<String>,
        provider_type: impl Into<String>,
        write_class: NodeWriteClass,
    ) -> Self {
        Self {
            node_id: node_id.into(),
            provider_type: provider_type.into(),
            write_class,
        }
    }
}

impl PolicyPreset {
    pub fn decision_for(self, node: &ProviderNodeMeta) -> ConfirmationDecision {
        match self {
            PolicyPreset::ManualAll => ConfirmationDecision::PauseForConfirmation,
            PolicyPreset::ManualWrite => match node.write_class {
                NodeWriteClass::ReadOnly => ConfirmationDecision::RunAutomatically,
                NodeWriteClass::WritesRuntime | NodeWriteClass::WritesWorkspace => {
                    ConfirmationDecision::PauseForConfirmation
                }
            },
            PolicyPreset::AutoReview => {
                if matches!(node.node_id.as_str(), "N11" | "N12" | "N16" | "N19") {
                    ConfirmationDecision::PauseForConfirmation
                } else {
                    ConfirmationDecision::RunAutomatically
                }
            }
            PolicyPreset::NonInteractive => ConfirmationDecision::RunAutomatically,
        }
    }
}

impl std::str::FromStr for PolicyPreset {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "manual-all" => Ok(Self::ManualAll),
            "manual-write" => Ok(Self::ManualWrite),
            "auto-review" => Ok(Self::AutoReview),
            "non-interactive" => Ok(Self::NonInteractive),
            other => Err(format!("unsupported policy preset: {other}")),
        }
    }
}
```

- [ ] **Step 5: Run policy tests**

Run:

```bash
cargo test --test interactive_policy --locked
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/interactive/mod.rs src/interactive/policy.rs tests/interactive_policy.rs
git commit -m "feat: add interactive policy decisions"
```

---

### Task 5: Checkpoint And Rollback Service

**Files:**
- Create: `src/interactive/checkpoint.rs`
- Modify: `src/interactive/mod.rs`
- Test: `tests/interactive_checkpoint.rs`

- [ ] **Step 1: Write failing checkpoint tests**

Create `tests/interactive_checkpoint.rs`:

```rust
use cadence_aria::interactive::checkpoint::{CheckpointService, RollbackRequest};
use cadence_aria::interactive::models::RuntimeCheckpoint;
use serde_json::json;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn rollback_restores_git_head_and_marks_later_history_dropped() {
    let workspace = tempdir().expect("workspace");
    git(workspace.path(), &["init", "-b", "main"]);
    fs::write(workspace.path().join("file.txt"), "before\n").expect("write before");
    git(workspace.path(), &["add", "file.txt"]);
    git(workspace.path(), &["commit", "-m", "before"]);
    let before_head = git_stdout(workspace.path(), &["rev-parse", "HEAD"]);

    let task_root = workspace.path().join(".aria/runtime/tasks/task_0001");
    fs::create_dir_all(task_root.join("turns")).expect("turns");
    fs::create_dir_all(task_root.join("node-runs")).expect("node runs");
    fs::create_dir_all(task_root.join("provider-runs/run_n16_0001")).expect("provider runs");
    fs::write(
        task_root.join("turns/turn_0001.json"),
        serde_json::to_vec_pretty(&json!({"turn_id":"turn_0001","dropped":false}))
            .expect("turn json"),
    )
    .expect("write turn");
    fs::write(
        task_root.join("node-runs/nrun_0001.json"),
        serde_json::to_vec_pretty(&json!({"node_run_id":"nrun_0001","dropped":false}))
            .expect("node json"),
    )
    .expect("write node");
    fs::write(
        task_root.join("provider-runs/run_n16_0001/run.json"),
        serde_json::to_vec_pretty(&json!({"provider_run_id":"run_n16_0001","dropped":false}))
            .expect("run json"),
    )
    .expect("write run");

    fs::write(workspace.path().join("file.txt"), "after\n").expect("write after");
    git(workspace.path(), &["add", "file.txt"]);
    git(workspace.path(), &["commit", "-m", "after"]);

    let service = CheckpointService::new(workspace.path(), "task_0001");
    let checkpoint = RuntimeCheckpoint {
        checkpoint_id: "ckpt_0001".to_string(),
        task_id: "task_0001".to_string(),
        session_id: "sess_task_0001".to_string(),
        turn_id: Some("turn_0001".to_string()),
        git_head: Some(before_head.clone()),
        dirty_summary: json!({"tracked":0,"untracked":0}),
        state_snapshot_ref: "state@ckpt_0001.json".to_string(),
        projection_snapshot_ref: "projection@ckpt_0001.json".to_string(),
        artifact_boundary: 0,
        provider_run_boundary: 0,
        node_run_boundary: 0,
        created_at: "2026-05-07T00:00:00Z".to_string(),
    };
    service.write_checkpoint(&checkpoint).expect("write checkpoint");

    service
        .rollback(RollbackRequest {
            checkpoint_id: "ckpt_0001".to_string(),
            force_when_dirty: true,
        })
        .expect("rollback");

    assert_eq!(git_stdout(workspace.path(), &["rev-parse", "HEAD"]), before_head);
    assert_eq!(fs::read_to_string(workspace.path().join("file.txt")).expect("file"), "before\n");
    assert!(fs::read_to_string(task_root.join("turns/turn_0001.json"))
        .expect("turn")
        .contains("\"dropped\": true"));
    assert!(fs::read_to_string(task_root.join("node-runs/nrun_0001.json"))
        .expect("node")
        .contains("\"dropped\": true"));
    assert!(fs::read_to_string(task_root.join("provider-runs/run_n16_0001/run.json"))
        .expect("run")
        .contains("\"dropped\": true"));
}

fn git(cwd: &std::path::Path, args: &[&str]) {
    let output = Command::new("git").args(args).current_dir(cwd).output().expect("git");
    assert!(
        output.status.success(),
        "git {:?} failed stdout={} stderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(cwd: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git").args(args).current_dir(cwd).output().expect("git");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
```

- [ ] **Step 2: Run failing checkpoint test**

Run:

```bash
cargo test --test interactive_checkpoint --locked
```

Expected: FAIL because `interactive::checkpoint` does not exist.

- [ ] **Step 3: Export checkpoint module**

Modify `src/interactive/mod.rs`:

```rust
pub mod checkpoint;
pub mod diagnostics;
pub mod models;
pub mod policy;
pub mod projection;
pub mod store;
```

- [ ] **Step 4: Implement checkpoint rollback**

Create `src/interactive/checkpoint.rs`:

```rust
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::interactive::models::RuntimeCheckpoint;
use crate::task_run::types::TaskRunError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackRequest {
    pub checkpoint_id: String,
    pub force_when_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointService {
    workspace_root: PathBuf,
    task_id: String,
}

impl CheckpointService {
    pub fn new(workspace_root: &Path, task_id: impl Into<String>) -> Self {
        Self {
            workspace_root: workspace_root.to_path_buf(),
            task_id: task_id.into(),
        }
    }

    pub fn write_checkpoint(&self, checkpoint: &RuntimeCheckpoint) -> Result<PathBuf, TaskRunError> {
        let path = self
            .task_root()
            .join("checkpoints")
            .join(format!("{}.json", checkpoint.checkpoint_id));
        write_json(&path, checkpoint)?;
        Ok(path)
    }

    pub fn read_checkpoint(&self, checkpoint_id: &str) -> Result<RuntimeCheckpoint, TaskRunError> {
        if checkpoint_id.contains('/') || checkpoint_id.contains("..") {
            return Err(TaskRunError::new("checkpoint_invalid_id", checkpoint_id));
        }
        let path = self
            .task_root()
            .join("checkpoints")
            .join(format!("{checkpoint_id}.json"));
        let bytes = std::fs::read(&path).map_err(|error| {
            TaskRunError::new("checkpoint_io", format!("read {}: {error}", path.display()))
        })?;
        serde_json::from_slice(&bytes)
            .map_err(|error| TaskRunError::new("checkpoint_json", error.to_string()))
    }

    pub fn rollback(&self, request: RollbackRequest) -> Result<(), TaskRunError> {
        let checkpoint = self.read_checkpoint(&request.checkpoint_id)?;
        if !request.force_when_dirty && worktree_dirty(&self.workspace_root)? {
            return Err(TaskRunError::new(
                "checkpoint_unsafe_dirty_worktree",
                "worktree has uncommitted changes; force_when_dirty is required",
            ));
        }
        if let Some(git_head) = checkpoint.git_head.as_deref() {
            git(&self.workspace_root, &["reset", "--hard", git_head])?;
        }
        mark_json_files_dropped(&self.task_root().join("turns"))?;
        mark_json_files_dropped(&self.task_root().join("node-runs"))?;
        mark_provider_runs_dropped(&self.task_root().join("provider-runs"))?;
        Ok(())
    }

    fn task_root(&self) -> PathBuf {
        self.workspace_root
            .join(".aria/runtime/tasks")
            .join(&self.task_id)
    }
}

fn worktree_dirty(workspace_root: &Path) -> Result<bool, TaskRunError> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(workspace_root)
        .output()
        .map_err(|error| TaskRunError::new("git_command_failed", error.to_string()))?;
    if !output.status.success() {
        return Err(TaskRunError::new(
            "git_command_failed",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(!output.stdout.is_empty())
}

fn git(workspace_root: &Path, args: &[&str]) -> Result<(), TaskRunError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .map_err(|error| TaskRunError::new("git_command_failed", error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(TaskRunError::new(
            "git_command_failed",
            format!(
                "git {:?}: {}",
                args,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ))
    }
}

fn mark_json_files_dropped(dir: &Path) -> Result<(), TaskRunError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).map_err(|error| {
        TaskRunError::new("checkpoint_io", format!("read {}: {error}", dir.display()))
    })? {
        let path = entry
            .map_err(|error| TaskRunError::new("checkpoint_io", error.to_string()))?
            .path();
        if path.extension().and_then(|value| value.to_str()) == Some("json") {
            mark_json_file_dropped(&path)?;
        }
    }
    Ok(())
}

fn mark_provider_runs_dropped(dir: &Path) -> Result<(), TaskRunError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).map_err(|error| {
        TaskRunError::new("checkpoint_io", format!("read {}: {error}", dir.display()))
    })? {
        let run_dir = entry
            .map_err(|error| TaskRunError::new("checkpoint_io", error.to_string()))?
            .path();
        let run_path = run_dir.join("run.json");
        if run_path.exists() {
            mark_json_file_dropped(&run_path)?;
        }
    }
    Ok(())
}

fn mark_json_file_dropped(path: &Path) -> Result<(), TaskRunError> {
    let bytes = std::fs::read(path).map_err(|error| {
        TaskRunError::new("checkpoint_io", format!("read {}: {error}", path.display()))
    })?;
    let mut value = serde_json::from_slice::<Value>(&bytes)
        .map_err(|error| TaskRunError::new("checkpoint_json", error.to_string()))?;
    value["dropped"] = Value::Bool(true);
    write_json(path, &value)
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), TaskRunError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            TaskRunError::new("checkpoint_io", format!("create {}: {error}", parent.display()))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| TaskRunError::new("checkpoint_json", error.to_string()))?;
    std::fs::write(path, bytes).map_err(|error| {
        TaskRunError::new("checkpoint_io", format!("write {}: {error}", path.display()))
    })
}
```

- [ ] **Step 5: Run checkpoint test**

Run:

```bash
cargo test --test interactive_checkpoint --locked
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/interactive/mod.rs src/interactive/checkpoint.rs tests/interactive_checkpoint.rs
git commit -m "feat: add interactive checkpoint rollback"
```

---

### Task 6: Interactive Controller With Pending Approval

**Files:**
- Create: `src/interactive/controller.rs`
- Modify: `src/interactive/mod.rs`
- Test: `tests/interactive_controller.rs`

- [ ] **Step 1: Write failing controller tests**

Create `tests/interactive_controller.rs`:

```rust
use cadence_aria::interactive::controller::{
    InteractiveController, PendingProviderStep, StepRunner, StepRunnerResult,
};
use cadence_aria::interactive::policy::{NodeWriteClass, PolicyPreset, ProviderNodeMeta};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn controller_pauses_before_manual_write_provider_step() {
    let workspace = tempdir().expect("workspace");
    let runner = FakeRunner {
        next: Some(PendingProviderStep {
            node_id: "N16".to_string(),
            provider_type: "codex".to_string(),
            prompt: "实现功能".to_string(),
            input_summary: json!({"allowed_write_scope":["src/"]}),
            output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
            write_class: NodeWriteClass::WritesWorkspace,
        }),
    };
    let mut controller = InteractiveController::new(
        workspace.path().to_path_buf(),
        "task_0001".to_string(),
        PolicyPreset::ManualWrite,
        runner,
    );

    let result = controller.advance().expect("advance");
    assert!(matches!(result, StepRunnerResult::PausedForApproval(_)));
    let pending = controller.pending_step().expect("pending step");
    assert_eq!(pending.node_id, "N16");
    assert_eq!(pending.provider_type, "codex");
}

#[test]
fn controller_runs_readonly_step_automatically_under_manual_write() {
    let workspace = tempdir().expect("workspace");
    let runner = FakeRunner {
        next: Some(PendingProviderStep {
            node_id: "N17".to_string(),
            provider_type: "codex".to_string(),
            prompt: "运行测试".to_string(),
            input_summary: json!({}),
            output_schema: "schema://aria/artifacts/testing_report/v1".to_string(),
            write_class: NodeWriteClass::ReadOnly,
        }),
    };
    let mut controller = InteractiveController::new(
        workspace.path().to_path_buf(),
        "task_0001".to_string(),
        PolicyPreset::ManualWrite,
        runner,
    );

    let result = controller.advance().expect("advance");
    assert!(matches!(result, StepRunnerResult::CompletedStep { .. }));
    assert!(controller.pending_step().is_none());
}

struct FakeRunner {
    next: Option<PendingProviderStep>,
}

impl StepRunner for FakeRunner {
    fn next_provider_step(&mut self) -> Result<Option<PendingProviderStep>, cadence_aria::task_run::types::TaskRunError> {
        Ok(self.next.clone())
    }

    fn run_provider_step(
        &mut self,
        step: PendingProviderStep,
        prompt: String,
    ) -> Result<StepRunnerResult, cadence_aria::task_run::types::TaskRunError> {
        Ok(StepRunnerResult::CompletedStep {
            node_id: step.node_id,
            provider_run_id: "run_fake_0001".to_string(),
            prompt,
        })
    }
}

impl PendingProviderStep {
    fn meta(&self) -> ProviderNodeMeta {
        ProviderNodeMeta::new(&self.node_id, &self.provider_type, self.write_class)
    }
}
```

- [ ] **Step 2: Run failing controller tests**

Run:

```bash
cargo test --test interactive_controller --locked
```

Expected: FAIL because `interactive::controller` does not exist.

- [ ] **Step 3: Export controller module**

Modify `src/interactive/mod.rs`:

```rust
pub mod checkpoint;
pub mod controller;
pub mod diagnostics;
pub mod models;
pub mod policy;
pub mod projection;
pub mod store;
```

- [ ] **Step 4: Implement controller skeleton**

Create `src/interactive/controller.rs`:

```rust
use std::path::PathBuf;

use serde_json::Value;

use crate::interactive::policy::{
    ConfirmationDecision, NodeWriteClass, PolicyPreset, ProviderNodeMeta,
};
use crate::task_run::types::TaskRunError;

#[derive(Debug, Clone, PartialEq)]
pub struct PendingProviderStep {
    pub node_id: String,
    pub provider_type: String,
    pub prompt: String,
    pub input_summary: Value,
    pub output_schema: String,
    pub write_class: NodeWriteClass,
}

impl PendingProviderStep {
    pub fn meta(&self) -> ProviderNodeMeta {
        ProviderNodeMeta::new(&self.node_id, &self.provider_type, self.write_class)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepRunnerResult {
    PausedForApproval(String),
    CompletedStep {
        node_id: String,
        provider_run_id: String,
        prompt: String,
    },
    NoMoreSteps,
}

pub trait StepRunner {
    fn next_provider_step(&mut self) -> Result<Option<PendingProviderStep>, TaskRunError>;

    fn run_provider_step(
        &mut self,
        step: PendingProviderStep,
        prompt: String,
    ) -> Result<StepRunnerResult, TaskRunError>;
}

pub struct InteractiveController<R: StepRunner> {
    workspace_root: PathBuf,
    task_id: String,
    policy: PolicyPreset,
    runner: R,
    pending_step: Option<PendingProviderStep>,
}

impl<R: StepRunner> InteractiveController<R> {
    pub fn new(
        workspace_root: PathBuf,
        task_id: String,
        policy: PolicyPreset,
        runner: R,
    ) -> Self {
        Self {
            workspace_root,
            task_id,
            policy,
            runner,
            pending_step: None,
        }
    }

    pub fn advance(&mut self) -> Result<StepRunnerResult, TaskRunError> {
        let Some(step) = self.runner.next_provider_step()? else {
            return Ok(StepRunnerResult::NoMoreSteps);
        };
        match self.policy.decision_for(&step.meta()) {
            ConfirmationDecision::PauseForConfirmation => {
                self.pending_step = Some(step.clone());
                Ok(StepRunnerResult::PausedForApproval(step.node_id))
            }
            ConfirmationDecision::RunAutomatically => {
                self.runner.run_provider_step(step.clone(), step.prompt.clone())
            }
        }
    }

    pub fn confirm_pending(&mut self, prompt: String) -> Result<StepRunnerResult, TaskRunError> {
        let Some(step) = self.pending_step.take() else {
            return Err(TaskRunError::new(
                "interactive_no_pending_step",
                "no pending provider step to confirm",
            ));
        };
        self.runner.run_provider_step(step, prompt)
    }

    pub fn pending_step(&self) -> Option<&PendingProviderStep> {
        self.pending_step.as_ref()
    }

    pub fn workspace_root(&self) -> &std::path::Path {
        &self.workspace_root
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }
}
```

- [ ] **Step 5: Remove duplicate test helper method if compiler reports duplicate method**

If `impl PendingProviderStep { fn meta }` in the test conflicts with production code, delete that helper from `tests/interactive_controller.rs`. The production method is the intended implementation.

- [ ] **Step 6: Run controller tests**

Run:

```bash
cargo test --test interactive_controller --locked
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/interactive/mod.rs src/interactive/controller.rs tests/interactive_controller.rs
git commit -m "feat: add interactive execution controller"
```

---

### Task 7: Task Run Step Runner Seam

**Files:**
- Create: `src/task_run/step_runner.rs`
- Modify: `src/task_run/mod.rs`
- Test: `tests/task_run_step_runner.rs`

- [ ] **Step 1: Write failing step runner tests**

Create `tests/task_run_step_runner.rs`:

```rust
use cadence_aria::interactive::controller::{PendingProviderStep, StepRunner, StepRunnerResult};
use cadence_aria::interactive::policy::NodeWriteClass;
use cadence_aria::task_run::step_runner::{ScriptedStepRunner, StepScriptItem};
use serde_json::json;

#[test]
fn scripted_step_runner_exposes_provider_steps_in_order() {
    let mut runner = ScriptedStepRunner::new(vec![
        StepScriptItem::Provider(PendingProviderStep {
            node_id: "N16".to_string(),
            provider_type: "codex".to_string(),
            prompt: "编码".to_string(),
            input_summary: json!({"worktask_id":"work_wt_001"}),
            output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
            write_class: NodeWriteClass::WritesWorkspace,
        }),
        StepScriptItem::Provider(PendingProviderStep {
            node_id: "N17".to_string(),
            provider_type: "codex".to_string(),
            prompt: "测试".to_string(),
            input_summary: json!({}),
            output_schema: "schema://aria/artifacts/testing_report/v1".to_string(),
            write_class: NodeWriteClass::ReadOnly,
        }),
    ]);

    assert_eq!(runner.next_provider_step().expect("first").expect("step").node_id, "N16");
    assert!(matches!(
        runner.run_provider_step(
            PendingProviderStep {
                node_id: "N16".to_string(),
                provider_type: "codex".to_string(),
                prompt: "编码".to_string(),
                input_summary: json!({}),
                output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
                write_class: NodeWriteClass::WritesWorkspace,
            },
            "确认后的编码 prompt".to_string()
        ).expect("run"),
        StepRunnerResult::CompletedStep { .. }
    ));
    assert_eq!(runner.next_provider_step().expect("second").expect("step").node_id, "N17");
}
```

- [ ] **Step 2: Run failing step runner test**

Run:

```bash
cargo test --test task_run_step_runner --locked
```

Expected: FAIL because `task_run::step_runner` does not exist.

- [ ] **Step 3: Export step runner module**

Modify `src/task_run/mod.rs`:

```rust
pub mod command;
pub mod openspec_bootstrap;
pub mod orchestrator;
pub mod provider_factory;
pub mod step_runner;
pub mod store;
pub mod types;
```

- [ ] **Step 4: Implement scripted seam**

Create `src/task_run/step_runner.rs`:

```rust
use std::collections::VecDeque;

use crate::interactive::controller::{PendingProviderStep, StepRunner, StepRunnerResult};
use crate::task_run::types::TaskRunError;

#[derive(Debug, Clone, PartialEq)]
pub enum StepScriptItem {
    Provider(PendingProviderStep),
}

pub struct ScriptedStepRunner {
    queue: VecDeque<StepScriptItem>,
    last_peeked: Option<PendingProviderStep>,
}

impl ScriptedStepRunner {
    pub fn new(items: Vec<StepScriptItem>) -> Self {
        Self {
            queue: items.into(),
            last_peeked: None,
        }
    }
}

impl StepRunner for ScriptedStepRunner {
    fn next_provider_step(&mut self) -> Result<Option<PendingProviderStep>, TaskRunError> {
        match self.queue.front() {
            Some(StepScriptItem::Provider(step)) => {
                self.last_peeked = Some(step.clone());
                Ok(Some(step.clone()))
            }
            None => Ok(None),
        }
    }

    fn run_provider_step(
        &mut self,
        step: PendingProviderStep,
        prompt: String,
    ) -> Result<StepRunnerResult, TaskRunError> {
        let _ = self.queue.pop_front();
        self.last_peeked = None;
        Ok(StepRunnerResult::CompletedStep {
            node_id: step.node_id,
            provider_run_id: "scripted_provider_run".to_string(),
            prompt,
        })
    }
}
```

- [ ] **Step 5: Run step runner tests**

Run:

```bash
cargo test --test task_run_step_runner --locked
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/task_run/mod.rs src/task_run/step_runner.rs tests/task_run_step_runner.rs
git commit -m "feat: add task run step runner seam"
```

---

### Task 8: TUI State Reducer

**Files:**
- Create: `src/tui/mod.rs`
- Create: `src/tui/state.rs`
- Modify: `src/lib.rs`
- Test: `tests/tui_state.rs`

- [ ] **Step 1: Write failing TUI reducer tests**

Create `tests/tui_state.rs`:

```rust
use cadence_aria::tui::state::{ActionInputMode, TuiAction, TuiState, TuiTab};

#[test]
fn tui_state_switches_tabs_and_selects_timeline_entries() {
    let mut state = TuiState::default();
    state.apply(TuiAction::SwitchTab(TuiTab::Timeline));
    state.apply(TuiAction::SelectTimelineIndex(3));

    assert_eq!(state.active_tab, TuiTab::Timeline);
    assert_eq!(state.selected_timeline_index, Some(3));
}

#[test]
fn tui_state_edits_action_input_for_pending_provider_step() {
    let mut state = TuiState::default();
    state.apply(TuiAction::SetActionInputMode(ActionInputMode::ProviderPrompt));
    state.apply(TuiAction::ReplaceActionInput("执行 N16".to_string()));
    state.apply(TuiAction::AppendActionInput("\n补充：只改 src/ 和 tests/".to_string()));

    assert_eq!(state.action_input_mode, ActionInputMode::ProviderPrompt);
    assert!(state.action_input.contains("执行 N16"));
    assert!(state.action_input.contains("只改 src/ 和 tests/"));
}

#[test]
fn tui_state_opens_and_closes_rollback_confirmation() {
    let mut state = TuiState::default();
    state.apply(TuiAction::OpenRollbackConfirmation("ckpt_0001".to_string()));
    assert_eq!(state.pending_rollback_checkpoint.as_deref(), Some("ckpt_0001"));
    state.apply(TuiAction::CloseRollbackConfirmation);
    assert!(state.pending_rollback_checkpoint.is_none());
}
```

- [ ] **Step 2: Run failing TUI state tests**

Run:

```bash
cargo test --test tui_state --locked
```

Expected: FAIL because `cadence_aria::tui` does not exist.

- [ ] **Step 3: Export TUI module**

Modify `src/lib.rs`:

```rust
pub mod cli;
pub mod cross_cutting;
pub mod daemon;
pub mod interactive;
pub mod protocol;
pub mod repl;
pub mod runtime_units;
pub mod task_run;
pub mod tui;
```

Create `src/tui/mod.rs`:

```rust
pub mod state;
```

- [ ] **Step 4: Implement state reducer**

Create `src/tui/state.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TuiTab {
    #[default]
    Overview,
    Timeline,
    Io,
    Artifacts,
    Changes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActionInputMode {
    #[default]
    Idle,
    ProviderPrompt,
    RollbackConfirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiState {
    pub active_tab: TuiTab,
    pub selected_timeline_index: Option<usize>,
    pub action_input: String,
    pub action_input_mode: ActionInputMode,
    pub pending_rollback_checkpoint: Option<String>,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            active_tab: TuiTab::Overview,
            selected_timeline_index: None,
            action_input: String::new(),
            action_input_mode: ActionInputMode::Idle,
            pending_rollback_checkpoint: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAction {
    SwitchTab(TuiTab),
    SelectTimelineIndex(usize),
    SetActionInputMode(ActionInputMode),
    ReplaceActionInput(String),
    AppendActionInput(String),
    OpenRollbackConfirmation(String),
    CloseRollbackConfirmation,
}

impl TuiState {
    pub fn apply(&mut self, action: TuiAction) {
        match action {
            TuiAction::SwitchTab(tab) => self.active_tab = tab,
            TuiAction::SelectTimelineIndex(index) => self.selected_timeline_index = Some(index),
            TuiAction::SetActionInputMode(mode) => self.action_input_mode = mode,
            TuiAction::ReplaceActionInput(value) => self.action_input = value,
            TuiAction::AppendActionInput(value) => self.action_input.push_str(&value),
            TuiAction::OpenRollbackConfirmation(checkpoint_id) => {
                self.action_input_mode = ActionInputMode::RollbackConfirm;
                self.pending_rollback_checkpoint = Some(checkpoint_id);
            }
            TuiAction::CloseRollbackConfirmation => {
                self.action_input_mode = ActionInputMode::Idle;
                self.pending_rollback_checkpoint = None;
            }
        }
    }
}
```

- [ ] **Step 5: Run TUI state tests**

Run:

```bash
cargo test --test tui_state --locked
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/tui/mod.rs src/tui/state.rs tests/tui_state.rs
git commit -m "feat: add tui state reducer"
```

---

### Task 9: CLI Route For `aria tui`

**Files:**
- Modify: `src/cli.rs`
- Test: `tests/tui_cli.rs`

- [ ] **Step 1: Write failing CLI tests**

Create `tests/tui_cli.rs`:

```rust
use cadence_aria::cli::{CliOutput, run_cli};
use tempfile::tempdir;

#[test]
fn cli_routes_tui_browse_with_workspace_and_task_id() {
    let workspace = tempdir().expect("workspace");
    let output = run_cli([
        "tui",
        "--workspace",
        workspace.path().to_str().expect("workspace path"),
        "--task-id",
        "task_0001",
    ])
    .expect("tui route");

    assert_eq!(
        output,
        CliOutput::Text(format!(
            "tui_browse:{}:task_0001",
            workspace.path().to_string_lossy()
        ))
    );
}

#[test]
fn cli_rejects_tui_task_id_without_value() {
    let error = run_cli(["tui", "--task-id"]).expect_err("missing value");
    assert_eq!(error.code, "invalid_cli_args");
    assert!(error.message.contains("--task-id"));
}
```

- [ ] **Step 2: Run failing CLI tests**

Run:

```bash
cargo test --test tui_cli --locked
```

Expected: FAIL because `tui` command is not accepted.

- [ ] **Step 3: Add TUI route parsing**

Modify `src/cli.rs`.

Add this arm before the final invalid-argument arm in `run_cli`:

```rust
[command, rest @ ..] if command == "tui" => {
    let workspace = parse_workspace(rest)?;
    let task_id = parse_task_id(rest)?;
    Ok(CliOutput::Text(match task_id {
        Some(task_id) => format!("tui_browse:{}:{task_id}", workspace.to_string_lossy()),
        None => format!("tui_browse:{}", workspace.to_string_lossy()),
    }))
}
```

Add this helper near `parse_socket`:

```rust
fn parse_task_id(args: &[String]) -> Result<Option<String>, CliError> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--task-id" {
            let value = args.get(index + 1).ok_or_else(|| CliError {
                code: "invalid_cli_args".to_string(),
                message: "--task-id requires a value".to_string(),
            })?;
            return Ok(Some(value.clone()));
        }
        index += 1;
    }
    Ok(None)
}
```

Update invalid args message to include `tui`:

```rust
message: "expected daemon status, repl, task run, or tui command".to_string(),
```

- [ ] **Step 4: Run CLI tests**

Run:

```bash
cargo test --test tui_cli --locked
```

Expected: PASS.

- [ ] **Step 5: Run existing CLI tests**

Run:

```bash
cargo test --test cli_entry --test task_run_command --locked
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/cli.rs tests/tui_cli.rs
git commit -m "feat: add tui cli route"
```

---

### Task 10: TUI Rendering Shell

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/tui/mod.rs`
- Create: `src/tui/render.rs`
- Test: `tests/tui_state.rs`

- [ ] **Step 1: Add failing render smoke test**

Append to `tests/tui_state.rs`:

```rust
use cadence_aria::tui::render::render_workspace_frame;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

#[test]
fn render_workspace_frame_draws_without_panicking() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let state = TuiState::default();
    terminal
        .draw(|frame| render_workspace_frame(frame, &state, None))
        .expect("draw frame");
}
```

- [ ] **Step 2: Run failing render test**

Run:

```bash
cargo test --test tui_state render_workspace_frame_draws_without_panicking --locked
```

Expected: FAIL because Ratatui dependency and `tui::render` do not exist.

- [ ] **Step 3: Add dependencies**

Modify `Cargo.toml` dependencies:

```toml
crossterm = "0.28"
ratatui = "0.29"
```

If Cargo resolves a newer compatible patch version, commit the resulting `Cargo.lock`.

- [ ] **Step 4: Export render module**

Modify `src/tui/mod.rs`:

```rust
pub mod render;
pub mod state;
```

- [ ] **Step 5: Implement minimal frame render**

Create `src/tui/render.rs`:

```rust
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Tabs};

use crate::interactive::models::WorkspaceProjection;
use crate::tui::state::{TuiState, TuiTab};

pub fn render_workspace_frame(
    frame: &mut Frame<'_>,
    state: &TuiState,
    projection: Option<&WorkspaceProjection>,
) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(5),
        ])
        .split(frame.area());

    let tabs = Tabs::new(vec!["Overview", "Timeline", "IO", "Artifacts", "Changes"])
        .select(tab_index(state.active_tab))
        .block(Block::default().title("Aria TUI").borders(Borders::ALL));
    frame.render_widget(tabs, root[0]);

    let body_text = match projection {
        Some(projection) => format!(
            "workspace: {}\ntask: {}\nstatus: {}",
            projection.workspace_root,
            projection.active_task_id.as_deref().unwrap_or("none"),
            projection.overview.get("status").and_then(serde_json::Value::as_str).unwrap_or("unknown")
        ),
        None => "No workspace projection loaded".to_string(),
    };
    frame.render_widget(
        Paragraph::new(body_text).block(Block::default().title("Workbench").borders(Borders::ALL)),
        root[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(state.action_input.as_str()))
            .block(Block::default().title("Action").borders(Borders::ALL)),
        root[2],
    );
}

fn tab_index(tab: TuiTab) -> usize {
    match tab {
        TuiTab::Overview => 0,
        TuiTab::Timeline => 1,
        TuiTab::Io => 2,
        TuiTab::Artifacts => 3,
        TuiTab::Changes => 4,
    }
}
```

- [ ] **Step 6: Run TUI state/render tests**

Run:

```bash
cargo test --test tui_state --locked
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/tui/mod.rs src/tui/render.rs tests/tui_state.rs
git commit -m "feat: add tui rendering shell"
```

---

### Task 11: Browse Mode Projection Wiring

**Files:**
- Create: `src/tui/app.rs`
- Modify: `src/tui/mod.rs`
- Modify: `src/cli.rs`
- Test: `tests/tui_cli.rs`

- [ ] **Step 1: Add failing browse route test**

Append to `tests/tui_cli.rs`:

```rust
#[test]
fn cli_tui_browse_fails_cleanly_when_task_is_missing() {
    let workspace = tempdir().expect("workspace");
    let error = run_cli([
        "tui",
        "--workspace",
        workspace.path().to_str().expect("workspace path"),
        "--task-id",
        "missing_task",
        "--check",
    ])
    .expect_err("missing task");

    assert_eq!(error.code, "interactive_task_missing");
}
```

- [ ] **Step 2: Run failing browse test**

Run:

```bash
cargo test --test tui_cli cli_tui_browse_fails_cleanly_when_task_is_missing --locked
```

Expected: FAIL because `--check` is not parsed and browse mode is not wired.

- [ ] **Step 3: Add TUI app check function**

Create `src/tui/app.rs`:

```rust
use std::path::Path;

use crate::interactive::projection::build_workspace_projection;
use crate::task_run::types::TaskRunError;

pub fn check_tui_browse(workspace: &Path, task_id: Option<&str>) -> Result<(), TaskRunError> {
    let projection = build_workspace_projection(workspace, task_id)?;
    let Some(active_task_id) = projection.active_task_id.as_deref() else {
        return Err(TaskRunError::new("interactive_task_missing", "no active task"));
    };
    let task_root = workspace.join(".aria/runtime/tasks").join(active_task_id);
    if !task_root.exists() {
        return Err(TaskRunError::new(
            "interactive_task_missing",
            format!("task does not exist: {active_task_id}"),
        ));
    }
    Ok(())
}
```

Modify `src/tui/mod.rs`:

```rust
pub mod app;
pub mod render;
pub mod state;
```

- [ ] **Step 4: Route `--check` through browse validation**

Modify the `tui` arm in `src/cli.rs`:

```rust
[command, rest @ ..] if command == "tui" => {
    let workspace = parse_workspace(rest)?;
    let task_id = parse_task_id(rest)?;
    if rest.iter().any(|item| item == "--check") {
        crate::tui::app::check_tui_browse(&workspace, task_id.as_deref())
            .map_err(task_run_error)?;
    }
    Ok(CliOutput::Text(match task_id {
        Some(task_id) => format!("tui_browse:{}:{task_id}", workspace.to_string_lossy()),
        None => format!("tui_browse:{}", workspace.to_string_lossy()),
    }))
}
```

- [ ] **Step 5: Run TUI CLI tests**

Run:

```bash
cargo test --test tui_cli --locked
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/tui/mod.rs src/tui/app.rs src/cli.rs tests/tui_cli.rs
git commit -m "feat: wire tui browse projection check"
```

---

### Task 12: Step Runner Integration With Existing Provider Inputs

**Files:**
- Modify: `src/task_run/step_runner.rs`
- Modify: `src/runtime_units/clarification.rs`
- Modify: `src/runtime_units/coding.rs`
- Test: `tests/task_run_step_runner.rs`
- Test: `tests/task_run_orchestrator.rs`

- [ ] **Step 1: Add failing metadata extraction test**

Append to `tests/task_run_step_runner.rs`:

```rust
use cadence_aria::task_run::step_runner::provider_step_from_adapter_input;
use cadence_aria::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};

#[test]
fn provider_step_from_adapter_input_maps_node_write_class_and_schema() {
    let input = AdapterInput {
        provider_type: ProviderType::Codex,
        role: AdapterRole::Executor,
        prompt: "prompt body".to_string(),
        worktree_path: Some("/tmp/worktree".to_string()),
        context_files: vec!["src/lib.rs".to_string()],
        output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
        timeout: 30,
        max_retries: 1,
    };

    let step = provider_step_from_adapter_input("N16", &input).expect("provider step");
    assert_eq!(step.node_id, "N16");
    assert_eq!(step.provider_type, "codex");
    assert_eq!(step.output_schema, "schema://aria/artifacts/coding_report/v1");
    assert_eq!(step.write_class, NodeWriteClass::WritesWorkspace);
}
```

- [ ] **Step 2: Run failing metadata test**

Run:

```bash
cargo test --test task_run_step_runner provider_step_from_adapter_input_maps_node_write_class_and_schema --locked
```

Expected: FAIL because `provider_step_from_adapter_input` does not exist.

- [ ] **Step 3: Add adapter input metadata mapping**

Append to `src/task_run/step_runner.rs`:

```rust
use serde_json::json;

use crate::interactive::policy::NodeWriteClass;
use crate::protocol::contracts::{AdapterInput, ProviderType};

pub fn provider_step_from_adapter_input(
    node_id: &str,
    input: &AdapterInput,
) -> Result<PendingProviderStep, TaskRunError> {
    Ok(PendingProviderStep {
        node_id: node_id.to_string(),
        provider_type: provider_type_text(&input.provider_type).to_string(),
        prompt: input.prompt.clone(),
        input_summary: json!({
            "worktree_path": input.worktree_path,
            "context_files": input.context_files,
            "timeout": input.timeout,
            "max_retries": input.max_retries,
        }),
        output_schema: input.output_schema.clone(),
        write_class: write_class_for_node(node_id),
    })
}

fn provider_type_text(provider_type: &ProviderType) -> &'static str {
    match provider_type {
        ProviderType::ClaudeCode => "claude_code",
        ProviderType::Codex => "codex",
        ProviderType::Fake => "fake",
    }
}

fn write_class_for_node(node_id: &str) -> NodeWriteClass {
    match node_id {
        "N16" | "N19" => NodeWriteClass::WritesWorkspace,
        "N04" | "N05" | "N07" | "N09" | "N10" | "N11" | "N12" | "N25" | "N26" | "N27" => {
            NodeWriteClass::WritesRuntime
        }
        _ => NodeWriteClass::ReadOnly,
    }
}
```

- [ ] **Step 4: Run metadata test**

Run:

```bash
cargo test --test task_run_step_runner provider_step_from_adapter_input_maps_node_write_class_and_schema --locked
```

Expected: PASS.

- [ ] **Step 5: Add provider call hook points**

Modify `src/runtime_units/clarification.rs` and `src/runtime_units/coding.rs` only by extracting the current provider call preparation into helper functions that return `AdapterInput` and node_id before running the provider. Keep behavior unchanged.

In `src/runtime_units/coding.rs`, create this helper near `run_report_node`:

```rust
fn pending_provider_step_for_context(
    node_id: &str,
    adapter_input: &crate::protocol::contracts::AdapterInput,
) -> Result<crate::interactive::controller::PendingProviderStep, crate::task_run::types::TaskRunError> {
    crate::task_run::step_runner::provider_step_from_adapter_input(node_id, adapter_input)
}
```

Call it in `run_report_node` immediately after the `provider_run_request` call:

```rust
let _pending_step = pending_provider_step_for_context(node_id, &context.adapter_input)
    .map_err(|error| ExecutionChainError::ProviderBlocked(error.message))?;
```

This introduces the seam without changing execution behavior. The variable is intentionally unused after validation.

- [ ] **Step 6: Run existing task run orchestrator tests**

Run:

```bash
cargo test --test task_run_orchestrator --locked
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/task_run/step_runner.rs src/runtime_units/clarification.rs src/runtime_units/coding.rs tests/task_run_step_runner.rs tests/task_run_orchestrator.rs
git commit -m "feat: expose provider steps from runtime inputs"
```

---

### Task 13: End-To-End Browse Acceptance For Fibonacci Shape

**Files:**
- Test: `tests/tui_cli.rs`
- Test fixture content created inside test only

- [ ] **Step 1: Add E2E-like browse acceptance test**

Append to `tests/tui_cli.rs`:

```rust
use std::fs;
use serde_json::json;

#[test]
fn tui_check_accepts_blocked_fibonacci_runtime_shape() {
    let workspace = tempdir().expect("workspace");
    let task_root = workspace.path().join(".aria/runtime/tasks/task_0001");
    fs::create_dir_all(task_root.join("reports")).expect("reports");
    fs::write(
        task_root.join("state.json"),
        serde_json::to_vec_pretty(&json!({
            "task_id": "task_0001",
            "change_id": "aria-fibonacci-square",
            "phase": "blocked_by_gate",
            "current_worktask": "work_wt_006"
        }))
        .expect("state json"),
    )
    .expect("write state");
    fs::write(
        task_root.join("reports/blocked-report.json"),
        serde_json::to_vec_pretty(&json!({
            "status": "blocked_by_gate",
            "reason": "rework_limit_exceeded",
            "next_node": "X08"
        }))
        .expect("blocked json"),
    )
    .expect("write blocked");
    fs::write(
        task_root.join("reports/testing-report.json"),
        serde_json::to_vec_pretty(&json!({
            "artifact_kind": "testing_report",
            "tests_passed": false,
            "failures": ["node_contract.allowed_write_scope=[]"]
        }))
        .expect("testing json"),
    )
    .expect("write testing");

    let output = run_cli([
        "tui",
        "--workspace",
        workspace.path().to_str().expect("workspace path"),
        "--task-id",
        "task_0001",
        "--check",
    ])
    .expect("tui check");

    assert_eq!(
        output,
        CliOutput::Text(format!(
            "tui_browse:{}:task_0001",
            workspace.path().to_string_lossy()
        ))
    );
}
```

- [ ] **Step 2: Run TUI CLI tests**

Run:

```bash
cargo test --test tui_cli --locked
```

Expected: PASS.

- [ ] **Step 3: Run focused interactive tests**

Run:

```bash
cargo test --test interactive_projection --test interactive_policy --test interactive_checkpoint --test interactive_controller --test tui_state --test task_run_step_runner --test tui_cli --locked
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add tests/tui_cli.rs
git commit -m "test: cover tui fibonacci browse acceptance"
```

---

### Task 14: Final Verification And Documentation Update

**Files:**
- Modify: `README.md` or create user-facing README only if product usage docs are required by the maintainer.
- Existing design: `cadence/designs/2026-05-07_技术方案_Aria_TUI工作台与可回退交互Runtime设计_v1.0.md`

- [ ] **Step 1: Run formatting check**

Run:

```bash
cargo fmt --check
```

Expected: PASS. If it fails, run `cargo fmt`, inspect the diff, and rerun `cargo fmt --check`.

- [ ] **Step 2: Run clippy**

Run:

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: PASS.

- [ ] **Step 3: Run full tests**

Run:

```bash
cargo test --locked -j 1
```

Expected: PASS.

- [ ] **Step 4: Verify CLI browse route manually**

Run:

```bash
cargo run --locked -- tui --workspace /tmp/aria-workspace --task-id task_0001 --check
```

Expected for missing task:

```text
interactive_task_missing: task does not exist: task_0001
```

This verifies clean error reporting before a real runtime store exists.

- [ ] **Step 5: Close verification**

If verification required fixes, return to the task that introduced the failing behavior, add a focused regression test there, and commit through that task's file list. If no files changed, do not create an empty commit.

---

## Self-Review

### Spec Coverage

- `aria tui` entry: covered by Tasks 9, 10, 11, and 13.
- Browse existing tasks: covered by Tasks 2, 3, 11, and 13.
- Node input/output and artifact visibility: covered by Tasks 2 and 3 through `WorkspaceProjection` and `ArtifactIndexEntry`.
- Configurable confirmation policy: covered by Task 4.
- Provider confirmation controller: covered by Tasks 6 and 12.
- Rollback semantics: covered by Task 5.
- TUI state and rendering: covered by Tasks 8 and 10.
- Non-interactive compatibility: protected by Tasks 9, 12, and 14 with existing task-run tests.
- Fibonacci blocked diagnostics: covered by Tasks 3 and 13.

### Placeholder Scan

The plan uses concrete file paths, test names, commands, expected results, and code snippets. It does not use unresolved markers.

### Type Consistency

The same names are used across tasks:

- `WorkspaceProjection`, `TaskSession`, `InteractionTurn`, `NodeRun`, `RuntimeCheckpoint`, `ArtifactIndexEntry`
- `PolicyPreset`, `ConfirmationDecision`, `NodeWriteClass`, `ProviderNodeMeta`
- `PendingProviderStep`, `StepRunner`, `StepRunnerResult`
- `TuiState`, `TuiAction`, `TuiTab`, `ActionInputMode`

Later tasks refer to types introduced in earlier tasks.
