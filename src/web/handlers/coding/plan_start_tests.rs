//! N3 测试族（REQ-CS-01/02/06/08）：plan 一键 POST/GET 聚合、真跑首启编排与
//! 旧入口形状回归。夹具路线同 N1/N2：PlanRepair 种子 + 真实 logical 仓 +
//! per-unit target 草稿 + 扩展 plan_revision_0002（五 WI/四 target，
//! frontend 桶含 wi_service/wi_panel，wi_panel 直接依赖跨桶
//! {wi_unrelated, wi_service}），再经 web 层 Manual advance 到 durable Ready。

use super::*;
use crate::product::advance_store::AdvanceInput;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionUnitStatus, CodingStartOrigin, CodingStartPhase,
};
use crate::product::coding_plan_run::{
    CodingPlanRunPhase, CodingPlanRunStore, CodingPlanSelection, CodingPlanStartOutcome,
};
use crate::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
use crate::product::entity_operation_store::EntityOperationStore;
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, IssueCodebaseSelection, IssueCodebaseSelectionStore,
    LogicalCodebaseFeature, LogicalCodebaseStore, LogicalRepositoryId,
};
use crate::product::models::automation::OperationState;
use crate::product::models::{
    DependencyGraphRevision, LogicalWorkItem, PlanProjectionBundle, PlanRevisionReason,
    WorkItemDraftCandidate, WorkItemDraftRecord, WorkItemDraftRevision, WorkItemDraftStatus,
    WorkItemDraftVerificationPlan, WorkItemGenerationMode, WorkItemPlanRevision,
};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractCompatibilityPolicy, DependencyContractEdge,
    HandoffContract, PromisedOutputContract, RequiredDependencyContract, RequiredInputContract,
    WorkItemContractIdentity, WorkItemGoal, WorkItemWritePolicy,
};
use crate::product::work_item_plan_store::WorkItemPlanStore;
use crate::product::work_item_projection::{
    CoderGroupContext, CompiledPlanProjections, HumanGroupProjection, HumanGroupWorkItemSummary,
    ReviewerGroupMatrix, ReviewerGroupMatrixEntry, WorkItemProjectionCompiler,
    plan_projection_hashes,
};
use crate::product::work_item_revision_store::{
    InitialWorkItemPublicationIds, WorkItemRevisionStore,
};
use crate::product::workspace_engine::{AdvanceOutcome, compile_work_item_revision};
use crate::web::advance_plan::{AdvancePlanOrigin, advance_plan};
use crate::web::handlers::automation_enrollment_test_support::response_json;
use crate::web::runtime::WebRuntime;
use crate::web::state::CodingAttemptRunKey;
use crate::web::test_controls::{PlanRepairFixtureControl, PlanRepairFixtureRuntime};
use crate::web::types::{PlanCodingProgressDto, PlanCodingStartRequest, PlanCodingStartResultDto};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_plan_0001";
const PLAN_ID: &str = "work_item_plan_0001";
const CREATED_AT: &str = "2026-10-13T00:00:00Z";
const PLAN_REVISION_ID: &str = "plan_revision_0002";

/// 五 WI/四 target（park-1 同形）：busi=wi_core、web=wi_registration（依赖
/// wi_core，跨桶）、gateway=wi_unrelated、frontend={wi_service, wi_panel}；
/// wi_panel 直接依赖 {wi_unrelated, wi_service}。
const ORDERED_LOGICAL_WORK_ITEM_IDS: &[&str] = &[
    "wi_core",
    "wi_registration",
    "wi_unrelated",
    "wi_service",
    "wi_panel",
];

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("start git");
    assert!(status.success(), "git {} failed", args.join(" "));
}

fn contract(
    logical_id: &str,
    input_contracts: Vec<RequiredInputContract>,
    output_contract_id: &str,
    capabilities: &[&str],
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
        blocker_rules: Vec::new(),
        design_traceability: Vec::new(),
    }
}

