use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::types::{AdvanceTaskResponse, ConfirmTaskRequest, CreateTaskRequest};
use tempfile::tempdir;

#[test]
fn manual_write_auto_runs_readonly_internal_step_then_pauses_write_provider_step() {
    let workspace = tempdir().expect("workspace");
    let mut runtime = WebRuntime::new_fake(workspace.path().to_path_buf());
    let created = runtime
        .create_task(request_with_policy("manual-write"))
        .expect("create");
    let response = runtime.advance_task(&created.task_id).expect("advance");
    assert!(matches!(
        response,
        AdvanceTaskResponse::PausedForApproval { .. }
    ));
    let projection = runtime
        .projection(Some(&created.task_id), None)
        .expect("projection");
    assert!(
        projection
            .timeline
            .iter()
            .any(|item| item["node_id"] == "N00" && item["status"] == "completed")
    );
}

#[test]
fn non_interactive_does_not_pause_for_provider_confirmation() {
    let workspace = tempdir().expect("workspace");
    let mut runtime = WebRuntime::new_fake(workspace.path().to_path_buf());
    let created = runtime
        .create_task(request_with_policy("non-interactive"))
        .expect("create");
    let response = runtime.advance_task(&created.task_id).expect("advance");
    assert!(matches!(
        response,
        AdvanceTaskResponse::Advanced { .. } | AdvanceTaskResponse::Completed { .. }
    ));
}

#[test]
fn single_node_policy_override_takes_precedence_for_confirmed_step() {
    let workspace = tempdir().expect("workspace");
    let mut runtime = WebRuntime::new_fake(workspace.path().to_path_buf());
    let created = runtime
        .create_task(request_with_policy("manual-write"))
        .expect("create");
    let pending = runtime
        .advance_task(&created.task_id)
        .expect("advance")
        .expect_pending_step()
        .expect("pending");
    let response = runtime
        .confirm_task(
            &created.task_id,
            ConfirmTaskRequest {
                checkpoint_id: pending.checkpoint_id,
                prompt: pending.prompt,
                policy_override: Some("manual-all".to_string()),
            },
        )
        .expect("confirm");
    assert_eq!(response.status, "provider_started");
}

#[test]
fn manual_all_pauses_first_provider_step_even_when_readonly() {
    let workspace = tempdir().expect("workspace");
    let mut runtime = WebRuntime::new_fake(workspace.path().to_path_buf());
    let created = runtime
        .create_task(request_with_policy("manual-all"))
        .expect("create");
    let response = runtime.advance_task(&created.task_id).expect("advance");
    let pending = response.expect_pending_step().expect("pending");
    assert_eq!(pending.node_id, "N04");
}

fn request_with_policy(policy_preset: &str) -> CreateTaskRequest {
    CreateTaskRequest {
        request_text: "实现 Fibonacci square sum".to_string(),
        change_id: "aria-fibonacci-square".to_string(),
        policy_preset: policy_preset.to_string(),
        provider_mode: "fake".to_string(),
        timeout_secs: 2400,
    }
}
