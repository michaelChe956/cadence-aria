use super::*;

use chrono::Utc;

use crate::product::coding_models::{
    CodeReviewReport, CodingAttemptScope, CodingExecutionAttempt, CodingExecutionUnit,
    CodingUnitRunStatus, GroupFinalReadinessDiagnostic, GroupFinalReadinessDiagnosticKind,
    GroupFinalReadinessSnapshot, GroupFinalReadinessStatus, GroupFinalReadinessUnit,
};
use crate::product::models::HandoffRevision;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

pub(crate) fn handoff_matches_unit_run(
    handoff: &HandoffRevision,
    unit: &CodingExecutionUnit,
    run: &crate::product::coding_models::CodingUnitRun,
) -> bool {
    handoff.coding_unit_run_id == run.id
        && handoff.work_item_revision_id == run.work_item_revision_id
        && handoff.logical_work_item_id == unit.logical_work_item_id
        && run
            .completion_commit
            .as_deref()
            .is_some_and(|completion_commit| handoff.commit_sha == completion_commit)
}

#[derive(Clone, Copy)]
struct GroupFinalReadinessPlanContext<'a> {
    binding: Option<&'a crate::product::coding_models::CodingAttemptPlanBinding>,
    lineage: Option<&'a crate::product::models::WorkItemPlanLineage>,
    binding_matches: bool,
}

impl CodingWorkspaceEngine {
    /// 从 group attempt 的权威 unit/run/review/handoff/plan 记录构建只读 readiness 证据。
    ///
    /// 该路径只读取 store 与 git 事实；它不会调用 provider，也不会基于 review
    /// verdict 生成 finding、返工或 Plan Repair 副作用。
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
            let plan_context = GroupFinalReadinessPlanContext {
                binding: binding.as_ref(),
                lineage: lineage.as_ref(),
                binding_matches: plan_binding_matches,
            };
            let (snapshot_unit, handoff_diagnostic) = self
                .build_group_final_readiness_unit(
                    attempt,
                    &worktree_path,
                    &reports,
                    &unit,
                    plan_context,
                )
                .await?;
            snapshot.units.push(snapshot_unit);
            self.append_unit_diagnostics(
                &unit,
                snapshot.units.last().expect("unit was just pushed"),
                &reports,
                plan_context,
                handoff_diagnostic,
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
        plan_context: GroupFinalReadinessPlanContext<'_>,
    ) -> Result<
        (
            GroupFinalReadinessUnit,
            Option<GroupFinalReadinessDiagnostic>,
        ),
        CodingWorkspaceEngineError,
    > {
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
            return Ok((result, None));
        };

        result.unit_run_id = Some(run.id.clone());
        result.start_commit = run.start_commit.clone();
        result.completion_commit = run.completion_commit.clone();
        let handoff_id = unit.latest_handoff_revision_id.clone();
        result.handoff_revision_id = handoff_id.clone();
        result.plan_revision_id = plan_context
            .binding_matches
            .then(|| {
                plan_context
                    .binding
                    .map(|binding| binding.bound_plan_revision_id.clone())
            })
            .flatten();
        let handoff_diagnostic = match (handoff_id.as_deref(), plan_context.lineage) {
            (None, _) => Some(diagnostic(
                GroupFinalReadinessDiagnosticKind::HandoffMissing,
                Some(unit.id.clone()),
                format!(
                    "unit run {} has no published output handoff revision",
                    run.id
                ),
            )),
            (Some(handoff_id), Some(lineage)) => {
                match WorkItemRevisionStore::new(self.store.paths()).get_handoff_revision(
                    lineage,
                    &unit.logical_work_item_id,
                    handoff_id,
                ) {
                    Ok(handoff) if handoff_matches_unit_run(&handoff, unit, &run) => None,
                    Ok(_) => {
                        result.handoff_revision_id = None;
                        Some(diagnostic(
                            GroupFinalReadinessDiagnosticKind::IdentityMismatch,
                            Some(unit.id.clone()),
                            format!(
                                "published output handoff revision {handoff_id} does not match unit run {} identity",
                                run.id
                            ),
                        ))
                    }
                    Err(crate::product::json_store::ProductStoreError::NotFound { .. }) => {
                        result.handoff_revision_id = None;
                        Some(diagnostic(
                            GroupFinalReadinessDiagnosticKind::HandoffMissing,
                            Some(unit.id.clone()),
                            format!(
                                "published output handoff revision {handoff_id} for unit run {} was not found",
                                run.id
                            ),
                        ))
                    }
                    Err(crate::product::json_store::ProductStoreError::IdentityMismatch {
                        ..
                    }) => {
                        result.handoff_revision_id = None;
                        Some(diagnostic(
                            GroupFinalReadinessDiagnosticKind::IdentityMismatch,
                            Some(unit.id.clone()),
                            format!(
                                "published output handoff revision {handoff_id} does not match unit run {} identity",
                                run.id
                            ),
                        ))
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            (Some(handoff_id), None) => {
                result.handoff_revision_id = None;
                Some(diagnostic(
                    GroupFinalReadinessDiagnosticKind::HandoffMissing,
                    Some(unit.id.clone()),
                    format!(
                        "published output handoff revision {handoff_id} for unit run {} cannot be read without plan lineage",
                        run.id
                    ),
                ))
            }
        };

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
        Ok((result, handoff_diagnostic))
    }

    fn append_unit_diagnostics(
        &self,
        unit: &CodingExecutionUnit,
        snapshot_unit: &GroupFinalReadinessUnit,
        reports: &[CodeReviewReport],
        plan_context: GroupFinalReadinessPlanContext<'_>,
        handoff_diagnostic: Option<GroupFinalReadinessDiagnostic>,
        diagnostics: &mut Vec<GroupFinalReadinessDiagnostic>,
    ) {
        let unit_id = Some(unit.id.clone());
        let Some(run_id) = snapshot_unit.unit_run_id.as_deref() else {
            diagnostics.push(diagnostic(
                GroupFinalReadinessDiagnosticKind::UnitRunMissing,
                unit_id.clone(),
                format!("unit {} has no completed unit run", unit.id),
            ));
            if !plan_context.binding_matches {
                diagnostics.push(plan_binding_diagnostic(unit, plan_context.binding));
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
        if let Some(handoff_diagnostic) = handoff_diagnostic {
            diagnostics.push(handoff_diagnostic);
        }
        if !plan_context.binding_matches {
            diagnostics.push(plan_binding_diagnostic(unit, plan_context.binding));
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
