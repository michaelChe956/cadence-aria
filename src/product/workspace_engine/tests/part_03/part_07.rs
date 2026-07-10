#[tokio::test]
async fn work_item_plan_outline_human_confirm_change_uses_outline_revision() {
    let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_outline_human_change");
    let persisted_session = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("workspace sessions")
        .into_iter()
        .next()
        .expect("persisted workspace session");
    engine.session.session_id = persisted_session.id;
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    engine.complete_active_node(Some("准备 Outline review".to_string())).await;

    let review_node_id = engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemPlanOutlineReview,
            agent: Some(ProviderName::Codex),
            stage: WorkspaceStage::CrossReview,
            round: Some(1),
            title: "WorkItemPlan Outline Review Round 1".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await;
    engine
        .update_timeline_node(
            &review_node_id,
            TimelineNodeStatus::Completed,
            Some("结构化输出降级到人工确认".to_string()),
        )
        .await;
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: "review 输出缺少结束 nonce".to_string(),
        summary: "需要人工确认 Outline review".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
        structured_output_diagnostic: Some(StructuredOutputDiagnostic {
            code: "missing_end_nonce".to_string(),
            message: "missing structured output end nonce".to_string(),
            repair_attempted: true,
            repair_succeeded: false,
            raw_output_preview: Some("[must_fix] 伪造 finding 不得进入返修反馈".to_string()),
        }),
    });
    engine
        .enter_human_confirm(Some("等待人工确认 Outline review".to_string()))
        .await;

    let outcome = engine
        .handle_human_confirm(
            HumanConfirmDecision::RequestChange,
            Some(serde_json::json!({"description": "请补齐共享状态影响面"})),
        )
        .await
        .expect("human request change should revise outline");

    let ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { feedback } = outcome else {
        panic!("expected WorkItemPlan Outline revision outcome");
    };
    let feedback = feedback.expect("outline revision feedback");
    assert!(feedback.contains("[impact_closure_contract]"));
    assert!(feedback.contains("请补齐共享状态影响面"));
    assert!(!feedback.contains("伪造 finding"));
    assert_eq!(engine.session().stage, WorkspaceStage::Running);
    assert!(
        !engine.timeline_nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::Revision
                && matches!(
                    node.status,
                    TimelineNodeStatus::Active | TimelineNodeStatus::Paused
                )
        }),
        "outline human revision must not create an active generic Revision node"
    );
    let human_confirm = engine
        .timeline_nodes
        .iter()
        .find(|node| node.node_type == TimelineNodeType::HumanConfirm)
        .expect("human confirm node");
    assert_eq!(human_confirm.status, TimelineNodeStatus::Completed);
    assert_eq!(
        human_confirm.summary.as_deref(),
        Some("Human Confirm 已请求返修 WorkItemPlan Outline")
    );
}

#[tokio::test]
async fn generic_human_confirm_request_change_remains_for_story_design_and_work_item() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session(&format!("sess_generic_human_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.artifact = Some(artifact_payload("# Generic candidate"));
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine
            .enter_human_confirm(Some("等待人工确认".to_string()))
            .await;

        let outcome = engine
            .handle_human_confirm(
                HumanConfirmDecision::RequestChange,
                Some(serde_json::json!({"description": "补充边界条件"})),
            )
            .await
            .expect("generic human request change");

        assert_eq!(
            outcome,
            ReviewDecisionOutcome::StartRevision,
            "{workspace_type:?} should keep the generic revision path"
        );
        assert_eq!(engine.session().stage, WorkspaceStage::Revision);
        assert!(
            engine.timeline_nodes.iter().any(|node| {
                node.node_type == TimelineNodeType::Revision
                    && node.status == TimelineNodeStatus::Active
            }),
            "{workspace_type:?} should create an active generic Revision node"
        );
    }
}

#[test]
fn strict_outline_revising_requires_active_index() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_strict_outline_revising");

    let error = engine
        .mark_work_item_plan_outline_revising()
        .expect_err("strict outline revising should require an active index");

    assert!(error.contains("work item plan active index missing"));
}

