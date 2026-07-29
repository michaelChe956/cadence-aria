use super::*;
use crate::product::coding_models::{CodingAttemptScope, CodingExecutionUnit, CodingUnitRunStatus};
use crate::product::models::{HandoffRevision, WorkItemPlanLineage};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::product::work_item_runtime_reader::{ResolvedWorkItemRuntime, WorkItemRuntimeReader};

pub(super) struct SchemaV2GroupCompletionGateFacts {
    pub(super) runtime: ResolvedWorkItemRuntime,
    pub(super) handoff: HandoffRevision,
}

impl CodingWorkspaceEngine {
    pub(crate) fn schema_v2_group_plan_lineage(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Option<WorkItemPlanLineage>, CodingWorkspaceEngineError> {
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Ok(None);
        }
        let Some(plan_id) = attempt.work_item_group_id.as_deref() else {
            return Ok(None);
        };
        match WorkItemRevisionStore::new(self.store.paths()).get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            plan_id,
        ) {
            Ok(lineage) => Ok(Some(lineage)),
            Err(ProductStoreError::NotFound {
                kind: "work_item_plan_lineage",
                ..
            }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub(super) fn schema_v2_group_completion_gate_facts(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Vec<SchemaV2GroupCompletionGateFacts>, CodingWorkspaceEngineError> {
        let Some(lineage) = self.schema_v2_group_plan_lineage(attempt)? else {
            return Ok(Vec::new());
        };
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let reader = WorkItemRuntimeReader::new(self.store.paths());
        self.store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .filter(|unit| {
                unit.status == crate::product::coding_models::CodingExecutionUnitStatus::Completed
            })
            .map(|unit| {
                self.schema_v2_group_completion_gate_facts_for_unit(
                    attempt,
                    &lineage,
                    &revision_store,
                    &reader,
                    unit,
                )
            })
            .collect()
    }

    fn schema_v2_group_completion_gate_facts_for_unit(
        &self,
        attempt: &CodingExecutionAttempt,
        lineage: &WorkItemPlanLineage,
        revision_store: &WorkItemRevisionStore,
        reader: &WorkItemRuntimeReader,
        unit: CodingExecutionUnit,
    ) -> Result<SchemaV2GroupCompletionGateFacts, CodingWorkspaceEngineError> {
        let run = self
            .store
            .list_coding_unit_runs(attempt, &unit.id)?
            .into_iter()
            .max_by_key(|run| run.execution_no)
            .ok_or_else(|| ProductStoreError::IdentityMismatch {
                kind: "runtime_binding_missing",
                id: unit.id.clone(),
            })?;
        if run.status != CodingUnitRunStatus::Completed {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "runtime_binding_integrity_mismatch",
                id: unit.logical_work_item_id.clone(),
            }
            .into());
        }
        let runtime = reader.normative_context_for_unit(attempt, &unit, Some(&run))?;
        let handoff_id = unit.latest_handoff_revision_id.as_deref().ok_or_else(|| {
            ProductStoreError::IdentityMismatch {
                kind: "runtime_binding_missing",
                id: unit.id.clone(),
            }
        })?;
        let handoff =
            revision_store.get_handoff_revision(lineage, &unit.logical_work_item_id, handoff_id)?;
        if handoff.work_item_revision_id != runtime.binding.work_item_revision_id
            || handoff.coding_unit_run_id != run.id
            || handoff.logical_work_item_id != runtime.binding.logical_work_item_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "runtime_binding_integrity_mismatch",
                id: unit.logical_work_item_id,
            }
            .into());
        }
        Ok(SchemaV2GroupCompletionGateFacts { runtime, handoff })
    }

    /// 某个已完成 unit 的 completion commit 实际改动的文件清单。
    ///
    /// 组完成写入范围门禁的唯一数据源。worktree 缺失时必须失败关闭；commit
    /// 存在时必须取到真实 git 事实，不得用空清单让范围校验静默空转。
    pub(crate) async fn changed_files_for_unit_completion_commit(
        &self,
        attempt: &CodingExecutionAttempt,
        completion_commit: &str,
    ) -> Result<Vec<String>, CodingWorkspaceEngineError> {
        let worktree_path = self.attempt_worktree_path(attempt).await?;
        if !worktree_path.exists() {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        }
        Ok(self
            ._git_service
            .git_commit_changed_files(&worktree_path, completion_commit)
            .await?)
    }

    pub(super) fn validate_changed_files_for_runtime(
        &self,
        runtime: &ResolvedWorkItemRuntime,
        changed_files: &[String],
        worktree_path: Option<&PathBuf>,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let write_policy = &runtime.work_item_revision.canonical_contract.write_policy;
        for relative_path in changed_files {
            let candidate = std::path::Path::new(relative_path);
            if write_policy
                .forbidden_scopes
                .iter()
                .any(|scope| scope_allows_path(scope, relative_path, true))
            {
                return Err(CodingWorkspaceEngineError::WorkItemDiffScopeViolation(
                    relative_path.clone(),
                ));
            }
            if !write_policy.exclusive_scopes.is_empty()
                && let Some(base) = worktree_path
            {
                let _ = validate_write_path(base, &write_policy.exclusive_scopes, candidate, true)
                    .map_err(|_| {
                        CodingWorkspaceEngineError::WorkItemDiffScopeViolation(
                            relative_path.clone(),
                        )
                    })?;
            }
        }
        Ok(())
    }
}
