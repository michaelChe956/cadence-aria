// spec-design-dialog-revision T2：provisional reviewer 快照持久化闭环
// serde 兼容（旧 record 无新字段反序列化）+ from_record 恢复用例 + start_generation 写入路径用例（T2 fix1）
use super::*;
use crate::product::models::{ProviderName, WorkspaceSessionRecord, WorkspaceSessionStatus};

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
// pub(super)：author_revision_review_routing 模块（T5 路由用例）复用同一构造器。

pub(super) fn prompt_engine_with_artifact(markdown: &str) -> WorkspaceEngine {
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
fn author_revision_prompt_story_and_work_item_remain_byte_for_byte_unchanged() {
    let story = prompt_engine_with_artifact("# Story Spec\n\n旧内容");
    let story_expected = "请作为 author 基于用户反馈对当前 Workspace 产物做增量修订。\n\n\
## 修订规则\n\
- 只修改与反馈相关的部分，保持其余章节原样（增量修订，不是重写）。\n\
- 若反馈要求全部重写（如方向调整），保留仍然有效的事实性内容并整体重组。\n\
- 输出修订后的完整产物正文（markdown），并在文末追加「## 改动摘要」小节：逐条列出本次改动的位置与原因。\n\n\
## 当前产物\n\n\
```\n\
# Story Spec\n\n\
旧内容\n\
```\n\n\
## 用户反馈\n\n\
补充异常场景与回滚策略\n";
    for resumed_session in [false, true] {
        assert_eq!(
            story.build_author_revision_prompt("补充异常场景与回滚策略", resumed_session),
            story_expected
        );
    }

    let mut work_item = prompt_engine_with_artifact("# Work Item\n\n旧内容");
    work_item.session.workspace_type = WorkspaceType::WorkItem;
    let work_item_expected = "请作为 author 基于用户反馈对当前 Workspace 产物做增量修订。\n\n\
## 修订规则\n\
- 只修改与反馈相关的部分，保持其余章节原样（增量修订，不是重写）。\n\
- 若反馈要求全部重写（如方向调整），保留仍然有效的事实性内容并整体重组。\n\
- 输出修订后的完整产物正文（markdown），并在文末追加「## 改动摘要」小节：逐条列出本次改动的位置与原因。\n\n\
## 当前产物\n\n\
```\n\
# Work Item\n\n\
旧内容\n\
```\n\n\
## 用户反馈\n\n\
补充验证命令\n";
    for resumed_session in [false, true] {
        assert_eq!(
            work_item.build_author_revision_prompt("补充验证命令", resumed_session),
            work_item_expected
        );
    }
}

#[test]
fn author_revision_prompt_includes_feedback_and_changelog_section() {
    let engine = prompt_engine_with_artifact("# Story Spec\n\n旧内容");
    let prompt = engine.build_author_revision_prompt("补充异常场景与回滚策略", false);
    assert!(prompt.contains("补充异常场景与回滚策略"));
    assert!(prompt.contains("# Story Spec"));
    assert!(
        prompt.contains("改动摘要"),
        "必须要求输出改动摘要小节: {prompt}"
    );
    assert!(prompt.contains("增量修订"), "约束不得整体重写无关章节");
}

#[tokio::test]
async fn design_author_revision_prompt_includes_output_contract_skeleton_and_context_note() {
    let mut engine = prompt_engine_with_artifact("# Design Spec\n\n旧内容");
    engine.session.workspace_type = WorkspaceType::Design;
    engine
        .append_completed_timeline_event(
            TimelineNodeType::ContextNote,
            WorkspaceStage::PrepareContext,
            "上下文补充".to_string(),
            Some("补充上下文：保留现有 API 兼容性。".to_string()),
            TimelineNodeStatus::Completed,
            false,
        )
        .await;

    for resumed_session in [false, true] {
        let prompt = engine.build_author_revision_prompt("补充失败路径的设计决策", resumed_session);

        assert!(prompt.contains("[artifact_schema_contract]"), "{prompt}");
        assert!(
            prompt.contains("原始返回必须使用完整 artifact fenced block"),
            "{prompt}"
        );
        assert!(
            prompt.contains("上一版 Artifact 是 daemon 已提取的 markdown"),
            "{prompt}"
        );
        assert!(prompt.contains("四反引号 ````artifact"), "{prompt}");
        assert!(prompt.contains("# Design Spec 标题"), "{prompt}");
        assert!(prompt.contains("## 设计决策"), "{prompt}");
        assert!(prompt.contains("准备阶段用户补充上下文"), "{prompt}");
        assert!(
            prompt.contains("补充上下文：保留现有 API 兼容性。"),
            "{prompt}"
        );
        assert!(
            prompt.contains("输入围栏仅用于界定材料，输出请按 artifact fence 契约重新包裹"),
            "{prompt}"
        );
        assert!(
            prompt.contains("````\n# Design Spec\n\n旧内容\n````"),
            "{prompt}"
        );
        assert!(
            !prompt.contains("会话上下文（滑动窗口压缩"),
            "反馈返修入口本期不得注入 compact_history: {prompt}"
        );
    }
}

#[test]
fn design_author_revision_four_backtick_input_fence_keeps_embedded_code_and_feedback_separate() {
    let artifact =
        "# Design Spec\n\n## 代码示例\n```rust\nlet retained = true;\n```\n\n## 仍属于产物的章节";
    let mut engine = prompt_engine_with_artifact(artifact);
    engine.session.workspace_type = WorkspaceType::Design;

    let prompt = engine.build_author_revision_prompt("反馈必须位于当前产物边界外", false);
    let artifact_start = prompt
        .find("````\n")
        .expect("Design current artifact must begin with a four-backtick fence");
    let artifact_end = prompt[artifact_start + 5..]
        .find("\n````\n\n## 用户反馈\n\n")
        .map(|offset| artifact_start + 5 + offset)
        .expect("Design current artifact must end before user feedback");
    let artifact_region = &prompt[artifact_start..artifact_end];

    assert!(artifact_region.contains("```rust\nlet retained = true;\n```"));
    assert!(artifact_region.contains("## 仍属于产物的章节"));
    assert_eq!(
        prompt[artifact_end + "\n````\n\n## 用户反馈\n\n".len()..]
            .lines()
            .next(),
        Some("反馈必须位于当前产物边界外")
    );
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

// ============================================================================
// spec-design-dialog-revision T6：存量会话迁移（HumanConfirm/ReviewDecision → AuthorConfirm）
// 任务3.1：Story/Design 的 HumanConfirm/ReviewDecision 已退役，存量恢复时迁移回 AuthorConfirm。
// 覆盖：单元（Story/Design × 两退役阶段迁移、WorkItemPlan 不迁、其他阶段不迁、产物保留）
//       + 持久化集成路径（from_record → new_persistent fallback 链）。
// ============================================================================

fn story_session_with_stage(
    workspace_type: WorkspaceType,
    stage: WorkspaceStage,
    artifact: Option<ArtifactPayload>,
) -> WorkspaceSession {
    let mut session = make_session("sess_retired_fallback");
    session.workspace_type = workspace_type;
    session.stage = stage;
    session.artifact = artifact;
    session
}

#[test]
fn retired_stage_fallback_migrates_story_design_sessions() {
    for stage in [WorkspaceStage::HumanConfirm, WorkspaceStage::ReviewDecision] {
        for workspace_type in [WorkspaceType::Story, WorkspaceType::Design] {
            let session = story_session_with_stage(
                workspace_type.clone(),
                stage.clone(),
                Some(ArtifactPayload::Markdown {
                    markdown: "# Story".to_string(),
                    diff: None,
                }),
            );
            let migrated =
                crate::product::workspace_engine::lifecycle::recover_story_design_retired_stage_fallback(
                    session,
                );
            assert_eq!(
                migrated.stage,
                WorkspaceStage::AuthorConfirm,
                "{workspace_type:?} 处于 {stage:?} 的存量会话必须迁移回 AuthorConfirm"
            );
            assert!(migrated.artifact.is_some(), "产物必须保留");
        }
    }
    // WorkItemPlan 不受影响（HumanConfirm 对 WorkItemPlan 仍有效）
    let plan_session = story_session_with_stage(
        WorkspaceType::WorkItemPlan,
        WorkspaceStage::HumanConfirm,
        None,
    );
    assert_eq!(
        crate::product::workspace_engine::lifecycle::recover_story_design_retired_stage_fallback(
            plan_session
        )
        .stage,
        WorkspaceStage::HumanConfirm
    );
    // 其他阶段不迁移
    let running = story_session_with_stage(WorkspaceType::Story, WorkspaceStage::Running, None);
    assert_eq!(
        crate::product::workspace_engine::lifecycle::recover_story_design_retired_stage_fallback(
            running
        )
        .stage,
        WorkspaceStage::Running
    );
}

#[tokio::test]
async fn legacy_record_with_review_decision_restores_to_author_confirm_via_persistent_path() {
    // 旧存量 Story 会话：record 持久化在 HumanConfirm（status=WaitingForHuman）+ 评审报告消息。
    // 经 from_record → new_persistent 恢复后迁移回 AuthorConfirm；fallback 不动 messages，
    // 评审报告保留在消息流（verdict 恢复语义保留在消息流）。
    let (_tmp, checkpoint_store) = setup();
    let app_root = tempfile::tempdir().expect("app root");
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(app_root.path().join(".aria")));
    let record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .unwrap();
    lifecycle_store
        .append_workspace_message(
            &record.id,
            "assistant".to_string(),
            "# Story Spec\n\n候选内容".to_string(),
        )
        .unwrap();
    lifecycle_store
        .append_workspace_message(
            &record.id,
            "reviewer".to_string(),
            "评审报告：建议补充异常场景。".to_string(),
        )
        .unwrap();
    let _ = lifecycle_store
        .update_workspace_session_status(&record.id, WorkspaceSessionStatus::WaitingForHuman)
        .unwrap();

    let persisted = lifecycle_store.get_workspace_session(&record.id).unwrap();
    let engine = WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle_store,
        mpsc::channel(64).0,
        WorkspaceSession::from_record(persisted),
    );

    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
    assert!(
        engine
            .session()
            .messages
            .iter()
            .any(|message| message.role == "reviewer"),
        "评审报告必须保留在消息流"
    );
    assert!(
        engine.session().messages.len() >= 2,
        "消息不得被 fallback 截断或清空"
    );
}

