use cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError;
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderSession, StreamChunk, StreamingProviderAdapter,
    StreamingProviderInput,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{
    CodingAttemptStore, CreateBlockedGateInput, CreateCodingAttemptInput,
    CreateCodingExecutionUnitInput, CreateGroupCodingAttemptInput,
};
use cadence_aria::product::coding_workspace_runner::CodingRunnerCommand;
use cadence_aria::product::coding_models::{
    CodingAgentRole, CodingAttemptPlanBinding, CodingAttemptStatus, CodingEntryType,
    CodingExecutionAttempt, CodingExecutionStage, CodingExecutionUnitStatus, CodingGateAction,
    CodingGateActionType, CodingGateKind, CodingGateRequired, CodingProviderPermissionMode,
    CodingProviderRole, CodingRoleProviderConfigSnapshot, CodingRoleRunEventType,
    CodingRoleRunStatus, CodingRoleRunTrigger, CodingTimelineNode, CodingTimelineNodeStatus,
    PushStatus, RemoteKind, ReviewRequest, ReviewRequestKind, ReviewVerdict, WorkItemExecutionPlan,
};
use cadence_aria::product::lifecycle_store::{
    CreateIssueWorkItemPlanInput, CreateWorkItemInput, CreateWorkspaceSessionInput,
    LifecycleStore,
};
use cadence_aria::product::models::WorkItemExecutionPlanStatus;
use cadence_aria::product::models::WorkItemStatus;
use cadence_aria::product::models::{
    DependencyGraphRevision, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, LogicalWorkItem,
    PlanRevisionReason, ProviderName, WorkItemPlanLineage, WorkItemPlanRevision,
    WorkItemPlanStatus, WorkItemProjectionBundle, WorkItemRevision, WorkspaceSessionStatus,
    WorkspaceType,
};
use cadence_aria::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use cadence_aria::product::work_item_contract::{
    CanonicalWorkItemContract, HandoffContract, WorkItemContractIdentity, WorkItemGoal,
    WorkItemWritePolicy, canonical_contract_hash,
};
use cadence_aria::product::work_item_revision_store::WorkItemRevisionStore;
use cadence_aria::product::work_item_projection::{
    WorkItemProjectionCompiler, projection_hashes,
};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterRole};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::coding_ws_handler::{
    CodingWsInMessage, CodingWsOutMessage, is_coding_ws_message_allowed,
};
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::{CodingAttemptRunKey, WebAppState};
use cadence_aria::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, ProviderConfigSnapshot,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

static WS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn create_legacy_coding_attempt_fixture(
    store: &CodingAttemptStore,
    input: CreateCodingAttemptInput,
) -> CodingExecutionAttempt {
    let attempt = store.create_attempt(input).expect("create attempt");
    rewrite_as_legacy_coding_attempt_fixture(store, attempt)
}

fn create_legacy_group_coding_attempt_fixture(
    store: &CodingAttemptStore,
    input: CreateGroupCodingAttemptInput,
) -> CodingExecutionAttempt {
    let attempt = store
        .create_group_attempt(input)
        .expect("create group attempt");
    rewrite_as_legacy_coding_attempt_fixture(store, attempt)
}

fn rewrite_as_legacy_coding_attempt_fixture(
    store: &CodingAttemptStore,
    mut attempt: CodingExecutionAttempt,
) -> CodingExecutionAttempt {
    let generated_id = attempt.id.clone();
    store
        .delete_attempt(&attempt.project_id, &attempt.issue_id, &generated_id)
        .expect("delete generated attempt fixture");
    attempt.id = "coding_attempt_0001".to_string();
    store
        .save_coding_attempt(&attempt)
        .expect("save legacy attempt fixture");
    attempt
}