#[tokio::test]
async fn item_and_batch_plan_reopen_without_index_enters_save_failure_human_confirm() {
    for (scope, review_node_type) in [
        (
            WorkItemPlanReviewScope::Item,
            TimelineNodeType::WorkItemDraftReview,
        ),
        (
            WorkItemPlanReviewScope::Batch,
            TimelineNodeType::WorkItemBatchReview,
        ),
    ] {
        let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
            make_work_item_plan_engine_with_draft_candidate(&format!(
                "sess_plan_reopen_missing_index_{scope:?}"
            ));
        engine.session.stage = WorkspaceStage::CrossReview;
        if scope == WorkItemPlanReviewScope::Item {
            engine.session.artifact = Some(work_item_draft_artifact_payload(
                &plan_id,
                "outline_a",
                "draft_a",
                WorkItemDraftStatus::Accepted,
            ));
        }
        engine
            .create_timeline_node(TimelineNodeDraft {
                node_type: review_node_type,
                agent: Some(ProviderName::Codex),
                stage: WorkspaceStage::CrossReview,
                round: Some(1),
                title: format!("{scope:?} review"),
                summary: None,
                status: TimelineNodeStatus::Active,
            })
            .await;
        let verdict = ReviewVerdict {
            verdict: ReviewVerdictType::NeedsHuman,
            comments: "需要重开 Outline".to_string(),
            summary: "需要重开 Outline".to_string(),
            findings: Vec::new(),
            review_gate: ReviewGate::UserTriageRequired,
            work_item_plan_review: Some(WorkItemPlanReviewComplete {
                verdict: WorkItemPlanReviewVerdict::PlanReopenRequired,
                review_scope: scope.clone(),
                target_outline_id: (scope == WorkItemPlanReviewScope::Item)
                    .then(|| "outline_a".to_string()),
                generation_round_id: "round_0001".to_string(),
                draft_id: (scope == WorkItemPlanReviewScope::Item)
                    .then(|| "draft_a".to_string()),
                batch_id: (scope == WorkItemPlanReviewScope::Batch)
                    .then(|| "batch_a".to_string()),
                review_action: WorkItemPlanReviewAction::ReviseOutline,
                gates: vec![WorkItemPlanReviewGate::RequiresPlanReopen],
                affects_items: Vec::new(),
                warnings: Vec::new(),
            }),
            structured_output_diagnostic: None,
        };

        match scope {
            WorkItemPlanReviewScope::Item => engine.route_work_item_draft_review(verdict).await,
            WorkItemPlanReviewScope::Batch => engine.route_work_item_batch_review(verdict).await,
            WorkItemPlanReviewScope::Outline => unreachable!(),
        }

        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
        let active_node = engine
            .timeline_nodes
            .iter()
            .find(|node| Some(&node.node_id) == engine.active_node_id.as_ref())
            .expect("active HumanConfirm node");
        assert_eq!(active_node.node_type, TimelineNodeType::HumanConfirm);
        assert_eq!(
            active_node.summary.as_deref(),
            Some("Outline 返修状态保存失败"),
            "{scope:?} must expose strict active-index persistence failure"
        );
    }
}

#[tokio::test]
async fn outline_human_confirm_uses_user_context_once_without_synthetic_review_feedback() {
    let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_outline_human_context_once");
    let persisted_session = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("workspace sessions")
        .into_iter()
        .next()
        .expect("persisted workspace session");
    engine.session.session_id = persisted_session.id;
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    engine
        .complete_active_node(Some("准备 Outline review".to_string()))
        .await;
    let review_node_id = engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemPlanOutlineReview,
            agent: Some(ProviderName::Codex),
            stage: WorkspaceStage::CrossReview,
            round: Some(1),
            title: "WorkItemPlan Outline Review Round 1".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await;
    engine
        .update_timeline_node(
            &review_node_id,
            TimelineNodeStatus::Completed,
            Some("review 降级但 verdict 未恢复".to_string()),
        )
        .await;
    engine.latest_review_verdict = None;
    engine
        .enter_human_confirm(Some("等待人工确认".to_string()))
        .await;

    let outcome = engine
        .handle_human_confirm(
            HumanConfirmDecision::RequestChange,
            Some(serde_json::json!({"description": "只作为用户补充出现一次"})),
        )
        .await
        .expect("outline human revision");

    let ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { feedback } = outcome else {
        panic!("expected outline revision outcome");
    };
    let feedback = feedback.expect("outline feedback");
    assert_eq!(feedback.matches("只作为用户补充出现一次").count(), 1);
    assert!(feedback.contains("用户补充信息"));
    assert!(!feedback.contains("Reviewer 摘要"));
    assert!(!feedback.contains("Reviewer 审核意见"));
    assert!(!feedback.contains("人工请求修改"));
}

