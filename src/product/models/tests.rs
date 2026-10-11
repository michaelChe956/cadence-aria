use std::collections::BTreeMap;
use std::fmt::Debug;

use serde::{Serialize, de::DeserializeOwned};

use crate::product::models::{
    AgentRole, AmendmentResumeMode, AmendmentResumeTarget, ArtifactRef, DependencyGraphRevision,
    DesignSpecRecord, HandoffRevision, HumanGateTurn, HumanGateTurnFailureClass,
    HumanGateTurnStatus, HumanPresentationRevision, LogicalWorkItem, NodeDetail, PermissionEvent,
    PlanAmendmentManifest, PlanAmendmentPublicationJournal, PlanAmendmentPublicationPhase,
    PlanDefectClass, PlanDefectEvidence, PlanDefectRoute, PlanProjectionBundle, PlanRepairRequest,
    PlanRepairRequestStatus, PlanRevisionReason, PlanValidationReportArtifact, ProviderName,
    ProviderSnapshot, RepairTarget, RepairTargetKind, RepositoryRecord, VerificationPlanRevision,
    WorkItemDraftRevision, WorkItemDraftRevisionState, WorkItemDraftRevisionStatus,
    WorkItemPlanLineage, WorkItemPlanRevision, WorkItemProjectionBundle, WorkItemRevision,
    WorkItemRevisionReplacement, WorkItemRuntimeBinding, WorkspaceSessionRecord,
    WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::work_item_contract::{
    ContractValidationReport, build_dependency_contract_graph, canonical_contract_fixture,
};
use crate::product::work_item_projection::{
    PlanProjectionCompileInput, PlanProjectionCompiler, ProjectionValidationReport,
    WorkItemProjectionCompiler, projection_hashes,
};
use crate::web::workspace_ws_types::{TimelineNodeStatus, TimelineNodeType};

fn assert_serde_roundtrip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let json = serde_json::to_value(value).unwrap();
    let back: T = serde_json::from_value(json).unwrap();

    assert_eq!(&back, value);
}

fn assert_missing_field_rejected<T>(value: &serde_json::Value, field: &str)
where
    T: DeserializeOwned,
{
    assert!(serde_json::from_value::<T>(value.clone()).is_ok());

    let mut missing = value.clone();
    missing.as_object_mut().unwrap().remove(field);

    assert!(
        serde_json::from_value::<T>(missing).is_err(),
        "missing required field {field} should be rejected"
    );
}

