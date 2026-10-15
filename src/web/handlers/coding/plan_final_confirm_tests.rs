//! P1-a（REQ-MTG-11 / 契约 A2）测试：plan 级聚合最终确认（GET 投影 +
//! 无 body POST 逐仓调用既有 `handle_final_confirm`）。
//!
//! 夹具复用 plan_start_tests 的 N3 四仓 plan（prepare → advance → bind，
//! 无 runner 竞态），逐 target 用引擎真实链路物化 worktree 与 complete
//! readiness 后进 WaitingForHuman+FinalConfirm（先例：
//! coding_workspace_engine/tests/group_final_readiness*.rs）。

use super::plan_start_tests::{
    ISSUE_ID, N3Fixture, PLAN_ID, PLAN_REVISION_ID, PROJECT_ID, get_progress, n3_fixture,
};
use super::*;
use crate::product::advance_store::{AdvanceInput, AdvanceOutcome};
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodeReviewReport, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionUnit,
    CodingUnitRun, CodingUnitRunStatus, ReviewFinding,
};
use crate::product::coding_plan_run::CodingPlanRunPhase;
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::{HandoffRevision, PlanDefectClass, PlanDefectRoute};
use crate::product::work_item_projection::{renderer_for, renderer_version_for_optional_provider};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::advance_plan::{AdvancePlanOrigin, advance_plan};
use crate::web::types::{PlanCodingFinalConfirmResultDto, PlanCodingFinalConfirmResultItemDto};

/// 冻结 4 targets（prepare → advance → bind → mark_start_requested），不启动
/// runner：确定性拿到 run.target_attempts 与 4 个 Created 成员 attempt。
async fn frozen_plan_fixture() -> N3Fixture {
    let fixture = n3_fixture().await;
    let run_store = fixture.run_store();
    let run = run_store
        .prepare(
            PROJECT_ID,
            ISSUE_ID,
            PLAN_ID,
            "cmd_p1a_final_confirm",
            PLAN_REVISION_ID,
        )
        .unwrap();
    let outcome = advance_plan(
        &fixture.state,
        AdvanceInput {
            command_id: run.command_id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: PLAN_ID.to_string(),
        },
        AdvancePlanOrigin::Manual,
    )
    .await
    .unwrap();
    let AdvanceOutcome::Completed {
        target_attempts, ..
    } = outcome
    else {
        panic!("expected advance Completed, got {outcome:?}");
    };
    let selection = fixture.busi_selection("cmd_p1a_final_confirm");
    let bound = run_store
        .bind_ready(&run, &target_attempts, Some(selection.clone()))
        .unwrap();
    run_store.mark_start_requested(&bound).unwrap();
    let run = run_store
        .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .expect("durable run");
    assert_eq!(run.phase, CodingPlanRunPhase::StartRequested);
    assert_eq!(run.target_attempts.len(), 4);
    fixture
}

fn git_stdout(cwd: &std::path::Path, args: &[&str]) -> String {
    String::from_utf8(
        std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git stdout probe")
            .stdout,
    )
    .expect("utf8 git output")
}

