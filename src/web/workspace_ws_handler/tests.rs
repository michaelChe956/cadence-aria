use super::*;
use crate::web::workspace_ws_types::{
    AuthorDecision, HumanConfirmDecision, ProviderConfigSnapshot, RevisionPath, StructuredFeedback,
};
use std::sync::atomic::{AtomicBool, Ordering};

fn provider_config() -> ProviderConfigSnapshot {
    ProviderConfigSnapshot {
        author: ProviderName::ClaudeCode,
        reviewer: None,
        review_rounds: 0,
    }
}

#[test]
fn context_note_is_only_valid_in_prepare_context() {
    let msg = WsInMessage::ContextNote {
        content: "补充上下文".to_string(),
    };

    assert!(is_message_valid_for_stage(
        &msg,
        &WorkspaceStage::PrepareContext
    ));
    assert!(!is_message_valid_for_stage(&msg, &WorkspaceStage::Running));
}

#[test]
fn start_generation_is_only_valid_in_prepare_context() {
    let msg = WsInMessage::StartGeneration {
        provider_config: provider_config(),
        reviewer_enabled: false,
    };

    assert!(is_message_valid_for_stage(
        &msg,
        &WorkspaceStage::PrepareContext
    ));
    assert!(!is_message_valid_for_stage(&msg, &WorkspaceStage::Running));
}

#[test]
fn hello_and_ping_are_valid_for_every_stage() {
    let hello = WsInMessage::Hello {
        session_id: "session-1".to_string(),
        last_seen_node_id: Some("node-1".to_string()),
    };
    let ping = WsInMessage::Ping;

    for stage in [
        WorkspaceStage::PrepareContext,
        WorkspaceStage::Running,
        WorkspaceStage::AuthorConfirm,
        WorkspaceStage::CrossReview,
        WorkspaceStage::ReviewDecision,
        WorkspaceStage::Revision,
        WorkspaceStage::HumanConfirm,
        WorkspaceStage::Completed,
    ] {
        assert!(is_message_valid_for_stage(&hello, &stage));
        assert!(is_message_valid_for_stage(&ping, &stage));
    }
}

#[tokio::test]
async fn idle_timeout_sends_close_control_after_client_quiet() {
    let last_client_message_at = Arc::new(Mutex::new(tokio::time::Instant::now()));
    let (tx, mut rx) = mpsc::channel(1);

    let task = spawn_idle_timeout_task(
        last_client_message_at,
        tx,
        Arc::new(|| false),
        std::time::Duration::from_millis(5),
        std::time::Duration::from_millis(1),
    );

    let control = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
        .await
        .expect("idle timeout control")
        .expect("close control");
    assert!(matches!(control, OutboundControl::CloseDueToIdleTimeout));

    task.abort();
}

#[tokio::test]
async fn idle_timeout_waits_while_provider_run_is_active() {
    let last_client_message_at = Arc::new(Mutex::new(tokio::time::Instant::now()));
    let (tx, mut rx) = mpsc::channel(1);
    let active = Arc::new(AtomicBool::new(true));
    let active_for_task = active.clone();

    let task = spawn_idle_timeout_task(
        last_client_message_at,
        tx,
        Arc::new(move || active_for_task.load(Ordering::SeqCst)),
        std::time::Duration::from_millis(5),
        std::time::Duration::from_millis(1),
    );

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), rx.recv())
            .await
            .is_err()
    );

    active.store(false, Ordering::SeqCst);
    let control = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
        .await
        .expect("idle timeout after active run")
        .expect("close control");
    assert!(matches!(control, OutboundControl::CloseDueToIdleTimeout));

    task.abort();
}

#[test]
fn revision_path_messages_are_only_valid_in_review_decision() {
    let select_path = WsInMessage::SelectRevisionPath {
        path: RevisionPath::ReviseWithContext,
        extra_context: Some("补充修改约束".to_string()),
    };
    let legacy_decision = WsInMessage::ReviewDecisionResponse {
        decision: "continue".to_string(),
        extra_context: None,
    };

    assert!(is_message_valid_for_stage(
        &select_path,
        &WorkspaceStage::ReviewDecision
    ));
    assert!(is_message_valid_for_stage(
        &legacy_decision,
        &WorkspaceStage::ReviewDecision
    ));
    assert!(!is_message_valid_for_stage(
        &select_path,
        &WorkspaceStage::HumanConfirm
    ));
    assert!(!is_message_valid_for_stage(
        &legacy_decision,
        &WorkspaceStage::HumanConfirm
    ));
}

#[test]
fn author_decision_is_only_valid_in_author_confirm() {
    let msg = WsInMessage::AuthorDecision {
        decision: AuthorDecision::Accept,
    };

    assert!(is_message_valid_for_stage(
        &msg,
        &WorkspaceStage::AuthorConfirm
    ));
    assert!(!is_message_valid_for_stage(
        &msg,
        &WorkspaceStage::PrepareContext
    ));
    assert!(!is_message_valid_for_stage(
        &msg,
        &WorkspaceStage::HumanConfirm
    ));
    assert!(requires_stage_validation(&msg));
    assert_eq!(message_type(&msg), "author_decision");
}