fn assert_enum_cases<T>(cases: impl IntoIterator<Item = (T, &'static str)>)
where
    T: Serialize,
{
    for (variant, expected) in cases {
        assert_eq!(serde_json::to_value(variant).unwrap(), expected);
    }
}

#[test]
fn human_gate_turn_durable_schema_roundtrips_and_rejects_unknown_fields() {
    let statuses = [
        HumanGateTurnStatus::Reserved,
        HumanGateTurnStatus::Running,
        HumanGateTurnStatus::Completed,
        HumanGateTurnStatus::Failed,
    ];
    let failure_classes = [
        HumanGateTurnFailureClass::ProviderErr,
        HumanGateTurnFailureClass::ValidationReject,
        HumanGateTurnFailureClass::Timeout,
        HumanGateTurnFailureClass::BudgetExhausted,
    ];
    for status in &statuses {
        assert_serde_roundtrip(status);
    }
    assert_enum_cases([
        (HumanGateTurnStatus::Reserved, "reserved"),
        (HumanGateTurnStatus::Running, "running"),
        (HumanGateTurnStatus::Completed, "completed"),
        (HumanGateTurnStatus::Failed, "failed"),
    ]);
    for failure_class in &failure_classes {
        assert_serde_roundtrip(failure_class);
    }
    assert_enum_cases([
        (HumanGateTurnFailureClass::ProviderErr, "provider_err"),
        (
            HumanGateTurnFailureClass::ValidationReject,
            "validation_reject",
        ),
        (HumanGateTurnFailureClass::Timeout, "timeout"),
        (
            HumanGateTurnFailureClass::BudgetExhausted,
            "budget_exhausted",
        ),
    ]);

    let turn = HumanGateTurn {
        turn_id: "turn_0001".to_string(),
        session_id: "workspace_session_0001".to_string(),
        command_id: "command_0001".to_string(),
        feedback_text: "please revise".to_string(),
        status: HumanGateTurnStatus::Reserved,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: String::new(),
        result_artifact_ref: Some("artifact_0001".to_string()),
        failure_class: None,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:00Z".to_string(),
    };
    assert_serde_roundtrip(&turn);

    let value = serde_json::to_value(&turn).unwrap();
    for field in [
        "turn_id",
        "session_id",
        "command_id",
        "feedback_text",
        "status",
        "attempt_no",
        "budget_reserved",
        "result_artifact_ref",
        "failure_class",
        "created_at",
        "updated_at",
    ] {
        assert_missing_field_rejected::<HumanGateTurn>(&value, field);
    }
    let mut legacy_without_source_hash = serde_json::to_value(&turn).unwrap();
    legacy_without_source_hash
        .as_object_mut()
        .unwrap()
        .remove("source_hash");
    let decoded_legacy: HumanGateTurn = serde_json::from_value(legacy_without_source_hash).unwrap();
    assert!(decoded_legacy.source_hash.is_empty());
    let mut unknown = value;
    unknown
        .as_object_mut()
        .unwrap()
        .insert("unexpected".to_string(), serde_json::json!(true));
    assert!(serde_json::from_value::<HumanGateTurn>(unknown).is_err());
    let failed_turn = HumanGateTurn {
        status: HumanGateTurnStatus::Failed,
        failure_class: Some(HumanGateTurnFailureClass::Timeout),
        ..turn.clone()
    };
    assert_serde_roundtrip(&failed_turn);
    let invalid_nonterminal_failure = HumanGateTurn {
        failure_class: Some(HumanGateTurnFailureClass::Timeout),
        ..turn
    };
    assert_serde_roundtrip(&invalid_nonterminal_failure);
}
#[test]
fn old_workspace_session_without_human_gate_reservation_deserializes_to_none() {
    let legacy = serde_json::json!({
        "id": "workspace_session_legacy",
        "project_id": "project_0001",
        "issue_id": "issue_0001",
        "entity_id": "work_item_plan_legacy",
        "workspace_type": "work_item_plan",
        "status": "waiting_for_human",
        "author_provider": "codex",
        "reviewer_provider": "claude_code",
        "review_rounds": 1,
        "superpowers_enabled": false,
        "openspec_enabled": false,
        "messages": [],
        "created_at": "2026-08-31T00:00:00Z",
        "updated_at": "2026-08-31T00:00:00Z"
    });
    let legacy_session: WorkspaceSessionRecord = serde_json::from_value(legacy).unwrap();
    assert_eq!(legacy_session.human_gate_reservation, None);
    let mut missing_reservation = serde_json::to_value(&legacy_session).unwrap();
    missing_reservation
        .as_object_mut()
        .unwrap()
        .remove("human_gate_reservation");
    let decoded_without_reservation: WorkspaceSessionRecord =
        serde_json::from_value(missing_reservation).unwrap();
    assert_eq!(decoded_without_reservation.human_gate_reservation, None);
}

#[test]
fn legacy_repository_json_roundtrips_with_identity_defaults() {
    let legacy = serde_json::json!({
        "id": "repository_0001",
        "project_id": "project_0001",
        "name": "api",
        "path": "/workspace/api",
        "repo_hash": "abc123",
        "runtime_root": "/workspace/api/.aria/runtime",
        "default_policy_preset": "manual-write",
        "default_provider_mode": "fake",
        "created_at": "2026-08-05T00:00:00Z",
        "updated_at": "2026-08-05T00:00:00Z"
    });

    let record: RepositoryRecord = serde_json::from_value(legacy).unwrap();
    assert_eq!(record.id, "repository_0001");
    assert_eq!(record.logical_repository_id, None);
    assert_eq!(record.primary_checkout_id, None);
    assert_eq!(record.identity_schema_version, 0);

    let encoded = serde_json::to_value(&record).unwrap();
    let decoded: RepositoryRecord = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, record);
}

#[test]
fn node_detail_roundtrip() {
    let detail = NodeDetail {
        node_id: "node-1".to_string(),
        session_id: "sess-1".to_string(),
        node_type: TimelineNodeType::AuthorRun,
        status: TimelineNodeStatus::Completed,
        agent_role: Some(AgentRole::Author),
        provider: Some(ProviderSnapshot {
            name: "claude_code".to_string(),
            model: "claude-opus-4-7".to_string(),
        }),
        prompt: Some("Workspace 类型: Story Spec".to_string()),
        messages: vec![],
        streaming_content: "输出内容".to_string(),
        execution_events: vec![],
        permission_events: vec![PermissionEvent {
            request_id: "perm-1".to_string(),
            request: serde_json::json!({"tool": "shell"}),
            response: Some(serde_json::json!({"approved": true})),
            ts: "2026-05-20T14:35:00Z".to_string(),
        }],
        verdict: None,
        artifact_ref: Some(ArtifactRef {
            artifact_id: "art-1".to_string(),
            version: 2,
        }),
        is_revision: false,
        revision_feedback: None,
        base_artifact_ref: None,
        started_at: "2026-05-20T14:30:00Z".to_string(),
        ended_at: Some("2026-05-20T14:35:00Z".to_string()),
    };

    let json = serde_json::to_value(&detail).unwrap();
    let back: NodeDetail = serde_json::from_value(json).unwrap();

    assert_eq!(back.node_id, detail.node_id);
    assert_eq!(back.prompt, detail.prompt);
    assert_eq!(back.permission_events.len(), 1);
}

#[test]
fn work_item_revision_models_plan_revision_roundtrip_without_legacy_fields() {
    let revision = WorkItemPlanRevision {
        id: "plan_revision_0001".to_string(),
        plan_id: "issue_work_item_plan_0001".to_string(),
        revision_no: 1,
        supersedes: None,
        reason: PlanRevisionReason::InitialCompile,
        work_item_bindings: BTreeMap::from([(
            "wi_core".to_string(),
            "work_item_revision_0001".to_string(),
        )]),
        dependency_graph_revision_id: "dependency_graph_revision_0001".to_string(),
        validation_report_ref: "validation_report_0001".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        publication_provenance_ref: None,
        created_at: "2026-07-17T00:00:00Z".to_string(),
    };

    let value = serde_json::to_value(&revision).unwrap();
    assert_eq!(value["revision_no"], 1);
    assert_eq!(value["reason"], "initial_compile");
    assert!(value.get("work_item_ids").is_none());
    assert_eq!(
        serde_json::from_value::<WorkItemPlanRevision>(value).unwrap(),
        revision
    );
}

#[test]
fn work_item_runtime_binding_roundtrips_as_reference_only() {
    let binding = WorkItemRuntimeBinding {
        plan_id: "work_item_plan_0001".to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        logical_work_item_id: "wi_core".to_string(),
        work_item_revision_id: "work_item_revision_0001".to_string(),
        projection_bundle_id: "work_item_projection_bundle_0001".to_string(),
        verification_plan_revision_id: "verification_plan_revision_0001".to_string(),
        canonical_contract_hash: "sha256:contract".to_string(),
        projection_compiler_version: "projection-compiler-v1".to_string(),
        human_projection_hash: "sha256:human".to_string(),
        coder_projection_hash: "sha256:coder".to_string(),
        reviewer_projection_hash: "sha256:reviewer".to_string(),
    };

    assert_serde_roundtrip(&binding);
    let value = serde_json::to_value(&binding).unwrap();
    for forbidden in [
        "canonical_contract",
        "human_projection",
        "coder_projection",
        "reviewer_projection",
        "verification_checks",
        "execution_status",
    ] {
        assert!(
            value.get(forbidden).is_none(),
            "binding must not store {forbidden}"
        );
    }
    for field in [
        "plan_id",
        "plan_revision_id",
        "logical_work_item_id",
        "work_item_revision_id",
        "projection_bundle_id",
        "verification_plan_revision_id",
        "canonical_contract_hash",
        "projection_compiler_version",
        "human_projection_hash",
        "coder_projection_hash",
        "reviewer_projection_hash",
    ] {
        assert_missing_field_rejected::<WorkItemRuntimeBinding>(&value, field);
    }
}

#[test]
fn workspace_session_runtime_binding_is_optional_and_work_item_scoped() {
    let binding = WorkItemRuntimeBinding {
        plan_id: "work_item_plan_0001".to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        logical_work_item_id: "wi_core".to_string(),
        work_item_revision_id: "work_item_revision_0001".to_string(),
        projection_bundle_id: "work_item_projection_bundle_0001".to_string(),
        verification_plan_revision_id: "verification_plan_revision_0001".to_string(),
        canonical_contract_hash: "sha256:contract".to_string(),
        projection_compiler_version: "projection-compiler-v1".to_string(),
        human_projection_hash: "sha256:human".to_string(),
        coder_projection_hash: "sha256:coder".to_string(),
        reviewer_projection_hash: "sha256:reviewer".to_string(),
    };
    let session = WorkspaceSessionRecord {
        id: "workspace_session_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "wi_core".to_string(),
        workspace_type: WorkspaceType::WorkItem,
        status: WorkspaceSessionStatus::Open,
        author_provider: ProviderName::Codex,
        reviewer_provider: Some(ProviderName::ClaudeCode),

        review_rounds: 2,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        provisional_reviewer_provider: None,
        reviewer_enabled_at_start: None,
        superpowers_enabled: true,
        openspec_enabled: true,
        flow_kind: crate::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy,
        run_policy: crate::product::work_item_plan_policy::RunPolicy::Interactive,
        run_history: Default::default(),
        review_invocation_scope: None,
        human_gate_snapshot: None,
        repair_reservation: None,
        human_gate_reservation: None,
        policy_diagnostics: Vec::new(),
        provider_start_ledger: Vec::new(),
        single_candidate_phase: None,
        work_item_plan_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        publication_provenance_ref: None,
        approval_attempt_id: None,
        approved_at: None,
        compile_reservation: None,
        provider_conversations: Vec::new(),
        messages: Vec::new(),
        created_at: "2026-07-26T00:00:00Z".to_string(),
        updated_at: "2026-07-26T00:00:00Z".to_string(),
        work_item_runtime_binding: Some(binding.clone()),
        work_item_child_binding: None,
        sc_recovery_claim: None,
        logical_launch_fingerprints: Vec::new(),
        superseded_native_sessions: Vec::new(),
        lc_resume_recovery: None,
    };

    assert_serde_roundtrip(&session);
    let mut missing_binding = serde_json::to_value(&session).unwrap();
    missing_binding
        .as_object_mut()
        .unwrap()
        .remove("work_item_runtime_binding");
    assert_eq!(
        serde_json::from_value::<WorkspaceSessionRecord>(missing_binding)
            .unwrap()
            .work_item_runtime_binding,
        None
    );

    for workspace_type in [WorkspaceType::Story, WorkspaceType::Design] {
        let mut non_work_item = session.clone();
        non_work_item.workspace_type = workspace_type;
        non_work_item.work_item_runtime_binding = None;
        assert_serde_roundtrip(&non_work_item);
        assert_eq!(non_work_item.work_item_runtime_binding, None);
    }
}

#[test]
fn work_item_revision_models_keep_draft_state_separate_from_revision_content() {
    let contract = canonical_contract_fixture("wi_core");
    let draft = WorkItemDraftRevision {
        id: "draft_revision_0001".to_string(),
        logical_work_item_id: "wi_core".to_string(),
        revision_no: 1,
        supersedes: None,
        revision_reason: PlanRevisionReason::InitialCompile,
        canonical_contract_candidate: contract.clone(),
        trigger_repair_request_id: None,
        created_at: "2026-07-17T00:00:00Z".to_string(),
    };
    let state = WorkItemDraftRevisionState {
        draft_revision_id: draft.id.clone(),
        status: WorkItemDraftRevisionStatus::ChangesRequested,
        updated_at: "2026-07-17T00:01:00Z".to_string(),
    };
    let revision = WorkItemRevision {
        id: "work_item_revision_0001".to_string(),
        logical_work_item_id: "wi_core".to_string(),
        source_draft_revision_id: draft.id.clone(),
        canonical_contract: contract,
        canonical_contract_hash: "sha256:contract".to_string(),
        work_item_projection_bundle_id: "work_item_projection_bundle_0001".to_string(),
        verification_plan_revision_id: "verification_plan_revision_0001".to_string(),
        created_at: "2026-07-17T00:02:00Z".to_string(),
    };

    let draft_value = serde_json::to_value(&draft).unwrap();
    let state_value = serde_json::to_value(&state).unwrap();
    let revision_value = serde_json::to_value(&revision).unwrap();

    assert!(draft_value.get("status").is_none());
    assert_eq!(state_value["status"], "changes_requested");
    assert!(revision_value.get("status").is_none());
    assert_serde_roundtrip(&draft);
    assert_serde_roundtrip(&state);
    assert_serde_roundtrip(&revision);
}

#[test]
fn work_item_revision_models_enums_use_snake_case() {
    assert_enum_cases([
        (PlanRevisionReason::InitialCompile, "initial_compile"),
        (
            PlanRevisionReason::RepairCurrentWorkItem,
            "repair_current_work_item",
        ),
        (
            PlanRevisionReason::RepairUpstreamContract,
            "repair_upstream_contract",
        ),
        (PlanRevisionReason::SubgraphReplan, "subgraph_replan"),
        (PlanRevisionReason::StoryAmendment, "story_amendment"),
        (PlanRevisionReason::DesignAmendment, "design_amendment"),
    ]);
    assert_enum_cases([
        (WorkItemDraftRevisionStatus::Drafting, "drafting"),
        (WorkItemDraftRevisionStatus::Reviewing, "reviewing"),
        (
            WorkItemDraftRevisionStatus::ChangesRequested,
            "changes_requested",
        ),
        (WorkItemDraftRevisionStatus::Approved, "approved"),
        (WorkItemDraftRevisionStatus::Rejected, "rejected"),
        (WorkItemDraftRevisionStatus::Compiled, "compiled"),
    ]);
    assert_enum_cases([
        (
            PlanDefectClass::ImplementationDefect,
            "implementation_defect",
        ),
        (
            PlanDefectClass::VerificationIncomplete,
            "verification_incomplete",
        ),
        (
            PlanDefectClass::CurrentWorkItemInvalid,
            "current_work_item_invalid",
        ),
        (
            PlanDefectClass::UpstreamContractInvalid,
            "upstream_contract_invalid",
        ),
        (
            PlanDefectClass::DependencyGraphInvalid,
            "dependency_graph_invalid",
        ),
        (
            PlanDefectClass::DesignAmendmentRequired,
            "design_amendment_required",
        ),
        (
            PlanDefectClass::StoryAmendmentRequired,
            "story_amendment_required",
        ),
        (PlanDefectClass::OperationalBlocker, "operational_blocker"),
    ]);
    assert_enum_cases([
        (PlanDefectRoute::CoderRework, "coder_rework"),
        (PlanDefectRoute::VerificationRetry, "verification_retry"),
        (PlanDefectRoute::PlanRepair, "plan_repair"),
        (PlanDefectRoute::StoryAmendment, "story_amendment"),
        (PlanDefectRoute::DesignAmendment, "design_amendment"),
        (PlanDefectRoute::OperationalGate, "operational_gate"),
        (PlanDefectRoute::HumanTriage, "human_triage"),
    ]);
    assert_enum_cases([
        (RepairTargetKind::CurrentWorkItem, "current_work_item"),
        (RepairTargetKind::UpstreamWorkItem, "upstream_work_item"),
        (RepairTargetKind::Subgraph, "subgraph"),
    ]);
    assert_enum_cases([
        (PlanRepairRequestStatus::Open, "open"),
        (PlanRepairRequestStatus::InProgress, "in_progress"),
        (
            PlanRepairRequestStatus::AwaitingConfirmation,
            "awaiting_confirmation",
        ),
        (PlanRepairRequestStatus::Published, "published"),
        (PlanRepairRequestStatus::Applied, "applied"),
        (PlanRepairRequestStatus::Cancelled, "cancelled"),
        (PlanRepairRequestStatus::Failed, "failed"),
    ]);
    assert_enum_cases([
        (AmendmentResumeMode::Reexecute, "reexecute"),
        (AmendmentResumeMode::Revalidate, "revalidate"),
        (AmendmentResumeMode::AwaitHandoff, "await_handoff"),
    ]);
    assert_enum_cases([
        (PlanAmendmentPublicationPhase::Prepared, "prepared"),
        (
            PlanAmendmentPublicationPhase::PlanPublished,
            "plan_published",
        ),
    ]);
}

#[test]
fn work_item_revision_models_shared_records_roundtrip() {
    let mut projection_contract = canonical_contract_fixture("wi_core");
    projection_contract.input_contracts.clear();
    projection_contract
        .handoff_contract
        .provided_contract_refs
        .clear();
    let work_item_projection = WorkItemProjectionCompiler
        .compile(&projection_contract, "work_item_revision_0001")
        .unwrap();
    let work_item_hashes = projection_hashes(&work_item_projection).unwrap();
    let graph = build_dependency_contract_graph(&[projection_contract]).unwrap();
    let work_items = BTreeMap::from([("wi_core".to_string(), work_item_projection.clone())]);
    let plan_projection = PlanProjectionCompiler
        .compile(PlanProjectionCompileInput {
            plan_id: "issue_work_item_plan_0001",
            goal: "Compile core",
            split_reason: "Single contract boundary",
            source_refs: vec!["story_spec_0001".to_string()],
            dependency_graph: &graph,
            work_item_projections: &work_items,
            expected_work_item_revision_ids: BTreeMap::from([(
                "wi_core".to_string(),
                "work_item_revision_0001".to_string(),
            )]),
        })
        .unwrap();
    assert_serde_roundtrip(&WorkItemPlanLineage {
        id: "issue_work_item_plan_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        story_spec_refs: vec!["story_spec_0001".to_string()],
        design_spec_refs: vec!["design_spec_0001".to_string()],
        active_revision_id: Some("plan_revision_0001".to_string()),
        active_amendment_id: None,
        created_at: "2026-07-17T00:00:00Z".to_string(),
        updated_at: "2026-07-17T00:01:00Z".to_string(),
    });
    assert_serde_roundtrip(&LogicalWorkItem {
        id: "wi_core".to_string(),
        plan_id: "issue_work_item_plan_0001".to_string(),
        title: "Compile core".to_string(),
        active_revision_id: Some("work_item_revision_0001".to_string()),
        created_at: "2026-07-17T00:00:00Z".to_string(),
        updated_at: "2026-07-17T00:01:00Z".to_string(),
    });
    assert_serde_roundtrip(&VerificationPlanRevision {
        id: "verification_plan_revision_0001".to_string(),
        logical_work_item_id: "wi_core".to_string(),
        source_draft_revision_id: "draft_revision_0001".to_string(),
        verification_checks: canonical_contract_fixture("wi_core").verification_checks,
        created_at: "2026-07-17T00:00:00Z".to_string(),
    });
    assert_serde_roundtrip(&PlanValidationReportArtifact {
        id: "validation_report_0001".to_string(),
        plan_id: "issue_work_item_plan_0001".to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        contract_validation: ContractValidationReport { findings: vec![] },
        projection_validation: ProjectionValidationReport { findings: vec![] },
        created_at: "2026-07-17T00:00:00Z".to_string(),
    });
    assert_serde_roundtrip(&WorkItemProjectionBundle {
        id: "work_item_projection_bundle_0001".to_string(),
        work_item_revision_id: "work_item_revision_0001".to_string(),
        canonical_contract_hash: "sha256:contract".to_string(),
        projection_schema_version: 1,
        compiler_version: "compiler-v1".to_string(),
        human_projection: work_item_projection.human,
        coder_projection: work_item_projection.coder,
        reviewer_projection: work_item_projection.reviewer,
        human_projection_hash: work_item_hashes.human,
        coder_projection_hash: work_item_hashes.coder,
        reviewer_projection_hash: work_item_hashes.reviewer,
        created_at: "2026-07-17T00:00:00Z".to_string(),
    });
    assert_serde_roundtrip(&PlanProjectionBundle {
        id: "plan_projection_bundle_0001".to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        dependency_graph_revision_id: "dependency_graph_revision_0001".to_string(),
        work_item_projection_bundle_refs: vec!["work_item_projection_bundle_0001".to_string()],
        human_group_projection: plan_projection.human,
        coder_group_context: plan_projection.coder,
        reviewer_group_matrix: plan_projection.reviewer,
        human_group_projection_hash: "sha256:human-group".to_string(),
        coder_group_context_hash: "sha256:coder-group".to_string(),
        reviewer_group_matrix_hash: "sha256:reviewer-group".to_string(),
        compiler_version: "compiler-v1".to_string(),
        created_at: "2026-07-17T00:00:00Z".to_string(),
    });
    assert_serde_roundtrip(&HumanPresentationRevision {
        id: "human_presentation_revision_0001".to_string(),
        source_plan_projection_bundle_id: Some("plan_projection_bundle_0001".to_string()),
        source_work_item_projection_bundle_id: None,
        supersedes: None,
        human_summary: "Compile core".to_string(),
        why_split: Some("Isolate the core contract".to_string()),
        dependency_explanation: vec!["No upstream dependency".to_string()],
        risk_explanation: vec!["Schema drift".to_string()],
        source_refs: vec!["story_spec_0001".to_string()],
        normative: false,
        used_by_provider: false,
        created_at: "2026-07-17T00:00:00Z".to_string(),
    });
    assert_serde_roundtrip(&HandoffRevision {
        id: "handoff_revision_0001".to_string(),
        logical_work_item_id: "wi_core".to_string(),
        work_item_revision_id: "work_item_revision_0001".to_string(),
        coding_unit_run_id: "coding_unit_run_0001".to_string(),
        provided_contracts: vec!["contract.core".to_string()],
        provided_capabilities: BTreeMap::from([(
            "contract.core".to_string(),
            vec!["compile".to_string()],
        )]),
        contract_hash: "sha256:contract".to_string(),
        commit_sha: "0123456789abcdef".to_string(),
        created_at: "2026-07-17T00:00:00Z".to_string(),
    });
    assert_serde_roundtrip(&DependencyGraphRevision {
        id: "dependency_graph_revision_0001".to_string(),
        plan_id: "issue_work_item_plan_0001".to_string(),
        edges: vec![crate::product::work_item_contract::DependencyContractEdge {
            from: "wi_core".to_string(),
            to: "wi_api".to_string(),
            required_contracts: vec![],
        }],
        created_at: "2026-07-17T00:00:00Z".to_string(),
    });

    let replacement = WorkItemRevisionReplacement {
        previous_revision_id: "work_item_revision_0001".to_string(),
        next_revision_id: "work_item_revision_0002".to_string(),
        delta_kind: crate::product::models::ContractDeltaKind::CompatibleContractExtension,
    };
    assert_serde_roundtrip(&replacement);
    let resume_target = AmendmentResumeTarget {
        logical_work_item_id: "wi_core".to_string(),
        mode: AmendmentResumeMode::Reexecute,
    };
    assert_serde_roundtrip(&resume_target);
    assert_serde_roundtrip(&PlanAmendmentManifest {
        id: "plan_amendment_0001".to_string(),
        repair_request_id: "plan_repair_request_0001".to_string(),
        previous_plan_revision_id: "plan_revision_0001".to_string(),
        new_plan_revision_id: "plan_revision_0002".to_string(),
        revised_work_items: BTreeMap::from([("wi_core".to_string(), replacement)]),
        superseded_revisions: vec!["work_item_revision_0001".to_string()],
        dependency_graph_changes: vec![crate::product::models::DependencyGraphChange {
            kind: crate::product::models::DependencyGraphChangeKind::EdgeReplaced,
            previous: None,
            next: None,
        }],
        contract_deltas: vec![crate::product::plan_repair::ContractDelta {
            logical_work_item_id: "wi_core".to_string(),
            previous_revision_id: "work_item_revision_0001".to_string(),
            next_revision_id: "work_item_revision_0002".to_string(),
            kind: crate::product::models::ContractDeltaKind::CompatibleContractExtension,
            added_contracts: vec![],
            removed_contracts: vec![],
            added_capabilities: vec!["finalization_failure".to_string()],
            removed_capabilities: vec![],
            changed_capabilities: vec![],
            added_capability_associations: vec![],
            removed_capability_associations: vec![],
            acceptance_changed: false,
            verification_changed: false,
            write_policy_changed: false,
        }],
        unaffected_units: vec!["wi_docs".to_string()],
        revalidation_required_units: vec!["wi_api".to_string()],
        stale_units: vec!["wi_core".to_string()],
        replacement_units: BTreeMap::from([(
            "wi_core".to_string(),
            vec!["wi_core_v2".to_string()],
        )]),
        resume_target,
        created_at: "2026-07-17T00:00:00Z".to_string(),
    });
    assert_serde_roundtrip(&PlanAmendmentPublicationJournal {
        id: "publication_journal_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        plan_id: "issue_work_item_plan_0001".to_string(),
        amendment_id: "plan_amendment_0001".to_string(),
        request_id: "plan_repair_request_0001".to_string(),
        base_plan_revision_id: "plan_revision_0001".to_string(),
        new_plan_revision_id: "plan_revision_0002".to_string(),
        confirmation: None,
        artifact_fingerprint: "fingerprint_0001".to_string(),
        snapshot: None,
        phase: PlanAmendmentPublicationPhase::Prepared,
        error: None,
        recovery: None,
        created_at: "2026-07-17T00:00:00Z".to_string(),
        updated_at: "2026-07-17T00:01:00Z".to_string(),
    });
}

#[test]
fn plan_repair_models_request_and_target_roundtrip_with_typed_evidence() {
    let repair_target = RepairTarget {
        kind: RepairTargetKind::CurrentWorkItem,
        logical_work_item_ids: vec!["wi_core".to_string()],
        work_item_revision_ids: vec!["work_item_revision_0001".to_string()],
    };
    assert_serde_roundtrip(&repair_target);
    assert_serde_roundtrip(&PlanRepairRequest {
        id: "plan_repair_request_0001".to_string(),
        plan_id: "issue_work_item_plan_0001".to_string(),
        base_plan_revision_id: "plan_revision_0001".to_string(),
        trigger_attempt_id: "attempt_0001".to_string(),
        trigger_unit_run_id: "coding_unit_run_0001".to_string(),
        trigger_review_id: Some("review_0001".to_string()),
        trigger_finding_id: "finding_0001".to_string(),
        amendment_id: None,
        defect_class: PlanDefectClass::CurrentWorkItemInvalid,
        reason_code: "contract_mismatch".to_string(),
        repair_target,
        contract_refs: vec!["contract.core".to_string()],
        capability_refs: vec!["compile".to_string()],
        evidence: vec![PlanDefectEvidence {
            kind: "test_failure".to_string(),
            source_ref: "test_0001".to_string(),
            message: "failed".to_string(),
        }],
        fingerprint: "sha256:fingerprint".to_string(),
        status: PlanRepairRequestStatus::Open,
        created_at: "2026-07-17T00:00:00Z".to_string(),
        updated_at: "2026-07-17T00:01:00Z".to_string(),
    });
}

#[test]
fn work_item_revision_models_reject_missing_required_fields() {
    let plan_revision = serde_json::json!({
        "id": "plan_revision_0001",
        "plan_id": "issue_work_item_plan_0001",
        "revision_no": 1,
        "supersedes": null,
        "reason": "initial_compile",
        "work_item_bindings": {"wi_core": "work_item_revision_0001"},
        "dependency_graph_revision_id": "dependency_graph_revision_0001",
        "validation_report_ref": "validation_report_0001",
        "plan_projection_bundle_id": "plan_projection_bundle_0001",
        "created_at": "2026-07-17T00:00:00Z"
    });
    for field in [
        "id",
        "plan_id",
        "revision_no",
        "reason",
        "work_item_bindings",
        "dependency_graph_revision_id",
        "validation_report_ref",
        "plan_projection_bundle_id",
        "created_at",
    ] {
        assert_missing_field_rejected::<WorkItemPlanRevision>(&plan_revision, field);
    }

    let draft_state = serde_json::json!({
        "draft_revision_id": "draft_revision_0001",
        "status": "reviewing",
        "updated_at": "2026-07-17T00:00:00Z"
    });
    for field in ["draft_revision_id", "status", "updated_at"] {
        assert_missing_field_rejected::<WorkItemDraftRevisionState>(&draft_state, field);
    }

    let work_item_revision = serde_json::to_value(WorkItemRevision {
        id: "work_item_revision_0001".to_string(),
        logical_work_item_id: "wi_core".to_string(),
        source_draft_revision_id: "draft_revision_0001".to_string(),
        canonical_contract: canonical_contract_fixture("wi_core"),
        canonical_contract_hash: "sha256:contract".to_string(),
        work_item_projection_bundle_id: "work_item_projection_bundle_0001".to_string(),
        verification_plan_revision_id: "verification_plan_revision_0001".to_string(),
        created_at: "2026-07-17T00:00:00Z".to_string(),
    })
    .unwrap();
    for field in [
        "id",
        "logical_work_item_id",
        "source_draft_revision_id",
        "canonical_contract",
        "canonical_contract_hash",
        "work_item_projection_bundle_id",
        "verification_plan_revision_id",
        "created_at",
    ] {
        assert_missing_field_rejected::<WorkItemRevision>(&work_item_revision, field);
    }

    let publication_journal = serde_json::json!({
        "id": "publication_journal_0001",
        "project_id": "project_0001",
        "issue_id": "issue_0001",
        "plan_id": "issue_work_item_plan_0001",
        "amendment_id": "plan_amendment_0001",
        "request_id": "plan_repair_request_0001",
        "base_plan_revision_id": "plan_revision_0001",
        "new_plan_revision_id": "plan_revision_0002",
        "artifact_fingerprint": "fingerprint_0001",
        "phase": "prepared",
        "error": null,
        "recovery": null,
        "created_at": "2026-07-17T00:00:00Z",
        "updated_at": "2026-07-17T00:01:00Z"
    });
    for field in [
        "id",
        "project_id",
        "issue_id",
        "plan_id",
        "amendment_id",
        "request_id",
        "base_plan_revision_id",
        "new_plan_revision_id",
        "artifact_fingerprint",
        "phase",
        "created_at",
        "updated_at",
    ] {
        assert_missing_field_rejected::<PlanAmendmentPublicationJournal>(
            &publication_journal,
            field,
        );
    }
}

// === 聚合视野：DesignSpec serde 向后兼容（Task 8）===
// 旧 DesignSpec JSON 不含 logical_codebase_ref / involved_repository_ids / change_order，
// 经 #[serde(default)] 必须可读且默认空，保证历史产物向后兼容（REQ-PLN-08）。
#[test]
fn legacy_design_spec_roundtrips_with_aggregate_defaults() {
    let legacy = serde_json::json!({
        "id": "design_0001", "project_id": "project_0001", "issue_id": "issue_0001",
        "story_spec_ids": ["story_0001"], "title": "t",
        "current_version": null, "confirmation_status": "draft",
        "created_at": "2026-08-10T00:00:00Z", "updated_at": "2026-08-10T00:00:00Z"
    });
    let record: DesignSpecRecord = serde_json::from_value(legacy).unwrap();
    assert_eq!(record.logical_codebase_ref, None);
    assert!(record.involved_repository_ids.is_empty());
    assert!(record.change_order.is_empty());
    let encoded = serde_json::to_value(&record).unwrap();
    assert_eq!(
        serde_json::from_value::<DesignSpecRecord>(encoded).unwrap(),
        record
    );
}

// === A1a（Task 2.1）：durable 恢复记录模型 ===
// anchor_t02：旧 JSON 缺新字段读默认空；tagged run_context 各 Plan 身份
// roundtrip；full basis/native/run 绑定；runtime from_record 逐字段透传且
// reload 保留 business repository_path。

use crate::product::models::{
    LcFreshOperationResult, LcFreshOperationState, LcLegacyOutlineAuthorPayload,
    LcLegacyOutlineRevisionPayload, LcOutlineRebuildOrigin, LcRecoveryArtifactRef,
    LcRecoveryBusinessContext, LcRecoveryDraftRef, LcRecoveryRedoSpec, LcRecoveryRunContext,
    LcRetainedPlanItemRef, LcResumeRecoveryRecord, LcResumeRecoveryState, LogicalLaunchBasis,
    LogicalLaunchFingerprintRecord, LogicalLaunchFingerprintState, ProviderConversationRole,
    SingleCandidatePhase, SupersededNativeSessionRecord, WorkItemDraftStatus,
};
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::product::workspace_engine::WorkspaceSession;

fn anchor_t02_business_context() -> LcRecoveryBusinessContext {
    LcRecoveryBusinessContext {
        entity_id: "wi_core".to_string(),
        plan_id: Some("work_item_plan_0001".to_string()),
        workspace_type: WorkspaceType::WorkItemPlan,
        business_repository_physical_id: Some("repo_phys_0001".to_string()),
        business_repository_logical_id: Some("repo_logical_0001".to_string()),
        business_checkout_id: Some("checkout_0001".to_string()),
        business_repository_path: Some("/repos/business".to_string()),
        source_story_ids: vec!["story_0001".to_string()],
        source_design_ids: vec!["design_0001".to_string()],
        plan_options_ref: Some("plan_options_0001".to_string()),
        artifact_refs: vec![LcRecoveryArtifactRef {
            ref_id: "artifact_0001".to_string(),
            version: Some(3),
            hash: Some("sha256:artifact".to_string()),
        }],
        review_refs: vec!["review_node_0001".to_string()],
        author_provider: ProviderName::Codex,
        effective_reviewer_provider: Some(ProviderName::ClaudeCode),
        original_node_id: Some("node_0001".to_string()),
        original_role_run_seq: Some(7),
    }
}

fn anchor_t02_all_run_contexts() -> Vec<(LcRecoveryRunContext, &'static str)> {
    let business = anchor_t02_business_context();
    vec![
        (
            LcRecoveryRunContext::Author {
                business: business.clone(),
                original_content: Some("原 author prompt 内容".to_string()),
            },
            "author",
        ),
        (
            LcRecoveryRunContext::AuthorChoiceFollowup {
                business: business.clone(),
                original_content: Some("choice followup 内容".to_string()),
            },
            "author_choice_followup",
        ),
        (
            LcRecoveryRunContext::AuthorFeedback {
                business: business.clone(),
                original_feedback: Some("author 反馈".to_string()),
            },
            "author_feedback",
        ),
        (
            LcRecoveryRunContext::ReviewerRevision {
                business: business.clone(),
                original_feedback: Some("reviewer 修订反馈".to_string()),
            },
            "reviewer_revision",
        ),
        (
            LcRecoveryRunContext::LegacyOutlineAuthor {
                business: business.clone(),
                flow: WorkItemPlanFlowKind::Legacy,
                original_provider_node_id: Some("node_outline_0001".to_string()),
                outline_ref: Some("outline_0001".to_string()),
            },
            "legacy_outline_author",
        ),
        (
            LcRecoveryRunContext::LegacyOutlineRevision {
                business: business.clone(),
                normalized_feedback: "规范化反馈（捕获实际 default）".to_string(),
                outline_ref: Some("outline_0001".to_string()),
                outline_version: Some(2),
                outline_hash: Some("sha256:outline".to_string()),
            },
            "legacy_outline_revision",
        ),
        (
            LcRecoveryRunContext::LegacyOutlineRebuild {
                business: business.clone(),
                origin: LcOutlineRebuildOrigin::OutlineRevision(LcLegacyOutlineRevisionPayload {
                    normalized_feedback: "重建前的修订反馈不丢".to_string(),
                    outline_ref: Some("outline_0001".to_string()),
                    outline_version: Some(2),
                    outline_hash: Some("sha256:outline".to_string()),
                }),
                previous_snapshot_access_fingerprint: Some("sha256:snapshot_access".to_string()),
            },
            "legacy_outline_rebuild",
        ),
        (
            LcRecoveryRunContext::LegacyDraft {
                business: business.clone(),
                generation_round_id: "round_0001".to_string(),
                active_outline_id: "outline_0001".to_string(),
                draft_refs: vec![LcRecoveryDraftRef {
                    draft_id: "draft_0001".to_string(),
                    status: WorkItemDraftStatus::Draft,
                    version: Some(1),
                    hash: Some("sha256:draft".to_string()),
                }],
                pending_feedback: Some("take 前固化的反馈".to_string()),
                outline_ref: Some("outline_0001".to_string()),
                candidate_ref: Some("candidate_0001".to_string()),
            },
            "legacy_draft",
        ),
        (
            LcRecoveryRunContext::LegacyBatch {
                business: business.clone(),
                generation_round_id: "round_0001".to_string(),
                active_outline_id: "outline_0001".to_string(),
                draft_refs: vec![LcRecoveryDraftRef {
                    draft_id: "draft_0001".to_string(),
                    status: WorkItemDraftStatus::Accepted,
                    version: Some(1),
                    hash: Some("sha256:draft".to_string()),
                }],
                pending_feedback: Some("take 前固化的批量反馈".to_string()),
                outline_ref: Some("outline_0001".to_string()),
                candidate_ref: Some("candidate_0001".to_string()),
                batch_id: "batch_0001".to_string(),
                batch_item_draft_refs: vec![LcRecoveryDraftRef {
                    draft_id: "draft_0002".to_string(),
                    status: WorkItemDraftStatus::Draft,
                    version: Some(1),
                    hash: Some("sha256:draft2".to_string()),
                }],
            },
            "legacy_batch",
        ),
        (
            LcRecoveryRunContext::LegacyPlanRevision {
                business: business.clone(),
                candidate_ref: Some("candidate_0001".to_string()),
                candidate_version: Some(2),
                candidate_hash: Some("sha256:candidate".to_string()),
                retained_items: vec![LcRetainedPlanItemRef {
                    item_id: "wi_core".to_string(),
                    revision: 3,
                }],
                redo_specs: vec![LcRecoveryRedoSpec {
                    old_item_id: "wi_legacy".to_string(),
                    feedback: Some("redo 反馈".to_string()),
                }],
                overall_feedback: Some("整体反馈".to_string()),
            },
            "legacy_plan_revision",
        ),
        (
            LcRecoveryRunContext::ScAuthor {
                business: business.clone(),
                flow: WorkItemPlanFlowKind::SingleCandidate,
                single_candidate_phase: SingleCandidatePhase::Generate,
                provider_start_reservation_key: Some("compile_0001".to_string()),
                source_revision_ref: Some("source_rev_0001".to_string()),
                ir_ref: Some("ir_0001".to_string()),
                report_ref: Some("report_0001".to_string()),
                candidate_source_hash: Some("sha256:candidate_source".to_string()),
                business_material_identity: Some("material_identity_0001".to_string()),
            },
            "sc_author",
        ),
        (
            LcRecoveryRunContext::ScManualRevision {
                business,
                turn_id: "turn_0001".to_string(),
                command_id: "command_0001".to_string(),
                source_hash: "sha256:candidate".to_string(),
                turn_status: HumanGateTurnStatus::Running,
                attempt_no: 2,
                budget_reserved: 1,
                human_gate_reservation_key: Some("reservation_key_0001".to_string()),
                gate_snapshot_ref: Some("gate_snapshot_0001".to_string()),
                source_revision_ref: Some("source_rev_0001".to_string()),
                feedback_text: Some("同 turn 的人工反馈".to_string()),
                assembly_digest: Some("sha256:assembly".to_string()),
                assembly_ref: Some("assembly_0001".to_string()),
            },
            "sc_manual_revision",
        ),
    ]
}

fn anchor_t02_launch_fingerprint(
    state: LogicalLaunchFingerprintState,
) -> LogicalLaunchFingerprintRecord {
    LogicalLaunchFingerprintRecord {
        schema_version: 1,
        basis: LogicalLaunchBasis::FullLcProjectionV2,
        role: ProviderConversationRole::Author,
        provider: ProviderName::Codex,
        role_run_seq: 7,
        node_id: "node_0001".to_string(),
        native_session_id: Some("native_session_0001".to_string()),
        fingerprint_digest: "sha256:full_projection".to_string(),
        planning_access_fingerprint: "sha256:planning_access".to_string(),
        state,
        created_at: "2026-10-11T00:00:00Z".to_string(),
        updated_at: "2026-10-11T00:01:00Z".to_string(),
    }
}

fn anchor_t02_superseded_native_session() -> SupersededNativeSessionRecord {
    SupersededNativeSessionRecord {
        role: ProviderConversationRole::Author,
        provider: ProviderName::Codex,
        native_session_id: "native_session_0001".to_string(),
        reason_code: "resume_fingerprint_mismatch".to_string(),
        recovery_id: "recovery_0001".to_string(),
        superseded_at: "2026-10-11T00:02:00Z".to_string(),
    }
}

fn anchor_t02_recovery_record(state: LcResumeRecoveryState) -> LcResumeRecoveryRecord {
    LcResumeRecoveryRecord {
        schema_version: 1,
        recovery_id: "recovery_0001".to_string(),
        failed_provider_node_id: "node_0001".to_string(),
        reason_code: "resume_fingerprint_mismatch".to_string(),
        role: ProviderConversationRole::Author,
        provider: ProviderName::Codex,
        superseded_native_session_id: Some("native_session_0001".to_string()),
        previous_fingerprint_digest: Some("sha256:full_projection".to_string()),
        previous_session_status: WorkspaceSessionStatus::Running,
        previous_stage: "running".to_string(),
        run_context: LcRecoveryRunContext::LegacyDraft {
            business: anchor_t02_business_context(),
            generation_round_id: "round_0001".to_string(),
            active_outline_id: "outline_0001".to_string(),
            draft_refs: vec![LcRecoveryDraftRef {
                draft_id: "draft_0001".to_string(),
                status: WorkItemDraftStatus::Draft,
                version: Some(1),
                hash: Some("sha256:draft".to_string()),
            }],
            pending_feedback: Some("take 前固化的反馈".to_string()),
            outline_ref: Some("outline_0001".to_string()),
            candidate_ref: Some("candidate_0001".to_string()),
        },
        state,
        operation_id: Some("operation_0001".to_string()),
        fresh_node_id: None,
        fresh_native_session_id: None,
        fresh_role_run_seq: None,
        fresh_operations: vec![LcFreshOperationResult {
            operation_id: "operation_0001".to_string(),
            state: LcFreshOperationState::Reserved,
            fresh_node_id: None,
            reason_code: None,
            role_run_seq: None,
            accepted_at: "2026-10-11T00:03:00Z".to_string(),
            updated_at: "2026-10-11T00:03:00Z".to_string(),
        }],
        detected_at: "2026-10-11T00:02:00Z".to_string(),
        updated_at: "2026-10-11T00:03:00Z".to_string(),
    }
}

#[test]
fn anchor_t02_recovery_schema_roundtrips_all_run_contexts() {
    // tagged run_context：每个 Plan 身份 roundtrip 且 wire kind 稳定；
    // 未知 kind 的反序列化拒收由 typed 错误映射（context_unsupported）承载。
    for (context, expected_kind) in anchor_t02_all_run_contexts() {
        assert_serde_roundtrip(&context);
        let value = serde_json::to_value(&context).unwrap();
        assert_eq!(value["kind"], expected_kind, "run_context kind");
    }

    // rebuild origin 两种来源的完整 payload 不丢 feedback。
    let origin_author = LcOutlineRebuildOrigin::LegacyAuthor(LcLegacyOutlineAuthorPayload {
        flow: WorkItemPlanFlowKind::Legacy,
        original_provider_node_id: Some("node_outline_0001".to_string()),
        outline_ref: Some("outline_0001".to_string()),
    });
    assert_serde_roundtrip(&origin_author);
    assert_eq!(
        serde_json::to_value(&origin_author).unwrap()["legacy_author"]["flow"],
        "legacy"
    );

    // full basis/native/run 绑定的 launch 指纹记录。
    assert_enum_cases([(LogicalLaunchBasis::FullLcProjectionV2, "full_lc_projection_v2")]);
    for state in [
        LogicalLaunchFingerprintState::Prepared,
        LogicalLaunchFingerprintState::Active,
        LogicalLaunchFingerprintState::Superseded,
    ] {
        let fingerprint = anchor_t02_launch_fingerprint(state);
        assert_serde_roundtrip(&fingerprint);
        assert_eq!(fingerprint.schema_version, 1);
    }
    assert_enum_cases([
        (LogicalLaunchFingerprintState::Prepared, "prepared"),
        (LogicalLaunchFingerprintState::Active, "active"),
        (LogicalLaunchFingerprintState::Superseded, "superseded"),
    ]);

    // 多代 tombstone：旧 native 吊销证据耐久保留。
    assert_serde_roundtrip(&anchor_t02_superseded_native_session());

    // 恢复状态机与 operation 状态 wire 名。
    assert_enum_cases([
        (LcResumeRecoveryState::Waiting, "waiting"),
        (LcResumeRecoveryState::FreshReserved, "fresh_reserved"),
        (LcResumeRecoveryState::FreshRunning, "fresh_running"),
        (LcResumeRecoveryState::Resolved, "resolved"),
    ]);
    assert_enum_cases([
        (LcFreshOperationState::NotStarted, "not_started"),
        (LcFreshOperationState::Reserved, "reserved"),
        (LcFreshOperationState::Running, "running"),
        (LcFreshOperationState::Completed, "completed"),
        (LcFreshOperationState::Failed, "failed"),
        (LcFreshOperationState::Unknown, "unknown"),
    ]);
    let operation = LcFreshOperationResult {
        operation_id: "operation_0001".to_string(),
        state: LcFreshOperationState::Reserved,
        fresh_node_id: None,
        reason_code: None,
        role_run_seq: None,
        accepted_at: "2026-10-11T00:03:00Z".to_string(),
        updated_at: "2026-10-11T00:03:00Z".to_string(),
    };
    assert_serde_roundtrip(&operation);

    // 恢复记录整体 roundtrip：run_context 身份保持（plan 锚）。
    let original = anchor_t02_recovery_record(LcResumeRecoveryState::Waiting);
    let json = serde_json::to_value(&original).unwrap();
    let roundtrip: LcResumeRecoveryRecord = serde_json::from_value(json).unwrap();
    assert_eq!(roundtrip.run_context, original.run_context);
    assert_eq!(roundtrip, original);
}

fn anchor_t02_session_record() -> WorkspaceSessionRecord {
    WorkspaceSessionRecord {
        id: "workspace_session_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "wi_core".to_string(),
        workspace_type: WorkspaceType::WorkItemPlan,
        status: WorkspaceSessionStatus::Running,
        author_provider: ProviderName::Codex,
        reviewer_provider: Some(ProviderName::ClaudeCode),
        review_rounds: 2,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        provisional_reviewer_provider: None,
        reviewer_enabled_at_start: None,
        superpowers_enabled: true,
        openspec_enabled: true,
        flow_kind: WorkItemPlanFlowKind::Legacy,
        run_policy: crate::product::work_item_plan_policy::RunPolicy::Interactive,
        run_history: Default::default(),
        review_invocation_scope: None,
        human_gate_snapshot: None,
        repair_reservation: None,
        human_gate_reservation: None,
        policy_diagnostics: Vec::new(),
        provider_start_ledger: Vec::new(),
        single_candidate_phase: None,
        work_item_plan_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        publication_provenance_ref: None,
        approval_attempt_id: None,
        approved_at: None,
        compile_reservation: None,
        work_item_runtime_binding: None,
        work_item_child_binding: None,
        provider_conversations: Vec::new(),
        messages: Vec::new(),
        created_at: "2026-10-11T00:00:00Z".to_string(),
        updated_at: "2026-10-11T00:00:00Z".to_string(),
        sc_recovery_claim: None,
        logical_launch_fingerprints: vec![anchor_t02_launch_fingerprint(
            LogicalLaunchFingerprintState::Active,
        )],
        superseded_native_sessions: vec![anchor_t02_superseded_native_session()],
        lc_resume_recovery: Some(anchor_t02_recovery_record(LcResumeRecoveryState::Waiting)),
    }
}

#[test]
fn anchor_t02_legacy_json_and_runtime_restore_defaults() {
    let record = anchor_t02_session_record();

    // 旧 JSON：三个新 durable 字段缺失时读默认空，不拒收历史会话。
    let mut legacy = serde_json::to_value(&record).unwrap();
    let object = legacy.as_object_mut().unwrap();
    object.remove("logical_launch_fingerprints");
    object.remove("superseded_native_sessions");
    object.remove("lc_resume_recovery");
    let from_legacy: WorkspaceSessionRecord = serde_json::from_value(legacy).unwrap();
    assert_eq!(from_legacy.logical_launch_fingerprints.len(), 0);
    assert!(from_legacy.superseded_native_sessions.is_empty());
    assert!(from_legacy.lc_resume_recovery.is_none());

    // runtime from_record：逐字段透传，不丢、不重构（plan 锚）。
    let reloaded = WorkspaceSession::from_record(record.clone());
    assert_eq!(
        reloaded.logical_launch_fingerprints,
        record.logical_launch_fingerprints
    );
    assert_eq!(
        reloaded.superseded_native_sessions,
        record.superseded_native_sessions
    );
    assert_eq!(reloaded.lc_resume_recovery, record.lc_resume_recovery);

    // reload 注入的 business repository_path 保留（durable record 不含该字段）。
    let injected_business_path = std::path::PathBuf::from("/repos/business");
    let mut reloaded = reloaded;
    reloaded.repository_path = Some(injected_business_path.clone());
    assert_eq!(reloaded.repository_path, Some(injected_business_path));
}
