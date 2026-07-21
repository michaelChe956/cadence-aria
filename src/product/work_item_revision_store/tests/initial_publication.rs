use super::*;
use crate::product::models::{
    DependencyGraphRevision, PlanProjectionBundle, PlanValidationReportArtifact,
    VerificationPlanRevision, WorkItemPlanRevision, WorkItemProjectionBundle,
};
use crate::product::work_item_contract::{
    ContractFindingSeverity, ContractValidationFinding, ContractValidationReport,
    build_dependency_contract_graph, canonical_contract_hash,
};
use crate::product::work_item_projection::{
    PlanProjectionCompileInput, PlanProjectionCompiler, ProjectionValidationReport,
    WorkItemProjectionCompiler, projection_hashes,
};
use crate::product::work_item_revision_store::{
    InitialPlanPublicationArtifacts, InitialPlanPublicationCheckpoint, InitialPlanPublicationPhase,
    InitialWorkItemPublicationArtifacts,
};
use sha2::Digest;

#[test]
fn initial_plan_publication_store_allocates_ids_deterministically_without_live_writes() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let first_store = WorkItemRevisionStore::new(paths.clone());
    let second_store = WorkItemRevisionStore::new(paths);
    let logical_ids = vec![
        "logical_work_item_0001".to_string(),
        "logical_work_item_0002".to_string(),
    ];

    let first = first_store
        .allocate_initial_plan_publication_ids(
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "compile_0001",
            &logical_ids,
        )
        .unwrap();
    let replay = second_store
        .allocate_initial_plan_publication_ids(
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "compile_0001",
            &logical_ids,
        )
        .unwrap();

    assert_eq!(first, replay);
    assert_eq!(first.work_items.len(), 2);
    assert_ne!(
        first.work_items["logical_work_item_0001"].work_item_revision_id,
        first.work_items["logical_work_item_0002"].work_item_revision_id
    );
    assert!(matches!(
        first_store.get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID),
        Err(ProductStoreError::NotFound { .. })
    ));
}

#[test]
fn initial_plan_publication_resumes_each_store_write_failure_after_restart() {
    for checkpoint in [
        InitialPlanPublicationCheckpoint::LineageWritten,
        InitialPlanPublicationCheckpoint::FirstWorkItemArtifactsWritten,
        InitialPlanPublicationCheckpoint::PlanArtifactsWritten,
        InitialPlanPublicationCheckpoint::FirstWorkItemActivated,
        InitialPlanPublicationCheckpoint::PlanActivated,
    ] {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = WorkItemRevisionStore::new(paths.clone());
        let journal = initial_publication_journal(&store);
        let failpoint = store.register_initial_plan_publication_failpoint(
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "compile_0001",
            checkpoint,
        );

        let first_error = store
            .publish_or_resume_initial_plan_revision(&journal)
            .unwrap_err();
        assert!(
            first_error
                .to_string()
                .contains("initial_publication_failpoint")
        );
        drop(failpoint);

        let restarted = WorkItemRevisionStore::new(paths);
        let published = restarted
            .publish_or_resume_initial_plan_revision(&journal)
            .unwrap();

        assert_eq!(published.phase, InitialPlanPublicationPhase::PlanActivated);
        assert_eq!(published.error, None);
        assert_initial_publication_is_complete(&restarted, &journal);
    }
}

