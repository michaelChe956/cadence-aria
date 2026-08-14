// spec-design-dialog-revision T2：provisional reviewer 快照持久化闭环
// serde 兼容（旧 record 无新字段反序列化）+ from_record 恢复用例 + start_generation 写入路径用例（T2 fix1）
use super::*;
use crate::product::models::{ProviderName, WorkspaceSessionRecord};

#[test]
fn legacy_session_record_without_provisional_fields_deserializes() {
    let legacy = serde_json::json!({
        "id": "s1", "project_id": "p1", "issue_id": "i1", "entity_id": "e1",
        "workspace_type": "story", "status": "open",
        "author_provider": "claude_code", "reviewer_provider": "codex", "review_rounds": 1,
        "superpowers_enabled": true, "openspec_enabled": true,
        "messages": [], "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
    });
    let record: WorkspaceSessionRecord = serde_json::from_value(legacy).unwrap();
    assert_eq!(record.provisional_reviewer_provider, None);
    assert_eq!(record.reviewer_enabled_at_start, None);
}

#[test]
fn provisional_fields_roundtrip() {
    let record = WorkspaceSessionRecord {
        provisional_reviewer_provider: Some(ProviderName::Codex),
        reviewer_enabled_at_start: Some(false),
        ..serde_json::from_value(serde_json::json!({
            "id": "s2", "project_id": "p1", "issue_id": "i1", "entity_id": "e1",
            "workspace_type": "story", "status": "open",
            "author_provider": "claude_code", "reviewer_provider": "claude_code", "review_rounds": 1,
            "superpowers_enabled": true, "openspec_enabled": true,
            "messages": [], "created_at": "t", "updated_at": "t"
        })).unwrap()
    };
    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["provisional_reviewer_provider"], "codex");
    assert_eq!(json["reviewer_enabled_at_start"], false);
    let back: WorkspaceSessionRecord = serde_json::from_value(json).unwrap();
    assert_eq!(
        back.provisional_reviewer_provider,
        Some(ProviderName::Codex)
    );
    assert_eq!(back.reviewer_enabled_at_start, Some(false));
}

#[test]
fn from_record_restores_provisional_fields() {
    use crate::product::workspace_engine::types::WorkspaceSession;
    let record: WorkspaceSessionRecord = serde_json::from_value(serde_json::json!({
        "id": "s3", "project_id": "p1", "issue_id": "i1", "entity_id": "e1",
        "workspace_type": "story", "status": "open",
        "author_provider": "claude_code", "reviewer_provider": "codex", "review_rounds": 1,
        "superpowers_enabled": true, "openspec_enabled": true,
        "messages": [], "created_at": "t", "updated_at": "t",
        "provisional_reviewer_provider": "codex",
        "reviewer_enabled_at_start": false
    }))
    .unwrap();
    let session = WorkspaceSession::from_record(record);
    assert_eq!(
        session.provisional_reviewer_provider,
        Some(ProviderName::Codex)
    );
    assert_eq!(session.reviewer_enabled_at_start, Some(false));
}

// T2 fix1：start_generation 写入路径断言。
// reviewer_enabled=false 时：provisional 必须保留快照【原始】reviewer 选择（Critical-1），
// 同时 reviewer_provider/review_rounds 仍被清空（默认行为不变）。
#[tokio::test]
async fn start_generation_disabled_review_keeps_provisional_snapshot() {
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let session = make_session("sess_provisional_disabled");
    let mut engine = WorkspaceEngine::new(store, tx, session);
    let snapshot = ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: Some(ProviderName::Codex),
        review_rounds: 1,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
    };

    engine
        .start_generation(snapshot, false)
        .await
        .expect("start generation");

    let session = engine.session();
    assert_eq!(
        session.provisional_reviewer_provider,
        Some(ProviderName::Codex),
        "provisional must retain the raw snapshot reviewer even when review is disabled"
    );
    assert_eq!(
        session.reviewer_provider, None,
        "disabled review must still clear reviewer_provider"
    );
    assert_eq!(
        session.review_rounds, 0,
        "disabled review must still zero review_rounds"
    );
    assert_eq!(session.reviewer_enabled_at_start, Some(false));
}

