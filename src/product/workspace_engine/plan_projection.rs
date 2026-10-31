use std::collections::{BTreeMap, BTreeSet};

use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::models::{
    DependencyGraphRevision, IssueWorkItemPlan, LogicalWorkItem, PlanProjectionBundle,
    PlanRevisionReason, PlanValidationReportArtifact, VerificationPlanRevision,
    WorkItemDraftRevision, WorkItemPlanCompileStatus, WorkItemPlanCompileTransaction,
    WorkItemPlanLineage, WorkItemPlanOutline, WorkItemPlanRevision, WorkItemProjectionBundle,
    WorkItemRevision,
};
use crate::product::work_item_contract::{
    ContractValidationReport, DependencyContractGraph, build_dependency_contract_graph,
    canonical_contract_hash, validate_dependency_contract_graph,
};
use crate::product::work_item_projection::{
    CompiledPlanProjections, CompiledWorkItemProjections, PlanProjectionCompileInput,
    PlanProjectionCompiler, PlanProjectionValidationInput, ProjectionCompileError,
    ProjectionValidationFinding, ProjectionValidationReport, WorkItemProjectionCompiler,
    plan_projection_hashes, projection_hashes, validate_plan_projection_coverage,
    validate_projection_coverage,
};
use crate::product::work_item_revision_store::{
    InitialPlanPublicationArtifacts, InitialPlanPublicationJournal, InitialPlanPublicationPhase,
    InitialWorkItemPublicationArtifacts, InitialWorkItemPublicationIds, WorkItemRevisionStore,
};

use super::WorkspaceEngine;

// SG1 纯图输入的 IR 投影与其归属解析同址（`member_attribution`，REQ-WG-02
// 唯一反查来源）；本模块保留同名 re-export，调用点路径不变。
pub(crate) use crate::product::workspace_engine::member_attribution::planning_structure_items_from_ir;

mod recovery;

const PROJECTION_SCHEMA_VERSION: u32 = 1;
const PROJECTION_COMPILER_VERSION: &str = "work-item-projection-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledWorkItemRevision {
    pub draft_revision: WorkItemDraftRevision,
    pub work_item_revision: WorkItemRevision,
    pub verification_plan_revision: VerificationPlanRevision,
    pub projection_bundle: WorkItemProjectionBundle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialPlanCompileOutcome {
    pub plan_revision: WorkItemPlanRevision,
    pub dependency_graph_revision: DependencyGraphRevision,
    pub validation_report: PlanValidationReportArtifact,
    pub plan_projection_bundle: PlanProjectionBundle,
    pub work_items: Vec<CompiledWorkItemRevision>,
    pub contract_validation: ContractValidationReport,
    pub projection_validation: ProjectionValidationReport,
}

#[derive(Debug)]
pub enum WorkspaceEngineError {
    Store(ProductStoreError),
    Projection(ProjectionCompileError),
    WorkItemPlanValidation(ContractValidationReport),
    ProjectionValidation(ProjectionValidationReport),
    InvalidInitialPlan(String),
    InvalidHumanPresentationTarget,
}

impl std::fmt::Display for WorkspaceEngineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "revision store operation failed: {error}"),
            Self::Projection(error) => write!(formatter, "projection compile failed: {error}"),
            Self::WorkItemPlanValidation(report) => write!(
                formatter,
                "canonical work item plan validation failed with {} finding(s): {}",
                report.findings.len(),
                format_contract_validation_findings(&report.findings)
            ),
            Self::ProjectionValidation(report) => write!(
                formatter,
                "work item projection validation failed with {} finding(s): {}",
                report.findings.len(),
                format_projection_validation_findings(&report.findings)
            ),
            Self::InvalidInitialPlan(message) => formatter.write_str(message),
            Self::InvalidHumanPresentationTarget => formatter
                .write_str("human presentation target must resolve exactly one projection bundle"),
        }
    }
}

