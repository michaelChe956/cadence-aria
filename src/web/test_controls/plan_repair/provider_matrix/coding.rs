use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;

use super::{ISSUE_ID, MatrixStreamingProvider, PROJECT_ID};
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingProviderRole, CodingRolePermissionModes, CodingRoleProviderConfigSnapshot,
};
use crate::product::coding_workspace_engine::{
    CodeReviewFlowDecision, CodingExecutionContext, CodingWorkspaceEngine,
    code_review_flow_decision,
};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::models::{PlanDefectClass, PlanDefectRoute, ProviderName};
use crate::web::test_controls::plan_repair::PlanRepairFixtureError;
use crate::web::test_controls::plan_repair::recovery::fixture_error;
use crate::web::test_controls::plan_repair::seed::fixture_paths;

const PLAN_ID: &str = "work_item_plan_0001";

pub(super) struct CodingMatrixOutcome {
    pub coder_defect_class: PlanDefectClass,
    pub code_review_defect_class: PlanDefectClass,
    pub code_review_route: PlanDefectRoute,
    pub role_run_count: usize,
    pub raw_output_ref_count: usize,
}

pub(super) async fn run_coding_provider_roles(
    root: &Path,
    provider: ProviderName,
    adapter: Arc<MatrixStreamingProvider>,
) -> Result<CodingMatrixOutcome, PlanRepairFixtureError> {
    let store = CodingAttemptStore::new(fixture_paths(root));
    let attempt = store
        .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .map_err(fixture_error)?
        .ok_or_else(|| fixture_error("provider matrix coding attempt is missing"))?;
    let placeholder_run = store.get_active_unit_run(&attempt).map_err(fixture_error)?;
    store
        .complete_coding_unit_run(
            &attempt,
            &placeholder_run.id,
            "commit_provider_matrix_boundary",
        )
        .map_err(fixture_error)?;
    store
        .update_role_provider_config_snapshot(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            CodingRoleProviderConfigSnapshot {
                coder: provider.clone(),
                code_reviewer: provider.clone(),
                internal_reviewer: provider,
                review_rounds: 1,
                permission_modes: CodingRolePermissionModes::default(),
            },
        )
        .map_err(fixture_error)?;

    let (event_tx, _event_rx) = mpsc::channel(128);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let (_coder_command_tx, mut coder_command_rx) = mpsc::channel::<CodingRunnerCommand>(1);
    let coder_outcome = engine
        .execute_coding_with_commands_outcome(
            &attempt,
            adapter.as_ref(),
            &CodingExecutionContext::default(),
            &mut coder_command_rx,
        )
        .await
        .map_err(fixture_error)?;
    if coder_outcome.plan_defect_decision != Some(CodeReviewFlowDecision::StartPlanRepair) {
        return Err(fixture_error(format!(
            "coder did not route plan defect to Plan Repair: {:?}",
            coder_outcome.plan_defect_decision
        )));
    }
    let coder_defect_class = coder_outcome
        .plan_defect_report
        .as_ref()
        .and_then(|report| report.findings.first())
        .map(|finding| finding.defect_class.clone())
        .ok_or_else(|| fixture_error("coder plan defect finding is missing"))?;

    let (_review_command_tx, mut review_command_rx) = mpsc::channel::<CodingRunnerCommand>(1);
    let code_review = engine
        .execute_code_review_with_commands(
            &coder_outcome.attempt,
            adapter.as_ref(),
            &mut review_command_rx,
        )
        .await
        .map_err(fixture_error)?;
    let reviewer_projection = engine
        .reviewer_projection_for_attempt(&coder_outcome.attempt)
        .map_err(fixture_error)?;
    let route = code_review_flow_decision(&code_review, &reviewer_projection);
    if route != CodeReviewFlowDecision::StartPlanRepair {
        return Err(fixture_error(format!(
            "code reviewer did not route finding to Plan Repair: {route:?}"
        )));
    }
    let finding = code_review
        .findings
        .first()
        .ok_or_else(|| fixture_error("code review finding is missing"))?;

    let restarted = CodingAttemptStore::new(fixture_paths(root));
    let role_runs = restarted
        .list_role_runs(PROJECT_ID, ISSUE_ID, &attempt.id)
        .map_err(fixture_error)?
        .into_iter()
        .filter(|run| {
            matches!(
                run.role,
                CodingProviderRole::Coder | CodingProviderRole::CodeReviewer
            )
        })
        .collect::<Vec<_>>();
    let raw_output_ref_count = role_runs
        .iter()
        .map(|run| run.raw_provider_output_refs.len())
        .sum();

    Ok(CodingMatrixOutcome {
        coder_defect_class,
        code_review_defect_class: finding.defect_class.clone(),
        code_review_route: finding.recommended_route.clone(),
        role_run_count: role_runs.len(),
        raw_output_ref_count,
    })
}