fn seeded_contracts() -> Vec<CanonicalWorkItemContract> {
    let mut core = contract(
        "wi_core",
        Vec::new(),
        "contract.workflow",
        &["workflow_explicit_completion"],
    );
    core.handoff_contract.provided_contract_refs.clear();
    let mut registration = contract(
        "wi_registration",
        vec![RequiredInputContract {
            contract_id: "contract.workflow".to_string(),
            provider_logical_work_item_id: "wi_core".to_string(),
            required_capabilities: vec!["finalization_failure".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }],
        "contract.registration",
        &["registration_ready"],
    );
    registration
        .handoff_contract
        .provided_contract_refs
        .push("contract.workflow".to_string());
    let mut unrelated = contract(
        "wi_unrelated",
        Vec::new(),
        "contract.unrelated",
        &["unrelated_ready"],
    );
    unrelated.handoff_contract.provided_contract_refs.clear();
    let mut service = contract(
        "wi_service",
        vec![RequiredInputContract {
            contract_id: "contract.unrelated".to_string(),
            provider_logical_work_item_id: "wi_unrelated".to_string(),
            required_capabilities: vec!["unrelated_ready".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }],
        "contract.service",
        &["service_ready"],
    );
    service.handoff_contract.provided_contract_refs.clear();
    let mut panel = contract(
        "wi_panel",
        vec![
            RequiredInputContract {
                contract_id: "contract.unrelated".to_string(),
                provider_logical_work_item_id: "wi_unrelated".to_string(),
                required_capabilities: vec!["unrelated_ready".to_string()],
                compatibility_policy: ContractCompatibilityPolicy::RequireAll,
            },
            RequiredInputContract {
                contract_id: "contract.service".to_string(),
                provider_logical_work_item_id: "wi_service".to_string(),
                required_capabilities: vec!["service_ready".to_string()],
                compatibility_policy: ContractCompatibilityPolicy::RequireAll,
            },
        ],
        "contract.panel",
        &["panel_ready"],
    );
    panel.handoff_contract.provided_contract_refs.clear();
    vec![core, registration, unrelated, service, panel]
}

fn dependency_graph_0002() -> DependencyGraphRevision {
    let edge = |from: &str, to: &str, contract_id: &str, capability: &str| DependencyContractEdge {
        from: from.to_string(),
        to: to.to_string(),
        required_contracts: vec![RequiredDependencyContract {
            contract_id: contract_id.to_string(),
            required_capabilities: vec![capability.to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }],
    };
    DependencyGraphRevision {
        id: "dependency_graph_revision_0002".to_string(),
        plan_id: PLAN_ID.to_string(),
        edges: vec![
            edge(
                "wi_core",
                "wi_registration",
                "contract.workflow",
                "finalization_failure",
            ),
            edge(
                "wi_unrelated",
                "wi_service",
                "contract.unrelated",
                "unrelated_ready",
            ),
            edge(
                "wi_unrelated",
                "wi_panel",
                "contract.unrelated",
                "unrelated_ready",
            ),
            edge(
                "wi_service",
                "wi_panel",
                "contract.service",
                "service_ready",
            ),
        ],
        created_at: CREATED_AT.to_string(),
    }
}

fn n3_plan_projection(
    plan_revision: &WorkItemPlanRevision,
    dependency_graph: &DependencyGraphRevision,
    published: &[crate::product::workspace_engine::CompiledWorkItemRevision],
    story_spec_id: &str,
    design_spec_id: &str,
) -> PlanProjectionBundle {
    let ordered_logical_work_item_ids = ORDERED_LOGICAL_WORK_ITEM_IDS
        .iter()
        .map(|logical_id| (*logical_id).to_string())
        .collect::<Vec<_>>();
    let published_work_item = |logical_id: &str| {
        published
            .iter()
            .find(|item| item.work_item_revision.logical_work_item_id == logical_id)
            .expect("published work item projection")
    };
    let compiled_plan = CompiledPlanProjections {
        human: HumanGroupProjection {
            plan_id: plan_revision.plan_id.clone(),
            goal: "N3 plan coding fixture".to_string(),
            split_reason: "Fixture publishes five schema v2 revisions".to_string(),
            work_items: ordered_logical_work_item_ids
                .iter()
                .map(|logical_id| {
                    let item = published_work_item(logical_id);
                    HumanGroupWorkItemSummary {
                        logical_work_item_id: logical_id.clone(),
                        title: item.projection_bundle.human_projection.title.clone(),
                        goal: item.projection_bundle.human_projection.goal.clone(),
                        depends_on: dependency_graph
                            .edges
                            .iter()
                            .filter(|edge| edge.to == *logical_id)
                            .map(|edge| edge.from.clone())
                            .collect(),
                        provides: item
                            .projection_bundle
                            .human_projection
                            .outputs
                            .iter()
                            .map(|output| output.contract_id.clone())
                            .collect(),
                        scope_summary: item
                            .projection_bundle
                            .human_projection
                            .scope_summary
                            .clone(),
                    }
                })
                .collect(),
            contract_flow: Vec::new(),
            risks: Vec::new(),
            source_refs: vec![story_spec_id.to_string(), design_spec_id.to_string()],
            normative: false,
            used_by_provider: false,
        },
        coder: CoderGroupContext {
            plan_id: plan_revision.plan_id.clone(),
            ordered_logical_work_item_ids: ordered_logical_work_item_ids.clone(),
            dependency_edges: dependency_graph.edges.clone(),
            group_write_scopes: ordered_logical_work_item_ids
                .iter()
                .map(|logical_id| {
                    (
                        logical_id.clone(),
                        published_work_item(logical_id)
                            .projection_bundle
                            .coder_projection
                            .write_policy
                            .clone(),
                    )
                })
                .collect(),
        },
        reviewer: ReviewerGroupMatrix {
            plan_id: plan_revision.plan_id.clone(),
            work_items: ordered_logical_work_item_ids
                .iter()
                .map(|logical_id| ReviewerGroupMatrixEntry {
                    logical_work_item_id: logical_id.clone(),
                    criterion_refs: published_work_item(logical_id)
                        .projection_bundle
                        .reviewer_projection
                        .criterion_refs
                        .clone(),
                    input_contract_refs: Vec::new(),
                    output_contract_refs: Vec::new(),
                })
                .collect(),
            dependency_edges: dependency_graph.edges.clone(),
            design_traceability_refs: Vec::new(),
        },
    };
    let hashes = plan_projection_hashes(&compiled_plan).expect("projection hashes");
    PlanProjectionBundle {
        id: plan_revision.plan_projection_bundle_id.clone(),
        plan_revision_id: plan_revision.id.clone(),
        dependency_graph_revision_id: plan_revision.dependency_graph_revision_id.clone(),
        work_item_projection_bundle_refs: ordered_logical_work_item_ids
            .iter()
            .map(|logical_id| published_work_item(logical_id).projection_bundle.id.clone())
            .collect(),
        human_group_projection: compiled_plan.human,
        coder_group_context: compiled_plan.coder,
        reviewer_group_matrix: compiled_plan.reviewer,
        human_group_projection_hash: hashes.human,
        coder_group_context_hash: hashes.coder,
        reviewer_group_matrix_hash: hashes.reviewer,
        compiler_version: "plan-projection-compiler-v1".to_string(),
        created_at: CREATED_AT.to_string(),
    }
}

struct N3Fixture {
    _root: tempfile::TempDir,
    paths: ProductAppPaths,
    state: WebAppState,
    busi: LogicalRepositoryId,
    web: LogicalRepositoryId,
    gateway: LogicalRepositoryId,
    frontend: LogicalRepositoryId,
}

impl N3Fixture {
    fn coding_store(&self) -> CodingAttemptStore {
        CodingAttemptStore::new(self.paths.clone())
    }

    fn run_store(&self) -> CodingPlanRunStore {
        CodingPlanRunStore::new(self.paths.clone())
    }

    fn run_file(&self) -> std::path::PathBuf {
        self.paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("coding-plan-runs")
            .join(format!("{PLAN_ID}.json"))
    }

    fn restart_state(&self) -> WebAppState {
        WebAppState::new(
            self._root.path().to_path_buf(),
            WebRuntime::new_fake(self._root.path().to_path_buf()),
        )
    }

    fn target_attempt(
        &self,
        target: LogicalRepositoryId,
    ) -> crate::product::coding_models::CodingExecutionAttempt {
        self.coding_store()
            .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID, Some(target))
            .unwrap()
            .expect("target attempt")
    }

    fn busi_attempt(&self) -> crate::product::coding_models::CodingExecutionAttempt {
        self.target_attempt(self.busi)
    }

    fn busi_selection(&self, command_id: &str) -> CodingPlanSelection {
        let attempt = self.busi_attempt();
        let unit = self
            .coding_store()
            .list_coding_units(PROJECT_ID, ISSUE_ID, &attempt.id)
            .unwrap()
            .into_iter()
            .find(|unit| unit.logical_work_item_id == "wi_core")
            .expect("wi_core unit materialized in the busi attempt");
        CodingPlanSelection {
            unit_id: unit.id,
            logical_work_item_id: "wi_core".to_string(),
            attempt_id: attempt.id,
            target_repository_id: self.busi.0.to_string(),
            start_command_id: CodingPlanSelection::deterministic_start_command_id(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                command_id,
                &self.busi.0.to_string(),
            ),
        }
    }
}

async fn n3_fixture() -> N3Fixture {
    let root = tempfile::tempdir().unwrap();
    PlanRepairFixtureRuntime::seed(root.path(), PlanRepairFixtureControl::default())
        .await
        .expect("seed plan repair fixture");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_store = CodingAttemptStore::new(paths.clone());
    let seeded = coding_store
        .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID, None)
        .unwrap()
        .expect("seeded legacy group attempt");
    coding_store
        .delete_attempt(PROJECT_ID, ISSUE_ID, &seeded.id)
        .unwrap();

    // 四个真实 logical 仓（park-1 同形）。
    let mut targets = Vec::new();
    for name in ["n3-busi", "n3-web", "n3-gateway", "n3-frontend"] {
        let canonical_path = root.path().join(name);
        std::fs::create_dir_all(&canonical_path).unwrap();
        run_git(&canonical_path, &["init", "--quiet"]);
        run_git(
            &canonical_path,
            &["config", "user.email", "n3@example.test"],
        );
        run_git(&canonical_path, &["config", "user.name", "N3 Plan Coding"]);
        std::fs::write(canonical_path.join("README.md"), format!("# {name}\n")).unwrap();
        run_git(&canonical_path, &["add", "README.md"]);
        run_git(
            &canonical_path,
            &["commit", "--quiet", "-m", "initial commit"],
        );
        let repository = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        )
        .create(CreateRepositoryInput {
            project_id: PROJECT_ID.to_string(),
            name: name.to_string(),
            path: canonical_path,
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: format!("n3-plan-coding-{name}"),
        })
        .unwrap();
        targets.push(
            repository
                .logical_repository_id
                .expect("logical repository ID"),
        );
    }
    let [busi, web, gateway, frontend] = [targets[0], targets[1], targets[2], targets[3]];
    let manifest = LogicalCodebaseStore::new(paths.clone())
        .load_manifest(PROJECT_ID)
        .unwrap()
        .expect("manifest");
    AggregatePolicyArtifactStore::new(paths.clone())
        .ensure_bootstrap(&manifest)
        .unwrap();
    IssueCodebaseSelectionStore::new(paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            PROJECT_ID,
            ISSUE_ID,
            targets.clone(),
            Vec::new(),
            Vec::new(),
            None,
        ))
        .unwrap();

    // 发布 wi_service/wi_panel（补齐五 WI），重编五契约得到权威投影材料。
    let revision_store = WorkItemRevisionStore::new(paths.clone());
    let mut lineage = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap();
    let mut published = Vec::new();
    for contract in seeded_contracts() {
        let logical_id = contract.identity.logical_work_item_id.clone();
        let revision_id = format!("work_item_revision_{logical_id}_0001");
        let draft = WorkItemDraftRevision {
            id: format!("work_item_draft_revision_{logical_id}_0001"),
            logical_work_item_id: logical_id.clone(),
            revision_no: 1,
            supersedes: None,
            revision_reason: PlanRevisionReason::InitialCompile,
            canonical_contract_candidate: contract,
            trigger_repair_request_id: None,
            created_at: CREATED_AT.to_string(),
        };
        let compiled = compile_work_item_revision(
            &draft,
            &WorkItemProjectionCompiler,
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
        .expect("compile work item revision");
        if logical_id == "wi_service" || logical_id == "wi_panel" {
            let logical = LogicalWorkItem {
                id: logical_id.clone(),
                plan_id: PLAN_ID.to_string(),
                title: logical_id.clone(),
                active_revision_id: None,
                created_at: CREATED_AT.to_string(),
                updated_at: CREATED_AT.to_string(),
            };
            revision_store
                .put_logical_work_item(&lineage, &logical)
                .unwrap();
            revision_store
                .put_draft_revision(&lineage, &compiled.draft_revision)
                .unwrap();
            revision_store
                .put_verification_plan_revision(&lineage, &compiled.verification_plan_revision)
                .unwrap();
            revision_store
                .put_work_item_projection_bundle(&lineage, &compiled.projection_bundle)
                .unwrap();
            revision_store
                .put_work_item_revision(&lineage, &compiled.work_item_revision)
                .unwrap();
            revision_store
                .set_active_work_item_revision(&lineage, &logical, None, &revision_id)
                .unwrap();
        }
        published.push(compiled);
    }

    // plan_revision_0002：五 WI 绑定 + 扩展依赖图 + 权威投影。
    let dependency_graph = dependency_graph_0002();
    revision_store
        .put_dependency_graph_revision(&lineage, &dependency_graph)
        .unwrap();
    let mut bindings = std::collections::BTreeMap::new();
    for logical_id in ORDERED_LOGICAL_WORK_ITEM_IDS {
        bindings.insert(
            (*logical_id).to_string(),
            format!("work_item_revision_{logical_id}_0001"),
        );
    }
    let plan_revision = WorkItemPlanRevision {
        id: PLAN_REVISION_ID.to_string(),
        plan_id: PLAN_ID.to_string(),
        revision_no: 2,
        supersedes: Some("plan_revision_0001".to_string()),
        reason: PlanRevisionReason::SubgraphReplan,
        work_item_bindings: bindings,
        dependency_graph_revision_id: dependency_graph.id.clone(),
        validation_report_ref: "plan_validation_report_0002".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0002".to_string(),
        publication_provenance_ref: None,
        created_at: CREATED_AT.to_string(),
    };
    let projection = n3_plan_projection(
        &plan_revision,
        &dependency_graph,
        &published,
        "story_spec_0001",
        "design_spec_0001",
    );
    revision_store
        .put_plan_projection_bundle(&lineage, &projection)
        .unwrap();
    revision_store
        .put_plan_revision(&lineage, &plan_revision)
        .unwrap();
    lineage = revision_store
        .set_active_plan_revision(&lineage, PLAN_REVISION_ID)
        .unwrap();

    // per-unit target 草稿（与 N2 同法）：busi=wi_core、web=wi_registration、
    // gateway=wi_unrelated、frontend={wi_service, wi_panel}。
    let plan_store = WorkItemPlanStore::new(paths.clone());
    for (logical_id, target) in [
        ("wi_core", busi),
        ("wi_registration", web),
        ("wi_unrelated", gateway),
        ("wi_service", frontend),
        ("wi_panel", frontend),
    ] {
        let revision = revision_store
            .get_work_item_revision(
                &lineage,
                logical_id,
                &format!("work_item_revision_{logical_id}_0001"),
            )
            .unwrap();
        plan_store
            .put_draft_record(&WorkItemDraftRecord {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                plan_id: PLAN_ID.to_string(),
                draft_id: revision.source_draft_revision_id.clone(),
                outline_id: format!("outline_{logical_id}"),
                generation_round_id: "round_n3_plan_coding".to_string(),
                batch_id: None,
                attempt_index: 1,
                outline_version_ref: "outline_version_n3".to_string(),
                generation_mode: WorkItemGenerationMode::Serial,
                generation_diagnostics: None,
                candidate: WorkItemDraftCandidate {
                    target_repository_id: Some(target),
                    outline_id: format!("outline_{logical_id}"),
                    logical_work_item_id: logical_id.to_string(),
                    canonical_contract_candidate: revision.canonical_contract,
                    verification_plan: WorkItemDraftVerificationPlan { checks: Vec::new() },
                },
                status: WorkItemDraftStatus::Accepted,
                active: true,
                superseded_by_draft_id: None,
                supersede_reason: None,
                copied_from_draft_id: None,
                review_node_id: None,
                review_verdict_ref: None,
                generated_from_node_id: "n3_plan_coding_fixture".to_string(),
                accepted_at: Some(CREATED_AT.to_string()),
                superseded_at: None,
                created_at: CREATED_AT.to_string(),
                updated_at: CREATED_AT.to_string(),
            })
            .unwrap();
    }

    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    N3Fixture {
        _root: root,
        paths,
        state,
        busi,
        web,
        gateway,
        frontend,
    }
}

