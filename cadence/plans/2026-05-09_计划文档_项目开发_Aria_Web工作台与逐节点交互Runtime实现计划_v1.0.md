# Aria Web 工作台与逐节点交互 Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 构建 `aria web --workspace <PATH>` 本地单机 Web 工作台，支持任务新建、继续、逐节点暂停确认、provider 执行观察、产物浏览、checkpoint 回退和编辑 prompt 后重跑。

**Architecture:** 后端新增 Rust `web` 模块，使用 axum 暴露 HTTP API、SSE 事件流和静态资源托管，并复用 `interactive`、`task_run`、provider adapter 与 checkpoint 能力。前端新增 `web/` React/Vite/TypeScript SPA，第一屏就是高密度工作台，围绕 Flow Rail、Node Workspace、Evidence Panel、Action Composer 和 Rollback Dialog 展示 TUI 的全部信息域。逐节点闭环先用 fake/scripted provider 打通，再把规划、执行、最终收口 provider 节点接入可暂停 runner。

**Tech Stack:** Rust 1.95、tokio、serde/serde_json、axum、tower-http、React、Vite、TypeScript、TanStack Router、Tailwind CSS、Radix UI primitives、lucide-react、Vitest、Testing Library、Playwright。

---

## Scope And Sequencing

第一版范围为单机本地、单 workspace、真实闭环。桌面端壳、多 workspace 管理和云端协作只作为架构预留，不进入本计划的实现任务。

实施顺序：

1. 先定义 Web API 类型、错误模型和 projection 增量字段，让前后端共用稳定契约。
2. 再补强 checkpoint preview、pending provider step 和事件流，使逐节点暂停、确认、回退具备可靠后端语义。
3. 然后实现 axum API 与 `aria web` CLI 入口，先以 fake/scripted runner 完成端到端测试。
4. 最后搭建前端工作台，把 TUI 的 Overview、Timeline、IO、Artifacts、Changes、Diagnostics、Action 输入完整搬到浏览器。

执行计划时必须小步提交。当前工作区已存在与本计划无关的 staged 文件，提交时只 `git add` 每个任务列出的路径。

Rust 验证优先使用项目 Docker 规则：

```bash
docker run --rm -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/work/.cargo-home \
  -e CARGO_NET_RETRY=10 \
  -e CARGO_HTTP_TIMEOUT=120 \
  -v "$PWD":/work -w /work \
  rust:1-bookworm cargo test --locked -j 1
```

前端包管理器必须使用 `pnpm`。前端验证命令统一从仓库根目录执行：

```bash
pnpm --dir web install
pnpm --dir web test -- --run
pnpm --dir web build
```

## File Structure

### Rust 后端

| Path | Responsibility |
|------|----------------|
| `src/web/mod.rs` | Web 模块导出 |
| `src/web/types.rs` | API request/response、SSE event、frontend-facing projection 类型 |
| `src/web/error.rs` | 标准化 `ApiError`、HTTP status 映射、`TaskRunError` 映射 |
| `src/web/state.rs` | `WebAppState`、workspace 配置、runtime manager 和 event hub 共享状态 |
| `src/web/events.rs` | SSE broadcast、event cursor、事件序列化 |
| `src/web/runtime.rs` | Web task 创建、advance、confirm、projection refresh、rollback orchestration |
| `src/web/handlers.rs` | axum handlers：health、tasks、projection、advance、confirm、rollback、artifacts、files、events |
| `src/web/static_assets.rs` | `web/dist` 静态资源托管和 SPA fallback |
| `src/web/app.rs` | axum router 组装 |
| `src/interactive/web_projection.rs` | 将 `WorkspaceProjection` 扩展为 Web 所需 selected node、git summary、pending step context |
| `src/task_run/interactive_runner.rs` | 可暂停 step runner，先接 scripted/fake，后接规划/执行/最终 provider 节点 |

### Rust 既有文件修改

| Path | Change |
|------|--------|
| `Cargo.toml` | 增加 axum/tower-http/tower/mime_guess 相关依赖 |
| `src/lib.rs` | 导出 `web` 模块 |
| `src/cli.rs` | 增加 `aria web --workspace <PATH> [--host HOST] [--port PORT] [--check]` 解析与 async 路由 |
| `src/interactive/models.rs` | 增加 Web projection 字段和 pending provider step serde 类型 |
| `src/interactive/controller.rs` | pending step 返回完整 payload，confirm 记录最终 prompt 和 checkpoint |
| `src/interactive/checkpoint.rs` | 增加 rollback preview 和边界计数 |
| `src/task_run/step_runner.rs` | 保留现有 scripted seam，补齐 checkpoint、scope、verification command 信息 |
| `src/task_run/mod.rs` | 导出 `interactive_runner` |

### Rust 测试

| Path | Coverage |
|------|----------|
| `tests/web_types.rs` | API 类型序列化、错误码 JSON |
| `tests/web_projection.rs` | Web projection 包含 pending step、node context、artifact refs、diagnostics、git summary |
| `tests/interactive_checkpoint_preview.rs` | rollback preview 计算 dirty、drop counts、files_may_change |
| `tests/web_runtime_fake.rs` | fake task create、advance pause、confirm、projection refresh、rollback |
| `tests/web_api_handlers.rs` | axum handler-level API contract |
| `tests/web_events.rs` | SSE event hub replay 和 projection_updated 事件 |
| `tests/web_resource_handlers.rs` | `GET /api/tasks`、artifact 内容、文件内容和 checkpoint diff API |
| `tests/web_provider_output_events.rs` | provider stdout/stderr、structured output、manual gate、retry 和 provider auth diagnostics 事件 |
| `tests/web_cli.rs` | `aria web --check`、host/port 解析、无效参数 |
| `tests/task_run_interactive_runner.rs` | 真实 orchestration 拆分前的 runner seam 和非交互回归 |

### 前端

| Path | Responsibility |
|------|----------------|
| `web/package.json` | pnpm scripts 和前端依赖 |
| `web/vite.config.ts` | Vite、React、Vitest、API proxy |
| `web/tsconfig.json` | TypeScript 配置 |
| `web/tailwind.config.ts` | Tailwind content 和主题 token |
| `web/postcss.config.js` | Tailwind/PostCSS 配置 |
| `web/index.html` | SPA mount |
| `web/src/main.tsx` | React entry |
| `web/src/router.tsx` | TanStack Router 路由和 search params |
| `web/src/api/types.ts` | 后端 API TypeScript 类型镜像 |
| `web/src/api/client.ts` | fetch wrapper、错误标准化、SSE client |
| `web/src/state/workbench-store.ts` | projection、selected node/tab、pending action、event log 状态 |
| `web/src/components/shell/TopStatusBar.tsx` | workspace/task/status/provider/git/SSE 状态 |
| `web/src/components/shell/TaskSwitcher.tsx` | 已有 task 列表、继续任务入口、当前 task 选择 |
| `web/src/components/flow/FlowRail.tsx` | N00-N28 节点流程、状态、dropped 灰显 |
| `web/src/components/node/NodeWorkspace.tsx` | Overview、Inputs、Run、Outputs、Diff tabs |
| `web/src/components/evidence/EvidencePanel.tsx` | artifact/report/source/log 预览 |
| `web/src/components/evidence/ArtifactViewer.tsx` | Markdown、JSON、source、test、log 内容查看器 |
| `web/src/components/action/ActionComposer.tsx` | Codex-like prompt 输入、确认执行、停止、回退入口 |
| `web/src/components/rollback/RollbackDialog.tsx` | rollback preview、dirty 确认、执行回退 |
| `web/src/components/diagnostics/DiagnosticsPanel.tsx` | provider/gate/validation/checkpoint/web_runtime 分类诊断 |
| `web/src/styles.css` | 全局布局、Tailwind layers、主题变量 |

### 前端测试

| Path | Coverage |
|------|----------|
| `web/src/api/client.test.ts` | API success/error、SSE parse |
| `web/src/state/workbench-store.test.ts` | projection refresh、node/tab selection、dropped history |
| `web/src/components/shell/TaskSwitcher.test.tsx` | 已有 task 展示和继续任务选择 |
| `web/src/components/action/ActionComposer.test.tsx` | pending provider step、prompt 编辑、confirm payload |
| `web/src/components/flow/FlowRail.test.tsx` | node 状态、provider badge、dropped 灰显 |
| `web/src/components/evidence/EvidencePanel.test.tsx` | markdown/json/source/log preview selection |
| `web/src/components/evidence/ArtifactViewer.test.tsx` | artifact 内容加载和 content-type 渲染 |
| `web/src/components/node/NodeRunPanel.test.tsx` | provider output、manual gate、retry、structured output 渲染 |
| `web/src/components/rollback/RollbackDialog.test.tsx` | preview counts、dirty checkbox、rollback confirm |
| `web/e2e/fake-workbench.spec.ts` | fake provider 浏览器闭环 |

---

## Task 1: Web API Contract And Error Model

**Files:**
- Create: `src/web/mod.rs`
- Create: `src/web/types.rs`
- Create: `src/web/error.rs`
- Modify: `src/lib.rs`
- Test: `tests/web_types.rs`

- [ ] **Step 1: Write the failing tests**

Create `tests/web_types.rs`:

```rust
use cadence_aria::web::error::ApiError;
use cadence_aria::web::types::{
    AdvanceTaskResponse, ConfirmTaskRequest, CreateTaskRequest, CreateTaskResponse,
    PendingProviderStepDto, RollbackPreviewRequest, WebEvent,
};
use serde_json::json;

#[test]
fn create_task_request_uses_snake_case_contract() {
    let value = serde_json::from_value::<CreateTaskRequest>(json!({
        "request_text": "实现 Fibonacci square sum",
        "change_id": "aria-fibonacci-square",
        "policy_preset": "manual-write",
        "provider_mode": "fake",
        "timeout_secs": 2400
    }))
    .expect("request json");

    assert_eq!(value.change_id, "aria-fibonacci-square");
    assert_eq!(value.policy_preset, "manual-write");
    assert_eq!(value.provider_mode, "fake");
}

#[test]
fn paused_advance_response_serializes_pending_step() {
    let response = AdvanceTaskResponse::PausedForApproval {
        pending_step: PendingProviderStepDto {
            node_id: "N16".to_string(),
            provider_type: "codex".to_string(),
            runtime_role: "executor".to_string(),
            adapter_role: "executor".to_string(),
            prompt: "请实现函数".to_string(),
            input_summary: json!({"worktask_id":"work_wt_001"}),
            output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
            allowed_write_scope: vec!["src/".to_string(), "tests/".to_string()],
            forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
            verification_commands: vec!["cargo test --locked -j 1".to_string()],
            checkpoint_id: "ckpt_0001".to_string(),
        },
    };

    let value = serde_json::to_value(response).expect("response json");
    assert_eq!(value["status"], "paused_for_approval");
    assert_eq!(value["pending_step"]["node_id"], "N16");
    assert_eq!(value["pending_step"]["checkpoint_id"], "ckpt_0001");
}

#[test]
fn confirm_and_rollback_requests_match_frontend_payloads() {
    let confirm = serde_json::from_value::<ConfirmTaskRequest>(json!({
        "checkpoint_id": "ckpt_0001",
        "prompt": "最终确认后的 prompt"
    }))
    .expect("confirm");
    assert_eq!(confirm.prompt, "最终确认后的 prompt");

    let preview = serde_json::from_value::<RollbackPreviewRequest>(json!({
        "checkpoint_id": "ckpt_0001"
    }))
    .expect("preview");
    assert_eq!(preview.checkpoint_id, "ckpt_0001");
}

#[test]
fn api_error_serializes_standard_shape() {
    let value = serde_json::to_value(ApiError::validation(
        "invalid_task_request",
        "request_text is required",
    ))
    .expect("error json");
    assert_eq!(value["code"], "invalid_task_request");
    assert_eq!(value["message"], "request_text is required");
    assert_eq!(value["details"], json!({}));
}

#[test]
fn web_event_has_cursor_kind_and_payload() {
    let event = WebEvent {
        cursor: 7,
        event_type: "projection_updated".to_string(),
        task_id: Some("task_0001".to_string()),
        payload: json!({"projection_version": 42}),
    };
    let value = serde_json::to_value(event).expect("event");
    assert_eq!(value["cursor"], 7);
    assert_eq!(value["event_type"], "projection_updated");
}
```

- [ ] **Step 2: Run the tests and verify failure**

Run:

```bash
cargo test --test web_types --locked
```

Expected: FAIL with unresolved module `cadence_aria::web`.

- [ ] **Step 3: Add the Web module and DTOs**

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
pub mod web;
```

Create `src/web/mod.rs`:

```rust
pub mod error;
pub mod types;
```

Create `src/web/types.rs` with serde DTOs:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateTaskRequest {
    pub request_text: String,
    pub change_id: String,
    pub policy_preset: String,
    pub provider_mode: String,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateTaskResponse {
    pub task_id: String,
    pub session_id: String,
    pub change_id: String,
    pub phase: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PendingProviderStepDto {
    pub node_id: String,
    pub provider_type: String,
    pub runtime_role: String,
    pub adapter_role: String,
    pub prompt: String,
    pub input_summary: Value,
    pub output_schema: String,
    pub allowed_write_scope: Vec<String>,
    pub forbidden_actions: Vec<String>,
    pub verification_commands: Vec<String>,
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AdvanceTaskResponse {
    Advanced { projection_version: u64 },
    PausedForApproval { pending_step: PendingProviderStepDto },
    Completed { projection_version: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConfirmTaskRequest {
    pub checkpoint_id: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RollbackPreviewRequest {
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WebEvent {
    pub cursor: u64,
    pub event_type: String,
    pub task_id: Option<String>,
    pub payload: Value,
}
```

Create `src/web/error.rs`:

```rust
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub details: Value,
}

impl ApiError {
    pub fn validation(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: json!({}),
        }
    }

    pub fn runtime(code: impl Into<String>, message: impl Into<String>, details: Value) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details,
        }
    }
}
```

- [ ] **Step 4: Run the tests and verify success**

Run:

```bash
cargo test --test web_types --locked
```

Expected: PASS for all tests in `web_types`.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/web/mod.rs src/web/types.rs src/web/error.rs tests/web_types.rs
git commit -m "feat: add web api contract types"
```

## Task 2: Web Projection Fields For Node IO And Workspace Context

**Files:**
- Modify: `src/interactive/models.rs`
- Create: `src/interactive/web_projection.rs`
- Modify: `src/interactive/mod.rs`
- Modify: `src/interactive/projection.rs`
- Test: `tests/web_projection.rs`

- [ ] **Step 1: Write the failing projection test**

Create `tests/web_projection.rs`:

```rust
use cadence_aria::interactive::projection::build_workspace_projection;
use cadence_aria::interactive::web_projection::build_web_projection;
use serde_json::json;
use std::fs;
use tempfile::tempdir;

