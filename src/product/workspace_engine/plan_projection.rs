use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::models::{
    DependencyGraphRevision, LogicalWorkItem, PlanProjectionBundle, PlanRevisionReason,
    PlanValidationReportArtifact, VerificationPlanRevision, WorkItemDraftRevision,
    WorkItemDraftRevisionStatus, WorkItemPlanLineage, WorkItemPlanOutline, WorkItemPlanRevision,
    WorkItemProjectionBundle, WorkItemRevision,
};
use crate::product::work_item_contract::{
    ContractValidationReport, DependencyContractGraph, build_dependency_contract_graph,
    canonical_contract_hash, validate_dependency_contract_graph,
};
use crate::product::work_item_projection::{
    CompiledPlanProjections, CompiledWorkItemProjections, PlanProjectionCompileInput,
    PlanProjectionCompiler, PlanProjectionValidationInput, ProjectionCompileError,
    ProjectionValidationFinding, ProjectionValidationReport, WorkItemProjectionCompiler,
    projection_hashes, validate_plan_projection_coverage, validate_projection_coverage,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

use super::WorkspaceEngine;

const PROJECTION_SCHEMA_VERSION: u32 = 1;
const PROJECTION_COMPILER_VERSION: &str = "work-item-projection-v1";
static NEXT_REVISION_ARTIFACT_ID: AtomicU64 = AtomicU64::new(1);

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
}

