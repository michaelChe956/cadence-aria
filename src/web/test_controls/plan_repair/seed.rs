use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use tokio::sync::mpsc;

use super::{PlanRepairFixtureError, PlanRepairFixtureWaiting};
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{
    CodingAttemptStore, CreateCodingExecutionUnitInput, CreateGroupCodingAttemptInput,
};
use crate::product::coding_models::{
    CodingAttemptPlanBinding, CodingAttemptStatus, CodingExecutionStage, CodingExecutionUnitStatus,
    CodingUnitRun, CodingUnitRunStatus, FindingSeverity, ReviewFinding,
};
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::json_store::{ProductStoreError, write_json};
use crate::product::lifecycle_store::{
    CreateDesignSpecInput, CreateIssueWorkItemPlanInput, CreateStorySpecInput,
    CreateWorkspaceSessionInput, LifecycleStore,
};
use crate::product::models::{
    DependencyGraphRevision, HandoffRevision, IssuePhase, IssueRecord, IssueStatus,
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, LogicalWorkItem, PlanDefectClass,
    PlanDefectEvidence, PlanDefectRoute, PlanRevisionReason, ProviderName, RepairTarget,
    RepairTargetKind, WorkItemDraftRevision, WorkItemPlanLineage, WorkItemPlanRevision,
    WorkspaceType,
};
use crate::product::plan_repair::PlanDefectConfidence;
use crate::product::product_data_schema::ensure_product_data_schema;
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::product::work_item_contract::{
    BlockerRoute, BlockerRule, CanonicalWorkItemContract, ContractCompatibilityPolicy,
    DependencyContractEdge, HandoffContract, PromisedOutputContract, RequiredDependencyContract,
    RequiredInputContract, WorkItemContractIdentity, WorkItemGoal, WorkItemWritePolicy,
};
use crate::product::work_item_revision_store::{
    InitialWorkItemPublicationIds, WorkItemRevisionStore,
};
use crate::product::workspace_engine::compile_work_item_revision;
use crate::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, ProviderConfigSnapshot, ReviewVerdictType,
    WorkItemRevisionHistoryDto,
};

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_plan_0001";
const PLAN_ID: &str = "work_item_plan_0001";
const CREATED_AT: &str = "2026-07-20T00:00:00Z";

