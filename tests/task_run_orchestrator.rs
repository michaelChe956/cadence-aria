use cadence_aria::cross_cutting::provider_adapter::{
    ProviderAdapter, ProviderAdapterError, STRUCTURED_OUTPUT_END, STRUCTURED_OUTPUT_START,
    parse_last_structured_output,
};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
use cadence_aria::task_run::orchestrator::TaskRunOrchestrator;
use cadence_aria::task_run::types::{ProviderMode, TaskRunRequest, TaskRunStatus};
use serde_json::json;
use std::collections::VecDeque;
use std::fs;
use std::process::Command;
use std::sync::Mutex;

#[test]
fn fake_provider_task_run_updates_state_json_phase_through_lifecycle() {
    let workspace = prepare_workspace();
    let provider = ScriptedTaskRunProvider::happy();
    let request = task_request(workspace.path());

    let _outcome = TaskRunOrchestrator::run_with_provider(request, &provider).expect("task run");

    let state = read_json(&workspace.path().join(".aria/runtime/tasks/task_0001/state.json"));
    assert_eq!(state["task_id"], "task_0001");
    assert_eq!(state["change_id"], "aria-login-jwt");
    assert_eq!(state["phase"], "completed");
    assert_eq!(state["openspec_bootstrap_status"], "bootstrapped");
}

#[test]
fn fake_provider_task_run_completes_planning_and_writes_openspec_tasks() {
    let workspace = prepare_workspace();
    let provider = ScriptedTaskRunProvider::happy();
    let request = task_request(workspace.path());

    let outcome = TaskRunOrchestrator::run_with_provider(request, &provider).expect("task run");

    assert_eq!(outcome.status, TaskRunStatus::Completed);
    assert_eq!(outcome.change_id, "aria-login-jwt");
    assert!(
        workspace
            .path()
            .join("openspec/changes/aria-login-jwt/tasks.md")
            .exists()
    );
    assert!(outcome.report_path.exists());
    assert!(
        fs::read_to_string(
            workspace
                .path()
                .join("openspec/changes/aria-login-jwt/tasks.md")
        )
        .expect("tasks")
        .contains("TASK-001")
    );

    let testing_report = read_json(
        &workspace
            .path()
            .join(".aria/runtime/tasks/task_0001/reports/testing-report.json"),
    );
    assert!(
        testing_report["_aria"]["projection_refs"]
            .as_array()
            .expect("projection refs")
            .iter()
            .any(|projection_ref| projection_ref
                .as_str()
                .is_some_and(|value| value.starts_with("proj_spec_projection_"))),
        "execution artifacts must carry the spec projection ref"
    );

    let final_summary = read_json(
        &workspace
            .path()
            .join(".aria/runtime/tasks/task_0001/reports/final-summary.json"),
    );
    assert!(
        final_summary["_aria"]["projection_refs"]
            .as_array()
            .expect("final projection refs")
            .iter()
            .any(|projection_ref| projection_ref
                .as_str()
                .is_some_and(|value| value.starts_with("proj_spec_projection_"))),
        "final closure must carry the spec projection ref"
    );

    let final_report = read_json(&outcome.report_path);
    let integration_report_path = final_report["integration_report_path"]
        .as_str()
        .expect("integration report path");
    let integration_report = read_json(&std::path::PathBuf::from(integration_report_path));
    assert_eq!(integration_report["artifact_kind"], "integration_report");
    assert_eq!(
        integration_report["integrated_worktasks"],
        json!(["work_wt_001"])
    );

    assert!(
        provider
            .seen_timeouts()
            .iter()
            .all(|timeout| *timeout == 3600),
        "task run timeout must be passed to every provider call"
    );
}

#[test]
fn rejects_interactive_task_run_requests() {
    let workspace = prepare_workspace();
    let provider = ScriptedTaskRunProvider::happy();
    let mut request = task_request(workspace.path());
    request.non_interactive = false;

    let error = TaskRunOrchestrator::run_with_provider(request, &provider)
        .expect_err("interactive task run must fail");

    assert_eq!(error.code, "task_run_requires_non_interactive");
}

#[test]
fn task_run_persists_execution_and_final_provider_records() {
    let workspace = prepare_workspace();
    let provider = ScriptedTaskRunProvider::happy();
    let outcome = TaskRunOrchestrator::run_with_provider(task_request(workspace.path()), &provider)
        .expect("task run");

    let provider_runs_dir = workspace
        .path()
        .join(".aria/runtime/tasks/task_0001/provider-runs");
    for provider_run_id in [
        "run_n16_0001",
        "run_n17_0001",
        "run_n18_0001",
        "run_n25_0001",
        "run_n27_0001",
    ] {
        assert!(
            provider_runs_dir
                .join(provider_run_id)
                .join("run.json")
                .exists(),
            "{provider_run_id} should be persisted"
        );
    }
    assert!(
        outcome
            .testing_report_path
            .expect("testing report")
            .exists()
    );
    assert!(outcome.final_summary_path.expect("final summary").exists());
}