#[test]
fn web_projection_exposes_pending_step_node_context_artifacts_and_git_summary() {
    let workspace = tempdir().expect("workspace");
    let task_root = workspace.path().join(".aria/runtime/tasks/task_0001");
    fs::create_dir_all(task_root.join("reports")).expect("reports");
    fs::create_dir_all(task_root.join("artifacts/execution")).expect("artifacts");
    fs::create_dir_all(task_root.join("pending")).expect("pending");
    fs::write(
        task_root.join("state.json"),
        serde_json::to_vec_pretty(&json!({
            "task_id":"task_0001",
            "phase":"execution",
            "change_id":"aria-fibonacci-square",
            "current_node":"N16",
            "current_worktask":"work_wt_001"
        }))
        .expect("state"),
    )
    .expect("write state");
    fs::write(
        task_root.join("pending/provider-step.json"),
        serde_json::to_vec_pretty(&json!({
            "node_id":"N16",
            "provider_type":"codex",
            "runtime_role":"executor",
            "adapter_role":"executor",
            "prompt":"实现 fibonacciSquareSum",
            "input_summary":{"context_files":["openspec/changes/aria-fibonacci-square/tasks.md"]},
            "output_schema":"schema://aria/artifacts/coding_report/v1",
            "allowed_write_scope":["src/","tests/"],
            "forbidden_actions":["修改 cadence/project-rules"],
            "verification_commands":["node --test"],
            "checkpoint_id":"ckpt_0001"
        }))
        .expect("pending"),
    )
    .expect("write pending");
    fs::write(
        task_root.join("artifacts/execution/0000.json"),
        serde_json::to_vec_pretty(&json!({
            "artifact_ref":"coding_report_work_wt_001_0001",
            "artifact_kind":"coding_report",
            "producer_node":"N16",
            "traceability_refs":["REQ-001"]
        }))
        .expect("artifact"),
    )
    .expect("write artifact");

    let base = build_workspace_projection(workspace.path(), Some("task_0001")).expect("base");
    let web = build_web_projection(workspace.path(), base, Some("N16")).expect("web");

    assert_eq!(web.pending_provider_step.expect("pending").node_id, "N16");
    assert_eq!(web.selected_node_context.node_id, Some("N16".to_string()));
    assert_eq!(web.git_summary.workspace_path, workspace.path().to_string_lossy());
    assert_eq!(web.artifact_index[0].producer_node, Some("N16".to_string()));
    assert!(web.available_actions.contains(&"confirm_provider_step".to_string()));
}
```

- [ ] **Step 2: Run the tests and verify failure**

Run:

```bash
cargo test --test web_projection --locked
```

Expected: FAIL because `interactive::web_projection` and Web projection fields are missing.

- [ ] **Step 3: Add frontend-facing projection structs**

Modify `src/interactive/models.rs` by adding:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GitSummary {
    pub workspace_path: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub dirty: bool,
    pub dirty_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SelectedNodeContext {
    pub node_id: Option<String>,
    pub overview: Value,
    pub inputs: Vec<Value>,
    pub run: Vec<Value>,
    pub outputs: Vec<Value>,
    pub diffs: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PendingProviderStepProjection {
    pub node_id: String,
    pub provider_type: String,
    pub runtime_role: String,
    pub adapter_role: String,
    pub prompt: String,
    pub input_summary: Value,
    pub output_schema: String,
    pub allowed_write_scope: Vec<String>,
    pub forbidden_actions: Vec<String>,
    pub verification_commands: Vec<String>,
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WebWorkspaceProjection {
    pub workspace_root: String,
    pub active_task_id: Option<String>,
    pub active_session_id: Option<String>,
    pub overview: Value,
    pub sessions: Vec<TaskSession>,
    pub timeline: Vec<Value>,
    pub artifact_index: Vec<ArtifactIndexEntry>,
    pub diagnostics: Vec<Value>,
    pub available_actions: Vec<String>,
    pub pending_provider_step: Option<PendingProviderStepProjection>,
    pub selected_node_context: SelectedNodeContext,
    pub git_summary: GitSummary,
    pub event_cursor: u64,
}
```

- [ ] **Step 4: Build Web projection from runtime files**

Modify `src/interactive/mod.rs`:

```rust
pub mod checkpoint;
pub mod controller;
pub mod diagnostics;
pub mod models;
pub mod policy;
pub mod projection;
pub mod store;
pub mod web_projection;
```

Create `src/interactive/web_projection.rs` with:

```rust
use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use crate::interactive::models::{
    GitSummary, PendingProviderStepProjection, SelectedNodeContext, WebWorkspaceProjection,
    WorkspaceProjection,
};
use crate::task_run::types::TaskRunError;

pub fn build_web_projection(
    workspace_root: &Path,
    base: WorkspaceProjection,
    selected_node_id: Option<&str>,
) -> Result<WebWorkspaceProjection, TaskRunError> {
    let task_id = base.active_task_id.clone();
    let pending_provider_step = task_id
        .as_deref()
        .and_then(|task_id| read_pending_step(workspace_root, task_id).transpose())
        .transpose()?;
    let mut available_actions = base.available_actions.clone();
    if pending_provider_step.is_some() {
        available_actions.push("confirm_provider_step".to_string());
        available_actions.push("rollback_pending_checkpoint".to_string());
    }
    Ok(WebWorkspaceProjection {
        workspace_root: base.workspace_root,
        active_task_id: base.active_task_id,
        active_session_id: base.active_session_id,
        overview: base.overview,
        sessions: base.sessions,
        timeline: base.timeline,
        artifact_index: base.artifact_index,
        diagnostics: base.diagnostics,
        available_actions,
        pending_provider_step,
        selected_node_context: selected_node_context(selected_node_id),
        git_summary: git_summary(workspace_root),
        event_cursor: 0,
    })
}

fn read_pending_step(
    workspace_root: &Path,
    task_id: &str,
) -> Result<Option<PendingProviderStepProjection>, TaskRunError> {
    let path = workspace_root
        .join(".aria/runtime/tasks")
        .join(task_id)
        .join("pending/provider-step.json");
    match fs::File::open(&path) {
        Ok(file) => serde_json::from_reader(file).map(Some).map_err(|error| {
            TaskRunError::new(
                "interactive_projection_json",
                format!("parse {}: {error}", path.display()),
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(TaskRunError::new(
            "interactive_projection_io",
            format!("open {}: {error}", path.display()),
        )),
    }
}

fn selected_node_context(selected_node_id: Option<&str>) -> SelectedNodeContext {
    SelectedNodeContext {
        node_id: selected_node_id.map(str::to_string),
        overview: json!({}),
        inputs: Vec::new(),
        run: Vec::new(),
        outputs: Vec::new(),
        diffs: Vec::new(),
    }
}

fn git_summary(workspace_root: &Path) -> GitSummary {
    GitSummary {
        workspace_path: workspace_root.to_string_lossy().to_string(),
        branch: git_stdout(workspace_root, &["branch", "--show-current"]),
        head: git_stdout(workspace_root, &["rev-parse", "--short", "HEAD"]),
        dirty: git_stdout(workspace_root, &["status", "--porcelain"])
            .is_some_and(|text| !text.trim().is_empty()),
        dirty_files: Vec::new(),
    }
}

fn git_stdout(workspace_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
```

- [ ] **Step 5: Run the tests and verify success**

Run:

```bash
cargo test --test web_projection --locked
```

Expected: PASS for `web_projection`.

- [ ] **Step 6: Commit**

```bash
git add src/interactive/models.rs src/interactive/mod.rs src/interactive/web_projection.rs tests/web_projection.rs
git commit -m "feat: add web workspace projection"
```

## Task 3: Rollback Preview And Safer Checkpoint Boundary

**Files:**
- Modify: `src/interactive/checkpoint.rs`
- Test: `tests/interactive_checkpoint_preview.rs`

- [ ] **Step 1: Write the failing rollback preview tests**

Create `tests/interactive_checkpoint_preview.rs`:

```rust
use cadence_aria::interactive::checkpoint::{CheckpointService, RollbackPreviewRequest};
use cadence_aria::interactive::models::RuntimeCheckpoint;
use serde_json::json;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn rollback_preview_counts_later_records_and_dirty_files() {
    let workspace = tempdir().expect("workspace");
    git(workspace.path(), &["init", "-b", "main"]);
    git(workspace.path(), &["config", "user.name", "Aria Test"]);
    git(workspace.path(), &["config", "user.email", "aria-test@example.com"]);
    fs::write(workspace.path().join("file.txt"), "before\n").expect("file");
    git(workspace.path(), &["add", "file.txt"]);
    git(workspace.path(), &["commit", "-m", "before"]);
    let head = git_stdout(workspace.path(), &["rev-parse", "HEAD"]);

    let task_root = workspace.path().join(".aria/runtime/tasks/task_0001");
    fs::create_dir_all(task_root.join("turns")).expect("turns");
    fs::create_dir_all(task_root.join("node-runs")).expect("node runs");
    fs::create_dir_all(task_root.join("provider-runs/run_n16_0001")).expect("runs");
    fs::write(task_root.join("turns/turn_0001.json"), r#"{"turn_id":"turn_0001"}"#).expect("turn");
    fs::write(task_root.join("node-runs/nrun_0001.json"), r#"{"node_run_id":"nrun_0001"}"#).expect("node");
    fs::write(task_root.join("provider-runs/run_n16_0001/run.json"), r#"{"provider_run_id":"run_n16_0001"}"#).expect("run");
    fs::write(workspace.path().join("file.txt"), "dirty\n").expect("dirty");

    let service = CheckpointService::new(workspace.path(), "task_0001");
    service
        .write_checkpoint(&RuntimeCheckpoint {
            checkpoint_id: "ckpt_0001".to_string(),
            task_id: "task_0001".to_string(),
            session_id: "sess_task_0001".to_string(),
            turn_id: Some("turn_0001".to_string()),
            git_head: Some(head),
            dirty_summary: json!({"tracked":0}),
            state_snapshot_ref: "state@ckpt_0001.json".to_string(),
            projection_snapshot_ref: "projection@ckpt_0001.json".to_string(),
            artifact_boundary: 0,
            provider_run_boundary: 0,
            node_run_boundary: 0,
            created_at: "2026-05-09T00:00:00Z".to_string(),
        })
        .expect("checkpoint");

    let preview = service
        .preview_rollback(RollbackPreviewRequest {
            checkpoint_id: "ckpt_0001".to_string(),
        })
        .expect("preview");

    assert_eq!(preview.checkpoint_id, "ckpt_0001");
    assert!(preview.dirty);
    assert_eq!(preview.turns_to_drop, 1);
    assert_eq!(preview.node_runs_to_drop, 1);
    assert_eq!(preview.provider_runs_to_drop, 1);
    assert!(preview.files_may_change.contains(&"file.txt".to_string()));
}

fn git(cwd: &std::path::Path, args: &[&str]) {
    let output = Command::new("git").args(args).current_dir(cwd).output().expect("git");
    assert!(output.status.success(), "git {:?} failed", args);
}

fn git_stdout(cwd: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git").args(args).current_dir(cwd).output().expect("git");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
```

- [ ] **Step 2: Run the tests and verify failure**

Run:

```bash
cargo test --test interactive_checkpoint_preview --locked
```

Expected: FAIL because `RollbackPreviewRequest` and `preview_rollback` do not exist.

- [ ] **Step 3: Add preview structs and counting logic**