pub(super) fn seed_initial_fixture(root: &Path) -> Result<(), PlanRepairFixtureError> {
    let paths = fixture_paths(root);
    ensure_product_data_schema(&paths).map_err(fixture_error)?;
    let worktree = root.join("worktree");
    initialize_git_worktree(&worktree)?;

    let store = CodingAttemptStore::new(paths.clone());
    if store
        .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?
        .is_some()
    {
        return Ok(());
    }

    let project_store = ProjectStore::new(paths.clone());
    match project_store.get(PROJECT_ID) {
        Ok(_) => {}
        Err(ProductStoreError::NotFound { .. }) => {
            let project = project_store
                .create(CreateProjectInput {
                    name: "Plan Repair fixture project".to_string(),
                    description: None,
                })
                .map_err(fixture_error)?;
            if project.id != PROJECT_ID {
                return Err(fixture_error("fixture project id is not deterministic"));
            }
        }
        Err(error) => return Err(fixture_error(error)),
    }
    let repository_store = RepositoryStore::new(paths.clone());
    if repository_store
        .list(PROJECT_ID)
        .map_err(fixture_error)?
        .is_empty()
    {
        let repository = repository_store
            .create(CreateRepositoryInput {
                project_id: PROJECT_ID.to_string(),
                name: "Plan Repair fixture repository".to_string(),
                path: worktree.clone(),
                default_policy_preset: Some("manual-write".to_string()),
                default_provider_mode: Some("fake".to_string()),
                idempotency_key: "plan-repair-seed-repository".to_string(),
            })
            .map_err(fixture_error)?;
        if repository.id != "repository_0001" {
            return Err(fixture_error("fixture repository id is not deterministic"));
        }
    }
    write_json(
        &paths.issue_root(PROJECT_ID, ISSUE_ID).join("issue.json"),
        &IssueRecord {
            id: ISSUE_ID.to_string(),
            project_id: PROJECT_ID.to_string(),
            repo_id: Some("repository_0001".to_string()),
            title: "Plan Repair fixture issue".to_string(),
            description: None,
            change_id: "plan-repair-fixture".to_string(),
            phase: IssuePhase::Development,
            status: IssueStatus::InProgress,
            active_binding_id: None,
            created_at: CREATED_AT.to_string(),
            updated_at: CREATED_AT.to_string(),
        },
    )
    .map_err(fixture_error)?;

    let lifecycle = LifecycleStore::new(paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Plan Repair fixture story".to_string(),
        })
        .map_err(fixture_error)?;
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "Plan Repair fixture design".to_string(),
        })
        .map_err(fixture_error)?;
    if story.id != "story_spec_0001" || design.id != "design_spec_0001" {
        return Err(fixture_error(
            "fixture specification ids are not deterministic",
        ));
    }
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some(PLAN_ID.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: vec![story.id.clone()],
            source_design_spec_ids: vec![design.id.clone()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: true,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Confirmed,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .map_err(fixture_error)?;

    let mut attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: PLAN_ID.to_string(),
            current_work_item_id: "wi_registration".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_plan_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            max_auto_rework: 2,
        })
        .map_err(fixture_error)?;

    let revision_store = WorkItemRevisionStore::new(paths.clone());
    let mut plan = WorkItemPlanLineage {
        id: PLAN_ID.to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        story_spec_refs: vec!["story_spec_0001".to_string()],
        design_spec_refs: vec!["design_spec_0001".to_string()],
        active_revision_id: None,
        active_amendment_id: None,
        created_at: CREATED_AT.to_string(),
        updated_at: CREATED_AT.to_string(),
    };
    revision_store
        .put_plan_lineage(&plan)
        .map_err(fixture_error)?;

    let contracts = [
        core_contract(&["workflow_explicit_completion"]),
        registration_contract(),
        unrelated_contract(),
    ];
    let mut bindings = BTreeMap::new();
    let mut published_work_items = Vec::new();
    for contract in &contracts {
        let logical_id = contract.identity.logical_work_item_id.clone();
        let revision_id = format!("work_item_revision_{logical_id}_0001");
        let draft = WorkItemDraftRevision {
            id: format!("work_item_draft_revision_{logical_id}_0001"),
            logical_work_item_id: logical_id.clone(),
            revision_no: 1,
            supersedes: None,
            revision_reason: PlanRevisionReason::InitialCompile,
            canonical_contract_candidate: contract.clone(),
            trigger_repair_request_id: None,
            created_at: CREATED_AT.to_string(),
        };
        let compiled = compile_work_item_revision(
            &draft,
            &crate::product::work_item_projection::WorkItemProjectionCompiler,
            &InitialWorkItemPublicationIds {
                work_item_revision_id: revision_id.clone(),
                verification_plan_revision_id: format!(
                    "verification_plan_revision_{logical_id}_0001"
                ),
                work_item_projection_bundle_id: format!(
                    "work_item_projection_bundle_{logical_id}_0001"
                ),
            },
            CREATED_AT,
        )
        .map_err(fixture_error)?;
        let logical = LogicalWorkItem {
            id: logical_id.clone(),
            plan_id: PLAN_ID.to_string(),
            title: contract.identity.title.clone(),
            active_revision_id: None,
            created_at: CREATED_AT.to_string(),
            updated_at: CREATED_AT.to_string(),
        };
        revision_store
            .put_logical_work_item(&plan, &logical)
            .map_err(fixture_error)?;
        revision_store
            .put_draft_revision(&plan, &compiled.draft_revision)
            .map_err(fixture_error)?;
        revision_store
            .put_verification_plan_revision(&plan, &compiled.verification_plan_revision)
            .map_err(fixture_error)?;
        revision_store
            .put_work_item_projection_bundle(&plan, &compiled.projection_bundle)
            .map_err(fixture_error)?;
        revision_store
            .put_work_item_revision(&plan, &compiled.work_item_revision)
            .map_err(fixture_error)?;
        revision_store
            .set_active_work_item_revision(&plan, &logical, None, &revision_id)
            .map_err(fixture_error)?;
        bindings.insert(logical_id, revision_id);
        published_work_items.push(compiled);
    }

    let dependency_graph = DependencyGraphRevision {
        id: "dependency_graph_revision_0001".to_string(),
        plan_id: PLAN_ID.to_string(),
        edges: vec![DependencyContractEdge {
            from: "wi_core".to_string(),
            to: "wi_registration".to_string(),
            required_contracts: vec![RequiredDependencyContract {
                contract_id: "contract.workflow".to_string(),
                required_capabilities: vec!["finalization_failure".to_string()],
                compatibility_policy: ContractCompatibilityPolicy::RequireAll,
            }],
        }],
        created_at: CREATED_AT.to_string(),
    };

    revision_store
        .put_dependency_graph_revision(&plan, &dependency_graph)
        .map_err(fixture_error)?;
    let plan_revision = WorkItemPlanRevision {
        id: "plan_revision_0001".to_string(),
        plan_id: PLAN_ID.to_string(),
        revision_no: 1,
        supersedes: None,
        reason: PlanRevisionReason::InitialCompile,
        work_item_bindings: bindings.clone(),
        dependency_graph_revision_id: dependency_graph.id.clone(),
        validation_report_ref: "plan_validation_report_0001".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        created_at: CREATED_AT.to_string(),
    };
    let plan_projection = super::seed_projection::compile_plan_projection_bundle(
        &plan_revision,
        &dependency_graph,
        &published_work_items,
        &story.id,
        &design.id,
        CREATED_AT,
    )
    .map_err(fixture_error)?;
    revision_store
        .put_plan_projection_bundle(&plan, &plan_projection)
        .map_err(fixture_error)?;
    revision_store
        .put_plan_revision(&plan, &plan_revision)
        .map_err(fixture_error)?;
    plan = revision_store
        .set_active_plan_revision(&plan, "plan_revision_0001")
        .map_err(fixture_error)?;

    store
        .save_plan_binding(
            &attempt,
            &CodingAttemptPlanBinding {
                attempt_id: attempt.id.clone(),
                plan_id: PLAN_ID.to_string(),
                bound_plan_revision_id: "plan_revision_0001".to_string(),
                applied_amendment_ids: Vec::new(),
                updated_at: CREATED_AT.to_string(),
            },
        )
        .map_err(fixture_error)?;
    let core = store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: PLAN_ID.to_string(),
            logical_work_item_id: "wi_core".to_string(),
            work_item_revision_id: "work_item_revision_wi_core_0001".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Completed,
        })
        .map_err(fixture_error)?;
    let registration = store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: PLAN_ID.to_string(),
            logical_work_item_id: "wi_registration".to_string(),
            work_item_revision_id: "work_item_revision_wi_registration_0001".to_string(),
            dependency_logical_work_item_ids: vec!["wi_core".to_string()],
            order_index: 1,
            status: CodingExecutionUnitStatus::Running,
        })
        .map_err(fixture_error)?;
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: PLAN_ID.to_string(),
            logical_work_item_id: "wi_unrelated".to_string(),
            work_item_revision_id: "work_item_revision_wi_unrelated_0001".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 2,
            status: CodingExecutionUnitStatus::Pending,
        })
        .map_err(fixture_error)?;
    attempt = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
        .map_err(fixture_error)?;
    seed_run(
        &store,
        &revision_store,
        &plan,
        &attempt,
        &core,
        SeedRunState {
            id: "0001",
            status: CodingUnitRunStatus::Completed,
            resolved_handoff_revision_ids: &[],
            completion_commit: Some("commit_core_v1"),
        },
    )?;
    seed_run(
        &store,
        &revision_store,
        &plan,
        &attempt,
        &registration,
        SeedRunState {
            id: "coding_unit_run_registration_0001",
            status: CodingUnitRunStatus::Running,
            resolved_handoff_revision_ids: &["handoff_revision_0001"],
            completion_commit: None,
        },
    )?;
    let handoff = HandoffRevision {
        id: "handoff_revision_0001".to_string(),
        logical_work_item_id: "wi_core".to_string(),
        work_item_revision_id: "work_item_revision_wi_core_0001".to_string(),
        coding_unit_run_id: "0001".to_string(),
        provided_contracts: vec!["contract.workflow".to_string()],
        provided_capabilities: BTreeMap::from([(
            "contract.workflow".to_string(),
            vec!["workflow_explicit_completion".to_string()],
        )]),
        contract_hash: "contract_hash_v1".to_string(),
        commit_sha: "commit_core_v1".to_string(),
        created_at: CREATED_AT.to_string(),
    };
    revision_store
        .put_handoff_revision(&plan, &handoff)
        .map_err(fixture_error)?;
    store
        .update_coding_unit_latest_handoff_revision_id(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            &core.id,
            Some(handoff.id),
        )
        .map_err(fixture_error)?;
    store
        .update_coding_unit_completion_commit(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            &core.id,
            Some("commit_core_v1".to_string()),
        )
        .map_err(fixture_error)?;

    attempt.status = CodingAttemptStatus::Running;
    attempt.stage = CodingExecutionStage::CodeReview;
    store.save_coding_attempt(&attempt).map_err(fixture_error)?;
    let lifecycle = LifecycleStore::new(paths);
    let base_plan_session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: PLAN_ID.to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .map_err(fixture_error)?;
    lifecycle
        .save_artifact_versions(
            &base_plan_session.id,
            &[ArtifactVersion {
                version: 1,
                payload: ArtifactPayload::WorkItemRevisionHistory {
                    history: Box::new(WorkItemRevisionHistoryDto {
                        entries: Vec::new(),
                    }),
                },
                generated_by: ProviderName::Codex,
                reviewed_by: Some(ProviderName::ClaudeCode),
                review_verdict: Some(ReviewVerdictType::Pass),
                confirmed_by: Some("fixture_user".to_string()),
                is_current: true,
                created_at: CREATED_AT.to_string(),
                source_node_id: "plan_repair_fixture_history".to_string(),
            }],
        )
        .map_err(fixture_error)?;
    Ok(())
}

