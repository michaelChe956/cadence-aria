use super::*;
use crate::cross_cutting::streaming_provider::ProviderSession;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{
    CreateCodingAttemptInput, CreateCodingExecutionUnitInput, CreateGroupCodingAttemptInput,
};
use crate::product::coding_models::{
    CodingAttemptPlanBinding, CodingAttemptScope, CodingExecutionUnitStatus, CodingProviderRole,
    CodingUnitRun, CodingUnitRunStatus, RemoteKind,
};
use crate::product::lifecycle_store::{
    CreateIssueWorkItemPlanInput, CreateWorkItemInput, LifecycleStore,
};
use crate::product::models::{
    DependencyGraphRevision, HandoffRevision, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus,
    LogicalWorkItem, PlanProjectionBundle, PlanRevisionReason, ProviderConversationRef,
    ProviderConversationRole, VerificationPlanRevision, WorkItemPlanLineage, WorkItemPlanRevision,
    WorkItemPlanStatus, WorkItemProjectionBundle, WorkItemRevision,
};
use crate::product::work_item_contract::{
    BlockerRoute, BlockerRule, CanonicalWorkItemContract, HandoffContract, PromisedOutputContract,
    VerificationCheck, WorkItemContractIdentity, WorkItemGoal, WorkItemWritePolicy,
    canonical_contract_hash,
};
use crate::product::work_item_projection::{
    CoderGroupContext, CompiledPlanProjections, HumanGroupProjection, HumanGroupWorkItemSummary,
    ReviewerGroupMatrix, ReviewerGroupMatrixEntry, WorkItemProjectionCompiler,
    plan_projection_hashes, projection_hashes, renderer_for,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;
use tempfile::tempdir;

fn seed_group_attempt_fixture(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    initialize_attempt: bool,
    with_dependency: bool,
) {
    seed_group_attempt_fixture_with_legacy_work_items(
        store,
        attempt,
        initialize_attempt,
        with_dependency,
        true,
        &[],
    );
}

fn seed_schema_v2_group_attempt_fixture(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    initialize_attempt: bool,
    with_dependency: bool,
    verification_checks: &[VerificationCheck],
) {
    seed_group_attempt_fixture_with_legacy_work_items(
        store,
        attempt,
        initialize_attempt,
        with_dependency,
        false,
        verification_checks,
    );
}

fn seed_group_attempt_fixture_with_legacy_work_items(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    initialize_attempt: bool,
    with_dependency: bool,
    include_legacy_work_items: bool,
    verification_checks: &[VerificationCheck],
) {
    let lifecycle = LifecycleStore::new(store.paths());
    let work_items = [
        ("work_item_0001", "work_item_revision_0001"),
        ("work_item_0002", "work_item_revision_0002"),
        ("work_item_0003", "work_item_revision_0003"),
    ];
    if include_legacy_work_items {
        for (index, (work_item_id, _)) in work_items.iter().enumerate() {
            lifecycle
                .create_work_item(CreateWorkItemInput {
                    id: Some((*work_item_id).to_string()),
                    project_id: attempt.project_id.clone(),
                    issue_id: attempt.issue_id.clone(),
                    repository_id: "repository_0001".to_string(),
                    title: format!("group work item {}", index + 1),
                    work_item_set_id: Some("work_item_plan_0001".to_string()),
                    sequence_hint: Some(((index + 1) * 10) as u32),
                    plan_status: WorkItemPlanStatus::Confirmed,
                    ..Default::default()
                })
                .expect("group work item");
        }
        lifecycle
            .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
                id: Some("work_item_plan_0001".to_string()),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                source_story_spec_ids: Vec::new(),
                source_design_spec_ids: Vec::new(),
                options: IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: IssueWorkItemPlanStatus::Confirmed,
                work_item_ids: work_items
                    .iter()
                    .map(|(work_item_id, _)| (*work_item_id).to_string())
                    .collect(),
                repository_profile_ref: None,
                verification_plan_ids: Vec::new(),
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
            })
            .expect("group plan");
    }

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
    let mut plan_bindings = std::collections::BTreeMap::new();
    let mut work_item_projections = std::collections::BTreeMap::new();
    let mut projection_bundle_refs = Vec::new();
    for (index, (work_item_id, revision_id)) in work_items.iter().enumerate() {
        let logical = LogicalWorkItem {
            id: (*work_item_id).to_string(),
            plan_id: lineage.id.clone(),
            title: format!("group work item {}", index + 1),
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
            output_contracts: vec![PromisedOutputContract {
                contract_id: format!("contract_{work_item_id}"),
                capabilities: vec![format!("capability_{work_item_id}")],
            }],
            tasks: Vec::new(),
            write_policy: WorkItemWritePolicy {
                exclusive_scopes: Vec::new(),
                forbidden_scopes: Vec::new(),
            },
            acceptance_criteria: Vec::new(),
            verification_checks: verification_checks.to_vec(),
            handoff_contract: HandoffContract {
                required_fields: Vec::new(),
                provided_contract_refs: vec![format!("contract_{work_item_id}")],
                reviewer_check_refs: Vec::new(),
            },
            blocker_rules: vec![
                BlockerRule {
                    reason_code: "current_work_item_contract_invalid".to_string(),
                    route: BlockerRoute::PlanRepairCurrent,
                    target_contract_refs: Vec::new(),
                },
                BlockerRule {
                    reason_code: "story_scope_invalid".to_string(),
                    route: BlockerRoute::StoryAmendment,
                    target_contract_refs: Vec::new(),
                },
                BlockerRule {
                    reason_code: "design_constraint_invalid".to_string(),
                    route: BlockerRoute::DesignAmendment,
                    target_contract_refs: Vec::new(),
                },
                BlockerRule {
                    reason_code: "verification_incomplete".to_string(),
                    route: BlockerRoute::VerificationRetry,
                    target_contract_refs: Vec::new(),
                },
                BlockerRule {
                    reason_code: "operational_blocker".to_string(),
                    route: BlockerRoute::OperationalGate,
                    target_contract_refs: Vec::new(),
                },
            ],
            design_traceability: Vec::new(),
        };
        let work_item_revision = WorkItemRevision {
            id: (*revision_id).to_string(),
            logical_work_item_id: logical.id.clone(),
            source_draft_revision_id: format!("draft_revision_{:04}", index + 1),
            canonical_contract_hash: canonical_contract_hash(&contract).expect("contract hash"),
            canonical_contract: contract.clone(),
            work_item_projection_bundle_id: format!("projection_bundle_{:04}", index + 1),
            verification_plan_revision_id: format!("verification_revision_{:04}", index + 1),
            created_at: "2026-07-18T00:00:00Z".to_string(),
        };
        revision_store
            .put_verification_plan_revision(
                &lineage,
                &VerificationPlanRevision {
                    id: work_item_revision.verification_plan_revision_id.clone(),
                    logical_work_item_id: logical.id.clone(),
                    source_draft_revision_id: work_item_revision.source_draft_revision_id.clone(),
                    verification_checks: work_item_revision
                        .canonical_contract
                        .verification_checks
                        .clone(),
                    created_at: "2026-07-18T00:00:00Z".to_string(),
                },
            )
            .expect("verification plan revision");
        revision_store
            .put_work_item_revision(&lineage, &work_item_revision)
            .expect("work item revision");
        let projections = WorkItemProjectionCompiler
            .compile(
                &work_item_revision.canonical_contract,
                &work_item_revision.id,
            )
            .expect("work item projections");
        let hashes = projection_hashes(&projections).expect("projection hashes");
        revision_store
            .put_work_item_projection_bundle(
                &lineage,
                &WorkItemProjectionBundle {
                    id: work_item_revision.work_item_projection_bundle_id.clone(),
                    work_item_revision_id: work_item_revision.id.clone(),
                    canonical_contract_hash: work_item_revision.canonical_contract_hash.clone(),
                    projection_schema_version: 1,
                    compiler_version: "work-item-projection-compiler-v1".to_string(),
                    human_projection: projections.human.clone(),
                    coder_projection: projections.coder.clone(),
                    reviewer_projection: projections.reviewer.clone(),
                    human_projection_hash: hashes.human,
                    coder_projection_hash: hashes.coder,
                    reviewer_projection_hash: hashes.reviewer,
                    created_at: "2026-07-18T00:00:00Z".to_string(),
                },
            )
            .expect("work item projection bundle");
        projection_bundle_refs.push(work_item_revision.work_item_projection_bundle_id.clone());
        work_item_projections.insert(logical.id.clone(), projections);
        revision_store
            .set_active_work_item_revision(&lineage, &logical, None, &work_item_revision.id)
            .expect("active work item revision");
        plan_bindings.insert(logical.id, work_item_revision.id);
    }
    let graph = DependencyGraphRevision {
        id: "dependency_graph_revision_0001".to_string(),
        plan_id: lineage.id.clone(),
        edges: if with_dependency {
            vec![crate::product::work_item_contract::DependencyContractEdge {
                from: "work_item_0001".to_string(),
                to: "work_item_0002".to_string(),
                required_contracts: Vec::new(),
            }]
        } else {
            Vec::new()
        },
        created_at: "2026-07-18T00:00:00Z".to_string(),
    };
    revision_store
        .put_dependency_graph_revision(&lineage, &graph)
        .expect("dependency graph");
    let ordered_logical_work_item_ids = work_items
        .iter()
        .map(|(logical_id, _)| (*logical_id).to_string())
        .collect::<Vec<_>>();
    let compiled_plan = CompiledPlanProjections {
        human: HumanGroupProjection {
            plan_id: lineage.id.clone(),
            goal: "group attempt fixture".to_string(),
            split_reason: "fixture uses deterministic plan projections".to_string(),
            work_items: ordered_logical_work_item_ids
                .iter()
                .map(|logical_id| {
                    let projection = &work_item_projections[logical_id].human;
                    HumanGroupWorkItemSummary {
                        logical_work_item_id: logical_id.clone(),
                        title: projection.title.clone(),
                        goal: projection.goal.clone(),
                        depends_on: graph
                            .edges
                            .iter()
                            .filter(|edge| edge.to == *logical_id)
                            .map(|edge| edge.from.clone())
                            .collect(),
                        provides: projection
                            .outputs
                            .iter()
                            .map(|output| output.contract_id.clone())
                            .collect(),
                        scope_summary: projection.scope_summary.clone(),
                    }
                })
                .collect(),
            contract_flow: Vec::new(),
            risks: Vec::new(),
            source_refs: Vec::new(),
            normative: false,
            used_by_provider: false,
        },
        coder: CoderGroupContext {
            plan_id: lineage.id.clone(),
            ordered_logical_work_item_ids: ordered_logical_work_item_ids.clone(),
            dependency_edges: graph.edges.clone(),
            group_write_scopes: ordered_logical_work_item_ids
                .iter()
                .map(|logical_id| {
                    (
                        logical_id.clone(),
                        work_item_projections[logical_id].coder.write_policy.clone(),
                    )
                })
                .collect(),
        },
        reviewer: ReviewerGroupMatrix {
            plan_id: lineage.id.clone(),
            work_items: ordered_logical_work_item_ids
                .iter()
                .map(|logical_id| ReviewerGroupMatrixEntry {
                    logical_work_item_id: logical_id.clone(),
                    criterion_refs: work_item_projections[logical_id]
                        .reviewer
                        .criterion_refs
                        .clone(),
                    input_contract_refs: Vec::new(),
                    output_contract_refs: Vec::new(),
                })
                .collect(),
            dependency_edges: graph.edges.clone(),
            design_traceability_refs: Vec::new(),
        },
    };
    let plan_projection_hashes = plan_projection_hashes(&compiled_plan).expect("plan hashes");
    let plan_revision = WorkItemPlanRevision {
        id: "plan_revision_0001".to_string(),
        plan_id: lineage.id.clone(),
        revision_no: 1,
        supersedes: None,
        reason: PlanRevisionReason::InitialCompile,
        work_item_bindings: plan_bindings,
        dependency_graph_revision_id: graph.id,
        validation_report_ref: "validation_report_0001".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        created_at: "2026-07-18T00:00:00Z".to_string(),
    };
    revision_store
        .put_plan_projection_bundle(
            &lineage,
            &PlanProjectionBundle {
                id: plan_revision.plan_projection_bundle_id.clone(),
                plan_revision_id: plan_revision.id.clone(),
                dependency_graph_revision_id: plan_revision.dependency_graph_revision_id.clone(),
                work_item_projection_bundle_refs: projection_bundle_refs,
                human_group_projection: compiled_plan.human,
                coder_group_context: compiled_plan.coder,
                reviewer_group_matrix: compiled_plan.reviewer,
                human_group_projection_hash: plan_projection_hashes.human,
                coder_group_context_hash: plan_projection_hashes.coder,
                reviewer_group_matrix_hash: plan_projection_hashes.reviewer,
                compiler_version: "plan-projection-compiler-v1".to_string(),
                created_at: "2026-07-18T00:00:00Z".to_string(),
            },
        )
        .expect("plan projection bundle");
    revision_store
        .put_plan_revision(&lineage, &plan_revision)
        .expect("plan revision");
    revision_store
        .set_active_plan_revision(&lineage, &plan_revision.id)
        .expect("active plan revision");
    if !initialize_attempt {
        return;
    }
    store
        .save_plan_binding(
            attempt,
            &CodingAttemptPlanBinding {
                attempt_id: attempt.id.clone(),
                plan_id: lineage.id.clone(),
                bound_plan_revision_id: plan_revision.id,
                applied_amendment_ids: Vec::new(),
                updated_at: "2026-07-18T00:00:00Z".to_string(),
            },
        )
        .expect("attempt plan binding");
    for (index, (work_item_id, revision_id)) in work_items.iter().enumerate() {
        store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: attempt.id.clone(),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                plan_id: lineage.id.clone(),
                logical_work_item_id: (*work_item_id).to_string(),
                work_item_revision_id: (*revision_id).to_string(),
                dependency_logical_work_item_ids: if with_dependency && index == 1 {
                    vec!["work_item_0001".to_string()]
                } else {
                    Vec::new()
                },
                order_index: index as u32,
                status: if index == 0 {
                    CodingExecutionUnitStatus::Running
                } else {
                    CodingExecutionUnitStatus::Pending
                },
            })
            .expect("coding unit");
    }
}