/// 与引擎 `seed_completed_run` 同构的 per-unit 完成事实播种（公开 store 面）。
fn seed_completed_unit_run(
    store: &CodingAttemptStore,
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    unit: &CodingExecutionUnit,
    run_index: usize,
    start_commit: &str,
) -> CodingUnitRun {
    let revision_store = WorkItemRevisionStore::new(paths.clone());
    let lineage = revision_store
        .get_plan_lineage(&attempt.project_id, &attempt.issue_id, PLAN_ID)
        .unwrap();
    let revision = revision_store
        .get_work_item_revision(
            &lineage,
            &unit.logical_work_item_id,
            &unit.work_item_revision_id,
        )
        .unwrap();
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .unwrap();
    let providers = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    let run = CodingUnitRun {
        id: format!("coding_unit_run_{:04}", run_index + 1),
        unit_id: unit.id.clone(),
        execution_no: 1,
        work_item_revision_id: revision.id,
        resolved_handoff_revision_ids: Vec::new(),
        canonical_contract_hash: bundle.canonical_contract_hash,
        projection_bundle_id: bundle.id,
        projection_compiler_version: bundle.compiler_version,
        coder_provider_renderer_version: renderer_for(&providers.coder)
            .renderer_version()
            .to_string(),
        reviewer_provider_renderer_version: renderer_version_for_optional_provider(
            providers.code_reviewer.as_ref(),
        )
        .to_string(),
        internal_reviewer_provider_renderer_version: None,
        coder_projection_hash: bundle.coder_projection_hash,
        reviewer_projection_hash: bundle.reviewer_projection_hash,
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        internal_reviewer_execution_context_hash: None,
        status: CodingUnitRunStatus::Completed,
        unit_rework_count: 1,
        verification_retry_count: 0,
        operational_retry_count: 0,
        plan_repair_count: 0,
        start_commit: Some(start_commit.to_string()),
        completion_commit: Some(start_commit.to_string()),
        created_at: "2026-10-15T00:00:00Z".to_string(),
        updated_at: "2026-10-15T00:00:00Z".to_string(),
    };
    store.create_coding_unit_run(attempt, &run).unwrap();
    run
}

/// 与引擎 `review_report` 同构的 Approve 报告（id 按 logical WI 全 plan 唯一）。
fn approve_review_report(
    attempt: &CodingExecutionAttempt,
    unit: &CodingExecutionUnit,
    run: &CodingUnitRun,
) -> CodeReviewReport {
    CodeReviewReport {
        id: format!(
            "code_review_report_{}_{}",
            unit.logical_work_item_id, attempt.id
        ),
        attempt_id: attempt.id.clone(),
        round: 1,
        verdict: crate::product::coding_models::ReviewVerdict::Approve,
        findings: vec![ReviewFinding {
            severity: crate::product::coding_models::FindingSeverity::Info,
            file_path: Some("README.md".to_string()),
            line: Some(1),
            message: "independent review approved".to_string(),
            required_action: None,
            source_stage: crate::product::coding_models::CodingExecutionStage::CodeReview,
            evidence: Vec::new(),
            plan_defect_evidence: Vec::new(),
            related_requirements: Vec::new(),
            related_design_constraints: Vec::new(),
            related_work_item_tasks: Vec::new(),
            defect_class: PlanDefectClass::ImplementationDefect,
            reason_code: None,
            contract_refs: Vec::new(),
            capability_refs: Vec::new(),
            repair_target: None,
            recommended_route: PlanDefectRoute::CoderRework,
            confidence: None,
        }],
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary: "independent review approved".to_string(),
        created_at: "2026-10-15T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: Some(1),
        unit_run_id: Some(run.id.clone()),
    }
}