async fn post_start(
    state: &WebAppState,
    command_id: &str,
    expected_plan_revision_id: &str,
) -> Result<PlanCodingStartResultDto, crate::web::error::ApiError> {
    super::plan_start::post_plan_coding_start(
        axum::extract::State(state.clone()),
        axum::extract::Path((
            PROJECT_ID.to_string(),
            ISSUE_ID.to_string(),
            PLAN_ID.to_string(),
        )),
        axum::Json(PlanCodingStartRequest {
            command_id: command_id.to_string(),
            expected_plan_revision_id: expected_plan_revision_id.to_string(),
        }),
    )
    .await
    .map(|json| json.0)
}

async fn get_progress(
    state: &WebAppState,
) -> Result<PlanCodingProgressDto, crate::web::error::ApiError> {
    super::plan_start::get_plan_coding_progress(
        axum::extract::State(state.clone()),
        axum::extract::Path((
            PROJECT_ID.to_string(),
            ISSUE_ID.to_string(),
            PLAN_ID.to_string(),
        )),
    )
    .await
    .map(|json| json.0)
}

fn unit_dto<'a>(
    dto: &'a PlanCodingProgressDto,
    logical_id: &str,
) -> &'a crate::web::types::PlanCodingUnitProgressDto {
    dto.units
        .iter()
        .find(|unit| unit.logical_work_item_id == logical_id)
        .unwrap_or_else(|| panic!("unit dto for {logical_id} missing"))
}

