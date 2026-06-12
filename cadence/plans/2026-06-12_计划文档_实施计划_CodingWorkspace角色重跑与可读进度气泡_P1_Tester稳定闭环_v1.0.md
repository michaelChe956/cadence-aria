# Coding RoleRun Tester Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立 Coding Workspace 角色 run 基础，并完成 Tester plan 超时、Tester 可读气泡、Tester 重跑隔离的第一阶段闭环。

**Architecture:** 本阶段采用轻量 `CodingRoleRun` 持久化模型，不改变 attempt/worktree 策略。Tester 执行入口创建 role run，TestPlan、TestingReport、chat entry 绑定 `role_run_id`，重跑时 supersede 旧 Tester run 并重新进入 Testing。Provider 仍输出 JSON-only，后端基于结构化结果生成用户可读 chat entry。

**Tech Stack:** Rust 1.95、serde JSON store、Tokio async、Axum WebSocket contract、React、Zustand、Vitest。

---

## Design Readiness Review

当前 design 符合实施落地条件，但全量范围不适合一个 session 完成。

可落地依据：

- 现有 `CodingExecutionStage`、`CodingProviderRole`、`TestingReport`、`CodingChatEntry` 和 blocked gate action 已覆盖阶段、角色、报告、消息和恢复入口，适合增量扩展。
- 现有 `.aria` JSON store 模式支持新增可选字段和新增子目录，不需要数据库迁移。
- 当前 Tester 已经是两段式 `plan_tests -> execute_test_plan`，可以在不改 Provider contract 的前提下补超时和可读摘要。
- 前端 chat entry metadata 已是 `Record<string, unknown>`，可以先用 `role_run_id` 和 `phase` 做兼容扩展。

需要拆分的原因：

- 全量 design 同时涉及 Tester、Analyst、Code Reviewer、Internal Reviewer 四个角色、后端模型、store、runner、WebSocket snapshot 和前端历史 run UI。
- Analyst 重跑需要持久化 evidence 重建策略；Reviewer/InternalReviewer 需要区分 review request 和 PR review 生命周期。这些不应和 Tester 卡住修复混在一个 session。

拆分建议：

- P1：RoleRun 基础 + Tester 稳定闭环。一个 session 完成。
- P2：Analyst role run + `retry_analyst` + evidence 重建。独立 session。
- P3：Code Reviewer/Internal Reviewer role run + review 重跑。独立 session。
- P4：完整历史 run UI 和真实 E2E 脚本。独立 session。

本计划只覆盖 P1。

## P1 Scope

包含：

- 新增 `CodingRoleRun` 基础模型和 store 方法。
- Tester `plan_tests` 和 `plan_tests_repair` 增加后端超时。
- `TestPlan`、`TestingReport` 绑定 `role_run_id`、`run_no`。
- Tester plan/result 生成用户可读 chat entry，raw JSON 只作为 metadata/ref。
- `retry_test_plan` 和 `rerun_missing_steps` 在 Testing gate 下 supersede 旧 Tester run，并重新执行 Testing。
- 前端能展示 Tester plan/result 可读气泡，并按 `node_id + role_run_id` 避免重跑消息混组。

不包含：

- `retry_analyst` 的完整实现。
- Code Reviewer/Internal Reviewer 的 role run 绑定和重跑。
- 完整历史 run 展开面板。
- 真实浏览器 E2E 自动化脚本。

## File Structure

- Modify: `src/product/coding_models.rs`
  - 增加 `CodingRoleRunStatus`、`CodingRoleRunTrigger`、`CodingRoleRun`。
  - 给 `TestPlan`、`TestingReport` 增加可选 `role_run_id` 和 `run_no`。

- Modify: `src/product/coding_attempt_store.rs`
  - 增加 role run JSON 存储目录。
  - 增加 create/list/latest/update/supersede 方法。

- Modify: `src/product/coding_workspace_engine.rs`
  - Tester 执行入口创建 role run。
  - plan 阶段使用 timeout 运行。
  - 保存可读 Tester plan/result chat entry。
  - gate response 的 Tester 重跑 supersede 当前 Testing run。

- Modify: `src/product/tester_agent_loop.rs`
  - 增加 `format_test_plan_chat_summary` 和 `format_testing_report_chat_summary`。
  - 保持 prompt JSON-only 不变。

- Modify: `src/web/coding_ws_handler.rs`
  - session snapshot 增加 role run 列表。
  - `should_resume_runner_after_gate_response` 保持 Tester gate 会恢复 runner。

- Modify: `web/src/api/types.ts`
  - 增加 `CodingRoleRun` 类型。
  - 给 `TestPlan`/`TestingReport` 对应类型增加 `role_run_id`、`run_no`。

- Modify: `web/src/state/coding-workspace-store.ts`
  - 保存 role runs。
  - chat 分组依赖 metadata 中的 `role_run_id`。

- Modify: `web/src/components/chat-workspace/message-grouping.ts`
  - group key 从 `node_id` 扩展为 `node_id + role_run_id`。

- Modify: `web/src/components/chat-workspace/entries/ProviderStreamEntry.tsx`
  - Tester plan/result 的 markdown 内容正常渲染。

- Tests:
  - `tests/it_product/product_coding_attempt_store.rs`
  - `tests/it_product/product_coding_workspace_engine.rs`
  - `tests/it_product/product_tester_agent_loop.rs`
  - `web/src/state/coding-workspace-store.test.ts`
  - `web/src/components/chat-workspace/message-grouping.test.ts`
  - `web/src/components/chat-workspace/entries/entries.test.tsx`