/// 驱动单个冻结 target 到「WaitingForHuman ∧ FinalConfirm ∧ complete
/// readiness」：真实 worktree 物化 → per-unit 完成事实 → head_commit →
/// 引擎 `prepare_group_final_confirm_from_readiness`（写 Complete snapshot +
/// Pending FinalConfirm 节点）。
async fn drive_target_to_final_confirm(
    fixture: &N3Fixture,
    target: LogicalRepositoryId,
    repo_name: &str,
) -> CodingExecutionAttempt {
    let store = fixture.coding_store();
    let repo_path = fixture._root.path().join(repo_name);
    let start_commit = git_stdout(&repo_path, &["rev-parse", "HEAD"])
        .trim()
        .to_string();

    // worktree 物化（真实 git worktree；先例 amr_ready_busi_attempt）。
    let mut attempt = fixture.target_attempt(target);
    for _ in 0..100 {
        if attempt
            .worktree_path
            .as_ref()
            .is_some_and(|worktree| worktree.exists())
        {
            break;
        }
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
        match CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
            .execute_worktree_prepare(&attempt, &repo_path)
            .await
        {
            Ok(_) => break,
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
        }
        attempt = fixture.target_attempt(target);
    }
    let attempt = fixture.target_attempt(target);
    assert!(
        attempt
            .worktree_path
            .as_ref()
            .is_some_and(|worktree| worktree.exists()),
        "{repo_name} worktree not materialized"
    );

    // per-unit 完成事实（completed run + handoff + Approve 报告 + Completed 状态）。
    let units = store
        .list_coding_units(PROJECT_ID, ISSUE_ID, &attempt.id)
        .unwrap();
    let revision_store = WorkItemRevisionStore::new(fixture.paths.clone());
    let lineage = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap();
    for (index, unit) in units.iter().enumerate() {
        let run =
            seed_completed_unit_run(&store, &fixture.paths, &attempt, unit, index, &start_commit);
        let handoff = HandoffRevision {
            id: format!(
                "handoff_revision_{}_{}",
                unit.logical_work_item_id, attempt.id
            ),
            logical_work_item_id: unit.logical_work_item_id.clone(),
            work_item_revision_id: run.work_item_revision_id.clone(),
            coding_unit_run_id: run.id.clone(),
            provided_contracts: Vec::new(),
            provided_capabilities: std::collections::BTreeMap::new(),
            contract_hash: "handoff_contract_hash".to_string(),
            commit_sha: start_commit.clone(),
            created_at: "2026-10-15T00:00:00Z".to_string(),
        };
        revision_store
            .put_handoff_revision(&lineage, &handoff)
            .unwrap();
        store
            .update_coding_unit_latest_handoff_revision_id(
                PROJECT_ID,
                ISSUE_ID,
                &attempt.id,
                &unit.id,
                Some(handoff.id.clone()),
            )
            .unwrap();
        store
            .save_code_review_report(&attempt, &approve_review_report(&attempt, unit, &run))
            .unwrap();
        store
            .update_coding_unit_status(
                PROJECT_ID,
                ISSUE_ID,
                &attempt.id,
                &unit.id,
                crate::product::coding_models::CodingExecutionUnitStatus::Completed,
                Some("independent review approved".to_string()),
            )
            .unwrap();
        store
            .update_coding_unit_completion_commit(
                PROJECT_ID,
                ISSUE_ID,
                &attempt.id,
                &unit.id,
                Some(start_commit.clone()),
            )
            .unwrap();
    }

    // head_commit + Running 播种 → 引擎真实 prepare（Complete snapshot +
    // WaitingForHuman/FinalConfirm + Pending 节点）。
    let mut stored = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
        .unwrap();
    stored.head_commit = Some(start_commit.clone());
    store.write_coding_attempt_for_test(&stored).unwrap();
    store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &stored.id)
        .unwrap();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    engine
        .prepare_group_final_confirm_from_readiness(
            &store.get_attempt(PROJECT_ID, ISSUE_ID, &stored.id).unwrap(),
        )
        .await
        .expect("prepare group final confirm")
}

/// 排序后的冻结绑定（与后端 sorted 同口径：target id → attempt id）。
fn sorted_bindings(fixture: &N3Fixture) -> Vec<(String, String)> {
    let mut bindings: Vec<(String, String)> = fixture
        .run_store()
        .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .expect("durable run")
        .target_attempts
        .into_iter()
        .map(|binding| (binding.target_repository_id, binding.attempt_id))
        .collect();
    bindings.sort();
    bindings
}

async fn post_final_confirm(
    state: &crate::web::state::WebAppState,
) -> Result<PlanCodingFinalConfirmResultDto, crate::web::error::ApiError> {
    super::plan_final_confirm::post_plan_coding_final_confirm(
        axum::extract::State(state.clone()),
        axum::extract::Path((
            PROJECT_ID.to_string(),
            ISSUE_ID.to_string(),
            PLAN_ID.to_string(),
        )),
    )
    .await
    .map(|json| json.0)
}