// Minor-4（T6）：decisions.rs Accept 兼容路由 None 分支（旧记录无 reviewer_enabled_at_start）
// 在无 reviewer 时此前会 enter_human_confirm——Story/Design 已退役该阶段，运行中会重新落入。
// 修复：Story/Design 走 Some(false) 同语义直接定稿；WorkItemPlan 保留 HumanConfirm（仍有效）。
#[tokio::test]
async fn legacy_accept_without_reviewer_finalizes_for_story_design_not_human_confirm() {
    for workspace_type in [WorkspaceType::Story, WorkspaceType::Design] {
        let (_tmp, store) = setup();
        let (tx, _rx) = mpsc::channel(64);
        let mut session = make_session("sess_legacy_no_reviewer");
        session.workspace_type = workspace_type.clone();
        session.stage = WorkspaceStage::AuthorConfirm;
        session.artifact = Some(ArtifactPayload::Markdown {
            markdown: "# Story Spec\n\n候选内容".to_string(),
            diff: None,
        });
        session.reviewer_provider = None;
        session.review_rounds = 0;
        session.reviewer_enabled_at_start = None;
        let mut engine = WorkspaceEngine::new(store, tx, session);

        let outcome = engine
            .handle_author_decision(AuthorDecision::Accept)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            AuthorDecisionOutcome::Finalized,
            "{workspace_type:?} legacy None + 无 reviewer 必须直接定稿，不得落入已退役 HumanConfirm"
        );
        assert_eq!(engine.session().stage, WorkspaceStage::Completed);
    }

    // WorkItemPlan 不受影响：legacy None + 无 reviewer 仍进 HumanConfirm（该阶段对 WorkItemPlan 有效）。
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_legacy_no_reviewer_plan");
    session.workspace_type = WorkspaceType::WorkItemPlan;
    session.stage = WorkspaceStage::AuthorConfirm;
    session.reviewer_provider = None;
    session.review_rounds = 0;
    session.reviewer_enabled_at_start = None;
    let mut engine = WorkspaceEngine::new(store, tx, session);

    let outcome = engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();
    assert_eq!(outcome, AuthorDecisionOutcome::HumanConfirm);
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
}

