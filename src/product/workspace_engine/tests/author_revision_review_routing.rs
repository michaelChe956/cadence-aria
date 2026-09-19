// spec-design-dialog-revision T5：review 完成路由回 AuthorConfirm（spec「review 结果回对话流」2 场景）。
// T9 自 author_revision_loop.rs 拆出（1200 行护栏）：本文件承载 review 路由与分流谓词用例。
// 真实完成路径：persistent_test_engine（Story）+ create_reviewer_run_node + complete_review（review/routing.rs）。
// 注意：WorkItem/WorkItemPlan 类型维持既有 HumanConfirm/ReviewDecision 路由（design.md「WorkItem 不受影响」）。

use super::author_revision_loop::prompt_engine_with_artifact;
use super::*;

/// 真实 LifecycleStore 的单仓 Design fixture：Design record 不含 aggregate scope，确保
/// Author/Revision 输入走单仓分支；session 同样经持久化路径恢复。
fn persistent_single_repo_design_test_engine() -> (TempDir, LifecycleStore, String, WorkspaceEngine)
{
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let story = lifecycle_store
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "单仓 Story".to_string(),
            aggregate_codebase: None,
        })
        .expect("seed single-repo Story record");
    let design = lifecycle_store
        .create_design_spec(CreateDesignSpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            story_spec_ids: vec![story.id],
            title: "单仓 Design".to_string(),
            aggregate_codebase: None,
        })
        .expect("seed single-repo Design record");
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: design.id.clone(),
            workspace_type: WorkspaceType::Design,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
            work_item_plan_options: None,
        })
        .expect("seed Design workspace session");
    let (tx, _rx) = mpsc::channel(64);
    let engine = WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle_store.clone(),
        tx,
        WorkspaceSession::from_record(session_record),
    );

    (tmp, lifecycle_store, design.id, engine)
}

fn revise_review_verdict(summary: &str, comments: &str) -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: comments.to_string(),
        summary: summary.to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

// 退役留档（T5/REQ-RET-02）：`single_repo_design_review_pass_waits_for_author_finalize_and_keeps_inputs_unstructured` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn review_completion_routes_back_to_author_confirm_for_story() {
    let (_tmp, _lifecycle, mut engine) = persistent_test_engine();
    create_reviewer_run_node(&mut engine).await;
    let verdict = revise_review_verdict("补充失败路径", "需要补充失败路径。");
    let completion = crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
        "需要补充失败路径。",
        None,
    );

    engine.complete_review(completion, verdict).await;

    assert_eq!(
        engine.session().stage,
        WorkspaceStage::AuthorConfirm,
        "Story review 完成必须回 AuthorConfirm"
    );
}

#[tokio::test]
async fn review_pass_does_not_auto_complete() {
    let (_tmp, _lifecycle, mut engine) = persistent_test_engine();
    create_reviewer_run_node(&mut engine).await;
    let verdict = ReviewVerdict {
        verdict: ReviewVerdictType::Pass,
        comments: "可以确认。".to_string(),
        summary: "可以确认".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserConfirmAllowed,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    let completion =
        crate::cross_cutting::streaming_provider::ProviderCompletion::plain("可以确认。", None);

    engine.complete_review(completion, verdict).await;

    assert_eq!(
        engine.session().stage,
        WorkspaceStage::AuthorConfirm,
        "reviewer pass 不得自动定稿，必须回 AuthorConfirm 等待用户确认"
    );
    assert!(
        !engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::Completed),
        "reviewer pass 不得自动进入 Completed"
    );
}

#[tokio::test]
async fn review_completion_records_formatted_report_in_conversation() {
    let (_tmp, _lifecycle, mut engine) = persistent_test_engine();
    create_reviewer_run_node(&mut engine).await;
    let verdict = revise_review_verdict("补充失败路径", "需要补充失败路径。");
    let completion = crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
        "需要补充失败路径。",
        None,
    );

    engine.complete_review(completion, verdict).await;

    assert!(
        engine
            .session()
            .messages
            .iter()
            .any(|m| m.role == "reviewer" && m.content.contains("[review_summary]")),
        "评审报告必须以 format_review_feedback 消息形式进入对话流"
    );
}

// I-1：review 完成后（latest_review_verdict 存在）用户提交反馈，Revise 臂必须清空 verdict，
// 使 T4 分流谓词（pending.is_some() && verdict.is_none()）成立 → 走 build_author_revision_prompt。
// 退役留档（T5/REQ-RET-02）：`revise_after_review_clears_verdict_and_uses_author_prompt` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// M-1：author 反馈修订分流谓词提取为共享 helper，两处（prompts/revision.rs / provider_drive.rs）同语义。
#[test]
fn is_author_feedback_revision_predicate_requires_pending_without_verdict() {
    let mut engine = prompt_engine_with_artifact("# Story Spec\n\n旧内容");
    assert!(
        !engine.is_author_feedback_revision(),
        "无 pending 不是 author 反馈修订"
    );

    engine.pending_revision_context = Some("反馈".to_string());
    assert!(
        engine.is_author_feedback_revision(),
        "pending 存在且无 verdict 是 author 反馈修订"
    );

    engine.latest_review_verdict = Some(revise_review_verdict("结论", "结论。"));
    assert!(
        !engine.is_author_feedback_revision(),
        "review verdict 存在时不得判为 author 反馈修订（reviewer 返修路径）"
    );
}