mod code_review_triage;
mod coder_resume_recovery;
mod gate_coder_feedback;
mod gate_rework;
mod git_operation_reconcile;
mod group_completion_authority;
mod group_terminal;
mod internal_review_triage;
mod parser_prompt;
mod plan_amendment;
mod plan_defect_entrypoints;
mod provider_driven;
mod provider_execution_context;
mod provider_failure_recovery;
mod provider_rework_context;
mod provider_start_persistence;
mod runtime_handoff_compatibility;
mod runtime_handoff_delta;
mod runtime_handoff_impact;
mod schema_v2_runtime;

#[tokio::test]
async fn group_start_attempt_with_existing_worktree_skips_worktree_prepare_node() {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("shared-worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    seed_group_attempt_fixture(&store, &attempt, true, false);
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start group attempt");

    assert_eq!(updated.scope, CodingAttemptScope::WorkItemGroup);
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(updated.worktree_path.as_deref(), Some(worktree.as_path()));
    assert!(
        store
            .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
            .expect("timeline")
            .is_empty()
    );
    assert_eq!(
        rx.recv().await.expect("stage event"),
        CodingWsOutMessage::CodingStageChange {
            stage: CodingExecutionStage::Coding,
        }
    );
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn coding_plan_repair_partial_group_attempt_cannot_start_coding() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
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
        .expect("partial group attempt");
    seed_group_attempt_fixture(&store, &attempt, false, false);
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let error = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect_err("partial group attempt must not start");

    assert!(
        error
            .to_string()
            .contains("coding_group_attempt_incomplete")
    );
    let persisted = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Created);
    assert_eq!(persisted.stage, CodingExecutionStage::PrepareContext);
}