// ============================================================================
// spec-design-dialog-revision T7：修订断线恢复扩展（Revision 恢复臂）
// 任务3.3：InterruptedRunRecoveryOutcome::Revision + 检测失败修订节点 + retry 臂 + inbound 映射。
// 节点类型决策：失败修订节点采用 Revise/Review-revise 臂【实际创建】的 TimelineNodeType::Revision
// （decisions.rs:159/466/618），而非 AuthorRun——修订 run 期间 active node 即 Revision 节点
// （drive_revision_session 完成后 complete_active_node 结束的是同一节点），断线时
// append_aborted_by_disconnect 将其标记 Failed（"连接断开，运行已中止"）。
// 覆盖 spec 两红线：①修订 run 未完成时重连（保留修订前产物 + 提供重试 + 无部分写入）；
// ②修订 run 完成后重连（provider_drive 完成路径已处理，验证修订版回 AuthorConfirm 且不再提供恢复）。
// ============================================================================

fn interrupted_revision_run_engine() -> (TempDir, WorkspaceEngine) {
    let (_tmp, checkpoint_store) = setup();
    let (tx, _rx) = mpsc::channel(8);
    let mut session = make_session("sess_interrupted_revision_run");
    session.stage = WorkspaceStage::PrepareContext;
    let payload = artifact_payload("# Story Spec\n\n修订前产物");
    session.artifact = Some(payload.clone());
    let mut engine = WorkspaceEngine::new(checkpoint_store, tx, session);
    engine.artifact_versions = vec![ArtifactVersion {
        version: 1,
        payload,
        generated_by: ProviderName::ClaudeCode,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: "2026-07-11T17:00:00Z".to_string(),
        source_node_id: "timeline_node_002".to_string(),
    }];
    engine.timeline_nodes = vec![
        interrupted_recovery_timeline_node(
            "timeline_node_002",
            TimelineNodeType::AuthorRun,
            TimelineNodeStatus::Completed,
            WsWorkspaceStage::Running,
            Some("修订前产物生成".to_string()),
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_003",
            TimelineNodeType::Revision,
            TimelineNodeStatus::Failed,
            WsWorkspaceStage::Revision,
            Some("连接断开，运行已中止".to_string()),
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_004",
            TimelineNodeType::AbortedByDisconnect,
            TimelineNodeStatus::Failed,
            WsWorkspaceStage::PrepareContext,
            Some("last_active_run_id: run-11".to_string()),
        ),
    ];
    (_tmp, engine)
}

