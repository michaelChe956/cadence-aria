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
    CodingAttemptStatus, CodingExecutionStage, CodingExecutionUnitStatus, CodingStartOrigin,
    CodingStartPhase,
};
use crate::product::coding_plan_run::{
    CodingPlanRun, CodingPlanRunPhase, CodingPlanRunStore, CodingPlanSelection,
    CodingPlanStartOutcome,
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

    // 受控 provider 进入（穿过真实 Coding 阶段 admission）：事故现场
    // Accepted／claim barrier／runner_count 三者全绿而 runner 实际死于
    // admission（兄弟仓预绑定锁被 issue 级租约分类误判 already-running，
    // 回 AwaitingManualRecovery）——故必须以 durable provider 进入事实
    // 为准：Coding 阶段 coder role run 存在，且期间不落人工恢复态。
    let coding_store = fixture.coding_store();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let attempt = coding_store
                .get_attempt(PROJECT_ID, ISSUE_ID, &busi_attempt.id)
                .expect("poll busi attempt");
            let coder_run = coding_store
                .latest_role_run(
                    PROJECT_ID,
                    ISSUE_ID,
                    &busi_attempt.id,
                    CodingExecutionStage::Coding,
                    crate::product::coding_models::CodingProviderRole::Coder,
                )
                .expect("poll coder role runs");
            if coder_run.is_some() {
                assert!(
                    attempt.stage.order() >= CodingExecutionStage::Coding.order(),
                    "provider entry must follow the Coding-stage admission: {:?}",
                    attempt.stage
                );
                return;
            }
            assert_ne!(
                attempt.status,
                CodingAttemptStatus::AwaitingManualRecovery,
                "首项 runner 不得死于 Coding 阶段 admission（多仓兄弟仓预绑定锁误杀现场）"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("selected attempt must cross the real Coding-stage admission and enter the provider");

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
    // S3（投影真实化）：POST 回执窗口 runner 已放行但 admission 尚未跨过
    //（attempt 仍 Created），provider_started 需 attempt.status==Running——
    // 回执投影 initialized，admission 跨过后的 GET 轮询转 running。
    assert_eq!(core.execution_state, "initialized");
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
// ─── S3（投影真实化，设计 DesignPushLocal2 Q3）：attempt 状态守卫 ───

/// 手工确定性构造 busi 首启场景（无 runner 竞态）：prepare → advance →
/// bind(selection=wi_core) → claim → 相位推进到 ProviderMayHaveStarted；
/// `executable=true` 时再走生产 admission CAS 到 Running。
async fn manually_provider_claimed_busi(
    fixture: &N3Fixture,
    command_id: &str,
    executable: bool,
) -> crate::product::coding_models::CodingExecutionAttempt {
    let run_store = fixture.run_store();
    let coding_store = fixture.coding_store();
    let run = run_store
        .prepare(PROJECT_ID, ISSUE_ID, PLAN_ID, command_id, PLAN_REVISION_ID)
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
    let selection = fixture.busi_selection(command_id);
    let bound = run_store
        .bind_ready(&run, &target_attempts, Some(selection.clone()))
        .unwrap();
    run_store.mark_start_requested(&bound).unwrap();
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
    for phase in [
        CodingStartPhase::RunnerRegistered,
        CodingStartPhase::ProviderMayHaveStarted,
    ] {
        coding_store
            .advance_coding_start_phase(&claimed, &selection.start_command_id, phase)
            .unwrap();
    }
    if executable {
        coding_store
            .admit_and_transition_attempt_to_executable(PROJECT_ID, ISSUE_ID, &busi_attempt.id)
            .unwrap();
    }
    fixture.busi_attempt()
}

/// S3 红绿判据：AMR 且 unit Running 且 claim≥ProviderMayHaveStarted →
/// `awaiting_manual_recovery`（waiting_reason 透传 manual_recovery_reason），
/// 不得再投影 `running`；同条件 Running 仍为 `running`。
#[tokio::test]
async fn plan_coding_s3_amr_attempt_projects_recovery_not_provider_running() {
    let fixture = n3_fixture().await;
    let attempt = manually_provider_claimed_busi(&fixture, "cmd_s3_amr", true).await;
    assert_eq!(attempt.status, CodingAttemptStatus::Running);

    // 同条件 Running 仍 running（守卫不误伤真实执行）。
    let progress = get_progress(&fixture.state).await.unwrap();
    assert_eq!(unit_dto(&progress, "wi_core").execution_state, "running");

    // 事故形态：runner 死亡转 AMR——unit 仍 Running、claim 仍 ≥Provider、
    // active_unit_id 对齐，旧投影会谎报 provider running。
    fixture
        .coding_store()
        .transition_to_awaiting_manual_recovery(&attempt.id, "coding_runner_failed_while_running")
        .unwrap();
    let progress = get_progress(&fixture.state).await.unwrap();
    let core = unit_dto(&progress, "wi_core");
    assert_eq!(core.execution_state, "awaiting_manual_recovery");
    assert_eq!(
        core.waiting_reason.as_deref(),
        Some("coding_runner_failed_while_running")
    );
    // unit 底层事实不被投影层改写（恢复复用 Running unit）。
    assert_eq!(core.unit_status, CodingExecutionUnitStatus::Running);
}

/// S3：claim≥Provider 但 admission 未跨（attempt 仍 Created）不构成 provider
/// running——POST 回执窗口/手工相位推进现场回落 initialized。
#[tokio::test]
async fn plan_coding_s3_created_claim_provider_window_is_not_provider_running() {
    let fixture = n3_fixture().await;
    let attempt = manually_provider_claimed_busi(&fixture, "cmd_s3_window", false).await;
    assert_eq!(attempt.status, CodingAttemptStatus::Created);
    let progress = get_progress(&fixture.state).await.unwrap();
    let core = unit_dto(&progress, "wi_core");
    assert_eq!(core.unit_status, CodingExecutionUnitStatus::Running);
    assert_eq!(
        core.execution_state, "initialized",
        "claim≥Provider 但 attempt 未 Running 不得投影 provider running"
    );
}

/// S3：attempt 级人控/终态屏幕——Blocked/WaitingForHuman → waiting_human，
/// Failed → failed，Aborted → aborted（unit 级事实不回滚，仅投影层屏幕）。
#[tokio::test]
async fn plan_coding_s3_attempt_statuses_screen_execution_state() {
    let cases: &[(&str, CodingAttemptStatus, &str)] = &[
        (
            "cmd_s3_wfh",
            CodingAttemptStatus::WaitingForHuman,
            "waiting_human",
        ),
        (
            "cmd_s3_blocked",
            CodingAttemptStatus::Blocked,
            "waiting_human",
        ),
        ("cmd_s3_failed", CodingAttemptStatus::Failed, "failed"),
        ("cmd_s3_aborted", CodingAttemptStatus::Aborted, "aborted"),
    ];
    for (command_id, status, expected) in cases {
        let fixture = n3_fixture().await;
        let attempt = manually_provider_claimed_busi(&fixture, command_id, true).await;
        fixture
            .coding_store()
            .update_attempt_status(PROJECT_ID, ISSUE_ID, &attempt.id, status.clone())
            .unwrap();
        let progress = get_progress(&fixture.state).await.unwrap();
        assert_eq!(
            unit_dto(&progress, "wi_core").execution_state,
            *expected,
            "attempt {status:?} must screen the unit projection"
        );
    }
}

/// S3（Q5 逐仓 PR 面板）：target 行九字段含 review request 数据——attempt
/// 面（branch/base/stage/head_commit/manual_recovery_reason）+ 最新
/// ReviewRequest 推送面（push_status 含 `skipped_not_configured`、
/// push_error、review_request_id）+ 仓名解析；未启 target 的无事实面为 None。
#[tokio::test]
async fn plan_coding_s3_target_attempt_rows_expose_repository_and_review_facts() {
    use crate::product::coding_models::{
        PushStatus, RemoteKind, ReviewRequest, ReviewRequestKind, ReviewRequestOwnerKind,
    };

    fn s3_review_request(
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        id: &str,
        push_status: PushStatus,
        push_error: Option<&str>,
        created_at: &str,
    ) -> ReviewRequest {
        ReviewRequest {
            id: id.to_string(),
            attempt_id: attempt.id.clone(),
            kind: ReviewRequestKind::GitBranchOnly,
            remote_kind: RemoteKind::GenericGit,
            remote: "origin".to_string(),
            base_branch: attempt.base_branch.clone(),
            branch_name: attempt.branch_name.clone(),
            commit_sha: "deadbeef0001".to_string(),
            push_status,
            external_url: None,
            manual_instructions: Vec::new(),
            push_error: push_error.map(str::to_string),
            owner_kind: ReviewRequestOwnerKind::Attempt,
            pointer_publication_id: None,
            revoked: false,
            created_at: created_at.to_string(),
            updated_at: created_at.to_string(),
        }
    }

    let fixture = n3_fixture().await;
    let coding_store = fixture.coding_store();
    let attempt = manually_provider_claimed_busi(&fixture, "cmd_s3_dto", true).await;

    // 推送窗口现场：ReviewRequest 阶段 + head_commit + 推送失败记录。
    let mut updated = coding_store
        .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
        .unwrap();
    updated.stage = CodingExecutionStage::ReviewRequest;
    updated.head_commit = Some("deadbeef0001".to_string());
    updated.review_request_id = Some("review_request_s3_0001".to_string());
    coding_store
        .update_attempt_non_status_fields(&updated)
        .unwrap();
    coding_store
        .save_review_request(
            &updated,
            &s3_review_request(
                &updated,
                "review_request_s3_0001",
                PushStatus::Failed,
                Some("remote push rejected"),
                CREATED_AT,
            ),
        )
        .unwrap();
    coding_store
        .transition_to_awaiting_manual_recovery(&attempt.id, "coding_runner_failed_while_running")
        .unwrap();

    let progress = get_progress(&fixture.state).await.unwrap();
    let busi_row = progress
        .target_attempts
        .iter()
        .find(|row| row.target_repository_id == fixture.busi.0.to_string())
        .unwrap();
    assert_eq!(busi_row.repository_name.as_deref(), Some("n3-busi"));
    assert_eq!(
        busi_row.branch_name.as_deref(),
        Some(attempt.branch_name.as_str())
    );
    assert_eq!(
        busi_row.base_branch.as_deref(),
        Some(attempt.base_branch.as_str())
    );
    assert_eq!(busi_row.stage.as_deref(), Some("review_request"));
    assert_eq!(busi_row.head_commit.as_deref(), Some("deadbeef0001"));
    assert_eq!(busi_row.push_status.as_deref(), Some("failed"));
    assert_eq!(busi_row.push_error.as_deref(), Some("remote push rejected"));
    assert_eq!(
        busi_row.review_request_id.as_deref(),
        Some("review_request_s3_0001")
    );
    assert_eq!(
        busi_row.manual_recovery_reason.as_deref(),
        Some("coding_runner_failed_while_running")
    );
    assert_eq!(
        busi_row.attempt_status,
        CodingAttemptStatus::AwaitingManualRecovery
    );

    // 最新 ReviewRequest 胜出 + skipped_not_configured 透传（S2a 值域）。
    let latest = coding_store
        .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
        .unwrap();
    coding_store
        .save_review_request(
            &latest,
            &s3_review_request(
                &latest,
                "review_request_s3_0002",
                PushStatus::SkippedNotConfigured,
                None,
                "2026-10-13T01:00:00Z",
            ),
        )
        .unwrap();
    let progress = get_progress(&fixture.state).await.unwrap();
    let busi_row = progress
        .target_attempts
        .iter()
        .find(|row| row.target_repository_id == fixture.busi.0.to_string())
        .unwrap();
    assert_eq!(
        busi_row.push_status.as_deref(),
        Some("skipped_not_configured")
    );
    assert_eq!(busi_row.push_error, None);
    assert_eq!(
        busi_row.review_request_id.as_deref(),
        Some("review_request_s3_0002")
    );

    // 未启 target：attempt 事实在（Created），无 review/恢复事实 → null 面。
    let web_row = progress
        .target_attempts
        .iter()
        .find(|row| row.target_repository_id == fixture.web.0.to_string())
        .unwrap();
    assert_eq!(web_row.repository_name.as_deref(), Some("n3-web"));
    assert!(web_row.branch_name.is_some());
    assert!(web_row.base_branch.is_some());
    assert_eq!(web_row.stage.as_deref(), Some("prepare_context"));
    assert_eq!(web_row.head_commit, None);
    assert_eq!(web_row.push_status, None);
    assert_eq!(web_row.push_error, None);
    assert_eq!(web_row.review_request_id, None);
    assert_eq!(web_row.manual_recovery_reason, None);
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
            Some("wi_core 首轮完成".to_string()),
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
    // Slice B（仪表盘跨仓依赖投影）：聚合 DTO 携带全量直接依赖、全图拓扑
    // 序与桶内摘要——前端才能以 plan 全图重建拓扑/依赖链（跨桶 WI 不再被
    // 桶内 session units 口径吞掉）。
    assert_eq!(core.dependency_logical_work_item_ids, Vec::<String>::new());
    assert_eq!(core.order_index, 0);
    assert_eq!(core.summary.as_deref(), Some("wi_core 首轮完成"));
    // wi_registration 在 web 桶，其上游 wi_core 在 busi 桶：全量依赖必须跨桶投影。
    assert_eq!(
        registration.dependency_logical_work_item_ids,
        vec!["wi_core".to_string()]
    );
    assert_eq!(registration.order_index, 1);
    assert_eq!(registration.summary, None);
    assert_eq!(
        unrelated.dependency_logical_work_item_ids,
        Vec::<String>::new()
    );
    assert_eq!(unrelated.order_index, 2);
    assert_eq!(
        service.dependency_logical_work_item_ids,
        vec!["wi_unrelated".to_string()]
    );
    assert_eq!(service.order_index, 3);
    assert_eq!(
        panel.dependency_logical_work_item_ids,
        vec!["wi_service".to_string(), "wi_unrelated".to_string()]
    );
    assert_eq!(panel.order_index, 4);
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

/// REQ-CS-04：claim 停在 Claimed 相位（尚未过 RunnerRegistered/
/// ProviderMayHaveStarted barrier）时，聚合不得把初始化 unit Running 当作
/// provider 执行——execution_state 回落 initialized（区分口径）。
#[tokio::test]
async fn plan_coding_n3_claimed_barrier_phase_is_not_provider_running() {
    let fixture = n3_fixture().await;
    let run_store = fixture.run_store();
    let coding_store = fixture.coding_store();
    let run = run_store
        .prepare(
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "cmd_n3_claimed",
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
    let selection = fixture.busi_selection("cmd_n3_claimed");
    let bound = run_store
        .bind_ready(&run, &target_attempts, Some(selection.clone()))
        .unwrap();
    run_store.mark_start_requested(&bound).unwrap();
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
    assert!(matches!(
        claimed.start_claim.as_ref().unwrap().phase,
        CodingStartPhase::Claimed
    ));

    let progress = get_progress(&fixture.state)
        .await
        .expect("GET aggregates the claimed window");
    assert_eq!(progress.phase.as_deref(), Some("start_requested"));
    let core = unit_dto(&progress, "wi_core");
    assert_eq!(core.unit_status, CodingExecutionUnitStatus::Running);
    assert_eq!(
        core.execution_state, "initialized",
        "claim below ProviderMayHaveStarted must not project provider running"
    );
}

/// REQ-CS-07：冻结集合成员被独立删除后，GET 聚合不再 500——该 target
/// 投影失效终态（Aborted）、成员 units 不再投影；run 失效（invalidated）
/// 后整体走失效文案，已完成事实保留。
#[tokio::test]
async fn plan_coding_n3_deleted_member_aggregates_invalidated_not_500() {
    let fixture = n3_fixture().await;
    let run_store = fixture.run_store();
    let coding_store = fixture.coding_store();
    let run = run_store
        .prepare(
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "cmd_n3_member_delete",
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
    let selection = fixture.busi_selection("cmd_n3_member_delete");
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
    for phase in [
        CodingStartPhase::RunnerRegistered,
        CodingStartPhase::ProviderMayHaveStarted,
    ] {
        coding_store
            .advance_coding_start_phase(&claimed, &selection.start_command_id, phase)
            .unwrap();
    }
    run_store
        .record_start_outcome(
            &requested,
            CodingPlanStartOutcome::Started {
                attempt_id: selection.attempt_id.clone(),
            },
        )
        .unwrap();

    // 删除 web 成员 attempt（N7 删除闭包接线前的窗口：run 仍 active）。
    let web_attempt = fixture.target_attempt(fixture.web);
    coding_store
        .delete_attempt(PROJECT_ID, ISSUE_ID, &web_attempt.id)
        .unwrap();
    let progress = get_progress(&fixture.state)
        .await
        .expect("member deletion must not 500 the aggregation");
    assert_eq!(progress.phase.as_deref(), Some("started"));
    // 冻结绑定集保留：该 target 行仍在，但投影失效终态。
    assert_eq!(progress.target_attempts.len(), 4);
    let web_row = progress
        .target_attempts
        .iter()
        .find(|row| row.target_repository_id == fixture.web.0.to_string())
        .unwrap();
    assert_eq!(web_row.attempt_status, CodingAttemptStatus::Aborted);
    assert!(web_row.worktree_path.is_none());
    // 已删成员桶的 units 不再投影（记录已删，不造 unit 猜测）。
    assert_eq!(progress.units.len(), 4);
    assert!(
        progress
            .units
            .iter()
            .all(|unit| unit.logical_work_item_id != "wi_registration")
    );
    // 未受影响成员的事实保持：busi 首项 claim≥Provider + unit Running，
    // 但 attempt 未跨 admission（手工相位推进现场 status 仍 Created）——
    // S3 守卫下回 initialized（同条件 Running 才是 running）。
    assert_eq!(
        unit_dto(&progress, "wi_core").execution_state,
        "initialized"
    );

    // 成员删除授权失效（N2 invalidate，N7 接线后的事实）→ 整体失效文案。
    run_store
        .invalidate_for_deleted_attempt(&web_attempt)
        .unwrap();
    let invalidated = get_progress(&fixture.state)
        .await
        .expect("invalidated run still aggregates");
    assert_eq!(invalidated.phase.as_deref(), Some("invalidated"));
    for unit in &invalidated.units {
        assert_eq!(
            unit.execution_state, "invalidated",
            "invalidated run must project its units as invalidated: {:?}",
            unit.logical_work_item_id
        );
        assert_eq!(unit.waiting_reason, None);
    }
    assert!(!invalidated.plan_complete);
}

// ─── H4（设计 2026-10-14 C/D 节）：组删端点与重启收敛 ───

use crate::product::advance_store::AdvanceStore;
use crate::product::coding_plan_run::{
    CODING_PLAN_WORKSPACE_LIFECYCLES_DIR, CodingPlanWorkspaceResetChecklist,
};
use crate::web::types::{PlanCodingWorkspaceDeletionQuery, PlanCodingWorkspaceDeletionResultDto};

async fn delete_workspaces(
    state: &WebAppState,
    command_id: &str,
    expected_generation: Option<u64>,
    confirmed: bool,
) -> Result<PlanCodingWorkspaceDeletionResultDto, crate::web::error::ApiError> {
    super::plan_start::delete_plan_coding_workspaces(
        axum::extract::State(state.clone()),
        axum::extract::Path((
            PROJECT_ID.to_string(),
            ISSUE_ID.to_string(),
            PLAN_ID.to_string(),
        )),
        axum::extract::Query(PlanCodingWorkspaceDeletionQuery {
            command_id: command_id.to_string(),
            expected_generation,
            confirmed: Some(confirmed),
        }),
    )
    .await
    .map(|json| json.0)
}

fn h4_member_ids(fixture: &N3Fixture) -> Vec<String> {
    let mut ids: Vec<String> = fixture
        .coding_store()
        .list_attempts_for_plan(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .into_iter()
        .map(|attempt| attempt.id)
        .collect();
    ids.sort();
    ids
}

/// store 级成员删除的测试补面：复刻 N7 端点路径的仓维 shared worktree 条件
/// 清理（owner 属已删成员且无其他活动 attempt → 删记录）。真实四删走端点
/// 时由 `cleanup_repo_shared_worktree_if_no_active_attempts` 完成；本捷径
/// 绕过端点时按同一语义手工收敛，避免孤儿锁阻塞新轮 advance。
fn h4_release_dead_member_worktrees(fixture: &N3Fixture, dead_ids: &[String]) {
    let lifecycle = LifecycleStore::new(fixture.paths.clone());
    for repository in lifecycle
        .list_repo_shared_worktrees(PROJECT_ID, ISSUE_ID)
        .unwrap()
    {
        let Some(record) = lifecycle
            .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, repository)
            .unwrap()
        else {
            continue;
        };
        if record
            .current_lock_owner_id
            .as_deref()
            .is_some_and(|owner| dead_ids.iter().any(|id| id == owner))
        {
            lifecycle
                .delete_repo_shared_worktree(PROJECT_ID, ISSUE_ID, repository)
                .unwrap();
        }
    }
}

/// H4 端到端（验收矩阵 1）：四仓一键首启 → GET lifecycle 事实 → 未确认 409
/// → 确认整组删除（run/advance/journal 清、Plan/WI/revisions 保留、
/// generation 2）→ 同 command 重试 Replayed → 新 command 开新轮（新 attempt
/// IDs、首启回执 Started）→ 晚到旧 DELETE/过期 generation 不删新轮。
#[tokio::test]
async fn plan_coding_h4_group_delete_end_to_end_then_new_round() {
    let fixture = n3_fixture().await;
    let started = post_start(&fixture.state, "cmd_h4_round1", PLAN_REVISION_ID)
        .await
        .expect("round 1 one-click start");
    assert!(matches!(
        started.start_outcome,
        Some(CodingPlanStartOutcome::Started { .. })
    ));
    let round1_ids = h4_member_ids(&fixture);
    assert_eq!(round1_ids.len(), 4);

    // GET 事实投影：generation 1、四成员齐全、complete。
    let progress = get_progress(&fixture.state).await.unwrap();
    let lifecycle = progress.workspace_lifecycle;
    assert_eq!(lifecycle.generation, 1);
    assert_eq!(lifecycle.existing_member_count, 4);
    assert_eq!(lifecycle.missing_member_count, 0);
    assert_eq!(lifecycle.state, "complete");

    // 未确认：稳定 409 + 强警告文案（首启事实在案）。
    let unconfirmed =
        delete_workspaces(&fixture.state, "plan_delete_ui_h4_1", Some(1), false).await;
    let error = unconfirmed.expect_err("unconfirmed group delete must be rejected");
    assert_eq!(error.code, "deletion_confirmation_required");
    assert!(
        error.message.contains("终止并删除"),
        "strong confirmation message is authoritative: {}",
        error.message
    );
    // 拒绝路径零变更。
    assert_eq!(h4_member_ids(&fixture), round1_ids);

    // 确认删除：四成员清、轮次关闭（generation 2）、回执四字段。
    let receipt = delete_workspaces(&fixture.state, "plan_delete_ui_h4_1", Some(1), true)
        .await
        .expect("confirmed group delete");
    assert_eq!(receipt.operation_state, "accepted");
    assert_eq!(receipt.generation, 2);
    assert_eq!(receipt.deleted_attempt_ids, round1_ids);
    assert!(receipt.remaining_attempt_ids.is_empty());
    assert_eq!(receipt.workspace_lifecycle.generation, 2);
    assert_eq!(receipt.workspace_lifecycle.existing_member_count, 0);
    assert_eq!(receipt.workspace_lifecycle.state, "clean");
    assert!(h4_member_ids(&fixture).is_empty());
    assert!(!fixture.run_file().exists(), "closure purges the run");
    let artifacts = AdvanceStore::new(fixture.paths.clone())
        .list_plan_artifacts(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap();
    assert!(artifacts.advance_ids.is_empty() && artifacts.retry_command_ids.is_empty());
    assert!(
        fixture
            .coding_store()
            .list_group_initialization_journals_for_plan(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap()
            .is_empty(),
        "closure purges group initialization journals"
    );
    // 保留面：Plan/WI/revisions 不删（闭包只清执行闭包）。
    let lineage = WorkItemRevisionStore::new(fixture.paths.clone())
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap();
    assert_eq!(
        lineage.active_revision_id.as_deref(),
        Some(PLAN_REVISION_ID)
    );
    assert!(
        LifecycleStore::new(fixture.paths.clone())
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .is_ok(),
        "plan record must survive workspace deletion"
    );

    // 同 command 幂等重放：Replayed、零删除、不清新轮。
    let replay = delete_workspaces(&fixture.state, "plan_delete_ui_h4_1", None, true)
        .await
        .expect("same-command retry replays");
    assert_eq!(replay.operation_state, "replayed");
    assert!(replay.deleted_attempt_ids.is_empty());

    // 新 command 开新轮：四个新 attempt IDs（不复用已删 ID）、Started 恰一次。
    let round2 = post_start(&fixture.state, "cmd_h4_round2", PLAN_REVISION_ID)
        .await
        .expect("new round start after full deletion");
    assert!(matches!(
        round2.start_outcome,
        Some(CodingPlanStartOutcome::Started { .. })
    ));
    let round2_ids = h4_member_ids(&fixture);
    assert_eq!(round2_ids.len(), 4);
    assert!(
        round2_ids.iter().all(|id| !round1_ids.contains(id)),
        "new round must not reuse deleted attempt ids"
    );
    let progress2 = get_progress(&fixture.state).await.unwrap();
    assert_eq!(progress2.workspace_lifecycle.generation, 2);
    assert_eq!(progress2.workspace_lifecycle.existing_member_count, 4);
    assert_eq!(progress2.workspace_lifecycle.state, "complete");
    assert_eq!(progress2.command_id.as_deref(), Some("cmd_h4_round2"));

    // 晚到旧 DELETE（同 command 回执、回执轮次==当前轮次）：Replayed 不扫
    // 下一代；remaining 如实报当前事实（新轮 4 成员在场）。
    let late_same = delete_workspaces(&fixture.state, "plan_delete_ui_h4_1", None, true)
        .await
        .expect("late same-command delete replays");
    assert_eq!(late_same.operation_state, "replayed");
    assert_eq!(late_same.generation, 2);
    assert!(late_same.deleted_attempt_ids.is_empty());
    assert_eq!(late_same.remaining_attempt_ids, round2_ids);
    assert_eq!(h4_member_ids(&fixture), round2_ids);

    // 过期 generation（新 command）：typed 409，零删除。
    let stale = delete_workspaces(&fixture.state, "plan_delete_ui_h4_2", Some(1), true).await;
    let error = stale.expect_err("stale generation must be rejected");
    assert_eq!(error.code, "coding_plan_workspace_generation_conflict");
    assert_eq!(h4_member_ids(&fixture), round2_ids);

    // 更旧回执不判重放（回执轮次≠当前轮次）：round2 再整删（generation 3）
    // 后，h4_1 的回执（to=2）过期——不按旧 command 重放，按当前状态走分类
    // 路径（此刻零成员零残留→幂等成功 accepted）。
    let round3 = delete_workspaces(&fixture.state, "plan_delete_ui_h4_3", None, true)
        .await
        .expect("delete round 2 workspaces");
    assert_eq!(round3.generation, 3);
    assert_eq!(round3.deleted_attempt_ids, round2_ids);
    let stale_receipt = delete_workspaces(&fixture.state, "plan_delete_ui_h4_1", None, true)
        .await
        .expect("stale-receipt command falls through to classification");
    assert_eq!(stale_receipt.operation_state, "accepted");
    assert!(stale_receipt.deleted_attempt_ids.is_empty());
    assert!(stale_receipt.remaining_attempt_ids.is_empty());
    assert_eq!(stale_receipt.generation, 3);

    // P1（设计 C「confirmed=true 不是跨 generation 授权」）：陈旧回执的旧
    // command 在新轮成员在场时不得整删新轮——开 round4（4 成员）后，h4_1
    // 的旧回执（to=2<当前 3）+expected_generation 缺省+confirmed=true 必须
    // 409 拒绝（与 expected_generation 互证同码），新轮成员保留。
    post_start(&fixture.state, "cmd_h4_round4", PLAN_REVISION_ID)
        .await
        .expect("round 4 start");
    let round4_ids = h4_member_ids(&fixture);
    assert_eq!(round4_ids.len(), 4);
    let cross_generation =
        delete_workspaces(&fixture.state, "plan_delete_ui_h4_1", None, true).await;
    let error =
        cross_generation.expect_err("stale receipt must not authorize cross-generation delete");
    assert_eq!(error.code, "coding_plan_workspace_generation_conflict");
    assert_eq!(
        h4_member_ids(&fixture),
        round4_ids,
        "new round must survive"
    );
}

/// H4 部分删除（验收矩阵 2）：单成员删除 → GET 投影 incomplete/缺失计数 →
/// 清剩余入口整组收口（deleted=剩余成员、generation 切换）。
#[tokio::test]
async fn plan_coding_h4_partial_delete_projection_and_remaining_cleanup() {
    let fixture = n3_fixture().await;
    post_start(&fixture.state, "cmd_h4_partial", PLAN_REVISION_ID)
        .await
        .expect("start for partial deletion");
    let all_ids = h4_member_ids(&fixture);

    // v1 单删语义（N7）：成员删除即整 run 失效。
    let web_attempt = fixture.target_attempt(fixture.web);
    fixture
        .coding_store()
        .delete_attempt(PROJECT_ID, ISSUE_ID, &web_attempt.id)
        .unwrap();
    fixture
        .run_store()
        .invalidate_for_deleted_attempt(&web_attempt)
        .unwrap();

    let progress = get_progress(&fixture.state).await.unwrap();
    assert_eq!(progress.workspace_lifecycle.existing_member_count, 3);
    assert_eq!(progress.workspace_lifecycle.missing_member_count, 1);
    assert_eq!(progress.workspace_lifecycle.state, "incomplete");
    assert_eq!(progress.phase.as_deref(), Some("invalidated"));

    // 清剩余：整组删除收口（Invalidated run 一并退役清空）。
    let receipt = delete_workspaces(&fixture.state, "plan_delete_ui_h4_partial", None, true)
        .await
        .expect("delete remaining workspaces");
    let mut expected_deleted: Vec<String> = all_ids
        .into_iter()
        .filter(|id| id != &web_attempt.id)
        .collect();
    expected_deleted.sort();
    assert_eq!(receipt.deleted_attempt_ids, expected_deleted);
    assert_eq!(receipt.generation, 2);
    assert!(h4_member_ids(&fixture).is_empty());
    assert!(!fixture.run_file().exists());

    // 清剩余后可重启（验收矩阵 2 出口）。
    let restarted = post_start(&fixture.state, "cmd_h4_partial_round2", PLAN_REVISION_ID)
        .await
        .expect("restart after clearing remaining");
    assert!(matches!(
        restarted.start_outcome,
        Some(CodingPlanStartOutcome::Started { .. })
    ));
    assert_eq!(h4_member_ids(&fixture).len(), 4);
}

/// H4 半残现场收敛（验收矩阵 3 事故形态）：旧 Ready/外层 journal 指向已亡
/// 全集 + 旧 run 仍盘 → 开始动作内部走 H2a 零成员闭包完整重置后按最新
/// Confirmed revision 新建；不复活旧 attempt ID，无手工修数据。
#[tokio::test]
async fn plan_coding_h4_start_converges_half_dead_scene() {
    let fixture = n3_fixture().await;
    post_start(&fixture.state, "cmd_h4_incident_1", PLAN_REVISION_ID)
        .await
        .expect("incident round start");
    let dead_ids = h4_member_ids(&fixture);
    // 事故形态：四次删除走了旧路径（store 级删除 + N7 失效），旧
    // run/Ready/外层 journal 全部残留。
    let run_store = fixture.run_store();
    for attempt_id in &dead_ids {
        let attempt = fixture
            .coding_store()
            .get_attempt(PROJECT_ID, ISSUE_ID, attempt_id)
            .unwrap();
        fixture
            .coding_store()
            .delete_attempt(PROJECT_ID, ISSUE_ID, attempt_id)
            .unwrap();
        run_store.invalidate_for_deleted_attempt(&attempt).unwrap();
    }
    assert!(h4_member_ids(&fixture).is_empty());
    assert!(fixture.run_file().exists(), "old run residue stays on disk");
    let residue = AdvanceStore::new(fixture.paths.clone())
        .list_plan_artifacts(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap();
    assert!(!residue.advance_ids.is_empty(), "old Ready record residue");

    // 半残投影：零成员 + 缺失全集 → incomplete。
    let progress = get_progress(&fixture.state).await.unwrap();
    assert_eq!(progress.workspace_lifecycle.existing_member_count, 0);
    assert_eq!(progress.workspace_lifecycle.missing_member_count, 4);
    assert_eq!(progress.workspace_lifecycle.state, "incomplete");

    // 开始动作自动收敛：旧轮完整关闭 + 新轮正常新建。
    let converged = post_start(&fixture.state, "cmd_h4_incident_2", PLAN_REVISION_ID)
        .await
        .expect("start converges the half-dead scene");
    assert!(matches!(
        converged.start_outcome,
        Some(CodingPlanStartOutcome::Started { .. })
    ));
    let run = run_store
        .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .expect("new round run");
    assert_eq!(run.command_id, "cmd_h4_incident_2");
    let new_ids = h4_member_ids(&fixture);
    assert_eq!(new_ids.len(), 4);
    assert!(
        new_ids.iter().all(|id| !dead_ids.contains(id)),
        "dead attempt ids must not be revived"
    );
    // 旧执行引用不指向已亡 attempts：新轮 advance 产物对新增成员 Intact。
    let references = AdvanceStore::new(fixture.paths.clone())
        .plan_execution_references(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap();
    assert_eq!(
        references.classify_stale_references(&new_ids),
        crate::product::advance_store::AdvanceStaleReferenceVerdict::Intact,
        "old stale references must be purged; new round references stay intact"
    );
    let lifecycle = run_store
        .get_workspace_lifecycle(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .expect("lifecycle metadata written by convergence");
    assert_eq!(lifecycle.generation, 2);
    assert!(lifecycle.clearing.is_none());

    // 同 command 重试：N2 重放语义（返回同 run，零二次 provider）。
    let replay = post_start(&fixture.state, "cmd_h4_incident_2", PLAN_REVISION_ID)
        .await
        .expect("same-command retry replays the new round");
    assert_eq!(replay.command_id, "cmd_h4_incident_2");
    assert_eq!(h4_member_ids(&fixture), new_ids);
}

/// H4 同 command 承接（设计 C 行135）：现场恰有请求 command 的空 Prepared
/// 前缀 run + 旧 Ready 指向已亡全集 → 先清前缀再闭包，同动作不退役请求
/// command 自拒；新轮以该 command 正常落位。
#[tokio::test]
async fn plan_coding_h4_start_carries_same_command_prepared_prefix() {
    let fixture = n3_fixture().await;
    post_start(&fixture.state, "cmd_h4_carry_seed", PLAN_REVISION_ID)
        .await
        .expect("seed round");
    let dead_ids = h4_member_ids(&fixture);
    let run_store = fixture.run_store();
    for attempt_id in &dead_ids {
        let attempt = fixture
            .coding_store()
            .get_attempt(PROJECT_ID, ISSUE_ID, attempt_id)
            .unwrap();
        fixture
            .coding_store()
            .delete_attempt(PROJECT_ID, ISSUE_ID, attempt_id)
            .unwrap();
        run_store.invalidate_for_deleted_attempt(&attempt).unwrap();
    }
    // 真实四删走端点时的仓维 worktree 条件清理（闭包收口前 run 已被前缀
    // 承接清走，需按端点同一语义先释放孤儿锁）。
    h4_release_dead_member_worktrees(&fixture, &dead_ids);
    // 手工手术形态：旧 run 被移走，留下请求 command 的空 Prepared 前缀。
    run_store.purge_plan(PROJECT_ID, ISSUE_ID, PLAN_ID).unwrap();
    run_store
        .prepare(
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "cmd_h4_carry",
            PLAN_REVISION_ID,
        )
        .unwrap();

    let carried = post_start(&fixture.state, "cmd_h4_carry", PLAN_REVISION_ID)
        .await
        .expect("same command is carried into the new round");
    assert_eq!(carried.command_id, "cmd_h4_carry");
    assert!(matches!(
        carried.start_outcome,
        Some(CodingPlanStartOutcome::Started { .. })
    ));
    let run = run_store
        .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .expect("carried run");
    assert_eq!(run.command_id, "cmd_h4_carry");
    let new_ids = h4_member_ids(&fixture);
    assert_eq!(new_ids.len(), 4);
    assert!(new_ids.iter().all(|id| !dead_ids.contains(id)));
}

/// H4 clearing 续清（H2a 复审硬约束）：清理中断态下，开始入口与组删入口
/// 都必须以记录身份 `clearing.command_id` 续清（同 command 续清/异 command
/// 拒），完成后才允许新轮；pending 冻结清单是回执 deleted 集。
#[tokio::test]
async fn plan_coding_h4_clearing_resume_from_recorded_command() {
    let fixture = n3_fixture().await;
    post_start(&fixture.state, "cmd_h4_clearing_seed", PLAN_REVISION_ID)
        .await
        .expect("seed round");
    let member_ids = h4_member_ids(&fixture);
    let run_store = fixture.run_store();
    // 钩子中断形态：成员已删（teardown 完成）、闭包冻结 clearing 后中断。
    for attempt_id in &member_ids {
        let attempt = fixture
            .coding_store()
            .get_attempt(PROJECT_ID, ISSUE_ID, attempt_id)
            .unwrap();
        fixture
            .coding_store()
            .delete_attempt(PROJECT_ID, ISSUE_ID, attempt_id)
            .unwrap();
        run_store.invalidate_for_deleted_attempt(&attempt).unwrap();
    }
    let scope = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);
    let lease = EntityOperationStore::new(fixture.paths.clone())
        .try_acquire_delete(&scope)
        .unwrap();
    run_store
        .retire_workspace_round_with_lease(
            &lease,
            &CodingPlanWorkspaceResetChecklist {
                command_id: "plan_workspace_close_interrupted".to_string(),
                retired_command_ids: vec!["cmd_h4_clearing_seed".to_string()],
                retired_attempt_ids: member_ids.clone(),
                retired_advance_ids: Vec::new(),
                pending_attempt_ids: member_ids.clone(),
                pending_advance_ids: Vec::new(),
                pending_retry_command_ids: Vec::new(),
                pending_target_repository_ids: Vec::new(),
            },
        )
        .unwrap();
    drop(lease);

    // clearing 投影。
    let progress = get_progress(&fixture.state).await.unwrap();
    assert_eq!(progress.workspace_lifecycle.state, "clearing");

    // 组删入口续清：以 clearing.command_id 记录身份，回执 deleted=pending。
    let receipt = delete_workspaces(&fixture.state, "plan_delete_ui_h4_resume", None, true)
        .await
        .expect("delete action resumes the interrupted clearing");
    assert_eq!(receipt.operation_state, "accepted");
    assert_eq!(receipt.deleted_attempt_ids, member_ids);
    assert_eq!(receipt.generation, 2);
    let lifecycle = run_store
        .get_workspace_lifecycle(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(lifecycle.generation, 2);
    assert!(lifecycle.clearing.is_none());

    // 新轮正常。
    let restarted = post_start(&fixture.state, "cmd_h4_clearing_round2", PLAN_REVISION_ID)
        .await
        .expect("start after clearing resume");
    assert!(matches!(
        restarted.start_outcome,
        Some(CodingPlanStartOutcome::Started { .. })
    ));
    assert_eq!(h4_member_ids(&fixture).len(), 4);
}

/// H4 clearing 续清（开始入口面）：新开始动作在 clearing 在案时先以记录
/// command 续清，再正常新建——钩子中断态的可达续清出口（H2a 硬约束）。
#[tokio::test]
async fn plan_coding_h4_start_resumes_clearing_before_new_round() {
    let fixture = n3_fixture().await;
    post_start(&fixture.state, "cmd_h4_start_resume_seed", PLAN_REVISION_ID)
        .await
        .expect("seed round");
    let member_ids = h4_member_ids(&fixture);
    let run_store = fixture.run_store();
    for attempt_id in &member_ids {
        let attempt = fixture
            .coding_store()
            .get_attempt(PROJECT_ID, ISSUE_ID, attempt_id)
            .unwrap();
        fixture
            .coding_store()
            .delete_attempt(PROJECT_ID, ISSUE_ID, attempt_id)
            .unwrap();
        run_store.invalidate_for_deleted_attempt(&attempt).unwrap();
    }
    let scope = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);
    let lease = EntityOperationStore::new(fixture.paths.clone())
        .try_acquire_delete(&scope)
        .unwrap();
    run_store
        .retire_workspace_round_with_lease(
            &lease,
            &CodingPlanWorkspaceResetChecklist {
                command_id: "plan_workspace_close_via_start".to_string(),
                retired_command_ids: vec!["cmd_h4_start_resume_seed".to_string()],
                retired_attempt_ids: member_ids.clone(),
                retired_advance_ids: Vec::new(),
                pending_attempt_ids: member_ids.clone(),
                pending_advance_ids: Vec::new(),
                pending_retry_command_ids: Vec::new(),
                pending_target_repository_ids: Vec::new(),
            },
        )
        .unwrap();
    drop(lease);

    let started = post_start(&fixture.state, "cmd_h4_start_resume_new", PLAN_REVISION_ID)
        .await
        .expect("start resumes clearing then builds the new round");
    assert!(matches!(
        started.start_outcome,
        Some(CodingPlanStartOutcome::Started { .. })
    ));
    let lifecycle = run_store
        .get_workspace_lifecycle(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(lifecycle.generation, 2);
    assert!(lifecycle.clearing.is_none());
    let new_ids = h4_member_ids(&fixture);
    assert_eq!(new_ids.len(), 4);
    assert!(new_ids.iter().all(|id| !member_ids.contains(id)));
}

// ===== S2a 续跑全自动化（owner 2026-10-14 直批）：unit 完成/失败 → 全局
// 重估就绪集（就绪=直接依赖全部 Completed 且未启动）→ 就绪集>1 并行
// start；失败链停、独立链继续；幂等重试；legacy Paused run 由入口 resume
// 不再 409 not enabled。种子不走 HTTP 首启（零 live runner 消除竞态），
// 直接落「首启已 Started」的 durable 事实集。 =====

/// S2 种子：Manual advance 到 durable Ready → run 冻结（selection=wi_core，
/// busi 桶）→ start_requested → Started 回执。busi attempt 保持 Created
///（claim/admission 不影响续跑面；驱动只按 unit 完成事实重估）。
async fn s2_seed_first_start_started(fixture: &N3Fixture, command_id: &str) -> CodingPlanRun {
    use crate::product::advance_store::AdvanceOutcome;
    let outcome = advance_plan(
        &fixture.state,
        AdvanceInput {
            command_id: command_id.to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: PLAN_ID.to_string(),
        },
        AdvancePlanOrigin::Manual,
    )
    .await
    .expect("manual advance to durable Ready");
    let AdvanceOutcome::Completed {
        target_attempts, ..
    } = outcome
    else {
        panic!("s2 seed advance must complete with full target bindings");
    };
    let mut bindings = target_attempts;
    bindings.sort_by(|left, right| {
        left.target_repository_id
            .cmp(&right.target_repository_id)
            .then_with(|| left.attempt_id.cmp(&right.attempt_id))
    });
    assert_eq!(bindings.len(), 4, "four per-target attempts bound");

    let run_store = fixture.run_store();
    let run = run_store
        .prepare(PROJECT_ID, ISSUE_ID, PLAN_ID, command_id, PLAN_REVISION_ID)
        .expect("prepare run");
    let selection = fixture.busi_selection(command_id);
    let bound = run_store
        .bind_ready(&run, &bindings, Some(selection))
        .expect("bind ready set with frozen selection");
    let requested = run_store
        .mark_start_requested(&bound)
        .expect("mark start requested");
    run_store
        .record_start_outcome(
            &requested,
            CodingPlanStartOutcome::Started {
                attempt_id: bound
                    .selection
                    .as_ref()
                    .expect("frozen selection")
                    .attempt_id
                    .clone(),
            },
        )
        .expect("record started outcome")
}

/// 首启 unit（wi_core）落终态事实：unit Completed/Failed（attempt 指针随
/// store 同步清空）。完成证据=unit 状态（与 select_first_ready_unit 同源）。
fn s2_finish_first_unit(fixture: &N3Fixture, status: CodingExecutionUnitStatus) {
    let attempt = fixture.busi_attempt();
    let unit = fixture
        .coding_store()
        .list_coding_units(PROJECT_ID, ISSUE_ID, &attempt.id)
        .unwrap()
        .into_iter()
        .find(|unit| unit.logical_work_item_id == "wi_core")
        .expect("wi_core unit materialized");
    fixture
        .coding_store()
        .update_coding_unit_status(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            &unit.id,
            status,
            Some("s2 first unit finished".to_string()),
        )
        .expect("finish first unit");
}

/// ①：wi_core 完成后新点击 → 就绪集 {wi_registration, wi_unrelated} 并行
/// start（各自冻结确定性 command、barrier 恰一次），未就绪 frontend 不动，
/// selection 轮换到本轮确定性首个新启 unit，run 保持 Started。
#[tokio::test]
async fn plan_coding_s2_continuation_starts_all_ready_units_in_parallel() {
    let fixture = n3_fixture().await;
    let run_command_id = "cmd_n3_s2_0001";
    s2_seed_first_start_started(&fixture, run_command_id).await;
    s2_finish_first_unit(&fixture, CodingExecutionUnitStatus::Completed);

    let result = post_start(&fixture.state, "cmd_n3_s2_0002", PLAN_REVISION_ID)
        .await
        .expect("continuation click must advance ready units, not 409");
    assert_eq!(result.operation_state, OperationState::Accepted);
    assert_eq!(
        result.command_id, run_command_id,
        "run command stays frozen"
    );

    let web = fixture.target_attempt(fixture.web);
    let gateway = fixture.target_attempt(fixture.gateway);
    for (attempt, target) in [
        (web.clone(), fixture.web),
        (gateway.clone(), fixture.gateway),
    ] {
        let claim = attempt.start_claim.as_ref().expect("ready unit started");
        assert_eq!(
            claim.command_id,
            CodingPlanSelection::deterministic_start_command_id(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                run_command_id,
                &target.0.to_string()
            ),
            "continuation start uses the frozen deterministic command"
        );
        assert_eq!(
            fixture
                .state
                .coding_runs
                .runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
            1,
            "each started target crossed the barrier exactly once"
        );
    }
    // wi_service 依赖 wi_unrelated 尚未完成：frontend 不启动。
    assert!(
        fixture
            .target_attempt(fixture.frontend)
            .start_claim
            .is_none()
    );

    let run = fixture
        .run_store()
        .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(run.phase, CodingPlanRunPhase::Started);
    let selection = run.selection.expect("selection rotated");
    assert_eq!(selection.logical_work_item_id, "wi_registration");
    assert_eq!(selection.attempt_id, web.id);
    assert_eq!(
        result.selected_unit_id.as_deref(),
        Some(selection.unit_id.as_str())
    );
}

/// ②：续跑后再点（新 command）幂等——已启动 attempt 零新 claim/零二次
/// barrier，selection 不漂移，回执 Replayed。
#[tokio::test]
async fn plan_coding_s2_continuation_retry_is_idempotent() {
    let fixture = n3_fixture().await;
    let run_command_id = "cmd_n3_s2_0001";
    s2_seed_first_start_started(&fixture, run_command_id).await;
    s2_finish_first_unit(&fixture, CodingExecutionUnitStatus::Completed);
    let first = post_start(&fixture.state, "cmd_n3_s2_0002", PLAN_REVISION_ID)
        .await
        .expect("first continuation click starts ready set");
    assert_eq!(first.operation_state, OperationState::Accepted);

    let web_before = fixture.target_attempt(fixture.web);
    let gateway_before = fixture.target_attempt(fixture.gateway);
    let selection_before = fixture
        .run_store()
        .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .unwrap()
        .selection
        .clone()
        .expect("rotated selection");

    let retry = post_start(&fixture.state, "cmd_n3_s2_0003", PLAN_REVISION_ID)
        .await
        .expect("retry must not fail");
    assert_eq!(retry.operation_state, OperationState::Replayed);

    let web_after = fixture.target_attempt(fixture.web);
    let gateway_after = fixture.target_attempt(fixture.gateway);
    assert_eq!(
        web_after
            .start_claim
            .as_ref()
            .map(|claim| claim.command_id.clone()),
        web_before
            .start_claim
            .as_ref()
            .map(|claim| claim.command_id.clone()),
        "no second claim on started attempt"
    );
    assert_eq!(
        gateway_after
            .start_claim
            .as_ref()
            .map(|claim| claim.command_id.clone()),
        gateway_before
            .start_claim
            .as_ref()
            .map(|claim| claim.command_id.clone())
    );
    for attempt in [web_after, gateway_after] {
        assert_eq!(
            fixture
                .state
                .coding_runs
                .runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
            1,
            "no second barrier crossing"
        );
    }
    let run = fixture
        .run_store()
        .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(run.selection.clone().expect("selection"), selection_before);
    assert_eq!(run.phase, CodingPlanRunPhase::Started);
}

/// ③：wi_core 失败 → 依赖链（wi_registration）不启动、保持等待依赖事实；
/// 独立链（wi_unrelated）继续启动。
#[tokio::test]
async fn plan_coding_s2_failed_unit_stops_dependents_and_starts_independent_chain() {
    let fixture = n3_fixture().await;
    let run_command_id = "cmd_n3_s2_0001";
    s2_seed_first_start_started(&fixture, run_command_id).await;
    s2_finish_first_unit(&fixture, CodingExecutionUnitStatus::Failed);

    let result = post_start(&fixture.state, "cmd_n3_s2_0002", PLAN_REVISION_ID)
        .await
        .expect("independent chains must still advance");
    assert_eq!(result.operation_state, OperationState::Accepted);

    let gateway = fixture.target_attempt(fixture.gateway);
    assert!(
        gateway.start_claim.is_some(),
        "independent chain (wi_unrelated) starts"
    );
    assert!(
        fixture.target_attempt(fixture.web).start_claim.is_none(),
        "failed dependency chain (wi_registration) must not start"
    );
    assert!(
        fixture
            .target_attempt(fixture.frontend)
            .start_claim
            .is_none()
    );

    // 失败链按事实可见：wi_registration 仍在等 wi_core（不是天书暂停文案）。
    let registration = unit_dto(&result.progress, "wi_registration");
    assert_eq!(registration.execution_state, "waiting_dependency");
    assert_eq!(
        registration.unsatisfied_dependency_ids,
        vec!["wi_core".to_string()]
    );
    assert_ne!(
        registration.waiting_reason.as_deref(),
        Some(crate::product::coding_plan_run::PLAN_CODING_CONTINUATION_NOT_ENABLED)
    );
}

/// ④：legacy N5 暂停（run Paused）后新点击 → resume 续跑，不再 409
/// `coding_plan_run_not_active`；横幅不出现 not enabled 天书。
#[tokio::test]
async fn plan_coding_s2_paused_run_resumes_and_drives_instead_of_409() {
    let fixture = n3_fixture().await;
    let run_command_id = "cmd_n3_s2_0001";
    let started = s2_seed_first_start_started(&fixture, run_command_id).await;
    s2_finish_first_unit(&fixture, CodingExecutionUnitStatus::Completed);
    let paused = fixture
        .run_store()
        .mark_paused(&started)
        .expect("legacy pause");
    assert_eq!(paused.phase, CodingPlanRunPhase::Paused);

    let result = post_start(&fixture.state, "cmd_n3_s2_0002", PLAN_REVISION_ID)
        .await
        .expect("paused run must resume and continue, not 409");
    assert_eq!(result.operation_state, OperationState::Accepted);

    let run = fixture
        .run_store()
        .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(run.phase, CodingPlanRunPhase::Started);
    assert_eq!(run.pause_reason, None, "resume clears the pause reason");
    assert!(
        fixture.target_attempt(fixture.web).start_claim.is_some(),
        "wi_registration starts after resume"
    );
    assert!(
        fixture
            .target_attempt(fixture.gateway)
            .start_claim
            .is_some(),
        "wi_unrelated starts after resume"
    );

    let progress = get_progress(&fixture.state).await.expect("progress");
    assert_eq!(progress.phase.as_deref(), Some("started"));
    for unit in &progress.units {
        assert_ne!(
            unit.waiting_reason.as_deref(),
            Some(crate::product::coding_plan_run::PLAN_CODING_CONTINUATION_NOT_ENABLED),
            "no continuation-not-enabled banner during normal advance: {}",
            unit.logical_work_item_id
        );
    }
}

/// 指定 target 桶的根 unit（建组物化 Running 的 order_index 0）落终态。
fn s2_finish_root_unit(
    fixture: &N3Fixture,
    target: LogicalRepositoryId,
    logical_work_item_id: &str,
    status: CodingExecutionUnitStatus,
) {
    let attempt = fixture.target_attempt(target);
    let unit = fixture
        .coding_store()
        .list_coding_units(PROJECT_ID, ISSUE_ID, &attempt.id)
        .unwrap()
        .into_iter()
        .find(|unit| unit.logical_work_item_id == logical_work_item_id)
        .unwrap_or_else(|| panic!("{logical_work_item_id} unit materialized"));
    fixture
        .coding_store()
        .update_coding_unit_status(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            &unit.id,
            status,
            Some("s2 root unit finished".to_string()),
        )
        .unwrap();
}

/// P1 修复轮：跨桶 parked 续跑——frontend 桶 Running 无 active unit
///（wi_service 已完成、current 保留完成 unit 的真实 park 形状），
/// wi_panel 跨桶依赖全部完成→续跑物化新 unit 时，worktree 锁转移 from
/// 必须取**推进前**身份（上一完成 unit），不得撞 repo_worktree_active。
#[tokio::test]
async fn plan_coding_s2_parked_resume_transfers_lock_from_pre_advance_identity() {
    use crate::product::lifecycle_store::{LifecycleStore, UpsertRepoSharedWorktreeInput};

    let fixture = n3_fixture().await;
    let run_command_id = "cmd_n3_s2_p1";
    s2_seed_first_start_started(&fixture, run_command_id).await;
    // 完成事实：wi_core（busi）/wi_unrelated（gateway）/wi_service（frontend）。
    s2_finish_first_unit(&fixture, CodingExecutionUnitStatus::Completed);
    s2_finish_root_unit(
        &fixture,
        fixture.gateway,
        "wi_unrelated",
        CodingExecutionUnitStatus::Completed,
    );
    let frontend = fixture.target_attempt(fixture.frontend);
    fixture
        .coding_store()
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &frontend.id)
        .unwrap();
    s2_finish_root_unit(
        &fixture,
        fixture.frontend,
        "wi_service",
        CodingExecutionUnitStatus::Completed,
    );
    // 真实 park 形状（advance Waiting 分支）：清 active、current 保留完成 unit。
    let mut parked = fixture.target_attempt(fixture.frontend);
    parked.active_unit_id = None;
    parked.current_work_item_id = Some("wi_service".to_string());
    fixture
        .coding_store()
        .update_attempt_non_status_fields(&parked)
        .unwrap();
    // repo 维锁事实：owner=frontend attempt、active=wi_service（执行 wi_service
    // 期间的锁记录，物化 worktree 后仍在场）。
    let lifecycle = LifecycleStore::new(fixture.paths.clone());
    lifecycle
        .upsert_repo_shared_worktree(UpsertRepoSharedWorktreeInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: fixture.frontend,
            branch_name: "aria-issues/issue_plan_0001".to_string(),
            worktree_path: std::env::temp_dir().join("s2-parked-frontend"),
            base_branch: "main".to_string(),
        })
        .unwrap();
    lifecycle
        .try_acquire_repo_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            fixture.frontend,
            "wi_service",
            &frontend.id,
        )
        .unwrap();

    let result = post_start(&fixture.state, "cmd_n3_s2_p1_b", PLAN_REVISION_ID)
        .await
        .expect("parked resume must transfer the lock, not fail on it");

    // wi_panel 物化 Running + 指针对齐 + runner 重启。
    let resumed = fixture.target_attempt(fixture.frontend);
    assert_eq!(
        resumed.current_work_item_id.as_deref(),
        Some("wi_panel"),
        "parked bucket advances to the next ready unit"
    );
    assert!(resumed.active_unit_id.is_some());
    assert_eq!(
        fixture
            .state
            .coding_runs
            .runner_count(&CodingAttemptRunKey::from_attempt(&resumed)),
        1,
        "parked attempt gets a fresh runner"
    );
    // 锁事实：active 转移到 wi_panel，owner 保持本 attempt。
    let record = lifecycle
        .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, fixture.frontend)
        .unwrap()
        .expect("repo shared worktree record");
    assert_eq!(
        record.current_active_work_item_id.as_deref(),
        Some("wi_panel")
    );
    assert_eq!(
        record.current_lock_owner_id.as_deref(),
        Some(frontend.id.as_str())
    );
    // selection 轮换到本轮确定性首个新启 unit（wi_registration 拓扑在前）。
    let run = fixture
        .run_store()
        .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .unwrap();
    let selection = run.selection.expect("rotated selection");
    assert_eq!(selection.logical_work_item_id, "wi_registration");
    let _ = result;
}
