use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::types::{ConfirmTaskRequest, CreateTaskRequest};
use serde_json::Value;
use std::fs;
use tempfile::tempdir;

#[test]
fn advance_creates_checkpoint_and_confirm_persists_turn_node_run_provider_run_artifact_report_and_events()
 {
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
    let pending = runtime
        .advance_task(&created.task_id)
        .expect("advance")
        .expect_pending_step()
        .expect("pending");
    let task_root = workspace.path().join(".aria/runtime/tasks/task_0001");
    assert!(task_root.join("checkpoints/ckpt_0001.json").exists());
    assert!(task_root.join("pending/provider-step.json").exists());

    runtime
        .confirm_task(
            &created.task_id,
            ConfirmTaskRequest {
                checkpoint_id: pending.checkpoint_id,
                prompt: "确认执行 N16".to_string(),
                policy_override: None,
            },
        )
        .expect("confirm");

    assert_json_field(
        task_root.join("turns/turn_0001.json"),
        "status",
        "completed",
    );
    assert_json_field(
        task_root.join("node-runs/nrun_0001.json"),
        "status",
        "completed",
    );
    assert_json_field(
        task_root.join("provider-runs/run_n16_0001/run.json"),
        "provider_type",
        "codex",
    );
    assert!(task_root.join("artifacts/execution/0000.json").exists());
    assert!(
        task_root
            .join("reports/provider-run-run_n16_0001.json")
            .exists()
    );
    let events = fs::read_to_string(task_root.join("logs/node-events.jsonl")).expect("events");
    assert!(events.contains("node_started"));
    assert!(events.contains("node_completed"));
    assert!(events.contains("artifact_written"));
}

fn assert_json_field(path: std::path::PathBuf, field: &str, expected: &str) {
    let value: Value = serde_json::from_slice(&fs::read(path).expect("read json")).expect("json");
    assert_eq!(value[field], expected);
}