## Task 1: Backend RoleRun Model And Store

**Files:**
- Modify: `src/product/coding_models.rs`
- Modify: `src/product/coding_attempt_store.rs`
- Test: `tests/it_product/product_coding_attempt_store.rs`

- [ ] **Step 1: Write failing store test**

Add this test to `tests/it_product/product_coding_attempt_store.rs`:

```rust
#[test]
fn saves_reads_and_supersedes_coding_role_runs() {
    let root = tempfile::tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: None,
            ..create_input()
        })
        .expect("create attempt");

    let first = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("first role run");
    assert_eq!(first.id, "coding_role_run_0001");
    assert_eq!(first.run_no, 1);
    assert_eq!(first.status, CodingRoleRunStatus::Running);
    assert_eq!(first.role, CodingProviderRole::Tester);

    let second = store
        .supersede_latest_role_run_and_create(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::RetryTestPlan,
            Some("coding_node_0004".to_string()),
            Some("plan_tests_timeout".to_string()),
        )
        .expect("second role run");

    assert_eq!(second.id, "coding_role_run_0002");
    assert_eq!(second.run_no, 2);
    assert_eq!(second.supersedes_run_id.as_deref(), Some("coding_role_run_0001"));

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Superseded);
    assert_eq!(
        runs[0].superseded_by_run_id.as_deref(),
        Some("coding_role_run_0002")
    );
    assert_eq!(runs[1].status, CodingRoleRunStatus::Running);

    let latest = store
        .latest_role_run(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
        )
        .expect("latest")
        .expect("latest role run");
    assert_eq!(latest.id, "coding_role_run_0002");
}
```

- [ ] **Step 2: Run test and verify it fails**

Run:

```bash
cargo test --locked --test it_product saves_reads_and_supersedes_coding_role_runs
```

Expected: compile failure for missing `CodingRoleRunStatus`, `CodingRoleRunTrigger`, and role run store methods.

- [ ] **Step 3: Add model types**