#[tokio::test]
async fn coding_plan_repair_group_attempt_missing_active_pointer_cannot_start() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
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
    seed_group_attempt_fixture(&store, &attempt, true, false);
    attempt = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("complete group attempt");
    attempt.active_unit_id = None;
    attempt.current_work_item_id = None;
    store
        .save_coding_attempt(&attempt)
        .expect("corrupt attempt pointers");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let error = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect_err("missing active pointer must fail closed");

    assert!(
        error
            .to_string()
            .contains("coding_group_attempt_incomplete")
    );
    let persisted = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Created);
    assert_eq!(persisted.stage, CodingExecutionStage::PrepareContext);
}

#[tokio::test]
async fn single_attempt_completes_after_review_request_without_internal_review_node() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    LifecycleStore::new(store.paths())
        .create_work_item(CreateWorkItemInput {
            id: Some(attempt.work_item_id.clone()),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: "repository_0001".to_string(),
            title: "single work item".to_string(),
            ..Default::default()
        })
        .expect("create work item");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("review request stage");
    let attempt = store
        .update_attempt_head_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            Some("deadbeef".to_string()),
        )
        .expect("head commit");
    store
        .save_review_request(
            &attempt,
            &ReviewRequest {
                id: "review_request_0001".to_string(),
                attempt_id: attempt.id.clone(),
                kind: ReviewRequestKind::GitBranchOnly,
                remote_kind: RemoteKind::GenericGit,
                remote: "origin".to_string(),
                base_branch: attempt.base_branch.clone(),
                branch_name: attempt.branch_name.clone(),
                commit_sha: "deadbeef".to_string(),
                push_status: PushStatus::Pushed,
                external_url: None,
                manual_instructions: Vec::new(),
                created_at: "2026-07-07T00:00:00Z".to_string(),
                updated_at: "2026-07-07T00:00:00Z".to_string(),
            },
        )
        .expect("review request");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let completed = engine
        .complete_attempt_after_review_request(&attempt)
        .await
        .expect("complete after review request");

    assert_eq!(completed.scope, CodingAttemptScope::WorkItem);
    assert_eq!(completed.status, CodingAttemptStatus::Completed);
    assert_eq!(completed.stage, CodingExecutionStage::ReviewRequest);
    assert!(
        store
            .get_timeline_nodes(&completed.project_id, &completed.issue_id, &completed.id)
            .expect("timeline")
            .iter()
            .all(|node| node.stage != CodingExecutionStage::InternalPrReview)
    );
}