#[test]
fn non_interactive_task_run_writes_blocked_report_when_rework_limit_is_exceeded() {
    let workspace = prepare_workspace();
    let provider = ScriptedTaskRunProvider::testing_always_fails();
    let outcome = TaskRunOrchestrator::run_with_provider(task_request(workspace.path()), &provider)
        .expect("task run");

    assert_eq!(outcome.status, TaskRunStatus::BlockedByGate);
    assert!(
        outcome
            .blocked_report_path
            .expect("blocked report")
            .exists()
    );
    assert!(outcome.final_summary_path.is_none());

    let state = read_json(&workspace.path().join(".aria/runtime/tasks/task_0001/state.json"));
    assert_eq!(state["phase"], "blocked_by_gate");
}

#[test]
fn non_interactive_task_run_writes_blocked_report_when_execution_provider_errors() {
    let workspace = prepare_workspace();
    let provider = ScriptedTaskRunProvider::testing_provider_errors();
    let outcome = TaskRunOrchestrator::run_with_provider(task_request(workspace.path()), &provider)
        .expect("provider execution error should become blocked task run");

    assert_eq!(outcome.status, TaskRunStatus::BlockedByGate);
    assert!(outcome.final_summary_path.is_none());
    assert!(
        outcome
            .blocked_report_path
            .as_ref()
            .expect("blocked report")
            .exists()
    );
    let blocked_report = read_json(
        outcome
            .blocked_report_path
            .as_ref()
            .expect("blocked report"),
    );
    assert_eq!(blocked_report["reason"], "provider_execution_failed");
    assert_eq!(blocked_report["next_node"], "X08");

    let failed_run = read_json(
        &workspace
            .path()
            .join(".aria/runtime/tasks/task_0001/provider-runs/run_n17_0001/run.json"),
    );
    assert_eq!(failed_run["status"], "failed");
    assert_eq!(failed_run["error_code"], "provider_execution_failed");
    assert!(
        outcome
            .testing_report_path
            .as_ref()
            .is_none_or(|path| path.exists()),
        "blocked outcome must not point at a missing testing report"
    );
}

fn prepare_workspace() -> tempfile::TempDir {
    let workspace = tempfile::tempdir().expect("workspace");
    fs::create_dir_all(workspace.path().join("openspec")).expect("openspec dir");
    fs::write(
        workspace.path().join("openspec/config.yaml"),
        "project: naruto\n",
    )
    .expect("openspec config");
    git(workspace.path(), &["init", "-b", "main"]);
    workspace
}

fn task_request(workspace: &std::path::Path) -> TaskRunRequest {
    TaskRunRequest {
        workspace: workspace.to_path_buf(),
        request_text: "做一个用户登录功能".to_string(),
        change_id: "aria-login-jwt".to_string(),
        provider_mode: ProviderMode::Real,
        non_interactive: true,
        timeout_secs: 3600,
    }
}

#[derive(Debug)]
struct ScriptedTaskRunProvider {
    output_schemas: Mutex<Vec<String>>,
    seen_timeouts: Mutex<Vec<u64>>,
    testing_passes: Mutex<VecDeque<bool>>,
    fail_testing_with_provider_error: bool,
}

impl ScriptedTaskRunProvider {
    fn happy() -> Self {
        Self {
            output_schemas: Mutex::new(Vec::new()),
            seen_timeouts: Mutex::new(Vec::new()),
            testing_passes: Mutex::new([true].into_iter().collect()),
            fail_testing_with_provider_error: false,
        }
    }

    fn testing_always_fails() -> Self {
        Self {
            output_schemas: Mutex::new(Vec::new()),
            seen_timeouts: Mutex::new(Vec::new()),
            testing_passes: Mutex::new([false, false, false, false].into_iter().collect()),
            fail_testing_with_provider_error: false,
        }
    }

    fn testing_provider_errors() -> Self {
        Self {
            fail_testing_with_provider_error: true,
            ..Self::happy()
        }
    }

    fn seen_timeouts(&self) -> Vec<u64> {
        self.seen_timeouts.lock().expect("timeouts").clone()
    }
}