// T2 fix1 对照：reviewer_enabled=true 时 provisional == reviewer_provider == 快照 reviewer。
#[tokio::test]
async fn start_generation_enabled_review_sets_provisional_and_reviewer() {
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let session = make_session("sess_provisional_enabled");
    let mut engine = WorkspaceEngine::new(store, tx, session);
    let snapshot = ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: Some(ProviderName::Codex),
        review_rounds: 1,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
    };

    engine
        .start_generation(snapshot, true)
        .await
        .expect("start generation");

    let session = engine.session();
    assert_eq!(
        session.provisional_reviewer_provider,
        Some(ProviderName::Codex),
        "provisional must equal the snapshot reviewer"
    );
    assert_eq!(
        session.reviewer_provider,
        Some(ProviderName::Codex),
        "enabled review keeps reviewer_provider"
    );
    assert_eq!(session.review_rounds, 1);
    assert_eq!(session.reviewer_enabled_at_start, Some(true));
}

// spec-design-dialog-revision T3：AuthorConfirm 对话式修订循环 + 确认双出口决策用例。
// 引擎构造：Story/AuthorConfirm + artifact + 不同 provisional / enabled_at_start / rounds 组合。

async fn author_confirm_engine() -> WorkspaceEngine {
    // reviewer_provider=None, rounds=0, provisional=Some(Codex), enabled_at_start=Some(false)
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_confirm");
    session.stage = WorkspaceStage::AuthorConfirm;
    session.workspace_type = WorkspaceType::Story;
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: "# Story Spec\n\n候选内容".to_string(),
        diff: None,
    });
    session.reviewer_provider = None;
    session.review_rounds = 0;
    session.provisional_reviewer_provider = Some(ProviderName::Codex);
    session.reviewer_enabled_at_start = Some(false);
    WorkspaceEngine::new(store, tx, session)
}

async fn author_confirm_engine_no_provisional() -> WorkspaceEngine {
    // 与 author_confirm_engine 相同，但 provisional=None（创建时未保留 reviewer 选择）。
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_confirm_no_provisional");
    session.stage = WorkspaceStage::AuthorConfirm;
    session.workspace_type = WorkspaceType::Story;
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: "# Story Spec\n\n候选内容".to_string(),
        diff: None,
    });
    session.reviewer_provider = None;
    session.review_rounds = 0;
    session.provisional_reviewer_provider = None;
    session.reviewer_enabled_at_start = Some(false);
    WorkspaceEngine::new(store, tx, session)
}

async fn author_confirm_engine_provisional_restored() -> WorkspaceEngine {
    // reviewer_enabled_at_start=Some(false) + provisional 已恢复（rounds=1）→ Accept 仍定稿。
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_confirm_provisional_restored");
    session.stage = WorkspaceStage::AuthorConfirm;
    session.workspace_type = WorkspaceType::Story;
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: "# Story Spec\n\n候选内容".to_string(),
        diff: None,
    });
    session.reviewer_provider = Some(ProviderName::Codex);
    session.review_rounds = 1;
    session.provisional_reviewer_provider = Some(ProviderName::Codex);
    session.reviewer_enabled_at_start = Some(false);
    WorkspaceEngine::new(store, tx, session)
}

async fn author_confirm_engine_legacy_record() -> WorkspaceEngine {
    // 旧记录：reviewer_enabled_at_start=None（未落盘）→ 按有效态判定。
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_confirm_legacy");
    session.stage = WorkspaceStage::AuthorConfirm;
    session.workspace_type = WorkspaceType::Story;
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: "# Story Spec\n\n候选内容".to_string(),
        diff: None,
    });
    session.reviewer_provider = Some(ProviderName::Codex);
    session.review_rounds = 1;
    session.provisional_reviewer_provider = None;
    session.reviewer_enabled_at_start = None;
    WorkspaceEngine::new(store, tx, session)
}

