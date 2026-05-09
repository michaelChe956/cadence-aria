use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::types::{ConfirmTaskRequest, CreateTaskRequest};
use tempfile::tempdir;

#[test]
fn web_runtime_fake_create_advance_confirm_and_projection() {
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
    assert_eq!(created.task_id, "task_0001");

    let paused = runtime.advance_task(&created.task_id).expect("advance");
    let pending = paused.expect_pending_step().expect("pending");
    assert_eq!(pending.node_id, "N16");

    let confirmed = runtime
        .confirm_task(
            &created.task_id,
            ConfirmTaskRequest {
                checkpoint_id: pending.checkpoint_id,
                prompt: "确认执行 N16".to_string(),
                policy_override: None,
            },
        )
        .expect("confirm");
    assert_eq!(confirmed.node_id, "N16");

    let projection = runtime
        .projection(Some(&created.task_id), Some("N16"))
        .expect("projection");
    assert_eq!(projection.active_task_id, Some("task_0001".to_string()));
    assert!(
        projection
            .timeline
            .iter()
            .any(|item| item["node_id"] == "N16")
    );
}