In `src/product/coding_models.rs`, add these near `CodingProviderRole`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingRoleRunStatus {
    Running,
    Completed,
    Failed,
    Blocked,
    Superseded,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingRoleRunTrigger {
    Initial,
    RetryTestPlan,
    RerunMissingSteps,
    RetryReview,
    RetryAnalyst,
    RetryInternalReview,
    ManualRerun,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingRoleRun {
    pub id: String,
    pub attempt_id: String,
    pub stage: CodingExecutionStage,
    pub role: CodingProviderRole,
    pub run_no: u32,
    pub status: CodingRoleRunStatus,
    pub trigger: CodingRoleRunTrigger,
    pub node_id: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    #[serde(default)]
    pub supersedes_run_id: Option<String>,
    #[serde(default)]
    pub superseded_by_run_id: Option<String>,
    #[serde(default)]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub raw_provider_output_refs: Vec<String>,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
}
```

- [ ] **Step 4: Add optional report fields**

In `src/product/coding_models.rs`, add these fields to both `TestPlan` and `TestingReport`:

```rust
#[serde(default)]
pub role_run_id: Option<String>,
#[serde(default)]
pub run_no: Option<u32>,
```

- [ ] **Step 5: Add store imports and methods**

In `src/product/coding_attempt_store.rs`, import the new model types:

```rust
CodingRoleRun, CodingRoleRunStatus, CodingRoleRunTrigger,
```

Add methods inside `impl CodingAttemptStore`:

```rust
pub fn create_role_run(
    &self,
    attempt: &CodingExecutionAttempt,
    stage: CodingExecutionStage,
    role: CodingProviderRole,
    trigger: CodingRoleRunTrigger,
    node_id: Option<String>,
) -> Result<CodingRoleRun, ProductStoreError> {
    let existing = self.list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let id = next_sequential_id("coding_role_run", existing.len());
    let run_no = existing
        .iter()
        .filter(|run| run.stage == stage && run.role == role)
        .map(|run| run.run_no)
        .max()
        .unwrap_or(0)
        + 1;
    let run = CodingRoleRun {
        id,
        attempt_id: attempt.id.clone(),
        stage,
        role,
        run_no,
        status: CodingRoleRunStatus::Running,
        trigger,
        node_id,
        started_at: Utc::now().to_rfc3339(),
        completed_at: None,
        supersedes_run_id: None,
        superseded_by_run_id: None,
        reason_code: None,
        raw_provider_output_refs: Vec::new(),
        artifact_refs: Vec::new(),
    };
    self.save_role_run(&attempt.project_id, &attempt.issue_id, &run)?;
    Ok(run)
}

pub fn list_role_runs(
    &self,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> Result<Vec<CodingRoleRun>, ProductStoreError> {
    let root = self.role_runs_root(project_id, issue_id, attempt_id);
    let mut runs = Vec::new();
    for path in json_file_paths(&root)? {
        runs.push(read_json(&path)?);
    }
    runs.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(runs)
}

pub fn latest_role_run(
    &self,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
    stage: CodingExecutionStage,
    role: CodingProviderRole,
) -> Result<Option<CodingRoleRun>, ProductStoreError> {
    Ok(self
        .list_role_runs(project_id, issue_id, attempt_id)?
        .into_iter()
        .rev()
        .find(|run| run.stage == stage && run.role == role))
}

pub fn update_role_run_status(
    &self,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
    role_run_id: &str,
    status: CodingRoleRunStatus,
    reason_code: Option<String>,
) -> Result<CodingRoleRun, ProductStoreError> {
    let mut run = self.get_role_run(project_id, issue_id, attempt_id, role_run_id)?;
    run.status = status;
    run.reason_code = reason_code;
    run.completed_at = Some(Utc::now().to_rfc3339());
    self.save_role_run(project_id, issue_id, &run)?;
    Ok(run)
}

pub fn supersede_latest_role_run_and_create(
    &self,
    attempt: &CodingExecutionAttempt,
    stage: CodingExecutionStage,
    role: CodingProviderRole,
    trigger: CodingRoleRunTrigger,
    node_id: Option<String>,
    reason_code: Option<String>,
) -> Result<CodingRoleRun, ProductStoreError> {
    let previous = self.latest_role_run(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        stage.clone(),
        role.clone(),
    )?;
    let mut next = self.create_role_run(attempt, stage, role, trigger, node_id)?;
    next.supersedes_run_id = previous.as_ref().map(|run| run.id.clone());
    next.reason_code = reason_code;
    self.save_role_run(&attempt.project_id, &attempt.issue_id, &next)?;
    if let Some(mut previous_run) = previous {
        previous_run.status = CodingRoleRunStatus::Superseded;
        previous_run.superseded_by_run_id = Some(next.id.clone());
        previous_run.completed_at = Some(Utc::now().to_rfc3339());
        self.save_role_run(&attempt.project_id, &attempt.issue_id, &previous_run)?;
    }
    Ok(next)
}
```

Add private helpers:

```rust
fn role_runs_root(&self, project_id: &str, issue_id: &str, attempt_id: &str) -> PathBuf {
    self.coding_attempt_root(project_id, issue_id, attempt_id)
        .join("role-runs")
}

fn role_run_path(
    &self,
    project_id: &str,
    issue_id: &str,
    run: &CodingRoleRun,
) -> PathBuf {
    self.role_runs_root(project_id, issue_id, &run.attempt_id)
        .join(format!("{}.json", run.id))
}

fn save_role_run(
    &self,
    project_id: &str,
    issue_id: &str,
    run: &CodingRoleRun,
) -> Result<(), ProductStoreError> {
    write_json(&self.role_run_path(project_id, issue_id, run), run)
}

fn get_role_run(
    &self,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
    role_run_id: &str,
) -> Result<CodingRoleRun, ProductStoreError> {
    read_json(
        self.role_runs_root(project_id, issue_id, attempt_id)
            .join(format!("{role_run_id}.json")),
    )
}
```

- [ ] **Step 6: Run store test**

Run:

```bash
cargo test --locked --test it_product saves_reads_and_supersedes_coding_role_runs
```

Expected: PASS.

- [ ] **Step 7: Commit Task 1**

Run:

```bash
git add src/product/coding_models.rs src/product/coding_attempt_store.rs tests/it_product/product_coding_attempt_store.rs
git commit -m "feat: add coding role run store"
```

## Task 2: Bind Tester Execution To RoleRun

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `src/product/tester_agent_loop.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **Step 1: Write failing engine test for role run binding**

Add this test to `tests/it_product/product_coding_workspace_engine.rs`:

```rust
#[tokio::test]
async fn execute_testing_binds_plan_report_and_chat_entries_to_tester_role_run() {
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
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, mut rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        [
            r#"{"summary":"unit plan","steps":[{"id":"unit","title":"Unit","intent":"run unit checks","required":true,"tool":"provider_managed","risk_level":"low","command_or_tool_input":{},"evidence_expectation":"unit evidence"}]}"#,
            r#"{"step_results":[{"step_id":"unit","status":"passed","evidence_refs":["unit.log"],"provider_analysis":"ok"}]}"#,
        ],
        [None, None],
    );

    let report = engine
        .execute_testing_with_provider(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("testing");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].role, CodingProviderRole::Tester);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Completed);
    assert_eq!(report.role_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(report.run_no, Some(1));

    let plans = store
        .list_test_plans("project_0001", "issue_0001", &attempt.id)
        .expect("plans");
    assert_eq!(plans[0].role_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(plans[0].run_no, Some(1));

    let mut saw_plan_entry = false;
    let mut saw_result_entry = false;
    while let Ok(message) = rx.try_recv() {
        if let CodingWsOutMessage::CodingChatEntryCreated { entry } = message {
            let metadata = entry.metadata.unwrap_or_default();
            if metadata.get("role_run_id").and_then(|value| value.as_str())
                == Some(runs[0].id.as_str())
                && metadata.get("phase").and_then(|value| value.as_str()) == Some("test_plan")
            {
                saw_plan_entry = true;
                assert!(entry.content.unwrap_or_default().contains("unit plan"));
            }
            if metadata.get("role_run_id").and_then(|value| value.as_str())
                == Some(runs[0].id.as_str())
                && metadata.get("phase").and_then(|value| value.as_str()) == Some("testing_result")
            {
                saw_result_entry = true;
                assert!(entry.content.unwrap_or_default().contains("passed"));
            }
        }
    }
    assert!(saw_plan_entry);
    assert!(saw_result_entry);
}
```

- [ ] **Step 2: Run test and verify it fails**

Run:

```bash
cargo test --locked --test it_product execute_testing_binds_plan_report_and_chat_entries_to_tester_role_run
```

Expected: FAIL because Tester execution does not create role runs or readable entries.

- [ ] **Step 3: Add formatting helpers**

In `src/product/tester_agent_loop.rs`, add:

```rust
pub fn format_test_plan_chat_summary(plan: &TestPlan) -> String {
    let mut output = format!("## Tester 测试计划\n\n{}\n\n", plan.summary.trim());
    if !plan.assumptions.is_empty() {
        output.push_str("### 假设\n");
        for assumption in &plan.assumptions {
            output.push_str("- ");
            output.push_str(assumption);
            output.push('\n');
        }
        output.push('\n');
    }
    output.push_str("### 步骤\n");
    for step in &plan.steps {
        output.push_str("- ");
        output.push_str(&step.id);
        output.push_str(" · ");
        output.push_str(&step.title);
        output.push_str(" · ");
        output.push_str(if step.required { "required" } else { "optional" });
        output.push_str(" · ");
        output.push_str(&format!("{:?}", step.risk_level).to_ascii_lowercase());
        output.push('\n');
        output.push_str("  - 证据预期：");
        output.push_str(&step.evidence_expectation);
        output.push('\n');
    }
    output
}

pub fn format_testing_report_chat_summary(report: &TestingReport) -> String {
    let mut output = format!(
        "## Tester 测试结果\n\n状态：`{:?}`\n",
        report.overall_status
    );
    if let Some(summary) = report.plan_summary.as_deref() {
        output.push_str("\n计划：");
        output.push_str(summary);
        output.push('\n');
    }
    if !report.missing_required_steps.is_empty() {
        output.push_str("\n### 缺失 required steps\n");
        for step in &report.missing_required_steps {
            output.push_str("- ");
            output.push_str(step);
            output.push('\n');
        }
    }
    if !report.skipped_required_steps.is_empty() {
        output.push_str("\n### 跳过 required steps\n");
        for step in &report.skipped_required_steps {
            output.push_str("- ");
            output.push_str(step);
            output.push('\n');
        }
    }
    if !report.steps.is_empty() {
        output.push_str("\n### 执行证据\n");
        for step in &report.steps {
            output.push_str("- ");
            output.push_str(&step.step_id);
            output.push_str(" · ");
            output.push_str(&format!("{:?}", step.status).to_ascii_lowercase());
            if !step.evidence_refs.is_empty() {
                output.push_str(" · ");
                output.push_str(&step.evidence_refs.join(", "));
            }
            output.push('\n');
        }
    }
    if let Some(raw_ref) = report.raw_provider_output_ref.as_deref() {
        output.push_str("\nraw：`");
        output.push_str(raw_ref);
        output.push_str("`\n");
    }
    output
}
```

- [ ] **Step 4: Create Tester role run in engine**

In `execute_testing_with_provider_commands`, after the testing timeline node is created, create a role run:

```rust
let role_run = self.store.create_role_run(
    &attempt,
    CodingExecutionStage::Testing,
    CodingProviderRole::Tester,
    CodingRoleRunTrigger::Initial,
    Some(node.id.clone()),
)?;
```

When a `TestPlan` is parsed, set:

```rust
plan.role_run_id = Some(role_run.id.clone());
plan.run_no = Some(role_run.run_no);
```

When a `TestingReport` is created, set:

```rust
report.role_run_id = Some(role_run.id.clone());
report.run_no = Some(role_run.run_no);
```

- [ ] **Step 5: Emit readable Tester chat entries**

Replace the current raw plan assistant chat entry content with:

```rust
let entry = tester_chat_entry(
    &attempt,
    &node.id,
    &mut chat_entry_sequence,
    CodingEntryType::AssistantMessage,
    Some(format_test_plan_chat_summary(&plan)),
    Some(serde_json::json!({
        "phase": "test_plan",
        "test_plan_id": plan.id.clone(),
        "role_run_id": role_run.id.clone(),
        "run_no": role_run.run_no,
        "raw_provider_output_ref": plan.raw_provider_output_ref.clone()
    })),
);
self.save_and_emit_chat_entry(entry).await;
```

After saving the final testing report, emit:

```rust
let entry = tester_chat_entry(
    &attempt,
    &node.id,
    &mut chat_entry_sequence,
    CodingEntryType::AssistantMessage,
    Some(format_testing_report_chat_summary(&report)),
    Some(serde_json::json!({
        "phase": "testing_result",
        "testing_report_id": report.id.clone(),
        "role_run_id": role_run.id.clone(),
        "run_no": role_run.run_no,
        "raw_provider_output_ref": report.raw_provider_output_ref.clone()
    })),
);
self.save_and_emit_chat_entry(entry).await;
```

- [ ] **Step 6: Update role run status**

After final report status is known:

```rust
let run_status = match report.overall_status {
    TestingOverallStatus::Passed | TestingOverallStatus::PassedWithWarnings => {
        CodingRoleRunStatus::Completed
    }
    TestingOverallStatus::Failed => CodingRoleRunStatus::Failed,
    TestingOverallStatus::Blocked => CodingRoleRunStatus::Blocked,
    TestingOverallStatus::SkippedByUserDecision => CodingRoleRunStatus::Completed,
};
self.store.update_role_run_status(
    &attempt.project_id,
    &attempt.issue_id,
    &attempt.id,
    &role_run.id,
    run_status,
    derive_testing_role_run_reason(&report),
)?;
```

Add helper:

```rust
fn derive_testing_role_run_reason(report: &TestingReport) -> Option<String> {
    report
        .context_warnings
        .iter()
        .find(|warning| warning.contains("provider_start_failed") || warning.contains("timeout"))
        .cloned()
}
```

- [ ] **Step 7: Run binding test**

Run:

```bash
cargo test --locked --test it_product execute_testing_binds_plan_report_and_chat_entries_to_tester_role_run
```

Expected: PASS.

- [ ] **Step 8: Commit Task 2**

Run:

```bash
git add src/product/coding_workspace_engine.rs src/product/tester_agent_loop.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: bind tester runs to readable reports"
```

## Task 3: Add Tester Plan Timeout

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **Step 1: Write failing timeout test**

Add a provider fixture:

```rust
struct HangingPlanTesterProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for HangingPlanTesterProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            if input.prompt.contains("Phase: plan_tests") {
                let _ = event_tx
                    .send(ProviderEvent::Execution(ProviderExecutionEvent {
                        event_id: "task_update_0001".to_string(),
                        kind: ProviderExecutionEventKind::Command,
                        status: ProviderExecutionEventStatus::Running,
                        title: "Task update".to_string(),
                        detail: Some("planning tests".to_string()),
                        command: None,
                        cwd: None,
                        output: None,
                        exit_code: None,
                    }))
                    .await;
                cancel.cancelled().await;
            }
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}
```

Add imports to `tests/it_product/product_coding_workspace_engine.rs`:

```rust
use std::time::Duration;
use tokio::time::timeout;
```

Add test:

```rust
#[tokio::test]
async fn tester_plan_timeout_blocks_with_retry_test_plan_gate() {
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
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let result = timeout(
        Duration::from_millis(250),
        engine.execute_testing_with_provider(
            &attempt,
            &HangingPlanTesterProvider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions {
                timeout: Duration::from_millis(20),
                failure_limit: 3,
            },
        ),
    )
    .await
    .expect("engine should return before outer timeout");
    let report = result
        .expect("timeout becomes blocked report");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert!(report.context_warnings.contains(&"plan_tests_timeout".to_string()));
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].reason_code.as_deref(), Some("plan_tests_timeout"));
    assert!(gates[0]
        .available_actions
        .iter()
        .any(|action| action.action_id == "retry_test_plan"));
}
```

- [ ] **Step 2: Run timeout test and verify it fails**

Run:

```bash
cargo test --locked --test it_product tester_plan_timeout_blocks_with_retry_test_plan_gate
```

Expected: FAIL with `engine should return before outer timeout` before implementation.

- [ ] **Step 3: Implement cancellable plan stream timeout**

Extend `CodingProviderStreamRun` in `coding_workspace_engine.rs`:

```rust
timeout: Option<Duration>,
timeout_reason_code: Option<&'static str>,
```

Add imports at the top of `coding_workspace_engine.rs`:

```rust
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
```

Set existing call sites to:

```rust
timeout: None,
timeout_reason_code: None,
```

For `plan_tests` and `plan_tests_repair`, set:

```rust
timeout: Some(options.timeout),
timeout_reason_code: Some("plan_tests_timeout"),
```

Inside `run_provider_stream_to_completion`, after `let cancel = CancellationToken::new();`, create an optional timeout:

```rust
let timeout = run_timeout_sleep(timeout);
tokio::pin!(timeout);
```

Add helper near the stream loop:

```rust
fn run_timeout_sleep(timeout: Option<Duration>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    match timeout {
        Some(duration) => Box::pin(tokio::time::sleep(duration)),
        None => Box::pin(std::future::pending()),
    }
}
```

Add a timeout branch inside the `tokio::select!` loop so the provider is cancelled:

```rust
_ = &mut timeout => {
    cancel.cancel();
    return Err(CodingWorkspaceEngineError::ProviderStream(
        timeout_reason_code.unwrap_or("provider_stream_timeout").to_string(),
    ));
}
```

In the `Err(error)` branch for `plan_tests`, map timeout to a blocked testing report:

```rust
let reason_code = if error.to_string().contains("plan_tests_timeout") {
    "plan_tests_timeout"
} else {
    "provider_start_failed"
};
return self
    .block_provider_driven_testing(
        &attempt,
        &node,
        reason_code,
        &format!("Tester provider failed during plan_tests: {error}"),
        None,
    )
    .await;
