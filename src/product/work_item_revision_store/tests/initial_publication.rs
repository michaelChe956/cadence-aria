use super::*;
use crate::product::coding_attempt_store::{
    CodingAttemptStore, CreateCodingExecutionUnitInput, CreateGroupCodingAttemptInput,
};
use crate::product::coding_evaluation_context::{
    EvaluationContextRole, build_evaluation_context_pack,
};
use crate::product::coding_models::{
    CodingAttemptPlanBinding, CodingExecutionUnit, CodingExecutionUnitStatus, CodingUnitRun,
    CodingUnitRunStatus,
};
use crate::product::coding_work_item_context::load_coding_work_item_context;
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::models::{
    DependencyGraphRevision, PlanProjectionBundle, PlanValidationReportArtifact, ProviderName,
    VerificationPlanRevision, WorkItemPlanRevision, WorkItemProjectionBundle,
    WorkItemRuntimeBinding, WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::product::work_item_contract::{
    ContractFindingSeverity, ContractValidationFinding, ContractValidationReport,
    build_dependency_contract_graph, canonical_contract_hash,
};
use crate::product::work_item_projection::{
    PlanProjectionCompileInput, PlanProjectionCompiler, ProjectionValidationReport,
    WorkItemProjectionCompiler, projection_hashes,
};
use crate::product::work_item_revision_store::{
    InitialPlanPublicationArtifacts, InitialPlanPublicationCheckpoint,
    InitialPlanPublicationJournal, InitialPlanPublicationPhase,
    InitialWorkItemPublicationArtifacts,
};
use crate::product::work_item_runtime_reader::WorkItemRuntimeReader;
use crate::web::coding_ws_handler::repository_path_for_attempt;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
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

#[test]
fn runtime_reader_resolves_a_published_binding_without_legacy_work_item_records() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let journal = initial_publication_journal(&store);
    let published = store
        .publish_or_resume_initial_plan_revision(&journal)
        .unwrap();
    let item = published.artifacts.work_items.first().unwrap();
    let bundle = &item.projection_bundle;
    let binding = crate::product::models::WorkItemRuntimeBinding {
        plan_id: PLAN_ID.to_string(),
        plan_revision_id: published.artifacts.plan_revision.id.clone(),
        logical_work_item_id: item.logical_work_item.id.clone(),
        work_item_revision_id: item.work_item_revision.id.clone(),
        projection_bundle_id: bundle.id.clone(),
        verification_plan_revision_id: item.verification_plan_revision.id.clone(),
        canonical_contract_hash: item.work_item_revision.canonical_contract_hash.clone(),
        projection_compiler_version: bundle.compiler_version.clone(),
        human_projection_hash: bundle.human_projection_hash.clone(),
        coder_projection_hash: bundle.coder_projection_hash.clone(),
        reviewer_projection_hash: bundle.reviewer_projection_hash.clone(),
    };

    let resolved = WorkItemRuntimeReader::new(paths)
        .resolve_binding(PROJECT_ID, ISSUE_ID, &binding)
        .unwrap();

    assert_eq!(resolved.binding, binding);
    assert_eq!(resolved.work_item_revision, item.work_item_revision);
    assert_eq!(
        resolved.verification_plan_revision,
        item.verification_plan_revision
    );
    assert_eq!(resolved.projection_bundle, *bundle);
}

#[test]
fn runtime_reader_fails_closed_when_binding_references_another_projection_bundle() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let journal = initial_publication_journal(&store);
    let published = store
        .publish_or_resume_initial_plan_revision(&journal)
        .unwrap();
    let item = published.artifacts.work_items.first().unwrap();
    let bundle = &item.projection_bundle;
    let binding = crate::product::models::WorkItemRuntimeBinding {
        plan_id: PLAN_ID.to_string(),
        plan_revision_id: published.artifacts.plan_revision.id.clone(),
        logical_work_item_id: item.logical_work_item.id.clone(),
        work_item_revision_id: item.work_item_revision.id.clone(),
        projection_bundle_id: "projection_bundle_from_another_revision".to_string(),
        verification_plan_revision_id: item.verification_plan_revision.id.clone(),
        canonical_contract_hash: item.work_item_revision.canonical_contract_hash.clone(),
        projection_compiler_version: bundle.compiler_version.clone(),
        human_projection_hash: bundle.human_projection_hash.clone(),
        coder_projection_hash: bundle.coder_projection_hash.clone(),
        reviewer_projection_hash: bundle.reviewer_projection_hash.clone(),
    };

    let error = WorkItemRuntimeReader::new(paths)
        .resolve_binding(PROJECT_ID, ISSUE_ID, &binding)
        .unwrap_err();

    assert!(matches!(
        error,
        ProductStoreError::IdentityMismatch {
            kind: "runtime_binding_integrity_mismatch",
            ..
        }
    ));
}