/// 把契约校验 finding 渲染成可诊断的单行摘要。
///
/// 只报告数量会让失败原因不可诊断：调用方（compile transaction 的
/// `failure_reason`、UI 错误提示）拿到的就只有一个数字。
fn format_contract_validation_findings(
    findings: &[crate::product::work_item_contract::ContractValidationFinding],
) -> String {
    if findings.is_empty() {
        return "(no findings)".to_string();
    }
    findings
        .iter()
        .map(|finding| {
            let reference = finding
                .contract_ref
                .as_deref()
                .or(finding.capability_ref.as_deref())
                .unwrap_or("-");
            format!(
                "[{:?}] {} ({}): {}",
                finding.severity, finding.code, reference, finding.message
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// 把 projection 校验 finding 渲染成可诊断的单行摘要。
fn format_projection_validation_findings(
    findings: &[crate::product::work_item_projection::ProjectionValidationFinding],
) -> String {
    if findings.is_empty() {
        return "(no findings)".to_string();
    }
    findings
        .iter()
        .map(|finding| {
            format!(
                "[{}] {} ({}): {}",
                finding.projection,
                finding.code,
                finding.contract_ref.as_deref().unwrap_or("-"),
                finding.message
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

impl std::error::Error for WorkspaceEngineError {}

impl From<ProductStoreError> for WorkspaceEngineError {
    fn from(error: ProductStoreError) -> Self {
        Self::Store(error)
    }
}

impl From<ProjectionCompileError> for WorkspaceEngineError {
    fn from(error: ProjectionCompileError) -> Self {
        Self::Projection(error)
    }
}

pub fn compile_work_item_revision(
    draft: &WorkItemDraftRevision,
    projection_compiler: &WorkItemProjectionCompiler,
    ids: &InitialWorkItemPublicationIds,
    created_at: &str,
) -> Result<CompiledWorkItemRevision, WorkspaceEngineError> {
    let work_item_revision_id = ids.work_item_revision_id.clone();
    let verification_plan_revision_id = ids.verification_plan_revision_id.clone();
    let projection_bundle_id = ids.work_item_projection_bundle_id.clone();
    let canonical_contract = draft.canonical_contract_candidate.clone();
    let canonical_contract_hash = canonical_contract_hash(&canonical_contract)?;
    let compiled = projection_compiler.compile(&canonical_contract, &work_item_revision_id)?;
    let hashes = projection_hashes(&compiled)?;
    let created_at = created_at.to_string();

    Ok(CompiledWorkItemRevision {
        draft_revision: draft.clone(),
        work_item_revision: WorkItemRevision {
            id: work_item_revision_id.clone(),
            logical_work_item_id: draft.logical_work_item_id.clone(),
            source_draft_revision_id: draft.id.clone(),
            canonical_contract: canonical_contract.clone(),
            canonical_contract_hash: canonical_contract_hash.clone(),
            work_item_projection_bundle_id: projection_bundle_id.clone(),
            verification_plan_revision_id: verification_plan_revision_id.clone(),
            created_at: created_at.clone(),
        },
        verification_plan_revision: VerificationPlanRevision {
            id: verification_plan_revision_id,
            logical_work_item_id: draft.logical_work_item_id.clone(),
            source_draft_revision_id: draft.id.clone(),
            verification_checks: canonical_contract.verification_checks.clone(),
            created_at: created_at.clone(),
        },
        projection_bundle: WorkItemProjectionBundle {
            id: projection_bundle_id,
            work_item_revision_id,
            canonical_contract_hash,
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            compiler_version: PROJECTION_COMPILER_VERSION.to_string(),
            human_projection: compiled.human,
            coder_projection: compiled.coder,
            reviewer_projection: compiled.reviewer,
            human_projection_hash: hashes.human,
            coder_projection_hash: hashes.coder,
            reviewer_projection_hash: hashes.reviewer,
            created_at,
        },
    })
}

pub fn compile_plan_projection_bundle(
    plan_revision_id: &str,
    dependency_graph_revision_id: &str,
    plan_projection_bundle_id: &str,
    created_at: &str,
    input: PlanProjectionCompileInput<'_>,
    projection_compiler: &PlanProjectionCompiler,
    work_items: &[CompiledWorkItemRevision],
) -> Result<PlanProjectionBundle, WorkspaceEngineError> {
    let expected_plan_id = input.plan_id.to_string();
    let expected_source_refs = input.source_refs.clone();
    let expected_work_item_revision_ids = input.expected_work_item_revision_ids.clone();
    let dependency_graph = input.dependency_graph;
    let work_item_projections = input.work_item_projections;
    let compiled = projection_compiler.compile(input)?;
    let validation = validate_plan_projection_coverage(PlanProjectionValidationInput {
        expected_plan_id: &expected_plan_id,
        expected_source_refs: &expected_source_refs,
        expected_work_item_revision_ids: &expected_work_item_revision_ids,
        dependency_graph,
        compiled: &compiled,
        work_item_projections,
    });
    if !validation.is_valid() {
        return Err(WorkspaceEngineError::ProjectionValidation(validation));
    }

    let work_items_by_logical_id = work_items
        .iter()
        .map(|item| (item.work_item_revision.logical_work_item_id.as_str(), item))
        .collect::<BTreeMap<_, _>>();
    let work_item_projection_bundle_refs = compiled
        .coder
        .ordered_logical_work_item_ids
        .iter()
        .map(|logical_id| {
            work_items_by_logical_id
                .get(logical_id.as_str())
                .map(|item| item.projection_bundle.id.clone())
                .ok_or_else(|| {
                    WorkspaceEngineError::InvalidInitialPlan(format!(
                        "plan projection references missing logical work item `{logical_id}`"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let hashes = plan_projection_hashes(&compiled)?;
    Ok(PlanProjectionBundle {
        id: plan_projection_bundle_id.to_string(),
        plan_revision_id: plan_revision_id.to_string(),
        dependency_graph_revision_id: dependency_graph_revision_id.to_string(),
        work_item_projection_bundle_refs,
        human_group_projection_hash: hashes.human,
        coder_group_context_hash: hashes.coder,
        reviewer_group_matrix_hash: hashes.reviewer,
        human_group_projection: compiled.human,
        coder_group_context: compiled.coder,
        reviewer_group_matrix: compiled.reviewer,
        compiler_version: PROJECTION_COMPILER_VERSION.to_string(),
        created_at: created_at.to_string(),
    })
}

pub fn plan_projection_input<'a>(
    outline: &'a WorkItemPlanOutline,
    dependency_graph: &'a DependencyContractGraph,
    work_item_projections: &'a BTreeMap<String, CompiledWorkItemProjections>,
) -> PlanProjectionCompileInput<'a> {
    let mut source_refs = outline.source_story_spec_ids.clone();
    source_refs.extend(outline.source_design_spec_ids.iter().cloned());
    PlanProjectionCompileInput {
        plan_id: &outline.id,
        goal: &outline.strategy_summary,
        split_reason: &outline.handoff_strategy,
        source_refs,
        dependency_graph,
        work_item_projections,
        expected_work_item_revision_ids: work_item_projections
            .iter()
            .map(|(logical_id, projections)| {
                (
                    logical_id.clone(),
                    projections.coder.work_item_revision_id.clone(),
                )
            })
            .collect(),
    }
}

pub fn publish_initial_plan_revision(
    store: &WorkItemRevisionStore,
    journal: &InitialPlanPublicationJournal,
) -> Result<InitialPlanCompileOutcome, WorkspaceEngineError> {
    let published = store.publish_or_resume_initial_plan_revision(journal)?;
    let artifacts = published.artifacts;
    let validation_report = artifacts.validation_report;
    let contract_validation = validation_report.contract_validation.clone();
    let projection_validation = validation_report.projection_validation.clone();
    Ok(InitialPlanCompileOutcome {
        plan_revision: artifacts.plan_revision,
        dependency_graph_revision: artifacts.dependency_graph_revision,
        validation_report,
        plan_projection_bundle: artifacts.plan_projection_bundle,
        work_items: artifacts
            .work_items
            .into_iter()
            .map(|item| CompiledWorkItemRevision {
                draft_revision: item.draft_revision,
                work_item_revision: item.work_item_revision,
                verification_plan_revision: item.verification_plan_revision,
                projection_bundle: item.projection_bundle,
            })
            .collect(),
        contract_validation,
        projection_validation,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialPlanPublicationInput {
    pub previous_plan: IssueWorkItemPlan,
    pub outline: WorkItemPlanOutline,
    pub outline_order: Vec<String>,
    pub accepted_drafts: Vec<WorkItemDraftRevision>,
    pub compile_id: String,
    pub now: String,
    pub allocated_ids: crate::product::work_item_revision_store::InitialPlanPublicationIds,
    pub publication_provenance_ref: Option<String>,
    pub publication_provenance_content_hash: Option<String>,
}

/// 只依赖已装配输入的 initial publication 投影。此函数不读取 lifecycle、outline、
/// transaction 或任意 store，便于 legacy 与后续 IR adapter 共享。
pub fn prepare_initial_plan_publication(
    input: InitialPlanPublicationInput,
) -> Result<InitialPlanPublicationJournal, WorkspaceEngineError> {
    if input.accepted_drafts.is_empty() {
        return Err(WorkspaceEngineError::InvalidInitialPlan(
            "initial plan compile requires at least one accepted draft".to_string(),
        ));
    }
    let ordered_drafts = order_accepted_drafts_by_outline_order(
        &input.outline,
        &input.outline_order,
        &input.accepted_drafts,
    )?;
    let active_draft_revision_ids = ordered_drafts
        .iter()
        .map(|draft| (draft.logical_work_item_id.clone(), draft.id.clone()))
        .collect::<BTreeMap<_, _>>();
    if active_draft_revision_ids.len() != ordered_drafts.len() {
        return Err(WorkspaceEngineError::InvalidInitialPlan(
            "accepted drafts contain duplicate logical work item identities".to_string(),
        ));
    }

    let projection_compiler = WorkItemProjectionCompiler;
    let work_items = ordered_drafts
        .iter()
        .map(|draft| {
            let ids = input
                .allocated_ids
                .work_items
                .get(&draft.logical_work_item_id)
                .ok_or_else(|| {
                    WorkspaceEngineError::InvalidInitialPlan(format!(
                        "allocated publication ids missing for `{}`",
                        draft.logical_work_item_id
                    ))
                })?;
            compile_work_item_revision(draft, &projection_compiler, ids, &input.now)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let contracts = work_items
        .iter()
        .map(|item| item.work_item_revision.canonical_contract.clone())
        .collect::<Vec<_>>();
    let dependency_graph = build_dependency_contract_graph(&contracts)
        .map_err(WorkspaceEngineError::WorkItemPlanValidation)?;
    let contract_validation = validate_dependency_contract_graph(&dependency_graph);
    if !contract_validation.is_valid() {
        return Err(WorkspaceEngineError::WorkItemPlanValidation(
            contract_validation,
        ));
    }

    let work_item_projections = work_items
        .iter()
        .map(|item| {
            (
                item.work_item_revision.logical_work_item_id.clone(),
                CompiledWorkItemProjections {
                    human: item.projection_bundle.human_projection.clone(),
                    coder: item.projection_bundle.coder_projection.clone(),
                    reviewer: item.projection_bundle.reviewer_projection.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut projection_outline = input.outline.clone();
    projection_outline.id = input.previous_plan.id.clone();
    projection_outline.source_story_spec_ids = input.previous_plan.source_story_spec_ids.clone();
    projection_outline.source_design_spec_ids = input.previous_plan.source_design_spec_ids.clone();
    let plan_projection_bundle = compile_plan_projection_bundle(
        &input.allocated_ids.plan_revision_id,
        &input.allocated_ids.dependency_graph_revision_id,
        &input.allocated_ids.plan_projection_bundle_id,
        &input.now,
        plan_projection_input(
            &projection_outline,
            &dependency_graph,
            &work_item_projections,
        ),
        &PlanProjectionCompiler,
        &work_items,
    )?;
    let compiled_plan = CompiledPlanProjections {
        human: plan_projection_bundle.human_group_projection.clone(),
        coder: plan_projection_bundle.coder_group_context.clone(),
        reviewer: plan_projection_bundle.reviewer_group_matrix.clone(),
    };
    let expected_revision_ids = work_items
        .iter()
        .map(|item| {
            (
                item.work_item_revision.logical_work_item_id.clone(),
                item.work_item_revision.id.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut projection_findings = work_items
        .iter()
        .flat_map(|item| {
            validate_projection_coverage(
                &item.work_item_revision.canonical_contract,
                &item.work_item_revision.id,
                &CompiledWorkItemProjections {
                    human: item.projection_bundle.human_projection.clone(),
                    coder: item.projection_bundle.coder_projection.clone(),
                    reviewer: item.projection_bundle.reviewer_projection.clone(),
                },
            )
            .findings
        })
        .collect::<Vec<_>>();
    projection_findings.extend(
        validate_plan_projection_coverage(PlanProjectionValidationInput {
            expected_plan_id: &input.previous_plan.id,
            expected_source_refs: &plan_projection_bundle.human_group_projection.source_refs,
            expected_work_item_revision_ids: &expected_revision_ids,
            dependency_graph: &dependency_graph,
            compiled: &compiled_plan,
            work_item_projections: &work_item_projections,
        })
        .findings,
    );
    let projection_validation = normalized_projection_report(projection_findings);
    if !projection_validation.is_valid() {
        return Err(WorkspaceEngineError::ProjectionValidation(
            projection_validation,
        ));
    }

    let lineage = WorkItemPlanLineage {
        id: input.previous_plan.id.clone(),
        project_id: input.previous_plan.project_id.clone(),
        issue_id: input.previous_plan.issue_id.clone(),
        story_spec_refs: input.previous_plan.source_story_spec_ids.clone(),
        design_spec_refs: input.previous_plan.source_design_spec_ids.clone(),
        active_revision_id: None,
        active_amendment_id: None,
        created_at: input.now.clone(),
        updated_at: input.now.clone(),
    };
    let dependency_graph_revision = DependencyGraphRevision {
        id: input.allocated_ids.dependency_graph_revision_id.clone(),
        plan_id: input.previous_plan.id.clone(),
        edges: dependency_graph.edges.clone(),
        created_at: input.now.clone(),
    };
    let plan_revision = WorkItemPlanRevision {
        id: input.allocated_ids.plan_revision_id.clone(),
        plan_id: input.previous_plan.id.clone(),
        revision_no: 1,
        supersedes: None,
        reason: PlanRevisionReason::InitialCompile,
        work_item_bindings: expected_revision_ids,
        dependency_graph_revision_id: input.allocated_ids.dependency_graph_revision_id.clone(),
        validation_report_ref: input.allocated_ids.validation_report_id.clone(),
        plan_projection_bundle_id: plan_projection_bundle.id.clone(),
        publication_provenance_ref: input.publication_provenance_ref.clone(),
        created_at: input.now.clone(),
    };
    let validation_report = PlanValidationReportArtifact {
        id: input.allocated_ids.validation_report_id.clone(),
        plan_id: input.previous_plan.id.clone(),
        plan_revision_id: plan_revision.id.clone(),
        plan_projection_bundle_id: plan_projection_bundle.id.clone(),
        contract_validation: contract_validation.clone(),
        projection_validation: projection_validation.clone(),
        created_at: input.now.clone(),
    };
    validate_initial_publication(
        &lineage,
        &plan_revision,
        &dependency_graph_revision,
        &plan_projection_bundle,
        &work_items,
        &contract_validation,
        &projection_validation,
    )?;
    let publication_work_items = work_items
        .iter()
        .map(|item| InitialWorkItemPublicationArtifacts {
            logical_work_item: LogicalWorkItem {
                id: item.work_item_revision.logical_work_item_id.clone(),
                plan_id: input.previous_plan.id.clone(),
                title: item
                    .work_item_revision
                    .canonical_contract
                    .identity
                    .title
                    .clone(),
                active_revision_id: None,
                created_at: input.now.clone(),
                updated_at: input.now.clone(),
            },
            draft_revision: item.draft_revision.clone(),
            work_item_revision: item.work_item_revision.clone(),
            verification_plan_revision: item.verification_plan_revision.clone(),
            projection_bundle: item.projection_bundle.clone(),
        })
        .collect();
    crate::product::work_item_revision_store::prepare_initial_plan_publication_journal(
        &input.compile_id,
        &input.outline.id,
        active_draft_revision_ids,
        input.allocated_ids,
        &input.now,
        InitialPlanPublicationArtifacts {
            lineage,
            plan_revision,
            dependency_graph_revision,
            validation_report,
            plan_projection_bundle,
            publication_provenance_ref: input.publication_provenance_ref,
            publication_provenance_content_hash: input.publication_provenance_content_hash,
            work_items: publication_work_items,
        },
    )
    .map_err(WorkspaceEngineError::Store)
}

impl WorkspaceEngine {
    pub fn revision_store(&self) -> WorkItemRevisionStore {
        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .expect("persistent WorkspaceEngine requires lifecycle_store");
        WorkItemRevisionStore::new(lifecycle.app_paths())
    }
}

impl WorkspaceEngine {
    /// Legacy adapter：仅在这里读取 lifecycle、latest outline 与 matching transaction。
    /// publication projection 统一委托给无 store 的 `prepare_initial_plan_publication`。
    pub fn compile_initial_plan_revision(
        &mut self,
        accepted_drafts: &[WorkItemDraftRevision],
    ) -> Result<InitialPlanCompileOutcome, WorkspaceEngineError> {
        if accepted_drafts.is_empty() {
            return Err(WorkspaceEngineError::InvalidInitialPlan(
                "initial plan compile requires at least one accepted draft".to_string(),
            ));
        }
        let lifecycle = self.lifecycle_store.clone().ok_or_else(|| {
            WorkspaceEngineError::InvalidInitialPlan("lifecycle_store unavailable".to_string())
        })?;
        let previous_plan = lifecycle.get_issue_work_item_plan(
            &self.session.project_id,
            &self.session.issue_id,
            &self.session.entity_id,
        )?;
        let outline = self
            .latest_work_item_plan_outline_candidate()
            .map_err(WorkspaceEngineError::InvalidInitialPlan)?
            .outline;
        let outline_order =
            crate::product::workspace_engine::work_item_plan_outline_topological_order(&outline)
                .map_err(WorkspaceEngineError::InvalidInitialPlan)?;
        let ordered_drafts = order_accepted_drafts(&outline, accepted_drafts)?;
        let active_draft_ids = ordered_drafts
            .iter()
            .map(|draft| draft.id.clone())
            .collect::<Vec<_>>();
        let plan_store = self
            .work_item_plan_store()
            .map_err(WorkspaceEngineError::InvalidInitialPlan)?;
        let matching_transactions = plan_store
            .list_compile_transactions(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )?
            .into_iter()
            .filter(|tx| {
                matches!(
                    tx.status,
                    WorkItemPlanCompileStatus::Committing
                        | WorkItemPlanCompileStatus::RecoveryRequired
                ) && tx.outline_version_ref == outline.id
                    && tx.previous_plan_snapshot == previous_plan
                    && same_unique_ids(&tx.active_draft_ids, &active_draft_ids)
            })
            .collect::<Vec<_>>();
        let tx = match matching_transactions.as_slice() {
            [tx] => tx,
            [] => {
                return Err(WorkspaceEngineError::InvalidInitialPlan(
                    "current initial plan compile transaction is missing".to_string(),
                ));
            }
            _ => {
                return Err(WorkspaceEngineError::InvalidInitialPlan(
                    "current initial plan compile transaction is ambiguous".to_string(),
                ));
            }
        };
        let logical_ids = outline
            .work_item_outlines
            .iter()
            .map(|outline| outline.logical_work_item_id.clone())
            .collect::<Vec<_>>();
        let allocated_ids =
            crate::product::work_item_revision_store::allocate_initial_plan_publication_ids(
                &previous_plan.project_id,
                &previous_plan.issue_id,
                &previous_plan.id,
                &tx.compile_id,
                &logical_ids,
            )?;
        let journal = prepare_initial_plan_publication(InitialPlanPublicationInput {
            previous_plan,
            outline,
            outline_order,
            accepted_drafts: accepted_drafts.to_vec(),
            compile_id: tx.compile_id.clone(),
            now: tx.created_at.clone(),
            allocated_ids,
            publication_provenance_ref: tx.publication_provenance_ref.clone(),
            publication_provenance_content_hash: tx.publication_provenance_content_hash.clone(),
        })?;
        let revision_store = WorkItemRevisionStore::new(lifecycle.app_paths());
        publish_initial_plan_revision(&revision_store, &journal)
    }
}

fn order_accepted_drafts<'a>(
    outline: &WorkItemPlanOutline,
    accepted_drafts: &'a [WorkItemDraftRevision],
) -> Result<Vec<&'a WorkItemDraftRevision>, WorkspaceEngineError> {
    let outline_order = outline
        .work_item_outlines
        .iter()
        .map(|item| item.outline_id.clone())
        .collect::<Vec<_>>();
    order_accepted_drafts_by_outline_order(outline, &outline_order, accepted_drafts)
}

fn order_accepted_drafts_by_outline_order<'a>(
    outline: &WorkItemPlanOutline,
    outline_order: &[String],
    accepted_drafts: &'a [WorkItemDraftRevision],
) -> Result<Vec<&'a WorkItemDraftRevision>, WorkspaceEngineError> {
    if outline_order.len() != outline.work_item_outlines.len()
        || outline_order.iter().collect::<BTreeSet<_>>().len() != outline_order.len()
    {
        return Err(WorkspaceEngineError::InvalidInitialPlan(
            "initial plan publication outline order is invalid".to_string(),
        ));
    }
    let outlines_by_id = outline
        .work_item_outlines
        .iter()
        .map(|item| (item.outline_id.as_str(), item))
        .collect::<BTreeMap<_, _>>();
    let drafts_by_logical_id = accepted_drafts
        .iter()
        .map(|draft| (draft.logical_work_item_id.as_str(), draft))
        .collect::<BTreeMap<_, _>>();
    if drafts_by_logical_id.len() != accepted_drafts.len() {
        return Err(WorkspaceEngineError::InvalidInitialPlan(
            "accepted drafts contain duplicate logical work item identities".to_string(),
        ));
    }
    let ordered = outline_order
        .iter()
        .map(|outline_id| {
            let item = outlines_by_id.get(outline_id.as_str()).ok_or_else(|| {
                WorkspaceEngineError::InvalidInitialPlan(format!(
                    "publication outline order references missing outline `{outline_id}`"
                ))
            })?;
            drafts_by_logical_id
                .get(item.logical_work_item_id.as_str())
                .copied()
                .ok_or_else(|| {
                    WorkspaceEngineError::InvalidInitialPlan(format!(
                        "accepted draft missing for logical work item `{}`",
                        item.logical_work_item_id
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if ordered.len() != accepted_drafts.len() {
        return Err(WorkspaceEngineError::InvalidInitialPlan(
            "accepted drafts contain logical work items outside the current outline".to_string(),
        ));
    }
    Ok(ordered)
}

fn same_unique_ids(left: &[String], right: &[String]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let expected_len = left.len();
    let left = left.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let right = right.iter().map(String::as_str).collect::<BTreeSet<_>>();
    left.len() == expected_len && right.len() == expected_len && left == right
}

fn normalized_projection_report(
    mut findings: Vec<ProjectionValidationFinding>,
) -> ProjectionValidationReport {
    findings.sort_by(|left, right| {
        (
            &left.code,
            &left.projection,
            &left.contract_ref,
            &left.message,
        )
            .cmp(&(
                &right.code,
                &right.projection,
                &right.contract_ref,
                &right.message,
            ))
    });
    findings.dedup();
    ProjectionValidationReport { findings }
}

fn validate_initial_publication(
    lineage: &WorkItemPlanLineage,
    plan_revision: &WorkItemPlanRevision,
    dependency_graph_revision: &DependencyGraphRevision,
    plan_projection_bundle: &PlanProjectionBundle,
    work_items: &[CompiledWorkItemRevision],
    contract_validation: &ContractValidationReport,
    projection_validation: &ProjectionValidationReport,
) -> Result<(), WorkspaceEngineError> {
    if !contract_validation.is_valid() {
        return Err(WorkspaceEngineError::WorkItemPlanValidation(
            contract_validation.clone(),
        ));
    }
    if !projection_validation.is_valid() {
        return Err(WorkspaceEngineError::ProjectionValidation(
            projection_validation.clone(),
        ));
    }
    if lineage.active_revision_id.is_some()
        || plan_revision.revision_no != 1
        || plan_revision.supersedes.is_some()
        || plan_revision.reason != PlanRevisionReason::InitialCompile
        || plan_revision.plan_id != lineage.id
        || dependency_graph_revision.plan_id != lineage.id
        || plan_revision.dependency_graph_revision_id != dependency_graph_revision.id
        || plan_revision.plan_projection_bundle_id != plan_projection_bundle.id
        || plan_projection_bundle.plan_revision_id != plan_revision.id
        || plan_projection_bundle.dependency_graph_revision_id != dependency_graph_revision.id
    {
        return Err(WorkspaceEngineError::InvalidInitialPlan(
            "initial plan publication bindings are inconsistent".to_string(),
        ));
    }

    let bindings = work_items
        .iter()
        .map(|item| {
            (
                item.work_item_revision.logical_work_item_id.clone(),
                item.work_item_revision.id.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let bundle_refs = work_items
        .iter()
        .map(|item| item.projection_bundle.id.clone())
        .collect::<BTreeSet<_>>();
    if bindings.len() != work_items.len()
        || plan_revision.work_item_bindings != bindings
        || plan_projection_bundle
            .work_item_projection_bundle_refs
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            != bundle_refs
    {
        return Err(WorkspaceEngineError::InvalidInitialPlan(
            "initial plan work item bindings are inconsistent".to_string(),
        ));
    }

    for value in [
        &lineage.id,
        &lineage.project_id,
        &lineage.issue_id,
        &plan_revision.id,
        &plan_revision.validation_report_ref,
        &dependency_graph_revision.id,
        &plan_projection_bundle.id,
    ] {
        validate_relative_id(value)?;
    }
    for item in work_items {
        if item.draft_revision.logical_work_item_id != item.work_item_revision.logical_work_item_id
            || item.verification_plan_revision.logical_work_item_id
                != item.work_item_revision.logical_work_item_id
            || item.projection_bundle.work_item_revision_id != item.work_item_revision.id
            || item.work_item_revision.source_draft_revision_id != item.draft_revision.id
            || item.work_item_revision.verification_plan_revision_id
                != item.verification_plan_revision.id
            || item.work_item_revision.work_item_projection_bundle_id != item.projection_bundle.id
        {
            return Err(WorkspaceEngineError::InvalidInitialPlan(format!(
                "compiled work item `{}` has inconsistent revision bindings",
                item.work_item_revision.logical_work_item_id
            )));
        }
        for value in [
            &item.draft_revision.id,
            &item.draft_revision.logical_work_item_id,
            &item.work_item_revision.id,
            &item.verification_plan_revision.id,
            &item.projection_bundle.id,
        ] {
            validate_relative_id(value)?;
        }
    }
    Ok(())
}

// ===================== SG3a.1 候选结构门与同代结构上下文（REQ-SG-02/03、REQ-PLN-09/10）=====================
//
// plan §0.6.2：结构报告的唯一生产/复验配套——author 落盘前的系统门用
// `build_single_candidate_structure_context` 一次装载 live 事实（只读，不
// selection 写回），SC reviewer builder 对同一事实重算同 digest 做「旧
// context 拒发」。两端口共用本节单源，禁止第二配方/第二上下文权威。

use crate::product::planning_failure::{
    PersistedEffectivePlanningEdge, PersistedPlanningStructureReport,
    PersistedPlanningStructureSpan, PlanningFailureKind,
};
use crate::product::workspace_engine::draft_batch::compile_support::{
    PlanningStructureContext, PlanningStructureReport, project_effective_planning_graph,
};
use crate::product::workspace_engine::types::{PreparedStructuralCandidate, StructuralAssessment};
use sha2::{Digest, Sha256};

/// 候选结构门/结构报告共用的有效图规则版本（SG1 纯投影首版：canonical
/// Inputs ∪ change_order 自动边）。producer 冻结与 reviewer builder 复验必须
/// 同值，漂移即旧 context 拒发。
pub(crate) const PLANNING_STRUCTURE_RULE_VERSION: &str = "rule/planning-structure@1";

/// 结构上下文 digest 的单源配方：对除 `context_digest` 自身外的全字段
/// serde_json 序列化取 sha256。
pub(crate) fn compute_planning_context_digest(context: &PlanningStructureContext) -> String {
    let mut canonical = context.clone();
    canonical.context_digest = String::new();
    let serialized = serde_json::to_string(&canonical).unwrap_or_default();
    hex::encode(Sha256::digest(serialized.as_bytes()))
}

fn digest_json<T: serde::Serialize>(value: &T) -> Result<String, String> {
    let serialized = serde_json::to_string(value)
        .map_err(|error| format!("serialize structure context digest input failed: {error}"))?;
    Ok(hex::encode(Sha256::digest(serialized.as_bytes())))
}

impl WorkspaceEngine {
    /// SG3a.1：装载当前 live 结构上下文（Design/member/options/baseline 同源
    /// 事实一次装载）。author 落盘门与 SC reviewer builder 共用——对同一
    /// session/plan 事实必然得到相同 `context_digest`；任一事实漂移（如
    /// options/confirmed design 变更）即失配，builder 侧旧 context 拒发。
    pub(crate) fn build_single_candidate_structure_context(
        &self,
        ir: &crate::product::work_item_plan_compiler::PlanCandidateIr,
        source_revision_ref: &str,
        plan_candidate_ir_ref: &str,
    ) -> Result<PlanningStructureContext, String> {
        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable for structure context".to_string())?;
        let plan = lifecycle
            .get_issue_work_item_plan(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load plan for structure context failed: {error}"))?;
        let confirmed = lifecycle
            .list_design_specs(&plan.project_id, &plan.issue_id)
            .map_err(|error| format!("list design specs for structure context failed: {error}"))?
            .into_iter()
            .find(|design| {
                plan.source_design_spec_ids.contains(&design.id)
                    && design.confirmation_status
                        == crate::product::models::LifecycleConfirmationStatus::Confirmed
            });
        let physical_by_logical =
            crate::product::workspace_engine::draft_batch::compile_support::resolve_logical_work_item_plan_repository_targets(lifecycle, &plan)?;
        // 成员表摘要：effective（manifest∩active∩selection）逻辑→物理映射的
        // 单源装载结果；selection 事实折叠进同一映射，不另立第二快照。
        let member_table_digest = physical_by_logical.as_ref().map(digest_json).transpose()?;
        let change_order =
            crate::product::workspace_engine::draft_batch::compile_support::load_change_order_from_confirmed_design(
                lifecycle, &plan,
            )?;
        let baseline_tree = crate::product::workspace_engine::plan_preflight::plan_baseline_tree(
            lifecycle,
            &self.session.project_id,
            &self.session.issue_id,
        )?;
        let baseline_tree_digest = baseline_tree.as_ref().map(digest_json).transpose()?;
        // Plan Intent 归属事实来自候选 IR（per-WI intent target）；无任何
        // intent 声明的 legacy 候选为 None。
        let intent_targets: Vec<(String, crate::product::logical_codebase::EnrollmentTarget)> = ir
            .items
            .iter()
            .filter_map(|item| {
                item.contract.intent_contract.as_ref().map(|intent| {
                    (
                        item.contract.identity.logical_work_item_id.clone(),
                        intent.target.clone(),
                    )
                })
            })
            .collect();
        let intent_digest = (!intent_targets.is_empty())
            .then(|| digest_json(&intent_targets))
            .transpose()?;
        let mut context = PlanningStructureContext {
            source_revision_ref: source_revision_ref.to_string(),
            plan_candidate_ir_ref: plan_candidate_ir_ref.to_string(),
            source_hash: ir.source_revision_hash.clone(),
            compiler_version: ir.compiler_version.clone(),
            rule_version: PLANNING_STRUCTURE_RULE_VERSION.to_string(),
            context_digest: String::new(),
            confirmed_design_id: confirmed.as_ref().map(|design| design.id.clone()),
            confirmed_design_digest: confirmed.as_ref().map(digest_json).transpose()?,
            member_table_digest,
            selection_digest: None,
            physical_by_logical: physical_by_logical.unwrap_or_default(),
            change_order,
            options_digest: digest_json(&plan.options)?,
            baseline_tree_digest,
            intent_digest,
        };
        context.context_digest = compute_planning_context_digest(&context);
        Ok(context)
    }
}

/// SG3a.1 系统门的有效图装配（纯函数，零 I/O）：IR 投影 → 归属唯一反查 →
/// SG1 有效图。`assess_single_candidate_structure` 与作者阶段 typed 失败的
/// 事实提取（`single_candidate_structure_failure`）共用本函数——同一次装配、
/// 同一份边集合，两处不会给出不同的仓序位次。
pub(crate) fn project_single_candidate_structure(
    ir: &crate::product::work_item_plan_compiler::PlanCandidateIr,
    context: &PlanningStructureContext,
) -> PlanningStructureReport {
    let mut items = planning_structure_items_from_ir(ir);
    // REQ-WG-01/REQ-WG-02：归属解析走唯一反查（与 Final Compile 同一函数）。
    // 成员表在场时 single_repository 与「未声明 intent」的 WI 不再落
    // (None, None)——否则仓序自动边在本门不生效，同一份 source 会在此判
    // 无环、在终编译判成环。
    super::member_attribution::resolve_member_targets(&mut items, ir, &context.physical_by_logical);
    // legacy 单仓（无授权成员表）与 Final Compile lowering 同口径：
    // member_targets=None 时 Plan Intent 归属不参与物理 target 解析，
    // 结构门不重裁其授权——归属核验只在 LC 多仓（成员表非空）生效。
    if context.physical_by_logical.is_empty() {
        for item in &mut items {
            item.declared_member = None;
            item.logical_target = None;
        }
    }
    project_effective_planning_graph(&items, context)
}

/// SG3a.1 系统门的纯评估：SG1 有效图投影 + typed 失败分流。零 store/网络
/// I/O——live 事实由外层 `build_single_candidate_structure_context` 一次装载。
pub(crate) fn assess_single_candidate_structure(
    source: String,
    ir: &crate::product::work_item_plan_compiler::PlanCandidateIr,
    context: &PlanningStructureContext,
) -> StructuralAssessment {
    let report = project_single_candidate_structure(ir, context);
    let source_hash = report.source_hash.clone();
    let context_digest = report.context_digest.clone();
    if report.failures.is_empty() {
        return StructuralAssessment::Accepted(Box::new(PreparedStructuralCandidate {
            source,
            ir: ir.clone(),
            report,
            source_hash,
            context_digest,
        }));
    }
    if report
        .failures
        .failures
        .iter()
        .any(|failure| matches!(failure.kind, PlanningFailureKind::Operational { .. }))
    {
        // 工程/装载内部错：typed operational 停点，不驱 author。
        return StructuralAssessment::Operational(report.failures);
    }
    // typed 结构失败：修复指令初轮为空集（SG5/SG6 接线 instruction producer）。
    StructuralAssessment::Rejected(report.failures, Vec::new())
}

/// SG1 纯报告 → SG3a.0 持久 payload 的单次冻结投影。`report_digest` 沿
/// store 单源函数封口；仅对 structurally_valid 报告调用（合法才冻结）。
pub(crate) fn freeze_persisted_structure_report(
    report: &PlanningStructureReport,
    source_revision_id: &str,
    ir_id: &str,
) -> Result<PersistedPlanningStructureReport, String> {
    let effective_edges: Vec<PersistedEffectivePlanningEdge> = report
        .effective_edges
        .iter()
        .map(|edge| PersistedEffectivePlanningEdge {
            from: edge.from.clone(),
            to: edge.to.clone(),
            sources: edge.sources.clone(),
            contract_refs: edge.contract_refs.clone(),
            spans: edge
                .spans
                .iter()
                .map(|span| PersistedPlanningStructureSpan {
                    field: span.field.clone(),
                    line: span.line,
                })
                .collect(),
            order_indices: edge.order_indices.clone(),
        })
        .collect();
    // 合法报告无环，witness 恒空；防御性提取保持 payload 合同完整——
    // 任一 witness 跳缺少真实有效边即 fail-closed，不虚构边。
    let edge_by_pair: std::collections::BTreeMap<(&str, &str), &PersistedEffectivePlanningEdge> =
        effective_edges
            .iter()
            .map(|edge| ((edge.from.as_str(), edge.to.as_str()), edge))
            .collect();
    let mut cycle_witness: Vec<PersistedEffectivePlanningEdge> = Vec::new();
    for failure in &report.failures.failures {
        if let PlanningFailureKind::DependencyCycle { witness } = &failure.kind {
            for hop in witness {
                let edge = edge_by_pair
                    .get(&(hop.from.as_str(), hop.to.as_str()))
                    .ok_or_else(|| {
                        format!(
                            "cycle witness hop {}->{} missing effective edge",
                            hop.from, hop.to
                        )
                    })?;
                cycle_witness.push((*edge).clone());
            }
        }
    }
    let protected_predicates = if report.structurally_valid {
        vec![
            "effective_graph_acyclic".to_string(),
            "canonical_inputs_preserved".to_string(),
            "acceptance_criteria_preserved".to_string(),
            "change_order_unchanged".to_string(),
        ]
    } else {
        Vec::new()
    };
    let mut payload = PersistedPlanningStructureReport {
        source_revision_id: source_revision_id.to_string(),
        source_hash: report.source_hash.clone(),
        ir_id: ir_id.to_string(),
        compiler_version: report.compiler_version.clone(),
        rule_version: report.rule_version.clone(),
        context_digest: report.context_digest.clone(),
        effective_edges,
        cycle_witness,
        findings: report.failures.clone(),
        structurally_valid: report.structurally_valid,
        protected_predicates,
        // 初轮无修复明确空（plan §0.6.2）：不伪模型许可；SG5/SG6 接线
        // instruction/audit producer 后才携带真实引用。
        instruction_refs: Vec::new(),
        repair_audit_refs: Vec::new(),
        report_digest: String::new(),
    };
    payload.report_digest =
        crate::product::work_item_plan_source_store::persisted_structure_report_digest(&payload)
            .map_err(|error| format!("derive structure report digest failed: {error:?}"))?;
    Ok(payload)
}

/// 系统门停点渲染：typed stable_code + affected_ids（依赖环的 affected_ids
/// 即 witness 节点序，`->` 连接呈现真实有向路径）。
pub(crate) fn structural_assessment_stop_reason(
    assessment: &StructuralAssessment,
) -> Result<(), String> {
    let render = |set: &crate::product::planning_failure::PlanningFailureSet| {
        set.failures
            .iter()
            .map(|failure| {
                format!(
                    "{}[{}]",
                    failure.kind.stable_code(),
                    failure.evidence.affected_ids.join("->")
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    match assessment {
        StructuralAssessment::Accepted(_) => Ok(()),
        StructuralAssessment::Rejected(set, _) => Err(format!(
            "single-candidate structure gate rejected: {}",
            render(set)
        )),
        StructuralAssessment::Operational(set) => Err(format!(
            "single-candidate structure gate operational stop: {}",
            render(set)
        )),
        StructuralAssessment::ExistingGate(set) => Err(format!(
            "single-candidate structure gate existing-gate stop: {}",
            render(set)
        )),
    }
}
