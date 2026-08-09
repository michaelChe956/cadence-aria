use std::collections::BTreeMap;
use std::fmt::Debug;

use serde::{Serialize, de::DeserializeOwned};

use crate::product::models::{
    AgentRole, AmendmentResumeMode, AmendmentResumeTarget, ArtifactRef, DependencyGraphRevision,
    DesignSpecRecord, HandoffRevision, HumanPresentationRevision, LogicalWorkItem, NodeDetail,
    PermissionEvent, PlanAmendmentManifest, PlanAmendmentPublicationJournal,
    PlanAmendmentPublicationPhase, PlanDefectClass, PlanDefectEvidence, PlanDefectRoute,
    PlanProjectionBundle, PlanRepairRequest, PlanRepairRequestStatus, PlanRevisionReason,
    PlanValidationReportArtifact, ProviderName, ProviderSnapshot, RepairTarget, RepairTargetKind,
    RepositoryRecord, VerificationPlanRevision, WorkItemDraftRevision, WorkItemDraftRevisionState,
    WorkItemDraftRevisionStatus, WorkItemPlanLineage, WorkItemPlanRevision,
    WorkItemProjectionBundle, WorkItemRevision, WorkItemRevisionReplacement,
    WorkItemRuntimeBinding, WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType,
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
        reviewer_provider: ProviderName::ClaudeCode,
        review_rounds: 2,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        superpowers_enabled: true,
        openspec_enabled: true,
        provider_conversations: Vec::new(),
        messages: Vec::new(),
        created_at: "2026-07-26T00:00:00Z".to_string(),
        updated_at: "2026-07-26T00:00:00Z".to_string(),
        work_item_runtime_binding: Some(binding.clone()),
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