#[tokio::test]
async fn revise_with_feedback_transitions_to_revision() {
    let mut engine = author_confirm_engine().await;
    let outcome = engine
        .handle_author_decision(AuthorDecision::Revise {
            feedback: "补充异常场景".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        outcome,
        AuthorDecisionOutcome::StartRevision {
            feedback: "补充异常场景".into()
        }
    );
    assert_eq!(engine.session().stage, WorkspaceStage::Revision);
    assert_eq!(
        engine.pending_revision_context.as_deref(),
        Some("补充异常场景")
    );
    assert!(engine.session().artifact.is_some(), "反馈修订不得清空产物");
}

#[tokio::test]
async fn revise_with_blank_feedback_rejected() {
    let mut engine = author_confirm_engine().await;
    let err = engine
        .handle_author_decision(AuthorDecision::Revise {
            feedback: "  ".into(),
        })
        .await
        .unwrap_err();
    assert!(err.contains("feedback"));
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
}

#[tokio::test]
async fn reject_returns_guidance_error_without_reset() {
    let mut engine = author_confirm_engine().await;
    let err = engine
        .handle_author_decision(AuthorDecision::Reject)
        .await
        .unwrap_err();
    assert!(err.contains("反馈"), "引导改用反馈修订: {err}");
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
    assert!(engine.session().artifact.is_some());
}

#[tokio::test]
async fn accept_finalize_completes_workspace() {
    let mut engine = author_confirm_engine().await;
    let outcome = engine
        .handle_author_decision(AuthorDecision::AcceptFinalize)
        .await
        .unwrap();
    assert_eq!(outcome, AuthorDecisionOutcome::Finalized);
    assert_eq!(engine.session().stage, WorkspaceStage::Completed);
}

#[tokio::test]
async fn accept_with_review_restores_provisional_when_disabled() {
    // reviewer_provider=None, rounds=0, provisional=Some(Codex)
    let mut engine = author_confirm_engine().await;
    let outcome = engine
        .handle_author_decision(AuthorDecision::AcceptWithReview)
        .await
        .unwrap();
    assert!(matches!(outcome, AuthorDecisionOutcome::StartReview));
    assert_eq!(
        engine.session().reviewer_provider,
        Some(ProviderName::Codex)
    );
    assert_eq!(engine.session().review_rounds, 1);
    assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
}

#[tokio::test]
async fn accept_with_review_errors_without_provisional() {
    let mut engine = author_confirm_engine_no_provisional().await;
    let err = engine
        .handle_author_decision(AuthorDecision::AcceptWithReview)
        .await
        .unwrap_err();
    assert!(err.contains("reviewer"), "{err}");
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
}

#[tokio::test]
async fn legacy_accept_routes_by_enabled_at_start() {
    // reviewer_enabled_at_start=Some(false) + provisional 已恢复(rounds=1) → Accept 仍定稿（按创建默认值）
    let mut engine = author_confirm_engine_provisional_restored().await;
    let outcome = engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();
    assert_eq!(outcome, AuthorDecisionOutcome::Finalized);
    // 旧记录（None）按有效态：rounds>0 && reviewer.is_some() → StartReview
    let mut legacy = author_confirm_engine_legacy_record().await;
    let outcome2 = legacy
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();
    assert!(matches!(outcome2, AuthorDecisionOutcome::StartReview));
}

// T3 fix round 1（reviewer Important-1）：Fake reviewer 快速路径下 outcome 与最终 stage 一致性回归。
// start_review 的 Fake 快速路径会直接进入 HumanConfirm（Skipped 节点 + mark_reviewed + enter_human_confirm），
// 此时 outcome 必须为 HumanConfirm，不得返回 StartReview（否则真实 handler 会向已处 HumanConfirm 的
// 会话 spawn ReviewOnly run）。回归 5867371b decisions.rs:102 的 stage 后置校验语义。

async fn fake_reviewer_author_confirm_engine_legacy() -> WorkspaceEngine {
    // 旧记录：enabled_at_start=None + Fake reviewer（有效态判定 rounds>0 && reviewer.is_some() 成立）
    // → Accept 经 start_review 快速路径落入 HumanConfirm，outcome 必须与 stage 一致（HumanConfirm）。
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_confirm_fake_legacy");
    session.stage = WorkspaceStage::AuthorConfirm;
    session.workspace_type = WorkspaceType::Story;
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: "# Story Spec\n\n候选内容".to_string(),
        diff: None,
    });
    session.reviewer_provider = Some(ProviderName::Fake);
    session.review_rounds = 1;
    session.provisional_reviewer_provider = None;
    session.reviewer_enabled_at_start = None;
    WorkspaceEngine::new(store, tx, session)
}

