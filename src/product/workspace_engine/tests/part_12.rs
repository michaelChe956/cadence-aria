#[test]
fn provider_resume_session_candidate_is_isolated_by_role_and_provider() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_role_isolation");
    session.author_provider = ProviderName::ClaudeCode;
    session.reviewer_provider = Some(ProviderName::ClaudeCode);
    session.provider_conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "author-session".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        last_node_id: Some("node-author".to_string()),
    }];
    let checkpoint_tmp = TempDir::new().unwrap();
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );

    assert_eq!(
        engine
            .provider_resume_session_candidate(
                ProviderConversationRole::Author,
                &ProviderName::ClaudeCode
            )
            .expect("typed candidate read")
            .map(|candidate| candidate.native_id),
        Some("author-session".to_string())
    );
    assert_eq!(
        engine
            .provider_resume_session_candidate(
                ProviderConversationRole::Reviewer,
                &ProviderName::ClaudeCode
            )
            .expect("typed candidate read"),
        None
    );
    assert!(
        engine
            .provider_resume_session_candidate(
                ProviderConversationRole::Author,
                &ProviderName::Codex
            )
            .expect("typed candidate read")
            .is_none()
    );
}

#[test]
fn design_artifact_gate_accepts_numbered_canonical_headings() {
    let content = r#"# Provider 依赖自检 Design Spec

## 1. 设计范围

本设计覆盖 provider 依赖自检与安装。

## 2. 设计决策

- [DEC-001] 新建 ProviderCatalog。

## 3. 公共组件

- [CMP-001] ProviderCatalog。

## 4. API 契约

- [API-001] ProviderCatalog::probe。

## 5. 数据模型

- ProviderCapability。

## 6. 风险

无。

## 7. 追踪关系

- source ids: Story Spec story_spec_0001, Issue issue_0001
- [DEC-001] -> [REQ-001]
"#;

    assert!(content_has_complete_workspace_artifact(
        content,
        &WorkspaceType::Design
    ));
}

#[test]
fn design_artifact_gate_rejects_legacy_key_decision_heading() {
    let content = r#"# Provider 依赖自检 Design Spec

## 1. 设计范围

本设计覆盖 provider 依赖自检与安装。

## 关键决策

- [DEC-001] 新建 ProviderCatalog。

## 组件 / API / 数据模型

- [CMP-001] ProviderCatalog。
"#;

    assert!(!content_has_complete_workspace_artifact(
        content,
        &WorkspaceType::Design
    ));
}

#[test]
fn review_input_does_not_resume_prior_reviewer_provider_session() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_review_no_resume");
    session.reviewer_provider = Some(ProviderName::Codex);
    session.artifact = Some(artifact_payload(
        "# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.\n",
    ));
    session.provider_conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Reviewer,
        provider: ProviderName::Codex,
        provider_session_id: "codex-review-thread-1".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        last_node_id: Some("timeline_node_003".to_string()),
    }];
    let checkpoint_tmp = TempDir::new().unwrap();
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );

    let input = engine.build_review_input().expect("review input");

    assert_eq!(input.resume_provider_session_id, None);
    assert!(input.prompt.contains("当前 Artifact"));
}