#[test]
fn initial_plan_publication_rejects_projection_failure_before_journal_or_live_facts() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths);
    let valid = initial_publication_journal(&store);
    let mut artifacts = valid.artifacts;
    artifacts
        .validation_report
        .projection_validation
        .findings
        .push(
            crate::product::work_item_projection::ProjectionValidationFinding {
                code: "projection_binding_mismatch".to_string(),
                projection: "coder".to_string(),
                contract_ref: None,
                message: "projection does not match the allocated revision".to_string(),
            },
        );

    let error = store
        .build_initial_plan_publication_journal(
            "compile_0001",
            "outline_0001",
            BTreeMap::from([(WORK_ITEM_ID.to_string(), "draft_revision_0001".to_string())]),
            "2026-07-17T00:00:20Z",
            artifacts,
        )
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("initial_projection_validation_failed")
    );
    assert!(matches!(
        store.get_initial_plan_publication_journal(PROJECT_ID, ISSUE_ID, PLAN_ID, "compile_0001",),
        Err(ProductStoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID),
        Err(ProductStoreError::NotFound { .. })
    ));
}

#[test]
fn initial_plan_publication_rejects_contract_failure_before_journal_or_live_facts() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths);
    let valid = initial_publication_journal(&store);
    let mut artifacts = valid.artifacts;
    artifacts
        .validation_report
        .contract_validation
        .findings
        .push(contract_validation_error());

    let error = store
        .build_initial_plan_publication_journal(
            "compile_0001",
            "outline_0001",
            BTreeMap::from([(WORK_ITEM_ID.to_string(), "draft_revision_0001".to_string())]),
            "2026-07-17T00:00:20Z",
            artifacts,
        )
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("initial_contract_validation_failed")
    );
    assert!(matches!(
        store.get_initial_plan_publication_journal(PROJECT_ID, ISSUE_ID, PLAN_ID, "compile_0001",),
        Err(ProductStoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID),
        Err(ProductStoreError::NotFound { .. })
    ));
}

#[test]
fn initial_plan_publication_boundary_rejects_invalid_validation_reports_without_writes() {
    for invalid_report in ["contract", "projection"] {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = WorkItemRevisionStore::new(paths);
        let mut journal = initial_publication_journal(&store);
        let expected_error = match invalid_report {
            "contract" => {
                journal
                    .artifacts
                    .validation_report
                    .contract_validation
                    .findings
                    .push(contract_validation_error());
                "initial_contract_validation_failed"
            }
            "projection" => {
                journal
                    .artifacts
                    .validation_report
                    .projection_validation
                    .findings
                    .push(
                        crate::product::work_item_projection::ProjectionValidationFinding {
                            code: "projection_binding_mismatch".to_string(),
                            projection: "coder".to_string(),
                            contract_ref: None,
                            message: "projection does not match publication".to_string(),
                        },
                    );
                "initial_projection_validation_failed"
            }
            _ => unreachable!(),
        };
        journal.artifact_fingerprint = hex::encode(sha2::Sha256::digest(
            serde_json::to_vec(&journal.artifacts).unwrap(),
        ));

        let error = store
            .publish_or_resume_initial_plan_revision(&journal)
            .unwrap_err();

        assert!(error.to_string().contains(expected_error));
        assert!(matches!(
            store.get_initial_plan_publication_journal(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "compile_0001",
            ),
            Err(ProductStoreError::NotFound { .. })
        ));
        assert!(matches!(
            store.get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID),
            Err(ProductStoreError::NotFound { .. })
        ));
    }
}

fn contract_validation_error() -> ContractValidationFinding {
    ContractValidationFinding {
        code: "invalid_contract".to_string(),
        severity: ContractFindingSeverity::Error,
        logical_work_item_id: Some(WORK_ITEM_ID.to_string()),
        contract_ref: None,
        capability_ref: None,
        message: "contract validation failed".to_string(),
    }
}

