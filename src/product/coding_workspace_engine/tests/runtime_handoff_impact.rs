use super::*;
use crate::product::coding_models::{CodingUnitRun, CodingUnitRunStatus};
use crate::product::models::{
    AmendmentResumeMode, AmendmentResumeTarget, ContractDeltaKind, DependencyGraphRevision,
    HandoffRevision, LogicalWorkItem, PlanAmendmentManifest, PlanRevisionReason,
    WorkItemPlanLineage, WorkItemPlanRevision, WorkItemRevision, WorkItemRevisionReplacement,
};
use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractCompatibilityPolicy, HandoffContract,
    PromisedOutputContract, RequiredDependencyContract, RequiredInputContract,
    WorkItemContractIdentity, WorkItemGoal, WorkItemWritePolicy, canonical_contract_hash,
};
use crate::product::work_item_projection::{WorkItemProjectionCompiler, projection_hashes};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use std::collections::BTreeMap;

fn handoff(
    id: &str,
    contracts: &[&str],
    capabilities: &[(&str, &[&str])],
    contract_hash: &str,
) -> HandoffRevision {
    HandoffRevision {
        id: id.to_string(),
        logical_work_item_id: "wi_core".to_string(),
        work_item_revision_id: "work_item_revision_wi01_v2".to_string(),
        coding_unit_run_id: format!("coding_unit_run_{id}"),
        provided_contracts: contracts.iter().map(|value| (*value).to_string()).collect(),
        provided_capabilities: capabilities
            .iter()
            .map(|(contract, values)| {
                (
                    (*contract).to_string(),
                    values.iter().map(|value| (*value).to_string()).collect(),
                )
            })
            .collect::<BTreeMap<_, _>>(),
        contract_hash: contract_hash.to_string(),
        commit_sha: format!("commit_{id}"),
        tests: vec![format!("test_{id}")],
        artifacts: vec![format!("artifact_{id}")],
        created_at: "2026-07-20T00:00:00Z".to_string(),
    }
}

struct RuntimeHandoffFixture {
    _root: tempfile::TempDir,
    store: CodingAttemptStore,
    engine: CodingWorkspaceEngine,
    attempt: CodingExecutionAttempt,
    next_handoff: HandoffRevision,
}

#[derive(Clone, Copy)]
enum RuntimeContractChange {
    Unchanged,
    CompatibleExtension,
    BreakingChange,
}

