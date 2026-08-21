#[test]
fn persistent_engine_accepts_complete_revision_artifact_misclassified_as_text_fallback() {
    for (workspace_type, original_artifact, revised_artifact) in [
        (
            WorkspaceType::Story,
            complete_story_artifact("保留原始需求。", "原始需求可验收。"),
            complete_story_artifact(
                "用户遇到失败时应该如何处理？",
                "失败路径有明确提示。",
            ),
        ),
        (
            WorkspaceType::Design,
            complete_design_artifact("保留原始设计。", "保留原始 API。"),
            complete_design_artifact(
                "失败时应该如何处理？",
                "返回类型化失败原因。",
            ),
        ),
    ] {
        let (_tmp, checkpoint_store) = setup();
        let app_root = tempfile::tempdir().expect("app root");
        let app_paths = ProductAppPaths::new(app_root.path().join(".aria"));
        let lifecycle_store = LifecycleStore::new(app_paths);
        let story = lifecycle_store
            .create_story_spec(CreateStorySpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repository_0001".to_string(),
                title: "Story".to_string(),
            aggregate_codebase: None,
            })
            .expect("story spec");
        let entity_id = match &workspace_type {
            WorkspaceType::Story => story.id.clone(),
            WorkspaceType::Design => lifecycle_store
                .create_design_spec(CreateDesignSpecInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    story_spec_ids: vec![story.id.clone()],
                    title: "Design".to_string(),
                aggregate_codebase: None,
                })
                .expect("design spec")
                .id,
            _ => unreachable!("text fallback recovery only applies to story and design"),
        };
        let session_record = lifecycle_store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: entity_id.clone(),
                workspace_type: workspace_type.clone(),
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: ProviderName::Codex,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .expect("workspace session");
        lifecycle_store
            .append_version(AppendSpecVersionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: entity_id.clone(),
                markdown: original_artifact.clone(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: None,
            })
            .expect("initial spec version");
        lifecycle_store
            .save_artifact_versions(
                &session_record.id,
                &[ArtifactVersion {
                    version: 1,
                    payload: artifact_payload(&original_artifact),
                    generated_by: ProviderName::ClaudeCode,
                    reviewed_by: Some(ProviderName::Codex),
                    review_verdict: Some(ReviewVerdictType::NeedsHuman),
                    confirmed_by: None,
                    is_current: true,
                    created_at: "2026-07-11T13:30:00Z".to_string(),
                    source_node_id: "timeline_node_002".to_string(),
                }],
            )
            .expect("initial artifact version");
        let provider_output = format!(
            "基于 reviewer 意见完成两处返修：\n\n\
             1. 补齐失败处理。\n\
             2. 保留既有兼容约束。\n\n\
             ````artifact\n{revised_artifact}````\n"
        );
        lifecycle_store
            .replace_workspace_messages(
                &session_record.id,
                vec![
                    WorkspaceMessageRecord {
                        role: "system".to_string(),
                        content: "context".to_string(),
                        created_at: "2026-07-11T13:20:00Z".to_string(),
                    },
                    WorkspaceMessageRecord {
                        role: "user".to_string(),
                        content: "按 reviewer 意见返修".to_string(),
                        created_at: "2026-07-11T13:34:00Z".to_string(),
                    },
                    WorkspaceMessageRecord {
                        role: "assistant".to_string(),
                        content: provider_output,
                        created_at: "2026-07-11T13:35:00Z".to_string(),
                    },
                ],
            )
            .expect("persist provider output");
        lifecycle_store
            .save_timeline_nodes(
                &session_record.id,
                &[TimelineNode {
                    node_id: "timeline_node_006".to_string(),
                    node_type: TimelineNodeType::Revision,
                    agent: Some(ProviderName::ClaudeCode),
                    stage: WsWorkspaceStage::Revision,
                    round: Some(1),
                    status: TimelineNodeStatus::Paused,
                    title: "返修 Round 1".to_string(),
                    summary: Some("等待用户选择".to_string()),
                    started_at: "2026-07-11T13:34:00Z".to_string(),
                    completed_at: None,
                    duration_ms: None,
                    artifact_ref: Some("artifact_current".to_string()),
                    provider_config_snapshot: ProviderConfigSnapshot {
                        author: ProviderName::ClaudeCode,
                        reviewer: Some(ProviderName::Codex),
                        review_rounds: 1,
                        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
                    },
                    retry: None,
                }],
            )
            .expect("persist paused revision");

        let session = WorkspaceSession::from_record(
            lifecycle_store
                .get_workspace_session(&session_record.id)
                .expect("reload session"),
        );
        let (tx, _rx) = mpsc::channel(8);
        let engine = WorkspaceEngine::new_persistent(
            checkpoint_store.clone(),
            lifecycle_store.clone(),
            tx,
            session,
        );

        assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
        assert!(engine.pending_author_choice_request_message().is_none());
        assert_eq!(
            engine
                .session()
                .artifact
                .as_ref()
                .map(ArtifactPayload::markdown_or_empty),
            Some(revised_artifact.trim())
        );
        assert!(engine.timeline_nodes.iter().any(|node| {
            node.node_id == "timeline_node_006" && node.status == TimelineNodeStatus::Completed
        }));
        assert!(engine.timeline_nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::AuthorConfirm
                && node.status == TimelineNodeStatus::Active
        }));
        let spec_versions = lifecycle_store
            .list_versions("project_0001", "issue_0001", &entity_id)
            .expect("spec versions");
        assert_eq!(spec_versions.len(), 2);
        assert_eq!(
            spec_versions.last().map(|version| version.markdown.as_str()),
            Some(revised_artifact.trim())
        );
        let current_version = match workspace_type {
            WorkspaceType::Story => lifecycle_store
                .list_story_specs("project_0001", "issue_0001")
                .expect("story specs")
                .into_iter()
                .find(|spec| spec.id == entity_id)
                .and_then(|spec| spec.current_version),
            WorkspaceType::Design => lifecycle_store
                .list_design_specs("project_0001", "issue_0001")
                .expect("design specs")
                .into_iter()
                .find(|spec| spec.id == entity_id)
                .and_then(|spec| spec.current_version),
            _ => unreachable!("text fallback recovery only applies to story and design"),
        };
        assert_eq!(current_version, Some(2));
        assert_eq!(
            lifecycle_store
                .get_workspace_session(&session_record.id)
                .expect("recovered workspace session")
                .status,
            WorkspaceSessionStatus::WaitingForHuman
        );

        let reloaded_session = WorkspaceSession::from_record(
            lifecycle_store
                .get_workspace_session(&session_record.id)
                .expect("reload recovered session"),
        );
        let (reload_tx, _reload_rx) = mpsc::channel(8);
        let reloaded = WorkspaceEngine::new_persistent(
            checkpoint_store.clone(),
            lifecycle_store.clone(),
            reload_tx,
            reloaded_session,
        );
        assert_eq!(reloaded.session().stage, WorkspaceStage::AuthorConfirm);
        assert_eq!(
            reloaded
                .timeline_nodes
                .iter()
                .filter(|node| node.node_type == TimelineNodeType::AuthorConfirm)
                .count(),
            1,
            "重复恢复不应追加第二个 Author Confirm 节点"
        );
        assert_eq!(
            lifecycle_store
                .list_versions("project_0001", "issue_0001", &entity_id)
                .expect("reloaded spec versions")
                .len(),
            2,
            "重复恢复不应追加第二个 spec version"
        );
        assert_eq!(
            lifecycle_store
                .list_artifact_versions(&session_record.id)
                .expect("reloaded artifact versions")
                .len(),
            2,
            "重复恢复不应追加第二个 artifact version"
        );
        assert_eq!(
            checkpoint_store
                .list_checkpoints(&session_record.id)
                .expect("reloaded checkpoints")
                .len(),
            1,
            "重复恢复不应追加第二个 checkpoint"
        );
    }
}