// 红线①「修订 run 未完成时重连」：保留修订前产物 + 提供重试（Revision 恢复臂）+ 无部分写入。
#[tokio::test]
async fn interrupted_revision_run_is_recoverable_and_retryable() {
    let (_tmp, mut engine) = interrupted_revision_run_engine();

    let recoverable = engine
        .recoverable_interrupted_run()
        .expect("修订 run 应可恢复");
    assert_eq!(
        recoverable.operation,
        RecoverableInterruptedOperation::Revision
    );
    assert_eq!(recoverable.label, "重试中断修订");

    let outcome = engine
        .retry_interrupted_run(&recoverable.failed_node_id)
        .await
        .expect("retry interrupted revision run");
    assert_eq!(outcome, InterruptedRunRecoveryOutcome::Revision);
    assert_eq!(engine.session().stage, WorkspaceStage::Running);

    let retry_node = engine.timeline_nodes.last().expect("retry node");
    assert_eq!(retry_node.node_type, TimelineNodeType::Revision);
    assert_eq!(retry_node.status, TimelineNodeStatus::Active);
    let retry = retry_node.retry.as_ref().expect("retry metadata");
    assert_eq!(retry.retry_of_node_id, recoverable.failed_node_id);
    assert_eq!(retry.retry_attempt, 1);
    assert_eq!(retry.retry_reason, "aborted_by_disconnect");

    // 修订前产物保留（无部分写入）：断线时不得清空或半写产物。
    assert!(
        engine
            .session()
            .artifact
            .as_ref()
            .expect("修订前产物必须保留")
            .markdown_or_empty()
            .contains("修订前产物"),
        "修订 run 断线必须保留修订前产物，不得有部分写入"
    );
}