#[test]
fn runtime_reader_resolves_a_work_item_workspace_binding() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let published = store
        .publish_or_resume_initial_plan_revision(&initial_publication_journal(&store))
        .unwrap();
    let binding = runtime_binding(&published);
    let session = work_item_workspace_session(Some(binding.clone()));

    let resolved = WorkItemRuntimeReader::new(paths)
        .resolve_workspace(&session)
        .unwrap();

    assert_eq!(resolved.binding, binding);
    assert_eq!(
        resolved.work_item_revision.id,
        binding.work_item_revision_id
    );
}

#[test]
fn runtime_reader_rejects_missing_or_non_work_item_workspace_bindings() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let published = store
        .publish_or_resume_initial_plan_revision(&initial_publication_journal(&store))
        .unwrap();
    let binding = runtime_binding(&published);
    let reader = WorkItemRuntimeReader::new(paths);

    let missing_error = reader
        .resolve_workspace(&work_item_workspace_session(None))
        .unwrap_err();
    assert!(matches!(
        missing_error,
        ProductStoreError::IdentityMismatch {
            kind: "runtime_binding_missing",
            ..
        }
    ));

    for workspace_type in [WorkspaceType::Story, WorkspaceType::Design] {
        let mut session = work_item_workspace_session(Some(binding.clone()));
        session.workspace_type = workspace_type;
        let error = reader.resolve_workspace(&session).unwrap_err();
        assert!(matches!(
            error,
            ProductStoreError::IdentityMismatch {
                kind: "runtime_workspace_type",
                ..
            }
        ));
    }
}