/// REQ-CS-01：五 WI/四 target 一次点击——4 bindings durable、busi 首项
/// （全图稳定选择，不硬编码）、frontend 两 units、start barrier 恰一次且
/// 未 selected 零副作用、wi_panel 直接依赖集合完整。
#[tokio::test]
async fn plan_coding_n3_one_click_binds_all_and_starts_selected_once() {
    let fixture = n3_fixture().await;
    let result = post_start(&fixture.state, "cmd_n3_user_0001", PLAN_REVISION_ID)
        .await
        .expect("one-click plan coding start succeeds");

    assert_eq!(result.operation_state, OperationState::Accepted);
    assert_eq!(result.bound_plan_revision_id, PLAN_REVISION_ID);
    assert_eq!(result.command_id, "cmd_n3_user_0001");
    assert_eq!(result.target_attempts.len(), 4, "one attempt per target");

    let run_store = fixture.run_store();
    let run = run_store
        .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .expect("durable run");
    assert_eq!(run.phase, CodingPlanRunPhase::Started);
    assert_eq!(run.target_attempts.len(), 4);
    let selection = run.selection.clone().expect("frozen selection");
    assert_eq!(selection.logical_work_item_id, "wi_core");
    assert_eq!(selection.target_repository_id, fixture.busi.0.to_string());
    let expected_start_command = CodingPlanSelection::deterministic_start_command_id(
        PROJECT_ID,
        ISSUE_ID,
        PLAN_ID,
        "cmd_n3_user_0001",
        &fixture.busi.0.to_string(),
    );
    assert_eq!(selection.start_command_id, expected_start_command);
    assert!(matches!(
        run.start_outcome,
        Some(CodingPlanStartOutcome::Started { .. })
    ));

    // busi 首项 attempt 事实：claim 用冻结首启 command，barrier 恰一次。
    let busi_attempt = fixture.busi_attempt();
    assert_eq!(selection.attempt_id, busi_attempt.id);
    let claim = busi_attempt.start_claim.as_ref().expect("start claim");
    assert_eq!(claim.command_id, expected_start_command);
    assert!(matches!(
        claim.phase,
        CodingStartPhase::ProviderMayHaveStarted
    ));
    assert_eq!(
        fixture
            .state
            .coding_runs
            .runner_count(&CodingAttemptRunKey::from_attempt(&busi_attempt)),
        1,
        "selected attempt crossed the start barrier exactly once"
    );

    // 未 selected target：零 claim、零 runner。
    for target in [fixture.web, fixture.gateway, fixture.frontend] {
        let attempt = fixture.target_attempt(target);
        assert!(
            attempt.start_claim.is_none(),
            "unselected target not started"
        );
        assert_eq!(
            fixture
                .state
                .coding_runs
                .runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
            0
        );
        assert_eq!(attempt.status, CodingAttemptStatus::Created);
    }

    // frontend 桶含两 units（wi_service/wi_panel）。
    let frontend_attempt = fixture.target_attempt(fixture.frontend);
    let frontend_units = fixture
        .coding_store()
        .list_coding_units(PROJECT_ID, ISSUE_ID, &frontend_attempt.id)
        .unwrap();
    assert_eq!(frontend_units.len(), 2);

    // 聚合：五 units、wi_core 真跑、wi_panel 直接依赖完整等待。
    assert_eq!(result.progress.units.len(), 5);
    assert!(!result.progress.plan_complete);
    assert_eq!(
        result.primary_attempt_id.as_deref(),
        Some(busi_attempt.id.as_str())
    );
    assert_eq!(
        result.selected_unit_id.as_deref(),
        Some(selection.unit_id.as_str())
    );
    assert_eq!(
        result.selected_attempt_id.as_deref(),
        Some(busi_attempt.id.as_str())
    );

    let core = unit_dto(&result.progress, "wi_core");
    assert_eq!(core.execution_state, "running");
    assert_eq!(core.target_repository_id, fixture.busi.0.to_string());
    assert_eq!(core.attempt_id, busi_attempt.id);
    assert!(core.unsatisfied_dependency_ids.is_empty());

    let registration = unit_dto(&result.progress, "wi_registration");
    assert_eq!(registration.execution_state, "waiting_dependency");
    assert_eq!(
        registration.unsatisfied_dependency_ids,
        vec!["wi_core".to_string()]
    );
    assert_eq!(
        registration.waiting_reason.as_deref(),
        Some("dependency_not_ready")
    );

    let service = unit_dto(&result.progress, "wi_service");
    assert_eq!(service.execution_state, "waiting_dependency");
    assert_eq!(
        service.unsatisfied_dependency_ids,
        vec!["wi_unrelated".to_string()]
    );

    let panel = unit_dto(&result.progress, "wi_panel");
    assert_eq!(panel.execution_state, "waiting_dependency");
    assert_eq!(
        panel.unsatisfied_dependency_ids,
        vec!["wi_service".to_string(), "wi_unrelated".to_string()],
        "wi_panel must list its full direct dependency set"
    );
    assert_eq!(
        panel.waiting_reason.as_deref(),
        Some("dependency_not_ready")
    );
    assert_eq!(panel.target_repository_id, fixture.frontend.0.to_string());
    assert_eq!(panel.attempt_id, frontend_attempt.id);
}

