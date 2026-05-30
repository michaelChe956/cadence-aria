use cadence_aria::interactive::projection::build_workspace_projection;
use cadence_aria::interactive::web_projection::build_web_projection;
use serde_json::json;
use std::fs;
use tempfile::tempdir;

#[test]
fn selected_node_context_contains_overview_inputs_outputs_diffs_and_openspec_refs() {
    let workspace = tempdir().expect("workspace");
    let task_root = workspace.path().join(".aria/runtime/tasks/task_0001");
    let openspec_root = workspace
        .path()
        .join("openspec/changes/aria-fibonacci-square");
    fs::create_dir_all(task_root.join("pending")).expect("pending");
    fs::create_dir_all(task_root.join("artifacts/execution")).expect("artifacts");
    fs::create_dir_all(task_root.join("logs")).expect("logs");
    fs::create_dir_all(openspec_root.join("specs/main")).expect("openspec");
    fs::write(
        task_root.join("state.json"),
        serde_json::to_vec_pretty(&json!({
            "task_id": "task_0001",
            "phase": "execution",
            "change_id": "aria-fibonacci-square",
            "current_node": "N16",
            "current_worktask": "work_wt_001"
        }))
        .expect("state"),
    )
    .expect("state file");
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
    .expect("pending file");
    fs::write(
        task_root.join("logs/node-events.jsonl"),
        r#"{"event_kind":"node_completed","task_id":"task_0001","node_id":"N16","status":"completed","details":{"duration_ms":42,"provider_run_id":"run_n16_0001","changed_files":["src/fibonacciSquareSum.js"]}}"#,
    )
    .expect("events");
    fs::write(
        task_root.join("logs/provider-output.jsonl"),
        r#"{"kind":"provider_output","node_id":"N16","provider_run_id":"run_n16_0001","stream":"stdout","text":"done"}"#,
    )
    .expect("provider output");
    fs::write(
        task_root.join("artifacts/execution/0000.json"),
        serde_json::to_vec_pretty(&json!({
            "artifact_ref":"coding_report_work_wt_001_0001",
            "artifact_kind":"coding_report",
            "producer_node":"N16",
            "changed_files":["src/fibonacciSquareSum.js"]
        }))
        .expect("artifact"),
    )
    .expect("artifact file");
    fs::write(openspec_root.join("proposal.md"), "# Proposal\n").expect("proposal");
    fs::write(openspec_root.join("design.md"), "# Design\n").expect("design");
    fs::write(openspec_root.join("tasks.md"), "# Tasks\n").expect("tasks");
    fs::write(openspec_root.join("specs/main/spec.md"), "# Spec\n").expect("spec");

    let base = build_workspace_projection(workspace.path(), Some("task_0001")).expect("base");
    let web = build_web_projection(workspace.path(), base, Some("N16")).expect("web");
    let context = web.selected_node_context;

    assert_eq!(context.node_id, Some("N16".to_string()));
    assert_eq!(context.overview["provider_type"], "codex");
    assert!(
        context
            .inputs
            .iter()
            .any(|item| item["kind"] == "prompt_snapshot")
    );
    assert!(
        context
            .run
            .iter()
            .any(|item| item["kind"] == "provider_output")
    );
    assert!(
        context
            .outputs
            .iter()
            .any(|item| item["artifact_ref"] == "coding_report_work_wt_001_0001")
    );
    assert!(
        context
            .diffs
            .iter()
            .any(|item| item["path"] == "src/fibonacciSquareSum.js")
    );
    assert!(
        web.artifact_index
            .iter()
            .any(|entry| entry.artifact_kind == "openspec_proposal")
    );
    assert!(
        web.artifact_index
            .iter()
            .any(|entry| entry.artifact_kind == "openspec_spec")
    );
}
