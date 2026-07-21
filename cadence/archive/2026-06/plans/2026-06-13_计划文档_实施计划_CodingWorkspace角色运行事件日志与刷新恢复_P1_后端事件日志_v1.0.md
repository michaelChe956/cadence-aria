# CodingWorkspace 角色运行事件日志 P1 后端事件日志 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 Tester、Analyst、CodeReviewer、InternalReviewer 的 `CodingRoleRun` 持久化 provider 过程事件，同时保持现有 WebSocket 实时输出不变。

**Architecture:** 新增 `CodingRoleRunEvent` 模型和 `CodingAttemptStore` JSONL append/list API，事件文件放在 `coding-attempts/<attempt_id>/role-run-events/<role_run_id>.jsonl`。`CodingWorkspaceEngine::run_provider_stream_to_completion` 接收可选 `role_run`，对目标角色做 provider event 双写；日志写入失败只记录 tracing warning，不改变 provider 流程。

**Tech Stack:** Rust 1.95.0、Serde JSON、Tokio、JSON Lines、Cargo integration tests。

---

## File Structure

- Modify: `src/product/coding_models.rs`
  - 新增 `CodingRoleRunEventType` 与 `CodingRoleRunEvent`。
- Modify: `src/product/coding_attempt_store.rs`
  - 新增 role run event JSONL 路径、append/list、payload 截断、artifact 写入 helper。
- Modify: `src/product/coding_workspace_engine.rs`
  - 扩展 `CodingProviderStreamRun`，把目标 role run 传给统一 provider event loop。
  - 在 provider prompt、start、stream event、terminal event 处调用 store append。
- Modify: `tests/it_product/product_coding_attempt_store.rs`
  - 覆盖 append/list、sequence、大字段 artifact 截断。
- Modify: `tests/it_product/product_coding_workspace_engine.rs`
  - 覆盖 engine 事件落盘与实时 WebSocket forwarding 并存。

## Task 1: Store Model And JSONL API

**Files:**
- Modify: `src/product/coding_models.rs`
- Modify: `src/product/coding_attempt_store.rs`
- Test: `tests/it_product/product_coding_attempt_store.rs`

- [ ] **Step 1: Write failing store tests**

In `tests/it_product/product_coding_attempt_store.rs`, extend the `coding_models` import:

```rust
use cadence_aria::product::coding_models::{
    AnalystDecisionRecord, AnalystDecisionVerdict, AnalystReworkInstructions,
    CodeReviewReport, CodingAgentRole, CodingAttemptStatus, CodingContextNote,
    CodingExecutionStage, CodingProviderRole, CodingReworkInstruction,
    CodingRolePermissionModes, CodingRoleProviderConfigSnapshot, CodingRoleRunEventType,
    CodingRoleRunStatus, CodingRoleRunTrigger, CodingStageGateStatus, CodingTimelineNode,
    CodingTimelineNodeStatus, FindingSeverity, InternalPrReview, PushStatus, RemoteKind,
    ReviewFinding, ReviewRequest, ReviewRequestKind, ReviewVerdict, TestCommand,
    TestCommandStatus, TestingOverallStatus, TestingReport,
};
```

Append these tests after `updates_coding_role_run_refs_without_duplicates`:

```rust
#[test]
fn appends_and_lists_coding_role_run_events_in_sequence() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("role run");

    let first = store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ProviderPrompt,
            serde_json::json!({
                "mode": "plan_tests",
                "prompt": "plan tests as JSON"
            }),
        )
        .expect("append first event");
    let second = store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::TextDelta,
            serde_json::json!({
                "content": "No tasks found"
            }),
        )
        .expect("append second event");

    assert_eq!(first.sequence, 1);
    assert_eq!(second.sequence, 2);
    assert_eq!(first.role_run_id, run.id);
    assert_eq!(second.node_id.as_deref(), Some("coding_node_0003"));

    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &run.id)
        .expect("events");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, CodingRoleRunEventType::ProviderPrompt);
    assert_eq!(events[1].event_type, CodingRoleRunEventType::TextDelta);
    assert_eq!(events[1].payload["content"], "No tasks found");
}

#[test]
fn role_run_event_large_string_payload_is_moved_to_artifact() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0007".to_string()),
        )
        .expect("role run");
    let long_prompt = "review this diff\n".repeat(2_000);

    let event = store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ProviderPrompt,
            serde_json::json!({
                "mode": "full_conversation",
                "prompt": long_prompt
            }),
        )
        .expect("append event");

    assert!(event.truncated);
    assert_eq!(
        event.artifact_ref.as_deref(),
        Some("artifacts/role-run-events/coding_role_run_0001/0001_prompt.txt")
    );
    assert_eq!(
        event.payload["prompt"]["artifact_ref"],
        "artifacts/role-run-events/coding_role_run_0001/0001_prompt.txt"
    );
    assert_eq!(event.payload["prompt"]["truncated"], true);

    let artifact = store
        .read_attempt_artifact_text(&attempt.id, event.artifact_ref.as_deref().expect("ref"))
        .expect("artifact text");
    assert_eq!(artifact, long_prompt);
}
```

- [ ] **Step 2: Run store tests and confirm failure**

Run:

```bash
cargo test --locked --test it_product appends_and_lists_coding_role_run_events_in_sequence
cargo test --locked --test it_product role_run_event_large_string_payload_is_moved_to_artifact
```

Expected:

- Both commands fail with unresolved `CodingRoleRunEventType`.
- The compiler also reports missing `append_role_run_event` and `list_role_run_events`.

- [ ] **Step 3: Add event model**

In `src/product/coding_models.rs`, add this block immediately after `CodingRoleRun`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingRoleRunEventType {
    ProviderPrompt,
    ProviderStart,
    TextDelta,
    ExecutionEvent,
    ToolCall,
    ToolResult,
    StatusChanged,
    PermissionRequest,
    ChoiceRequest,
    MessageComplete,
    ProviderFailed,
    Timeout,
    Aborted,
    PersistenceWarning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodingRoleRunEvent {
    pub attempt_id: String,
    pub role_run_id: String,
    pub node_id: Option<String>,
    pub stage: CodingExecutionStage,
    pub role: CodingProviderRole,
    pub sequence: u64,
    pub event_type: CodingRoleRunEventType,
    pub created_at: String,
    pub payload: serde_json::Value,
    pub truncated: bool,
    pub artifact_ref: Option<String>,
}
```

- [ ] **Step 4: Add store imports and constants**

In `src/product/coding_attempt_store.rs`, change the imports:

```rust
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
```

In the `coding_models` import list, add:

```rust
CodingRoleRunEvent, CodingRoleRunEventType,
```

Add this constant near the top-level structs:

```rust
const ROLE_RUN_EVENT_INLINE_STRING_LIMIT: usize = 16_384;
```

- [ ] **Step 5: Implement store append/list API**

In the public `impl CodingAttemptStore` block, place these methods after `update_role_run_refs`:

```rust
    pub fn append_role_run_event(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: &CodingRoleRun,
        event_type: CodingRoleRunEventType,
        payload: serde_json::Value,
    ) -> Result<CodingRoleRunEvent, ProductStoreError> {
        validate_relative_id(&attempt.project_id)?;
        validate_relative_id(&attempt.issue_id)?;
        validate_relative_id(&attempt.id)?;
        validate_relative_id(&role_run.id)?;
        if attempt.id != role_run.attempt_id {
            return Err(ProductStoreError::NotFound {
                kind: "coding_role_run_attempt",
                id: role_run.id.clone(),
            });
        }

        let path =
            self.role_run_event_log_path(&attempt.project_id, &attempt.issue_id, &attempt.id, &role_run.id);
        let sequence = next_jsonl_sequence(&path)?;
        let (payload, truncated, artifact_ref) = self.normalize_role_run_event_payload(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            sequence,
            payload,
        )?;
        let event = CodingRoleRunEvent {
            attempt_id: attempt.id.clone(),
            role_run_id: role_run.id.clone(),
            node_id: role_run.node_id.clone(),
            stage: role_run.stage.clone(),
            role: role_run.role.clone(),
            sequence,
            event_type,
            created_at: Utc::now().to_rfc3339(),
            payload,
            truncated,
            artifact_ref,
        };
        append_jsonl(&path, &event)?;
        Ok(event)
    }

    pub fn list_role_run_events(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
    ) -> Result<Vec<CodingRoleRunEvent>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        validate_relative_id(role_run_id)?;
        let path = self.role_run_event_log_path(project_id, issue_id, attempt_id, role_run_id);
        let mut events: Vec<CodingRoleRunEvent> = read_jsonl_records(&path)?;
        events.sort_by_key(|event| event.sequence);
        Ok(events)
    }