async fn drive_all_targets(fixture: &N3Fixture) {
    for (target, repo_name) in [
        (fixture.busi, "n3-busi"),
        (fixture.web, "n3-web"),
        (fixture.gateway, "n3-gateway"),
        (fixture.frontend, "n3-frontend"),
    ] {
        drive_target_to_final_confirm(fixture, target, repo_name).await;
    }
}

/// ① 多仓全部 FinalConfirm + readiness 完整 → GET `final_confirm.state=ready`。
#[tokio::test]
async fn plan_final_confirm_get_ready_when_all_targets_waiting_with_complete_readiness() {
    let fixture = frozen_plan_fixture().await;
    drive_all_targets(&fixture).await;

    let progress = get_progress(&fixture.state).await.unwrap();
    assert_eq!(progress.final_confirm.state, "ready");
    assert_eq!(progress.final_confirm.ready_attempt_ids.len(), 4);
    assert!(progress.final_confirm.confirmed_attempt_ids.is_empty());
    assert!(progress.final_confirm.blocking.is_empty());
    let expected: Vec<String> = sorted_bindings(&fixture)
        .into_iter()
        .map(|(_, attempt_id)| attempt_id)
        .collect();
    assert_eq!(progress.final_confirm.ready_attempt_ids, expected);
}

/// ② 一次 POST 全确认（逐仓 confirmed，响应 all_confirmed；GET 转 done）。
#[tokio::test]
async fn plan_final_confirm_post_confirms_all_targets_in_one_call() {
    let fixture = frozen_plan_fixture().await;
    drive_all_targets(&fixture).await;

    let result = post_final_confirm(&fixture.state).await.unwrap();
    assert_eq!(result.outcome, "all_confirmed");
    assert_eq!(result.plan_id, PLAN_ID);
    assert_eq!(result.results.len(), 4);
    for item in &result.results {
        assert_eq!(item.status, "confirmed");
        assert!(item.error_code.is_none());
    }

    // durable：全部成员 attempt Completed；GET 投影 done + confirmed 集齐。
    for (target, _) in [
        (fixture.busi, "n3-busi"),
        (fixture.web, "n3-web"),
        (fixture.gateway, "n3-gateway"),
        (fixture.frontend, "n3-frontend"),
    ] {
        assert_eq!(
            fixture.target_attempt(target).status,
            CodingAttemptStatus::Completed
        );
    }
    let progress = get_progress(&fixture.state).await.unwrap();
    assert_eq!(progress.final_confirm.state, "done");
    assert!(progress.final_confirm.ready_attempt_ids.is_empty());
    assert_eq!(progress.final_confirm.confirmed_attempt_ids.len(), 4);
}

/// ③ 第 2 项失败 → 第 3 项未被调用（not_attempted 且 durable 仍
/// WaitingForHuman），失败仓 error_code/message 可见。
#[tokio::test]
async fn plan_final_confirm_post_stops_at_first_failure_and_skips_rest() {
    let fixture = frozen_plan_fixture().await;
    drive_all_targets(&fixture).await;

    // 预检不可见的失败方式：清空第 2 个 target 的 head_commit（引擎
    // run_group_completion_gates → CompletionCommitMissing）。
    let bindings = sorted_bindings(&fixture);
    let store = fixture.coding_store();
    let mut second = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &bindings[1].1)
        .unwrap();
    second.head_commit = None;
    store.write_coding_attempt_for_test(&second).unwrap();

    let result = post_final_confirm(&fixture.state).await.unwrap();
    assert_eq!(result.outcome, "partially_confirmed");
    assert_eq!(result.results.len(), 4);
    assert_eq!(result.results[0].status, "confirmed");
    assert_eq!(result.results[1].status, "failed");
    assert_eq!(
        result.results[1].error_code.as_deref(),
        Some("completion_commit_missing")
    );
    assert!(
        result.results[1]
            .message
            .as_deref()
            .is_some_and(|message| !message.is_empty())
    );
    assert_eq!(result.results[2].status, "not_attempted");
    assert_eq!(result.results[3].status, "not_attempted");

    // 第 3 项未调用：durable 状态仍是 WaitingForHuman（不是 Completed）。
    let third = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &bindings[2].1)
        .unwrap();
    assert_eq!(third.status, CodingAttemptStatus::WaitingForHuman);
}

