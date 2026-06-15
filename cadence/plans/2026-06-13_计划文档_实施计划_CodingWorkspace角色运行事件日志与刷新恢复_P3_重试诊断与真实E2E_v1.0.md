# CodingWorkspace 角色运行事件日志 P3 重试诊断与真实 E2E Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** retry 时使用上一轮 role run 的诊断摘要和 refs，而不是注入完整日志；并用真实 E2E 验证过程事件、刷新恢复和重试行为。

**Architecture:** 在 store 层提供 `role_run_retry_diagnostic_summary`，摘要由 reason code、terminal event、最近关键事件、raw refs、artifact refs、event artifact refs 组成。Engine 在 Tester、Analyst、CodeReviewer、InternalReviewer 的 retry run 中读取 `supersedes_run_id` 指向的旧 run 摘要，追加到 prompt 的专用诊断段落；最终 JSON-only 契约保持不变。

**Tech Stack:** Rust 1.95.0、Serde JSON、Axum WebSocket、React、Playwright、Vitest。

---

## P1/P2 Dependency Check

Before starting P3, confirm P1 and P2 exist:

```bash
rg -n "CodingRoleRunEvent|list_role_run_events|role_run_event_summary|recent_events" src web/src
```

Expected:

- P1 store event log API exists.
- P2 snapshot and frontend `recent_events` fields exist.

## File Structure

- Modify: `src/product/coding_attempt_store.rs`
  - 新增 retry diagnostic summary helper。
- Modify: `src/product/tester_agent_loop.rs`
  - `build_tester_plan_prompt` 增加 retry diagnostic 参数。
- Modify: `src/product/coding_workspace_engine.rs`
  - 在 retry role run 的 prompt 中追加 diagnostic section。
- Modify: `src/web/coding_ws_handler.rs`
  - Analyst retry evidence 读取时追加旧 run 诊断摘要。
- Modify: `tests/it_product/product_coding_attempt_store.rs`
  - 覆盖 diagnostic summary 内容。
- Modify: `tests/it_product/product_coding_workspace_engine.rs`
  - 覆盖 Tester retry prompt 使用 diagnostic summary，同时保持最终 JSON-only。
- Modify: `tests/it_web/web_coding_ws_handler.rs`
  - 覆盖 Analyst/InternalReviewer retry prompt 捕获旧 run 诊断。
- Modify: `web/e2e/coding-role-runs.spec.ts`
  - 覆盖 seeded events、刷新恢复、retry 后新旧 run 可见。
- Modify: `web/e2e/helpers/coding.ts`
  - seed role run fixture 时写入 event log。
- Modify: `src/web/test_controls.rs`
  - test control fixture 写入 role run events，供 E2E 稳定断言。

## Task 1: Store Retry Diagnostic Summary

**Files:**
- Modify: `src/product/coding_attempt_store.rs`
- Test: `tests/it_product/product_coding_attempt_store.rs`

- [ ] **Step 1: Write failing diagnostic summary test**

In `tests/it_product/product_coding_attempt_store.rs`, append:

```rust
#[test]
fn role_run_retry_diagnostic_summary_compacts_events_and_refs() {
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
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Task update",
                "status": "running",
                "detail": "No tasks found"
            }),
        )
        .expect("event");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::Timeout,
            serde_json::json!({
                "reason_code": "plan_tests_timeout",
                "message": "Tester provider timed out"
            }),
        )
        .expect("timeout");
    store
        .update_role_run_refs(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            vec!["provider-raw/testing/plan_tests_0001.txt".to_string()],
            vec!["artifacts/role-run-events/coding_role_run_0001/0001_output.txt".to_string()],
        )
        .expect("refs");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            CodingRoleRunStatus::Blocked,
            Some("plan_tests_timeout".to_string()),
        )
        .expect("blocked");

    let summary = store
        .role_run_retry_diagnostic_summary("project_0001", "issue_0001", &attempt.id, &run.id)
        .expect("summary")
        .expect("summary text");

    assert!(summary.contains("role_run_id: coding_role_run_0001"));
    assert!(summary.contains("reason_code: plan_tests_timeout"));
    assert!(summary.contains("terminal_event: timeout"));
    assert!(summary.contains("Task update"));
    assert!(summary.contains("No tasks found"));
    assert!(summary.contains("provider-raw/testing/plan_tests_0001.txt"));
    assert!(summary.contains("artifacts/role-run-events/coding_role_run_0001/0001_output.txt"));
    assert!(
        summary.len() < 8_000,
        "retry diagnostic summary must stay prompt-safe"
    );
}
```

