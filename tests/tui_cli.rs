use cadence_aria::cli::{CliOutput, run_cli};
use serde_json::json;
use std::fs;
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