Modify `src/interactive/checkpoint.rs` by adding:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackPreviewRequest {
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackPreview {
    pub checkpoint_id: String,
    pub git_head: Option<String>,
    pub dirty: bool,
    pub turns_to_drop: usize,
    pub node_runs_to_drop: usize,
    pub provider_runs_to_drop: usize,
    pub artifacts_to_drop: usize,
    pub files_may_change: Vec<String>,
}
```

Add methods to `impl CheckpointService`:

```rust
pub fn preview_rollback(
    &self,
    request: RollbackPreviewRequest,
) -> Result<RollbackPreview, TaskRunError> {
    let checkpoint = self.read_checkpoint(&request.checkpoint_id)?;
    Ok(RollbackPreview {
        checkpoint_id: checkpoint.checkpoint_id,
        git_head: checkpoint.git_head,
        dirty: self.worktree_dirty()?,
        turns_to_drop: count_json_files(&self.task_root().join("turns"))?,
        node_runs_to_drop: count_json_files(&self.task_root().join("node-runs"))?,
        provider_runs_to_drop: count_provider_runs(&self.task_root().join("provider-runs"))?,
        artifacts_to_drop: count_json_files(&self.task_root().join("artifacts"))?,
        files_may_change: self.changed_files()?,
    })
}

fn changed_files(&self) -> Result<Vec<String>, TaskRunError> {
    let output = self.git(&["status", "--porcelain"])?;
    Ok(output
        .lines()
        .filter_map(|line| line.get(3..))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect())
}
```

Add helper functions near existing `mark_json_files_dropped`:

```rust
fn count_json_files(root: &Path) -> Result<usize, TaskRunError> {
    if !root.exists() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in fs::read_dir(root).map_err(|error| {
        TaskRunError::new("checkpoint_io", format!("read {}: {error}", root.display()))
    })? {
        let path = entry
            .map_err(|error| TaskRunError::new("checkpoint_io", error.to_string()))?
            .path();
        if path.is_dir() {
            count += count_json_files(&path)?;
        } else if path.extension().is_some_and(|extension| extension == "json") {
            count += 1;
        }
    }
    Ok(count)
}

fn count_provider_runs(root: &Path) -> Result<usize, TaskRunError> {
    if !root.exists() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in fs::read_dir(root).map_err(|error| {
        TaskRunError::new("checkpoint_io", format!("read {}: {error}", root.display()))
    })? {
        let path = entry
            .map_err(|error| TaskRunError::new("checkpoint_io", error.to_string()))?
            .path()
            .join("run.json");
        if path.exists() {
            count += 1;
        }
    }
    Ok(count)
}
```

- [ ] **Step 4: Run checkpoint tests**

Run:

```bash
cargo test --test interactive_checkpoint --test interactive_checkpoint_preview --locked
```

Expected: PASS for existing rollback behavior and new preview behavior.

- [ ] **Step 5: Commit**

```bash
git add src/interactive/checkpoint.rs tests/interactive_checkpoint_preview.rs
git commit -m "feat: add rollback preview"
```

## Task 4: Persistent Pending Provider Step And Confirmed Prompt

**Files:**
- Modify: `src/interactive/controller.rs`
- Modify: `src/task_run/step_runner.rs`
- Test: `tests/interactive_controller.rs`
- Test: `tests/task_run_step_runner.rs`

- [ ] **Step 1: Add failing tests for checkpoint and scope metadata**

Append to `tests/interactive_controller.rs`:

```rust
#[test]
fn pending_step_includes_checkpoint_scope_and_verification_metadata() {
    let workspace = tempdir().expect("workspace");
    let runner = FakeRunner {
        next: Some(PendingProviderStep {
            node_id: "N16".to_string(),
            provider_type: "codex".to_string(),
            runtime_role: "executor".to_string(),
            adapter_role: "executor".to_string(),
            prompt: "实现功能".to_string(),
            input_summary: json!({"worktask_id":"work_wt_001"}),
            output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
            write_class: NodeWriteClass::WritesWorkspace,
            allowed_write_scope: vec!["src/".to_string(), "tests/".to_string()],
            forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
            verification_commands: vec!["node --test".to_string()],
            checkpoint_id: Some("ckpt_0001".to_string()),
        }),
    };
    let mut controller = InteractiveController::new(
        workspace.path().to_path_buf(),
        "task_0001".to_string(),
        PolicyPreset::ManualWrite,
        runner,
    );

    controller.advance().expect("advance");
    let pending = controller.pending_step().expect("pending");
    assert_eq!(pending.checkpoint_id.as_deref(), Some("ckpt_0001"));
    assert_eq!(pending.allowed_write_scope, vec!["src/".to_string(), "tests/".to_string()]);
    assert_eq!(pending.verification_commands, vec!["node --test".to_string()]);
}
```

Append to `tests/task_run_step_runner.rs`:

```rust
#[test]
fn provider_step_from_adapter_input_exposes_web_confirmation_metadata() {
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
    assert_eq!(step.runtime_role, "executor");
    assert_eq!(step.adapter_role, "executor");
    assert_eq!(step.allowed_write_scope, vec!["src/".to_string(), "tests/".to_string()]);
    assert!(step.verification_commands.iter().any(|command| command.contains("test")));
}
```

- [ ] **Step 2: Run controller and step runner tests**

Run:

```bash
cargo test --test interactive_controller --test task_run_step_runner --locked
```

Expected: FAIL because `PendingProviderStep` lacks the new fields.

- [ ] **Step 3: Extend `PendingProviderStep`**

Modify `src/interactive/controller.rs`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct PendingProviderStep {
    pub node_id: String,
    pub provider_type: String,
    pub runtime_role: String,
    pub adapter_role: String,
    pub prompt: String,
    pub input_summary: Value,
    pub output_schema: String,
    pub write_class: NodeWriteClass,
    pub allowed_write_scope: Vec<String>,
    pub forbidden_actions: Vec<String>,
    pub verification_commands: Vec<String>,
    pub checkpoint_id: Option<String>,
}
```

Update all test constructors in `tests/interactive_controller.rs` and `tests/task_run_step_runner.rs` with deterministic values:

```rust
runtime_role: "executor".to_string(),
adapter_role: "executor".to_string(),
allowed_write_scope: vec!["src/".to_string(), "tests/".to_string()],
forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
verification_commands: vec!["cargo test --locked -j 1".to_string()],
checkpoint_id: Some("ckpt_0001".to_string()),
```

- [ ] **Step 4: Populate metadata from adapter input**

Modify `src/task_run/step_runner.rs` inside `provider_step_from_adapter_input`:

```rust
runtime_role: adapter_role_text(&input.role).to_string(),
adapter_role: adapter_role_text(&input.role).to_string(),
allowed_write_scope: allowed_write_scope_for_node(node_id),
forbidden_actions: vec![
    "不要修改 .claude/rules".to_string(),
    "不要修改 cadence/project-rules".to_string(),
],
verification_commands: verification_commands_for_node(node_id),
checkpoint_id: None,
```

Add helpers:

```rust
fn adapter_role_text(role: &AdapterRole) -> &'static str {
    match role {
        AdapterRole::Planner => "planner",
        AdapterRole::Reviewer => "reviewer",
        AdapterRole::Executor => "executor",
    }
}

fn allowed_write_scope_for_node(node_id: &str) -> Vec<String> {
    match node_id {
        "N16" | "N19" => vec!["src/".to_string(), "tests/".to_string(), "openspec/".to_string()],
        "N04" | "N05" | "N07" | "N09" | "N10" | "N11" | "N12" | "N25" | "N26" | "N27" => {
            vec![".aria/runtime/".to_string(), "openspec/".to_string()]
        }
        _ => Vec::new(),
    }
}

fn verification_commands_for_node(node_id: &str) -> Vec<String> {
    match node_id {
        "N16" | "N17" | "N18" | "N19" => vec!["cargo test --locked -j 1".to_string()],
        "N25" | "N27" => vec!["cargo test --locked -j 1".to_string()],
        _ => vec!["cargo check --locked".to_string()],
    }
}
```

- [ ] **Step 5: Run tests and full compile check**

Run:

```bash
cargo test --test interactive_controller --test task_run_step_runner --locked
cargo check --locked
```

Expected: PASS for both tests and compile check.

- [ ] **Step 6: Commit**

```bash
git add src/interactive/controller.rs src/task_run/step_runner.rs tests/interactive_controller.rs tests/task_run_step_runner.rs
git commit -m "feat: enrich pending provider step metadata"
```

## Task 5: Web Runtime Fake Closed Loop

**Files:**
- Create: `src/web/state.rs`
- Create: `src/web/runtime.rs`
- Modify: `src/web/mod.rs`
- Test: `tests/web_runtime_fake.rs`

- [ ] **Step 1: Write failing runtime tests**

Create `tests/web_runtime_fake.rs`:

```rust
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::types::{ConfirmTaskRequest, CreateTaskRequest};
use tempfile::tempdir;

#[test]
fn web_runtime_fake_create_advance_confirm_and_projection() {
    let workspace = tempdir().expect("workspace");
    let mut runtime = WebRuntime::new_fake(workspace.path().to_path_buf());

    let created = runtime
        .create_task(CreateTaskRequest {
            request_text: "实现 Fibonacci square sum".to_string(),
            change_id: "aria-fibonacci-square".to_string(),
            policy_preset: "manual-write".to_string(),
            provider_mode: "fake".to_string(),
            timeout_secs: 2400,
        })
        .expect("create");
    assert_eq!(created.task_id, "task_0001");

    let paused = runtime.advance_task(&created.task_id).expect("advance");
    let pending = paused.expect_pending_step().expect("pending");
    assert_eq!(pending.node_id, "N16");

    let confirmed = runtime
        .confirm_task(
            &created.task_id,
            ConfirmTaskRequest {
                checkpoint_id: pending.checkpoint_id,
                prompt: "确认执行 N16".to_string(),
            },
        )
        .expect("confirm");
    assert_eq!(confirmed.node_id, "N16");

    let projection = runtime
        .projection(Some(&created.task_id), Some("N16"))
        .expect("projection");
    assert_eq!(projection.active_task_id, Some("task_0001".to_string()));
    assert!(projection.timeline.iter().any(|item| item["node_id"] == "N16"));
}
```

- [ ] **Step 2: Run the test and verify failure**

Run:

```bash
cargo test --test web_runtime_fake --locked
```

Expected: FAIL because `WebRuntime` is missing.

- [ ] **Step 3: Add runtime state**

Create `src/web/state.rs`:

```rust
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::web::runtime::WebRuntime;

#[derive(Clone)]
pub struct WebAppState {
    pub workspace_root: PathBuf,
    pub runtime: Arc<Mutex<WebRuntime>>,
}

impl WebAppState {
    pub fn new(workspace_root: PathBuf, runtime: WebRuntime) -> Self {
        Self {
            workspace_root,
            runtime: Arc::new(Mutex::new(runtime)),
        }
    }
}
```

- [ ] **Step 4: Add fake runtime orchestration**

Create `src/web/runtime.rs` with:

```rust
use std::fs;
use std::path::PathBuf;

use serde_json::json;

use crate::interactive::projection::build_workspace_projection;
use crate::interactive::web_projection::build_web_projection;
use crate::interactive::models::WebWorkspaceProjection;
use crate::task_run::types::TaskRunError;
use crate::web::types::{
    AdvanceTaskResponse, ConfirmTaskRequest, ConfirmTaskResponse, CreateTaskRequest,
    CreateTaskResponse, PendingProviderStepDto,
};

pub struct WebRuntime {
    workspace_root: PathBuf,
    next_projection_version: u64,
}

impl WebRuntime {
    pub fn new_fake(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            next_projection_version: 1,
        }
    }

    pub fn create_task(
        &mut self,
        request: CreateTaskRequest,
    ) -> Result<CreateTaskResponse, TaskRunError> {
        let task_id = "task_0001".to_string();
        let session_id = "sess_task_0001".to_string();
        let task_root = self.workspace_root.join(".aria/runtime/tasks").join(&task_id);
        fs::create_dir_all(task_root.join("pending")).map_err(io_error)?;
        fs::create_dir_all(task_root.join("logs")).map_err(io_error)?;
        fs::write(
            task_root.join("state.json"),
            serde_json::to_vec_pretty(&json!({
                "task_id": task_id,
                "phase": "intake",
                "change_id": request.change_id,
                "current_node": "N16"
            }))
            .map_err(json_error)?,
        )
        .map_err(io_error)?;
        Ok(CreateTaskResponse {
            task_id,
            session_id,
            change_id: request.change_id,
            phase: "intake".to_string(),
        })
    }

    pub fn advance_task(&mut self, task_id: &str) -> Result<AdvanceTaskResponse, TaskRunError> {
        let pending = PendingProviderStepDto {
            node_id: "N16".to_string(),
            provider_type: "codex".to_string(),
            runtime_role: "executor".to_string(),
            adapter_role: "executor".to_string(),
            prompt: "实现 Fibonacci square sum".to_string(),
            input_summary: json!({"worktask_id":"work_wt_001"}),
            output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
            allowed_write_scope: vec!["src/".to_string(), "tests/".to_string()],
            forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
            verification_commands: vec!["cargo test --locked -j 1".to_string()],
            checkpoint_id: "ckpt_0001".to_string(),
        };
        let task_root = self.workspace_root.join(".aria/runtime/tasks").join(task_id);
        fs::create_dir_all(task_root.join("pending")).map_err(io_error)?;
        fs::write(
            task_root.join("pending/provider-step.json"),
            serde_json::to_vec_pretty(&pending).map_err(json_error)?,
        )
        .map_err(io_error)?;
        Ok(AdvanceTaskResponse::PausedForApproval {
            pending_step: pending,
        })
    }

    pub fn confirm_task(
        &mut self,
        task_id: &str,
        request: ConfirmTaskRequest,
    ) -> Result<ConfirmTaskResponse, TaskRunError> {
        let task_root = self.workspace_root.join(".aria/runtime/tasks").join(task_id);
        fs::write(
            task_root.join("logs/node-events.jsonl"),
            format!(
                "{{\"event_kind\":\"node_completed\",\"task_id\":\"{task_id}\",\"node_id\":\"N16\",\"status\":\"completed\",\"details\":{{\"checkpoint_id\":\"{}\"}}}}\n",
                request.checkpoint_id
            ),
        )
        .map_err(io_error)?;
        let pending_path = task_root.join("pending/provider-step.json");
        if pending_path.exists() {
            fs::remove_file(pending_path).map_err(io_error)?;
        }
        self.next_projection_version += 1;
        Ok(ConfirmTaskResponse {
            status: "provider_started".to_string(),
            node_id: "N16".to_string(),
            turn_id: "turn_0001".to_string(),
        })
    }

    pub fn projection(
        &self,
        task_id: Option<&str>,
        selected_node_id: Option<&str>,
    ) -> Result<WebWorkspaceProjection, TaskRunError> {
        let base = build_workspace_projection(&self.workspace_root, task_id)?;
        build_web_projection(&self.workspace_root, base, selected_node_id)
    }
}

fn io_error(error: std::io::Error) -> TaskRunError {
    TaskRunError::new("web_runtime_io", error.to_string())
}

fn json_error(error: serde_json::Error) -> TaskRunError {
    TaskRunError::new("web_runtime_json", error.to_string())
}
```

Add `ConfirmTaskResponse` to `src/web/types.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConfirmTaskResponse {
    pub status: String,
    pub node_id: String,
    pub turn_id: String,
}
```

Add a test helper method on `AdvanceTaskResponse`:

```rust
impl AdvanceTaskResponse {
    pub fn expect_pending_step(self) -> Option<PendingProviderStepDto> {
        match self {
            AdvanceTaskResponse::PausedForApproval { pending_step } => Some(pending_step),
            _ => None,
        }
    }
}
```

Modify `src/web/mod.rs`:

```rust
pub mod error;
pub mod runtime;
pub mod state;
pub mod types;
```

- [ ] **Step 5: Run the fake runtime test**

Run:

```bash
cargo test --test web_runtime_fake --locked
```

Expected: PASS for create, advance, confirm, projection refresh.

- [ ] **Step 6: Commit**

```bash
git add src/web/mod.rs src/web/runtime.rs src/web/state.rs src/web/types.rs tests/web_runtime_fake.rs
git commit -m "feat: add fake web runtime loop"
```

## Task 6: Axum API Handlers

**Files:**
- Modify: `Cargo.toml`
- Create: `src/web/app.rs`
- Create: `src/web/handlers.rs`
- Modify: `src/web/error.rs`
- Modify: `src/web/mod.rs`
- Test: `tests/web_api_handlers.rs`

- [ ] **Step 1: Write failing handler tests**

Create `tests/web_api_handlers.rs`:

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
async fn api_create_advance_confirm_projection_contract() {
    let workspace = tempdir().expect("workspace");
    let state = WebAppState::new(
        workspace.path().to_path_buf(),
        WebRuntime::new_fake(workspace.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let create = request_json(
        app.clone(),
        Method::POST,
        "/api/tasks",
        json!({
            "request_text":"实现 Fibonacci square sum",
            "change_id":"aria-fibonacci-square",
            "policy_preset":"manual-write",
            "provider_mode":"fake",
            "timeout_secs":2400
        }),
    )
    .await;
    assert_eq!(create["task_id"], "task_0001");

    let advance = request_json(app.clone(), Method::POST, "/api/tasks/task_0001/advance", json!({})).await;
    assert_eq!(advance["status"], "paused_for_approval");
    assert_eq!(advance["pending_step"]["node_id"], "N16");

    let confirm = request_json(
        app.clone(),
        Method::POST,
        "/api/tasks/task_0001/confirm",
        json!({"checkpoint_id":"ckpt_0001","prompt":"确认执行"}),
    )
    .await;
    assert_eq!(confirm["node_id"], "N16");

    let projection = request_json(app, Method::GET, "/api/projection?task_id=task_0001", json!({})).await;
    assert_eq!(projection["active_task_id"], "task_0001");
}

async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> Value {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.expect("body");
    serde_json::from_slice(&bytes).expect("json")
}
```

- [ ] **Step 2: Run the test and verify failure**

Run:

```bash
cargo test --test web_api_handlers --locked
```

Expected: FAIL because axum dependency and handlers do not exist.

- [ ] **Step 3: Add dependencies**

Modify `Cargo.toml`:

```toml
[dependencies]
axum = "0.8"
tower-http = { version = "0.6", features = ["fs", "trace"] }
mime_guess = "2.0"

[dev-dependencies]
tower = { version = "0.5", features = ["util"] }
```

- [ ] **Step 4: Implement app router and handlers**

Create `src/web/app.rs`:

```rust
use axum::routing::{get, post};
use axum::Router;

use crate::web::handlers;
use crate::web::state::WebAppState;

pub fn build_web_router(state: WebAppState) -> Router {
    Router::new()
        .route("/api/health", get(handlers::health))
        .route("/api/projection", get(handlers::projection))
        .route("/api/tasks", post(handlers::create_task))
        .route("/api/tasks/{task_id}/advance", post(handlers::advance_task))
        .route("/api/tasks/{task_id}/confirm", post(handlers::confirm_task))
        .with_state(state)
}
```

Create `src/web/handlers.rs`:

```rust
use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::web::error::ApiResult;
use crate::web::state::WebAppState;
use crate::web::types::{ConfirmTaskRequest, CreateTaskRequest};

#[derive(Debug, Deserialize)]
pub struct ProjectionQuery {
    pub task_id: Option<String>,
    pub node_id: Option<String>,
}

pub async fn health() -> Json<serde_json::Value> {
    Json(json!({"status":"ok"}))
}

pub async fn create_task(
    State(state): State<WebAppState>,
    Json(request): Json<CreateTaskRequest>,
) -> ApiResult<Json<crate::web::types::CreateTaskResponse>> {
    let mut runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(runtime.create_task(request)?))
}

pub async fn advance_task(
    State(state): State<WebAppState>,
    Path(task_id): Path<String>,
) -> ApiResult<Json<crate::web::types::AdvanceTaskResponse>> {
    let mut runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(runtime.advance_task(&task_id)?))
}

pub async fn confirm_task(
    State(state): State<WebAppState>,
    Path(task_id): Path<String>,
    Json(request): Json<ConfirmTaskRequest>,
) -> ApiResult<Json<crate::web::types::ConfirmTaskResponse>> {
    let mut runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(runtime.confirm_task(&task_id, request)?))
}

pub async fn projection(
    State(state): State<WebAppState>,
    Query(query): Query<ProjectionQuery>,
) -> ApiResult<Json<crate::interactive::models::WebWorkspaceProjection>> {
    let runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(runtime.projection(query.task_id.as_deref(), query.node_id.as_deref())?))
}
```

Modify `src/web/error.rs`:

```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

pub type ApiResult<T> = Result<T, ApiError>;

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.code.as_str() {
            "invalid_task_request" => StatusCode::BAD_REQUEST,
            "checkpoint_unsafe_dirty_worktree" => StatusCode::CONFLICT,
            "interactive_task_missing" => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(self)).into_response()
    }
}