fn seed_authoritative_group_plan_fixture(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = WorkItemPlanLineage {
        id: "work_item_plan_0001".to_string(),
        project_id: attempt.project_id.clone(),
        issue_id: attempt.issue_id.clone(),
        story_spec_refs: Vec::new(),
        design_spec_refs: Vec::new(),
        active_revision_id: None,
        active_amendment_id: None,
        created_at: "2026-07-18T00:00:00Z".to_string(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
    };
    revision_store
        .put_plan_lineage(&lineage)
        .expect("plan lineage");
    let mut bindings = std::collections::BTreeMap::new();
    for (logical_id, revision_id) in [
        ("work_item_0001", "work_item_revision_0001"),
        ("work_item_0002", "work_item_revision_0002"),
    ] {
        let logical = LogicalWorkItem {
            id: logical_id.to_string(),
            plan_id: lineage.id.clone(),
            title: logical_id.to_string(),
            active_revision_id: None,
            created_at: "2026-07-18T00:00:00Z".to_string(),
            updated_at: "2026-07-18T00:00:00Z".to_string(),
        };
        revision_store
            .put_logical_work_item(&lineage, &logical)
            .expect("logical work item");
        let contract = CanonicalWorkItemContract {
            schema_version: 1,
            identity: WorkItemContractIdentity {
                logical_work_item_id: logical.id.clone(),
                title: logical.title.clone(),
                kind: "implementation".to_string(),
            },
            goal: WorkItemGoal {
                summary: logical.title.clone(),
            },
            non_goals: Vec::new(),
            input_contracts: Vec::new(),
            output_contracts: Vec::new(),
            tasks: Vec::new(),
            write_policy: WorkItemWritePolicy {
                exclusive_scopes: Vec::new(),
                forbidden_scopes: Vec::new(),
            },
            acceptance_criteria: Vec::new(),
            verification_checks: Vec::new(),
            handoff_contract: HandoffContract {
                required_fields: Vec::new(),
                provided_contract_refs: Vec::new(),
                reviewer_check_refs: Vec::new(),
            },
            blocker_rules: Vec::new(),
            design_traceability: Vec::new(),
        };
        let revision = WorkItemRevision {
            id: revision_id.to_string(),
            logical_work_item_id: logical.id.clone(),
            source_draft_revision_id: format!("draft_{revision_id}"),
            canonical_contract_hash: canonical_contract_hash(&contract).expect("contract hash"),
            canonical_contract: contract,
            work_item_projection_bundle_id: format!("projection_{revision_id}"),
            verification_plan_revision_id: format!("verification_{revision_id}"),
            created_at: "2026-07-18T00:00:00Z".to_string(),
        };
        revision_store
            .put_work_item_revision(&lineage, &revision)
            .expect("work item revision");
        let projections = WorkItemProjectionCompiler
            .compile(&revision.canonical_contract, &revision.id)
            .expect("compile work item projections");
        let hashes = projection_hashes(&projections).expect("projection hashes");
        revision_store
            .put_work_item_projection_bundle(
                &lineage,
                &WorkItemProjectionBundle {
                    id: revision.work_item_projection_bundle_id.clone(),
                    work_item_revision_id: revision.id.clone(),
                    canonical_contract_hash: revision.canonical_contract_hash.clone(),
                    projection_schema_version: 1,
                    compiler_version: "work-item-projection-compiler-v1".to_string(),
                    human_projection: projections.human,
                    coder_projection: projections.coder,
                    reviewer_projection: projections.reviewer,
                    human_projection_hash: hashes.human,
                    coder_projection_hash: hashes.coder,
                    reviewer_projection_hash: hashes.reviewer,
                    created_at: "2026-07-18T00:00:00Z".to_string(),
                },
            )
            .expect("work item projection bundle");
        revision_store
            .set_active_work_item_revision(&lineage, &logical, None, revision_id)
            .expect("active work item revision");
        bindings.insert(logical.id, revision.id);
    }
    let graph = DependencyGraphRevision {
        id: "dependency_graph_revision_0001".to_string(),
        plan_id: lineage.id.clone(),
        edges: vec![cadence_aria::product::work_item_contract::DependencyContractEdge {
            from: "work_item_0001".to_string(),
            to: "work_item_0002".to_string(),
            required_contracts: Vec::new(),
        }],
        created_at: "2026-07-18T00:00:00Z".to_string(),
    };
    revision_store
        .put_dependency_graph_revision(&lineage, &graph)
        .expect("dependency graph");
    let plan_revision = WorkItemPlanRevision {
        id: "plan_revision_0001".to_string(),
        plan_id: lineage.id.clone(),
        revision_no: 1,
        supersedes: None,
        reason: PlanRevisionReason::InitialCompile,
        work_item_bindings: bindings,
        dependency_graph_revision_id: graph.id,
        validation_report_ref: "validation_report_0001".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        created_at: "2026-07-18T00:00:00Z".to_string(),
    };
    revision_store
        .put_plan_revision(&lineage, &plan_revision)
        .expect("plan revision");
    revision_store
        .set_active_plan_revision(&lineage, &plan_revision.id)
        .expect("active plan revision");
    store
        .save_plan_binding(
            attempt,
            &CodingAttemptPlanBinding {
                attempt_id: attempt.id.clone(),
                plan_id: lineage.id,
                bound_plan_revision_id: plan_revision.id,
                applied_amendment_ids: Vec::new(),
                updated_at: "2026-07-18T00:00:00Z".to_string(),
            },
        )
        .expect("attempt plan binding");
}

#[test]
fn coding_ws_out_messages_serialize_with_coding_message_type_names() {
    let message = CodingWsOutMessage::CodingStageChange {
        stage: CodingExecutionStage::Testing,
    };

    let value = serde_json::to_value(message).expect("serialize");

    assert_eq!(
        value,
        json!({
            "type": "coding_stage_change",
            "stage": "testing"
        })
    );

    let provider_update = CodingWsOutMessage::CodingProviderConfigUpdated {
        role: CodingProviderRole::Coder,
        provider: ProviderName::Codex,
    };

    assert_eq!(
        serde_json::to_value(provider_update).expect("serialize provider update"),
        json!({
            "type": "coding_provider_config_updated",
            "role": "coder",
            "provider": "codex"
        })
    );
}

#[test]
fn coding_ws_in_messages_deserialize_client_commands() {
    let message: CodingWsInMessage = serde_json::from_value(json!({
        "type": "gate_response",
        "gate_id": "gate_0001",
        "action_id": "send_to_coder",
        "extra_context": "已补充测试"
    }))
    .expect("deserialize");

    assert_eq!(
        message,
        CodingWsInMessage::GateResponse {
            gate_id: "gate_0001".to_string(),
            action_id: "send_to_coder".to_string(),
            extra_context: Some("已补充测试".to_string())
        }
    );

    let provider_select: CodingWsInMessage = serde_json::from_value(json!({
        "type": "provider_select",
        "role": "author",
        "provider": "codex"
    }))
    .expect("deserialize provider select");

    assert_eq!(
        provider_select,
        CodingWsInMessage::ProviderSelect {
            role: "author".to_string(),
            provider: ProviderName::Codex
        }
    );

    let stage_gate_confirm: CodingWsInMessage = serde_json::from_value(json!({
        "type": "stage_gate_confirm",
        "stage": "testing"
    }))
    .expect("deserialize stage gate confirm");

    assert_eq!(
        stage_gate_confirm,
        CodingWsInMessage::StageGateConfirm {
            stage: CodingExecutionStage::Testing,
        }
    );

    let max_auto_rework_select: CodingWsInMessage = serde_json::from_value(json!({
        "type": "max_auto_rework_select",
        "max_auto_rework": 4
    }))
    .expect("deserialize max auto rework select");

    assert_eq!(
        max_auto_rework_select,
        CodingWsInMessage::MaxAutoReworkSelect { max_auto_rework: 4 }
    );
}

#[test]
fn coding_ws_stage_validation_matches_attempt_status_and_stage() {
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Running,
        &CodingExecutionStage::PrepareContext,
        &CodingWsInMessage::StartCoding,
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Running,
        &CodingExecutionStage::Testing,
        &CodingWsInMessage::ContextNote {
            content: "补充背景".to_string()
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::WaitingForHuman,
        &CodingExecutionStage::FinalConfirm,
        &CodingWsInMessage::ContextNote {
            content: "最终确认前补充背景".to_string()
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Blocked,
        &CodingExecutionStage::CodeReview,
        &CodingWsInMessage::ContextNote {
            content: "阻塞时补充背景".to_string()
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::WaitingForHuman,
        &CodingExecutionStage::FinalConfirm,
        &CodingWsInMessage::FinalConfirm,
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Blocked,
        &CodingExecutionStage::CodeReview,
        &CodingWsInMessage::GateResponse {
            gate_id: "gate_0001".to_string(),
            action_id: "accept_risk".to_string(),
            extra_context: None
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Completed,
        &CodingExecutionStage::FinalConfirm,
        &CodingWsInMessage::CodingPing,
    ));
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::Completed,
        &CodingExecutionStage::FinalConfirm,
        &CodingWsInMessage::AbortAttempt,
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Created,
        &CodingExecutionStage::PrepareContext,
        &CodingWsInMessage::ProviderSelect {
            role: "author".to_string(),
            provider: ProviderName::Codex,
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Created,
        &CodingExecutionStage::PrepareContext,
        &CodingWsInMessage::MaxAutoReworkSelect { max_auto_rework: 4 },
    ));
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::Running,
        &CodingExecutionStage::Coding,
        &CodingWsInMessage::MaxAutoReworkSelect { max_auto_rework: 4 },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Running,
        &CodingExecutionStage::Testing,
        &CodingWsInMessage::ProviderSelect {
            role: "author".to_string(),
            provider: ProviderName::Codex,
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Running,
        &CodingExecutionStage::Testing,
        &CodingWsInMessage::StageGateConfirm {
            stage: CodingExecutionStage::Testing,
        },
    ));
}

#[test]
fn blocked_attempt_allows_gate_response_messages() {
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Blocked,
        &CodingExecutionStage::Testing,
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_0001".to_string(),
            action_id: "retry_test_plan".to_string(),
            extra_context: None,
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Blocked,
        &CodingExecutionStage::Testing,
        &CodingWsInMessage::AbortAttempt,
    ));
}