// 红线②「修订 run 完成后重连」：provider_drive 完成路径已处理——修订版应用到产物并回 AuthorConfirm，
// 且会话无失败修订节点（recoverable_interrupted_run 为 None），不提供多余恢复入口。
#[tokio::test]
async fn completed_revision_run_reconnects_to_author_confirm_with_revised_artifact() {
    let (_tmp, store) = setup();
    // drive_revision_session 会发射超过 8 个 engine 事件（prompt/StreamChunk/ExecutionEvent/
    // stage 与 node 变更）：receiver 存活却不消费时，event_tx 容量占满后 send().await 永久阻塞。
    // 按 part_04.rs 驱动类测试惯例直接丢弃 receiver——engine 侧全部 `let _ = send().await`，
    // 对已关闭 channel 立即返回 Err 并被丢弃，不会阻塞也不会影响断言。
    let (tx, _) = mpsc::channel(8);
    let mut session = make_session("sess_completed_revision_run");
    session.stage = WorkspaceStage::Revision;
    session.artifact = Some(artifact_payload("# Story Spec\n\n修订前产物"));
    let mut engine = WorkspaceEngine::new(store, tx, session);
    engine.pending_revision_context = Some("补充异常场景".to_string());
    engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::Revision,
            agent: Some(ProviderName::ClaudeCode),
            stage: WorkspaceStage::Revision,
            round: None,
            title: "反馈修订".to_string(),
            summary: Some("补充异常场景".to_string()),
            status: TimelineNodeStatus::Active,
        })
        .await;

    engine
        .drive_revision_session(
            Arc::new(ReviewVerdictStreamingProvider {
                // 输出必须是完整 Story 产物（必需小节 + source id + [REQ-*]/[AC-*]），否则
                // Completed 分支的 artifact gate（content_has_complete_workspace_artifact）
                // 判失败并 finish_failed_run 回 PrepareContext，而非回 AuthorConfirm。
                output: "# Story Spec\n\n\
                    ## 范围\n来源 source id: Issue issue_0001；修订后产物：补充异常场景。\n\n\
                    ## 用户故事\n作为用户，我希望异常场景有明确处理。\n\n\
                    ## 功能需求\n- [REQ-001] 登录成功路径。\n- [REQ-002] 补充异常场景。\n\n\
                    ## 成功标准\n- [AC-001] 覆盖异常场景。\n\n\
                    ## 待确认项\n无。\n\n\
                    ## 非功能需求\n无。\n\n\
                    ## 改动摘要\n- 补充异常场景 [REQ-002]\n",
                provider_type: Arc::new(Mutex::new(None)),
                prompt: Arc::new(Mutex::new(None)),
            }),
            empty_provider_commands(),
        )
        .await;

    assert_eq!(
        engine.session().stage,
        WorkspaceStage::AuthorConfirm,
        "修订 run 完成后重连必须回 AuthorConfirm"
    );
    assert!(
        engine
            .session()
            .artifact
            .as_ref()
            .expect("修订产物")
            .markdown_or_empty()
            .contains("修订后产物"),
        "修订版必须应用到产物"
    );
    assert!(
        engine.recoverable_interrupted_run().is_none(),
        "修订完成后无失败节点，不得提供恢复入口"
    );
}