impl From<crate::task_run::types::TaskRunError> for ApiError {
    fn from(error: crate::task_run::types::TaskRunError) -> Self {
        ApiError::runtime(error.code, error.message, serde_json::json!({}))
    }
}
```

Modify `src/web/mod.rs`:

```rust
pub mod app;
pub mod error;
pub mod handlers;
pub mod runtime;
pub mod state;
pub mod types;
```

- [ ] **Step 5: Run handler tests**

Run:

```bash
cargo test --test web_api_handlers --locked
```

Expected: PASS for create, advance, confirm, projection.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/web/app.rs src/web/handlers.rs src/web/error.rs src/web/mod.rs tests/web_api_handlers.rs
git commit -m "feat: add web api handlers"
```

## Task 7: SSE Event Hub And Projection Updates

**Files:**
- Create: `src/web/events.rs`
- Modify: `src/web/state.rs`
- Modify: `src/web/runtime.rs`
- Modify: `src/web/handlers.rs`
- Modify: `src/web/app.rs`
- Modify: `src/web/mod.rs`
- Test: `tests/web_events.rs`

- [ ] **Step 1: Write failing event hub tests**

Create `tests/web_events.rs`:

```rust
use cadence_aria::web::events::EventHub;
use serde_json::json;

#[test]
fn event_hub_records_events_with_incrementing_cursor() {
    let hub = EventHub::new();
    let first = hub.publish("projection_updated", Some("task_0001"), json!({"version":1}));
    let second = hub.publish("paused_for_approval", Some("task_0001"), json!({"node_id":"N16"}));

    assert_eq!(first.cursor, 1);
    assert_eq!(second.cursor, 2);
    let replay = hub.replay_after(0);
    assert_eq!(replay.len(), 2);
    assert_eq!(replay[1].event_type, "paused_for_approval");
}
```

- [ ] **Step 2: Run the test and verify failure**

Run:

```bash
cargo test --test web_events --locked
```

Expected: FAIL because `EventHub` is missing.

- [ ] **Step 3: Implement event hub**

Create `src/web/events.rs`:

```rust
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::broadcast;

use crate::web::types::WebEvent;

#[derive(Clone)]
pub struct EventHub {
    inner: Arc<Mutex<EventHubInner>>,
    tx: broadcast::Sender<WebEvent>,
}

struct EventHubInner {
    cursor: u64,
    replay: VecDeque<WebEvent>,
}

impl EventHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            inner: Arc::new(Mutex::new(EventHubInner {
                cursor: 0,
                replay: VecDeque::new(),
            })),
            tx,
        }
    }

    pub fn publish(
        &self,
        event_type: impl Into<String>,
        task_id: Option<&str>,
        payload: Value,
    ) -> WebEvent {
        let mut inner = self.inner.lock().expect("event hub lock");
        inner.cursor += 1;
        let event = WebEvent {
            cursor: inner.cursor,
            event_type: event_type.into(),
            task_id: task_id.map(str::to_string),
            payload,
        };
        inner.replay.push_back(event.clone());
        while inner.replay.len() > 512 {
            inner.replay.pop_front();
        }
        let _ = self.tx.send(event.clone());
        event
    }

    pub fn replay_after(&self, cursor: u64) -> Vec<WebEvent> {
        let inner = self.inner.lock().expect("event hub lock");
        inner
            .replay
            .iter()
            .filter(|event| event.cursor > cursor)
            .cloned()
            .collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WebEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: Wire event hub into state and runtime**

Modify `src/web/state.rs`:

```rust
use crate::web::events::EventHub;

#[derive(Clone)]
pub struct WebAppState {
    pub workspace_root: PathBuf,
    pub runtime: Arc<Mutex<WebRuntime>>,
    pub events: EventHub,
}

impl WebAppState {
    pub fn new(workspace_root: PathBuf, runtime: WebRuntime) -> Self {
        Self {
            workspace_root,
            runtime: Arc::new(Mutex::new(runtime)),
            events: EventHub::new(),
        }
    }
}
```

Modify handlers after create/advance/confirm:

```rust
state.events.publish("projection_updated", Some(&task_id), json!({}));
```

Add SSE route in `src/web/app.rs`:

```rust
.route("/api/events", get(handlers::events))
```

Add `events` handler that replays missed events and then stays subscribed to live broadcasts:

```rust
pub async fn events(State(state): State<WebAppState>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let replay_stream = stream::iter(state.events.replay_after(0));
    let live_stream = BroadcastStream::new(state.events.subscribe())
        .filter_map(|event| async move { event.ok() });
    let sse_stream = replay_stream
        .chain(live_stream)
        .map(|event| Ok(sse_event(event)));
    Sse::new(sse_stream).keep_alive(KeepAlive::default())
}

fn sse_event(event: WebEvent) -> Event {
    Event::default()
        .id(event.cursor.to_string())
        .event(event.event_type.clone())
        .json_data(event)
        .expect("serialize web event")
}
```

Use concrete imports:

```rust
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::{self, Stream, StreamExt};
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;
```

Add dependency:

```toml
futures-util = "0.3"
tokio-stream = { version = "0.1", features = ["sync"] }
```

- [ ] **Step 5: Run event and handler tests**

Run:

```bash
cargo test --test web_events --test web_api_handlers --locked
```

Expected: PASS and existing API contract still returns JSON.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/web/events.rs src/web/state.rs src/web/runtime.rs src/web/handlers.rs src/web/app.rs src/web/mod.rs tests/web_events.rs
git commit -m "feat: add web event stream"
```

## Task 7.5: Task List, Artifact Content, File Content, And Diff APIs

**Files:**
- Modify: `src/web/types.rs`
- Modify: `src/web/runtime.rs`
- Modify: `src/web/handlers.rs`
- Modify: `src/web/app.rs`
- Modify: `web/src/api/types.ts`
- Modify: `web/src/api/client.ts`
- Create: `web/src/components/evidence/ArtifactViewer.tsx`
- Test: `tests/web_resource_handlers.rs`
- Test: `web/src/components/evidence/ArtifactViewer.test.tsx`

- [ ] **Step 1: Write failing backend API tests**

Create `tests/web_resource_handlers.rs`:

```rust
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use std::fs;
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn resource_handlers_cover_tasks_artifact_file_and_diff_contracts() {
    let workspace = tempdir().expect("workspace");
    let task_root = workspace.path().join(".aria/runtime/tasks/task_0001");
    fs::create_dir_all(task_root.join("artifacts/execution")).expect("artifacts");
    fs::create_dir_all(workspace.path().join("src")).expect("src");
    fs::write(
        task_root.join("state.json"),
        r#"{"task_id":"task_0001","phase":"execution","change_id":"aria-fibonacci-square"}"#,
    )
    .expect("state");
    fs::write(
        task_root.join("artifacts/execution/0000.json"),
        r#"{"artifact_ref":"coding_report_work_wt_001_0001","artifact_kind":"coding_report","producer_node":"N16"}"#,
    )
    .expect("artifact");
    fs::write(workspace.path().join("src/fibonacciSquareSum.js"), "export const ok = true;\n")
        .expect("source");

    let state = WebAppState::new(
        workspace.path().to_path_buf(),
        WebRuntime::new_fake(workspace.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let tasks = request_json(app.clone(), Method::GET, "/api/tasks", json!({})).await;
    assert_eq!(tasks["tasks"][0]["task_id"], "task_0001");

    let artifact = request_json(
        app.clone(),
        Method::GET,
        "/api/artifacts/coding_report_work_wt_001_0001",
        json!({}),
    )
    .await;
    assert_eq!(artifact["artifact_ref"], "coding_report_work_wt_001_0001");
    assert_eq!(artifact["content_type"], "json");

    let file = request_json(
        app.clone(),
        Method::GET,
        "/api/files/content?path=src/fibonacciSquareSum.js",
        json!({}),
    )
    .await;
    assert_eq!(file["path"], "src/fibonacciSquareSum.js");
    assert!(file["content"].as_str().expect("content").contains("export const ok"));

    let diff = request_json(
        app,
        Method::GET,
        "/api/files/diff?base_checkpoint=ckpt_0001&path=src/fibonacciSquareSum.js",
        json!({}),
    )
    .await;
    assert_eq!(diff["path"], "src/fibonacciSquareSum.js");
    assert!(diff["diff"].is_string());
}

async fn request_json(app: axum::Router, method: Method, uri: &str, body: Value) -> Value {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.expect("body");
    serde_json::from_slice(&bytes).expect("json")
}
```

- [ ] **Step 2: Write failing frontend artifact viewer test**

Create `web/src/components/evidence/ArtifactViewer.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ArtifactViewer } from "./ArtifactViewer";

describe("ArtifactViewer", () => {
  it("renders json artifact content with path and producer node", () => {
    render(<ArtifactViewer artifact={{
      artifact_ref: "coding_report_work_wt_001_0001",
      artifact_kind: "coding_report",
      producer_node: "N16",
      path: ".aria/runtime/tasks/task_0001/artifacts/execution/0000.json",
      content_type: "json",
      content: "{\"status\":\"completed\"}"
    }} />);
    expect(screen.getByText("coding_report")).toBeInTheDocument();
    expect(screen.getByText(/N16/)).toBeInTheDocument();
    expect(screen.getByText(/completed/)).toBeInTheDocument();
  });
});
```

- [ ] **Step 3: Run tests and verify failure**

Run:

```bash
cargo test --test web_resource_handlers --locked
pnpm --dir web test -- --run web/src/components/evidence/ArtifactViewer.test.tsx
```

Expected: FAIL because resource DTOs, handlers, client methods and viewer are missing.

- [ ] **Step 4: Add resource DTOs**

Add to `src/web/types.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskListResponse {
    pub tasks: Vec<TaskListItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskListItem {
    pub task_id: String,
    pub change_id: Option<String>,
    pub phase: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ArtifactContentResponse {
    pub artifact_ref: String,
    pub artifact_kind: String,
    pub producer_node: Option<String>,
    pub path: String,
    pub content_type: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FileContentResponse {
    pub path: String,
    pub content_type: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FileDiffResponse {
    pub base_checkpoint: String,
    pub path: String,
    pub diff: String,
}
```

- [ ] **Step 5: Add backend runtime methods and handlers**

Add runtime methods to `src/web/runtime.rs`:

```rust
pub fn list_tasks(&self) -> Result<TaskListResponse, TaskRunError> {
    let tasks_root = self.workspace_root.join(".aria/runtime/tasks");
    let mut tasks = Vec::new();
    for entry in std::fs::read_dir(&tasks_root).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        if !entry.file_type().map_err(io_error)?.is_dir() {
            continue;
        }
        let task_id = entry.file_name().to_string_lossy().to_string();
        let state = read_optional_json(&entry.path().join("state.json"))?;
        tasks.push(TaskListItem {
            task_id,
            change_id: state.get("change_id").and_then(|value| value.as_str()).map(str::to_string),
            phase: state.get("phase").and_then(|value| value.as_str()).map(str::to_string),
            updated_at: None,
        });
    }
    tasks.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    Ok(TaskListResponse { tasks })
}

pub fn artifact_content(&self, artifact_ref: &str) -> Result<ArtifactContentResponse, TaskRunError> {
    let projection = self.projection(None, None)?;
    let entry = projection
        .artifact_index
        .iter()
        .find(|entry| entry.artifact_ref == artifact_ref)
        .ok_or_else(|| TaskRunError::new("artifact_not_found", format!("artifact not found: {artifact_ref}")))?;
    let path = self.workspace_root.join(&entry.path);
    let content = std::fs::read_to_string(&path).map_err(io_error)?;
    Ok(ArtifactContentResponse {
        artifact_ref: entry.artifact_ref.clone(),
        artifact_kind: entry.artifact_kind.clone(),
        producer_node: entry.producer_node.clone(),
        path: entry.path.clone(),
        content_type: format!("{:?}", entry.content_type).to_lowercase(),
        content,
    })
}

pub fn file_content(&self, path: &str) -> Result<FileContentResponse, TaskRunError> {
    let safe = safe_workspace_path(&self.workspace_root, path)?;
    Ok(FileContentResponse {
        path: path.to_string(),
        content_type: content_type_for_path(path),
        content: std::fs::read_to_string(safe).map_err(io_error)?,
    })
}

pub fn file_diff(&self, base_checkpoint: &str, path: &str) -> Result<FileDiffResponse, TaskRunError> {
    let diff = std::process::Command::new("git")
        .args(["diff", base_checkpoint, "--", path])
        .current_dir(&self.workspace_root)
        .output()
        .map_err(|error| TaskRunError::new("git_command_failed", error.to_string()))?;
    Ok(FileDiffResponse {
        base_checkpoint: base_checkpoint.to_string(),
        path: path.to_string(),
        diff: String::from_utf8_lossy(&diff.stdout).to_string(),
    })
}
```

Add safe path helpers in the same file:

```rust
fn safe_workspace_path(root: &std::path::Path, path: &str) -> Result<std::path::PathBuf, TaskRunError> {
    if path.contains("..") || path.starts_with('/') {
        return Err(TaskRunError::new("invalid_file_path", format!("unsafe path: {path}")));
    }
    Ok(root.join(path))
}

fn content_type_for_path(path: &str) -> String {
    if path.ends_with(".md") {
        "markdown".to_string()
    } else if path.ends_with(".json") {
        "json".to_string()
    } else if path.contains("/tests/") || path.contains(".test.") || path.contains(".spec.") {
        "test".to_string()
    } else {
        "source".to_string()
    }
}

fn read_optional_json(path: &std::path::Path) -> Result<serde_json::Value, TaskRunError> {
    match std::fs::File::open(path) {
        Ok(file) => serde_json::from_reader(file)
            .map_err(|error| TaskRunError::new("web_runtime_json", error.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(error) => Err(io_error(error)),
    }
}
```

Add routes in `src/web/app.rs`:

```rust
.route("/api/tasks", get(handlers::list_tasks).post(handlers::create_task))
.route("/api/artifacts/{artifact_ref}", get(handlers::artifact_content))
.route("/api/files/content", get(handlers::file_content))
.route("/api/files/diff", get(handlers::file_diff))
```

Add handlers with query structs:

```rust
#[derive(Debug, Deserialize)]
pub struct FileContentQuery {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct FileDiffQuery {
    pub base_checkpoint: String,
    pub path: String,
}

pub async fn list_tasks(State(state): State<WebAppState>) -> ApiResult<Json<TaskListResponse>> {
    let runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(runtime.list_tasks()?))
}

pub async fn artifact_content(
    State(state): State<WebAppState>,
    Path(artifact_ref): Path<String>,
) -> ApiResult<Json<ArtifactContentResponse>> {
    let runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(runtime.artifact_content(&artifact_ref)?))
}

pub async fn file_content(
    State(state): State<WebAppState>,
    Query(query): Query<FileContentQuery>,
) -> ApiResult<Json<FileContentResponse>> {
    let runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(runtime.file_content(&query.path)?))
}

pub async fn file_diff(
    State(state): State<WebAppState>,
    Query(query): Query<FileDiffQuery>,
) -> ApiResult<Json<FileDiffResponse>> {
    let runtime = state.runtime.lock().expect("runtime lock");
    Ok(Json(runtime.file_diff(&query.base_checkpoint, &query.path)?))
}
```

- [ ] **Step 6: Add frontend client methods and viewer**

Add TypeScript types:

```ts
export type ArtifactContentResponse = {
  artifact_ref: string;
  artifact_kind: string;
  producer_node: string | null;
  path: string;
  content_type: "markdown" | "json" | "source" | "test" | "log" | "unknown";
  content: string;
};
```

Add client methods:

```ts
export function listTasks() {
  return requestJson<{ tasks: Array<{ task_id: string; change_id: string | null; phase: string | null }> }>("/api/tasks");
}

export function getArtifactContent(artifactRef: string) {
  return requestJson<ArtifactContentResponse>(`/api/artifacts/${encodeURIComponent(artifactRef)}`);
}

export function getFileContent(path: string) {
  return requestJson<{ path: string; content_type: string; content: string }>(
    `/api/files/content?path=${encodeURIComponent(path)}`
  );
}

export function getFileDiff(baseCheckpoint: string, path: string) {
  return requestJson<{ base_checkpoint: string; path: string; diff: string }>(
    `/api/files/diff?base_checkpoint=${encodeURIComponent(baseCheckpoint)}&path=${encodeURIComponent(path)}`
  );
}
```