/// REQ-CS-02：同 command 重放与服务重启（第二 command）都复用同一冻结
/// selection/首启 command——不重选、不重捕、不二次首启。
#[tokio::test]
async fn plan_coding_n3_replay_and_restart_keep_same_start_command() {
    let fixture = n3_fixture().await;
    let run_store = fixture.run_store();

    let first = post_start(&fixture.state, "cmd_n3_user_0001", PLAN_REVISION_ID)
        .await
        .expect("first one-click start");
    assert_eq!(first.operation_state, OperationState::Accepted);
    let first_run = run_store
        .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .unwrap();
    let first_selection = first_run.selection.clone().unwrap();

    // 同 command 重放：原 run、原 selection、原首启 command。
    let replay = post_start(&fixture.state, "cmd_n3_user_0001", PLAN_REVISION_ID)
        .await
        .expect("same-command replay");
    assert_eq!(replay.operation_state, OperationState::Replayed);
    assert_eq!(replay.command_id, "cmd_n3_user_0001");
    let replay_run = run_store
        .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(replay_run.command_id, first_run.command_id);
    assert_eq!(replay_run.created_at, first_run.created_at);
    assert_eq!(replay_run.selection.clone().unwrap(), first_selection);

    // 服务重启（新 registry/state）：第二 command 命中同一 active run，
    // AdvanceInput/回执沿 run.command_id（不是新的 request.command_id）。
    let restart_state = fixture.restart_state();
    let second = post_start(&restart_state, "cmd_n3_user_0002", PLAN_REVISION_ID)
        .await
        .expect("second command returns the existing active run");
    assert_eq!(second.operation_state, OperationState::Replayed);
    assert_eq!(second.command_id, "cmd_n3_user_0001");
    let second_run = run_store
        .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(second_run.command_id, "cmd_n3_user_0001");
    assert_eq!(second_run.created_at, first_run.created_at);
    assert_eq!(second_run.selection.clone().unwrap(), first_selection);

    // 单发首启事实不重复：claim 恰一次且仍为冻结首启 command。
    let busi_attempt = fixture.busi_attempt();
    let claim = busi_attempt
        .start_claim
        .as_ref()
        .expect("single start claim");
    assert_eq!(claim.command_id, first_selection.start_command_id);
    for target in [fixture.web, fixture.gateway, fixture.frontend] {
        assert!(fixture.target_attempt(target).start_claim.is_none());
    }
    // 重启进程内零新 runner（事实由 durable claim 承载）。
    assert_eq!(
        restart_state
            .coding_runs
            .runner_count(&CodingAttemptRunKey::from_attempt(&busi_attempt)),
        0
    );
}

