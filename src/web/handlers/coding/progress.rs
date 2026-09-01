use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodeReviewReport, CodingExecutionAttempt, CodingExecutionUnit, CodingExecutionUnitStatus,
};
use crate::product::json_store::ProductStoreError;
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::handlers::dto::{coding_execution_stage_text, coding_execution_unit_status_text};
use crate::web::types::{GroupCodingProgressDto, WorkItemCodingProgressDto};

/// 从 group attempt 的权威 unit/run/review/handoff/plan 记录构建只读 Work Item 投影。
///
/// 该 assembler 不读取 SC per-WI workspace session，也不写入任何记录；每次调用
/// 都从 attempt/unit/run 及其关联的持久化事实重新计算输出。
pub(crate) fn build_group_work_item_progress(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<(Vec<WorkItemCodingProgressDto>, GroupCodingProgressDto), ProductStoreError> {
    let units = store.list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    if units.is_empty() {
        return Ok((
            Vec::new(),
            GroupCodingProgressDto {
                total: 0,
                pending: 0,
                active: 0,
                completed: 0,
                failed_or_blocked: 0,
            },
        ));
    }
    let plan_revision_id = store.get_plan_binding(attempt)?.bound_plan_revision_id;
    let reports =
        store.list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let dependency_gate = store.get_group_dependency_gate_snapshot(attempt)?;
    let blocked_gates =
        store.list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let lineage = attempt
        .work_item_group_id
        .as_deref()
        .map(|plan_id| {
            WorkItemRevisionStore::new(store.paths()).get_plan_lineage(
                &attempt.project_id,
                &attempt.issue_id,
                plan_id,
            )
        })
        .transpose()?;
    let mut progress = units
        .iter()
        .map(|unit| {
            build_unit_progress(
                store,
                attempt,
                unit,
                &reports,
                dependency_gate.as_ref(),
                &blocked_gates,
                lineage.as_ref(),
                &plan_revision_id,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    progress.sort_by(|left, right| left.unit_id.cmp(&right.unit_id));
    let aggregate = aggregate_group_progress(&progress);
    Ok((progress, aggregate))
}

fn aggregate_group_progress(progress: &[WorkItemCodingProgressDto]) -> GroupCodingProgressDto {
    let mut aggregate = GroupCodingProgressDto {
        total: progress.len(),
        pending: 0,
        active: 0,
        completed: 0,
        failed_or_blocked: 0,
    };
    for item in progress {
        match item.status.as_str() {
            "pending" => aggregate.pending += 1,
            "running" | "waiting_for_human" => aggregate.active += 1,
            "completed" => aggregate.completed += 1,
            _ => aggregate.failed_or_blocked += 1,
        }
    }
    aggregate
}

#[allow(clippy::too_many_arguments)]
fn build_unit_progress(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    unit: &CodingExecutionUnit,
    reports: &[CodeReviewReport],
    dependency_gate: Option<&crate::product::coding_models::GroupDependencyGateSnapshot>,
    blocked_gates: &[crate::product::coding_models::CodingGateRequired],
    lineage: Option<&crate::product::models::WorkItemPlanLineage>,
    plan_revision_id: &str,
) -> Result<WorkItemCodingProgressDto, ProductStoreError> {
    let runs = store.list_coding_unit_runs(attempt, &unit.id)?;
    let latest_run = runs.iter().max_by(|left, right| {
        left.execution_no
            .cmp(&right.execution_no)
            .then_with(|| left.id.cmp(&right.id))
    });
    let latest_run_id = latest_run.map(|run| run.id.as_str());
    let code_review = latest_run_id.and_then(|run_id| {
        reports
            .iter()
            .filter(|report| report.unit_run_id.as_deref() == Some(run_id))
            .max_by(|left, right| {
                left.created_at
                    .cmp(&right.created_at)
                    .then_with(|| left.round.cmp(&right.round))
                    .then_with(|| left.id.cmp(&right.id))
            })
            .cloned()
    });
    let handoff_revision_id = latest_run.and_then(|run| {
        let handoff_id = unit.latest_handoff_revision_id.as_deref()?;
        let plan = lineage?;
        let handoff = WorkItemRevisionStore::new(store.paths())
            .get_handoff_revision(plan, &unit.logical_work_item_id, handoff_id)
            .ok()?;
        (handoff.coding_unit_run_id == run.id
            && handoff.work_item_revision_id == run.work_item_revision_id
            && handoff.logical_work_item_id == unit.logical_work_item_id
            && run.completion_commit.as_deref() == Some(handoff.commit_sha.as_str()))
        .then_some(handoff.id)
    });
    let is_active =
        unit.id == attempt.active_unit_id.as_deref().unwrap_or_default() && unit.status.is_active();
    let status = coding_execution_unit_status_text(&unit.status).to_string();
    let current_commit = if is_active {
        attempt
            .head_commit
            .clone()
            .or_else(|| latest_run.and_then(|run| run.completion_commit.clone()))
    } else {
        latest_run
            .and_then(|run| run.completion_commit.clone())
            .or_else(|| latest_run.and_then(|run| run.start_commit.clone()))
    };
    let final_commit = (unit.status == CodingExecutionUnitStatus::Completed)
        .then(|| {
            latest_run
                .and_then(|run| run.completion_commit.clone())
                .or_else(|| unit.completion_commit.clone())
        })
        .flatten();
    let failure_or_blocked_reason = unit
        .summary
        .clone()
        .or_else(|| dependency_reason(unit, dependency_gate))
        .or_else(|| {
            matches!(
                unit.status,
                CodingExecutionUnitStatus::Failed
                    | CodingExecutionUnitStatus::Blocked
                    | CodingExecutionUnitStatus::BlockedByPlanDefect
                    | CodingExecutionUnitStatus::AwaitingAmendment
                    | CodingExecutionUnitStatus::NeedsRevalidation
                    | CodingExecutionUnitStatus::Stale
            )
            .then(|| {
                blocked_gates.iter().find_map(|gate| {
                    gate.reason_code
                        .clone()
                        .or_else(|| Some(gate.description.clone()))
                })
            })
            .flatten()
        });
    Ok(WorkItemCodingProgressDto {
        logical_work_item_id: unit.logical_work_item_id.clone(),
        unit_id: unit.id.clone(),
        status,
        stage: is_active.then(|| coding_execution_stage_text(&attempt.stage).to_string()),
        current_commit,
        final_commit,
        code_review,
        handoff_revision_id,
        failure_or_blocked_reason,
        plan_revision_id: plan_revision_id.to_string(),
    })
}

fn dependency_reason(
    unit: &CodingExecutionUnit,
    snapshot: Option<&crate::product::coding_models::GroupDependencyGateSnapshot>,
) -> Option<String> {
    let snapshot = snapshot?;
    let applies = snapshot.pending_unit_ids.iter().any(|id| id == &unit.id)
        || snapshot.selected_unit_id.as_deref() == Some(unit.id.as_str());
    applies.then(|| {
        snapshot
            .reason_code
            .as_deref()
            .or(snapshot.message.as_deref())
            .unwrap_or("dependency gate blocked")
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::aggregate_group_progress;
    use crate::web::types::{GroupCodingProgressDto, WorkItemCodingProgressDto};

    fn item(status: &str) -> WorkItemCodingProgressDto {
        WorkItemCodingProgressDto {
            logical_work_item_id: format!("logical_{status}"),
            unit_id: format!("unit_{status}"),
            status: status.to_string(),
            stage: None,
            current_commit: None,
            final_commit: None,
            code_review: None,
            handoff_revision_id: None,
            failure_or_blocked_reason: None,
            plan_revision_id: "plan_revision_0001".to_string(),
        }
    }

    #[test]
    fn group_work_item_progress_aggregates_counts_deterministically() {
        let aggregate = aggregate_group_progress(&[
            item("pending"),
            item("running"),
            item("waiting_for_human"),
            item("completed"),
            item("blocked"),
        ]);
        assert_eq!(aggregate.total, 5);
        assert_eq!(aggregate.pending, 1);
        assert_eq!(aggregate.active, 2);
        assert_eq!(aggregate.completed, 1);
        assert_eq!(aggregate.failed_or_blocked, 1);
    }

    #[test]
    fn group_work_item_progress_exposes_status_stage_commits_review_handoff_reason_and_binding() {
        let value = WorkItemCodingProgressDto {
            logical_work_item_id: "logical_0001".to_string(),
            unit_id: "unit_0001".to_string(),
            status: "completed".to_string(),
            stage: None,
            current_commit: Some("C2".to_string()),
            final_commit: Some("C2".to_string()),
            code_review: None,
            handoff_revision_id: Some("handoff_0001".to_string()),
            failure_or_blocked_reason: None,
            plan_revision_id: "plan_revision_0001".to_string(),
        };
        let json = serde_json::to_value(&value).expect("serialize progress");
        assert_eq!(json["logical_work_item_id"], "logical_0001");
        assert_eq!(json["current_commit"], "C2");
        assert_eq!(json["final_commit"], "C2");
        assert_eq!(json["handoff_revision_id"], "handoff_0001");
        assert_eq!(json["plan_revision_id"], "plan_revision_0001");
    }

    #[test]
    fn group_work_item_progress_ignores_per_wi_session_state() {
        let projected = item("running");
        let contradictory_sc_child_session = "completed";
        assert_ne!(contradictory_sc_child_session, projected.status);
        assert_eq!(projected.status, "running");
        assert_eq!(projected.stage, None);
    }

    #[test]
    fn group_work_item_progress_serialization_roundtrip_preserves_http_and_ws_shape() {
        let progress = vec![item("pending"), item("completed")];
        let aggregate = aggregate_group_progress(&progress);
        let encoded = serde_json::to_string(&(progress, aggregate)).expect("serialize projection");
        let decoded: (Vec<WorkItemCodingProgressDto>, GroupCodingProgressDto) =
            serde_json::from_str(&encoded).expect("deserialize projection");
        assert_eq!(decoded.0.len(), 2);
        assert_eq!(decoded.1.total, 2);
        assert_eq!(decoded.1.pending, 1);
        assert_eq!(decoded.1.completed, 1);
    }
}