Create `web/src/components/evidence/ArtifactViewer.tsx`:

```tsx
import type { ArtifactContentResponse } from "../../api/types";

export function ArtifactViewer({ artifact }: { artifact: ArtifactContentResponse | null }) {
  if (!artifact) {
    return <div className="text-sm text-slate-500">未选择 artifact。</div>;
  }
  return (
    <section className="rounded-md border border-line bg-white">
      <header className="border-b border-line px-3 py-2">
        <h3 className="text-sm font-semibold">{artifact.artifact_kind}</h3>
        <p className="truncate text-xs text-slate-500">
          {artifact.producer_node ?? "unknown node"} · {artifact.path}
        </p>
      </header>
      <pre className="max-h-[34rem] overflow-auto p-3 text-xs leading-5">
        {artifact.content}
      </pre>
    </section>
  );
}
```

- [ ] **Step 7: Run resource API and viewer tests**

Run:

```bash
cargo test --test web_resource_handlers --locked
pnpm --dir web test -- --run web/src/components/evidence/ArtifactViewer.test.tsx
pnpm --dir web build
```

Expected: PASS for backend resource APIs, frontend artifact viewer and TypeScript build.

- [ ] **Step 8: Commit**

```bash
git add src/web/types.rs src/web/runtime.rs src/web/handlers.rs src/web/app.rs tests/web_resource_handlers.rs web/src/api/types.ts web/src/api/client.ts web/src/components/evidence/ArtifactViewer.tsx web/src/components/evidence/ArtifactViewer.test.tsx
git commit -m "feat: add web task and resource APIs"
```

## Task 7.6: Provider Output Stream, Run Diagnostics, And Stop Signal

**Files:**
- Modify: `src/web/types.rs`
- Modify: `src/web/events.rs`
- Modify: `src/web/runtime.rs`
- Modify: `src/web/handlers.rs`
- Modify: `src/web/app.rs`
- Modify: `src/interactive/models.rs`
- Modify: `src/interactive/web_projection.rs`
- Modify: `web/src/api/types.ts`
- Modify: `web/src/api/client.ts`
- Create: `web/src/components/node/NodeRunPanel.tsx`
- Modify: `web/src/components/node/NodeWorkspace.tsx`
- Test: `tests/web_provider_output_events.rs`
- Test: `web/src/components/node/NodeRunPanel.test.tsx`

- [ ] **Step 1: Write failing backend provider output tests**

Create `tests/web_provider_output_events.rs`:

```rust
use cadence_aria::web::events::EventHub;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::types::ProviderOutputChunk;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn provider_output_event_carries_stdout_stderr_structured_output_gate_and_retry() {
    let hub = EventHub::new();
    let event = hub.publish_provider_output(
        Some("task_0001"),
        ProviderOutputChunk {
            node_id: "N16".to_string(),
            provider_run_id: "run_n16_0001".to_string(),
            stream: "stdout".to_string(),
            text: "running tests".to_string(),
            structured_output: Some(json!({"artifact_kind":"coding_report"})),
            manual_gate: Some("approval_required".to_string()),
            retry_attempt: Some(1),
        },
    );

    assert_eq!(event.event_type, "provider_output");
    assert_eq!(event.payload["stream"], "stdout");
    assert_eq!(event.payload["structured_output"]["artifact_kind"], "coding_report");
    assert_eq!(event.payload["manual_gate"], "approval_required");
    assert_eq!(event.payload["retry_attempt"], 1);
}

#[test]
fn provider_auth_failure_is_classified_for_diagnostics_panel() {
    let workspace = tempdir().expect("workspace");
    let runtime = WebRuntime::new_fake(workspace.path().to_path_buf());
    let diagnostic = runtime.provider_command_diagnostic(
        "codex",
        "command not found or not authenticated",
    );

    assert_eq!(diagnostic["category"], "provider_error");
    assert_eq!(diagnostic["code"], "provider_authorization_or_command_unavailable");
    assert!(diagnostic["message"].as_str().expect("message").contains("codex"));
}
```

- [ ] **Step 2: Write failing frontend run panel test**

Create `web/src/components/node/NodeRunPanel.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { NodeRunPanel } from "./NodeRunPanel";

describe("NodeRunPanel", () => {
  it("renders stdout stderr structured output manual gate and retry status", () => {
    render(<NodeRunPanel runItems={[
      {
        kind: "provider_output",
        node_id: "N16",
        provider_run_id: "run_n16_0001",
        stream: "stdout",
        text: "running tests",
        structured_output: { artifact_kind: "coding_report" },
        manual_gate: "approval_required",
        retry_attempt: 1
      },
      {
        kind: "provider_output",
        node_id: "N16",
        provider_run_id: "run_n16_0001",
        stream: "stderr",
        text: "warning: retrying",
        retry_attempt: 2
      }
    ]} />);

    expect(screen.getByText("stdout")).toBeInTheDocument();
    expect(screen.getByText("stderr")).toBeInTheDocument();
    expect(screen.getByText(/running tests/)).toBeInTheDocument();
    expect(screen.getByText(/coding_report/)).toBeInTheDocument();
    expect(screen.getByText(/approval_required/)).toBeInTheDocument();
    expect(screen.getByText(/retry 2/)).toBeInTheDocument();
  });
});
```

- [ ] **Step 3: Run tests and verify failure**

Run:

```bash
cargo test --test web_provider_output_events --locked
pnpm --dir web test -- --run web/src/components/node/NodeRunPanel.test.tsx
```

Expected: FAIL because provider output DTOs, diagnostics helper and run panel are missing.

- [ ] **Step 4: Add provider output DTOs and event helper**

Add to `src/web/types.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderOutputChunk {
    pub node_id: String,
    pub provider_run_id: String,
    pub stream: String,
    pub text: String,
    pub structured_output: Option<serde_json::Value>,
    pub manual_gate: Option<String>,
    pub retry_attempt: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StopTaskResponse {
    pub status: String,
    pub task_id: String,
}
```

Add to `src/web/events.rs`:

```rust
pub fn publish_provider_output(
    &self,
    task_id: Option<&str>,
    chunk: crate::web::types::ProviderOutputChunk,
) -> WebEvent {
    let payload = serde_json::to_value(chunk).expect("provider output chunk");
    self.publish("provider_output", task_id, payload)
}
```

- [ ] **Step 5: Add runtime diagnostics and stop signal**

Add to `src/web/runtime.rs`:

```rust
pub fn provider_command_diagnostic(
    &self,
    provider_type: &str,
    message: &str,
) -> serde_json::Value {
    serde_json::json!({
        "category": "provider_error",
        "code": "provider_authorization_or_command_unavailable",
        "provider_type": provider_type,
        "message": format!("{provider_type} provider unavailable: {message}"),
        "details": {
            "action": "check provider CLI installation, authentication, and PATH"
        }
    })
}

pub fn stop_task(&mut self, task_id: &str) -> Result<StopTaskResponse, TaskRunError> {
    Ok(StopTaskResponse {
        status: "stop_requested".to_string(),
        task_id: task_id.to_string(),
    })
}
```

Add route in `src/web/app.rs`:

```rust
.route("/api/tasks/{task_id}/stop", post(handlers::stop_task))
```

Add handler in `src/web/handlers.rs`:

```rust
pub async fn stop_task(
    State(state): State<WebAppState>,
    Path(task_id): Path<String>,
) -> ApiResult<Json<StopTaskResponse>> {
    let mut runtime = state.runtime.lock().expect("runtime lock");
    let response = runtime.stop_task(&task_id)?;
    state.events.publish("stop_requested", Some(&task_id), serde_json::json!({ "task_id": task_id }));
    Ok(Json(response))
}
```

- [ ] **Step 6: Include provider output in selected node context**

Modify `src/interactive/web_projection.rs` so `selected_node_context.run` includes `provider_output` entries from `.aria/runtime/tasks/<task_id>/logs/provider-output.jsonl`:

```rust
fn read_provider_output(task_root: &Path, selected_node_id: Option<&str>) -> Result<Vec<Value>, TaskRunError> {
    let path = task_root.join("logs/provider-output.jsonl");
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(TaskRunError::new("interactive_projection_io", error.to_string())),
    };
    let mut items = Vec::new();
    for line in std::io::BufReader::new(file).lines() {
        let value: Value = serde_json::from_str(&line.map_err(|error| {
            TaskRunError::new("interactive_projection_io", error.to_string())
        })?)
        .map_err(|error| TaskRunError::new("interactive_projection_json", error.to_string()))?;
        if selected_node_id.is_none() || value.get("node_id").and_then(Value::as_str) == selected_node_id {
            items.push(value);
        }
    }
    Ok(items)
}
```

- [ ] **Step 7: Add frontend run panel and stop client**

Add to `web/src/api/types.ts`:

```ts
export type ProviderOutputChunk = {
  node_id: string;
  provider_run_id: string;
  stream: "stdout" | "stderr";
  text: string;
  structured_output?: unknown;
  manual_gate?: string;
  retry_attempt?: number;
};
```

Add to `web/src/api/client.ts`:

```ts
export function stopTask(taskId: string) {
  return requestJson<{ status: string; task_id: string }>(
    `/api/tasks/${encodeURIComponent(taskId)}/stop`,
    { method: "POST", body: JSON.stringify({}) }
  );
}
```

Create `web/src/components/node/NodeRunPanel.tsx`:

```tsx
export function NodeRunPanel({ runItems }: { runItems: Array<Record<string, unknown>> }) {
  return (
    <section className="space-y-2">
      {runItems.map((item, index) => (
        <article key={index} className="rounded-md border border-line bg-white p-3">
          <div className="flex items-center gap-2 text-xs font-semibold text-slate-500">
            <span>{String(item.stream ?? item.kind ?? "run")}</span>
            {item.retry_attempt ? <span>retry {String(item.retry_attempt)}</span> : null}
            {item.manual_gate ? <span>{String(item.manual_gate)}</span> : null}
          </div>
          <pre className="mt-2 whitespace-pre-wrap text-xs">{String(item.text ?? "")}</pre>
          {item.structured_output ? (
            <pre className="mt-2 overflow-auto rounded bg-slate-50 p-2 text-xs">
              {JSON.stringify(item.structured_output, null, 2)}
            </pre>
          ) : null}
        </article>
      ))}
    </section>
  );
}
```

Modify `NodeWorkspace.tsx` so the `Run` tab renders `NodeRunPanel` with `selected_node_context.run`.

- [ ] **Step 8: Run provider output tests**

Run:

```bash
cargo test --test web_provider_output_events --locked
pnpm --dir web test -- --run web/src/components/node/NodeRunPanel.test.tsx
pnpm --dir web build
```

Expected: PASS backend event/diagnostics test, frontend run panel test and build.

- [ ] **Step 9: Commit**

```bash
git add src/web/types.rs src/web/events.rs src/web/runtime.rs src/web/handlers.rs src/web/app.rs src/interactive/models.rs src/interactive/web_projection.rs tests/web_provider_output_events.rs web/src/api/types.ts web/src/api/client.ts web/src/components/node/NodeRunPanel.tsx web/src/components/node/NodeWorkspace.tsx web/src/components/node/NodeRunPanel.test.tsx
git commit -m "feat: add provider output diagnostics stream"
```

## Task 8: `aria web` CLI Entry And Static Asset Serving

**Files:**
- Create: `src/web/static_assets.rs`
- Modify: `src/web/app.rs`
- Modify: `src/web/mod.rs`
- Modify: `src/cli.rs`
- Test: `tests/web_cli.rs`

- [ ] **Step 1: Write failing CLI tests**

Create `tests/web_cli.rs`:

```rust
use cadence_aria::cli::{run_cli, run_cli_async, CliOutput};
use tempfile::tempdir;

#[test]
fn web_check_reports_workspace_and_bind_address() {
    let workspace = tempdir().expect("workspace");
    let output = run_cli([
        "web",
        "--workspace",
        workspace.path().to_str().expect("path"),
        "--host",
        "127.0.0.1",
        "--port",
        "4317",
        "--check",
    ])
    .expect("cli");
    assert_eq!(
        output,
        CliOutput::Text(format!("web_check_ok:{}:127.0.0.1:4317", workspace.path().display()))
    );
}

#[tokio::test]
async fn async_web_check_uses_same_parser() {
    let workspace = tempdir().expect("workspace");
    let output = run_cli_async([
        "web",
        "--workspace",
        workspace.path().to_str().expect("path"),
        "--check",
    ])
    .await
    .expect("cli");
    assert!(matches!(output, CliOutput::Text(text) if text.starts_with("web_check_ok:")));
}
```

- [ ] **Step 2: Run the tests and verify failure**

Run:

```bash
cargo test --test web_cli --locked
```

Expected: FAIL because `web` command is not parsed.

- [ ] **Step 3: Add CLI parsing**

Modify `src/cli.rs`:

```rust
[command, rest @ ..] if command == "web" => {
    let options = parse_web_options(rest)?;
    if options.check {
        return Ok(CliOutput::Text(format!(
            "web_check_ok:{}:{}:{}",
            options.workspace.to_string_lossy(),
            options.host,
            options.port
        )));
    }
    Err(CliError {
        code: "web_requires_async".to_string(),
        message: "web server is only available through run_cli_async".to_string(),
    })
}
```

Add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
struct WebOptions {
    workspace: PathBuf,
    host: String,
    port: u16,
    check: bool,
}

fn parse_web_options(args: &[String]) -> Result<WebOptions, CliError> {
    let workspace = parse_workspace(args)?;
    let host = parse_value(args, "--host").unwrap_or_else(|| "127.0.0.1".to_string());
    let port = parse_value(args, "--port")
        .map(|value| value.parse::<u16>())
        .transpose()
        .map_err(|error| CliError {
            code: "invalid_cli_args".to_string(),
            message: format!("--port must be a u16: {error}"),
        })?
        .unwrap_or(4317);
    Ok(WebOptions {
        workspace,
        host,
        port,
        check: args.iter().any(|item| item == "--check"),
    })
}

fn parse_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].clone())
}
```

In `run_cli_async`, add async server branch:

```rust
[command, rest @ ..] if command == "web" => {
    let options = parse_web_options(rest)?;
    if options.check {
        return run_cli(args);
    }
    crate::web::app::serve_web(options.workspace, options.host, options.port)
        .await
        .map_err(internal_error)?;
    Ok(CliOutput::Text(String::new()))
}
```

- [ ] **Step 4: Add static assets and server entry**

Create `src/web/static_assets.rs`:

```rust
use std::path::PathBuf;

use tower_http::services::{ServeDir, ServeFile};

pub fn static_dist_service() -> ServeDir<ServeFile> {
    let dist = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/dist");
    let index = dist.join("index.html");
    ServeDir::new(dist).fallback(ServeFile::new(index))
}
```

Modify `src/web/app.rs`:

```rust
use std::net::SocketAddr;
use tokio::net::TcpListener;

pub async fn serve_web(
    workspace_root: std::path::PathBuf,
    host: String,
    port: u16,
) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    let state = WebAppState::new(
        workspace_root.clone(),
        crate::web::runtime::WebRuntime::new_fake(workspace_root),
    );
    let app = build_web_router(state).fallback_service(crate::web::static_assets::static_dist_service());
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

Modify `src/web/mod.rs`:

```rust
pub mod static_assets;
```

- [ ] **Step 5: Run CLI tests**

Run:

```bash
cargo test --test web_cli --locked
cargo check --locked
```

Expected: PASS for CLI tests and compile check.

- [ ] **Step 6: Commit**

```bash
git add src/cli.rs src/web/app.rs src/web/static_assets.rs src/web/mod.rs tests/web_cli.rs
git commit -m "feat: add aria web cli entry"
```

## Task 9: Frontend Scaffold And Design System Baseline