```

- [ ] **Step 6: Implement store path and payload helpers**

In the private helper section near `role_runs_root`, add:

```rust
    fn role_run_events_root(&self, project_id: &str, issue_id: &str, attempt_id: &str) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("role-run-events")
    }

    fn role_run_event_log_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
    ) -> PathBuf {
        self.role_run_events_root(project_id, issue_id, attempt_id)
            .join(format!("{role_run_id}.jsonl"))
    }

    fn role_run_event_artifact_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("artifacts")
            .join("role-run-events")
            .join(role_run_id)
    }

    fn normalize_role_run_event_payload(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
        sequence: u64,
        payload: serde_json::Value,
    ) -> Result<(serde_json::Value, bool, Option<String>), ProductStoreError> {
        let mut payload = payload;
        let Some(object) = payload.as_object_mut() else {
            return Ok((payload, false, None));
        };

        for field in [
            "prompt",
            "content",
            "output",
            "stdout",
            "stderr",
            "detail",
            "message",
        ] {
            let Some(value) = object.get_mut(field) else {
                continue;
            };
            let Some(text) = value.as_str() else {
                continue;
            };
            if text.len() <= ROLE_RUN_EVENT_INLINE_STRING_LIMIT {
                continue;
            }

            let artifact_ref = self.save_role_run_event_artifact(
                project_id,
                issue_id,
                attempt_id,
                role_run_id,
                sequence,
                field,
                text,
            )?;
            *value = serde_json::json!({
                "preview": truncate_utf8(text, ROLE_RUN_EVENT_INLINE_STRING_LIMIT),
                "artifact_ref": artifact_ref,
                "truncated": true
            });
            return Ok((payload, true, Some(artifact_ref)));
        }

        Ok((payload, false, None))
    }

    fn save_role_run_event_artifact(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
        sequence: u64,
        field: &str,
        content: &str,
    ) -> Result<String, ProductStoreError> {
        validate_relative_id(role_run_id)?;
        validate_relative_id(field)?;
        let root = self.role_run_event_artifact_root(project_id, issue_id, attempt_id, role_run_id);
        fs::create_dir_all(&root)
            .map_err(|error| ProductStoreError::Io(format!("create {}: {error}", root.display())))?;
        let file_name = format!("{sequence:04}_{field}.txt");
        let path = root.join(&file_name);
        fs::write(&path, content)
            .map_err(|error| ProductStoreError::Io(format!("write {}: {error}", path.display())))?;
        let artifact_ref = format!("artifacts/role-run-events/{role_run_id}/{file_name}");
        validate_relative_artifact_ref(&artifact_ref)?;
        Ok(artifact_ref)
    }
```

Add these free functions near `list_json_records`:

```rust
fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<(), ProductStoreError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| ProductStoreError::Io(format!("create {}: {error}", parent.display())))?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| ProductStoreError::Io(format!("open {}: {error}", path.display())))?;
    serde_json::to_writer(&mut file, value)
        .map_err(|error| ProductStoreError::Json(error.to_string()))?;
    file.write_all(b"\n")
        .map_err(|error| ProductStoreError::Io(format!("write {}: {error}", path.display())))?;
    file.flush()
        .map_err(|error| ProductStoreError::Io(format!("flush {}: {error}", path.display())))?;
    Ok(())
}