```

Ensure the blocked report receives role run metadata before saving:

```rust
report.role_run_id = Some(role_run.id.clone());
report.run_no = Some(role_run.run_no);
```

- [ ] **Step 4: Run timeout test**

Run:

```bash
cargo test --locked --test it_product tester_plan_timeout_blocks_with_retry_test_plan_gate
```

Expected: PASS in under 5 seconds.

- [ ] **Step 5: Commit Task 3**

Run:

```bash
git add src/product/coding_workspace_engine.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "fix: time out tester planning"
```

## Task 4: Supersede Tester Run On Retry Gate

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **Step 1: Write failing gate retry test**

Add test:

```rust
#[tokio::test]
async fn retry_test_plan_supersedes_latest_testing_role_run_and_resumes_testing() {
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(root.path().join("worktree")),
            ..create_input()
        })
        .expect("create attempt");
    fs::create_dir_all(attempt.worktree_path.as_ref().expect("worktree")).expect("worktree dir");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    let first_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("first run");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &first_run.id,
            CodingRoleRunStatus::Blocked,
            Some("plan_tests_timeout".to_string()),
        )
        .expect("blocked run");
    let gate = store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Testing,
            node_id: Some("coding_node_0003".to_string()),
            role: Some(CodingProviderRole::Tester),
            title: "Testing blocked".to_string(),
            description: "Tester plan timeout".to_string(),
            reason_code: Some("plan_tests_timeout".to_string()),
            evidence_refs: vec![],
            raw_provider_output_ref: None,
            available_actions: vec![
                CodingGateAction {
                    action_id: "retry_test_plan".to_string(),
                    label: "重新执行 Tester".to_string(),
                    action_type: CodingGateActionType::RetryTestPlan,
                },
                CodingGateAction {
                    action_id: "send_raw_output_to_analyst".to_string(),
                    label: "发送给 Analyst 决策".to_string(),
                    action_type: CodingGateActionType::SendRawOutputToAnalyst,
                },
                CodingGateAction {
                    action_id: "abort".to_string(),
                    label: "终止".to_string(),
                    action_type: CodingGateActionType::Abort,
                },
            ],
        })
        .expect("gate");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .handle_blocked_gate_response(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &gate.gate_id,
            "retry_test_plan",
            None,
        )
        .await
        .expect("gate response");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Testing);
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Superseded);
    assert_eq!(runs[1].trigger, CodingRoleRunTrigger::RetryTestPlan);
    assert_eq!(runs[1].run_no, 2);
}
```

- [ ] **Step 2: Run gate retry test and verify it fails**

Run:

```bash
cargo test --locked --test it_product retry_test_plan_supersedes_latest_testing_role_run_and_resumes_testing
```

Expected: FAIL because gate response currently only changes attempt stage.

- [ ] **Step 3: Supersede latest Tester run during gate response**

In `handle_blocked_gate_response`, change Tester retry branch:

```rust
CodingGateActionType::RetryTestPlan | CodingGateActionType::RerunMissingSteps => {
    let trigger = match action.action_type {
        CodingGateActionType::RetryTestPlan => CodingRoleRunTrigger::RetryTestPlan,
        CodingGateActionType::RerunMissingSteps => CodingRoleRunTrigger::RerunMissingSteps,
        _ => CodingRoleRunTrigger::ManualRerun,
    };
    let resumed = self.resume_blocked_attempt_at_stage(&current, CodingExecutionStage::Testing)?;
    self.store.supersede_latest_role_run_and_create(
        &resumed,
        CodingExecutionStage::Testing,
        CodingProviderRole::Tester,
        trigger,
        None,
        gate.reason_code.clone(),
    )?;
    resumed
}
```

In `execute_testing_with_provider_commands`, before creating a new role run, reuse an existing running Tester run with `node_id = None` for this retry:

```rust
let role_run = match self.store.latest_role_run(
    &attempt.project_id,
    &attempt.issue_id,
    &attempt.id,
    CodingExecutionStage::Testing,
    CodingProviderRole::Tester,
)? {
    Some(mut run) if run.status == CodingRoleRunStatus::Running && run.node_id.is_none() => {
        run.node_id = Some(node.id.clone());
        self.store.save_role_run(&attempt.project_id, &attempt.issue_id, &run)?;
        run
    }
    _ => self.store.create_role_run(
        &attempt,
        CodingExecutionStage::Testing,
        CodingProviderRole::Tester,
        CodingRoleRunTrigger::Initial,
        Some(node.id.clone()),
    )?,
};
```

If `save_role_run` is private, expose a narrow public method:

```rust
pub fn attach_role_run_node(
    &self,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
    role_run_id: &str,
    node_id: String,
) -> Result<CodingRoleRun, ProductStoreError> {
    let mut run = self.get_role_run(project_id, issue_id, attempt_id, role_run_id)?;
    run.node_id = Some(node_id);
    self.save_role_run(project_id, issue_id, &run)?;
    Ok(run)
}
```

Then replace the direct save call in `execute_testing_with_provider_commands` with:

```rust
self.store.attach_role_run_node(
    &attempt.project_id,
    &attempt.issue_id,
    &attempt.id,
    &run.id,
    node.id.clone(),
)?
```

- [ ] **Step 4: Run gate retry test**

Run:

```bash
cargo test --locked --test it_product retry_test_plan_supersedes_latest_testing_role_run_and_resumes_testing
```

Expected: PASS.

- [ ] **Step 5: Commit Task 4**

Run:

```bash
git add src/product/coding_workspace_engine.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: rerun tester from blocked gate"
```

## Task 5: Frontend Tester Chat Rendering And Grouping

**Files:**
- Modify: `web/src/api/types.ts`
- Modify: `web/src/state/coding-workspace-store.ts`
- Modify: `web/src/components/chat-workspace/message-grouping.ts`
- Modify: `web/src/components/chat-workspace/entries/ProviderStreamEntry.tsx`
- Test: `web/src/state/coding-workspace-store.test.ts`
- Test: `web/src/components/chat-workspace/message-grouping.test.ts`
- Test: `web/src/components/chat-workspace/entries/entries.test.tsx`

- [ ] **Step 1: Write failing grouping test**

Add to `web/src/components/chat-workspace/message-grouping.test.ts`:

```ts
it("separates rerun tester messages by role run id", () => {
  const entries = [
    makeEntry("run-1-plan", "provider_stream", "tester", "old plan", "coding_node_0003", {
      role_run_id: "coding_role_run_0001",
    }),
    makeEntry("run-2-plan", "provider_stream", "tester", "new plan", "coding_node_0003", {
      role_run_id: "coding_role_run_0002",
    }),
  ];

  const items = groupEntries(entries);

  expect(items).toHaveLength(2);
  expect(items[0]).toMatchObject({ kind: "group" });
  expect(items[1]).toMatchObject({ kind: "group" });
});
```

If local `makeEntry` helper does not accept metadata, extend only that test helper:

```ts
function makeEntry(
  id: string,
  type: ChatEntry["type"],
  role: ChatEntry["role"],
  content: string,
  nodeId?: string,
  metadata?: Record<string, unknown>,
): ChatEntry {
  return {
    id,
    type,
    role,
    content,
    timestamp: "2026-06-12T00:00:00Z",
    node_id: nodeId,
    metadata,
  };
}
```

- [ ] **Step 2: Run grouping test and verify it fails**

Run:

```bash
pnpm test -- --run web/src/components/chat-workspace/message-grouping.test.ts
```

Expected: FAIL because entries with same node are grouped together.

- [ ] **Step 3: Implement role run grouping key**

In `web/src/components/chat-workspace/message-grouping.ts`, change the node key:

```ts
const nodeKey = groupKeyForEntry(entry);
```

Add:

```ts
function groupKeyForEntry(entry: ChatEntry) {
  const roleRunId = entry.metadata?.role_run_id;
  const roleRunKey = typeof roleRunId === "string" && roleRunId.length > 0 ? roleRunId : "legacy";
  return `${entry.node_id ?? "global"}:${roleRunKey}`;
}
```

- [ ] **Step 4: Add API type**

In `web/src/api/types.ts`, add:

```ts
export type CodingRoleRunStatus =
  | "running"
  | "completed"
  | "failed"
  | "blocked"
  | "superseded"
  | "aborted";

