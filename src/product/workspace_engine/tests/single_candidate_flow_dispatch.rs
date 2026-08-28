use super::*;
use crate::product::lifecycle_store::{
    CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
};
use crate::product::models::{ProviderName, WorkspaceType};
use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};
use crate::web::workspace_ws_types::{ReviewGate, ReviewVerdict, ReviewVerdictType};

fn needs_human_verdict() -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: "needs human decision".to_string(),
        summary: "needs human decision".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

fn work_item_plan_session_input(flow_kind: WorkItemPlanFlowKind) -> CreateWorkspaceSessionInput {
    CreateWorkspaceSessionInput {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "plan_0001".to_string(),
        workspace_type: WorkspaceType::WorkItemPlan,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: ProviderName::Codex,
        review_rounds: 0,
        superpowers_enabled: false,
        openspec_enabled: false,
        work_item_plan_options: Some(WorkItemPlanSessionOptions {
            flow_kind,
            run_policy: RunPolicy::Interactive,
            rollout_snapshot: flow_kind == WorkItemPlanFlowKind::SingleCandidate,
        }),
    }
}

#[test]
fn durable_flow_kind_dispatches_start_retry_and_reconnect_to_explicit_author_run_kinds() {
    let root = tempfile::tempdir().expect("temporary root");
    let lifecycle = LifecycleStore::new(crate::product::app_paths::ProductAppPaths::new(
        root.path().join(".aria"),
    ));
    let legacy = lifecycle
        .create_workspace_session(work_item_plan_session_input(WorkItemPlanFlowKind::Legacy))
        .expect("create legacy session");
    let single = lifecycle
        .create_workspace_session(work_item_plan_session_input(
            WorkItemPlanFlowKind::SingleCandidate,
        ))
        .expect("create single-candidate session");

    let mut legacy_json = serde_json::to_value(&legacy).expect("serialize legacy session");
    legacy_json
        .as_object_mut()
        .expect("workspace session object")
        .remove("flow_kind");
    let legacy_without_flow_kind: crate::product::models::WorkspaceSessionRecord =
        serde_json::from_value(legacy_json)
            .expect("old durable JSON without flow_kind must deserialize");
    assert_eq!(
        legacy_without_flow_kind.flow_kind,
        WorkItemPlanFlowKind::Legacy,
        "old durable sessions must retain the legacy default"
    );

    for flow_kind in [legacy_without_flow_kind.flow_kind, single.flow_kind] {
        let expected_single_candidate = flow_kind == WorkItemPlanFlowKind::SingleCandidate;
        for entry in ["start", "retry", "reconnect"] {
            let kind = ProviderRunKind::work_item_plan_author_for_durable_flow(flow_kind);
            assert_eq!(
                kind.is_single_candidate_work_item_plan_author(),
                expected_single_candidate,
                "{entry} must preserve the durable flow decision in its explicit run kind",
            );
            assert_eq!(
                kind.is_legacy_work_item_plan_author(),
                !expected_single_candidate,
                "{entry} must not route the other flow through a generic author kind",
            );
        }
    }
}

#[test]
fn stale_policy_cas_retry_refreshes_single_candidate_durable_refs() {
    let root = tempfile::tempdir().expect("temporary root");
    let app_paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let mut durable = lifecycle
        .create_workspace_session(work_item_plan_session_input(
            WorkItemPlanFlowKind::SingleCandidate,
        ))
        .expect("create single candidate session");
    durable.work_item_plan_source_revision_ref = Some("old-source-ref".to_string());
    durable.plan_candidate_ir_ref = Some("old-ir-ref".to_string());
    durable.mechanical_report_ref = Some("old-report-ref".to_string());
    crate::product::json_store::write_json(
        &app_paths
            .issue_root(&durable.project_id, &durable.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", durable.id)),
        &durable,
    )
    .expect("persist stale source refs");

    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(durable.clone()),
    );
    engine.policy_route_before_persist = Some(Box::new(|store, session_id| {
        let mut newer = store
            .get_workspace_session(session_id)
            .expect("reload session for competing worker");
        newer.work_item_plan_source_revision_ref = Some("new-source-ref".to_string());
        newer.plan_candidate_ir_ref = Some("new-ir-ref".to_string());
        newer.mechanical_report_ref = Some("new-report-ref".to_string());
        crate::product::json_store::write_json(
            &store
                .app_paths()
                .issue_root(&newer.project_id, &newer.issue_id)
                .join("workspace-sessions")
                .join(format!("{}.json", newer.id)),
            &newer,
        )
        .expect("persist competing refs");
    }));

    let action = engine
        .work_item_policy_action("single-candidate-stale-cas", &needs_human_verdict())
        .expect("policy action after one stale CAS retry");
    assert!(matches!(
        action,
        crate::product::workspace_engine::review::policy_routing::RoutingAction::EnterHumanGate { .. }
    ));
    assert_eq!(
        engine
            .session()
            .work_item_plan_source_revision_ref
            .as_deref(),
        Some("new-source-ref")
    );
    assert_eq!(
        engine.session().plan_candidate_ir_ref.as_deref(),
        Some("new-ir-ref")
    );
    assert_eq!(
        engine.session().mechanical_report_ref.as_deref(),
        Some("new-report-ref")
    );
}
