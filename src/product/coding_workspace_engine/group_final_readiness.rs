#![allow(dead_code)]

use super::*;

use chrono::Utc;

use crate::product::coding_models::{
    CodeReviewReport, CodingAttemptScope, CodingExecutionAttempt, CodingExecutionUnit,
    CodingUnitRunStatus, GroupFinalReadinessDiagnostic, GroupFinalReadinessDiagnosticKind,
    GroupFinalReadinessSnapshot, GroupFinalReadinessStatus, GroupFinalReadinessUnit,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

impl CodingWorkspaceEngine {
    /// 从 group attempt 的权威 unit/run/review/handoff/plan 记录构建只读 readiness 证据。
    ///
    /// 该路径只读取 store 与 git 事实；它不会调用 provider，也不会基于 review
    /// verdict 生成 finding、rework 或 Plan Repair 副作用。
    pub(crate) async fn build_group_final_readiness_snapshot(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<GroupFinalReadinessSnapshot, CodingWorkspaceEngineError> {
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                attempt.id.clone(),
            ));
        }

        // readiness 依赖 git 区间事实。环境缺失属于 attempt 级错误，不能静默
        // 写入缺少提交证据的 snapshot。
        let worktree_path = self.attempt_worktree_path(attempt).await?;
        if !worktree_path.exists() {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        }

        let units =
            self.store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let mut snapshot = GroupFinalReadinessSnapshot {
            attempt_id: attempt.id.clone(),
            status: GroupFinalReadinessStatus::Incomplete,
            units: Vec::new(),
            diagnostics: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
        };

        if units.is_empty() {
            snapshot.diagnostics.push(GroupFinalReadinessDiagnostic {
                kind: GroupFinalReadinessDiagnosticKind::UnitRunMissing,
                unit_id: None,
                message: format!("attempt {} has no coding units", attempt.id),
            });
            self.store
                .write_group_final_readiness_snapshot(attempt, &snapshot)?;
            return self
                .store
                .get_group_final_readiness_snapshot(attempt)?
                .ok_or_else(|| crate::product::json_store::ProductStoreError::NotFound {
                    kind: "group_final_readiness_snapshot",
                    id: attempt.id.clone(),
                })
                .map_err(Into::into);
        }

        let plan_id = attempt.work_item_group_id.as_deref();
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let lineage = match plan_id {
            Some(id) => {
                match revision_store.get_plan_lineage(&attempt.project_id, &attempt.issue_id, id) {
                    Ok(lineage) => Some(lineage),
                    Err(crate::product::json_store::ProductStoreError::NotFound { .. }) => None,
                    Err(error) => return Err(error.into()),
                }
            }
            None => None,
        };
        let binding = match self.store.get_plan_binding(attempt) {
            Ok(binding) => Some(binding),
            Err(crate::product::json_store::ProductStoreError::NotFound { .. }) => None,
            Err(error) => return Err(error.into()),
        };

        let active_plan = match (lineage.as_ref(), binding.as_ref()) {
            (Some(lineage), Some(binding)) => match revision_store.get_plan_revision(
                &attempt.project_id,
                &attempt.issue_id,
                &lineage.id,
                &binding.bound_plan_revision_id,
            ) {
                Ok(plan) => Some(plan),
                Err(crate::product::json_store::ProductStoreError::NotFound { .. }) => None,
                Err(error) => return Err(error.into()),
            },
            _ => None,
        };

        let reports = self.store.list_code_review_reports(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;

        for unit in units {
            let plan_binding_matches = binding.as_ref().is_some_and(|binding| {
                binding.plan_id == unit.plan_id
                    && lineage.as_ref().is_some_and(|lineage| {
                        lineage.active_revision_id.as_deref()
                            == Some(binding.bound_plan_revision_id.as_str())
                    })
                    && active_plan.as_ref().is_some_and(|plan| {
                        plan.work_item_bindings.get(&unit.logical_work_item_id)
                            == Some(&unit.work_item_revision_id)
                    })
            });
            snapshot.units.push(
                self.build_group_final_readiness_unit(
                    attempt,
                    &worktree_path,
                    &reports,
                    &unit,
                    binding.as_ref(),
                    lineage.as_ref(),
                    plan_binding_matches,
                )
                .await?,
            );
            self.append_unit_diagnostics(
                &unit,
                &snapshot.units.last().expect("unit was just pushed"),
                &reports,
                binding.as_ref(),
                plan_binding_matches,
                &mut snapshot.diagnostics,
            );
        }

        snapshot.status = if snapshot.diagnostics.is_empty() {
            GroupFinalReadinessStatus::Complete
        } else {
            GroupFinalReadinessStatus::Incomplete
        };
        self.store
            .write_group_final_readiness_snapshot(attempt, &snapshot)?;
        self.store
            .get_group_final_readiness_snapshot(attempt)?
            .ok_or_else(|| crate::product::json_store::ProductStoreError::NotFound {
                kind: "group_final_readiness_snapshot",
                id: attempt.id.clone(),
            })
            .map_err(Into::into)
    }

    async fn build_group_final_readiness_unit(
        &self,
        attempt: &CodingExecutionAttempt,
        worktree_path: &std::path::Path,
        reports: &[CodeReviewReport],
        unit: &CodingExecutionUnit,
        binding: Option<&crate::product::coding_models::CodingAttemptPlanBinding>,
        lineage: Option<&crate::product::models::WorkItemPlanLineage>,
        plan_binding_matches: bool,
    ) -> Result<GroupFinalReadinessUnit, CodingWorkspaceEngineError> {
        let mut result = GroupFinalReadinessUnit {
            unit_id: unit.id.clone(),
            logical_work_item_id: unit.logical_work_item_id.clone(),
            ..Default::default()
        };

        let runs = self
            .store
            .list_unit_runs_by_logical_id(attempt, &unit.logical_work_item_id)?;
        let run = runs
            .into_iter()
            .filter(|run| run.status == CodingUnitRunStatus::Completed)
            .max_by(|left, right| {
                left.execution_no
                    .cmp(&right.execution_no)
                    .then_with(|| left.id.cmp(&right.id))
            });
        let Some(run) = run else {
            return Ok(result);
        };

        result.unit_run_id = Some(run.id.clone());
        result.start_commit = run.start_commit.clone();
        result.completion_commit = run.completion_commit.clone();
        result.handoff_revision_id = run.resolved_handoff_revision_ids.last().cloned();
        result.plan_revision_id = plan_binding_matches
            .then(|| binding.map(|binding| binding.bound_plan_revision_id.clone()))
            .flatten();

        if let Some(handoff_id) = result.handoff_revision_id.as_deref()
            && let Some(lineage) = lineage
        {
            match WorkItemRevisionStore::new(self.store.paths()).get_handoff_revision(
                lineage,
                &unit.logical_work_item_id,
                handoff_id,
            ) {
                Ok(handoff)
                    if handoff.coding_unit_run_id == run.id
                        && handoff.work_item_revision_id == run.work_item_revision_id
                        && handoff.logical_work_item_id == unit.logical_work_item_id => {}
                Ok(_) | Err(crate::product::json_store::ProductStoreError::NotFound { .. }) => {
                    result.handoff_revision_id = None;
                }
                Err(error) => return Err(error.into()),
            }
        }

        let report = reports
            .iter()
            .filter(|report| report.unit_run_id.as_deref() == Some(run.id.as_str()))
            .max_by(|left, right| {
                left.created_at
                    .cmp(&right.created_at)
                    .then_with(|| left.round.cmp(&right.round))
                    .then_with(|| left.id.cmp(&right.id))
            });
        if let Some(report) = report {
            result.code_review_report_id = Some(report.id.clone());
            result.review_verdict = Some(report.verdict.clone());
            result.review_summary = Some(report.summary.clone());
            result.review_findings = Some(report.findings.clone());
            result.review_raw_provider_output_ref = report.raw_provider_output_ref.clone();
        }

        if let (Some(start), Some(completion)) = (
            run.start_commit.as_deref(),
            run.completion_commit.as_deref(),
        ) {
            if start == completion {
                result.empty_observation = true;
            } else {
                result.commit_shas = self
                    ._git_service
                    .git_commit_range_commits(worktree_path, start, completion)
                    .await?;
                let _changed_files = self
                    ._git_service
                    .git_commit_range_changed_files(worktree_path, start, completion)
                    .await?;
                result.diff_ref = format!("{start}..{completion}");
            }
        }
        Ok(result)
    }

    fn append_unit_diagnostics(
        &self,
        unit: &CodingExecutionUnit,
        snapshot_unit: &GroupFinalReadinessUnit,
        reports: &[CodeReviewReport],
        binding: Option<&crate::product::coding_models::CodingAttemptPlanBinding>,
        plan_binding_matches: bool,
        diagnostics: &mut Vec<GroupFinalReadinessDiagnostic>,
    ) {
        let unit_id = Some(unit.id.clone());
        let Some(run_id) = snapshot_unit.unit_run_id.as_deref() else {
            diagnostics.push(diagnostic(
                GroupFinalReadinessDiagnosticKind::UnitRunMissing,
                unit_id.clone(),
                format!("unit {} has no completed unit run", unit.id),
            ));
            if !plan_binding_matches {
                diagnostics.push(plan_binding_diagnostic(unit, binding));
            }
            return;
        };
        if snapshot_unit.completion_commit.is_none() {
            diagnostics.push(diagnostic(
                GroupFinalReadinessDiagnosticKind::CompletionCommitMissing,
                unit_id.clone(),
                format!("completed unit run {run_id} has no completion commit"),
            ));
        } else if snapshot_unit.start_commit.is_none() {
            diagnostics.push(diagnostic(
                GroupFinalReadinessDiagnosticKind::IdentityMismatch,
                unit_id.clone(),
                format!("completed unit run {run_id} has no start commit"),
            ));
        }
        if !reports
            .iter()
            .any(|report| report.unit_run_id.as_deref() == Some(run_id))
        {
            diagnostics.push(diagnostic(
                GroupFinalReadinessDiagnosticKind::CodeReviewMissing,
                unit_id.clone(),
                format!("unit run {run_id} has no independent code review report"),
            ));
        }
        if snapshot_unit.handoff_revision_id.is_none() {
            diagnostics.push(diagnostic(
                GroupFinalReadinessDiagnosticKind::HandoffMissing,
                unit_id.clone(),
                format!("unit run {run_id} has no resolved handoff revision"),
            ));
        }
        if !plan_binding_matches {
            diagnostics.push(plan_binding_diagnostic(unit, binding));
        }
    }
}

fn diagnostic(
    kind: GroupFinalReadinessDiagnosticKind,
    unit_id: Option<String>,
    message: String,
) -> GroupFinalReadinessDiagnostic {
    GroupFinalReadinessDiagnostic {
        kind,
        unit_id,
        message,
    }
}

fn plan_binding_diagnostic(
    unit: &CodingExecutionUnit,
    binding: Option<&crate::product::coding_models::CodingAttemptPlanBinding>,
) -> GroupFinalReadinessDiagnostic {
    let detail = binding
        .map(|binding| {
            format!(
                "bound plan revision {} is not active",
                binding.bound_plan_revision_id
            )
        })
        .unwrap_or_else(|| "attempt plan binding is missing or unreadable".to_string());
    diagnostic(
        GroupFinalReadinessDiagnosticKind::PlanBindingMismatch,
        Some(unit.id.clone()),
        format!("unit {} plan binding mismatch: {detail}", unit.id),
    )
}