fn initial_publication_journal(
    store: &WorkItemRevisionStore,
) -> crate::product::work_item_revision_store::InitialPlanPublicationJournal {
    let timestamp = "2026-07-17T00:00:20Z";
    let ids = store
        .allocate_initial_plan_publication_ids(
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "compile_0001",
            &[WORK_ITEM_ID.to_string()],
        )
        .unwrap();
    let work_item_ids = ids.work_items[WORK_ITEM_ID].clone();
    let mut contract = canonical_contract_fixture(WORK_ITEM_ID);
    contract.input_contracts.clear();
    contract.handoff_contract.provided_contract_refs.clear();
    let contract_hash = canonical_contract_hash(&contract).unwrap();
    let work_item_revision = WorkItemRevision {
        id: work_item_ids.work_item_revision_id.clone(),
        logical_work_item_id: WORK_ITEM_ID.to_string(),
        source_draft_revision_id: "draft_revision_0001".to_string(),
        canonical_contract: contract.clone(),
        canonical_contract_hash: contract_hash.clone(),
        work_item_projection_bundle_id: work_item_ids.work_item_projection_bundle_id.clone(),
        verification_plan_revision_id: work_item_ids.verification_plan_revision_id.clone(),
        created_at: timestamp.to_string(),
    };
    let compiled_work_item = WorkItemProjectionCompiler
        .compile(&contract, &work_item_revision.id)
        .unwrap();
    let hashes = projection_hashes(&compiled_work_item).unwrap();
    let work_item_projection = WorkItemProjectionBundle {
        id: work_item_ids.work_item_projection_bundle_id.clone(),
        work_item_revision_id: work_item_revision.id.clone(),
        canonical_contract_hash: contract_hash,
        projection_schema_version: 1,
        compiler_version: "compiler-v1".to_string(),
        human_projection: compiled_work_item.human.clone(),
        coder_projection: compiled_work_item.coder.clone(),
        reviewer_projection: compiled_work_item.reviewer.clone(),
        human_projection_hash: hashes.human,
        coder_projection_hash: hashes.coder,
        reviewer_projection_hash: hashes.reviewer,
        created_at: timestamp.to_string(),
    };
    let graph = build_dependency_contract_graph(&[contract.clone()]).unwrap();
    let work_item_projections = BTreeMap::from([(WORK_ITEM_ID.to_string(), compiled_work_item)]);
    let compiled_plan = PlanProjectionCompiler
        .compile(PlanProjectionCompileInput {
            plan_id: PLAN_ID,
            goal: "Publish initial plan revision",
            split_reason: "Single work item",
            source_refs: vec![
                "story_spec_0001".to_string(),
                "design_spec_0001".to_string(),
            ],
            dependency_graph: &graph,
            work_item_projections: &work_item_projections,
            expected_work_item_revision_ids: BTreeMap::from([(
                WORK_ITEM_ID.to_string(),
                work_item_revision.id.clone(),
            )]),
        })
        .unwrap();
    let plan_projection = PlanProjectionBundle {
        id: ids.plan_projection_bundle_id.clone(),
        plan_revision_id: ids.plan_revision_id.clone(),
        dependency_graph_revision_id: ids.dependency_graph_revision_id.clone(),
        work_item_projection_bundle_refs: vec![work_item_projection.id.clone()],
        human_group_projection: compiled_plan.human,
        coder_group_context: compiled_plan.coder,
        reviewer_group_matrix: compiled_plan.reviewer,
        human_group_projection_hash: "human_group_hash".to_string(),
        coder_group_context_hash: "coder_group_hash".to_string(),
        reviewer_group_matrix_hash: "reviewer_group_hash".to_string(),
        compiler_version: "compiler-v1".to_string(),
        created_at: timestamp.to_string(),
    };
    let artifacts = InitialPlanPublicationArtifacts {
        lineage: WorkItemPlanLineage {
            id: PLAN_ID.to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_refs: vec!["story_spec_0001".to_string()],
            design_spec_refs: vec!["design_spec_0001".to_string()],
            active_revision_id: None,
            active_amendment_id: None,
            created_at: timestamp.to_string(),
            updated_at: timestamp.to_string(),
        },
        plan_revision: WorkItemPlanRevision {
            id: ids.plan_revision_id.clone(),
            plan_id: PLAN_ID.to_string(),
            revision_no: 1,
            supersedes: None,
            reason: PlanRevisionReason::InitialCompile,
            work_item_bindings: BTreeMap::from([(
                WORK_ITEM_ID.to_string(),
                work_item_revision.id.clone(),
            )]),
            dependency_graph_revision_id: ids.dependency_graph_revision_id.clone(),
            validation_report_ref: ids.validation_report_id.clone(),
            plan_projection_bundle_id: ids.plan_projection_bundle_id.clone(),
            created_at: timestamp.to_string(),
        },
        dependency_graph_revision: DependencyGraphRevision {
            id: ids.dependency_graph_revision_id.clone(),
            plan_id: PLAN_ID.to_string(),
            edges: graph.edges,
            created_at: timestamp.to_string(),
        },
        validation_report: PlanValidationReportArtifact {
            id: ids.validation_report_id.clone(),
            plan_id: PLAN_ID.to_string(),
            plan_revision_id: ids.plan_revision_id.clone(),
            plan_projection_bundle_id: ids.plan_projection_bundle_id.clone(),
            contract_validation: ContractValidationReport { findings: vec![] },
            projection_validation: ProjectionValidationReport { findings: vec![] },
            created_at: timestamp.to_string(),
        },
        plan_projection_bundle: plan_projection,
        work_items: vec![InitialWorkItemPublicationArtifacts {
            logical_work_item: LogicalWorkItem {
                id: WORK_ITEM_ID.to_string(),
                plan_id: PLAN_ID.to_string(),
                title: contract.identity.title.clone(),
                active_revision_id: None,
                created_at: timestamp.to_string(),
                updated_at: timestamp.to_string(),
            },
            draft_revision: WorkItemDraftRevision {
                id: "draft_revision_0001".to_string(),
                logical_work_item_id: WORK_ITEM_ID.to_string(),
                revision_no: 1,
                supersedes: None,
                revision_reason: PlanRevisionReason::InitialCompile,
                canonical_contract_candidate: contract.clone(),
                trigger_repair_request_id: None,
                created_at: timestamp.to_string(),
            },
            work_item_revision,
            verification_plan_revision: VerificationPlanRevision {
                id: work_item_ids.verification_plan_revision_id,
                logical_work_item_id: WORK_ITEM_ID.to_string(),
                source_draft_revision_id: "draft_revision_0001".to_string(),
                verification_checks: contract.verification_checks,
                created_at: timestamp.to_string(),
            },
            projection_bundle: work_item_projection,
        }],
    };
    store
        .build_initial_plan_publication_journal(
            "compile_0001",
            "outline_0001",
            BTreeMap::from([(WORK_ITEM_ID.to_string(), "draft_revision_0001".to_string())]),
            timestamp,
            artifacts,
        )
        .unwrap()
}

fn assert_initial_publication_is_complete(
    store: &WorkItemRevisionStore,
    journal: &crate::product::work_item_revision_store::InitialPlanPublicationJournal,
) {
    let lineage = store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap();
    assert_eq!(
        lineage.active_revision_id.as_deref(),
        Some(journal.artifacts.plan_revision.id.as_str())
    );
    let plan_revision = store
        .get_plan_revision(
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            &journal.artifacts.plan_revision.id,
        )
        .unwrap();
    assert_eq!(plan_revision, journal.artifacts.plan_revision);
    for item in &journal.artifacts.work_items {
        let revision = store
            .get_work_item_revision(
                &lineage,
                &item.logical_work_item.id,
                &item.work_item_revision.id,
            )
            .unwrap();
        assert_eq!(revision, item.work_item_revision);
        assert_eq!(
            store
                .get_verification_plan_revision(&lineage, &item.verification_plan_revision.id,)
                .unwrap(),
            item.verification_plan_revision
        );
        assert_eq!(
            store
                .get_work_item_projection_bundle(&lineage, &item.projection_bundle.id)
                .unwrap(),
            item.projection_bundle
        );
    }
}