fn runtime_handoff_fixture(
    change: RuntimeContractChange,
    waiting_status: CodingUnitRunStatus,
) -> RuntimeHandoffFixture {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "wi_core".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = WorkItemPlanLineage {
        id: "work_item_plan_0001".to_string(),
        project_id: attempt.project_id.clone(),
        issue_id: attempt.issue_id.clone(),
        story_spec_refs: Vec::new(),
        design_spec_refs: Vec::new(),
        active_revision_id: None,
        active_amendment_id: None,
        created_at: "2026-07-20T00:00:00Z".to_string(),
        updated_at: "2026-07-20T00:00:00Z".to_string(),
    };
    revision_store
        .put_plan_lineage(&lineage)
        .expect("plan lineage");

    let complete_capabilities = vec![
        "failure_message",
        "finalization_failure",
        "registration_ready",
        "workflow_explicit_completion",
    ];
    let (old_capabilities, next_capabilities) = match change {
        RuntimeContractChange::Unchanged => {
            (vec!["registration_ready"], vec!["registration_ready"])
        }
        RuntimeContractChange::CompatibleExtension => {
            (vec!["registration_ready"], complete_capabilities.clone())
        }
        RuntimeContractChange::BreakingChange => {
            (complete_capabilities, vec!["registration_ready"])
        }
    };
    let core_v1 = runtime_contract("wi_core", Vec::new(), old_capabilities.clone());
    let core_v2 = runtime_contract("wi_core", Vec::new(), next_capabilities.clone());
    let registration = runtime_contract(
        "wi_registration",
        vec![RequiredInputContract {
            contract_id: "registration_contract".to_string(),
            provider_logical_work_item_id: "wi_core".to_string(),
            required_capabilities: vec![
                "failure_message".to_string(),
                "finalization_failure".to_string(),
                "workflow_explicit_completion".to_string(),
            ],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }],
        vec!["registration_consumed"],
    );
    let unrelated = runtime_contract("wi_unrelated", Vec::new(), vec!["unrelated_ready"]);
    let followup = runtime_contract(
        "wi_followup",
        vec![RequiredInputContract {
            contract_id: "registration_contract".to_string(),
            provider_logical_work_item_id: "wi_registration".to_string(),
            required_capabilities: vec!["registration_consumed".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }],
        vec!["followup_ready"],
    );
    for (logical_id, revision_id, contract) in [
        ("wi_core", "work_item_revision_wi01_v1", &core_v1),
        ("wi_core", "work_item_revision_wi01_v2", &core_v2),
        (
            "wi_registration",
            "work_item_revision_wi02_v1",
            &registration,
        ),
        ("wi_unrelated", "work_item_revision_wi03_v1", &unrelated),
        ("wi_followup", "work_item_revision_wi04_v1", &followup),
    ] {
        ensure_runtime_logical_item(&revision_store, &lineage, logical_id);
        put_runtime_revision(&revision_store, &lineage, logical_id, revision_id, contract);
    }

    let graph = DependencyGraphRevision {
        id: "dependency_graph_revision_0002".to_string(),
        plan_id: lineage.id.clone(),
        edges: vec![
            crate::product::work_item_contract::DependencyContractEdge {
                from: "wi_core".to_string(),
                to: "wi_registration".to_string(),
                required_contracts: registration
                    .input_contracts
                    .iter()
                    .map(|input| RequiredDependencyContract {
                        contract_id: input.contract_id.clone(),
                        required_capabilities: input.required_capabilities.clone(),
                        compatibility_policy: input.compatibility_policy.clone(),
                    })
                    .collect(),
            },
            crate::product::work_item_contract::DependencyContractEdge {
                from: "wi_registration".to_string(),
                to: "wi_followup".to_string(),
                required_contracts: followup
                    .input_contracts
                    .iter()
                    .map(|input| RequiredDependencyContract {
                        contract_id: input.contract_id.clone(),
                        required_capabilities: input.required_capabilities.clone(),
                        compatibility_policy: input.compatibility_policy.clone(),
                    })
                    .collect(),
            },
        ],
        created_at: "2026-07-20T00:00:02Z".to_string(),
    };
    revision_store
        .put_dependency_graph_revision(&lineage, &graph)
        .expect("dependency graph");
    let plan_revision = WorkItemPlanRevision {
        id: "plan_revision_0002".to_string(),
        plan_id: lineage.id.clone(),
        revision_no: 2,
        supersedes: Some("plan_revision_0001".to_string()),
        reason: PlanRevisionReason::RepairUpstreamContract,
        work_item_bindings: BTreeMap::from([
            (
                "wi_core".to_string(),
                "work_item_revision_wi01_v2".to_string(),
            ),
            (
                "wi_registration".to_string(),
                "work_item_revision_wi02_v1".to_string(),
            ),
            (
                "wi_unrelated".to_string(),
                "work_item_revision_wi03_v1".to_string(),
            ),
            (
                "wi_followup".to_string(),
                "work_item_revision_wi04_v1".to_string(),
            ),
        ]),
        dependency_graph_revision_id: graph.id,
        validation_report_ref: "validation_report_0002".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0002".to_string(),
        created_at: "2026-07-20T00:00:02Z".to_string(),
    };
    revision_store
        .put_plan_revision(&lineage, &plan_revision)
        .expect("plan revision");
    revision_store
        .set_active_plan_revision(&lineage, &plan_revision.id)
        .expect("active plan revision");
    let manifest = PlanAmendmentManifest {
        id: "plan_amendment_0001".to_string(),
        repair_request_id: "plan_repair_request_0001".to_string(),
        previous_plan_revision_id: "plan_revision_0001".to_string(),
        new_plan_revision_id: plan_revision.id.clone(),
        revised_work_items: BTreeMap::from([(
            "wi_core".to_string(),
            WorkItemRevisionReplacement {
                previous_revision_id: "work_item_revision_wi01_v1".to_string(),
                next_revision_id: "work_item_revision_wi01_v2".to_string(),
                delta_kind: match change {
                    RuntimeContractChange::Unchanged => ContractDeltaKind::InformativeOnly,
                    RuntimeContractChange::CompatibleExtension => {
                        ContractDeltaKind::CompatibleContractExtension
                    }
                    RuntimeContractChange::BreakingChange => {
                        ContractDeltaKind::BreakingContractChange
                    }
                },
            },
        )]),
        superseded_revisions: vec!["work_item_revision_wi01_v1".to_string()],
        dependency_graph_changes: Vec::new(),
        contract_deltas: Vec::new(),
        unaffected_units: vec!["wi_unrelated".to_string()],
        revalidation_required_units: matches!(
            change,
            RuntimeContractChange::Unchanged | RuntimeContractChange::CompatibleExtension
        )
        .then(|| vec!["wi_registration".to_string()])
        .unwrap_or_default(),
        stale_units: matches!(change, RuntimeContractChange::BreakingChange)
            .then(|| vec!["wi_registration".to_string()])
            .unwrap_or_default(),
        replacement_units: BTreeMap::new(),
        resume_target: AmendmentResumeTarget {
            logical_work_item_id: "wi_registration".to_string(),
            mode: AmendmentResumeMode::AwaitHandoff,
        },
        created_at: "2026-07-20T00:00:03Z".to_string(),
    };
    revision_store
        .put_amendment_manifest(&lineage, &manifest)
        .expect("manifest");
    store
        .save_plan_binding(
            &attempt,
            &CodingAttemptPlanBinding {
                attempt_id: attempt.id.clone(),
                plan_id: lineage.id.clone(),
                bound_plan_revision_id: plan_revision.id,
                applied_amendment_ids: vec![manifest.id.clone()],
                updated_at: "2026-07-20T00:00:03Z".to_string(),
            },
        )
        .expect("plan binding");

    for (order, logical_id, revision_id, dependencies, status) in [
        (
            0,
            "wi_core",
            "work_item_revision_wi01_v2",
            Vec::new(),
            CodingExecutionUnitStatus::Completed,
        ),
        (
            1,
            "wi_registration",
            "work_item_revision_wi02_v1",
            vec!["wi_core".to_string()],
            match waiting_status {
                CodingUnitRunStatus::AwaitingAmendment => {
                    CodingExecutionUnitStatus::AwaitingAmendment
                }
                CodingUnitRunStatus::NeedsRevalidation => {
                    CodingExecutionUnitStatus::NeedsRevalidation
                }
                CodingUnitRunStatus::Stale => CodingExecutionUnitStatus::Stale,
                _ => panic!("unsupported fixture waiting status"),
            },
        ),
        (
            2,
            "wi_unrelated",
            "work_item_revision_wi03_v1",
            Vec::new(),
            CodingExecutionUnitStatus::Pending,
        ),
        (
            3,
            "wi_followup",
            "work_item_revision_wi04_v1",
            vec!["wi_registration".to_string()],
            CodingExecutionUnitStatus::Pending,
        ),
    ] {
        store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: attempt.id.clone(),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                plan_id: lineage.id.clone(),
                logical_work_item_id: logical_id.to_string(),
                work_item_revision_id: revision_id.to_string(),
                dependency_logical_work_item_ids: dependencies,
                order_index: order,
                status,
            })
            .expect("coding unit");
    }
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units");
    let registration_unit = units
        .iter()
        .find(|unit| unit.logical_work_item_id == "wi_registration")
        .expect("registration unit");
    let registration_revision = revision_store
        .get_work_item_revision(&lineage, "wi_registration", "work_item_revision_wi02_v1")
        .expect("registration revision");
    let registration_bundle = revision_store
        .get_work_item_projection_bundle(
            &lineage,
            &registration_revision.work_item_projection_bundle_id,
        )
        .expect("registration bundle");
    let providers = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("provider snapshot");
    let amendment_run_id = format!(
        "coding_unit_run_{}_plan_amendment_0001",
        registration_unit.id
    );
    for (id, execution_no, status, handoffs) in [
        (
            "coding_unit_run_registration_0001".to_string(),
            1,
            CodingUnitRunStatus::Completed,
            vec!["handoff_revision_0001".to_string()],
        ),
        (amendment_run_id, 2, waiting_status, Vec::new()),
    ] {
        store
            .create_coding_unit_run(
                &attempt,
                &CodingUnitRun {
                    id,
                    unit_id: registration_unit.id.clone(),
                    execution_no,
                    work_item_revision_id: registration_revision.id.clone(),
                    resolved_handoff_revision_ids: handoffs,
                    canonical_contract_hash: registration_revision.canonical_contract_hash.clone(),
                    projection_bundle_id: registration_bundle.id.clone(),
                    projection_compiler_version: registration_bundle.compiler_version.clone(),
                    coder_provider_renderer_version:
                        crate::product::work_item_projection::renderer_for(&providers.coder)
                            .renderer_version()
                            .to_string(),
                    reviewer_provider_renderer_version:
                        crate::product::work_item_projection::renderer_for(&providers.code_reviewer)
                            .renderer_version()
                            .to_string(),
                    internal_reviewer_provider_renderer_version: None,
                    coder_projection_hash: registration_bundle.coder_projection_hash.clone(),
                    reviewer_projection_hash: registration_bundle.reviewer_projection_hash.clone(),
                    coder_execution_context_hash: None,
                    reviewer_execution_context_hash: None,
                    internal_reviewer_execution_context_hash: None,
                    status,
                    unit_rework_count: 0,
                    verification_retry_count: 0,
                    operational_retry_count: 0,
                    plan_repair_count: 1,
                    start_commit: Some("commit_core_v1".to_string()),
                    completion_commit: (execution_no == 1)
                        .then(|| "commit_registration_v1".to_string()),
                    created_at: format!("2026-07-20T00:00:0{execution_no}Z"),
                    updated_at: format!("2026-07-20T00:00:0{execution_no}Z"),
                },
            )
            .expect("registration run");
    }

    let previous_handoff = handoff(
        "handoff_revision_0001",
        &["registration_contract"],
        &[("registration_contract", old_capabilities.as_slice())],
        "contract_hash_v1",
    );
    let next_handoff = handoff(
        "handoff_revision_0002",
        &["registration_contract"],
        &[("registration_contract", next_capabilities.as_slice())],
        match change {
            RuntimeContractChange::Unchanged => "contract_hash_v1",
            RuntimeContractChange::CompatibleExtension => "contract_hash_v2",
            RuntimeContractChange::BreakingChange => "contract_hash_v3",
        },
    );
    revision_store
        .put_handoff_revision(&lineage, &previous_handoff)
        .expect("previous handoff");
    revision_store
        .put_handoff_revision(&lineage, &next_handoff)
        .expect("next handoff");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    RuntimeHandoffFixture {
        _root: root,
        store,
        engine,
        attempt,
        next_handoff,
    }
}