- [ ] **Step 2: Run test and confirm failure**

Run:

```bash
cargo test --locked --test it_product role_run_retry_diagnostic_summary_compacts_events_and_refs
```

Expected:

- Test fails because `role_run_retry_diagnostic_summary` does not exist.

- [ ] **Step 3: Implement diagnostic summary helper**

In `src/product/coding_attempt_store.rs`, add this method after `list_role_run_events`:

```rust
    pub fn role_run_retry_diagnostic_summary(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        let run = self.get_role_run(project_id, issue_id, attempt_id, role_run_id)?;
        let events = self.list_role_run_events(project_id, issue_id, attempt_id, role_run_id)?;
        if events.is_empty()
            && run.reason_code.is_none()
            && run.raw_provider_output_refs.is_empty()
            && run.artifact_refs.is_empty()
        {
            return Ok(None);
        }

        let terminal = events.iter().rev().find(|event| {
            matches!(
                event.event_type,
                CodingRoleRunEventType::MessageComplete
                    | CodingRoleRunEventType::ProviderFailed
                    | CodingRoleRunEventType::Timeout
                    | CodingRoleRunEventType::Aborted
            )
        });
        let mut lines = Vec::new();
        lines.push("[previous_role_run_diagnostic]".to_string());
        lines.push(format!("role_run_id: {}", run.id));
        lines.push(format!("stage: {:?}", run.stage));
        lines.push(format!("role: {:?}", run.role));
        lines.push(format!("status: {:?}", run.status));
        if let Some(reason_code) = run.reason_code.as_deref() {
            lines.push(format!("reason_code: {reason_code}"));
        }
        if let Some(event) = terminal {
            lines.push(format!(
                "terminal_event: {}",
                coding_role_run_event_type_name(event.event_type)
            ));
            if let Some(reason) = role_run_event_payload_reason(event) {
                lines.push(format!("terminal_reason: {reason}"));
            }
        }
        lines.push("recent_events:".to_string());
        for event in events.iter().rev().take(5).collect::<Vec<_>>().into_iter().rev() {
            lines.push(format!(
                "- #{} {} title={} status={} detail={}",
                event.sequence,
                coding_role_run_event_type_name(event.event_type),
                role_run_event_payload_text(event, "title").unwrap_or("-"),
                role_run_event_payload_text(event, "status").unwrap_or("-"),
                role_run_event_payload_text(event, "detail")
                    .or_else(|| role_run_event_payload_text(event, "content"))
                    .unwrap_or("-")
            ));
            if let Some(artifact_ref) = event.artifact_ref.as_deref() {
                lines.push(format!("  event_artifact_ref: {artifact_ref}"));
            }
        }
        if !run.raw_provider_output_refs.is_empty() {
            lines.push(format!("raw_provider_output_refs: {}", run.raw_provider_output_refs.join(", ")));
        }
        if !run.artifact_refs.is_empty() {
            lines.push(format!("artifact_refs: {}", run.artifact_refs.join(", ")));
        }
        let summary = truncate_utf8(&lines.join("\n"), 8_000);
        Ok(Some(summary))
    }
```

Add these free functions near `truncate_utf8`:

