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
            "canonical_input_refs":["plan_projection_task_0001_0001"],
            "context_files":["openspec/changes/aria-fibonacci-square/tasks.md"],
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
    assert_eq!(
        web.git_summary.workspace_path,
        workspace.path().to_string_lossy()
    );
    assert_eq!(web.artifact_index[0].producer_node, Some("N16".to_string()));
    assert!(
        web.available_actions
            .contains(&"confirm_provider_step".to_string())
    );
}

#[test]
fn web_projection_surfaces_fibonacci_blocked_by_gate_report_breakdown() {
    let workspace = tempdir().expect("workspace");
    let task_root = workspace.path().join(".aria/runtime/tasks/task_0001");
    fs::create_dir_all(task_root.join("reports")).expect("reports");
    fs::write(
        task_root.join("state.json"),
        serde_json::to_vec_pretty(&json!({
            "task_id":"task_0001",
            "phase":"blocked_by_gate",
            "change_id":"aria-fibonacci-square",
            "current_worktask":"work_wt_archive"
        }))
        .expect("state"),
    )
    .expect("write state");
    fs::write(
        task_root.join("reports/final-report.json"),
        serde_json::to_vec_pretty(&json!({
            "status": "blocked_by_gate",
            "business_code": "generated",
            "unit_tests": "passed",
            "coverage_gate": "passed",
            "archive_worktask": "failed",
            "root_cause": "write scope contract"
        }))
        .expect("final"),
    )
    .expect("write final");

    let base = build_workspace_projection(workspace.path(), Some("task_0001")).expect("base");
    let web = build_web_projection(workspace.path(), base, None).expect("web");

    assert_eq!(web.overview["status"], "blocked_by_gate");
    assert_eq!(web.overview["business_code"], "generated");
    assert_eq!(web.overview["unit_tests"], "passed");
    assert_eq!(web.overview["coverage_gate"], "passed");
    assert_eq!(web.overview["archive_worktask"], "failed");
    assert_eq!(web.overview["root_cause"], "write scope contract");
    let text = serde_json::to_string(&web.diagnostics).expect("diagnostics");
    assert!(text.contains("archive_worktask"));
    assert!(text.contains("write scope contract"));
}