/// REQ-CS-02/07：异 revision 拒绝不落 run；plan 删除准入不绕
/// entity_deleting；NeedsHuman 按事实返回且不再 start（零首启副作用）。
#[tokio::test]
async fn plan_coding_n3_revision_deleting_and_needs_human_fail_closed() {
    let fixture = n3_fixture().await;

    // 异 revision：拒绝且不写 run。
    let drift = post_start(&fixture.state, "cmd_n3_user_0001", "plan_revision_drift")
        .await
        .expect_err("revision drift must fail closed");
    assert_eq!(
        drift.details["kind"], "coding_plan_run_revision",
        "unexpected error: {drift:?}"
    );
    assert!(!fixture.run_file().exists());

    // 删除准入：plan scope 独占删除 lease 持有期间 POST/GET 都 entity_deleting。
    let scope = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);
    scope.validate().unwrap();
    let deletion_lease = EntityOperationStore::new(fixture.paths.clone())
        .try_acquire_delete(&scope)
        .expect("acquire deletion lease");
    let deleting_post = post_start(&fixture.state, "cmd_n3_user_0001", PLAN_REVISION_ID)
        .await
        .expect_err("deleting plan must fail closed");
    assert_eq!(deleting_post.details["kind"], "entity_deleting");
    let deleting_get = get_progress(&fixture.state)
        .await
        .expect_err("deleting plan GET must fail closed");
    assert_eq!(deleting_get.details["kind"], "entity_deleting");
    assert!(!fixture.run_file().exists());
    drop(deletion_lease);

    // NeedsHuman：既有 NeedsHuman run 按事实返回，不再 start。
    let fixture_needs_human = n3_fixture().await;
    let run_store = fixture_needs_human.run_store();
    let run = run_store
        .prepare(
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "cmd_n3_needs_human",
            PLAN_REVISION_ID,
        )
        .unwrap();
    let outcome = advance_plan(
        &fixture_needs_human.state,
        AdvanceInput {
            command_id: run.command_id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: PLAN_ID.to_string(),
        },
        AdvancePlanOrigin::Manual,
    )
    .await
    .unwrap();
    let AdvanceOutcome::Completed {
        target_attempts, ..
    } = outcome
    else {
        panic!("expected advance Completed, got {outcome:?}");
    };
    let selection = fixture_needs_human.busi_selection("cmd_n3_needs_human");
    let bound = run_store
        .bind_ready(&run, &target_attempts, Some(selection.clone()))
        .unwrap();
    let requested = run_store.mark_start_requested(&bound).unwrap();
    run_store
        .record_start_outcome(
            &requested,
            CodingPlanStartOutcome::NeedsHuman {
                attempt_id: selection.attempt_id.clone(),
                reason: "simulated unproven first-start side effects".to_string(),
            },
        )
        .unwrap();

    let result = post_start(
        &fixture_needs_human.state,
        "cmd_n3_needs_human",
        PLAN_REVISION_ID,
    )
    .await
    .expect("needs-human run is returned as-is");
    assert_eq!(result.operation_state, OperationState::NeedsHuman);
    assert_eq!(result.command_id, "cmd_n3_needs_human");
    assert!(matches!(
        result.start_outcome,
        Some(CodingPlanStartOutcome::NeedsHuman { .. })
    ));
    // 零首启副作用：无 claim、无 runner、run 不漂移。
    let busi_attempt = fixture_needs_human.busi_attempt();
    assert!(busi_attempt.start_claim.is_none());
    assert_eq!(
        fixture_needs_human
            .state
            .coding_runs
            .runner_count(&CodingAttemptRunKey::from_attempt(&busi_attempt)),
        0
    );
    let stored = run_store
        .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(stored.phase, CodingPlanRunPhase::NeedsHuman);
    assert_eq!(stored.command_id, "cmd_n3_needs_human");

    // 新 command 也不能用重试清除不明状态：仍返回原 run。
    let retry = post_start(&fixture_needs_human.state, "cmd_n3_retry", PLAN_REVISION_ID)
        .await
        .expect("retry returns the existing needs-human run");
    assert_eq!(retry.command_id, "cmd_n3_needs_human");
    assert_eq!(retry.operation_state, OperationState::NeedsHuman);
}