// ============================================================================
// T7 fix1（Finding-A，Important）：真实断线重连后，重试【用户反馈修订】run 必须能启动并完成。
// 修复前链路：socket 每连接重建 engine → new_persistent 将 pending_revision_context 置 None
// （该字段不在 WorkspaceSessionRecord，不随会话记录恢复）→ retry 后 drive_revision_session →
// build_revision_input 中 is_author_feedback_revision()=false（pending=None）且
// latest_review_verdict=None → Err("review verdict is unavailable for revision") →
// finish_failed_run 回 PrepareContext。本测试走真实持久化路径：第一阶段用真实 Revise 臂
// 启动修订 run，第二阶段 new_persistent 从磁盘重建 engine 后 retry 并驱动 retried run 完成。
// ============================================================================

#[tokio::test]
async fn retried_author_feedback_revision_completes_after_real_reconnect() {
    let (_tmp, checkpoint_store) = setup();
    let app_root = tempfile::tempdir().expect("app root");
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(app_root.path().join(".aria")));
    // review_rounds=0：未启用 review 的用户反馈修订（最主流路径），重连后 verdict 恒 None，
    // 正是 Finding-A 中 Err("review verdict is unavailable for revision") 的触发条件。
    let record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 0,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .unwrap();

    // 第一次连接：AuthorConfirm + 产物 v1（current 版本 source=AuthorRun 节点），
    // 用户提交反馈进入修订 run（真实 Revise 臂：创建 Revision 节点并置 pending 上下文）。
    let (tx, _) = mpsc::channel(64);
    let mut first = WorkspaceEngine::new_persistent(
        checkpoint_store.clone(),
        lifecycle_store.clone(),
        tx,
        WorkspaceSession::from_record(lifecycle_store.get_workspace_session(&record.id).unwrap()),
    );
    let payload = artifact_payload("# Story Spec\n\n修订前产物");
    first.session.artifact = Some(payload.clone());
    first.artifact_versions = vec![ArtifactVersion {
        version: 1,
        payload,
        generated_by: ProviderName::ClaudeCode,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: "2026-08-14T17:00:00Z".to_string(),
        source_node_id: "timeline_node_002".to_string(),
    }];
    first.timeline_nodes = vec![
        interrupted_recovery_timeline_node(
            "timeline_node_001",
            TimelineNodeType::PrepareContext,
            TimelineNodeStatus::Completed,
            WsWorkspaceStage::PrepareContext,
            Some("上下文已就绪".to_string()),
        ),
        interrupted_recovery_timeline_node(
            "timeline_node_002",
            TimelineNodeType::AuthorRun,
            TimelineNodeStatus::Completed,
            WsWorkspaceStage::Running,
            Some("修订前产物生成".to_string()),
        ),
    ];
    first.active_node_id = Some("timeline_node_002".to_string());
    first.persist_timeline_nodes();
    first.persist_artifact_versions();
    first.session.stage = WorkspaceStage::AuthorConfirm;

    let outcome = first
        .handle_author_decision(AuthorDecision::Revise {
            feedback: "补充异常场景".to_string(),
        })
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        AuthorDecisionOutcome::StartRevision { .. }
    ));
    assert_eq!(first.session().stage, WorkspaceStage::Revision);
    // 修订 run 进行程中断线：socket 层丢弃 engine，内存态（pending_revision_context）随之消失。

    // 真实重连：new_persistent 从磁盘重建 engine（socket.rs 每连接重建的同一路径）。
    let (tx2, _) = mpsc::channel(8);
    let mut reconnected = WorkspaceEngine::new_persistent(
        checkpoint_store.clone(),
        lifecycle_store.clone(),
        tx2,
        WorkspaceSession::from_record(lifecycle_store.get_workspace_session(&record.id).unwrap()),
    );
    assert_eq!(reconnected.session().stage, WorkspaceStage::Revision);
    // Finding-A 根因前置：pending_revision_context 不在会话记录中，重连后必然丢失。
    assert!(
        reconnected.pending_revision_context.is_none(),
        "前置：重连后 pending_revision_context 丢失（Finding-A 根因）"
    );
    // 重连既有前置：Revision 阶段的 stale run 归位 PrepareContext（lifecycle.rs 既有路径）。
    reconnected
        .recover_stale_active_run_after_disconnect()
        .await;
    assert_eq!(reconnected.session().stage, WorkspaceStage::PrepareContext);

    let recoverable = reconnected
        .recoverable_interrupted_run()
        .expect("用户反馈修订 run 断线后应可恢复");
    assert_eq!(
        recoverable.operation,
        RecoverableInterruptedOperation::Revision
    );

    // retry：必须重建用户反馈上下文，否则 retried run 无法走 author 反馈 prompt 分支。
    reconnected
        .retry_interrupted_run(&recoverable.failed_node_id)
        .await
        .unwrap();
    assert!(
        reconnected.is_author_feedback_revision(),
        "重连后 retry 必须恢复用户反馈修订上下文（Finding-A：否则 build_revision_input 报 \
         review verdict is unavailable for revision）"
    );

    // 端到端：驱动 retried run 完成。prompt 捕获自 fixture provider，断言走 author 反馈
    // prompt 分支且携带断线前的用户反馈全文；产物 gate 需要完整 Story 产物小节。
    let prompt = Arc::new(Mutex::new(None));
    reconnected
        .drive_revision_session(
            Arc::new(ReviewVerdictStreamingProvider {
                output: "# Story Spec\n\n\
                    ## 范围\n来源 source id: Issue issue_0001；修订后产物：补充异常场景。\n\n\
                    ## 用户故事\n作为用户，我希望异常场景有明确处理。\n\n\
                    ## 功能需求\n- [REQ-001] 登录成功路径。\n- [REQ-002] 补充异常场景。\n\n\
                    ## 成功标准\n- [AC-001] 覆盖异常场景。\n\n\
                    ## 待确认项\n无。\n\n\
                    ## 非功能需求\n无。\n\n\
                    ## 改动摘要\n- 补充异常场景 [REQ-002]\n",
                provider_type: Arc::new(Mutex::new(None)),
                prompt: prompt.clone(),
            }),
            empty_provider_commands(),
        )
        .await;

    let captured_prompt = prompt.lock().unwrap().clone().expect("retried run prompt");
    assert!(
        captured_prompt.contains("## 用户反馈"),
        "retried run 必须走 author 反馈 prompt 分支（而非 reviewer 返修 prompt）"
    );
    assert!(
        captured_prompt.contains("补充异常场景"),
        "retried run prompt 必须携带断线前的用户反馈全文"
    );
    assert_eq!(
        reconnected.session().stage,
        WorkspaceStage::AuthorConfirm,
        "retried 修订 run 必须完成并回 AuthorConfirm，而非 Err 后 finish_failed_run 回 PrepareContext"
    );
    assert!(
        reconnected
            .session()
            .artifact
            .as_ref()
            .expect("修订产物")
            .markdown_or_empty()
            .contains("修订后产物"),
        "修订版必须应用到产物"
    );
}