export type CodingRoleRunTrigger =
  | "initial"
  | "retry_test_plan"
  | "rerun_missing_steps"
  | "retry_review"
  | "retry_analyst"
  | "retry_internal_review"
  | "manual_rerun";

export type CodingRoleRun = {
  id: string;
  attempt_id: string;
  stage: CodingExecutionStage;
  role: CodingProviderRole;
  run_no: number;
  status: CodingRoleRunStatus;
  trigger: CodingRoleRunTrigger;
  node_id: string | null;
  started_at: string;
  completed_at: string | null;
  supersedes_run_id?: string | null;
  superseded_by_run_id?: string | null;
  reason_code?: string | null;
  raw_provider_output_refs: string[];
  artifact_refs: string[];
};
```

Extend the coding session state type with:

```ts
role_runs: CodingRoleRun[];
```

- [ ] **Step 5: Store role runs from session snapshot**

In `web/src/state/coding-workspace-store.ts`, add state:

```ts
roleRuns: CodingRoleRun[];
```

Initialize:

```ts
roleRuns: [],
```

In `setSessionState`, assign:

```ts
roleRuns: snapshot.role_runs ?? [],
```

- [ ] **Step 6: Write render test for Tester readable content**

Add to `web/src/components/chat-workspace/entries/entries.test.tsx`:

```tsx
it("renders tester plan summaries as readable markdown", () => {
  render(
    <ProviderStreamEntry
      entry={{
        id: "tester-plan",
        type: "provider_stream",
        role: "tester",
        content:
          "## Tester 测试计划\n\nunit plan\n\n### 步骤\n- unit · Unit · required · low\n  - 证据预期：unit evidence",
        timestamp: "2026-06-12T00:00:00Z",
        node_id: "coding_node_0003",
        metadata: {
          phase: "test_plan",
          role_run_id: "coding_role_run_0001",
        },
      }}
    />,
  );

  expect(screen.getByText("Tester 测试计划")).toBeInTheDocument();
  expect(screen.getByText(/unit plan/)).toBeInTheDocument();
  expect(screen.getByText(/证据预期/)).toBeInTheDocument();
});
```

- [ ] **Step 7: Run frontend focused tests**

Run:

```bash
pnpm test -- --run web/src/components/chat-workspace/message-grouping.test.ts web/src/components/chat-workspace/entries/entries.test.tsx web/src/state/coding-workspace-store.test.ts
```

Expected: PASS.

- [ ] **Step 8: Commit Task 5**

Run:

```bash
git add web/src/api/types.ts web/src/state/coding-workspace-store.ts web/src/components/chat-workspace/message-grouping.ts web/src/components/chat-workspace/entries/ProviderStreamEntry.tsx web/src/state/coding-workspace-store.test.ts web/src/components/chat-workspace/message-grouping.test.ts web/src/components/chat-workspace/entries/entries.test.tsx
git commit -m "feat: show tester role run messages"
```

## Task 6: Session Snapshot Contract

**Files:**
- Modify: `src/web/coding_ws_handler.rs`
- Modify: `web/src/api/types.ts`
- Test: `tests/it_web/web_coding_ws_handler.rs`
- Test: `web/src/hooks/useCodingWorkspaceWs.test.tsx`

- [ ] **Step 1: Write failing backend snapshot test**

Add to `tests/it_web/web_coding_ws_handler.rs`:

```rust
#[tokio::test]
async fn coding_session_snapshot_includes_role_runs() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("role run");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState { role_runs, .. } => {
            assert_eq!(role_runs.len(), 1);
            assert_eq!(role_runs[0].role, CodingProviderRole::Tester);
            assert_eq!(role_runs[0].run_no, 1);
            assert_eq!(role_runs[0].node_id.as_deref(), Some("coding_node_0003"));
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}
```

Add `CodingRoleRunTrigger` to the existing `coding_models` import list in this test file.

- [ ] **Step 2: Run backend snapshot test and verify it fails**

Run:

```bash
cargo test --locked --test it_web coding_session_snapshot_includes_role_runs
```

Expected: FAIL because session state lacks `role_runs`.

- [ ] **Step 3: Add role_runs to session state**

In `src/web/coding_ws_handler.rs`, add field to `CodingSessionState` variant:

```rust
role_runs: Vec<CodingRoleRun>,
```

When building session state:

```rust
let role_runs =
    coding_store.list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