/// REQ-CS-06：GET 聚合——首次 GET 不写 run 且返 bound revision；冻结
/// target/桶 units 覆盖核对；初始化 Running 与 provider 执行分开；首项停点
/// 显示“下一项编排尚未启用”；primary 仅打开入口，不代表完成。
#[tokio::test]
async fn plan_coding_n3_get_aggregation_primary_is_not_completion() {
    let fixture = n3_fixture().await;

    // 首次 GET：不持久化、不启动，返回 bound revision 供 N6 互证。
    let first = get_progress(&fixture.state).await.expect("first GET");
    assert_eq!(first.bound_plan_revision_id, PLAN_REVISION_ID);
    assert_eq!(first.command_id, None);
    assert_eq!(first.mode, None);
    assert_eq!(first.phase, None);
    assert!(!first.plan_complete);
    assert!(!fixture.run_file().exists(), "GET must not persist a run");

    // durable 事实驱动（无后台 runner，确定性）：Manual advance → Ready 集 →
    // 冻结 selection → 首启 barrier 回执 → 首项 Completed → run Paused。
    let run_store = fixture.run_store();
    let coding_store = fixture.coding_store();
    let run = run_store
        .prepare(
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "cmd_n3_aggregation",
            PLAN_REVISION_ID,
        )
        .unwrap();
    let outcome = advance_plan(
        &fixture.state,
        AdvanceInput {
            command_id: run.command_id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: PLAN_ID.to_string(),
        },
        AdvancePlanOrigin::Manual,
    )
    .await
    .unwrap();
    let AdvanceOutcome::Completed {
        target_attempts, ..
    } = outcome
    else {
        panic!("expected advance Completed, got {outcome:?}");
    };
    assert_eq!(target_attempts.len(), 4);
    let selection = fixture.busi_selection("cmd_n3_aggregation");
    let bound = run_store
        .bind_ready(&run, &target_attempts, Some(selection.clone()))
        .unwrap();
    let requested = run_store.mark_start_requested(&bound).unwrap();
    let busi_attempt = fixture.busi_attempt();
    let crate::product::coding_attempt_store::ClaimCodingStartOutcome::Claimed(claimed) =
        coding_store
            .claim_coding_start(
                &busi_attempt,
                &selection.start_command_id,
                &CodingStartOrigin::Manual,
            )
            .unwrap()
    else {
        panic!("expected fresh claim");
    };
    coding_store
        .advance_coding_start_phase(
            &claimed,
            &selection.start_command_id,
            CodingStartPhase::RunnerRegistered,
        )
        .unwrap();
    coding_store
        .advance_coding_start_phase(
            &claimed,
            &selection.start_command_id,
            CodingStartPhase::ProviderMayHaveStarted,
        )
        .unwrap();
    let started = run_store
        .record_start_outcome(
            &requested,
            CodingPlanStartOutcome::Started {
                attempt_id: selection.attempt_id.clone(),
            },
        )
        .unwrap();
    // 首项完成事实 + 安全暂停（N5 运行期接线前的确定性聚合事实）。
    coding_store
        .update_coding_unit_status(
            PROJECT_ID,
            ISSUE_ID,
            &selection.attempt_id,
            &selection.unit_id,
            CodingExecutionUnitStatus::Completed,
            None,
        )
        .unwrap();
    let paused = run_store.mark_paused(&started).unwrap();
    assert_eq!(paused.phase, CodingPlanRunPhase::Paused);

    let progress = get_progress(&fixture.state).await.expect("aggregated GET");
    assert_eq!(progress.bound_plan_revision_id, PLAN_REVISION_ID);
    assert_eq!(progress.command_id.as_deref(), Some("cmd_n3_aggregation"));
    assert_eq!(progress.mode.as_deref(), Some("first_ready_unit"));
    assert_eq!(progress.phase.as_deref(), Some("paused"));
    assert_eq!(progress.target_attempts.len(), 4);
    assert!(
        !progress.plan_complete,
        "primary completion is not plan completion"
    );
    assert_eq!(
        progress.primary_attempt_id.as_deref(),
        Some(selection.attempt_id.as_str())
    );

    // 每仓 lease/worktree 沿 repo lock 事实：advance 绑定到 attempt → held。
    for target_dto in &progress.target_attempts {
        assert_eq!(target_dto.lease_state, "held");
        assert!(target_dto.worktree_path.is_some());
        assert_eq!(target_dto.attempt_status, CodingAttemptStatus::Created);
    }

    assert_eq!(progress.units.len(), 5);
    // wi_core：完成事实 + provider 首启回执 → completed（不是 plan 完成）。
    let core = unit_dto(&progress, "wi_core");
    assert_eq!(core.execution_state, "completed");
    assert_eq!(core.unit_status, CodingExecutionUnitStatus::Completed);
    // 依赖已满足的 wi_registration：首项停点 → 下一项编排尚未启用。
    let registration = unit_dto(&progress, "wi_registration");
    assert_eq!(registration.execution_state, "paused");
    assert!(registration.unsatisfied_dependency_ids.is_empty());
    assert_eq!(
        registration.waiting_reason.as_deref(),
        Some("plan_coding_continuation_not_enabled")
    );
    // 根 unit（未跑）同样被暂停挡住。
    let unrelated = unit_dto(&progress, "wi_unrelated");
    assert_eq!(unrelated.execution_state, "paused");
    assert_eq!(
        unrelated.waiting_reason.as_deref(),
        Some("plan_coding_continuation_not_enabled")
    );
    // 依赖未满足的 units 仍是等待依赖（不会被暂停文案掩盖）。
    let service = unit_dto(&progress, "wi_service");
    assert_eq!(service.execution_state, "waiting_dependency");
    assert_eq!(
        service.waiting_reason.as_deref(),
        Some("dependency_not_ready")
    );
    let panel = unit_dto(&progress, "wi_panel");
    assert_eq!(panel.execution_state, "waiting_dependency");
    assert_eq!(
        panel.unsatisfied_dependency_ids,
        vec!["wi_service".to_string(), "wi_unrelated".to_string()]
    );
}

