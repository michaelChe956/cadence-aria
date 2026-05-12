use cadence_aria::cross_cutting::provider_adapter::{
    ProviderAdapter, ProviderAdapterError, STRUCTURED_OUTPUT_END, STRUCTURED_OUTPUT_START,
    parse_last_structured_output,
};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::types::{ConfirmTaskRequest, CreateTaskRequest};
use serde_json::json;
use std::fs;
use std::process::Command;
use std::sync::{Arc, Mutex};

#[test]
fn web_runtime_real_mode_runs_task_orchestrator_provider_path() {
    let workspace = prepare_workspace();
    let seen_schemas = Arc::new(Mutex::new(Vec::new()));
    let provider = ScriptedTaskRunProvider::happy(seen_schemas.clone());
    let mut runtime =
        WebRuntime::new_with_provider(workspace.path().to_path_buf(), Box::new(provider));

    let created = runtime
        .create_task(CreateTaskRequest {
            request_text: "实现 climbStairs(n)，每次可爬 1 或 2 阶，返回不同爬法数量。".to_string(),
            change_id: "aria-climb-stairs".to_string(),
            policy_preset: "manual-write".to_string(),
            provider_mode: "real".to_string(),
            timeout_secs: 2400,
        })
        .expect("create real task");

    let pending = runtime
        .advance_task(&created.task_id)
        .expect("advance")
        .expect_pending_step()
        .expect("pending real provider step");
    assert!(pending.prompt.contains("climbStairs"));
    assert!(!pending.prompt.contains("Fibonacci square sum"));

    runtime
        .confirm_task(
            &created.task_id,
            ConfirmTaskRequest {
                checkpoint_id: pending.checkpoint_id,
                prompt: pending.prompt,
                policy_override: None,
            },
        )
        .expect("confirm real task");

    let final_report = read_json(
        &workspace
            .path()
            .join(".aria/runtime/tasks/task_0001/reports/final-report.json"),
    );
    assert_eq!(final_report["status"], "completed");
    assert_eq!(final_report["change_id"], "aria-climb-stairs");

    let state = read_json(
        &workspace
            .path()
            .join(".aria/runtime/tasks/task_0001/state.json"),
    );
    assert_eq!(state["phase"], "completed");
    assert_eq!(state["openspec_bootstrap_status"], "bootstrapped");
    assert_eq!(state["provider_mode"], "real");

    let seen_schemas = seen_schemas.lock().expect("seen schemas").clone();
    assert!(seen_schemas.contains(&json!("schema://aria/artifacts/clarification_record/v1")));
    assert!(seen_schemas.contains(&json!("schema://aria/artifacts/coding_report/v1")));
    assert!(seen_schemas.contains(&json!("schema://aria/artifacts/final_summary/v1")));
}