```

Then include:

```rust
role_runs,
```

- [ ] **Step 4: Write frontend WS mapping test**

Add to `web/src/hooks/useCodingWorkspaceWs.test.tsx`:

```ts
it("stores role runs from coding session snapshots", () => {
  const harness = renderCodingWorkspaceHook();
  harness.emit({
    type: "coding_session_state",
    attempt_id: "coding_attempt_0001",
    project_id: "project_0001",
    issue_id: "issue_0001",
    work_item_id: "work_item_0001",
    status: "running",
    stage: "testing",
    base_branch: "main",
    worktree_path: "/tmp/worktree",
    rework_count: 0,
    max_auto_rework: 2,
    head_commit: null,
    pushed_remote: null,
    role_provider_config_snapshot: defaultRoleProviderConfig,
    provider_config_snapshot: defaultProviderConfig,
    chat_entries: [],
    timeline_nodes: [],
    active_node_id: null,
    testing_report: null,
    code_review_reports: [],
    review_request: null,
    internal_pr_review: null,
    pending_gates: [],
    latest_analyst_decision: null,
    work_item_markdown: null,
    verification_commands: [],
    role_runs: [
      {
        id: "coding_role_run_0001",
        attempt_id: "coding_attempt_0001",
        stage: "testing",
        role: "tester",
        run_no: 1,
        status: "running",
        trigger: "initial",
        node_id: "coding_node_0003",
        started_at: "2026-06-12T00:00:00Z",
        completed_at: null,
        supersedes_run_id: null,
        superseded_by_run_id: null,
        reason_code: null,
        raw_provider_output_refs: [],
        artifact_refs: [],
      },
    ],
  });

  expect(useCodingWorkspaceStore.getState().roleRuns).toHaveLength(1);
  expect(useCodingWorkspaceStore.getState().roleRuns[0].role).toBe("tester");
});
```

- [ ] **Step 5: Run snapshot tests**

Run:

```bash
cargo test --locked --test it_web coding_session_snapshot_includes_role_runs
pnpm test -- --run web/src/hooks/useCodingWorkspaceWs.test.tsx
```

Expected: PASS.

- [ ] **Step 6: Commit Task 6**

Run:

```bash
git add src/web/coding_ws_handler.rs tests/it_web/web_coding_ws_handler.rs web/src/api/types.ts web/src/hooks/useCodingWorkspaceWs.test.tsx web/src/state/coding-workspace-store.ts
git commit -m "feat: expose coding role runs to workspace"
```

## Task 7: Full Verification

**Files:**
- No production code changes in this task.

- [ ] **Step 1: Run Rust formatting**

Run:

```bash
cargo fmt --check
```

Expected: PASS.

- [ ] **Step 2: Run Rust check**

Run:

```bash
cargo check --locked
```

Expected: PASS.

- [ ] **Step 3: Run Rust clippy**

Run:

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: PASS.

- [ ] **Step 4: Run Rust tests**

Run:

```bash
cargo test --locked
```

Expected: PASS.

- [ ] **Step 5: Run frontend build**

Run:

```bash
pnpm build
```

Expected: PASS. Chunk size warnings are acceptable if the command exits 0.

- [ ] **Step 6: Run frontend tests**

Run:

```bash
pnpm test
```

Expected: PASS.

- [ ] **Step 7: Inspect final diff**

Run:

```bash
git status --short
git log --oneline -6
```

Expected: working tree contains only expected changes for this implementation branch, and the recent commits match the tasks above.

## P1 Landing Criteria

P1 is implementation-ready if all conditions hold:

- `CodingRoleRun` can be saved, listed, marked completed/blocked/superseded, and queried by latest role/stage.
- Tester creates exactly one current role run per execution.
- `retry_test_plan` and `rerun_missing_steps` supersede the prior Tester run before the runner re-enters Testing.
- `plan_tests` timeout produces a blocked testing report with reason `plan_tests_timeout` and a retry gate.
- Tester plan and Tester result appear as readable chat entries with `role_run_id` metadata.
- Frontend does not group old and new Tester role run messages together.
- All focused tests and full verification commands pass.

## Follow-Up Plans

P2 should implement Analyst role run and `retry_analyst`. It must persist source evidence for Analyst reruns before executing the provider.

P3 should implement Code Reviewer and Internal Reviewer role runs. It must distinguish `retry_review` for CodeReview from InternalPrReview by gate stage.

P4 should add complete historical run UI and real E2E regression coverage.
