use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAttemptScope, CodingExecutionAttempt, CodingExecutionUnit, CodingUnitRun,
};
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::models::{
    DependencyGraphRevision, HumanPresentationRevision, PlanProjectionBundle,
    VerificationPlanRevision, WorkItemPlanLineage, WorkItemPlanRevision, WorkItemProjectionBundle,
    WorkItemRevision, WorkItemRuntimeBinding, WorkspaceSessionRecord, WorkspaceType,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWorkItemRuntime {
    pub binding: WorkItemRuntimeBinding,
    pub lineage: WorkItemPlanLineage,
    pub plan_revision: WorkItemPlanRevision,
    pub dependency_graph: DependencyGraphRevision,
    pub work_item_revision: WorkItemRevision,
    pub verification_plan_revision: VerificationPlanRevision,
    pub projection_bundle: WorkItemProjectionBundle,
    pub plan_projection_bundle: PlanProjectionBundle,
    pub human_presentation: Option<HumanPresentationRevision>,
}

#[derive(Debug, Clone)]
pub struct WorkItemRuntimeReader {
    paths: ProductAppPaths,
}

impl WorkItemRuntimeReader {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn resolve_binding(
        &self,
        project_id: &str,
        issue_id: &str,
        binding: &WorkItemRuntimeBinding,
    ) -> Result<ResolvedWorkItemRuntime, ProductStoreError> {
        validate_binding(binding)?;
        let store = WorkItemRevisionStore::new(self.paths.clone());
        let lineage = binding_reference(
            store.get_plan_lineage(project_id, issue_id, &binding.plan_id),
            binding,
        )?;
        let plan_revision = binding_reference(
            store.get_plan_revision(
                project_id,
                issue_id,
                &binding.plan_id,
                &binding.plan_revision_id,
            ),
            binding,
        )?;
        let dependency_graph = binding_reference(
            store.get_dependency_graph_revision(
                &lineage,
                &plan_revision.dependency_graph_revision_id,
            ),
            binding,
        )?;
        let plan_projection_bundle = binding_reference(
            store.get_plan_projection_bundle(&lineage, &plan_revision.plan_projection_bundle_id),
            binding,
        )?;
        let logical_work_item = binding_reference(
            store.get_logical_work_item(&lineage, &binding.logical_work_item_id),
            binding,
        )?;
        let work_item_revision = binding_reference(
            store.get_work_item_revision(
                &lineage,
                &binding.logical_work_item_id,
                &binding.work_item_revision_id,
            ),
            binding,
        )?;
        let verification_plan_revision = binding_reference(
            store.get_verification_plan_revision(&lineage, &binding.verification_plan_revision_id),
            binding,
        )?;
        let projection_bundle = binding_reference(
            store.get_work_item_projection_bundle(&lineage, &binding.projection_bundle_id),
            binding,
        )?;
        let human_presentation = binding_reference(
            store.get_latest_human_presentation_revision(&lineage, &binding.projection_bundle_id),
            binding,
        )?;

        let valid = plan_revision.plan_id == binding.plan_id
            && plan_revision
                .work_item_bindings
                .get(&binding.logical_work_item_id)
                == Some(&binding.work_item_revision_id)
            && dependency_graph.id == plan_revision.dependency_graph_revision_id
            && dependency_graph.plan_id == binding.plan_id
            && plan_projection_bundle.id == plan_revision.plan_projection_bundle_id
            && plan_projection_bundle.plan_revision_id == binding.plan_revision_id
            && plan_projection_bundle.dependency_graph_revision_id
                == plan_revision.dependency_graph_revision_id
            && plan_projection_bundle
                .work_item_projection_bundle_refs
                .contains(&binding.projection_bundle_id)
            && logical_work_item.id == binding.logical_work_item_id
            && logical_work_item.plan_id == binding.plan_id
            && work_item_revision.logical_work_item_id == binding.logical_work_item_id
            && work_item_revision.id == binding.work_item_revision_id
            && work_item_revision.work_item_projection_bundle_id == binding.projection_bundle_id
            && work_item_revision.verification_plan_revision_id
                == binding.verification_plan_revision_id
            && work_item_revision.canonical_contract_hash == binding.canonical_contract_hash
            && verification_plan_revision.id == binding.verification_plan_revision_id
            && verification_plan_revision.logical_work_item_id == binding.logical_work_item_id
            && verification_plan_revision.source_draft_revision_id
                == work_item_revision.source_draft_revision_id
            && projection_bundle.id == binding.projection_bundle_id
            && projection_bundle.work_item_revision_id == binding.work_item_revision_id
            && projection_bundle.canonical_contract_hash == binding.canonical_contract_hash
            && projection_bundle.compiler_version == binding.projection_compiler_version
            && projection_bundle.human_projection_hash == binding.human_projection_hash
            && projection_bundle.coder_projection_hash == binding.coder_projection_hash
            && projection_bundle.reviewer_projection_hash == binding.reviewer_projection_hash
            && projection_bundle.human_projection.logical_work_item_id
                == binding.logical_work_item_id
            && projection_bundle.coder_projection.work_item_revision_id
                == binding.work_item_revision_id
            && projection_bundle.reviewer_projection.work_item_revision_id
                == binding.work_item_revision_id;
        if !valid {
            return Err(runtime_binding_integrity_mismatch(
                &binding.logical_work_item_id,
            ));
        }

        Ok(ResolvedWorkItemRuntime {
            binding: binding.clone(),
            lineage,
            plan_revision,
            dependency_graph,
            work_item_revision,
            verification_plan_revision,
            projection_bundle,
            plan_projection_bundle,
            human_presentation,
        })
    }

    pub fn resolve_workspace(
        &self,
        session: &WorkspaceSessionRecord,
    ) -> Result<ResolvedWorkItemRuntime, ProductStoreError> {
        if session.workspace_type != WorkspaceType::WorkItem {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "runtime_workspace_type",
                id: session.id.clone(),
            });
        }
        let binding = session
            .work_item_runtime_binding
            .as_ref()
            .ok_or_else(|| runtime_binding_missing(&session.id))?;
        if session.entity_id != binding.logical_work_item_id {
            return Err(runtime_binding_integrity_mismatch(
                &binding.logical_work_item_id,
            ));
        }
        self.resolve_binding(&session.project_id, &session.issue_id, binding)
    }

    pub fn resolve_coding_unit(
        &self,
        attempt: &CodingExecutionAttempt,
        unit: &CodingExecutionUnit,
        run: Option<&CodingUnitRun>,
    ) -> Result<ResolvedWorkItemRuntime, ProductStoreError> {
        let unit_id = unit.logical_work_item_id.as_str();
        let expected_plan_id = attempt.work_item_group_id.as_deref();
        if attempt.scope != CodingAttemptScope::WorkItemGroup
            || expected_plan_id != Some(unit.plan_id.as_str())
            || attempt.id != unit.attempt_id
            || attempt.project_id != unit.project_id
            || attempt.issue_id != unit.issue_id
        {
            return Err(runtime_binding_integrity_mismatch(unit_id));
        }

        let attempt_store = CodingAttemptStore::new(self.paths.clone());
        let plan_binding =
            attempt_store
                .get_plan_binding(attempt)
                .map_err(|error| match error {
                    ProductStoreError::NotFound { .. } => runtime_binding_missing(&attempt.id),
                    ProductStoreError::IdentityMismatch { .. } => {
                        runtime_binding_integrity_mismatch(unit_id)
                    }
                    other => other,
                })?;
        if plan_binding.attempt_id != attempt.id
            || plan_binding.plan_id != unit.plan_id
            || plan_binding.bound_plan_revision_id.is_empty()
        {
            return Err(runtime_binding_integrity_mismatch(unit_id));
        }

        let revision_store = WorkItemRevisionStore::new(self.paths.clone());
        let lineage = runtime_reference(
            revision_store.get_plan_lineage(&unit.project_id, &unit.issue_id, &unit.plan_id),
            unit_id,
        )?;
        let work_item_revision = runtime_reference(
            revision_store.get_work_item_revision(
                &lineage,
                &unit.logical_work_item_id,
                &unit.work_item_revision_id,
            ),
            unit_id,
        )?;
        let projection_bundle = runtime_reference(
            revision_store.get_work_item_projection_bundle(
                &lineage,
                &work_item_revision.work_item_projection_bundle_id,
            ),
            unit_id,
        )?;
        let binding = WorkItemRuntimeBinding {
            plan_id: unit.plan_id.clone(),
            plan_revision_id: plan_binding.bound_plan_revision_id,
            logical_work_item_id: unit.logical_work_item_id.clone(),
            work_item_revision_id: unit.work_item_revision_id.clone(),
            projection_bundle_id: projection_bundle.id.clone(),
            verification_plan_revision_id: work_item_revision.verification_plan_revision_id.clone(),
            canonical_contract_hash: work_item_revision.canonical_contract_hash.clone(),
            projection_compiler_version: projection_bundle.compiler_version.clone(),
            human_projection_hash: projection_bundle.human_projection_hash.clone(),
            coder_projection_hash: projection_bundle.coder_projection_hash.clone(),
            reviewer_projection_hash: projection_bundle.reviewer_projection_hash.clone(),
        };
        let resolved = self.resolve_binding(&unit.project_id, &unit.issue_id, &binding)?;

        if let Some(run) = run {
            let valid = run.unit_id == unit.id
                && run.work_item_revision_id == binding.work_item_revision_id
                && run.canonical_contract_hash == binding.canonical_contract_hash
                && run.projection_bundle_id == binding.projection_bundle_id
                && run.projection_compiler_version == binding.projection_compiler_version
                && run.coder_projection_hash == binding.coder_projection_hash
                && run.reviewer_projection_hash == binding.reviewer_projection_hash;
            if !valid {
                return Err(runtime_binding_integrity_mismatch(unit_id));
            }
        }

        Ok(resolved)
    }
}