pub(super) async fn route_upstream_contract_invalid(
    root: &Path,
) -> Result<PlanRepairFixtureWaiting, PlanRepairFixtureError> {
    let store = CodingAttemptStore::new(fixture_paths(root));
    let attempt = store
        .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?
        .ok_or_else(|| PlanRepairFixtureError::not_implemented("attempt_missing"))?;
    if attempt.status != CodingAttemptStatus::AwaitingPlanAmendment {
        replay_plan_defect_finding(root, "code_review_report_0001_finding_0001").await?;
    }
    let waiting = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
        .map_err(fixture_error)?;
    let unit = store
        .get_active_coding_unit(PROJECT_ID, ISSUE_ID, &attempt.id)
        .map_err(fixture_error)?
        .ok_or_else(|| PlanRepairFixtureError::not_implemented("active_unit_missing"))?;
    let run = store.get_active_unit_run(&waiting).map_err(fixture_error)?;
    Ok(PlanRepairFixtureWaiting {
        attempt_status: status_name(&waiting.status).to_string(),
        active_logical_work_item_id: unit.logical_work_item_id,
        active_unit_rework_count: run.unit_rework_count,
    })
}

pub(super) async fn replay_plan_defect_finding(
    root: &Path,
    finding_id: &str,
) -> Result<(), PlanRepairFixtureError> {
    start_plan_repair_finding(root, finding_id, upstream_contract_finding(finding_id)).await
}