```rust
fn coding_role_run_event_type_name(event_type: CodingRoleRunEventType) -> &'static str {
    match event_type {
        CodingRoleRunEventType::ProviderPrompt => "provider_prompt",
        CodingRoleRunEventType::ProviderStart => "provider_start",
        CodingRoleRunEventType::TextDelta => "text_delta",
        CodingRoleRunEventType::ExecutionEvent => "execution_event",
        CodingRoleRunEventType::ToolCall => "tool_call",
        CodingRoleRunEventType::ToolResult => "tool_result",
        CodingRoleRunEventType::StatusChanged => "status_changed",
        CodingRoleRunEventType::PermissionRequest => "permission_request",
        CodingRoleRunEventType::ChoiceRequest => "choice_request",
        CodingRoleRunEventType::MessageComplete => "message_complete",
        CodingRoleRunEventType::ProviderFailed => "provider_failed",
        CodingRoleRunEventType::Timeout => "timeout",
        CodingRoleRunEventType::Aborted => "aborted",
        CodingRoleRunEventType::PersistenceWarning => "persistence_warning",
    }
}

fn role_run_event_payload_text<'a>(
    event: &'a CodingRoleRunEvent,
    field: &str,
) -> Option<&'a str> {
    event.payload.get(field).and_then(|value| value.as_str())
}

fn role_run_event_payload_reason(event: &CodingRoleRunEvent) -> Option<&str> {
    role_run_event_payload_text(event, "reason_code")
        .or_else(|| role_run_event_payload_text(event, "message"))
}
```

- [ ] **Step 4: Run store test**

Run:

```bash
cargo test --locked --test it_product role_run_retry_diagnostic_summary_compacts_events_and_refs
```

Expected:

- Test passes.

- [ ] **Step 5: Commit diagnostic helper**

Run:

```bash
git add src/product/coding_attempt_store.rs tests/it_product/product_coding_attempt_store.rs
git commit -m "feat: summarize coding role run retry diagnostics"
```

## Task 2: Prompt Integration Without Breaking JSON-Only Final Outputs

**Files:**
- Modify: `src/product/tester_agent_loop.rs`
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `src/web/coding_ws_handler.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`
- Test: `tests/it_web/web_coding_ws_handler.rs`

- [ ] **Step 1: Write Tester retry prompt test**

In `tests/it_product/product_coding_workspace_engine.rs`, add a capture provider:

```rust
struct TesterRetryPromptCaptureProvider {
    prompts: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for TesterRetryPromptCaptureProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.prompts.lock().expect("prompts").push(input.prompt.clone());
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: r#"{"summary":"retry plan","context_warnings":[],"assumptions":[],"steps":[{"id":"unit","title":"unit","intent":"run unit tests","required":true,"tool":"provider_managed","risk_level":"low","command_or_tool_input":{},"evidence_expectation":"provider evidence"}]}"#.to_string(),
                provider_session_id: None,
            })
            .expect("send completed");
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}
```

Add this test near retry tests:

```rust
#[tokio::test]
async fn retry_test_plan_prompt_includes_previous_role_run_diagnostic() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status("project_0001", "issue_0001", &attempt.id, CodingAttemptStatus::Running)
        .expect("running");
    store
        .update_attempt_status("project_0001", "issue_0001", &attempt.id, CodingAttemptStatus::Blocked)
        .expect("blocked");
    store
        .update_attempt_stage("project_0001", "issue_0001", &attempt.id, CodingExecutionStage::Testing)
        .expect("testing");

    let first_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0001".to_string()),
        )
        .expect("first run");
    store
        .append_role_run_event(
            &attempt,
            &first_run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Task update",
                "status": "running",
                "detail": "No tasks found"
            }),
        )
        .expect("event");
    store
        .append_role_run_event(
            &attempt,
            &first_run,
            CodingRoleRunEventType::Timeout,
            serde_json::json!({
                "reason_code": "plan_tests_timeout",
                "message": "timed out"
            }),
        )
        .expect("timeout");
    let resumed = store
        .update_attempt_status("project_0001", "issue_0001", &attempt.id, CodingAttemptStatus::Running)
        .expect("resume status");
    let retry_run = store
        .supersede_latest_role_run_and_create(
            &resumed,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::RetryTestPlan,
            None,
            Some("plan_tests_timeout".to_string()),
        )
        .expect("retry run");
    assert_eq!(retry_run.supersedes_run_id.as_deref(), Some(first_run.id.as_str()));

    let prompts = Arc::new(Mutex::new(Vec::new()));
    let provider = TesterRetryPromptCaptureProvider {
        prompts: prompts.clone(),
    };
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .execute_testing_with_provider(
            &resumed,
            &provider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions {
                timeout: Duration::from_secs(5),
                failure_limit: 3,
            },
        )
        .await
        .expect("execute retry tester");

    let captured = prompts.lock().expect("prompts");
    let prompt = captured.first().expect("first prompt");
    assert!(prompt.contains("[previous_role_run_diagnostic]"));
    assert!(prompt.contains("reason_code: plan_tests_timeout"));
    assert!(prompt.contains("No tasks found"));
    assert!(prompt.contains("CRITICAL: Return ONLY a single JSON object"));
}
```

