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
        publication_provenance_ref: None,
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
            publication_provenance_ref: None,
            publication_provenance_content_hash: None,
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