fn running_attempt_with_worktree() -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
) {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running attempt");
    (root, store, attempt)
}

fn test_attempt(id: &str) -> CodingExecutionAttempt {
    CodingExecutionAttempt {
        id: id.to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        scope: crate::product::coding_models::CodingAttemptScope::WorkItem,
        status: CodingAttemptStatus::Running,
        stage: CodingExecutionStage::Coding,
        base_branch: "HEAD".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 1,
        },
        provider_conversations: Vec::new(),
        rework_count: 0,
        max_auto_rework: 2,
        work_item_group_id: None,
        current_work_item_id: Some("work_item_0001".to_string()),
        active_unit_id: None,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        created_at: "2026-06-01T00:00:00Z".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        completed_at: None,
    }
}

fn init_test_git_repo(repo: &Path) {
    run_test_git(repo, &["init"]);
    run_test_git(repo, &["config", "user.email", "aria@example.com"]);
    run_test_git(repo, &["config", "user.name", "Aria Test"]);
    fs::write(repo.join("README.md"), "initial\n").expect("seed file");
    run_test_git(repo, &["add", "."]);
    run_test_git(repo, &["commit", "-m", "initial"]);
}

fn git_stdout(cwd: &Path, args: &[&str]) -> String {
    let output = run_test_git(cwd, args);
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn run_test_git(cwd: &Path, args: &[&str]) -> std::process::Output {
    let output = StdCommand::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("git {} failed to start: {error}", args.join(" ")));
    if !output.status.success() {
        panic!(
            "git {} failed\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    output
}