#[tokio::test]
async fn latest_completed_item_or_batch_review_keeps_human_confirm_on_generic_revision() {
    for review_node_type in [
        TimelineNodeType::WorkItemDraftReview,
        TimelineNodeType::WorkItemBatchReview,
    ] {
        let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_draft_candidate(&format!(
                "sess_outline_human_negative_{review_node_type:?}"
            ));
        let persisted_session = lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .expect("workspace sessions")
            .into_iter()
            .next()
            .expect("persisted workspace session");
        engine.session.session_id = persisted_session.id;
        prepare_work_item_plan_outline_artifact(&mut engine).await;
        engine
            .complete_active_node(Some("准备 review".to_string()))
            .await;
        let review_node_id = engine
            .create_timeline_node(TimelineNodeDraft {
                node_type: review_node_type.clone(),
                agent: Some(ProviderName::Codex),
                stage: WorkspaceStage::CrossReview,
                round: Some(1),
                title: format!("{review_node_type:?}"),
                summary: None,
                status: TimelineNodeStatus::Active,
            })
            .await;
        engine
            .update_timeline_node(
                &review_node_id,
                TimelineNodeStatus::Completed,
                Some("转人工确认".to_string()),
            )
            .await;
        engine
            .enter_human_confirm(Some("等待人工确认".to_string()))
            .await;

        assert!(
            !engine.human_confirm_should_revise_work_item_plan_outline(),
            "{review_node_type:?} must not route to Outline revision"
        );
        let outcome = engine
            .handle_human_confirm(
                HumanConfirmDecision::RequestChange,
                Some(serde_json::json!({"description": "按最近 review 返修"})),
            )
            .await
            .expect("generic human revision");

        assert_eq!(outcome, ReviewDecisionOutcome::StartRevision);
        assert_eq!(engine.session().stage, WorkspaceStage::Revision);
    }
}