#[tokio::test]
async fn build_session_state_inlines_sanitized_work_item_plan_run_details() {
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let (tx, _) = mpsc::channel(64);
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput { project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "issue_work_item_plan_0001".to_string(),
        workspace_type: WorkspaceType::WorkItemPlan,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: Some(ProviderName::Codex),

        review_rounds: 2,
        superpowers_enabled: true, openspec_enabled: true, work_item_plan_options: None, })
        .unwrap();
    let session = WorkspaceSession::from_record(session_record);
    let session_id = session.session_id.clone();
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);
    engine.timeline_nodes.push(TimelineNode {
        node_id: "outline-node".to_string(),
        node_type: TimelineNodeType::WorkItemPlanOutlineRun,
        agent: Some(ProviderName::ClaudeCode),
        stage: WsWorkspaceStage::Running,
        round: None,
        status: TimelineNodeStatus::Completed,
        title: "生成 Work Item Plan Outline".to_string(),
        summary: None,
        started_at: "2026-05-20T14:30:00Z".to_string(),
        completed_at: Some("2026-05-20T14:35:00Z".to_string()),
        duration_ms: Some(300000),
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: None,
            review_rounds: 0,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        },
        retry: None,
    });
    let huge_prompt = "P".repeat(3000);
    let huge_output = "O".repeat(3000);
    lifecycle_store
        .save_node_detail(
            &session_id,
            "outline-node",
            &NodeDetail {
                node_id: "outline-node".to_string(),
                session_id: session_id.clone(),
                node_type: TimelineNodeType::WorkItemPlanOutlineRun,
                status: TimelineNodeStatus::Completed,
                agent_role: Some(AgentRole::Author),
                provider: Some(ProviderSnapshot {
                    name: "claude_code".to_string(),
                    model: "claude-opus-4-7".to_string(),
                }),
                prompt: Some(huge_prompt.clone()),
                messages: vec![serde_json::json!({"content": "hidden message"})],
                streaming_content: "Fake Work Item Plan streaming draft".to_string(),
                execution_events: vec![serde_json::json!({
                    "event_id": "provider_output",
                    "kind": "output",
                    "status": "completed",
                    "title": "Provider output",
                    "output": huge_output,
                })],
                permission_events: vec![PermissionEvent {
                    request_id: "perm-1".to_string(),
                    request: serde_json::json!({"tool": "shell"}),
                    response: Some(serde_json::json!({"approved": true})),
                    ts: "2026-05-20T14:31:00Z".to_string(),
                }],
                verdict: None,
                artifact_ref: None,
                is_revision: false,
                revision_feedback: None,
                base_artifact_ref: None,
                started_at: "2026-05-20T14:30:00Z".to_string(),
                ended_at: Some("2026-05-20T14:35:00Z".to_string()),
            },
        )
        .unwrap();

    let state = engine.build_session_state();
    let serialized = serde_json::to_string(&state).unwrap();
    match state {
        WsOutMessage::SessionState {
            timeline_node_details,
            timeline_node_summaries,
            ..
        } => {
            let detail = timeline_node_details
                .get("outline-node")
                .expect("outline run detail should be inlined");
            assert!(detail.prompt.is_none());
            assert!(detail.messages.is_empty());
            assert_eq!(
                detail.streaming_content,
                "Fake Work Item Plan streaming draft"
            );
            assert!(detail.permission_events.is_empty());
            assert_eq!(detail.execution_events.len(), 1);
            assert!(
                detail.execution_events[0]
                    .get("output")
                    .is_some_and(serde_json::Value::is_null)
            );
            assert!(timeline_node_summaries.is_empty());
        }
        _ => panic!("expected SessionState"),
    }
    assert!(!serialized.contains(&huge_prompt));
    assert!(!serialized.contains(&huge_output));
    assert!(!serialized.contains("hidden message"));
}

#[tokio::test]
async fn provider_resume_session_candidate_falls_back_to_author_provider_start_audit() {
    // r50(#3,claude r49 plan resume 现场):SC fresh 轮 Author native session
    // 已由 gateway spawn 审计落盘(provider_start 记录),但会话引用
    //(provider_conversations)在修订 turn 读取时缺失(内存重建/durable
    // clobber 面)——修订 turn 输入无 resume id,CLI fresh 出新 native id,
    // resume 格「原生恢复未确认」denied。修:内存+durable 会话引用皆缺时,
    // 以 spawn 时点审计的最新**有恢复资格** provider_start 兜底还原原生
    // resume(B1a/Task 4.1 起 typed 候选 {native_id, role_run_seq})。
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput { project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "issue_work_item_plan_0001".to_string(),
        workspace_type: WorkspaceType::WorkItemPlan,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: Some(ProviderName::Codex),

        review_rounds: 0,
        superpowers_enabled: false, openspec_enabled: false, work_item_plan_options: None, })
        .unwrap();
    let session_id = session_record.id.clone();
    // fresh 轮 spawn 审计:role=work_item_splitter(Plan author 的 AdapterRole),
    // native id 在场——会话引用刻意留空(现场形态)。
    use crate::cross_cutting::tool_policy_audit::{
        DurableToolPolicyEvent, ProviderStartAudit, RoleRunBoundAuditSink,
    };
    let sink = RoleRunBoundAuditSink::new(
        std::sync::Arc::new(lifecycle_store.clone()),
        &session_id,
        1,
    )
    .into_sink();
    sink.append_bound(DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
        provider: "claude-code".to_string(),
        role: "work_item_splitter".to_string(),
        provider_session_id: "audit-native-fresh-1".to_string(),
        ..ProviderStartAudit::default()
    }))
    .expect("seed provider_start audit");

    let session = WorkspaceSession::from_record(lifecycle_store.get_workspace_session(&session_id).unwrap());
    let (tx, _) = mpsc::channel(64);
    let engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);

    assert_eq!(
        engine
            .provider_resume_session_candidate(
                ProviderConversationRole::Author,
                &ProviderName::ClaudeCode
            )
            .expect("typed candidate read"),
        Some(crate::product::workspace_engine::lifecycle::NativeResumeCandidate {
            native_id: "audit-native-fresh-1".to_string(),
            role_run_seq: Some(1),
        }),
        "会话引用缺失时必须以 spawn 审计的最新 Author provider_start 兜底 resume"
    );
    // reviewer 角色的审计不得污染 Author resume。
    assert!(
        engine
            .provider_resume_session_candidate(
                ProviderConversationRole::Reviewer,
                &ProviderName::Codex
            )
            .expect("typed candidate read")
            .is_none(),
        "审计兜底只还原 Author 面(Reviewer 会话引用缺失不兜底)"
    );
}

