// spec-design-dialog-revision T5：review 完成路由回 AuthorConfirm（spec「review 结果回对话流」2 场景）。
// T9 自 author_revision_loop.rs 拆出（1200 行护栏）：本文件承载 review 路由与分流谓词用例。
// 真实完成路径：persistent_test_engine（Story）+ create_reviewer_run_node + complete_review（review/routing.rs）。
// 注意：WorkItem/WorkItemPlan 类型维持既有 HumanConfirm/ReviewDecision 路由（design.md「WorkItem 不受影响」）。

use super::author_revision_loop::prompt_engine_with_artifact;
use super::*;

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