#[test]
fn waiting_for_human_code_review_allows_blocked_gate_responses() {
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::WaitingForHuman,
        &CodingExecutionStage::CodeReview,
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_0007".to_string(),
            action_id: "send_to_coder".to_string(),
            extra_context: None,
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::WaitingForHuman,
        &CodingExecutionStage::Testing,
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_0008".to_string(),
            action_id: "accept_testing_result".to_string(),
            extra_context: None,
        },
    ));
}

#[test]
fn coding_gate_required_out_message_preserves_action_contract() {
    let gate = CodingGateRequired {
        gate_id: "gate_0001".to_string(),
        kind: CodingGateKind::Blocked,
        title: "需要人工决策".to_string(),
        description: "测试失败次数达到上限".to_string(),
        stage: None,
        role: None,
        expires_at: None,
        provider_snapshot: None,
        available_actions: vec![CodingGateAction {
            action_id: "accept_risk".to_string(),
            label: "接受风险".to_string(),
            action_type: CodingGateActionType::AcceptRisk,
        }],
        reason_code: None,
        evidence_refs: Vec::new(),
        raw_provider_output_ref: None,
    };
    let message = CodingWsOutMessage::CodingGateRequired { gate };

    let value = serde_json::to_value(message).expect("serialize");

    assert_eq!(value["type"], "coding_gate_required");
    assert_eq!(value["gate"]["kind"], "blocked");
    assert_eq!(
        value["gate"]["available_actions"][0]["action_type"],
        "accept_risk"
    );
}