pub(super) async fn start_plan_repair_finding(
    root: &Path,
    finding_id: &str,
    finding: ReviewFinding,
) -> Result<(), PlanRepairFixtureError> {
    let store = CodingAttemptStore::new(fixture_paths(root));
    let attempt = store
        .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?
        .ok_or_else(|| PlanRepairFixtureError::not_implemented("attempt_missing"))?;
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let plan = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?;
    let registration_revision = revision_store
        .get_work_item_revision(
            &plan,
            "wi_registration",
            "work_item_revision_wi_registration_0001",
        )
        .map_err(fixture_error)?;
    let registration_bundle = revision_store
        .get_work_item_projection_bundle(
            &plan,
            &registration_revision.work_item_projection_bundle_id,
        )
        .map_err(fixture_error)?;
    let (tx, _rx) = mpsc::channel(16);
    CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx)
        .start_plan_repair_from_review(
            &attempt,
            "code_review_report_0001",
            finding_id,
            &finding,
            &registration_bundle.reviewer_projection,
        )
        .await
        .map(|_| ())
        .map_err(fixture_error)
}

pub(super) fn fixture_paths(root: &Path) -> ProductAppPaths {
    ProductAppPaths::new(root.join(".aria"))
}

pub(super) fn core_v2_contract() -> CanonicalWorkItemContract {
    core_contract(&[
        "workflow_explicit_completion",
        "finalization_failure",
        "failure_message",
    ])
}