- [ ] **Step 2: Update existing web retry prompt tests**

In `coding_ws_retry_analyst_resumes_rework_from_persisted_evidence`, after writing `analyst_evidence`, append a previous event:

```rust
    store
        .append_role_run_event(
            &store
                .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
                .expect("get attempt"),
            &first_run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Analyst task update",
                "status": "running",
                "detail": "No tasks found"
            }),
        )
        .expect("append analyst event");
```

Extend the captured prompt assertion:

```rust
        assert!(
            prompts
                .iter()
                .any(|prompt| prompt.contains("[previous_role_run_diagnostic]")
                    && prompt.contains("Analyst task update")
                    && prompt.contains("No tasks found")),
            "expected analyst retry prompt to contain compact role run diagnostics"
        );
```

In `coding_ws_retry_internal_review_resumes_internal_reviewer_run`, append a previous event to `first_run` and assert captured prompts include:

```rust
        assert!(
            prompts
                .iter()
                .any(|prompt| prompt.contains("[previous_role_run_diagnostic]")
                    && prompt.contains("internal_review_blocked")),
            "expected internal reviewer retry prompt to contain previous run diagnostics"
        );
```

- [ ] **Step 3: Run prompt tests and confirm failure**

Run:

```bash
cargo test --locked --test it_product retry_test_plan_prompt_includes_previous_role_run_diagnostic
cargo test --locked --test it_web coding_ws_retry_analyst_resumes_rework_from_persisted_evidence
cargo test --locked --test it_web coding_ws_retry_internal_review_resumes_internal_reviewer_run
```

Expected:

- Tests fail because prompts do not include `[previous_role_run_diagnostic]`.

- [ ] **Step 4: Extend Tester prompt builder**

In `src/product/tester_agent_loop.rs`, change:

```rust
pub fn build_tester_plan_prompt(
    attempt: &CodingExecutionAttempt,
    evaluation_context_json: &str,
) -> String {
```

to:

```rust
pub fn build_tester_plan_prompt(
    attempt: &CodingExecutionAttempt,
    evaluation_context_json: &str,
    retry_diagnostic: Option<&str>,
) -> String {
```

Add before the final critical line in the returned prompt:

```rust
         {retry_diagnostic_section}\n\
```

Build the variable before `format!`:

```rust
    let retry_diagnostic_section = retry_diagnostic
        .map(|summary| {
            format!(
                "[retry_diagnostic]\n\
                 以下为上一轮 role run 的压缩诊断摘要，只用于规划本轮测试；不要把这段内容原样放入最终 JSON。\n\
                 过程进度通过 provider events 实时输出，最终回答仍必须是 TestPlan JSON。\n\
                 \n{}\n",
                summary
            )
        })
        .unwrap_or_default();
```

Update all direct calls:

```rust
build_tester_plan_prompt(&attempt, &evaluation_context_json, None)
```

- [ ] **Step 5: Add engine diagnostic lookup helper**

In `src/product/coding_workspace_engine.rs`, add:

