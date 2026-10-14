//! 跨 attempt（跨 target 桶）依赖 handoff 解析（S1）。
//!
//! 多 target 拆分增殖形态下，一个 plan 的 WI 分属多个 target-attempt 桶，依赖边可以
//! 越过桶。权威 handoff 解析先查本 attempt（命中走原逻辑，同桶与单仓零变化）；本桶
//! 未命中才在同 plan 的其他 attempt 中找 owner，并对 owner 做与同桶完全相同的绑定
//! 校验。解析全程 fail-closed：找不到合格 owner 一律 `WorkItemHandoffMissing`，消息
//! 带上各 owner attempt 与 unit 的实际状态，便于定位。

use super::*;
use crate::product::coding_models::{
    CodingExecutionUnit, CodingExecutionUnitStatus, CodingUnitRunStatus,
};
use crate::product::models::WorkItemPlanLineage;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

impl CodingWorkspaceEngine {
    /// 解析 `dependency_id` 的权威 handoff id：本桶优先，未命中再跨 attempt。
    pub(super) fn resolve_dependency_handoff_id(
        &self,
        attempt: &CodingExecutionAttempt,
        local_units: &[CodingExecutionUnit],
        dependency_id: &str,
        lineage: &WorkItemPlanLineage,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let mut local = local_units
            .iter()
            .filter(|candidate| candidate.logical_work_item_id == dependency_id);
        if let Some(dependency) = local.next() {
            if local.next().is_some() {
                return Err(ambiguous_dependency(dependency_id));
            }
            return self.validate_dependency_handoff(attempt, dependency, lineage);
        }
        let (owner, dependency) = self.cross_attempt_dependency(attempt, dependency_id, lineage)?;
        self.validate_dependency_handoff(&owner, &dependency, lineage)
    }

    /// 对 owner attempt 内的依赖 unit 做 handoff 绑定校验（同桶与跨桶共用）。
    fn validate_dependency_handoff(
        &self,
        owner: &CodingExecutionAttempt,
        dependency: &CodingExecutionUnit,
        lineage: &WorkItemPlanLineage,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let dependency_id = &dependency.logical_work_item_id;
        let handoff_id = dependency
            .latest_handoff_revision_id
            .as_deref()
            .ok_or_else(|| {
                CodingWorkspaceEngineError::WorkItemHandoffMissing(dependency_id.clone())
            })?;
        let handoff = WorkItemRevisionStore::new(self.store.paths()).get_handoff_revision(
            lineage,
            dependency_id,
            handoff_id,
        )?;
        let source_run = self
            .store
            .list_coding_unit_runs(owner, &dependency.id)?
            .into_iter()
            .max_by_key(|run| run.execution_no);
        if dependency.status != CodingExecutionUnitStatus::Completed
            || dependency.completion_commit.as_deref() != Some(handoff.commit_sha.as_str())
            || handoff.id != format!("handoff_revision_{}", handoff.coding_unit_run_id)
            || handoff.logical_work_item_id != dependency.logical_work_item_id
            || handoff.work_item_revision_id != dependency.work_item_revision_id
            || !source_run.is_some_and(|run| {
                run.id == handoff.coding_unit_run_id
                    && run.unit_id == dependency.id
                    && run.work_item_revision_id == dependency.work_item_revision_id
                    && run.status == CodingUnitRunStatus::Completed
                    && run.completion_commit.as_deref() == Some(handoff.commit_sha.as_str())
            })
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "unit_run_handoff_binding_mismatch: {handoff_id}"
            )));
        }
        Ok(handoff_id.to_string())
    }

    /// 在同 plan 的其他 attempt 中选出 `dependency_id` 的 owner：unit 已 Completed、
    /// 已发布 handoff、有 completion commit，且 `work_item_revision_id` 与当前 attempt
    /// 绑定的 plan revision 一致；多个候选取 `attempt_no` 最大者（与 REQ-MTG-04
    /// “每 target 取最新 attempt”一致）。无合格候选 fail-closed。
    fn cross_attempt_dependency(
        &self,
        attempt: &CodingExecutionAttempt,
        dependency_id: &str,
        lineage: &WorkItemPlanLineage,
    ) -> Result<(CodingExecutionAttempt, CodingExecutionUnit), CodingWorkspaceEngineError> {
        let binding = self.store.get_plan_binding(attempt)?;
        let expected_revision = WorkItemRevisionStore::new(self.store.paths())
            .get_plan_revision(
                &attempt.project_id,
                &attempt.issue_id,
                &lineage.id,
                &binding.bound_plan_revision_id,
            )?
            .work_item_bindings
            .get(dependency_id)
            .cloned();

        let mut candidates: Vec<(CodingExecutionAttempt, CodingExecutionUnit)> = Vec::new();
        let mut observed: Vec<String> = Vec::new();
        for owner in self.store.list_attempts_for_work_item_group(
            &attempt.project_id,
            &attempt.issue_id,
            &lineage.id,
        )? {
            if owner.id == attempt.id {
                continue;
            }
            let mut hosted = self
                .store
                .list_coding_units(&owner.project_id, &owner.issue_id, &owner.id)?
                .into_iter()
                .filter(|unit| unit.logical_work_item_id == dependency_id);
            let Some(unit) = hosted.next() else {
                continue;
            };
            if hosted.next().is_some() {
                return Err(ambiguous_dependency(dependency_id));
            }
            observed.push(format!(
                "attempt {} (attempt_no {}) unit {} status {:?} work_item_revision_id {} handoff {} completion_commit {}",
                owner.id,
                owner.attempt_no,
                unit.id,
                unit.status,
                unit.work_item_revision_id,
                unit.latest_handoff_revision_id.as_deref().unwrap_or("none"),
                unit.completion_commit.as_deref().unwrap_or("none"),
            ));
            if unit.status == CodingExecutionUnitStatus::Completed
                && unit.latest_handoff_revision_id.is_some()
                && unit.completion_commit.is_some()
                && expected_revision.as_deref() == Some(unit.work_item_revision_id.as_str())
            {
                candidates.push((owner, unit));
            }
        }
        candidates
            .into_iter()
            .max_by(|left, right| {
                left.0
                    .attempt_no
                    .cmp(&right.0.attempt_no)
                    .then_with(|| left.0.id.cmp(&right.0.id))
            })
            .ok_or_else(|| {
                let detail = if observed.is_empty() {
                    format!("no other attempt in plan {} hosts it", lineage.id)
                } else {
                    format!(
                        "no completed owner with published handoff bound to plan revision {} \
                         (expected work_item_revision_id {}): {}",
                        binding.bound_plan_revision_id,
                        expected_revision.as_deref().unwrap_or("none"),
                        observed.join("; ")
                    )
                };
                CodingWorkspaceEngineError::WorkItemHandoffMissing(format!(
                    "{dependency_id}: {detail}"
                ))
            })
    }
}

fn ambiguous_dependency(dependency_id: &str) -> CodingWorkspaceEngineError {
    ProductStoreError::Ambiguous {
        kind: "coding_execution_unit",
        id: dependency_id.to_string(),
    }
    .into()
}
