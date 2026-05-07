use cadence_aria::interactive::models::{ArtifactStatus, ContentType};
use cadence_aria::interactive::projection::build_workspace_projection;
use serde_json::json;
use std::fs;
use tempfile::tempdir;

#[test]
fn projection_reads_state_reports_events_and_artifacts() {
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

    let projection =
        build_workspace_projection(workspace.path(), Some("task_0001")).expect("build projection");

    assert_eq!(projection.active_task_id.as_deref(), Some("task_0001"));
    assert_eq!(projection.overview["phase"], "blocked_by_gate");
    assert_eq!(projection.overview["change_id"], "aria-fibonacci-square");
    assert!(
        projection
            .timeline
            .iter()
            .any(|entry| { entry["node_id"] == "N16" && entry["status"] == "completed" })
    );
    assert!(projection.artifact_index.iter().any(|entry| {
        entry.artifact_ref == "coding_report_work_wt_006_0001"
            && entry.status == ArtifactStatus::Active
            && entry.content_type == ContentType::Json
    }));
}

#[test]
fn projection_uses_empty_state_when_state_file_is_missing() {
    let workspace = tempdir().expect("workspace");
    let task_root = workspace.path().join(".aria/runtime/tasks/task_0002");
    fs::create_dir_all(task_root.join("reports")).expect("reports dir");
    fs::write(
        task_root.join("reports/final-report.json"),
        serde_json::to_vec_pretty(&json!({
            "change_id": "aria-no-state",
            "status": "completed"
        }))
        .expect("final json"),
    )
    .expect("write final");

    let projection =
        build_workspace_projection(workspace.path(), Some("task_0002")).expect("build projection");

    assert_eq!(projection.active_task_id.as_deref(), Some("task_0002"));
    assert_eq!(projection.overview["task_id"], "task_0002");
    assert_eq!(projection.overview["change_id"], "aria-no-state");
    assert_eq!(projection.overview["status"], "completed");
}

#[test]
fn projection_rejects_task_id_that_would_escape_task_root() {
    let workspace = tempdir().expect("workspace");

    let error = build_workspace_projection(workspace.path(), Some("../outside"))
        .expect_err("reject escaping task id");

    assert_eq!(error.code, "interactive_projection_invalid_task_id");
}

#[test]
fn projection_fallback_artifact_refs_are_unique_for_nested_same_name_files() {
    let workspace = tempdir().expect("workspace");
    let task_root = workspace.path().join(".aria/runtime/tasks/task_0003");
    fs::create_dir_all(task_root.join("artifacts/a")).expect("artifact a dir");
    fs::create_dir_all(task_root.join("artifacts/b")).expect("artifact b dir");
    fs::write(task_root.join("artifacts/a/0000.md"), "# A\n").expect("write a");
    fs::write(task_root.join("artifacts/b/0000.md"), "# B\n").expect("write b");

    let projection =
        build_workspace_projection(workspace.path(), Some("task_0003")).expect("projection");
    let mut refs = projection
        .artifact_index
        .iter()
        .map(|entry| entry.artifact_ref.as_str())
        .collect::<Vec<_>>();
    refs.sort();

    assert_eq!(refs, vec!["artifacts_a_0000", "artifacts_b_0000"]);
}

#[test]
fn projection_classifies_test_artifacts_from_path() {
    let workspace = tempdir().expect("workspace");
    let task_root = workspace.path().join(".aria/runtime/tasks/task_0004");
    fs::create_dir_all(task_root.join("artifacts/tests")).expect("tests dir");
    fs::write(
        task_root.join("artifacts/tests/fibonacci_square_sum.test.js"),
        "test('works', () => {});\n",
    )
    .expect("write test");

    let projection =
        build_workspace_projection(workspace.path(), Some("task_0004")).expect("projection");

    let entry = projection
        .artifact_index
        .iter()
        .find(|entry| entry.path.ends_with("fibonacci_square_sum.test.js"))
        .expect("test artifact");
    assert_eq!(entry.content_type, ContentType::Test);
}

#[test]
fn projection_reads_json_metadata_even_when_json_file_is_classified_as_test() {
    let workspace = tempdir().expect("workspace");
    let task_root = workspace.path().join(".aria/runtime/tasks/task_0005");
    fs::create_dir_all(task_root.join("artifacts/tests")).expect("tests dir");
    fs::write(
        task_root.join("artifacts/tests/result.test.json"),
        serde_json::to_vec_pretty(&json!({
            "artifact_kind": "testing_report",
            "artifact_ref": "testing_report_task_0005_0001",
            "_aria": {
                "traceability_refs": ["REQ-1"]
            }
        }))
        .expect("test json"),
    )
    .expect("write test json");

    let projection =
        build_workspace_projection(workspace.path(), Some("task_0005")).expect("projection");

    let entry = projection
        .artifact_index
        .iter()
        .find(|entry| entry.artifact_ref == "testing_report_task_0005_0001")
        .expect("test json artifact");
    assert_eq!(entry.artifact_kind, "testing_report");
    assert_eq!(entry.content_type, ContentType::Test);
    assert_eq!(entry.traceability_refs, vec!["REQ-1"]);
}