```rust
    fn retry_diagnostic_for_previous_run(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: &CodingRoleRun,
    ) -> Result<Option<String>, CodingWorkspaceEngineError> {
        let Some(previous_run_id) = role_run.supersedes_run_id.as_deref() else {
            return Ok(None);
        };
        self.store
            .role_run_retry_diagnostic_summary(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                previous_run_id,
            )
            .map_err(CodingWorkspaceEngineError::Store)
    }
```

In `execute_testing_with_provider_commands`, after `role_run` is resolved and before `build_tester_plan_prompt`, add:

```rust
        let retry_diagnostic = self.retry_diagnostic_for_previous_run(&attempt, &role_run)?;
```

Change the prompt call:

```rust
        let plan_prompt = build_tester_plan_prompt(
            &attempt,
            &evaluation_context_json,
            retry_diagnostic.as_deref(),
        );
```

- [ ] **Step 6: Add diagnostic sections to Analyst, CodeReviewer, InternalReviewer prompts**

In `execute_rework_with_commands`, after `role_run` is resolved:

```rust
        let retry_diagnostic = self.retry_diagnostic_for_previous_run(&attempt, &role_run)?;
```

Change `build_rework_prompt` signature:

```rust
fn build_rework_prompt(
    attempt: &CodingExecutionAttempt,
    evidence: &str,
    source_stage: &CodingExecutionStage,
    rework_round: u32,
    context_notes: &ReworkContextNoteInput,
    evaluation_context_json: &str,
    retry_diagnostic: Option<&str>,
) -> String {
```

Add this variable before `format!`:

```rust
    let retry_diagnostic_section = retry_diagnostic
        .map(|summary| {
            format!(
                "\n上一轮 Analyst role run 诊断摘要:\n{}\n",
                summary
            )
        })
        .unwrap_or_default();
```

Add `{retry_diagnostic_section}` after `ContextNotes Truncated` block and update call sites.

For `build_code_review_prompt` and `build_internal_pr_review_prompt`, add `retry_diagnostic: Option<&str>` parameters and include:

```rust
let retry_diagnostic_section = retry_diagnostic
    .map(|summary| {
        format!(
            "\n上一轮 role run 诊断摘要:\n{}\n",
            summary
        )
    })
    .unwrap_or_default();
```

Place `{retry_diagnostic_section}` before the final JSON output requirements. At call sites, compute:

```rust
let retry_diagnostic = self.retry_diagnostic_for_previous_run(&attempt, &role_run)?;
```

and pass `retry_diagnostic.as_deref()`.

- [ ] **Step 7: Append diagnostic to Analyst retry evidence in WebSocket runner**

In `src/web/coding_ws_handler.rs`, update `latest_analyst_role_run_evidence` so after reading `evidence` it appends summary for the same old run:

```rust
    let evidence = coding_store
        .read_attempt_artifact_text(&attempt.id, &evidence_ref)
        .map_err(CodingWorkspaceEngineError::Store)?;
    let diagnostic = coding_store
        .role_run_retry_diagnostic_summary(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &run.id,
        )
        .map_err(CodingWorkspaceEngineError::Store)?;
    Ok(match diagnostic {
        Some(summary) => format!("{evidence}\n\n{summary}"),
        None => evidence,
    })
```

Remove the previous direct `read_attempt_artifact_text` return that immediately maps the store error into `CodingWorkspaceEngineError::Store`.

- [ ] **Step 8: Run prompt tests**

Run:

```bash
cargo test --locked --test it_product retry_test_plan_prompt_includes_previous_role_run_diagnostic
cargo test --locked --test it_web coding_ws_retry_analyst_resumes_rework_from_persisted_evidence
cargo test --locked --test it_web coding_ws_retry_internal_review_resumes_internal_reviewer_run
```

Expected:

- All commands pass.

- [ ] **Step 9: Commit prompt integration**

Run:

```bash
git add src/product/tester_agent_loop.rs src/product/coding_workspace_engine.rs src/web/coding_ws_handler.rs tests/it_product/product_coding_workspace_engine.rs tests/it_web/web_coding_ws_handler.rs
git commit -m "feat: include role run diagnostics in retry prompts"
```

## Task 3: Fixture And Playwright E2E Coverage