#[test]
fn human_confirm_messages_are_only_valid_in_human_confirm() {
    let human_confirm = WsInMessage::HumanConfirm {
        decision: HumanConfirmDecision::RequestChange,
        payload: Some(serde_json::json!({"description": "补充验收条件"})),
    };
    let legacy_request_revision = WsInMessage::RequestRevision {
        feedback: StructuredFeedback {
            feedback_types: vec!["clarity".to_string()],
            description: "补充验收条件".to_string(),
            target_artifact_version: Some(1),
        },
    };
    let legacy_confirm = WsInMessage::Confirm;

    assert!(is_message_valid_for_stage(
        &human_confirm,
        &WorkspaceStage::HumanConfirm
    ));
    assert!(is_message_valid_for_stage(
        &legacy_request_revision,
        &WorkspaceStage::HumanConfirm
    ));
    assert!(is_message_valid_for_stage(
        &legacy_confirm,
        &WorkspaceStage::HumanConfirm
    ));
    assert!(!is_message_valid_for_stage(
        &human_confirm,
        &WorkspaceStage::ReviewDecision
    ));
    assert!(!is_message_valid_for_stage(
        &legacy_request_revision,
        &WorkspaceStage::ReviewDecision
    ));
}

#[test]
fn completed_stage_rejects_business_messages() {
    assert!(!is_message_valid_for_stage(
        &WsInMessage::Abort,
        &WorkspaceStage::Completed
    ));
    assert!(!is_message_valid_for_stage(
        &WsInMessage::ContextNote {
            content: "late note".to_string()
        },
        &WorkspaceStage::Completed
    ));
}

#[test]
fn control_and_legacy_messages_do_not_require_stage_lock_validation() {
    assert!(!requires_stage_validation(&WsInMessage::Abort));
    assert!(!requires_stage_validation(
        &WsInMessage::PermissionResponse {
            id: "permission-1".to_string(),
            approved: true,
            reason: None,
        }
    ));
    assert!(!requires_stage_validation(&WsInMessage::ChoiceResponse {
        id: "choice-1".to_string(),
        selected_option_ids: vec!["continue".to_string()],
        free_text: None,
    }));
    assert!(!requires_stage_validation(&WsInMessage::UserMessage {
        content: "legacy generation request".to_string(),
    }));
    assert!(!requires_stage_validation(&WsInMessage::Rollback {
        checkpoint_id: "cp_001".to_string(),
    }));
    assert!(!requires_stage_validation(&WsInMessage::Hello {
        session_id: "session-1".to_string(),
        last_seen_node_id: None,
    }));
    assert!(!requires_stage_validation(&WsInMessage::Ping));
    assert!(requires_stage_validation(&WsInMessage::ContextNote {
        content: "new protocol action".to_string(),
    }));
}

#[test]
fn choice_response_message_type_is_reported_for_protocol_errors() {
    assert_eq!(
        message_type(&WsInMessage::ChoiceResponse {
            id: "choice-1".to_string(),
            selected_option_ids: vec!["continue".to_string()],
            free_text: None,
        }),
        "choice_response"
    );
}

#[test]
fn missing_active_run_error_uses_protocol_error() {
    let error = missing_active_run_error("choice_response", "choice-1");

    match error {
        WsOutMessage::ProtocolError {
            code,
            message,
            context,
        } => {
            assert_eq!(code, "ACTIVE_RUN_NOT_FOUND");
            assert!(message.contains("choice_response"));
            assert_eq!(
                context
                    .as_ref()
                    .and_then(|value| value.get("id"))
                    .and_then(|value| value.as_str()),
                Some("choice-1")
            );
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
}

#[test]
fn revision_path_maps_to_existing_review_decision_contract() {
    assert_eq!(
        map_revision_path(RevisionPath::Revise, Some("ignored".to_string())),
        ("continue".to_string(), None)
    );
    assert_eq!(
        map_revision_path(
            RevisionPath::ReviseWithContext,
            Some("补充约束".to_string())
        ),
        (
            "continue_with_context".to_string(),
            Some("补充约束".to_string())
        )
    );
    assert_eq!(
        map_revision_path(RevisionPath::SkipToHuman, Some("ignored".to_string())),
        ("human_intervene".to_string(), None)
    );
}

#[test]
fn build_work_item_plan_generate_request_includes_validator_findings_as_revision_feedback() {
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::checkpoint_store::CheckpointStore;
    use crate::product::lifecycle_store::{
        CreateDesignSpecInput, CreateIssueWorkItemPlanInput, CreateStorySpecInput,
        CreateWorkspaceSessionInput,
    };
    use crate::product::models::{
        IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, ProviderName, WorkItemSplitFinding,
        WorkItemSplitFindingSeverity, WorkspaceType,
    };
    use std::sync::Arc;

    let tmp = tempfile::tempdir().unwrap();
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));

    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repo_0001".to_string(),
            title: "Story".to_string(),
        })
        .unwrap();
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "Design".to_string(),
        })
        .unwrap();

    let finding = WorkItemSplitFinding {
        severity: WorkItemSplitFindingSeverity::Error,
        code: "write_scope_required".to_string(),
        message: "work item must have at least one exclusive_write_scope".to_string(),
        work_item_ids: vec!["wi_001".to_string()],
    };
    let plan = lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("issue_work_item_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: vec![story.id],
            source_design_spec_ids: vec![design.id],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec![],
            repository_profile_ref: None,
            verification_plan_ids: vec![],
            dependency_graph: vec![],
            created_from_provider_run: None,
            validator_findings: vec![finding],
        })
        .unwrap();

    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: plan.id,
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .unwrap();

    let session = WorkspaceSession::from_record(session_record);
    let checkpoint_store = Arc::new(CheckpointStore::new(tmp.path().to_path_buf()));
    let (tx, _rx) = mpsc::channel(1);
    let engine = WorkspaceEngine::new_persistent(checkpoint_store, lifecycle.clone(), tx, session);

    let request = build_work_item_plan_generate_request(&engine, &lifecycle).unwrap();

    let feedback = request
        .revision_feedback
        .expect("revision_feedback should be set when plan has findings");
    assert!(feedback.contains("write_scope_required"));
    assert!(feedback.contains("work item must have at least one exclusive_write_scope"));
}