fn seed_run(
    store: &CodingAttemptStore,
    revision_store: &WorkItemRevisionStore,
    plan: &WorkItemPlanLineage,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    unit: &crate::product::coding_models::CodingExecutionUnit,
    state: SeedRunState<'_>,
) -> Result<(), PlanRepairFixtureError> {
    let revision = revision_store
        .get_work_item_revision(
            plan,
            &unit.logical_work_item_id,
            &unit.work_item_revision_id,
        )
        .map_err(fixture_error)?;
    let bundle = revision_store
        .get_work_item_projection_bundle(plan, &revision.work_item_projection_bundle_id)
        .map_err(fixture_error)?;
    store
        .create_coding_unit_run(
            attempt,
            &CodingUnitRun {
                id: state.id.to_string(),
                unit_id: unit.id.clone(),
                execution_no: 1,
                work_item_revision_id: revision.id,
                resolved_handoff_revision_ids: state
                    .resolved_handoff_revision_ids
                    .iter()
                    .map(|id| (*id).to_string())
                    .collect(),
                canonical_contract_hash: bundle.canonical_contract_hash,
                projection_bundle_id: bundle.id,
                projection_compiler_version: bundle.compiler_version,
                coder_provider_renderer_version: "fixture-coder-v1".to_string(),
                reviewer_provider_renderer_version: "fixture-reviewer-v1".to_string(),
                internal_reviewer_provider_renderer_version: None,
                coder_projection_hash: bundle.coder_projection_hash,
                reviewer_projection_hash: bundle.reviewer_projection_hash,
                coder_execution_context_hash: None,
                reviewer_execution_context_hash: None,
                internal_reviewer_execution_context_hash: None,
                status: state.status,
                unit_rework_count: 0,
                verification_retry_count: 0,
                operational_retry_count: 0,
                plan_repair_count: 0,
                start_commit: Some("commit_fixture_start".to_string()),
                completion_commit: state.completion_commit.map(str::to_string),
                created_at: CREATED_AT.to_string(),
                updated_at: CREATED_AT.to_string(),
            },
        )
        .map_err(fixture_error)
}

struct SeedRunState<'a> {
    id: &'a str,
    status: CodingUnitRunStatus,
    resolved_handoff_revision_ids: &'a [&'a str],
    completion_commit: Option<&'a str>,
}

fn upstream_contract_finding(finding_id: &str) -> ReviewFinding {
    ReviewFinding {
        severity: FindingSeverity::Error,
        file_path: Some("src/registration.rs".to_string()),
        line: Some(1),
        message: "registration cannot observe repository finalization failure".to_string(),
        required_action: Some("repair the upstream finalization contract".to_string()),
        source_stage: CodingExecutionStage::CodeReview,
        evidence: vec!["src/registration.rs:1".to_string()],
        plan_defect_evidence: vec![PlanDefectEvidence {
            kind: "review_finding".to_string(),
            source_ref: format!("code_review_report_0001#{finding_id}"),
            message: "finalization_failure capability is missing".to_string(),
        }],
        related_requirements: Vec::new(),
        related_design_constraints: Vec::new(),
        related_work_item_tasks: Vec::new(),
        defect_class: PlanDefectClass::UpstreamContractInvalid,
        reason_code: Some("upstream_contract_invalid".to_string()),
        contract_refs: vec!["contract.workflow".to_string()],
        capability_refs: vec!["finalization_failure".to_string()],
        repair_target: Some(RepairTarget {
            kind: RepairTargetKind::UpstreamWorkItem,
            logical_work_item_ids: vec!["wi_core".to_string()],
            work_item_revision_ids: vec!["work_item_revision_wi_core_0001".to_string()],
        }),
        recommended_route: PlanDefectRoute::PlanRepair,
        confidence: Some(PlanDefectConfidence::High),
    }
}