fn read_jsonl_records<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?;
    let mut records = Vec::new();
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        records.push(
            serde_json::from_str(line)
                .map_err(|error| ProductStoreError::Json(error.to_string()))?,
        );
    }
    Ok(records)
}

fn next_jsonl_sequence(path: &Path) -> Result<u64, ProductStoreError> {
    Ok(read_jsonl_records::<serde_json::Value>(path)?.len() as u64 + 1)
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}
```

- [ ] **Step 7: Run store tests and confirm pass**

Run:

```bash
cargo test --locked --test it_product appends_and_lists_coding_role_run_events_in_sequence
cargo test --locked --test it_product role_run_event_large_string_payload_is_moved_to_artifact
```

Expected:

- Both tests pass.

- [ ] **Step 8: Commit store API**

Run:

```bash
git add src/product/coding_models.rs src/product/coding_attempt_store.rs tests/it_product/product_coding_attempt_store.rs
git commit -m "feat: persist coding role run events"
```

## Task 2: Engine Double-Write Provider Events

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **Step 1: Write failing engine event persistence test**

In `tests/it_product/product_coding_workspace_engine.rs`, add `CodingRoleRunEventType` to the `coding_models` import.

Append this test near `execute_code_review_forwards_provider_execution_events`:

```rust
#[tokio::test]
async fn execute_code_review_persists_role_run_events_while_forwarding_realtime_events() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = EventThenCompletedProvider {
        output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#.to_string(),
    };
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    assert_provider_command_event(&drain_events(&mut rx));
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &runs[0].id)
        .expect("role run events");
    let event_types = events
        .iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert_eq!(
        event_types,
        vec![
            CodingRoleRunEventType::ProviderPrompt,
            CodingRoleRunEventType::ProviderStart,
            CodingRoleRunEventType::ExecutionEvent,
            CodingRoleRunEventType::MessageComplete,
        ]
    );
    assert_eq!(events[2].payload["title"], "Provider command");
    assert_eq!(events[2].payload["output"], "changed files");
}
```

- [ ] **Step 2: Extend timeout test expectations**

In `tester_plan_start_timeout_blocks_with_retry_test_plan_gate`, after role run assertions, add:

```rust
    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &runs[0].id)
        .expect("role run events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == CodingRoleRunEventType::ProviderPrompt)
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == CodingRoleRunEventType::Timeout)
    );
```

- [ ] **Step 3: Run engine tests and confirm failure**

Run:

```bash
cargo test --locked --test it_product execute_code_review_persists_role_run_events_while_forwarding_realtime_events
cargo test --locked --test it_product tester_plan_start_timeout_blocks_with_retry_test_plan_gate
```

Expected:

- The new test fails because no event log is written.
- The timeout test fails because no `Timeout` event exists.

- [ ] **Step 4: Import event model and JSON macro**

In `src/product/coding_workspace_engine.rs`, extend imports:

```rust
use serde_json::json;
```

Add `CodingRoleRunEventType` to the `coding_models` import list.

- [ ] **Step 5: Add role_run to provider stream input**

Change `CodingProviderStreamRun`:

```rust
struct CodingProviderStreamRun<'a> {
    attempt: &'a CodingExecutionAttempt,
    node_id: &'a str,
    role_run: Option<&'a CodingRoleRun>,
    provider: &'a dyn StreamingProviderAdapter,
    legacy_input: &'a AdapterInput,
    input: StreamingProviderInput,
    provider_name: &'a ProviderName,
    provider_role: CodingProviderRole,
    command_rx: &'a mut mpsc::Receiver<CodingRunnerCommand>,
    allow_legacy_stream_fallback: bool,
    timeout: Option<Duration>,
    timeout_reason_code: Option<&'static str>,
}
```

For every `run_provider_stream_to_completion` call that builds a `CodingProviderStreamRun` struct literal:

- Tester plan/repair/execution calls pass `role_run: Some(&role_run)`.
- Code Reviewer calls pass `role_run: Some(&role_run)`.
- Analyst calls pass `role_run: Some(&role_run)`.
- Internal Reviewer calls pass `role_run: Some(&role_run)`.
- Coder calls pass `role_run: None`.

- [ ] **Step 6: Add non-blocking record helper**

Inside `impl CodingWorkspaceEngine`, before `run_provider_stream_to_completion`, add:

```rust
    fn record_role_run_event(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: Option<&CodingRoleRun>,
        event_type: CodingRoleRunEventType,
        payload: serde_json::Value,
    ) {
        let Some(role_run) = role_run else {
            return;
        };
        if let Err(error) = self
            .store
            .append_role_run_event(attempt, role_run, event_type, payload)
        {
            tracing::warn!(
                role_run_id = role_run.id.as_str(),
                event_type = ?event_type,
                error = %error,
                "failed to persist coding role run event"
            );
        }
    }