async fn fake_reviewer_author_confirm_engine_with_review() -> WorkspaceEngine {
    // AcceptWithReview：reviewer=Some(Fake) + rounds=1（reviewer 就绪）→ 同样走 Fake 快速路径。
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_confirm_fake_with_review");
    session.stage = WorkspaceStage::AuthorConfirm;
    session.workspace_type = WorkspaceType::Story;
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: "# Story Spec\n\n候选内容".to_string(),
        diff: None,
    });
    session.reviewer_provider = Some(ProviderName::Fake);
    session.review_rounds = 1;
    session.provisional_reviewer_provider = Some(ProviderName::Fake);
    session.reviewer_enabled_at_start = Some(true);
    WorkspaceEngine::new(store, tx, session)
}

async fn fake_reviewer_author_confirm_engine_enabled_at_start() -> WorkspaceEngine {
    // Accept Some(true) 分支：enabled_at_start=Some(true) + Fake reviewer → 快速路径同样落入 HumanConfirm。
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_confirm_fake_enabled");
    session.stage = WorkspaceStage::AuthorConfirm;
    session.workspace_type = WorkspaceType::Story;
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: "# Story Spec\n\n候选内容".to_string(),
        diff: None,
    });
    session.reviewer_provider = Some(ProviderName::Fake);
    session.review_rounds = 1;
    session.provisional_reviewer_provider = Some(ProviderName::Fake);
    session.reviewer_enabled_at_start = Some(true);
    WorkspaceEngine::new(store, tx, session)
}

async fn author_confirm_engine_enabled_review() -> WorkspaceEngine {
    // 顺手项（reviewer Minor-1）：enabled_at_start=Some(true) + 真实 reviewer（Codex）→ Accept 路由 StartReview。
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_confirm_enabled_review");
    session.stage = WorkspaceStage::AuthorConfirm;
    session.workspace_type = WorkspaceType::Story;
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: "# Story Spec\n\n候选内容".to_string(),
        diff: None,
    });
    session.reviewer_provider = Some(ProviderName::Codex);
    session.review_rounds = 1;
    session.provisional_reviewer_provider = Some(ProviderName::Codex);
    session.reviewer_enabled_at_start = Some(true);
    WorkspaceEngine::new(store, tx, session)
}

#[tokio::test]
async fn fake_reviewer_legacy_accept_outcome_matches_human_confirm_stage() {
    // Fake reviewer + legacy Accept（None 记录）：有效态判定成立但 Fake 快速路径直入 HumanConfirm，
    // outcome 必须与最终 stage 一致（HumanConfirm），不得返回 StartReview。
    let mut engine = fake_reviewer_author_confirm_engine_legacy().await;
    let outcome = engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        AuthorDecisionOutcome::HumanConfirm,
        "Fake 快速路径下 outcome 必须与最终 stage 一致"
    );
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
}

#[tokio::test]
async fn fake_reviewer_accept_with_review_outcome_matches_human_confirm_stage() {
    // AcceptWithReview + Fake：同样必须 HumanConfirm（快速路径），与 stage 一致。
    let mut engine = fake_reviewer_author_confirm_engine_with_review().await;
    let outcome = engine
        .handle_author_decision(AuthorDecision::AcceptWithReview)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        AuthorDecisionOutcome::HumanConfirm,
        "AcceptWithReview + Fake 快速路径下 outcome 必须与最终 stage 一致"
    );
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
}

#[tokio::test]
async fn fake_reviewer_accept_enabled_at_start_outcome_matches_human_confirm_stage() {
    // Accept Some(true) 分支 + Fake：快速路径同样落入 HumanConfirm，outcome 与 stage 一致。
    let mut engine = fake_reviewer_author_confirm_engine_enabled_at_start().await;
    let outcome = engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        AuthorDecisionOutcome::HumanConfirm,
        "Accept Some(true) + Fake 快速路径下 outcome 必须与最终 stage 一致"
    );
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
}