#[tokio::test]
async fn persistent_engine_recovers_outline_review_with_valid_string_references() {
    let (_tmp, checkpoint_store, lifecycle_store, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_outline_review_schema_recovery");
    let persisted_session = lifecycle_store
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("workspace sessions")
        .into_iter()
        .next()
        .expect("persisted workspace session");
    let session_id = persisted_session.id.clone();
    engine.session.session_id = session_id.clone();
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    engine
        .complete_active_node(Some("准备 Outline review".to_string()))
        .await;
    let review_node_id = engine.begin_work_item_plan_outline_review_run().await;
    let review_output = r#"审核结论：Outline 可以继续，以下均为非阻塞建议。

<ARIA_STRUCTURED_OUTPUT nonce="abc12345">
{"nonce":"abc12345","verdict":"pass","review_scope":"outline","generation_round_id":"round_0001","summary":"Outline 可以继续，但有非阻塞建议","affects_items":["outline_a","outline_b"],"findings":[{"severity":"suggestion","message":"handoff 可以更明确","evidence":"handoff_notes 较简短","impact":"不影响 Draft 生成","required_action":"补充 handoff 说明"}]}
</ARIA_STRUCTURED_OUTPUT>"#;
    let fallback = ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: "审核结论：Outline 可以继续，以下均为非阻塞建议。".to_string(),
        summary: "需要人工确认".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
        structured_output_diagnostic: Some(StructuredOutputDiagnostic {
            code: "invalid_outline_reference".to_string(),
            message: "审核引用了无效的 outline".to_string(),
            repair_attempted: false,
            repair_succeeded: false,
            raw_output_preview: Some("truncated preview".to_string()),
        }),
    };
    let review_detail = NodeDetail {
        node_id: review_node_id.clone(),
        session_id: session_id.clone(),
        node_type: TimelineNodeType::WorkItemPlanOutlineReview,
        status: TimelineNodeStatus::Completed,
        agent_role: Some(AgentRole::Reviewer),
        provider: None,
        prompt: None,
        messages: Vec::new(),
        streaming_content: review_output.to_string(),
        execution_events: Vec::new(),
        permission_events: Vec::new(),
        verdict: Some(serde_json::to_value(&fallback).expect("fallback verdict json")),
        artifact_ref: None,
        is_revision: false,
        revision_feedback: None,
        base_artifact_ref: None,
        started_at: chrono::Utc::now().to_rfc3339(),
        ended_at: Some(chrono::Utc::now().to_rfc3339()),
    };
    lifecycle_store
        .save_node_detail(&session_id, &review_node_id, &review_detail)
        .expect("persist fallback review detail");
    engine
        .update_timeline_node(
            &review_node_id,
            TimelineNodeStatus::Completed,
            Some("需要人工确认".to_string()),
        )
        .await;
    engine.latest_review_verdict = Some(fallback);
    engine
        .enter_human_confirm(Some("需要人工确认".to_string()))
        .await;

    let (tx, _rx) = mpsc::channel(8);
    let recovered = WorkspaceEngine::new_persistent(
        checkpoint_store.clone(),
        lifecycle_store.clone(),
        tx,
        WorkspaceSession::from_record(
            lifecycle_store
                .get_workspace_session(&session_id)
                .expect("reload workspace session"),
        ),
    );

    assert_eq!(recovered.session().stage, WorkspaceStage::ReviewDecision);
    assert_eq!(
        recovered.review_decision_options(),
        vec![
            "apply_optional_findings".to_string(),
            "skip_optional_findings".to_string(),
        ]
    );
    let recovered_verdict = recovered
        .latest_review_verdict
        .as_ref()
        .expect("recovered review verdict");
    assert_eq!(recovered_verdict.verdict, ReviewVerdictType::Pass);
    assert!(recovered_verdict.structured_output_diagnostic.is_none());
    assert_eq!(recovered_verdict.findings.len(), 1);
    assert_eq!(
        recovered
            .timeline_nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::ReviewDecision)
            .count(),
        1
    );
    assert!(recovered.timeline_nodes.iter().any(|node| {
        node.node_type == TimelineNodeType::HumanConfirm
            && node.status == TimelineNodeStatus::Completed
    }));
    let persisted_review_detail = lifecycle_store
        .load_node_detail(&session_id, &review_node_id)
        .expect("recovered review detail");
    let persisted_verdict: ReviewVerdict = serde_json::from_value(
        persisted_review_detail
            .verdict
            .expect("persisted recovered verdict"),
    )
    .expect("decode recovered verdict");
    assert_eq!(persisted_verdict.verdict, ReviewVerdictType::Pass);

    let (reload_tx, _reload_rx) = mpsc::channel(8);
    let reloaded = WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle_store.clone(),
        reload_tx,
        WorkspaceSession::from_record(
            lifecycle_store
                .get_workspace_session(&session_id)
                .expect("reload recovered workspace session"),
        ),
    );
    assert_eq!(reloaded.session().stage, WorkspaceStage::ReviewDecision);
    assert_eq!(
        reloaded
            .timeline_nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::ReviewDecision)
            .count(),
        1,
        "repeated reconnect must not append duplicate review decision nodes"
    );
}

