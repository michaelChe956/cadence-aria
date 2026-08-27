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

#[tokio::test]
async fn single_repo_design_review_pass_waits_for_author_finalize_and_keeps_inputs_unstructured() {
    let (_tmp, lifecycle_store, design_id, mut engine) =
        persistent_single_repo_design_test_engine();

    let author_input = engine
        .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
        .expect("single-repo Design author input");
    assert!(
        author_input.structured_output_contract.is_none(),
        "single-repo Design author input must not carry aggregate structured-output contract"
    );
    assert!(
        !author_input.prompt.contains("<ARIA_STRUCTURED_OUTPUT"),
        "single-repo Design author prompt must not inject aggregate output instructions"
    );

    create_reviewer_run_node(&mut engine).await;
    let pass_verdict = ReviewVerdict {
        verdict: ReviewVerdictType::Pass,
        comments: "可以确认。".to_string(),
        summary: "可以确认".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserConfirmAllowed,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    engine
        .complete_review(
            crate::cross_cutting::streaming_provider::ProviderCompletion::plain("可以确认。", None),
            pass_verdict,
        )
        .await;

    assert_eq!(
        engine.session().stage,
        WorkspaceStage::AuthorConfirm,
        "single-repo Design reviewer pass must wait for the author to confirm"
    );
    assert!(
        !engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::Completed),
        "reviewer pass must not automatically create a Completed node"
    );
    match lifecycle_store
        .load_existing_spec("project_0001", "issue_0001", &design_id)
        .expect("load Design record after reviewer pass")
    {
        ExistingSpecRecord::Design { record, .. } => assert_eq!(
            record.confirmation_status,
            LifecycleConfirmationStatus::Draft,
            "reviewer pass alone must not confirm the single-repo Design record"
        ),
        ExistingSpecRecord::Story { .. } => panic!("expected single-repo Design record"),
    }

    let revision_input = engine
        .build_revision_input()
        .expect("single-repo Design revision input after reviewer pass");
    assert!(
        revision_input.structured_output_contract.is_none(),
        "single-repo Design revision input must not carry aggregate structured-output contract"
    );
    assert!(
        !revision_input.prompt.contains("<ARIA_STRUCTURED_OUTPUT"),
        "single-repo Design revision prompt must not inject aggregate output instructions"
    );

    let outcome = engine
        .handle_author_decision(AuthorDecision::AcceptFinalize)
        .await
        .expect("author finalization");
    assert_eq!(outcome, AuthorDecisionOutcome::Finalized);
    assert_eq!(engine.session().stage, WorkspaceStage::Completed);

    let record = lifecycle_store
        .load_existing_spec("project_0001", "issue_0001", &design_id)
        .expect("load finalized Design record");
    match record {
        ExistingSpecRecord::Design { record, .. } => assert_eq!(
            record.confirmation_status,
            LifecycleConfirmationStatus::Confirmed,
            "only AcceptFinalize may confirm the single-repo Design record"
        ),
        ExistingSpecRecord::Story { .. } => panic!("expected single-repo Design record"),
    }
    let persisted_timeline = lifecycle_store
        .load_timeline_nodes_for_issue_session(
            "project_0001",
            "issue_0001",
            &engine.session().session_id,
        )
        .expect("load finalized Design timeline");
    assert!(
        persisted_timeline
            .iter()
            .any(|node| node.node_type == TimelineNodeType::Completed),
        "AcceptFinalize must persist a Completed timeline node"
    );
}

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
#[tokio::test]
async fn revise_after_review_clears_verdict_and_uses_author_prompt() {
    let mut engine = prompt_engine_with_artifact("# Story Spec\n\n旧内容");
    engine.latest_review_verdict = Some(revise_review_verdict("review 结论", "review 结论。"));

    engine
        .handle_author_decision(AuthorDecision::Revise {
            feedback: "补充异常场景与回滚策略".into(),
        })
        .await
        .expect("post-review feedback revision");

    assert!(
        engine.latest_review_verdict.is_none(),
        "post-review 新反馈必须清空 latest_review_verdict（I-1）"
    );
    assert_eq!(
        engine.pending_revision_context.as_deref(),
        Some("补充异常场景与回滚策略")
    );

    let input = engine.build_revision_input().expect("revision input");
    assert!(
        input.prompt.contains("## 用户反馈"),
        "review 后反馈必须走 author 增量修订 prompt（含产物全文与用户反馈）: {}",
        input.prompt
    );
    assert!(input.prompt.contains("补充异常场景与回滚策略"));
}

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