fn runtime_contract(
    logical_id: &str,
    input_contracts: Vec<RequiredInputContract>,
    capabilities: Vec<&str>,
) -> CanonicalWorkItemContract {
    CanonicalWorkItemContract {
        schema_version: 1,
        identity: WorkItemContractIdentity {
            logical_work_item_id: logical_id.to_string(),
            title: logical_id.to_string(),
            kind: "implementation".to_string(),
        },
        goal: WorkItemGoal {
            summary: logical_id.to_string(),
        },
        non_goals: Vec::new(),
        input_contracts,
        output_contracts: vec![PromisedOutputContract {
            contract_id: "registration_contract".to_string(),
            capabilities: capabilities.into_iter().map(str::to_string).collect(),
        }],
        tasks: Vec::new(),
        write_policy: WorkItemWritePolicy {
            exclusive_scopes: Vec::new(),
            forbidden_scopes: Vec::new(),
        },
        acceptance_criteria: Vec::new(),
        verification_checks: Vec::new(),
        handoff_contract: HandoffContract {
            required_fields: Vec::new(),
            provided_contract_refs: vec!["registration_contract".to_string()],
            reviewer_check_refs: Vec::new(),
        },
        blocker_rules: Vec::new(),
        design_traceability: Vec::new(),
    }
}

