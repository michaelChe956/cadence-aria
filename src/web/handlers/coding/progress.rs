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
            "failed"
            | "blocked"
            | "blocked_by_plan_defect"
            | "awaiting_amendment"
            | "needs_revalidation"
            | "stale" => aggregate.failed_or_blocked += 1,
            "superseded" | "skipped" => {}
            other => unreachable!("unknown coding unit progress status: {other}"),
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
    let handoff_revision_id = validated_handoff_revision_id(store, unit, latest_run, lineage)?;
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

fn validated_handoff_revision_id(
    store: &CodingAttemptStore,
    unit: &CodingExecutionUnit,
    latest_run: Option<&crate::product::coding_models::CodingUnitRun>,
    lineage: Option<&crate::product::models::WorkItemPlanLineage>,
) -> Result<Option<String>, ProductStoreError> {
    let Some(run) = latest_run else {
        return Ok(None);
    };
    let Some(handoff_id) = unit.latest_handoff_revision_id.as_deref() else {
        return Ok(None);
    };
    let Some(plan) = lineage else {
        return Ok(None);
    };
    let handoff = match WorkItemRevisionStore::new(store.paths()).get_handoff_revision(
        plan,
        &unit.logical_work_item_id,
        handoff_id,
    ) {
        Ok(handoff) => handoff,
        Err(ProductStoreError::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(
        crate::product::coding_workspace_engine::handoff_matches_unit_run(&handoff, unit, run)
            .then_some(handoff.id),
    )
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
    use super::{aggregate_group_progress, build_group_work_item_progress};
    use crate::product::coding_workspace_engine::readiness_fixture;
    use crate::product::lifecycle_store::workspace_session_read_spy::{
        reset_workspace_session_read_spy, set_workspace_session_read_panic,
        workspace_session_read_count,
    };
    use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
    use crate::product::models::{ProviderName, WorkspaceSessionStatus, WorkspaceType};
    use crate::web::coding_ws_handler::CodingWsOutMessage;
    use crate::web::types::{CodingAttemptSnapshotResponse, WorkItemCodingProgressDto};

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
        let fixture = readiness_fixture();
        let lifecycle = LifecycleStore::new(fixture.store.paths());
        let child = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: fixture.attempt.project_id.clone(),
                issue_id: fixture.attempt.issue_id.clone(),
                entity_id: "work_item_0001".to_string(),
                workspace_type: WorkspaceType::WorkItem,
                author_provider: ProviderName::Fake,
                reviewer_provider: ProviderName::Fake,
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("SC child session");
        lifecycle
            .update_workspace_session_status(&child.id, WorkspaceSessionStatus::Failed)
            .expect("contradictory SC child state");
        lifecycle
            .append_workspace_message(
                &child.id,
                "assistant".to_string(),
                "stage=completed commit=SC_CHILD_COMMIT".to_string(),
            )
            .expect("contaminating SC child payload");

        reset_workspace_session_read_spy();
        set_workspace_session_read_panic(true);
        let attempt = fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("reloaded group attempt");
        let result = build_group_work_item_progress(&fixture.store, &attempt)
            .expect("group progress must ignore SC child session");
        set_workspace_session_read_panic(false);

        assert_eq!(workspace_session_read_count(), 0);
        assert_eq!(result.0.len(), 3);
        assert_eq!(result.0[0].logical_work_item_id, "work_item_0001");
        assert_eq!(result.0[0].unit_id, "coding_unit_0001");
        assert_eq!(result.0[0].status, "running");
        assert_eq!(result.0[0].stage, Some("prepare_context".to_string()));
        assert_eq!(result.0[0].current_commit, None);
        assert_eq!(result.0[0].final_commit, None);
        assert_eq!(result.0[0].code_review, None);
        assert_eq!(result.0[0].handoff_revision_id, None);
        assert_eq!(result.0[0].failure_or_blocked_reason, None);
        assert_eq!(result.0[0].plan_revision_id, "plan_revision_0001");
        assert_eq!(result.1.total, 3);
        assert_eq!(result.1.pending, 2);
        assert_eq!(result.1.active, 1);
        assert_eq!(result.1.completed, 0);
        assert_eq!(result.1.failed_or_blocked, 0);
    }

    #[test]
    fn group_work_item_progress_serialization_roundtrip_preserves_http_and_ws_shape() {
        let fixture = readiness_fixture();
        let (progress, aggregate) =
            build_group_work_item_progress(&fixture.store, &fixture.attempt)
                .expect("group progress");
        let attempt_dto =
            crate::web::handlers::dto::coding_attempt_dto(&fixture.store, &fixture.attempt)
                .expect("attempt dto");
        let units = fixture
            .store
            .list_coding_units(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("units")
            .iter()
            .map(crate::web::handlers::dto::coding_execution_unit_dto)
            .collect();
        let http = CodingAttemptSnapshotResponse {
            attempt: attempt_dto,
            attempt_scope: "work_item_group".to_string(),
            work_item_group_id: fixture.attempt.work_item_group_id.clone(),
            current_work_item_id: fixture.attempt.current_work_item_id.clone(),
            active_unit_id: fixture.attempt.active_unit_id.clone(),
            units,
            provider_config_snapshot: fixture.attempt.provider_config_snapshot.clone(),
            timeline_nodes: Vec::new(),
            active_node_id: None,
            code_review_reports: Vec::new(),
            review_request: None,
            internal_pr_review: None,
            group_coding_progress: Some(progress.clone()),
            group_progress: Some(aggregate.clone()),
            group_review_artifacts: None,
            group_final_readiness: None,
            pending_gates: Vec::new(),
            pending_choices: Vec::new(),
            role_runs: Vec::new(),
            work_item_execution_plan: None,
        };
        let ws = CodingWsOutMessage::CodingSessionState {
            project_id: fixture.attempt.project_id.clone(),
            issue_id: fixture.attempt.issue_id.clone(),
            attempt_id: fixture.attempt.id.clone(),
            attempt_scope: "work_item_group".to_string(),
            work_item_group_id: fixture.attempt.work_item_group_id.clone(),
            current_work_item_id: fixture.attempt.current_work_item_id.clone(),
            active_unit_id: fixture.attempt.active_unit_id.clone(),
            units: http.units.clone(),
            group_coding_progress: Box::new(Some(progress.clone())),
            group_progress: Box::new(Some(aggregate.clone())),
            status: fixture.attempt.status,
            stage: fixture.attempt.stage,
            branch_name: fixture.attempt.branch_name.clone(),
            base_branch: fixture.attempt.base_branch.clone(),
            worktree_path: fixture.attempt.worktree_path.clone(),
            rework_count: fixture.attempt.rework_count,
            max_auto_rework: fixture.attempt.max_auto_rework,
            head_commit: Box::new(fixture.attempt.head_commit.clone()),
            pushed_remote: Box::new(fixture.attempt.pushed_remote.clone()),
            role_provider_config_snapshot: Box::new(
                fixture
                    .store
                    .get_role_provider_config_snapshot(
                        &fixture.attempt.project_id,
                        &fixture.attempt.issue_id,
                        &fixture.attempt.id,
                    )
                    .expect("role provider snapshot"),
            ),
            provider_config_snapshot: Box::new(fixture.attempt.provider_config_snapshot.clone()),
            chat_entries: Box::new(Vec::new()),
            timeline_nodes: Box::new(Vec::new()),
            active_node_id: Box::new(None),
            code_review_reports: Box::new(Vec::new()),
            review_request: Box::new(None),
            internal_pr_review: Box::new(None),
            group_review_artifacts: Box::new(None),
            group_final_readiness: Box::new(None),
            pending_gates: Box::new(Vec::new()),
            pending_choices: Box::new(Vec::new()),
            role_runs: Box::new(Vec::new()),
            work_item_markdown: Box::new(None),
            verification_commands: Box::new(Vec::new()),
            work_item_execution_plan: Box::new(None),
            linked_plan_repair: Box::new(None),
        };

        let http_roundtrip: CodingAttemptSnapshotResponse =
            serde_json::from_value(serde_json::to_value(&http).expect("serialize HTTP envelope"))
                .expect("deserialize HTTP envelope");
        let ws_roundtrip: CodingWsOutMessage =
            serde_json::from_value(serde_json::to_value(&ws).expect("serialize WS envelope"))
                .expect("deserialize WS envelope");
        let CodingWsOutMessage::CodingSessionState {
            group_coding_progress: ws_progress,
            group_progress: ws_aggregate,
            ..
        } = ws_roundtrip
        else {
            panic!("expected coding session state");
        };
        assert_eq!(http_roundtrip.group_coding_progress, Some(progress.clone()));
        assert_eq!(http_roundtrip.group_progress, Some(aggregate.clone()));
        assert_eq!(*ws_progress, Some(progress));
        assert_eq!(*ws_aggregate, Some(aggregate));
        let http_progress = http_roundtrip.group_coding_progress.as_ref().unwrap();
        let ws_progress = ws_progress.as_ref().as_ref().unwrap();
        assert_eq!(http_progress.len(), ws_progress.len());
        for (http_item, ws_item) in http_progress.iter().zip(ws_progress) {
            assert_eq!(http_item.logical_work_item_id, ws_item.logical_work_item_id);
            assert_eq!(http_item.unit_id, ws_item.unit_id);
            assert_eq!(http_item.status, ws_item.status);
            assert_eq!(http_item.stage, ws_item.stage);
            assert_eq!(http_item.current_commit, ws_item.current_commit);
            assert_eq!(http_item.final_commit, ws_item.final_commit);
            assert_eq!(http_item.code_review, ws_item.code_review);
            assert_eq!(http_item.handoff_revision_id, ws_item.handoff_revision_id);
            assert_eq!(
                http_item.failure_or_blocked_reason,
                ws_item.failure_or_blocked_reason
            );
            assert_eq!(http_item.plan_revision_id, ws_item.plan_revision_id);
        }
        assert_eq!(http_roundtrip.group_progress, *ws_aggregate);
    }
}