#[tokio::test]
async fn coding_ws_sends_session_state_on_connect_and_responds_to_ping() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            attempt_id,
            status,
            stage,
            branch_name,
            role_provider_config_snapshot,
            timeline_nodes,
            testing_report,
            ..
        } => {
            assert_eq!(attempt_id, "coding_attempt_0001");
            assert_eq!(status, CodingAttemptStatus::Created);
            assert_eq!(stage, CodingExecutionStage::PrepareContext);
            assert_eq!(branch_name, "aria/work-items/work_item_0001/attempt-1");
            assert_eq!(role_provider_config_snapshot.coder, ProviderName::Fake);
            assert_eq!(
                role_provider_config_snapshot.code_reviewer,
                ProviderName::Fake
            );
            assert!(timeline_nodes.is_empty());
            assert!(testing_report.is_none());
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    send_json(&mut ws, &CodingWsInMessage::CodingPing).await;
    assert_eq!(recv_json(&mut ws).await, CodingWsOutMessage::CodingPong);

    ws.close(None).await.expect("close ws");
    server.abort();
}
#[tokio::test]
async fn coding_session_snapshot_includes_role_runs() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("role run");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ProviderPrompt,
            serde_json::json!({
                "mode": "plan_tests",
                "prompt": "plan tests"
            }),
        )
        .expect("prompt event");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Task update",
                "status": "running",
                "detail": "No tasks found"
            }),
        )
        .expect("execution event");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::Aborted,
            serde_json::json!({
                "reason": "abort_attempt"
            }),
        )
        .expect("aborted event");
    let provider_failed_detail = format!("provider failed detail: {}", "d".repeat(16_500));
    let provider_failed_detail_preview = provider_failed_detail[..16_384].to_string();
    let provider_failed_message = format!("provider failed: {}", "x".repeat(16_500));
    let provider_failed_preview = provider_failed_message[..16_384].to_string();
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ProviderFailed,
            serde_json::json!({
                "detail": provider_failed_detail,
                "message": provider_failed_message
            }),
        )
        .expect("provider failed event");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    let raw_state = recv_json_value(&mut ws).await;
    assert_eq!(raw_state["role_runs"][0]["role"], "tester");
    assert!(raw_state["role_runs"][0].get("run").is_none());

    match serde_json::from_value(raw_state).expect("coding session state") {
        CodingWsOutMessage::CodingSessionState { role_runs, .. } => {
            assert_eq!(role_runs.len(), 1);
            assert_eq!(role_runs[0].role, CodingProviderRole::Tester);
            assert_eq!(role_runs[0].run_no, 1);
            assert_eq!(role_runs[0].node_id.as_deref(), Some("coding_node_0003"));
            let summary = role_runs[0].event_summary.as_ref().expect("event summary");
            assert_eq!(summary.event_count, 4);
            assert_eq!(
                summary.last_event_type,
                Some(CodingRoleRunEventType::ProviderFailed)
            );
            assert_eq!(summary.last_event_title.as_deref(), Some("ProviderFailed"));
            assert_eq!(summary.last_event_status.as_deref(), None);
            assert_eq!(
                summary.terminal_event_type,
                Some(CodingRoleRunEventType::ProviderFailed)
            );
            assert_eq!(
                summary.terminal_reason.as_deref(),
                Some(provider_failed_preview.as_str())
            );
            assert_eq!(role_runs[0].recent_events.len(), 4);
            assert_eq!(
                role_runs[0].recent_events[1].title.as_deref(),
                Some("Task update")
            );
            assert_eq!(
                role_runs[0].recent_events[3].detail.as_deref(),
                Some(provider_failed_detail_preview.as_str())
            );
            assert!(role_runs[0].recent_events[3].truncated);
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_session_snapshot_ignores_corrupt_role_run_events() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("role run");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Task update",
                "status": "running",
                "detail": "No tasks found"
            }),
        )
        .expect("execution event");
    let event_log = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/role-run-events")
        .join(format!("{}.jsonl", run.id));
    fs::write(&event_log, "{not valid jsonl\n").expect("corrupt role run event log");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState { role_runs, .. } => {
            assert_eq!(role_runs.len(), 1);
            assert_eq!(role_runs[0].role, CodingProviderRole::Tester);
            assert!(role_runs[0].event_summary.is_none());
            assert!(role_runs[0].recent_events.is_empty());
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_session_state_includes_persisted_open_stage_gates() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    store
        .create_stage_gate(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            "2026-05-28T00:00:05Z".to_string(),
            CodingRoleProviderConfigSnapshot::from(ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            }),
        )
        .expect("create stage gate");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState { pending_gates, .. } => {
            assert_eq!(pending_gates.len(), 1);
            assert_eq!(pending_gates[0].gate_id, "coding_stage_gate_0001");
            assert_eq!(pending_gates[0].kind, CodingGateKind::StageGate);
            assert_eq!(pending_gates[0].stage, Some(CodingExecutionStage::Testing));
            assert_eq!(pending_gates[0].role, Some(CodingProviderRole::Tester));
            assert_eq!(
                pending_gates[0].expires_at.as_deref(),
                Some("2026-05-28T00:00:05Z")
            );
            assert!(pending_gates[0].title.contains("Testing"));
            assert_eq!(
                pending_gates[0].available_actions[0].action_type,
                CodingGateActionType::ConfirmStage
            );
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}