impl std::fmt::Display for WorkspaceEngineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "revision store operation failed: {error}"),
            Self::Projection(error) => write!(formatter, "projection compile failed: {error}"),
            Self::WorkItemPlanValidation(report) => write!(
                formatter,
                "canonical work item plan validation failed with {} finding(s)",
                report.findings.len()
            ),
            Self::ProjectionValidation(report) => write!(
                formatter,
                "work item projection validation failed with {} finding(s)",
                report.findings.len()
            ),
            Self::InvalidInitialPlan(message) => formatter.write_str(message),
        }
    }
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
) -> Result<CompiledWorkItemRevision, WorkspaceEngineError> {
    let work_item_revision_id = next_revision_artifact_id("work_item_revision");
    let verification_plan_revision_id = next_revision_artifact_id("verification_plan_revision");
    let projection_bundle_id = next_revision_artifact_id("work_item_projection_bundle");
    let canonical_contract = draft.canonical_contract_candidate.clone();
    let canonical_contract_hash = canonical_contract_hash(&canonical_contract)?;
    let compiled = projection_compiler.compile(&canonical_contract, &work_item_revision_id)?;
    let hashes = projection_hashes(&compiled)?;
    let created_at = chrono::Utc::now().to_rfc3339();

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
    let created_at = chrono::Utc::now().to_rfc3339();

    Ok(PlanProjectionBundle {
        id: next_revision_artifact_id("plan_projection_bundle"),
        plan_revision_id: plan_revision_id.to_string(),
        dependency_graph_revision_id: dependency_graph_revision_id.to_string(),
        work_item_projection_bundle_refs,
        human_group_projection_hash: projection_hash(&compiled.human)?,
        coder_group_context_hash: projection_hash(&compiled.coder)?,
        reviewer_group_matrix_hash: projection_hash(&compiled.reviewer)?,
        human_group_projection: compiled.human,
        coder_group_context: compiled.coder,
        reviewer_group_matrix: compiled.reviewer,
        compiler_version: PROJECTION_COMPILER_VERSION.to_string(),
        created_at,
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

#[allow(clippy::too_many_arguments)]
pub fn publish_initial_plan_revision(
    store: &WorkItemRevisionStore,
    lineage: &WorkItemPlanLineage,
    plan_revision: WorkItemPlanRevision,
    dependency_graph_revision: DependencyGraphRevision,
    plan_projection_bundle: PlanProjectionBundle,
    work_items: Vec<CompiledWorkItemRevision>,
    contract_validation: ContractValidationReport,
    projection_validation: ProjectionValidationReport,
) -> Result<InitialPlanCompileOutcome, WorkspaceEngineError> {
    validate_initial_publication(
        lineage,
        &plan_revision,
        &dependency_graph_revision,
        &plan_projection_bundle,
        &work_items,
        &contract_validation,
        &projection_validation,
    )?;
    match store.get_plan_lineage(&lineage.project_id, &lineage.issue_id, &lineage.id) {
        Err(ProductStoreError::NotFound { .. }) => {}
        Ok(_) => {
            return Err(WorkspaceEngineError::InvalidInitialPlan(format!(
                "initial plan lineage `{}` already exists",
                lineage.id
            )));
        }
        Err(error) => return Err(error.into()),
    }

    let validation_report = PlanValidationReportArtifact {
        id: plan_revision.validation_report_ref.clone(),
        plan_id: lineage.id.clone(),
        contract_validation: contract_validation.clone(),
        projection_validation: projection_validation.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    store.put_plan_lineage(lineage)?;
    let mut logical_work_items = Vec::with_capacity(work_items.len());
    for item in &work_items {
        let now = chrono::Utc::now().to_rfc3339();
        let logical_work_item = LogicalWorkItem {
            id: item.work_item_revision.logical_work_item_id.clone(),
            plan_id: lineage.id.clone(),
            title: item
                .work_item_revision
                .canonical_contract
                .identity
                .title
                .clone(),
            active_revision_id: None,
            created_at: now.clone(),
            updated_at: now,
        };
        store.put_logical_work_item(lineage, &logical_work_item)?;
        store.put_draft_revision(lineage, &item.draft_revision)?;
        store.put_verification_plan_revision(lineage, &item.verification_plan_revision)?;
        store.put_work_item_projection_bundle(lineage, &item.projection_bundle)?;
        store.put_work_item_revision(lineage, &item.work_item_revision)?;
        logical_work_items.push(logical_work_item);
    }
    store.put_dependency_graph_revision(lineage, &dependency_graph_revision)?;
    store.put_plan_projection_bundle(lineage, &plan_projection_bundle)?;
    store.put_plan_validation_report(lineage, &validation_report)?;
    store.put_plan_revision(lineage, &plan_revision)?;

    for (logical_work_item, item) in logical_work_items.iter().zip(work_items.iter()) {
        store.update_draft_revision_state(
            lineage,
            &item.draft_revision.id,
            WorkItemDraftRevisionStatus::Compiled,
        )?;
        store.set_active_work_item_revision(
            lineage,
            logical_work_item,
            None,
            &item.work_item_revision.id,
        )?;
    }
    store.set_active_plan_revision(lineage, &plan_revision.id)?;

    Ok(InitialPlanCompileOutcome {
        plan_revision,
        dependency_graph_revision,
        validation_report,
        plan_projection_bundle,
        work_items,
        contract_validation,
        projection_validation,
    })
}

impl WorkspaceEngine {
    pub fn revision_store(&self) -> WorkItemRevisionStore {
        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .expect("persistent WorkspaceEngine requires lifecycle_store");
        WorkItemRevisionStore::new(lifecycle.app_paths())
    }

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
        let ordered_drafts = order_accepted_drafts(&outline, accepted_drafts)?;
        let projection_compiler = WorkItemProjectionCompiler;
        let work_items = ordered_drafts
            .iter()
            .map(|draft| compile_work_item_revision(draft, &projection_compiler))
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
        let plan_revision_id = next_revision_artifact_id("plan_revision");
        let dependency_graph_revision_id = next_revision_artifact_id("dependency_graph_revision");
        let mut projection_outline = outline.clone();
        projection_outline.id = previous_plan.id.clone();
        projection_outline.source_story_spec_ids = previous_plan.source_story_spec_ids.clone();
        projection_outline.source_design_spec_ids = previous_plan.source_design_spec_ids.clone();
        let plan_projection_bundle = compile_plan_projection_bundle(
            &plan_revision_id,
            &dependency_graph_revision_id,
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
                expected_plan_id: &previous_plan.id,
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

        let now = chrono::Utc::now().to_rfc3339();
        let lineage = WorkItemPlanLineage {
            id: previous_plan.id.clone(),
            project_id: previous_plan.project_id.clone(),
            issue_id: previous_plan.issue_id.clone(),
            story_spec_refs: previous_plan.source_story_spec_ids.clone(),
            design_spec_refs: previous_plan.source_design_spec_ids.clone(),
            active_revision_id: None,
            active_amendment_id: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let dependency_graph_revision = DependencyGraphRevision {
            id: dependency_graph_revision_id.clone(),
            plan_id: previous_plan.id.clone(),
            edges: dependency_graph.edges.clone(),
            created_at: now.clone(),
        };
        let plan_revision = WorkItemPlanRevision {
            id: plan_revision_id,
            plan_id: previous_plan.id,
            revision_no: 1,
            supersedes: None,
            reason: PlanRevisionReason::InitialCompile,
            work_item_bindings: expected_revision_ids,
            dependency_graph_revision_id,
            validation_report_ref: next_revision_artifact_id("plan_validation_report"),
            plan_projection_bundle_id: plan_projection_bundle.id.clone(),
            created_at: now,
        };

        publish_initial_plan_revision(
            &WorkItemRevisionStore::new(lifecycle.app_paths()),
            &lineage,
            plan_revision,
            dependency_graph_revision,
            plan_projection_bundle,
            work_items,
            contract_validation,
            projection_validation,
        )
    }
}

fn next_revision_artifact_id(prefix: &str) -> String {
    let sequence = NEXT_REVISION_ARTIFACT_ID.fetch_add(1, Ordering::Relaxed);
    format!(
        "{prefix}_{}_{sequence:06}",
        chrono::Utc::now().format("%Y%m%d%H%M%S%3f")
    )
}

fn projection_hash<T: Serialize>(value: &T) -> Result<String, WorkspaceEngineError> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        WorkspaceEngineError::InvalidInitialPlan(format!(
            "serialize plan projection for hashing failed: {error}"
        ))
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn order_accepted_drafts<'a>(
    outline: &WorkItemPlanOutline,
    accepted_drafts: &'a [WorkItemDraftRevision],
) -> Result<Vec<&'a WorkItemDraftRevision>, WorkspaceEngineError> {
    let drafts_by_logical_id = accepted_drafts
        .iter()
        .map(|draft| (draft.logical_work_item_id.as_str(), draft))
        .collect::<BTreeMap<_, _>>();
    if drafts_by_logical_id.len() != accepted_drafts.len() {
        return Err(WorkspaceEngineError::InvalidInitialPlan(
            "accepted drafts contain duplicate logical work item identities".to_string(),
        ));
    }
    let ordered = outline
        .work_item_outlines
        .iter()
        .map(|item| {
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