fn ensure_runtime_logical_item(
    store: &WorkItemRevisionStore,
    lineage: &WorkItemPlanLineage,
    logical_id: &str,
) {
    if store.get_logical_work_item(lineage, logical_id).is_ok() {
        return;
    }
    store
        .put_logical_work_item(
            lineage,
            &LogicalWorkItem {
                id: logical_id.to_string(),
                plan_id: lineage.id.clone(),
                title: logical_id.to_string(),
                active_revision_id: None,
                created_at: "2026-07-20T00:00:00Z".to_string(),
                updated_at: "2026-07-20T00:00:00Z".to_string(),
            },
        )
        .expect("logical work item");
}

fn put_runtime_revision(
    store: &WorkItemRevisionStore,
    lineage: &WorkItemPlanLineage,
    logical_id: &str,
    revision_id: &str,
    contract: &CanonicalWorkItemContract,
) {
    let projections = WorkItemProjectionCompiler
        .compile(contract, revision_id)
        .expect("projections");
    let hashes = projection_hashes(&projections).expect("projection hashes");
    let bundle_id = format!("projection_bundle_{revision_id}");
    store
        .put_work_item_revision(
            lineage,
            &WorkItemRevision {
                id: revision_id.to_string(),
                logical_work_item_id: logical_id.to_string(),
                source_draft_revision_id: format!("draft_{revision_id}"),
                canonical_contract: contract.clone(),
                canonical_contract_hash: canonical_contract_hash(contract).expect("contract hash"),
                work_item_projection_bundle_id: bundle_id.clone(),
                verification_plan_revision_id: format!("verification_{revision_id}"),
                created_at: "2026-07-20T00:00:01Z".to_string(),
            },
        )
        .expect("work item revision");
    store
        .put_work_item_projection_bundle(
            lineage,
            &WorkItemProjectionBundle {
                id: bundle_id,
                work_item_revision_id: revision_id.to_string(),
                canonical_contract_hash: canonical_contract_hash(contract).expect("contract hash"),
                projection_schema_version: 1,
                compiler_version: "work-item-projection-compiler-v1".to_string(),
                human_projection: projections.human,
                coder_projection: projections.coder,
                reviewer_projection: projections.reviewer,
                human_projection_hash: hashes.human,
                coder_projection_hash: hashes.coder,
                reviewer_projection_hash: hashes.reviewer,
                created_at: "2026-07-20T00:00:01Z".to_string(),
            },
        )
        .expect("projection bundle");
}