async fn make_atomic_outline_revision_engine(
    session_name: &str,
    with_active_index: bool,
) -> (
    TempDir,
    LifecycleStore,
    String,
    String,
    WorkspaceEngine,
) {
    let (tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate(session_name);
    let persisted_session = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("workspace sessions")
        .into_iter()
        .next()
        .expect("persisted workspace session");
    let session_id = persisted_session.id;
    engine.session.session_id = session_id.clone();
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    if with_active_index {
        save_serial_work_item_plan_index(&engine, &plan_id, "outline_a");
    }
    engine
        .complete_active_node(Some("进入 Outline revision 原子性测试".to_string()))
        .await;
    engine.session.stage = WorkspaceStage::AuthorConfirm;
    let source_node_id = engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemPlanOutlineConfirm,
            agent: None,
            stage: WorkspaceStage::AuthorConfirm,
            round: None,
            title: "WorkItemPlan Outline 确认".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await;
    engine.pending_revision_context = Some("既有返修上下文".to_string());
    engine.work_item_plan_author_retry_count = 2;
    engine.work_item_plan_revision_retry_count = 2;
    lifecycle
        .update_workspace_session_status(&session_id, WorkspaceSessionStatus::WaitingForHuman)
        .expect("set waiting status");
    (tmp, lifecycle, plan_id, source_node_id, engine)
}

fn assert_outline_revision_failure_keeps_engine_state(
    engine: &WorkspaceEngine,
    source_node_id: &str,
) {
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
    assert_eq!(
        engine.pending_revision_context.as_deref(),
        Some("既有返修上下文")
    );
    assert_eq!(engine.work_item_plan_author_retry_count, 2);
    assert_eq!(engine.work_item_plan_revision_retry_count, 2);
    assert!(
        engine
            .artifact_versions
            .last()
            .is_some_and(|version| version.is_current),
        "failed preparation must keep current artifact"
    );
    let source_node = engine
        .timeline_nodes
        .iter()
        .find(|node| node.node_id == source_node_id)
        .expect("source node");
    assert_eq!(source_node.status, TimelineNodeStatus::Active);
    assert_eq!(source_node.summary, None);
}

fn active_index_path(lifecycle: &LifecycleStore, plan_id: &str) -> std::path::PathBuf {
    lifecycle
        .app_paths()
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("work_item_plan_drafts")
        .join(plan_id)
        .join("active_index.json")
}

#[tokio::test]
async fn outline_revision_status_update_failure_keeps_engine_state() {
    let (_tmp, _lifecycle, _plan_id, source_node_id, mut engine) =
        make_atomic_outline_revision_engine("sess_outline_atomic_missing_session", false).await;
    engine.session.session_id = "workspace_session_missing".to_string();

    let error = engine
        .prepare_work_item_plan_outline_revision(
            Some("新返修上下文".to_string()),
            WorkItemPlanOutlineRevisionSource::AuthorConfirm,
            OutlineRevisionPersistencePolicy::AllowMissingInitialRound,
        )
        .await
        .expect_err("missing session status update must fail");

    assert!(error.contains("workspace session status"));
    assert_outline_revision_failure_keeps_engine_state(&engine, &source_node_id);
}

#[cfg(unix)]
#[tokio::test]
async fn outline_revision_status_save_failure_keeps_engine_state() {
    use std::os::unix::fs::PermissionsExt;

    let (_tmp, lifecycle, _plan_id, source_node_id, mut engine) =
        make_atomic_outline_revision_engine("sess_outline_atomic_status_save", false).await;
    let session_id = engine.session.session_id.clone();
    let sessions_root = lifecycle
        .app_paths()
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("workspace-sessions");
    let original_permissions = std::fs::metadata(&sessions_root)
        .expect("workspace sessions metadata")
        .permissions();
    std::fs::set_permissions(&sessions_root, std::fs::Permissions::from_mode(0o555))
        .expect("make workspace sessions read-only");

    let result = engine
        .prepare_work_item_plan_outline_revision(
            Some("新返修上下文".to_string()),
            WorkItemPlanOutlineRevisionSource::AuthorConfirm,
            OutlineRevisionPersistencePolicy::AllowMissingInitialRound,
        )
        .await;

    std::fs::set_permissions(&sessions_root, original_permissions)
        .expect("restore workspace sessions permissions");
    let error = result.expect_err("workspace session status save must fail");
    assert!(error.contains("update workspace session status"));
    assert_eq!(
        lifecycle
            .get_workspace_session(&session_id)
            .expect("workspace session")
            .status,
        WorkspaceSessionStatus::WaitingForHuman
    );
    assert_outline_revision_failure_keeps_engine_state(&engine, &source_node_id);
}

#[tokio::test]
async fn outline_revision_active_index_load_failure_rolls_back_session_and_engine_state() {
    let (_tmp, lifecycle, plan_id, source_node_id, mut engine) =
        make_atomic_outline_revision_engine("sess_outline_atomic_load_failure", false).await;
    let session_id = engine.session.session_id.clone();
    let index_path = active_index_path(&lifecycle, &plan_id);
    std::fs::create_dir_all(index_path.parent().expect("active index parent"))
        .expect("create active index parent");
    std::fs::write(&index_path, b"not-json").expect("corrupt active index");

    let error = engine
        .prepare_work_item_plan_outline_revision(
            Some("新返修上下文".to_string()),
            WorkItemPlanOutlineRevisionSource::ReviewDecision,
            OutlineRevisionPersistencePolicy::AllowMissingInitialRound,
        )
        .await
        .expect_err("active index load failure must fail");

    assert!(error.contains("load work item plan active index failed"));
    assert_eq!(
        lifecycle
            .get_workspace_session(&session_id)
            .expect("persisted session")
            .status,
        WorkspaceSessionStatus::WaitingForHuman
    );
    assert_outline_revision_failure_keeps_engine_state(&engine, &source_node_id);
}

#[cfg(unix)]
#[tokio::test]
async fn outline_revision_active_index_save_failure_rolls_back_session_and_engine_state() {
    use std::os::unix::fs::PermissionsExt;

    let (_tmp, lifecycle, plan_id, source_node_id, mut engine) =
        make_atomic_outline_revision_engine("sess_outline_atomic_save_failure", true).await;
    let session_id = engine.session.session_id.clone();
    let index_path = active_index_path(&lifecycle, &plan_id);
    let index_parent = index_path.parent().expect("active index parent");
    let original_permissions = std::fs::metadata(index_parent)
        .expect("active index parent metadata")
        .permissions();
    let mut readonly_permissions = original_permissions.clone();
    readonly_permissions.set_mode(0o555);
    std::fs::set_permissions(index_parent, readonly_permissions)
        .expect("make active index parent readonly");

    let result = engine
        .prepare_work_item_plan_outline_revision(
            Some("新返修上下文".to_string()),
            WorkItemPlanOutlineRevisionSource::HumanConfirm,
            OutlineRevisionPersistencePolicy::AllowMissingInitialRound,
        )
        .await;
    std::fs::set_permissions(index_parent, original_permissions)
        .expect("restore active index parent permissions");
    let error = result.expect_err("active index save failure must fail");

    assert!(error.contains("save work item plan active index failed"));
    assert_eq!(
        lifecycle
            .get_workspace_session(&session_id)
            .expect("persisted session")
            .status,
        WorkspaceSessionStatus::WaitingForHuman
    );
    assert_outline_revision_failure_keeps_engine_state(&engine, &source_node_id);
}