```

- [ ] **Step 7: Record prompt/start/timeout/failure events**

In `run_provider_stream_to_completion`, destructure `role_run`:

```rust
            role_run,
```

Immediately after destructuring `run`, before `provider.start`, add:

```rust
        self.record_role_run_event(
            attempt,
            role_run,
            CodingRoleRunEventType::ProviderPrompt,
            json!({
                "provider": provider_name.to_string(),
                "role": format!("{provider_role:?}"),
                "output_schema": legacy_input.output_schema,
                "prompt": legacy_input.prompt
            }),
        );
```

In the provider start timeout branch, before returning:

```rust
                    self.record_role_run_event(
                        attempt,
                        role_run,
                        CodingRoleRunEventType::Timeout,
                        json!({
                            "phase": "provider_start",
                            "reason_code": timeout_reason_code.unwrap_or("provider_stream_timeout")
                        }),
                    );
```

After `start_result` is converted into `session`, add:

```rust
        self.record_role_run_event(
            attempt,
            role_run,
            CodingRoleRunEventType::ProviderStart,
            json!({
                "provider": provider_name.to_string(),
                "role": format!("{provider_role:?}")
            }),
        );
```

In `Err(error) if !allow_legacy_stream_fallback`, before return:

```rust
                self.record_role_run_event(
                    attempt,
                    role_run,
                    CodingRoleRunEventType::ProviderFailed,
                    json!({
                        "phase": "provider_start",
                        "message": error.details
                    }),
                );
```

In the stream timeout branch, before returning:

```rust
                    self.record_role_run_event(
                        attempt,
                        role_run,
                        CodingRoleRunEventType::Timeout,
                        json!({
                            "phase": "provider_stream",
                            "reason_code": timeout_reason_code.unwrap_or("provider_stream_timeout")
                        }),
                    );
```

In the abort branch, before returning:

```rust
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::Aborted,
                                json!({
                                    "reason": "abort_attempt"
                                }),
                            );
```

- [ ] **Step 8: Record stream event variants**

In each `ProviderEvent` match branch, add the corresponding record call before sending WebSocket:

```rust
                        ProviderEvent::TextDelta { content } => {
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::TextDelta,
                                json!({ "content": content }),
                            );
                            full_output.push_str(&content);
```

```rust
                        ProviderEvent::Execution(event) => {
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::ExecutionEvent,
                                json!({
                                    "event_id": event.event_id,
                                    "kind": format!("{:?}", event.kind),
                                    "status": format!("{:?}", event.status),
                                    "title": event.title,
                                    "detail": event.detail,
                                    "command": event.command,
                                    "cwd": event.cwd,
                                    "output": event.output,
                                    "exit_code": event.exit_code
                                }),
                            );
```

Because the code above consumes `event`, make the WebSocket call use the same local binding after the record block. If the compiler reports moved fields, clone the fields before the `json!` call:

```rust
                            let event_payload = json!({
                                "event_id": event.event_id.clone(),
                                "kind": format!("{:?}", event.kind),
                                "status": format!("{:?}", event.status),
                                "title": event.title.clone(),
                                "detail": event.detail.clone(),
                                "command": event.command.clone(),
                                "cwd": event.cwd.clone(),
                                "output": event.output.clone(),
                                "exit_code": event.exit_code
                            });
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::ExecutionEvent,
                                event_payload,
                            );