#[tokio::test]
async fn runtime_reader_derives_coding_unit_binding_and_rejects_run_hash_mismatch() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let repository = RepositoryStore::new(paths.clone())
        .create(CreateRepositoryInput {
            project_id: PROJECT_ID.to_string(),
            name: "Runtime Reader Repository".to_string(),
            path: temp.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: "runtime-reader-repository".to_string(),
        })
        .unwrap();
    let issue = IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: Some(repository.id.clone()),
            title: "Runtime Reader Issue".to_string(),
            description: None,
            change_id: None,
        })
        .unwrap();
    assert_eq!(issue.id, ISSUE_ID);
    let revision_store = WorkItemRevisionStore::new(paths.clone());
    let published = revision_store
        .publish_or_resume_initial_plan_revision(&initial_publication_journal(&revision_store))
        .unwrap();
    let binding = runtime_binding(&published);
    let attempt_store = CodingAttemptStore::new(paths.clone());
    let attempt = attempt_store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: binding.plan_id.clone(),
            current_work_item_id: binding.logical_work_item_id.clone(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/runtime-reader".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            max_auto_rework: 2,
        })
        .unwrap();
    attempt_store
        .save_plan_binding(
            &attempt,
            &CodingAttemptPlanBinding {
                attempt_id: attempt.id.clone(),
                plan_id: binding.plan_id.clone(),
                bound_plan_revision_id: binding.plan_revision_id.clone(),
                applied_amendment_ids: Vec::new(),
                updated_at: "2026-07-26T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    let unit = attempt_store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            plan_id: binding.plan_id.clone(),
            logical_work_item_id: binding.logical_work_item_id.clone(),
            work_item_revision_id: binding.work_item_revision_id.clone(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .unwrap();
    let attempt = attempt_store
        .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
        .unwrap();
    let run = coding_unit_run(&unit, &binding);
    let reader = WorkItemRuntimeReader::new(paths.clone());

    let resolved = reader
        .resolve_coding_unit(&attempt, &unit, Some(&run))
        .unwrap();
    assert_eq!(resolved.binding, binding);

    let (coder_projection, coder_hash) = reader
        .coder_projection_for_unit(&attempt, &unit, Some(&run))
        .unwrap();
    assert_eq!(
        coder_projection,
        resolved.projection_bundle.coder_projection
    );
    assert_eq!(coder_hash, resolved.binding.coder_projection_hash);

    let (reviewer_projection, reviewer_hash) = reader
        .reviewer_projection_for_unit(&attempt, &unit, Some(&run))
        .unwrap();
    assert_eq!(
        reviewer_projection,
        resolved.projection_bundle.reviewer_projection
    );
    assert_eq!(reviewer_hash, resolved.binding.reviewer_projection_hash);

    let normative = reader
        .normative_context_for_unit(&attempt, &unit, Some(&run))
        .unwrap();
    assert_eq!(normative.work_item_revision, resolved.work_item_revision);
    assert_eq!(
        normative.verification_plan_revision,
        resolved.verification_plan_revision
    );

    let coder_context = load_coding_work_item_context(&paths, &attempt).unwrap();
    let markdown = coder_context
        .markdown
        .expect("bound coder projection markdown");
    assert!(markdown.contains("## Coder Projection"));
    assert!(markdown.contains(&resolved.projection_bundle.coder_projection.objective));
    assert!(!markdown.contains("Canonical Contract"));

    let evaluation =
        build_evaluation_context_pack(paths.clone(), &attempt, EvaluationContextRole::CodeReviewer)
            .unwrap();
    assert_eq!(
        evaluation.work_item.artifact_id,
        binding.logical_work_item_id
    );
    assert_eq!(evaluation.work_item.repository_id, repository.id);
    assert_eq!(
        evaluation.work_item.title,
        resolved.projection_bundle.human_projection.title
    );
    assert!(
        evaluation
            .work_item
            .raw_markdown_or_sections
            .contains("schema_version")
    );
    assert_eq!(
        evaluation
            .group_context
            .as_ref()
            .map(|context| &context.plan_id),
        Some(&binding.plan_id)
    );
    assert!(
        !evaluation
            .context_warnings
            .iter()
            .any(|warning| warning == "missing_work_item")
    );

    assert_eq!(
        repository_path_for_attempt(&paths, &attempt).unwrap(),
        repository.path
    );

    let mut different_hash = run;
    different_hash.coder_projection_hash = "sha256:another-coder-projection".to_string();
    let error = reader
        .resolve_coding_unit(&attempt, &unit, Some(&different_hash))
        .unwrap_err();
    assert!(matches!(
        error,
        ProductStoreError::IdentityMismatch {
            kind: "runtime_binding_integrity_mismatch",
            ..
        }
    ));
}

fn runtime_binding(published: &InitialPlanPublicationJournal) -> WorkItemRuntimeBinding {
    let item = published.artifacts.work_items.first().unwrap();
    let bundle = &item.projection_bundle;
    WorkItemRuntimeBinding {
        plan_id: PLAN_ID.to_string(),
        plan_revision_id: published.artifacts.plan_revision.id.clone(),
        logical_work_item_id: item.logical_work_item.id.clone(),
        work_item_revision_id: item.work_item_revision.id.clone(),
        projection_bundle_id: bundle.id.clone(),
        verification_plan_revision_id: item.verification_plan_revision.id.clone(),
        canonical_contract_hash: item.work_item_revision.canonical_contract_hash.clone(),
        projection_compiler_version: bundle.compiler_version.clone(),
        human_projection_hash: bundle.human_projection_hash.clone(),
        coder_projection_hash: bundle.coder_projection_hash.clone(),
        reviewer_projection_hash: bundle.reviewer_projection_hash.clone(),
    }
}

fn work_item_workspace_session(
    work_item_runtime_binding: Option<WorkItemRuntimeBinding>,
) -> WorkspaceSessionRecord {
    WorkspaceSessionRecord {
        id: "workspace_session_0001".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        entity_id: WORK_ITEM_ID.to_string(),
        workspace_type: WorkspaceType::WorkItem,
        status: WorkspaceSessionStatus::Open,
        author_provider: ProviderName::Codex,
        reviewer_provider: ProviderName::ClaudeCode,
        review_rounds: 1,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        superpowers_enabled: true,
        openspec_enabled: true,
        work_item_runtime_binding,
        provider_conversations: Vec::new(),
        messages: Vec::new(),
        created_at: "2026-07-26T00:00:00Z".to_string(),
        updated_at: "2026-07-26T00:00:00Z".to_string(),
    }
}

fn coding_unit_run(unit: &CodingExecutionUnit, binding: &WorkItemRuntimeBinding) -> CodingUnitRun {
    CodingUnitRun {
        id: "coding_unit_run_0001".to_string(),
        unit_id: unit.id.clone(),
        execution_no: 1,
        work_item_revision_id: binding.work_item_revision_id.clone(),
        resolved_handoff_revision_ids: Vec::new(),
        canonical_contract_hash: binding.canonical_contract_hash.clone(),
        projection_bundle_id: binding.projection_bundle_id.clone(),
        projection_compiler_version: binding.projection_compiler_version.clone(),
        coder_provider_renderer_version: "codex-provider-projection-renderer-v1".to_string(),
        reviewer_provider_renderer_version: "claude-code-provider-projection-renderer-v1"
            .to_string(),
        internal_reviewer_provider_renderer_version: None,
        coder_projection_hash: binding.coder_projection_hash.clone(),
        reviewer_projection_hash: binding.reviewer_projection_hash.clone(),
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        internal_reviewer_execution_context_hash: None,
        status: CodingUnitRunStatus::Pending,
        unit_rework_count: 0,
        verification_retry_count: 0,
        operational_retry_count: 0,
        plan_repair_count: 0,
        start_commit: None,
        completion_commit: None,
        created_at: "2026-07-26T00:00:00Z".to_string(),
        updated_at: "2026-07-26T00:00:00Z".to_string(),
    }
}