#[tokio::test]
async fn coding_runtime_handoff_resumes_original_consumer_revision_with_new_handoff() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::CompatibleExtension,
        CodingUnitRunStatus::AwaitingAmendment,
    );

    let result = fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, &fixture.next_handoff)
        .await
        .unwrap();

    assert_eq!(result.resumed_units, vec!["wi_registration"]);
    let runs = fixture
        .store
        .list_unit_runs_by_logical_id(&fixture.attempt, "wi_registration")
        .unwrap();
    let resumed = runs.last().unwrap();
    assert_eq!(resumed.execution_no, 2);
    assert_eq!(resumed.work_item_revision_id, "work_item_revision_wi02_v1");
    assert_eq!(
        resumed.resolved_handoff_revision_ids,
        vec!["handoff_revision_0002"]
    );
    assert_eq!(resumed.status, CodingUnitRunStatus::Pending);
    assert!(
        fixture
            .store
            .list_unit_runs_by_logical_id(&fixture.attempt, "wi_unrelated")
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn coding_runtime_handoff_stops_conditional_propagation_when_contract_hash_is_unchanged() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::Unchanged,
        CodingUnitRunStatus::AwaitingAmendment,
    );

    let result = fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, &fixture.next_handoff)
        .await
        .unwrap();

    assert!(result.newly_stale_units.is_empty());
    assert!(result.conditional_units_released.is_empty());
    assert_eq!(result.propagation_stopped_at, Some("wi_core".to_string()));
}