fn core_contract(capabilities: &[&str]) -> CanonicalWorkItemContract {
    contract(
        "wi_core",
        Vec::new(),
        "contract.workflow",
        capabilities,
        Vec::new(),
    )
}

pub(super) fn registration_contract() -> CanonicalWorkItemContract {
    let mut contract = contract(
        "wi_registration",
        vec![RequiredInputContract {
            contract_id: "contract.workflow".to_string(),
            provider_logical_work_item_id: "wi_core".to_string(),
            required_capabilities: vec!["finalization_failure".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }],
        "contract.registration",
        &["registration_ready"],
        vec![
            BlockerRule {
                reason_code: "upstream_contract_invalid".to_string(),
                route: BlockerRoute::PlanRepairUpstream,
                target_contract_refs: vec!["contract.workflow".to_string()],
            },
            BlockerRule {
                reason_code: "dependency_graph_invalid".to_string(),
                route: BlockerRoute::SubgraphReplan,
                target_contract_refs: vec!["contract.workflow".to_string()],
            },
        ],
    );
    contract.handoff_contract.provided_contract_refs.clear();
    contract
}

fn unrelated_contract() -> CanonicalWorkItemContract {
    let mut contract = contract(
        "wi_unrelated",
        Vec::new(),
        "contract.unrelated",
        &["unrelated_ready"],
        Vec::new(),
    );
    contract.handoff_contract.provided_contract_refs.clear();
    contract
}

fn contract(
    logical_id: &str,
    input_contracts: Vec<RequiredInputContract>,
    output_contract_id: &str,
    capabilities: &[&str],
    blocker_rules: Vec<BlockerRule>,
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
            contract_id: output_contract_id.to_string(),
            capabilities: capabilities
                .iter()
                .map(|capability| (*capability).to_string())
                .collect(),
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
            provided_contract_refs: vec![output_contract_id.to_string()],
            reviewer_check_refs: Vec::new(),
        },
        blocker_rules,
        design_traceability: Vec::new(),
    }
}

pub(super) fn status_name(status: &CodingAttemptStatus) -> &'static str {
    match status {
        CodingAttemptStatus::Created => "created",
        CodingAttemptStatus::Running => "running",
        CodingAttemptStatus::WaitingForHuman => "waiting_for_human",
        CodingAttemptStatus::Blocked => "blocked",
        CodingAttemptStatus::AwaitingPlanAmendment => "awaiting_plan_amendment",
        CodingAttemptStatus::ApplyingPlanAmendment => "applying_plan_amendment",
        CodingAttemptStatus::AmendmentApplyFailed => "amendment_apply_failed",
        CodingAttemptStatus::Completed => "completed",
        CodingAttemptStatus::Failed => "failed",
        CodingAttemptStatus::Aborted => "aborted",
    }
}

fn initialize_git_worktree(path: &Path) -> Result<(), PlanRepairFixtureError> {
    if path.join(".git").exists() {
        return Ok(());
    }
    std::fs::create_dir_all(path).map_err(fixture_error)?;
    run_git(path, &["init", "-q"])?;
    run_git(path, &["config", "user.email", "fixture@example.invalid"])?;
    run_git(path, &["config", "user.name", "Plan Repair Fixture"])?;
    std::fs::write(path.join("README.md"), "plan repair fixture\n").map_err(fixture_error)?;
    run_git(path, &["add", "README.md"])?;
    run_git(path, &["commit", "-q", "-m", "seed fixture"])
}

fn run_git(path: &Path, args: &[&str]) -> Result<(), PlanRepairFixtureError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .map_err(fixture_error)?;
    if output.status.success() {
        return Ok(());
    }
    Err(PlanRepairFixtureError {
        message: format!(
            "plan_repair_fixture_git_failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        fault_point: None,
    })
}

fn fixture_error(error: impl std::fmt::Display) -> PlanRepairFixtureError {
    PlanRepairFixtureError {
        message: format!("plan_repair_fixture_failed: {error}"),
        fault_point: None,
    }
}