/// REQ-CS-08：旧 group coding POST 的响应形状与准入结果不变——多 target
/// 仍按原 TargetAmbiguous 拒绝（单对象入口不扩权），新 coding-start 路由
/// 与之共存且聚合 GET 走新 DTO。
#[tokio::test]
async fn plan_coding_n3_old_post_shape_unchanged() {
    let fixture = n3_fixture().await;
    let router = crate::web::app::build_web_router(fixture.state.clone());

    // 旧 POST：多 target 计划仍被旧单对象入口拒绝（原准入零变化）。
    let old = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/work-item-plans/{PLAN_ID}/coding-attempts"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(old.status(), StatusCode::CONFLICT);
    let old_body = response_json(old).await;
    assert_eq!(
        old_body["code"], "repository_routing_ambiguous",
        "old POST admission must stay unchanged: {old_body}"
    );

    // 新路由共存：聚合 GET 走新 DTO（bound revision、非数组响应）。
    let new_get = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/work-item-plans/{PLAN_ID}/coding-start"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(new_get.status(), StatusCode::OK);
    let new_body = response_json(new_get).await;
    assert_eq!(new_body["bound_plan_revision_id"], PLAN_REVISION_ID);
    assert_eq!(new_body["plan_id"], PLAN_ID);
    assert!(new_body["target_attempts"].is_array());
}