#[tokio::test]
async fn coding_runtime_handoff_marks_explicit_compatible_consumer_for_revalidation() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::CompatibleExtension,
        CodingUnitRunStatus::NeedsRevalidation,
    );

    let result = fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, &fixture.next_handoff)
        .await
        .unwrap();

    assert_eq!(result.revalidation_units, vec!["wi_registration"]);
    let run = fixture
        .store
        .list_unit_runs_by_logical_id(&fixture.attempt, "wi_registration")
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(run.status, CodingUnitRunStatus::NeedsRevalidation);
    assert_eq!(
        run.resolved_handoff_revision_ids,
        vec!["handoff_revision_0002"]
    );
}

#[tokio::test]
async fn coding_runtime_handoff_marks_direct_consumer_stale_on_breaking_change() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::BreakingChange,
        CodingUnitRunStatus::AwaitingAmendment,
    );

    let result = fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, &fixture.next_handoff)
        .await
        .unwrap();

    assert_eq!(result.newly_stale_units, vec!["wi_registration"]);
    let runs = fixture
        .store
        .list_unit_runs_by_logical_id(&fixture.attempt, "wi_registration")
        .unwrap();
    assert_eq!(runs[0].status, CodingUnitRunStatus::Completed);
    assert_eq!(runs[1].status, CodingUnitRunStatus::Stale);
    assert_eq!(
        runs[1].resolved_handoff_revision_ids,
        vec!["handoff_revision_0002"]
    );
}

#[tokio::test]
async fn coding_runtime_handoff_replay_uses_fixed_idempotency_key_without_new_run() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::CompatibleExtension,
        CodingUnitRunStatus::AwaitingAmendment,
    );

    let first = fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, &fixture.next_handoff)
        .await
        .unwrap();
    let runs_after_first = fixture
        .store
        .list_unit_runs_by_logical_id(&fixture.attempt, "wi_registration")
        .unwrap();
    let second = fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, &fixture.next_handoff)
        .await
        .unwrap();

    assert_eq!(second, first);
    assert_eq!(
        fixture
            .store
            .list_unit_runs_by_logical_id(&fixture.attempt, "wi_registration")
            .unwrap(),
        runs_after_first
    );
}

#[tokio::test]
async fn coding_runtime_handoff_releases_conditional_consumer_only_after_changed_next_handoff() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::CompatibleExtension,
        CodingUnitRunStatus::AwaitingAmendment,
    );
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    let mut previous = handoff(
        "handoff_revision_registration_0001",
        &["registration_contract"],
        &[("registration_contract", &["registration_started"])],
        "registration_hash_v1",
    );
    previous.logical_work_item_id = "wi_registration".to_string();
    previous.work_item_revision_id = "work_item_revision_wi02_v1".to_string();
    let mut next = handoff(
        "handoff_revision_registration_0002",
        &["registration_contract"],
        &[(
            "registration_contract",
            &["registration_consumed", "registration_started"],
        )],
        "registration_hash_v2",
    );
    next.logical_work_item_id = "wi_registration".to_string();
    next.work_item_revision_id = "work_item_revision_wi02_v1".to_string();
    revision_store
        .put_handoff_revision(&lineage, &previous)
        .unwrap();
    revision_store
        .put_handoff_revision(&lineage, &next)
        .unwrap();

    let result = fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, &next)
        .await
        .unwrap();
    let runs_after_first = fixture
        .store
        .list_unit_runs_by_logical_id(&fixture.attempt, "wi_followup")
        .unwrap();
    fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, &next)
        .await
        .unwrap();

    assert_eq!(result.conditional_units_released, vec!["wi_followup"]);
    let followup = fixture
        .store
        .list_unit_runs_by_logical_id(&fixture.attempt, "wi_followup")
        .unwrap();
    assert_eq!(followup, runs_after_first);
    assert_eq!(followup.len(), 1);
    assert_eq!(followup[0].status, CodingUnitRunStatus::Pending);
    assert_eq!(
        followup[0].resolved_handoff_revision_ids,
        vec!["handoff_revision_registration_0002"]
    );
    assert!(
        fixture
            .store
            .list_unit_runs_by_logical_id(&fixture.attempt, "wi_unrelated")
            .unwrap()
            .is_empty()
    );
}

include!("runtime_handoff_history.rs");
