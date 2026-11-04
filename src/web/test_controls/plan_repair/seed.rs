use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use chrono::Utc;
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
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
};
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, CheckoutAvailability, CheckoutKind, CodebaseMemberRecord,
    IssueCodebaseSelection, IssueCodebaseSelectionStore, LogicalCodebaseManifest,
    LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
    RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
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
    // REQ-COD-03：夹具 attempt 带冻结快照 ⇒ target 桶键为
    // `Some(logical_repository_id)`（见 coding_attempt_store/group.rs
    // `attempt_target_bucket`）。桶键检索 `get_attempt_for_work_item_group`
    // 会落空，按 plan 取 group attempt 改用桶无关的
    // `list_attempts_for_work_item_group`（group.rs 明示口径）。
    if !store
        .list_attempts_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?
        .is_empty()
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
            // 归属必填（REQ-ROUTE-01）：夹具无 per-LC 权威记录 ⇒ issue 归属 project
            // 默认作用域（legacy 别名代码库 id）。
            logical_codebase_id:
                crate::product::logical_codebase::store::legacy_logical_codebase_id(PROJECT_ID),
            title: "Plan Repair fixture issue".to_string(),
            description: None,
            change_id: "plan-repair-fixture".to_string(),
            phase: IssuePhase::Development,
            status: IssueStatus::InProgress,
            active_binding_id: None,
            created_at: CREATED_AT.to_string(),
            updated_at: CREATED_AT.to_string(),
            base_branch: None,
        },
    )
    .map_err(fixture_error)?;

    // REQ-ROUTE-01 两态化（去单仓入口）后，admission/租约/advance 完成链的
    // 唯一凭证是「LC 权威 + 冻结快照」。物理仓保持确定性 `repository_0001`
    // （下游 fixture 全按字面量引用），这里按项目级 legacy 布局补权威：
    // manifest + 本仓 member + 主 checkout + selection + policy 产物，并把
    // repos.json 回填逻辑归属（`resolve_logical_repository_for_issue_codebase`
    // 与 `build_attempt_target_snapshot` 都按它解析物理仓与快照事实）。
    // issue 记录已上方直接落盘（固定 id，无 IssueStore 计数漂移问题），
    // selection 在其之后写入，与共享夹具 `logical_test_fixture` 的顺序纪律
    // 一致。
    let logical_repository_id = LogicalRepositoryId(uuid::Uuid::new_v4());
    let primary_checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
    let source_identity =
        RepositorySourceIdentity::from_git_parts(&worktree, worktree.join(".git"), None);
    let authority = LogicalCodebaseStore::new(paths.clone());
    let manifest =
        LogicalCodebaseManifest::new(PROJECT_ID, worktree.clone(), vec![logical_repository_id]);
    authority
        .save_manifest(PROJECT_ID, &manifest)
        .map_err(fixture_error)?;
    authority
        .save_member(
            PROJECT_ID,
            &CodebaseMemberRecord {
                logical_repository_id,
                physical_repository_id: "repository_0001".to_string(),
                alias: "Plan Repair fixture repository".to_string(),
                role: "repository".to_string(),
                ordinal: 1,
                source_identity: source_identity.clone(),
                repo_type: RepositoryType::Unknown,
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![primary_checkout_id],
                status: MemberStatus::Active,
                created_at: CREATED_AT.to_string(),
                updated_at: CREATED_AT.to_string(),
            },
        )
        .map_err(fixture_error)?;
    authority
        .save_checkout(
            PROJECT_ID,
            &RepositoryCheckoutRecord {
                checkout_id: primary_checkout_id,
                logical_repository_id,
                physical_repository_id: "repository_0001".to_string(),
                kind: CheckoutKind::Main,
                canonical_path: worktree.clone(),
                checkout_path_hash: "sha256:plan-repair-checkout".to_string(),
                git_dir_identity: source_identity.git_dir_identity(),
                // 生产不变量（snapshot_validator.rs 头注）：仓库登记与 identity
                // 迁移从不持久化 checkout revision（恒 None）；快照的 revision 是
                // admission 时冻结的 git HEAD。checkout 侧写死值会与冻结 HEAD
                // 逐字比对失败 ⇒ validate_snapshot_fields 判 identity drift。
                revision: None,
                availability: CheckoutAvailability::Available,
                observed_at: CREATED_AT.to_string(),
                created_at: CREATED_AT.to_string(),
                updated_at: CREATED_AT.to_string(),
            },
        )
        .map_err(fixture_error)?;
    IssueCodebaseSelectionStore::new(paths.clone())
        .save(&IssueCodebaseSelection::all_members(
            PROJECT_ID, ISSUE_ID, None,
        ))
        .map_err(fixture_error)?;
    AggregatePolicyArtifactStore::new(paths.clone())
        .ensure_bootstrap(&manifest)
        .map_err(fixture_error)?;
    let mut repositories = repository_store.list(PROJECT_ID).map_err(fixture_error)?;
    for repository in repositories.iter_mut() {
        if repository.id == "repository_0001" {
            repository.logical_repository_id = Some(logical_repository_id);
            repository.primary_checkout_id = Some(primary_checkout_id);
            // strict 解析链（resolve_logical_repository_strict）要求权威
            // member/checkout 与 repos.json 投影配对一致：本仓由 legacy
            // 形态创建（identity_schema_version=0），仅回填逻辑归属会让
            // 三层身份复验在 schema 版本上 identity_mismatch fail-closed。
            repository.identity_schema_version = 1;
        }
    }
    write_json(
        &paths.project_root(PROJECT_ID).join("repos.json"),
        &repositories,
    )
    .map_err(fixture_error)?;

    let lifecycle = LifecycleStore::new(paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Plan Repair fixture story".to_string(),
            aggregate_codebase: None,
        })
        .map_err(fixture_error)?;
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "Plan Repair fixture design".to_string(),
            aggregate_codebase: None,
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

    // REQ-COD-03（§4.2.3）：admission/租约/advance 完成链的唯一凭证是
    // 「LC 权威 + 冻结快照」。快照取权威事实源 build_attempt_target_snapshot
    // （上方 member/checkout/selection/policy/repos.json 投影同源解析 +
    // checkout HEAD），不自造字段——validate_snapshot_fields 的三层身份
    // 复验（member/checkout/repos 投影 + manifest membership_revision +
    // policy digest）与权威逐字一致。
    let target_snapshot =
        crate::product::coding_attempt_store::target_snapshot::build_attempt_target_snapshot(
            &paths,
            PROJECT_ID,
            logical_repository_id,
            None,
        )
        .map_err(fixture_error)?;
    // REQ-PLN-01/03：规划上下文（child workspace context）经
    // PlanningContextResolver 读 active 聚合索引，缺失即
    // `aggregate_index_active_missing`（child session context 不可用）。
    // 合成索引世代：成员快照 revision 取权威 checkout 的冻结 HEAD（与
    // freshness 采集同源 ⇒ 不漂移 ⇒ 不触发 codegraph 同步）。
    // 落盘 scope 与 `for_lc(legacy 别名)` 一致（legacy 别名 LC 的 scope root
    // 即项目级 legacy 根，见 logical_codebase::lc_scope_root）。
    let aggregate_index_id = format!("aggregate_index_{}", uuid::Uuid::new_v4().simple());
    let mut aggregate_index_record = AggregateIndexRecord::building(
        aggregate_index_id.clone(),
        PROJECT_ID.to_string(),
        manifest.membership_revision,
        vec![AggregateIndexMemberSnapshot::indexed(
            logical_repository_id,
            primary_checkout_id,
            target_snapshot.revision.clone().unwrap_or_default(),
            false,
            CREATED_AT.to_string(),
        )],
        CREATED_AT.to_string(),
    );
    aggregate_index_record.status = AggregateIndexStatus::Active;
    aggregate_index_record.codegraph_root = worktree.clone();
    AggregateIndexStore::for_lc(
        paths.clone(),
        crate::product::logical_codebase::store::legacy_logical_codebase_id(PROJECT_ID),
    )
    .create(PROJECT_ID, aggregate_index_record)
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
            target_snapshot: Some(target_snapshot),
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
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
        publication_provenance_ref: None,
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
    // 播种合法可执行态：Running 必须带 admission 会话标记，否则过不了
    // ensure_provider_run_allowed 的第二道防线。
    attempt.admission_ticket_consumed_at = Some(Utc::now().to_rfc3339());
    attempt.stage = CodingExecutionStage::CodeReview;
    write_json(
        &store.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id),
        &attempt,
    )
    .map_err(fixture_error)?;
    let lifecycle = LifecycleStore::new(paths);
    let base_plan_session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: PLAN_ID.to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
            work_item_plan_options: None,
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
        .list_attempts_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?
        .into_iter()
        .next()
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
        .list_attempts_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?
        .into_iter()
        .next()
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
        intent_contract: None,
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
        depends_on: Vec::new(),
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
        CodingAttemptStatus::AwaitingManualRecovery => "awaiting_manual_recovery",
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