**Files:**
- Create: `web/package.json`
- Create: `web/vite.config.ts`
- Create: `web/tsconfig.json`
- Create: `web/tailwind.config.ts`
- Create: `web/postcss.config.js`
- Create: `web/index.html`
- Create: `web/src/main.tsx`
- Create: `web/src/styles.css`
- Create: `web/src/test/setup.ts`
- Test: `web/src/main.test.tsx`

- [ ] **Step 1: Create failing smoke test**

Create `web/src/main.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AppShell } from "./main";

describe("AppShell", () => {
  it("renders the first-screen workbench shell", () => {
    render(<AppShell />);
    expect(screen.getByRole("banner")).toHaveTextContent("Aria Web");
    expect(screen.getByRole("navigation", { name: "Node flow" })).toBeInTheDocument();
    expect(screen.getByRole("main")).toHaveTextContent("Node Workspace");
  });
});
```

- [ ] **Step 2: Add frontend package and run failing test**

Create `web/package.json`:

```json
{
  "name": "aria-web-workbench",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite --host 127.0.0.1",
    "build": "tsc -b && vite build",
    "test": "vitest",
    "test:e2e": "playwright test"
  },
  "dependencies": {
    "@radix-ui/react-dialog": "^1.1.0",
    "@radix-ui/react-tabs": "^1.1.0",
    "@tanstack/react-router": "^1.120.0",
    "clsx": "^2.1.1",
    "lucide-react": "^0.468.0",
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "tailwind-merge": "^2.5.0"
  },
  "devDependencies": {
    "@playwright/test": "^1.49.0",
    "@testing-library/jest-dom": "^6.6.0",
    "@testing-library/react": "^16.1.0",
    "@testing-library/user-event": "^14.5.0",
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "@vitejs/plugin-react": "^4.3.0",
    "autoprefixer": "^10.4.0",
    "postcss": "^8.4.0",
    "tailwindcss": "^3.4.0",
    "typescript": "^5.7.0",
    "vite": "^6.0.0",
    "vitest": "^2.1.0"
  }
}
```

Run:

```bash
pnpm --dir web install
pnpm --dir web test -- --run
```

Expected: FAIL because `web/src/main.tsx` and config files are missing.

- [ ] **Step 3: Add Vite, TypeScript, Tailwind and test config**

Create `web/vite.config.ts`:

```ts
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/api": "http://127.0.0.1:4317"
    }
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"]
  }
});
```

Create `web/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "useDefineForClassFields": true,
    "lib": ["DOM", "DOM.Iterable", "ES2022"],
    "allowJs": false,
    "skipLibCheck": true,
    "esModuleInterop": true,
    "allowSyntheticDefaultImports": true,
    "strict": true,
    "forceConsistentCasingInFileNames": true,
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "jsx": "react-jsx"
  },
  "include": ["src"]
}
```

Create `web/tailwind.config.ts`:

```ts
import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: "#101418",
        panel: "#f6f8f9",
        line: "#d8e0e6",
        signal: "#14b8a6",
        caution: "#f59e0b",
        danger: "#dc2626"
      }
    }
  },
  plugins: []
} satisfies Config;
```

Create `web/postcss.config.js`:

```js
export default {
  plugins: {
    tailwindcss: {},
    autoprefixer: {}
  }
};
```

Create `web/src/test/setup.ts`:

```ts
import "@testing-library/jest-dom/vitest";
```

- [ ] **Step 4: Add first-screen shell**

Create `web/index.html`:

```html
<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Aria Web</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

Create `web/src/main.tsx`:

```tsx
import React from "react";
import ReactDOM from "react-dom/client";
import "./styles.css";

export function AppShell() {
  return (
    <div className="min-h-screen bg-[#eef3f4] text-ink">
      <header role="banner" className="h-12 border-b border-line bg-white px-4 flex items-center justify-between">
        <strong>Aria Web</strong>
        <span className="text-sm text-slate-600">single workspace</span>
      </header>
      <div className="grid min-h-[calc(100vh-3rem)] grid-cols-[18rem_minmax(0,1fr)_24rem]">
        <nav aria-label="Node flow" className="border-r border-line bg-panel p-3">
          <span className="text-xs font-semibold uppercase tracking-[0.12em] text-slate-500">Flow</span>
        </nav>
        <main className="p-4">
          <h1 className="text-xl font-semibold">Node Workspace</h1>
        </main>
        <aside className="border-l border-line bg-white p-3">
          <span className="text-sm font-medium">Evidence</span>
        </aside>
      </div>
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <AppShell />
  </React.StrictMode>
);
```

Create `web/src/styles.css`:

```css
@tailwind base;
@tailwind components;
@tailwind utilities;

:root {
  font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  color-scheme: light;
}

body {
  margin: 0;
}
```

- [ ] **Step 5: Run frontend smoke test and build**

Run:

```bash
pnpm --dir web test -- --run
pnpm --dir web build
```

Expected: PASS smoke test and `web/dist` is generated.

- [ ] **Step 6: Commit**

```bash
git add web/package.json web/pnpm-lock.yaml web/vite.config.ts web/tsconfig.json web/tailwind.config.ts web/postcss.config.js web/index.html web/src/main.tsx web/src/styles.css web/src/test/setup.ts web/src/main.test.tsx
git commit -m "feat: scaffold aria web frontend"
```

## Task 10: Frontend API Client, Router, And Store

**Files:**
- Create: `web/src/api/types.ts`
- Create: `web/src/api/client.ts`
- Create: `web/src/state/workbench-store.ts`
- Create: `web/src/router.tsx`
- Modify: `web/src/main.tsx`
- Test: `web/src/api/client.test.ts`
- Test: `web/src/state/workbench-store.test.ts`

- [ ] **Step 1: Write failing API client and store tests**

Create `web/src/api/client.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import { createTask, normalizeApiError } from "./client";

describe("api client", () => {
  it("posts create task payload and returns task response", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({
      task_id: "task_0001",
      session_id: "sess_task_0001",
      change_id: "aria-fibonacci-square",
      phase: "intake"
    }), { status: 200 })));

    const result = await createTask({
      request_text: "实现 Fibonacci square sum",
      change_id: "aria-fibonacci-square",
      policy_preset: "manual-write",
      provider_mode: "fake",
      timeout_secs: 2400
    });

    expect(result.task_id).toBe("task_0001");
  });

  it("normalizes standard api error", async () => {
    const error = await normalizeApiError(new Response(JSON.stringify({
      code: "checkpoint_unsafe_dirty_worktree",
      message: "worktree has uncommitted changes",
      details: {}
    }), { status: 409 }));
    expect(error.code).toBe("checkpoint_unsafe_dirty_worktree");
  });
});
```

Create `web/src/state/workbench-store.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { createWorkbenchStore } from "./workbench-store";

describe("workbench store", () => {
  it("tracks projection, selected node, tab and event log", () => {
    const store = createWorkbenchStore();
    store.setProjection({
      workspace_root: "/tmp/workspace",
      active_task_id: "task_0001",
      active_session_id: "sess_task_0001",
      overview: { phase: "execution" },
      sessions: [],
      timeline: [{ node_id: "N16", status: "completed" }],
      artifact_index: [],
      diagnostics: [],
      available_actions: ["confirm_provider_step"],
      pending_provider_step: null,
      selected_node_context: { node_id: "N16", overview: {}, inputs: [], run: [], outputs: [], diffs: [] },
      git_summary: { workspace_path: "/tmp/workspace", branch: "main", head: "abc1234", dirty: false, dirty_files: [] },
      event_cursor: 3
    });
    store.selectNode("N17");
    store.selectTab("outputs");
    store.pushEvent({ cursor: 4, event_type: "projection_updated", task_id: "task_0001", payload: {} });

    expect(store.snapshot.selectedNodeId).toBe("N17");
    expect(store.snapshot.selectedTab).toBe("outputs");
    expect(store.snapshot.events).toHaveLength(1);
  });
});
```

- [ ] **Step 2: Run tests and verify failure**

Run:

```bash
pnpm --dir web test -- --run web/src/api/client.test.ts web/src/state/workbench-store.test.ts
```

Expected: FAIL because modules are missing.

- [ ] **Step 3: Add TypeScript API types and client**

Create `web/src/api/types.ts` mirroring Rust snake_case JSON:

```ts
export type ApiError = {
  code: string;
  message: string;
  details: Record<string, unknown>;
};

export type CreateTaskRequest = {
  request_text: string;
  change_id: string;
  policy_preset: string;
  provider_mode: string;
  timeout_secs: number;
};

export type CreateTaskResponse = {
  task_id: string;
  session_id: string;
  change_id: string;
  phase: string;
};

export type PendingProviderStep = {
  node_id: string;
  provider_type: string;
  runtime_role: string;
  adapter_role: string;
  prompt: string;
  input_summary: unknown;
  output_schema: string;
  allowed_write_scope: string[];
  forbidden_actions: string[];
  verification_commands: string[];
  checkpoint_id: string;
};

export type WebWorkspaceProjection = {
  workspace_root: string;
  active_task_id: string | null;
  active_session_id: string | null;
  overview: Record<string, unknown>;
  sessions: unknown[];
  timeline: Array<Record<string, unknown>>;
  artifact_index: Array<Record<string, unknown>>;
  diagnostics: Array<Record<string, unknown>>;
  available_actions: string[];
  pending_provider_step: PendingProviderStep | null;
  selected_node_context: {
    node_id: string | null;
    overview: Record<string, unknown>;
    inputs: unknown[];
    run: unknown[];
    outputs: unknown[];
    diffs: unknown[];
  };
  git_summary: {
    workspace_path: string;
    branch: string | null;
    head: string | null;
    dirty: boolean;
    dirty_files: string[];
  };
  event_cursor: number;
};

export type WebEvent = {
  cursor: number;
  event_type: string;
  task_id: string | null;
  payload: unknown;
};
```

Create `web/src/api/client.ts`:

```ts
import type { ApiError, CreateTaskRequest, CreateTaskResponse, WebWorkspaceProjection } from "./types";

export async function normalizeApiError(response: Response): Promise<ApiError> {
  const body = await response.json().catch(() => ({}));
  return {
    code: typeof body.code === "string" ? body.code : "web_client_error",
    message: typeof body.message === "string" ? body.message : response.statusText,
    details: typeof body.details === "object" && body.details !== null ? body.details : {}
  };
}

async function requestJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: {
      "content-type": "application/json",
      ...(init?.headers ?? {})
    }
  });
  if (!response.ok) {
    throw await normalizeApiError(response);
  }
  return response.json() as Promise<T>;
}

export function createTask(payload: CreateTaskRequest): Promise<CreateTaskResponse> {
  return requestJson<CreateTaskResponse>("/api/tasks", {
    method: "POST",
    body: JSON.stringify(payload)
  });
}

export function getProjection(taskId?: string, nodeId?: string): Promise<WebWorkspaceProjection> {
  const params = new URLSearchParams();
  if (taskId) params.set("task_id", taskId);
  if (nodeId) params.set("node_id", nodeId);
  return requestJson<WebWorkspaceProjection>(`/api/projection?${params.toString()}`);
}
```

- [ ] **Step 4: Add store and router**

Create `web/src/state/workbench-store.ts`:

```ts
import type { WebEvent, WebWorkspaceProjection } from "../api/types";

export type WorkbenchTab = "overview" | "inputs" | "run" | "outputs" | "diff";

export type WorkbenchSnapshot = {
  projection: WebWorkspaceProjection | null;
  selectedNodeId: string | null;
  selectedTab: WorkbenchTab;
  events: WebEvent[];
};

export function createWorkbenchStore() {
  const snapshot: WorkbenchSnapshot = {
    projection: null,
    selectedNodeId: null,
    selectedTab: "overview",
    events: []
  };
  return {
    snapshot,
    setProjection(projection: WebWorkspaceProjection) {
      snapshot.projection = projection;
      snapshot.selectedNodeId = projection.selected_node_context.node_id;
    },
    selectNode(nodeId: string) {
      snapshot.selectedNodeId = nodeId;
    },
    selectTab(tab: WorkbenchTab) {
      snapshot.selectedTab = tab;
    },
    pushEvent(event: WebEvent) {
      snapshot.events = [...snapshot.events.slice(-199), event];
    }
  };
}
```

Create `web/src/router.tsx`:

```tsx
import { createRootRoute, createRoute, createRouter, RouterProvider } from "@tanstack/react-router";
import { AppShell } from "./main";

const rootRoute = createRootRoute({ component: AppShell });
const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: AppShell
});

const routeTree = rootRoute.addChildren([indexRoute]);
export const router = createRouter({ routeTree });

export function AppRouter() {
  return <RouterProvider router={router} />;
}
```

Modify `web/src/main.tsx` to render `AppRouter`.

- [ ] **Step 5: Run frontend tests**

Run:

```bash
pnpm --dir web test -- --run web/src/api/client.test.ts web/src/state/workbench-store.test.ts
pnpm --dir web build
```

Expected: PASS tests and TypeScript build.

- [ ] **Step 6: Commit**

```bash
git add web/src/api/types.ts web/src/api/client.ts web/src/state/workbench-store.ts web/src/router.tsx web/src/main.tsx web/src/api/client.test.ts web/src/state/workbench-store.test.ts
git commit -m "feat: add web frontend api store"
```

## Task 11: Workbench Layout Components

**Files:**
- Create: `web/src/components/shell/TopStatusBar.tsx`
- Create: `web/src/components/shell/TaskSwitcher.tsx`
- Create: `web/src/components/flow/FlowRail.tsx`
- Create: `web/src/components/node/NodeWorkspace.tsx`
- Create: `web/src/components/evidence/EvidencePanel.tsx`
- Create: `web/src/components/diagnostics/DiagnosticsPanel.tsx`
- Modify: `web/src/main.tsx`
- Test: `web/src/components/shell/TaskSwitcher.test.tsx`
- Test: `web/src/components/flow/FlowRail.test.tsx`
- Test: `web/src/components/evidence/EvidencePanel.test.tsx`

- [ ] **Step 1: Write failing component tests**

Create `web/src/components/flow/FlowRail.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { FlowRail } from "./FlowRail";

describe("FlowRail", () => {
  it("renders node state provider badge and dropped history", () => {
    render(<FlowRail timeline={[
      { node_id: "N16", status: "completed", provider_type: "codex", dropped: false },
      { node_id: "N17", status: "dropped", provider_type: "codex", dropped: true }
    ]} selectedNodeId="N16" onSelectNode={() => undefined} />);
    expect(screen.getByRole("button", { name: /N16/ })).toHaveTextContent("completed");
    expect(screen.getByRole("button", { name: /N17/ })).toHaveAttribute("data-dropped", "true");
  });
});
```

Create `web/src/components/evidence/EvidencePanel.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { EvidencePanel } from "./EvidencePanel";

describe("EvidencePanel", () => {
  it("groups artifacts and diagnostics for the selected node", () => {
    render(<EvidencePanel artifacts={[
      { artifact_ref: "coding_report_work_wt_001_0001", artifact_kind: "coding_report", producer_node: "N16", path: ".aria/report.json", content_type: "json", dropped: false }
    ]} diagnostics={[
      { code: "gate_blocked", message: "archive worktask failed", node_id: "N18" }
    ]} />);
    expect(screen.getByText("coding_report")).toBeInTheDocument();
    expect(screen.getByText("archive worktask failed")).toBeInTheDocument();
  });
});
```

Create `web/src/components/shell/TaskSwitcher.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { TaskSwitcher } from "./TaskSwitcher";