impl ProviderAdapter for ScriptedTaskRunProvider {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.output_schemas
            .lock()
            .expect("schemas")
            .push(input.output_schema.clone());
        self.seen_timeouts
            .lock()
            .expect("timeouts")
            .push(input.timeout);
        let payload = match input.output_schema.as_str() {
            "schema://aria/artifacts/clarification_record/v1" => json!({
                "artifact_kind": "clarification_record",
                "goal_summary": "实现登录功能",
                "constraints": ["JWT", "内存存储"],
                "assumptions": ["目标项目可使用 pnpm"],
                "open_questions": [],
                "suggested_scope": "前端登录页与后端 JWT API"
            }),
            "schema://aria/artifacts/spec/v1" => json!({
                "artifact_kind": "spec",
                "markdown": "# Spec\n\n## 功能需求\n\n- [REQ-001] 用户可以登录。Priority: must\n\n## 成功标准\n\n- [AC-001] 登录成功后返回 JWT。Refs: REQ-001\n"
            }),
            "schema://aria/advisory/spec_gate_review/v1" => json!({
                "artifact_kind": "advisory_review",
                "findings": [],
                "blocking_issues": [],
                "decision_recommendation": "pass"
            }),
            "schema://aria/artifacts/design/v1" => json!({
                "artifact_kind": "design",
                "markdown": "# Design\n\n## 设计决策\n\n- [DD-001] 使用 Express JWT API 和 React 登录页。Refs: REQ-001\n\n## 公共组件\n\n- [CMP-001] 登录表单\n\n## 风险\n\n- [RISK-001] 测试命令不明确。Severity: medium; Refs: DD-001\n"
            }),
            "schema://aria/artifacts/design_review/v1" => json!({
                "artifact_kind": "design_review",
                "review_decision": "pass",
                "findings": []
            }),
            "schema://aria/artifacts/readiness_check/v1" => json!({
                "artifact_kind": "readiness_check",
                "ready": true,
                "blocking_items": []
            }),
            "schema://aria/artifacts/plan/v1" => json!({
                "artifact_kind": "plan",
                "markdown": "# Plan\n\n## 工作包\n\n| ID | Description | Execution Mode | Human Reason | Traceability | Acceptance |\n|----|-------------|----------------|--------------|--------------|------------|\n| WT-001 | 实现登录功能 | agent_only | | REQ-001, DD-001, TASK-001 | AC-001 |\n\n## 依赖关系\n\n| From | To | Type |\n|------|----|------|\n"
            }),
            "schema://aria/artifacts/dispatch_package/v1" => json!({
                "artifact_kind": "dispatch_package",
                "worktask_routing": []
            }),
            "schema://aria/artifacts/coding_report/v1" => json!({
                "artifact_kind": "coding_report",
                "artifact_ref": "coding_report_work_wt_001_0001",
                "worktask_id": "work_wt_001",
                "files_modified": ["src/login.ts"],
                "commands_run": ["pnpm test"],
                "candidate_traceability_refs": [],
                "status": "completed"
            }),
            "schema://aria/artifacts/testing_report/v1" => {
                if self.fail_testing_with_provider_error {
                    return Err(ProviderAdapterError::execution_failed(
                        Some(1),
                        "",
                        "provider quota exhausted",
                        1,
                    ));
                }
                json!({
                    "artifact_kind": "testing_report",
                    "artifact_ref": "testing_report_work_wt_001_0001",
                    "worktask_id": "work_wt_001",
                    "commands_run": ["pnpm test"],
                    "tests_passed": self.testing_passes.lock().expect("testing").pop_front().unwrap_or(true),
                    "failures": [],
                    "candidate_traceability_refs": []
                })
            }
            "schema://aria/artifacts/code_review_report/v1" => json!({
                "artifact_kind": "code_review_report",
                "artifact_ref": "code_review_report_work_wt_001_0001",
                "worktask_id": "work_wt_001",
                "findings": [],
                "blocking": false,
                "candidate_traceability_refs": []
            }),
            "schema://aria/artifacts/final_review/v1" => json!({
                "artifact_kind": "final_review",
                "overall_decision": "pass",
                "coverage_summary": {
                    "closed": ["req-001", "dd-001", "task-001"],
                    "uncovered": [],
                    "exempted": []
                },
                "uncovered_items": [],
                "followup_required": false
            }),
            "schema://aria/artifacts/final_summary/v1" => json!({
                "artifact_kind": "final_summary",
                "overall_status": "closed_successfully",
                "next_steps": [],
                "remaining_risks": [],
                "closed_items": ["req-001", "dd-001", "task-001"]
            }),
            other => panic!("unexpected schema {other}"),
        };
        let stdout = format!(
            "provider log\n{STRUCTURED_OUTPUT_START}\n{}\n{STRUCTURED_OUTPUT_END}\n",
            serde_json::to_string(&payload).expect("payload json")
        );
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: stdout.clone(),
            stderr: String::new(),
            structured_output: parse_last_structured_output(&stdout)?,
            files_modified: payload
                .get("files_modified")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect(),
            duration_ms: 1,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

fn read_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap_or_else(|error| {
        panic!("read {}: {error}", path.display());
    }))
    .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn git(cwd: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout={}\nstderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