#[tokio::test]
async fn accept_enabled_review_at_start_routes_to_start_review() {
    // 顺手项（reviewer Minor-1）：enabled_at_start=Some(true) + 真实 reviewer → Accept 路由 StartReview
    // 且 stage=CrossReview（不走 Fake 快速路径）。
    let mut engine = author_confirm_engine_enabled_review().await;
    let outcome = engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();
    assert_eq!(outcome, AuthorDecisionOutcome::StartReview);
    assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
}

// spec-design-dialog-revision T4：author 反馈修订 prompt 构造 + Revision run 分流 + 完成路径改动摘要。
// prompt_engine_with_artifact 辅助构造：session.artifact = Markdown（Story/AuthorConfirm）。

fn prompt_engine_with_artifact(markdown: &str) -> WorkspaceEngine {
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_revision_prompt");
    session.stage = WorkspaceStage::AuthorConfirm;
    session.workspace_type = WorkspaceType::Story;
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: markdown.to_string(),
        diff: None,
    });
    WorkspaceEngine::new(store, tx, session)
}

#[test]
fn author_revision_prompt_includes_feedback_and_changelog_section() {
    let engine = prompt_engine_with_artifact("# Story Spec\n\n旧内容");
    let prompt = engine.build_author_revision_prompt("补充异常场景与回滚策略");
    assert!(prompt.contains("补充异常场景与回滚策略"));
    assert!(prompt.contains("# Story Spec"));
    assert!(
        prompt.contains("改动摘要"),
        "必须要求输出改动摘要小节: {prompt}"
    );
    assert!(prompt.contains("增量修订"), "约束不得整体重写无关章节");
}

// T4 分流：pending_revision_context 存在且无 review verdict（author 反馈路径）时，
// build_revision_input 必须走 build_author_revision_prompt，而非 reviewer 返修 prompt。
#[tokio::test]
async fn author_revision_input_uses_author_prompt_when_no_review_verdict() {
    let mut engine = prompt_engine_with_artifact("# Story Spec\n\n旧内容");
    engine.session.stage = WorkspaceStage::Revision;
    engine.pending_revision_context = Some("补充异常场景与回滚策略".to_string());
    assert!(engine.latest_review_verdict.is_none());

    let input = engine.build_revision_input().expect("revision input");

    assert!(
        input.prompt.contains("增量修订"),
        "author 反馈修订必须走增量修订 prompt: {}",
        input.prompt
    );
    assert!(input.prompt.contains("改动摘要"), "{}", input.prompt);
    assert!(
        input.prompt.contains("补充异常场景与回滚策略"),
        "{}",
        input.prompt
    );
}

// T4 完成路径：author 反馈修订产物末尾的「## 改动摘要」小节被提取为 AuthorConfirm 的 summary 载荷。
#[test]
fn extract_changelog_summary_captures_author_revision_changelog_section() {
    let markdown = "# Story Spec\n\n## 功能需求\n- [REQ-001] 初版。\n\n## 改动摘要\n- 补充异常场景 [REQ-002]\n- 调整回滚策略\n";
    let summary =
        crate::product::workspace_engine::provider_drive::extract_changelog_summary(markdown)
            .expect("changelog summary");
    assert!(summary.contains("补充异常场景"));
    assert!(summary.contains("回滚策略"));
    assert!(
        !summary.contains("初版"),
        "不得包含改动摘要之前的正文: {summary}"
    );
}

#[test]
fn extract_changelog_summary_none_when_section_missing_or_empty() {
    assert_eq!(
        crate::product::workspace_engine::provider_drive::extract_changelog_summary(
            "# Story Spec\n\n## 功能需求\n- [REQ-001] 初版。\n"
        ),
        None
    );
    assert_eq!(
        crate::product::workspace_engine::provider_drive::extract_changelog_summary(
            "# Story Spec\n\n## 改动摘要\n\n## 待确认项\n无。\n"
        ),
        None,
        "空改动摘要返回 None"
    );
}

// spec-design-dialog-revision T5：review 完成路由回 AuthorConfirm（spec「review 结果回对话流」2 场景）。
// 真实完成路径：persistent_test_engine（Story）+ create_reviewer_run_node + complete_review（review/routing.rs）。
// 注意：WorkItem/WorkItemPlan 类型维持既有 HumanConfirm/ReviewDecision 路由（design.md「WorkItem 不受影响」）。

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