describe("TaskSwitcher", () => {
  it("renders existing tasks and selects one to continue", async () => {
    const onSelectTask = vi.fn();
    render(<TaskSwitcher tasks={[
      { task_id: "task_0001", change_id: "aria-fibonacci-square", phase: "blocked_by_gate" },
      { task_id: "task_0002", change_id: "aria-login-jwt", phase: "execution" }
    ]} activeTaskId="task_0001" onSelectTask={onSelectTask} />);

    await userEvent.selectOptions(screen.getByLabelText("继续任务"), "task_0002");
    expect(onSelectTask).toHaveBeenCalledWith("task_0002");
  });
});
```

- [ ] **Step 2: Run tests and verify failure**

Run:

```bash
pnpm --dir web test -- --run web/src/components/shell/TaskSwitcher.test.tsx web/src/components/flow/FlowRail.test.tsx web/src/components/evidence/EvidencePanel.test.tsx
```

Expected: FAIL because components do not exist.

- [ ] **Step 3: Implement TopStatusBar, TaskSwitcher and FlowRail**

Create `TopStatusBar.tsx` with workspace, task, phase, provider, git and SSE labels. Create `TaskSwitcher.tsx`:

```tsx
export function TaskSwitcher({
  tasks,
  activeTaskId,
  onSelectTask
}: {
  tasks: Array<{ task_id: string; change_id: string | null; phase: string | null }>;
  activeTaskId: string | null;
  onSelectTask: (taskId: string) => void;
}) {
  return (
    <label className="flex items-center gap-2 text-sm">
      <span className="text-slate-500">继续任务</span>
      <select
        aria-label="继续任务"
        value={activeTaskId ?? ""}
        onChange={(event) => onSelectTask(event.target.value)}
        className="rounded-md border border-line bg-white px-2 py-1"
      >
        {tasks.map((task) => (
          <option key={task.task_id} value={task.task_id}>
            {task.task_id} · {task.change_id ?? "no change"} · {task.phase ?? "unknown"}
          </option>
        ))}
      </select>
    </label>
  );
}
```

Create `FlowRail.tsx`:

```tsx
type TimelineItem = Record<string, unknown>;

export function FlowRail({
  timeline,
  selectedNodeId,
  onSelectNode
}: {
  timeline: TimelineItem[];
  selectedNodeId: string | null;
  onSelectNode: (nodeId: string) => void;
}) {
  const nodes = timeline.length > 0 ? timeline : Array.from({ length: 29 }, (_, index) => ({ node_id: `N${String(index).padStart(2, "0")}`, status: "idle" }));
  return (
    <nav aria-label="Node flow" className="border-r border-line bg-panel p-3">
      <div className="mb-3 text-xs font-semibold uppercase tracking-[0.12em] text-slate-500">Flow</div>
      <div className="space-y-1">
        {nodes.map((item) => {
          const nodeId = String(item.node_id ?? "unknown");
          const dropped = Boolean(item.dropped) || item.status === "dropped";
          return (
            <button
              key={nodeId}
              type="button"
              data-dropped={dropped ? "true" : "false"}
              aria-pressed={selectedNodeId === nodeId}
              onClick={() => onSelectNode(nodeId)}
              className="grid w-full grid-cols-[3.5rem_1fr] items-center gap-2 rounded-md px-2 py-2 text-left text-sm hover:bg-white aria-pressed:bg-white"
            >
              <span className="font-mono font-semibold">{nodeId}</span>
              <span className={dropped ? "text-slate-400 line-through" : "text-slate-700"}>
                {String(item.status ?? "idle")} {String(item.provider_type ?? "")}
              </span>
            </button>
          );
        })}
      </div>
    </nav>
  );
}
```

- [ ] **Step 4: Implement NodeWorkspace, EvidencePanel and DiagnosticsPanel**

Create `EvidencePanel.tsx`:

```tsx
export function EvidencePanel({
  artifacts,
  diagnostics
}: {
  artifacts: Array<Record<string, unknown>>;
  diagnostics: Array<Record<string, unknown>>;
}) {
  return (
    <aside className="border-l border-line bg-white p-3">
      <h2 className="text-sm font-semibold">Evidence</h2>
      <section className="mt-3">
        <h3 className="text-xs font-semibold uppercase text-slate-500">Artifacts</h3>
        {artifacts.map((artifact) => (
          <button key={String(artifact.artifact_ref)} className="mt-2 block w-full rounded-md border border-line px-2 py-2 text-left text-sm">
            <span className="font-medium">{String(artifact.artifact_kind)}</span>
            <span className="block truncate text-xs text-slate-500">{String(artifact.path)}</span>
          </button>
        ))}
      </section>
      <section className="mt-4">
        <h3 className="text-xs font-semibold uppercase text-slate-500">Diagnostics</h3>
        {diagnostics.map((diagnostic, index) => (
          <div key={index} className="mt-2 rounded-md border border-caution/40 bg-amber-50 px-2 py-2 text-sm">
            {String(diagnostic.message ?? diagnostic.code)}
          </div>
        ))}
      </section>
    </aside>
  );
}
```

Create `NodeWorkspace.tsx` with five tabs named `Overview`、`Inputs`、`Run`、`Outputs`、`Diff` and render selected node context arrays. Create `DiagnosticsPanel.tsx` grouping diagnostics by `provider_error`、`gate_blocked`、`validation_failed`、`checkpoint_unsafe`、`web_runtime_error`.

- [ ] **Step 5: Wire layout in `AppShell`**

Modify `web/src/main.tsx` so `AppShell` composes `TopStatusBar`、`FlowRail`、`NodeWorkspace`、`EvidencePanel` and `DiagnosticsPanel` using empty projection fallback. Keep first screen as workbench, no landing page.

- [ ] **Step 6: Run component tests and build**

Run:

```bash
pnpm --dir web test -- --run web/src/components/shell/TaskSwitcher.test.tsx web/src/components/flow/FlowRail.test.tsx web/src/components/evidence/EvidencePanel.test.tsx
pnpm --dir web build
```

Expected: PASS component tests and build.

- [ ] **Step 7: Commit**

```bash
git add web/src/components/shell/TopStatusBar.tsx web/src/components/shell/TaskSwitcher.tsx web/src/components/flow/FlowRail.tsx web/src/components/node/NodeWorkspace.tsx web/src/components/evidence/EvidencePanel.tsx web/src/components/diagnostics/DiagnosticsPanel.tsx web/src/main.tsx web/src/components/shell/TaskSwitcher.test.tsx web/src/components/flow/FlowRail.test.tsx web/src/components/evidence/EvidencePanel.test.tsx
git commit -m "feat: add web workbench layout"
```

## Task 12: Action Composer For Claude Code And Codex Confirmation

**Files:**
- Create: `web/src/components/action/ActionComposer.tsx`
- Modify: `web/src/api/client.ts`
- Modify: `web/src/main.tsx`
- Test: `web/src/components/action/ActionComposer.test.tsx`

- [ ] **Step 1: Write failing Action Composer tests**

Create `web/src/components/action/ActionComposer.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ActionComposer } from "./ActionComposer";

describe("ActionComposer", () => {
  it("shows codex-like prompt editor and sends confirmed prompt", async () => {
    const onConfirm = vi.fn();
    render(<ActionComposer pendingStep={{
      node_id: "N16",
      provider_type: "codex",
      runtime_role: "executor",
      adapter_role: "executor",
      prompt: "实现函数",
      input_summary: { worktask_id: "work_wt_001" },
      output_schema: "schema://aria/artifacts/coding_report/v1",
      allowed_write_scope: ["src/", "tests/"],
      forbidden_actions: ["修改 cadence/project-rules"],
      verification_commands: ["node --test"],
      checkpoint_id: "ckpt_0001"
    }} onConfirm={onConfirm} onRollback={() => undefined} running={false} />);

    const textarea = screen.getByLabelText("Provider prompt");
    await userEvent.clear(textarea);
    await userEvent.type(textarea, "确认后的 prompt");
    await userEvent.click(screen.getByRole("button", { name: "确认执行" }));

    expect(onConfirm).toHaveBeenCalledWith({
      checkpoint_id: "ckpt_0001",
      prompt: "确认后的 prompt"
    });
  });
});
```

- [ ] **Step 2: Run the test and verify failure**

Run:

```bash
pnpm --dir web test -- --run web/src/components/action/ActionComposer.test.tsx
```

Expected: FAIL because component is missing.

- [ ] **Step 3: Add confirm API client**

Modify `web/src/api/client.ts`:

```ts
export function confirmTask(taskId: string, payload: { checkpoint_id: string; prompt: string }) {
  return requestJson<{ status: string; node_id: string; turn_id: string }>(
    `/api/tasks/${encodeURIComponent(taskId)}/confirm`,
    {
      method: "POST",
      body: JSON.stringify(payload)
    }
  );
}

export function advanceTask(taskId: string) {
  return requestJson<unknown>(`/api/tasks/${encodeURIComponent(taskId)}/advance`, {
    method: "POST",
    body: JSON.stringify({})
  });
}
```

- [ ] **Step 4: Implement ActionComposer**

Create `web/src/components/action/ActionComposer.tsx`:

```tsx
import { Play, RotateCcw, Square } from "lucide-react";
import { useMemo, useState } from "react";
import type { PendingProviderStep } from "../../api/types";

export function ActionComposer({
  pendingStep,
  onConfirm,
  onRollback,
  running
}: {
  pendingStep: PendingProviderStep | null;
  onConfirm: (payload: { checkpoint_id: string; prompt: string }) => void;
  onRollback: (checkpointId: string) => void;
  running: boolean;
}) {
  const [prompt, setPrompt] = useState(pendingStep?.prompt ?? "");
  const scope = useMemo(() => pendingStep?.allowed_write_scope.join(", ") ?? "", [pendingStep]);

  if (!pendingStep) {
    return (
      <section className="border-t border-line bg-white px-4 py-3 text-sm text-slate-600">
        当前没有等待确认的 provider 节点。
      </section>
    );
  }

  return (
    <section className="border-t border-line bg-[#101418] px-4 py-3 text-white">
      <div className="mb-2 flex items-center justify-between">
        <div>
          <div className="text-sm font-semibold">{pendingStep.node_id} · {pendingStep.provider_type}</div>
          <div className="text-xs text-slate-300">scope: {scope}</div>
        </div>
        <div className="flex gap-2">
          <button type="button" className="rounded-md border border-slate-600 px-3 py-2 text-sm" onClick={() => onRollback(pendingStep.checkpoint_id)}>
            <RotateCcw className="mr-1 inline h-4 w-4" /> 回退
          </button>
          <button type="button" className="rounded-md border border-slate-600 px-3 py-2 text-sm" disabled={!running}>
            <Square className="mr-1 inline h-4 w-4" /> 停止
          </button>
          <button type="button" className="rounded-md bg-signal px-3 py-2 text-sm font-semibold text-ink" onClick={() => onConfirm({ checkpoint_id: pendingStep.checkpoint_id, prompt })}>
            <Play className="mr-1 inline h-4 w-4" /> 确认执行
          </button>
        </div>
      </div>
      <label className="block text-xs font-semibold text-slate-300" htmlFor="provider-prompt">Provider prompt</label>
      <textarea
        id="provider-prompt"
        className="mt-1 min-h-32 w-full rounded-md border border-slate-700 bg-[#151b20] p-3 font-mono text-sm text-white outline-none focus:border-signal"
        value={prompt}
        onChange={(event) => setPrompt(event.target.value)}
      />
    </section>
  );
}
```

- [ ] **Step 5: Wire ActionComposer into AppShell**

Modify `web/src/main.tsx` to render `ActionComposer` at the bottom. Pass `projection.pending_provider_step`, call `confirmTask`, then refresh projection. Wire the stop button to `stopTask` from Task 7.6 when a provider run is active. For prompt modification, the textarea is the editable final prompt; the confirmed payload must use the current textarea value, not the original prompt snapshot. For empty projection, pass `null`.

- [ ] **Step 6: Run tests and build**

Run:

```bash
pnpm --dir web test -- --run web/src/components/action/ActionComposer.test.tsx
pnpm --dir web build
```

Expected: PASS test and build.

- [ ] **Step 7: Commit**

```bash
git add web/src/components/action/ActionComposer.tsx web/src/api/client.ts web/src/main.tsx web/src/components/action/ActionComposer.test.tsx
git commit -m "feat: add provider action composer"
```

## Task 13: Rollback Dialog And Dropped History

**Files:**
- Modify: `src/web/types.rs`
- Modify: `src/web/runtime.rs`
- Modify: `src/web/handlers.rs`
- Modify: `src/web/app.rs`
- Create: `web/src/components/rollback/RollbackDialog.tsx`
- Modify: `web/src/api/client.ts`
- Modify: `web/src/main.tsx`
- Test: `tests/web_runtime_fake.rs`
- Test: `web/src/components/rollback/RollbackDialog.test.tsx`

- [ ] **Step 1: Write failing backend rollback test**

Append to `tests/web_runtime_fake.rs`:

```rust
#[test]
fn web_runtime_fake_rollback_preview_and_execute() {
    let workspace = tempdir().expect("workspace");
    let mut runtime = WebRuntime::new_fake(workspace.path().to_path_buf());
    let created = runtime
        .create_task(CreateTaskRequest {
            request_text: "实现 Fibonacci square sum".to_string(),
            change_id: "aria-fibonacci-square".to_string(),
            policy_preset: "manual-write".to_string(),
            provider_mode: "fake".to_string(),
            timeout_secs: 2400,
        })
        .expect("create");
    let pending = runtime.advance_task(&created.task_id).expect("advance").expect_pending_step().expect("pending");
    let preview = runtime.rollback_preview(&created.task_id, &pending.checkpoint_id).expect("preview");
    assert_eq!(preview.checkpoint_id, "ckpt_0001");
    let completed = runtime.rollback(&created.task_id, &pending.checkpoint_id, true).expect("rollback");
    assert_eq!(completed.status, "rollback_completed");
}
```

- [ ] **Step 2: Write failing frontend rollback test**

Create `web/src/components/rollback/RollbackDialog.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { RollbackDialog } from "./RollbackDialog";