#[test]
fn web_runtime_real_mode_preserves_web_metadata_after_provider_failure() {
    let workspace = prepare_workspace();
    let provider = ScriptedTaskRunProvider::failing_at_design();
    let mut runtime =
        WebRuntime::new_with_provider(workspace.path().to_path_buf(), Box::new(provider));
    let request_text = "实现 climbStairs(n)，每次可爬 1 或 2 阶，返回不同爬法数量。".to_string();

    let created = runtime
        .create_task(CreateTaskRequest {
            request_text: request_text.clone(),
            change_id: "aria-climb-stairs-fails-on-design".to_string(),
            policy_preset: "manual-write".to_string(),
            provider_mode: "real".to_string(),
            timeout_secs: 2400,
        })
        .expect("create real task");
    let pending = runtime
        .advance_task(&created.task_id)
        .expect("advance")
        .expect_pending_step()
        .expect("pending real provider step");

    runtime
        .confirm_task(
            &created.task_id,
            ConfirmTaskRequest {
                checkpoint_id: pending.checkpoint_id,
                prompt: pending.prompt,
                policy_override: None,
            },
        )
        .expect_err("design provider failure should fail the real run");

    let state = read_json(
        &workspace
            .path()
            .join(".aria/runtime/tasks/task_0001/state.json"),
    );
    assert_eq!(state["provider_mode"], "real");
    assert_eq!(state["request_text"], request_text);
    assert_eq!(state["policy_preset"], "manual-write");
    assert_eq!(state["timeout_secs"], 2400);
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

#[derive(Debug)]
struct ScriptedTaskRunProvider {
    output_schemas: Arc<Mutex<Vec<serde_json::Value>>>,
    fail_at_design: bool,
}

impl ScriptedTaskRunProvider {
    fn happy(output_schemas: Arc<Mutex<Vec<serde_json::Value>>>) -> Self {
        Self {
            output_schemas,
            fail_at_design: false,
        }
    }

    fn failing_at_design() -> Self {
        Self {
            output_schemas: Arc::new(Mutex::new(Vec::new())),
            fail_at_design: true,
        }
    }
}

impl ProviderAdapter for ScriptedTaskRunProvider {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.output_schemas
            .lock()
            .expect("schemas")
            .push(json!(input.output_schema.clone()));
        if self.fail_at_design && input.output_schema == "schema://aria/artifacts/design/v1" {
            return Err(ProviderAdapterError::execution_failed(
                Some(1),
                "API Error: The server had an error while processing your request",
                "",
                1,
            ));
        }
        let payload = match input.output_schema.as_str() {
            "schema://aria/artifacts/clarification_record/v1" => json!({
                "artifact_kind": "clarification_record",
                "goal_summary": "实现爬楼梯算法",
                "constraints": ["JavaScript", "基础测试"],
                "assumptions": [],
                "open_questions": [],
                "suggested_scope": "climbStairs 函数与测试"
            }),
            "schema://aria/artifacts/spec/v1" => json!({
                "artifact_kind": "spec",
                "markdown": "# Spec\n\n## 功能需求\n\n- [REQ-001] 用户可以计算爬 n 阶楼梯的不同爬法。Priority: must\n\n## 成功标准\n\n- [AC-001] climbStairs(5) 返回 8。Refs: REQ-001\n"
            }),
            "schema://aria/advisory/spec_gate_review/v1" => json!({
                "artifact_kind": "advisory_review",
                "findings": [],
                "blocking_issues": [],
                "decision_recommendation": "pass"
            }),
            "schema://aria/artifacts/design/v1" => json!({
                "artifact_kind": "design",
                "markdown": "# Design\n\n## 设计决策\n\n- [DD-001] 使用迭代动态规划避免指数递归。Refs: REQ-001\n\n## 公共组件\n\n- [CMP-001] climbStairs 函数\n\n## 风险\n\n- [RISK-001] 输入边界需明确。Severity: medium; Refs: DD-001\n"
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
                "markdown": "# Plan\n\n## 工作包\n\n| ID | Description | Execution Mode | Human Reason | Traceability | Acceptance |\n|----|-------------|----------------|--------------|--------------|------------|\n| WT-001 | 实现 climbStairs 函数与测试 | agent_only | | REQ-001, DD-001, TASK-001 | AC-001 |\n\n## 依赖关系\n\n| From | To | Type |\n|------|----|------|\n"
            }),
            "schema://aria/artifacts/dispatch_package/v1" => json!({
                "artifact_kind": "dispatch_package",
                "worktask_routing": []
            }),
            "schema://aria/artifacts/coding_report/v1" => json!({
                "artifact_kind": "coding_report",
                "artifact_ref": "coding_report_work_wt_001_0001",
                "worktask_id": "work_wt_001",
                "files_modified": ["src/climbStairs.js"],
                "commands_run": ["node --test tests/climbStairs.test.js"],
                "candidate_traceability_refs": [],
                "status": "completed"
            }),
            "schema://aria/artifacts/testing_report/v1" => json!({
                "artifact_kind": "testing_report",
                "artifact_ref": "testing_report_work_wt_001_0001",
                "worktask_id": "work_wt_001",
                "commands_run": ["node --test tests/climbStairs.test.js"],
                "tests_passed": true,
                "failures": [],
                "candidate_traceability_refs": []
            }),
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
                    "closed": ["req-001", "dd-001"],
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
                "closed_items": ["req-001", "dd-001"]
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
            files_modified: Vec::new(),
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