```

Use the same clone-first pattern for `ToolCall`, `ToolResult`, permission/choice/status:

```rust
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::ToolCall,
                                json!({
                                    "id": call.id,
                                    "tool_name": call.tool_name,
                                    "input": call.input
                                }),
                            );
```

```rust
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::ToolResult,
                                json!({
                                    "tool_use_id": result.tool_use_id,
                                    "output": result.output,
                                    "is_error": result.is_error
                                }),
                            );
```

```rust
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::PermissionRequest,
                                json!({
                                    "id": request.id,
                                    "tool_name": request.tool_name,
                                    "description": request.description,
                                    "risk_level": format!("{:?}", request.risk_level)
                                }),
                            );
```

```rust
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::ChoiceRequest,
                                json!({
                                    "id": request.id,
                                    "prompt": request.prompt,
                                    "allow_multiple": request.allow_multiple,
                                    "allow_free_text": request.allow_free_text
                                }),
                            );
```

```rust
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::StatusChanged,
                                json!({
                                    "status": format!("{status:?}")
                                }),
                            );
```

In `ProviderEvent::Completed`, before `record_attempt_provider_session`:

```rust
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::MessageComplete,
                                json!({
                                    "provider_session_id": provider_session_id,
                                    "output_bytes": completed_output.len()
                                }),
                            );
```

In `Failed`, `ProtocolError`, and `PermissionTimeout`, record `ProviderFailed`:

```rust
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::ProviderFailed,
                                json!({ "message": message }),
                            );
```

For `ProtocolError`, include `code` and `context`:

```rust
                            self.record_role_run_event(
                                attempt,
                                role_run,
                                CodingRoleRunEventType::ProviderFailed,
                                json!({
                                    "code": code,
                                    "message": message,
                                    "context": context
                                }),
                            );
```

- [ ] **Step 9: Run targeted engine tests**

Run:

```bash
cargo test --locked --test it_product execute_code_review_persists_role_run_events_while_forwarding_realtime_events
cargo test --locked --test it_product tester_plan_start_timeout_blocks_with_retry_test_plan_gate
cargo test --locked --test it_product execute_code_review_forwards_provider_execution_events
cargo test --locked --test it_product execute_rework_forwards_provider_execution_events
cargo test --locked --test it_product execute_internal_pr_review_forwards_provider_execution_events
```

Expected:

- All commands pass.

- [ ] **Step 10: Run format/check**

Run:

```bash
cargo fmt --check
cargo check --locked
```

Expected:

- Both commands pass.

- [ ] **Step 11: Commit engine double-write**

Run:

```bash
git add src/product/coding_workspace_engine.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: record coding role provider events"
```

## Task 3: P1 Final Verification

**Files:**
- Verify only.

- [ ] **Step 1: Run all touched integration tests**

Run:

```bash
cargo test --locked --test it_product product_coding_attempt_store
cargo test --locked --test it_product execute_code_review_persists_role_run_events_while_forwarding_realtime_events
cargo test --locked --test it_product tester_plan_start_timeout_blocks_with_retry_test_plan_gate
cargo test --locked --test it_product execute_testing_binds_plan_report_and_chat_entries_to_tester_role_run
cargo test --locked --test it_product execute_rework_binds_analyst_decision_chat_and_gate_to_role_run
cargo test --locked --test it_product execute_code_review_binds_report_chat_and_status_to_role_run
cargo test --locked --test it_product execute_internal_pr_review_binds_review_chat_and_status_to_role_run
```

Expected:

- All commands pass.

- [ ] **Step 2: Run standard local checks for this slice**

Run:

```bash
cargo fmt --check
cargo check --locked
```

Expected:

- Both commands pass.

- [ ] **Step 3: Inspect final diff**

Run:

```bash
git diff --check
git status --short
```

Expected:

- `git diff --check` produces no output.
- `git status --short` shows only changes from this P1 plan before the final commit, or a clean worktree after commits.