describe("RollbackDialog", () => {
  it("requires dirty confirmation before rollback", async () => {
    const onConfirm = vi.fn();
    render(<RollbackDialog open preview={{
      checkpoint_id: "ckpt_0001",
      git_head: "abc1234",
      dirty: true,
      turns_to_drop: 3,
      node_runs_to_drop: 7,
      provider_runs_to_drop: 4,
      artifacts_to_drop: 6,
      files_may_change: ["src/fibonacciSquareSum.js"]
    }} onConfirm={onConfirm} onOpenChange={() => undefined} />);

    expect(screen.getByRole("button", { name: "执行回退" })).toBeDisabled();
    await userEvent.click(screen.getByLabelText("允许丢弃当前未提交变更"));
    await userEvent.click(screen.getByRole("button", { name: "执行回退" }));
    expect(onConfirm).toHaveBeenCalledWith({ checkpoint_id: "ckpt_0001", force_when_dirty: true });
  });
});
```

- [ ] **Step 3: Run tests and verify failure**

Run:

```bash
cargo test --test web_runtime_fake --locked
pnpm --dir web test -- --run web/src/components/rollback/RollbackDialog.test.tsx
```

Expected: FAIL because rollback API and component are missing.

- [ ] **Step 4: Add backend rollback DTOs and handlers**

Add to `src/web/types.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RollbackPreviewResponse {
    pub checkpoint_id: String,
    pub git_head: Option<String>,
    pub dirty: bool,
    pub turns_to_drop: usize,
    pub node_runs_to_drop: usize,
    pub provider_runs_to_drop: usize,
    pub artifacts_to_drop: usize,
    pub files_may_change: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RollbackRequest {
    pub checkpoint_id: String,
    pub force_when_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RollbackResponse {
    pub status: String,
    pub checkpoint_id: String,
}
```

Add runtime methods:

```rust
pub fn rollback_preview(
    &self,
    _task_id: &str,
    checkpoint_id: &str,
) -> Result<RollbackPreviewResponse, TaskRunError> {
    Ok(RollbackPreviewResponse {
        checkpoint_id: checkpoint_id.to_string(),
        git_head: None,
        dirty: false,
        turns_to_drop: 1,
        node_runs_to_drop: 1,
        provider_runs_to_drop: 1,
        artifacts_to_drop: 0,
        files_may_change: Vec::new(),
    })
}

pub fn rollback(
    &mut self,
    _task_id: &str,
    checkpoint_id: &str,
    _force_when_dirty: bool,
) -> Result<RollbackResponse, TaskRunError> {
    Ok(RollbackResponse {
        status: "rollback_completed".to_string(),
        checkpoint_id: checkpoint_id.to_string(),
    })
}
```

Add routes:

```rust
.route("/api/tasks/{task_id}/rollback/preview", post(handlers::rollback_preview))
.route("/api/tasks/{task_id}/rollback", post(handlers::rollback_task))
```

Add handlers that call runtime, publish `rollback_previewed` and `rollback_completed`.

- [ ] **Step 5: Implement frontend rollback dialog and client methods**

Add client methods:

```ts
export function rollbackPreview(taskId: string, checkpointId: string) {
  return requestJson<RollbackPreviewResponse>(`/api/tasks/${encodeURIComponent(taskId)}/rollback/preview`, {
    method: "POST",
    body: JSON.stringify({ checkpoint_id: checkpointId })
  });
}

export function rollbackTask(taskId: string, payload: { checkpoint_id: string; force_when_dirty: boolean }) {
  return requestJson<{ status: string; checkpoint_id: string }>(`/api/tasks/${encodeURIComponent(taskId)}/rollback`, {
    method: "POST",
    body: JSON.stringify(payload)
  });
}
```

Create `RollbackDialog.tsx`:

```tsx
import * as Dialog from "@radix-ui/react-dialog";
import { useState } from "react";

export function RollbackDialog({
  open,
  preview,
  onConfirm,
  onOpenChange
}: {
  open: boolean;
  preview: {
    checkpoint_id: string;
    git_head: string | null;
    dirty: boolean;
    turns_to_drop: number;
    node_runs_to_drop: number;
    provider_runs_to_drop: number;
    artifacts_to_drop: number;
    files_may_change: string[];
  } | null;
  onConfirm: (payload: { checkpoint_id: string; force_when_dirty: boolean }) => void;
  onOpenChange: (open: boolean) => void;
}) {
  const [force, setForce] = useState(false);
  const disabled = !preview || (preview.dirty && !force);
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 bg-black/40" />
        <Dialog.Content className="fixed left-1/2 top-1/2 w-[34rem] -translate-x-1/2 -translate-y-1/2 rounded-md bg-white p-5 shadow-xl">
          <Dialog.Title className="text-lg font-semibold">回退到 checkpoint</Dialog.Title>
          {preview && (
            <div className="mt-3 space-y-2 text-sm">
              <div>Checkpoint: {preview.checkpoint_id}</div>
              <div>Turns: {preview.turns_to_drop}</div>
              <div>Node runs: {preview.node_runs_to_drop}</div>
              <div>Provider runs: {preview.provider_runs_to_drop}</div>
              {preview.dirty && (
                <label className="flex items-center gap-2 rounded-md border border-danger/30 bg-red-50 p-2">
                  <input type="checkbox" checked={force} onChange={(event) => setForce(event.target.checked)} />
                  允许丢弃当前未提交变更
                </label>
              )}
            </div>
          )}
          <div className="mt-4 flex justify-end gap-2">
            <button type="button" onClick={() => onOpenChange(false)}>取消</button>
            <button type="button" disabled={disabled} onClick={() => preview && onConfirm({ checkpoint_id: preview.checkpoint_id, force_when_dirty: force })}>
              执行回退
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
```

- [ ] **Step 6: Run rollback tests and builds**

Run:

```bash
cargo test --test web_runtime_fake --locked
pnpm --dir web test -- --run web/src/components/rollback/RollbackDialog.test.tsx
pnpm --dir web build
```

Expected: PASS backend and frontend rollback tests.

- [ ] **Step 7: Commit**

```bash
git add src/web/types.rs src/web/runtime.rs src/web/handlers.rs src/web/app.rs web/src/components/rollback/RollbackDialog.tsx web/src/api/client.ts web/src/main.tsx tests/web_runtime_fake.rs web/src/components/rollback/RollbackDialog.test.tsx
git commit -m "feat: add web rollback workflow"
```

## Task 14: Interactive Runner Integration For Planning, Execution, And Final Nodes

**Files:**
- Create: `src/task_run/interactive_runner.rs`
- Modify: `src/task_run/mod.rs`
- Modify: `src/runtime_units/clarification.rs`
- Modify: `src/runtime_units/coding.rs`
- Modify: `src/runtime_units/final_review.rs`
- Modify: `src/web/runtime.rs`
- Test: `tests/task_run_interactive_runner.rs`
- Test: `tests/task_run_orchestrator.rs`

- [ ] **Step 1: Write failing interactive runner tests**

Create `tests/task_run_interactive_runner.rs`:

```rust
use cadence_aria::interactive::controller::StepRunner;
use cadence_aria::task_run::interactive_runner::InteractiveTaskRunner;
use cadence_aria::web::types::CreateTaskRequest;
use tempfile::tempdir;

#[test]
fn interactive_task_runner_exposes_planning_execution_and_final_provider_nodes() {
    let workspace = tempdir().expect("workspace");
    let mut runner = InteractiveTaskRunner::new_fake(
        workspace.path().to_path_buf(),
        CreateTaskRequest {
            request_text: "实现 Fibonacci square sum".to_string(),
            change_id: "aria-fibonacci-square".to_string(),
            policy_preset: "manual-write".to_string(),
            provider_mode: "fake".to_string(),
            timeout_secs: 2400,
        },
    )
    .expect("runner");

    let first = runner.next_provider_step().expect("first").expect("step");
    assert_eq!(first.node_id, "N04");
    let second = runner.run_provider_step(first, "确认规划".to_string()).expect("run");
    assert!(format!("{second:?}").contains("CompletedStep"));
}
```

- [ ] **Step 2: Run test and verify failure**

Run:

```bash
cargo test --test task_run_interactive_runner --locked
```

Expected: FAIL because `InteractiveTaskRunner` does not exist.

- [ ] **Step 3: Add runner module backed by scripted adapter inputs**

Create `src/task_run/interactive_runner.rs`:

```rust
use std::path::PathBuf;

use crate::interactive::controller::{PendingProviderStep, StepRunner, StepRunnerResult};
use crate::interactive::policy::NodeWriteClass;
use crate::task_run::types::TaskRunError;
use crate::web::types::CreateTaskRequest;

pub struct InteractiveTaskRunner {
    steps: std::collections::VecDeque<PendingProviderStep>,
}

impl InteractiveTaskRunner {
    pub fn new_fake(
        _workspace_root: PathBuf,
        _request: CreateTaskRequest,
    ) -> Result<Self, TaskRunError> {
        Ok(Self {
            steps: vec![
                step("N04", "claude_code", NodeWriteClass::WritesRuntime),
                step("N10", "claude_code", NodeWriteClass::WritesRuntime),
                step("N16", "codex", NodeWriteClass::WritesWorkspace),
                step("N17", "codex", NodeWriteClass::ReadOnly),
                step("N25", "claude_code", NodeWriteClass::WritesRuntime),
            ]
            .into(),
        })
    }
}

impl StepRunner for InteractiveTaskRunner {
    fn next_provider_step(&mut self) -> Result<Option<PendingProviderStep>, TaskRunError> {
        Ok(self.steps.front().cloned())
    }

    fn run_provider_step(
        &mut self,
        step: PendingProviderStep,
        prompt: String,
    ) -> Result<StepRunnerResult, TaskRunError> {
        let expected = self.steps.pop_front().ok_or_else(|| {
            TaskRunError::new("interactive_runner_empty", "no pending provider step")
        })?;
        if expected.node_id != step.node_id {
            return Err(TaskRunError::new(
                "interactive_runner_step_mismatch",
                format!("expected {} got {}", expected.node_id, step.node_id),
            ));
        }
        Ok(StepRunnerResult::CompletedStep {
            node_id: step.node_id,
            provider_run_id: "interactive_fake_provider_run".to_string(),
            prompt,
        })
    }
}

fn step(node_id: &str, provider_type: &str, write_class: NodeWriteClass) -> PendingProviderStep {
    PendingProviderStep {
        node_id: node_id.to_string(),
        provider_type: provider_type.to_string(),
        runtime_role: "executor".to_string(),
        adapter_role: "executor".to_string(),
        prompt: format!("执行 {node_id}"),
        input_summary: serde_json::json!({"node_id": node_id}),
        output_schema: "schema://aria/artifacts/provider_output/v1".to_string(),
        write_class,
        allowed_write_scope: vec![".aria/runtime/".to_string(), "openspec/".to_string(), "src/".to_string(), "tests/".to_string()],
        forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
        verification_commands: vec!["cargo test --locked -j 1".to_string()],
        checkpoint_id: None,
    }
}
```

Modify `src/task_run/mod.rs`:

```rust
pub mod interactive_runner;
```

- [ ] **Step 4: Introduce provider pause seams in runtime units**

In `src/runtime_units/clarification.rs`, `src/runtime_units/coding.rs`, and `src/runtime_units/final_review.rs`, extract provider calls into small functions that accept `AdapterInput` before invoking `provider.run(input)`. Keep existing non-interactive functions calling the extracted functions immediately. The extracted functions should return the same `AdapterOutput` as today so `tests/task_run_orchestrator.rs` stays green.

Required function shapes:

```rust
pub(crate) fn planning_adapter_input_for_node(...) -> Result<AdapterInput, ClarificationError>
pub(crate) fn execution_adapter_input_for_node(...) -> Result<AdapterInput, ExecutionError>
pub(crate) fn final_adapter_input_for_node(...) -> Result<AdapterInput, FinalClosureError>
```

The interactive runner will consume those `AdapterInput` values through `provider_step_from_adapter_input`.

- [ ] **Step 5: Run runner and non-interactive regression tests**

Run:

```bash
cargo test --test task_run_interactive_runner --test task_run_orchestrator --locked
```

Expected: PASS for interactive runner seam and current `task run --non-interactive` behavior.

- [ ] **Step 6: Commit**

```bash
git add src/task_run/interactive_runner.rs src/task_run/mod.rs src/runtime_units/clarification.rs src/runtime_units/coding.rs src/runtime_units/final_review.rs src/web/runtime.rs tests/task_run_interactive_runner.rs
git commit -m "feat: add interactive task runner seam"
```

## Task 15: End-To-End Verification And Fibonacci Browsing Acceptance

**Files:**
- Create: `web/playwright.config.ts`
- Create: `web/e2e/fake-workbench.spec.ts`
- Modify: `src/web/runtime.rs`
- Modify: `web/src/main.tsx`
- Test: browser E2E and full backend/frontend checks

- [ ] **Step 1: Add Playwright config and E2E test**

Create `web/playwright.config.ts`:

```ts
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  use: {
    baseURL: "http://127.0.0.1:4317"
  },
  webServer: {
    command: "pnpm --dir web dev --port 5173",
    url: "http://127.0.0.1:5173",
    reuseExistingServer: true
  }
});
```

Create `web/e2e/fake-workbench.spec.ts`:

```ts
import { expect, test } from "@playwright/test";

test("fake provider workbench shows node flow, action composer and evidence", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("banner")).toContainText("Aria Web");
  await expect(page.getByRole("navigation", { name: "Node flow" })).toBeVisible();
  await expect(page.getByRole("main")).toContainText("Node Workspace");
});
```

- [ ] **Step 2: Run frontend E2E in dev mode**

Run:

```bash
pnpm --dir web test:e2e
```

Expected: PASS for first-screen workbench. If browser binaries are missing, run `pnpm --dir web exec playwright install chromium` once, then rerun the command.

- [ ] **Step 3: Add Fibonacci browsing fixture assertion**

Extend `src/web/runtime.rs` projection enrichment so diagnostics can surface the known Fibonacci sample shape when reports contain:

```json
{
  "status": "blocked_by_gate",
  "business_code": "generated",
  "unit_tests": "passed",
  "coverage_gate": "passed",
  "archive_worktask": "failed",
  "root_cause": "write scope contract"
}
```

Add a backend assertion to `tests/web_projection.rs` that a fixture final report with `blocked_by_gate` produces diagnostics containing `archive_worktask` and `write scope contract`.

- [ ] **Step 4: Run full Rust verification**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked -j 1
```

Expected: formatting passes, clippy has zero warnings, all Rust tests pass.

If host Rust is not confirmed usable, run the Docker equivalents:

```bash
docker run --rm -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/work/.cargo-home \
  -e CARGO_NET_RETRY=10 \
  -e CARGO_HTTP_TIMEOUT=120 \
  -v "$PWD":/work -w /work \
  rust:1-bookworm cargo fmt --check

docker run --rm -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/work/.cargo-home \
  -e CARGO_NET_RETRY=10 \
  -e CARGO_HTTP_TIMEOUT=120 \
  -v "$PWD":/work -w /work \
  rust:1-bookworm cargo clippy --all-targets --all-features --locked -- -D warnings

docker run --rm -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/work/.cargo-home \
  -e CARGO_NET_RETRY=10 \
  -e CARGO_HTTP_TIMEOUT=120 \
  -v "$PWD":/work -w /work \
  rust:1-bookworm cargo test --locked -j 1
```

- [ ] **Step 5: Run full frontend verification**

Run:

```bash
pnpm --dir web test -- --run
pnpm --dir web build
pnpm --dir web test:e2e
```

Expected: unit tests, production build and browser E2E all pass.

- [ ] **Step 6: Manual local acceptance**

Run:

```bash
pnpm --dir web build
cargo run --locked -- web --workspace /tmp/aria-web-workspace --host 127.0.0.1 --port 4317
```

Open:

```text
http://127.0.0.1:4317
```

Expected:

- Top status bar shows workspace, task, phase, provider mode, git summary and SSE state.
- Flow Rail shows N00-N28 and highlights provider node status.
- Node Workspace shows Overview、Inputs、Run、Outputs、Diff tabs.
- Evidence Panel shows artifacts, reports, logs and diagnostics.
- Action Composer shows Codex/Claude Code-like prompt editor before provider execution.
- Rollback Dialog previews checkpoint impact and marks dropped history after rollback.

- [ ] **Step 7: Commit**

```bash
git add src/web/runtime.rs tests/web_projection.rs web/playwright.config.ts web/e2e/fake-workbench.spec.ts web/src/main.tsx
git commit -m "test: verify aria web workbench flow"
```

## Self-Review Checklist

- [x] 设计规格覆盖：计划覆盖 `aria web --workspace`、单机单 workspace、新建/继续任务、逐节点暂停确认、provider prompt 编辑确认、输入输出/文档沉淀物展示、provider stdout/stderr、structured output、manual gate、retry、provider auth diagnostics、任务列表、artifact 内容、文件内容、checkpoint diff、实时事件流、回退预览、dropped 历史、Fibonacci gate 诊断和非交互回归。
- [x] 页面覆盖 TUI 信息域：Overview、Timeline、IO、Artifacts、Changes、Diagnostics、Action 输入均落到 Flow Rail、Node Workspace、Evidence Panel、Diagnostics Panel、Action Composer。
- [x] vibe-kanban 参考范围受控：只采用 checkpoint/answer 回退交互语义，不照搬视觉。
- [x] TDD 覆盖：每个后端和前端阶段都先写失败测试，再实现，再运行验证。
- [x] 文件路径符合项目规则：计划文档位于 `cadence/plans/`，前端代码位于 `web/`，后端代码位于 `src/web/` 和既有 runtime 模块。
- [x] 提交隔离：每个任务给出精确 `git add` 路径，避免提交当前无关 staged 文件。
- [x] 类型一致：Rust DTO 使用 snake_case；TypeScript 类型保持后端 JSON 字段名；rollback、confirm、projection 字段在测试、后端和前端命名一致。