/// B1a（Task 4.1，设计 §5.2）：audit 兜底 clean-cutover——旧 run
/// `session_terminated`（superseded_policy_drift 多代）与 durable tombstone
/// 均不得再被兜底取回；只有**有恢复资格**的当代 provider_start 才构成候选。
/// 全部吊销时不能捞更旧代、也不能当出生 fresh——typed 等待。
#[test]
fn anchor_t11_native_candidate_excludes_superseded_audit() {
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput { project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "issue_work_item_plan_0001".to_string(),
        workspace_type: WorkspaceType::WorkItemPlan,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: Some(ProviderName::Codex),

        review_rounds: 0,
        superpowers_enabled: false, openspec_enabled: false, work_item_plan_options: None, })
        .unwrap();
    let session_id = session_record.id.clone();

    use crate::cross_cutting::tool_policy_audit::{
        DurableToolPolicyEvent, ProviderStartAudit, RoleRunBoundAuditSink, SessionTerminatedAudit,
    };
    // 多代旧 run：gen0/old 已被 superseded 终止，revoked 命中 durable
    // tombstone；fresh 是唯一有恢复资格的当代 run。
    for (seq, native_id, terminated) in [
        (0u64, "audit-native-gen0", true),
        (1, "audit-native-old", true),
        (2, "audit-native-revoked", false),
        (3, "audit-native-fresh", false),
    ] {
        let sink = RoleRunBoundAuditSink::new(
            std::sync::Arc::new(lifecycle_store.clone()),
            &session_id,
            seq,
        )
        .into_sink();
        sink.append_bound(DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
            provider: "claude-code".to_string(),
            role: "work_item_splitter".to_string(),
            provider_session_id: native_id.to_string(),
            ..ProviderStartAudit::default()
        }))
        .expect("seed provider_start audit");
        if terminated {
            sink.append_bound(DurableToolPolicyEvent::SessionTerminated(
                SessionTerminatedAudit {
                    reason_code: "superseded_policy_drift".to_string(),
                },
            ))
            .expect("seed superseded termination");
        }
    }
    // durable tombstone：revoked 代已吊销（多代保留，恢复资格排除）。
    let superseded_native = |native_id: &str| {
        crate::product::models::SupersededNativeSessionRecord {
            role: ProviderConversationRole::Author,
            provider: ProviderName::ClaudeCode,
            native_session_id: native_id.to_string(),
            reason_code: "superseded_policy_drift".to_string(),
            recovery_id: "lc_resume_recovery_seed".to_string(),
            superseded_at: "2026-10-11T00:00:00Z".to_string(),
        }
    };
    let session_path = lifecycle_store
        .workspace_sessions_root("project_0001", "issue_0001")
        .join(format!("{session_id}.json"));
    let mut record = lifecycle_store.get_workspace_session(&session_id).unwrap();
    record.superseded_native_sessions = vec![superseded_native("audit-native-revoked")];
    crate::product::json_store::write_json(&session_path, &record)
        .expect("rewrite session record with tombstone");

    let session = WorkspaceSession::from_record(
        lifecycle_store.get_workspace_session(&session_id).unwrap(),
    );
    let (tx, _) = mpsc::channel(64);
    let engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);

    let candidate = engine
        .provider_resume_session_candidate(
            ProviderConversationRole::Author,
            &ProviderName::ClaudeCode,
        )
        .expect("typed candidate read must succeed")
        .expect("the non-revoked latest provider_start must stay eligible");
    assert_eq!(candidate.native_id, "audit-native-fresh");
    assert_eq!(candidate.role_run_seq, Some(3));
    assert_ne!(candidate.native_id, "audit-native-gen0");
    assert_ne!(candidate.native_id, "audit-native-old");
    assert_ne!(candidate.native_id, "audit-native-revoked");

    // 全部吊销：旧代不复活，也不折出生 fresh——typed FrozenBindingUnavailable。
    let mut exhausted = lifecycle_store.get_workspace_session(&session_id).unwrap();
    exhausted
        .superseded_native_sessions
        .push(superseded_native("audit-native-fresh"));
    crate::product::json_store::write_json(&session_path, &exhausted)
        .expect("rewrite session record with exhausted tombstones");
    let error = engine
        .provider_resume_session_candidate(
            ProviderConversationRole::Author,
            &ProviderName::ClaudeCode,
        )
        .expect_err("all superseded audit generations must not resume");
    assert!(matches!(
        error,
        crate::product::workspace_engine::lifecycle::NativeResumeCandidateError::FrozenBindingUnavailable
    ));
}