#[tokio::test]
async fn outline_review_schema_recovery_rolls_back_verdict_when_timeline_commit_fails() {
    let (_tmp, _checkpoint_store, lifecycle_store, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_outline_review_schema_rollback");
    let persisted_session = lifecycle_store
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("workspace sessions")
        .into_iter()
        .next()
        .expect("persisted workspace session");
    let session_id = persisted_session.id;
    engine.session.session_id = session_id.clone();
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    engine
        .complete_active_node(Some("准备 Outline review".to_string()))
        .await;
    let review_node_id = engine.begin_work_item_plan_outline_review_run().await;
    let fallback = ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: "Outline 可以继续，但结构化格式不兼容".to_string(),
        summary: "需要人工确认".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
        structured_output_diagnostic: Some(StructuredOutputDiagnostic {
            code: "invalid_outline_reference".to_string(),
            message: "审核引用了无效的 outline".to_string(),
            repair_attempted: false,
            repair_succeeded: false,
            raw_output_preview: None,
        }),
    };
    lifecycle_store
        .save_node_detail(
            &session_id,
            &review_node_id,
            &NodeDetail {
                node_id: review_node_id.clone(),
                session_id: session_id.clone(),
                node_type: TimelineNodeType::WorkItemPlanOutlineReview,
                status: TimelineNodeStatus::Completed,
                agent_role: Some(AgentRole::Reviewer),
                provider: None,
                prompt: None,
                messages: Vec::new(),
                streaming_content: r#"Outline 可以继续，但有可选建议。
<ARIA_STRUCTURED_OUTPUT nonce="abc12345">
{"nonce":"abc12345","verdict":"pass","review_scope":"outline","generation_round_id":"round_0001","summary":"Outline 可以继续，但有可选建议","affects_items":["outline_a"],"findings":[{"severity":"suggestion","message":"handoff 可以更明确","evidence":"handoff_notes 较简短","impact":"不影响 Draft 生成","required_action":"补充说明"}]}
</ARIA_STRUCTURED_OUTPUT>"#
                    .to_string(),
                execution_events: Vec::new(),
                permission_events: Vec::new(),
                verdict: Some(
                    serde_json::to_value(&fallback).expect("serialize fallback verdict"),
                ),
                artifact_ref: None,
                is_revision: false,
                revision_feedback: None,
                base_artifact_ref: None,
                started_at: chrono::Utc::now().to_rfc3339(),
                ended_at: Some(chrono::Utc::now().to_rfc3339()),
            },
        )
        .expect("persist fallback review detail");
    engine
        .update_timeline_node(
            &review_node_id,
            TimelineNodeStatus::Completed,
            Some("需要人工确认".to_string()),
        )
        .await;
    engine.latest_review_verdict = Some(fallback);
    engine
        .enter_human_confirm(Some("需要人工确认".to_string()))
        .await;

    let timeline_path = lifecycle_store
        .workspace_timeline_root_for_session(&session_id)
        .expect("timeline root")
        .join("timeline_nodes.json");
    std::fs::remove_file(&timeline_path).expect("remove timeline file for blocker");
    std::fs::create_dir(&timeline_path).expect("block timeline commit target");

    let result = recover_work_item_plan_outline_review_schema_fallback(
        &lifecycle_store,
        &mut engine.session,
        &mut engine.timeline_nodes,
        &mut engine.active_node_id,
    );

    std::fs::remove_dir(&timeline_path).expect("remove timeline blocker");
    lifecycle_store
        .save_timeline_nodes(&session_id, &engine.timeline_nodes)
        .expect("restore original timeline");
    assert!(result
        .expect_err("timeline commit failure should abort recovery")
        .contains("commit recovered outline review timeline failed"));
    let persisted_detail = lifecycle_store
        .load_node_detail(&session_id, &review_node_id)
        .expect("review detail after rollback");
    let persisted_verdict: ReviewVerdict = serde_json::from_value(
        persisted_detail
            .verdict
            .expect("persisted fallback verdict after rollback"),
    )
    .expect("decode fallback verdict after rollback");
    assert_eq!(persisted_verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert_eq!(
        persisted_verdict
            .structured_output_diagnostic
            .as_ref()
            .map(|diagnostic| diagnostic.code.as_str()),
        Some("invalid_outline_reference")
    );
}