**Files:**
- Modify: `src/web/test_controls.rs`
- Modify: `web/e2e/helpers/coding.ts`
- Modify: `web/e2e/coding-role-runs.spec.ts`

- [ ] **Step 1: Seed role run events in test controls**

In `src/web/test_controls.rs`, add `CodingRoleRunEventType` to imports.

After each seeded role run is created, append events. For the tester run:

```rust
    store
        .append_role_run_event(
            &attempt,
            &tester_run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Tester task update",
                "status": "running",
                "detail": "No tasks found"
            }),
        )
        .expect("seed tester role run event");
```

For analyst/internal reviewer seeded runs, use role-specific titles:

```rust
            serde_json::json!({
                "title": "Analyst task update",
                "status": "blocked",
                "detail": "Inspecting previous testing evidence"
            }),
```

```rust
            serde_json::json!({
                "title": "Internal reviewer task update",
                "status": "blocked",
                "detail": "Inspecting pushed review request"
            }),
```

- [ ] **Step 2: Write E2E assertions for refresh recovery**

In `web/e2e/coding-role-runs.spec.ts`, extend `coding role run history renders seeded runs and chat badges`:

```ts
  await expect(history).toContainText("events");
  await expect(history).toContainText("Tester task update");
  await expect(history).toContainText("No tasks found");
  await page.reload();
  const refreshedHistory = page.getByTestId("coding-role-run-history");
  await expect(refreshedHistory).toContainText("Tester task update");
  await expect(refreshedHistory).toContainText("No tasks found");
```

Extend `retry analyst from browser gate creates a new visible run`:

```ts
  await expect(history).toContainText("Analyst task update");
```

Extend `retry internal reviewer from browser gate stays on internal review run`:

```ts
  await expect(history).toContainText("Internal reviewer task update");
```

- [ ] **Step 3: Run E2E and confirm failure before fixture/UI changes are complete**

Run:

```bash
pnpm -C web test:e2e -- coding-role-runs.spec.ts
```

Expected:

- Fails before fixture events and P2 UI are both available.

- [ ] **Step 4: Run backend test controls tests**

Run:

```bash
cargo test --locked --test it_web coding_role_run_fixture_seed_route_creates_attempt_with_runs
```

Expected:

- Test passes after fixture code compiles.

- [ ] **Step 5: Run Playwright E2E**

Run:

```bash
pnpm -C web test:e2e -- coding-role-runs.spec.ts
```

Expected:

- All tests in `coding-role-runs.spec.ts` pass.

- [ ] **Step 6: Commit E2E fixture coverage**

Run:

```bash
git add src/web/test_controls.rs web/e2e/coding-role-runs.spec.ts web/e2e/helpers/coding.ts
git commit -m "test: cover coding role run event recovery"
```

## Task 4: P3 Final Verification

**Files:**
- Verify only.

- [ ] **Step 1: Run focused backend tests**

Run:

```bash
cargo test --locked --test it_product role_run_retry_diagnostic_summary_compacts_events_and_refs
cargo test --locked --test it_product retry_test_plan_prompt_includes_previous_role_run_diagnostic
cargo test --locked --test it_web coding_ws_retry_analyst_resumes_rework_from_persisted_evidence
cargo test --locked --test it_web coding_ws_retry_internal_review_resumes_internal_reviewer_run
cargo test --locked --test it_web coding_role_run_fixture_seed_route_creates_attempt_with_runs
```

Expected:

- All commands pass.

- [ ] **Step 2: Run frontend and E2E checks**

Run:

```bash
pnpm -C web test -- RoleRunHistoryPanel.test.tsx
pnpm -C web test:e2e -- coding-role-runs.spec.ts
```

Expected:

- Both commands pass.

- [ ] **Step 3: Run format/check**

Run:

```bash
cargo fmt --check
cargo check --locked
pnpm -C web build
```

Expected:

- All commands pass.

- [ ] **Step 4: Inspect final diff**

Run:

```bash
git diff --check
git status --short
```

Expected:

- `git diff --check` produces no output.
- `git status --short` is clean after commits.
