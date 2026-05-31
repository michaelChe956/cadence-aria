use cadence_aria::web::error::ApiError;
use cadence_aria::web::types::{
    AdvanceTaskResponse, ConfirmTaskRequest, CreateTaskRequest, PendingProviderStepDto,
    RollbackPreviewRequest, WebEvent,
};
use serde_json::json;

#[test]
fn create_task_request_uses_snake_case_contract() {
    let value = serde_json::from_value::<CreateTaskRequest>(json!({
        "request_text": "实现 Fibonacci square sum",
        "change_id": "aria-fibonacci-square",
        "policy_preset": "manual-write",
        "provider_mode": "fake",
        "timeout_secs": 2400
    }))
    .expect("request json");

    assert_eq!(value.change_id, "aria-fibonacci-square");
    assert_eq!(value.policy_preset, "manual-write");
    assert_eq!(value.provider_mode, "fake");
}

#[test]
fn paused_advance_response_serializes_pending_step() {
    let response = AdvanceTaskResponse::PausedForApproval {
        pending_step: Box::new(PendingProviderStepDto {
            node_id: "N16".to_string(),
            provider_type: "codex".to_string(),
            runtime_role: "executor".to_string(),
            adapter_role: "executor".to_string(),
            prompt: "请实现函数".to_string(),
            input_summary: json!({"worktask_id":"work_wt_001"}),
            canonical_input_refs: vec!["plan_projection_task_0001_0001".to_string()],
            context_files: vec!["openspec/changes/aria-fibonacci-square/tasks.md".to_string()],
            output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
            allowed_write_scope: vec!["src/".to_string(), "tests/".to_string()],
            forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
            verification_commands: vec!["cargo test --locked -j 1".to_string()],
            checkpoint_id: "ckpt_0001".to_string(),
        }),
    };

    let value = serde_json::to_value(response).expect("response json");
    assert_eq!(value["status"], "paused_for_approval");
    assert_eq!(value["pending_step"]["node_id"], "N16");
    assert_eq!(value["pending_step"]["checkpoint_id"], "ckpt_0001");
    assert_eq!(
        value["pending_step"]["canonical_input_refs"][0],
        "plan_projection_task_0001_0001"
    );
    assert_eq!(
        value["pending_step"]["context_files"][0],
        "openspec/changes/aria-fibonacci-square/tasks.md"
    );
}

#[test]
fn confirm_and_rollback_requests_match_frontend_payloads() {
    let confirm = serde_json::from_value::<ConfirmTaskRequest>(json!({
        "checkpoint_id": "ckpt_0001",
        "prompt": "最终确认后的 prompt",
        "policy_override": "manual-all"
    }))
    .expect("confirm");
    assert_eq!(confirm.prompt, "最终确认后的 prompt");
    assert_eq!(confirm.policy_override.as_deref(), Some("manual-all"));

    let preview = serde_json::from_value::<RollbackPreviewRequest>(json!({
        "checkpoint_id": "ckpt_0001"
    }))
    .expect("preview");
    assert_eq!(preview.checkpoint_id, "ckpt_0001");
}

#[test]
fn api_error_serializes_standard_shape() {
    let value = serde_json::to_value(ApiError::validation(
        "invalid_task_request",
        "request_text is required",
    ))
    .expect("error json");
    assert_eq!(value["code"], "invalid_task_request");
    assert_eq!(value["message"], "request_text is required");
    assert_eq!(value["details"], json!({}));
}

#[test]
fn web_event_has_cursor_kind_and_payload() {
    let event = WebEvent {
        cursor: 7,
        event_type: "projection_updated".to_string(),
        task_id: Some("task_0001".to_string()),
        payload: json!({"projection_version": 42}),
    };
    let value = serde_json::to_value(event).expect("event");
    assert_eq!(value["cursor"], 7);
    assert_eq!(value["event_type"], "projection_updated");
}