fn validate_binding(binding: &WorkItemRuntimeBinding) -> Result<(), ProductStoreError> {
    for value in [
        binding.plan_id.as_str(),
        binding.plan_revision_id.as_str(),
        binding.logical_work_item_id.as_str(),
        binding.work_item_revision_id.as_str(),
        binding.projection_bundle_id.as_str(),
        binding.verification_plan_revision_id.as_str(),
        binding.canonical_contract_hash.as_str(),
        binding.projection_compiler_version.as_str(),
        binding.human_projection_hash.as_str(),
        binding.coder_projection_hash.as_str(),
        binding.reviewer_projection_hash.as_str(),
    ] {
        validate_relative_id(value)?;
    }
    Ok(())
}

fn runtime_binding_integrity_mismatch(id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind: "runtime_binding_integrity_mismatch",
        id: id.to_string(),
    }
}

fn runtime_binding_missing(id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind: "runtime_binding_missing",
        id: id.to_string(),
    }
}

fn binding_reference<T>(
    result: Result<T, ProductStoreError>,
    binding: &WorkItemRuntimeBinding,
) -> Result<T, ProductStoreError> {
    runtime_reference(result, &binding.logical_work_item_id)
}

fn runtime_reference<T>(
    result: Result<T, ProductStoreError>,
    id: &str,
) -> Result<T, ProductStoreError> {
    result.map_err(|error| match error {
        ProductStoreError::NotFound { .. } | ProductStoreError::IdentityMismatch { .. } => {
            runtime_binding_integrity_mismatch(id)
        }
        other => other,
    })
}