/// ④ 预检失败（一个 target 未进 FinalConfirm）→ outcome not_ready，
/// 不确认任何仓。
#[tokio::test]
async fn plan_final_confirm_post_not_ready_when_one_target_not_final_confirm() {
    let fixture = frozen_plan_fixture().await;
    // frontend 留在 Created：其余三仓已就绪。
    for (target, repo_name) in [
        (fixture.busi, "n3-busi"),
        (fixture.web, "n3-web"),
        (fixture.gateway, "n3-gateway"),
    ] {
        drive_target_to_final_confirm(&fixture, target, repo_name).await;
    }

    let progress = get_progress(&fixture.state).await.unwrap();
    assert_eq!(progress.final_confirm.state, "not_ready");
    assert_eq!(progress.final_confirm.ready_attempt_ids.len(), 3);
    assert_eq!(progress.final_confirm.blocking.len(), 1);
    let frontend_attempt_id = fixture.target_attempt(fixture.frontend).id;
    assert_eq!(
        progress.final_confirm.blocking[0].attempt_id,
        frontend_attempt_id
    );

    let result = post_final_confirm(&fixture.state).await.unwrap();
    assert_eq!(result.outcome, "not_ready");
    assert!(
        result.results.iter().all(|item| item.status != "confirmed"),
        "not_ready must not confirm any target"
    );
    let blocking_item = result
        .results
        .iter()
        .find(|item| item.attempt_id == frontend_attempt_id)
        .unwrap();
    assert_eq!(blocking_item.status, "failed");
    assert!(blocking_item.error_code.is_some());
    // 就绪三仓未被触碰。
    let busi = fixture.busi_attempt();
    assert_eq!(busi.status, CodingAttemptStatus::WaitingForHuman);
}

/// ⑤ worktree 脏预检 → not_ready 且零确认；GET 仍 ready（契约 A2：GET 只看
/// 持久事实，不宣称已检查 git）。
#[tokio::test]
async fn plan_final_confirm_post_not_ready_on_dirty_worktree_without_confirming_anything() {
    let fixture = frozen_plan_fixture().await;
    drive_all_targets(&fixture).await;

    let bindings = sorted_bindings(&fixture);
    let store = fixture.coding_store();
    let first = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &bindings[0].1)
        .unwrap();
    let worktree = first.worktree_path.clone().expect("worktree path");
    std::fs::write(worktree.join("p1a-dirty.txt"), "dirty").unwrap();

    let result = post_final_confirm(&fixture.state).await.unwrap();
    assert_eq!(result.outcome, "not_ready");
    assert_eq!(result.results[0].status, "failed");
    assert_eq!(
        result.results[0].error_code.as_deref(),
        Some("worktree_dirty")
    );
    assert_eq!(result.results[1].status, "not_attempted");
    assert!(
        result.results.iter().all(|item| item.status != "confirmed"),
        "dirty worktree precheck must not confirm any target"
    );
    // GET 投影不检查 git：持久事实齐备仍 ready。
    let progress = get_progress(&fixture.state).await.unwrap();
    assert_eq!(progress.final_confirm.state, "ready");
}

/// 结果条目排序与字段完整性（attempt_id/target_repository_id 成对）。
#[tokio::test]
async fn plan_final_confirm_results_carry_target_and_attempt_ids_in_sorted_order() {
    let fixture = frozen_plan_fixture().await;
    drive_all_targets(&fixture).await;

    let result = post_final_confirm(&fixture.state).await.unwrap();
    let expected = sorted_bindings(&fixture);
    let got: Vec<(String, String)> = result
        .results
        .iter()
        .map(
            |PlanCodingFinalConfirmResultItemDto {
                 target_repository_id,
                 attempt_id,
                 ..
             }| (target_repository_id.clone(), attempt_id.clone()),
        )
        .collect();
    assert_eq!(got, expected);
}
