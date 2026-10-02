// part_02 拆分（large_file_guard 1200 行上限）：planning 快照 helper 与 provider drive 结构化输出回写用例族经 include! 挂载，模块作用域与用例名不变，行为零变化。

// 退役留档（T5/REQ-RET-02）：`fake_reviewer_creates_skipped_review_node_and_enters_human_confirm` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// ---- Task 4（方案X阶段2）：AI run 后解析 structured output 回写 involved ----

fn save_planning_snapshot(app_paths: &ProductAppPaths, project_id: &str, issue_id: &str, effective_member_ids: Vec<LogicalRepositoryId>) {
    PlanningContextSnapshotStore::new(app_paths.clone())
        .save(&PlanningContextSnapshot {
            schema_version: 1,
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            membership_revision: 1,
            effective_member_ids,
            member_fingerprints: Vec::new(),
            aggregate_index_id: "agg_index_0001".to_string(),
            index_revision: 1,
            policy_digest: "policy_0001".to_string(),
            working_directory: std::path::PathBuf::new(),
            access_fingerprint: String::new(),
            invalidation: None,
            captured_at: "2026-07-01T00:00:00Z".to_string(),
        })
        .unwrap();
}

async fn drive_author_completed(
    engine: &mut WorkspaceEngine,
    full_output: String,
) {
    let node_id = create_author_run_node(engine).await;
    let (provider_event_tx, provider_event_rx) = mpsc::channel(8);
    let (provider_command_tx, _provider_command_rx) = mpsc::channel(8);
    provider_event_tx
        .send(ProviderEvent::Completed(ProviderCompletion::plain(
            full_output,
            None,
        )))
        .await
        .unwrap();
    drop(provider_event_tx);
    engine
        .drive_provider_session(ProviderSessionDriveInput {
            session: Ok(ProviderSession {
                native_session_id: None,
                events: provider_event_rx,
                commands: provider_command_tx,
            }),
            command_rx: empty_provider_commands(),
            node_id: Some(node_id),
            agent: Some(ProviderName::ClaudeCode),
            role: ProviderConversationRole::Author,
            artifact_retry: None,
            revision_resume_fallback: None,
        })
        .await;
}

#[tokio::test]
async fn provider_drive_story_run_writes_back_involved_from_structured_output() {
    let (tmp, checkpoint_store) = setup();
    let app_paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle_store = LifecycleStore::new(app_paths.clone());
    let member_a = LogicalRepositoryId(Uuid::from_u128(0xaaaa));
    let member_b = LogicalRepositoryId(Uuid::from_u128(0xbbbb));
    let effective_member_ids = vec![member_a, member_b];
    let logical_codebase_ref = Uuid::from_u128(0x0100);

    // 聚合代码库 Story（Draft、空 involved）。
    let story = lifecycle_store
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                logical_codebase_ref,
                effective_member_ids: effective_member_ids.clone(),
                involved_repository_ids: Vec::new(),
                focus_repository_id: None,
            }),
        })
        .unwrap();
    assert_eq!(story.confirmation_status, LifecycleConfirmationStatus::Draft);
    save_planning_snapshot(&app_paths, "project_0001", "issue_0001", effective_member_ids.clone());

    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput { project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: story.id.clone(),
        workspace_type: WorkspaceType::Story,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: Some(ProviderName::Codex),

        review_rounds: 2,
        superpowers_enabled: true, openspec_enabled: true, work_item_plan_options: None, })
        .unwrap();
    let session = WorkspaceSession::from_record(session_record);
    let (tx, _rx) = mpsc::channel(64);
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);

    // AI 产出含 structured output 的完整 Story artifact。
    let structured = format!(
        "<ARIA_STRUCTURED_OUTPUT nonce=\"abcd1234\">{{\"nonce\":\"abcd1234\",\"involved_repository_ids\":[\"{a}\",\"{b}\"],\"focus_repository_id\":\"{b}\"}}</ARIA_STRUCTURED_OUTPUT>",
        a = member_a.0,
        b = member_b.0,
    );
    let artifact_markdown = format!(
        "{}\n{structured}",
        complete_story_artifact("生成候选草稿。", "候选草稿可进入人工确认。")
    );
    drive_author_completed(&mut engine, format!("```artifact\n{artifact_markdown}\n```")).await;

    // 验证 record.involved_repository_ids / focus_repository_id 被回写。
    let updated = lifecycle_store
        .load_existing_spec("project_0001", "issue_0001", &story.id)
        .unwrap();
    match updated {
        ExistingSpecRecord::Story { record, .. } => {
            assert_eq!(
                record.involved_repository_ids, effective_member_ids,
                "AI 产出的 involved 应回写到 Story record"
            );
            assert_eq!(record.focus_repository_id, Some(member_b));
        }
        _ => panic!("expected story spec"),
    }
}

/// 缺陷（2026-10-02 E2E story/design 首跑标签形态）：聚合回写要求
/// ARIA_STRUCTURED_OUTPUT 标签，真实 AI 首跑常回显模板占位值或省 nonce 属性
///（E2E 靠反馈样板才收敛）。author prompt 必须自带「完整标签样板（含 nonce
/// 属性）+ 占位 nonce 示例 + 实际 nonce 模板 + 替换占位值说明」。
#[test]
fn aggregate_author_prompt_pins_complete_nonce_tag_template() {
    for workspace_type in [WorkspaceType::Story, WorkspaceType::Design] {
        let (tmp, checkpoint_store) = setup();
        let app_paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let lifecycle_store = LifecycleStore::new(app_paths.clone());
        let member_a = LogicalRepositoryId(Uuid::from_u128(0xaaaa));

        match workspace_type {
            WorkspaceType::Story => {
                lifecycle_store
                    .create_story_spec(CreateStorySpecInput {
                        project_id: "project_0001".to_string(),
                        issue_id: "issue_0001".to_string(),
                        repository_id: "repository_0001".to_string(),
                        title: "Story".to_string(),
                        aggregate_codebase: Some(AggregateStorySpecScope {
                            logical_codebase_ref: Uuid::from_u128(0x0100),
                            effective_member_ids: vec![member_a],
                            involved_repository_ids: Vec::new(),
                            focus_repository_id: None,
                        }),
                    })
                    .unwrap();
            }
            WorkspaceType::Design => {
                lifecycle_store
                    .create_design_spec(CreateDesignSpecInput {
                        project_id: "project_0001".to_string(),
                        issue_id: "issue_0001".to_string(),
                        story_spec_ids: Vec::new(),
                        title: "Design".to_string(),
                        aggregate_codebase: Some(AggregateDesignSpecScope {
                            logical_codebase_ref: Uuid::from_u128(0x0100),
                            effective_member_ids: vec![member_a],
                            involved_repository_ids: Vec::new(),
                            change_order: Vec::new(),
                        }),
                    })
                    .unwrap();
            }
            _ => unreachable!("仅聚合 Story/Design"),
        }
        save_planning_snapshot(&app_paths, "project_0001", "issue_0001", vec![member_a]);

        let session_record = lifecycle_store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: format!("spec_{workspace_type:?}"),
                workspace_type: workspace_type.clone(),
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::Codex),
                review_rounds: 2,
                superpowers_enabled: true,
                openspec_enabled: true,
                work_item_plan_options: None,
            })
            .unwrap();
        let mut session = WorkspaceSession::from_record(session_record);
        // 聚合视野上下文（生产由 ensure_workspace_context_message 在 Logical
        // routing 下注入同 marker 系统消息）。
        session.messages.push(SessionMessage {
            id: "msg_aggregate_scope".to_string(),
            role: "system".to_string(),
            content: "## 聚合代码库成员清单\n- member-a".to_string(),
            checkpoint_id: None,
            created_at: "2026-10-02T00:00:00Z".to_string(),
        });
        let (tx, _rx) = mpsc::channel(64);
        let engine =
            WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);

        let input = engine
            .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
            .expect("aggregate author streaming input");
        let contract = input
            .structured_output_contract
            .expect("aggregate Story/Design 必须设置 structured_output_contract");
        let prompt = input.prompt;

        // 完整示例：nonce 属性样板 + 占位值不可照抄说明。
        assert!(
            prompt.contains("<ARIA_STRUCTURED_OUTPUT nonce=\"EXAMPLE_NONCE\">"),
            "{workspace_type:?} prompt 必须含完整示例标签（含 nonce 属性）：{prompt}"
        );
        assert!(
            prompt.contains("EXAMPLE_NONCE 是占位值，绝不可照抄"),
            "{workspace_type:?} prompt 必须说明占位 nonce 不可照抄：{prompt}"
        );
        // 实际输出模板：本请求真实 nonce（标签属性与 JSON 顶层一致）。
        let real_tag = format!("<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">", contract.nonce);
        assert!(
            prompt.contains(&real_tag),
            "{workspace_type:?} prompt 必须含本请求 nonce 的实际输出模板：{prompt}"
        );
        assert!(
            prompt.contains(&format!("\"nonce\":\"{}\"", contract.nonce)),
            "{workspace_type:?} prompt 模板 JSON 顶层必须携带本请求 nonce：{prompt}"
        );
        // 替换占位值说明。
        assert!(
            prompt.contains("把占位值 EXAMPLE_NONCE 替换为本请求 nonce"),
            "{workspace_type:?} prompt 必须给出占位值替换指引：{prompt}"
        );
    }
}

#[tokio::test]
async fn provider_drive_aggregate_story_writes_back_tag_outside_artifact_fence() {
    // 缺陷 #4（2026-10-02 E2E）回归：aggregate_author_output_contract 要求 sentinel
    // 标签输出在 artifact 围栏**之外**，真实 provider 输出即此形态。回写解析源必须
    // 是 provider 全量输出——围栏内产物正文不含标签，修复前恒 MissingStructuredOutput、
    // involved 恒空、终确认恒 involved_repositories_undetermined。
    let (tmp, checkpoint_store) = setup();
    let app_paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle_store = LifecycleStore::new(app_paths.clone());
    let member_a = LogicalRepositoryId(Uuid::from_u128(0xaaaa));
    let member_b = LogicalRepositoryId(Uuid::from_u128(0xbbbb));
    let effective_member_ids = vec![member_a, member_b];

    let story = lifecycle_store
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                logical_codebase_ref: Uuid::from_u128(0x0100),
                effective_member_ids: effective_member_ids.clone(),
                involved_repository_ids: Vec::new(),
                focus_repository_id: None,
            }),
        })
        .unwrap();
    save_planning_snapshot(&app_paths, "project_0001", "issue_0001", effective_member_ids.clone());

    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput { project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: story.id.clone(),
        workspace_type: WorkspaceType::Story,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: Some(ProviderName::Codex),

        review_rounds: 2,
        superpowers_enabled: true, openspec_enabled: true, work_item_plan_options: None, })
        .unwrap();
    let session = WorkspaceSession::from_record(session_record);
    let (tx, _rx) = mpsc::channel(64);
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);

    // 真实形态：artifact 在围栏内，sentinel 标签紧跟围栏之后（之外）。
    let structured = format!(
        "<ARIA_STRUCTURED_OUTPUT nonce=\"abcd1234\">{{\"nonce\":\"abcd1234\",\"involved_repository_ids\":[\"{a}\",\"{b}\"],\"focus_repository_id\":\"{b}\"}}</ARIA_STRUCTURED_OUTPUT>",
        a = member_a.0,
        b = member_b.0,
    );
    let artifact_markdown =
        complete_story_artifact("生成候选草稿。", "候选草稿可进入人工确认。");
    drive_author_completed(
        &mut engine,
        format!("```artifact\n{artifact_markdown}\n```\n\n{structured}"),
    )
    .await;

    let updated = lifecycle_store
        .load_existing_spec("project_0001", "issue_0001", &story.id)
        .unwrap();
    match updated {
        ExistingSpecRecord::Story { record, .. } => {
            assert_eq!(
                record.involved_repository_ids, effective_member_ids,
                "围栏外 sentinel 标签也应回写 involved（缺陷 #4）"
            );
            assert_eq!(record.focus_repository_id, Some(member_b));
        }
        _ => panic!("expected story spec"),
    }
}

#[tokio::test]
async fn provider_drive_design_run_writes_back_involved_and_change_order_from_structured_output() {
    let (tmp, checkpoint_store) = setup();
    let app_paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle_store = LifecycleStore::new(app_paths.clone());
    let member_a = LogicalRepositoryId(Uuid::from_u128(0xaaaa));
    let member_b = LogicalRepositoryId(Uuid::from_u128(0xbbbb));
    let effective_member_ids = vec![member_a, member_b];
    let logical_codebase_ref = Uuid::from_u128(0x0100);

    let story = lifecycle_store
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Story".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    let design = lifecycle_store
        .create_design_spec(CreateDesignSpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "Design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                logical_codebase_ref,
                effective_member_ids: effective_member_ids.clone(),
                involved_repository_ids: Vec::new(),
                change_order: Vec::new(),
            }),
        })
        .unwrap();
    assert_eq!(design.confirmation_status, LifecycleConfirmationStatus::Draft);
    save_planning_snapshot(&app_paths, "project_0001", "issue_0001", effective_member_ids.clone());

    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput { project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: design.id.clone(),
        workspace_type: WorkspaceType::Design,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: Some(ProviderName::Codex),

        review_rounds: 2,
        superpowers_enabled: true, openspec_enabled: true, work_item_plan_options: None, })
        .unwrap();
    let session = WorkspaceSession::from_record(session_record);
    let (tx, _rx) = mpsc::channel(64);
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);

    let structured = format!(
        "<ARIA_STRUCTURED_OUTPUT nonce=\"abcd1234\">{{\"nonce\":\"abcd1234\",\"involved_repository_ids\":[\"{a}\",\"{b}\"],\"change_order\":[\"{a}\",\"{b}\"]}}</ARIA_STRUCTURED_OUTPUT>",
        a = member_a.0,
        b = member_b.0,
    );
    let artifact_markdown = format!(
        "{}\n{structured}",
        complete_design_artifact("保留设计边界。", "公开接口保持稳定。")
    );
    drive_author_completed(&mut engine, format!("```artifact\n{artifact_markdown}\n```")).await;

    let updated = lifecycle_store
        .load_existing_spec("project_0001", "issue_0001", &design.id)
        .unwrap();
    match updated {
        ExistingSpecRecord::Design { record, .. } => {
            assert_eq!(
                record.involved_repository_ids, effective_member_ids,
                "AI 产出的 involved 应回写到 Design record"
            );
            assert_eq!(
                record.change_order, effective_member_ids,
                "AI 产出的 change_order 应回写到 Design record"
            );
        }
        _ => panic!("expected design spec"),
    }
}

#[tokio::test]
async fn provider_drive_single_repo_story_run_does_not_write_back_aggregate() {
    // 传统单仓 Story（aggregate_codebase=None）：AI 产出无 structured output，
    // 回写应跳过且不产生诊断/失败，既有 append_version 行为不变。
    let (tmp, checkpoint_store) = setup();
    let app_paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle_store = LifecycleStore::new(app_paths.clone());
    let story = lifecycle_store
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Story".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();

    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput { project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: story.id.clone(),
        workspace_type: WorkspaceType::Story,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: Some(ProviderName::Codex),

        review_rounds: 2,
        superpowers_enabled: true, openspec_enabled: true, work_item_plan_options: None, })
        .unwrap();
    let session = WorkspaceSession::from_record(session_record);
    let (tx, _rx) = mpsc::channel(64);
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);

    // 单仓：无 structured output，直接产出完整 Story artifact。
    drive_author_completed(&mut engine, complete_story_artifact("生成候选。", "可验收。")).await;

    let updated = lifecycle_store
        .load_existing_spec("project_0001", "issue_0001", &story.id)
        .unwrap();
    match updated {
        ExistingSpecRecord::Story { record, .. } => {
            assert!(
                record.involved_repository_ids.is_empty(),
                "单仓 Story 不应回写 involved"
            );
            assert_eq!(record.focus_repository_id, None);
            // 既有 append_version 行为不变：已追加一个版本。
            assert_eq!(record.current_version, Some(1));
        }
        _ => panic!("expected story spec"),
    }
    // 无诊断 system 消息。
    let system_messages = engine
        .session()
        .messages
        .iter()
        .filter(|message| message.role == "system")
        .count();
    assert_eq!(system_messages, 0, "单仓路径不应产生聚合回写诊断");
}

#[tokio::test]
async fn provider_drive_aggregate_story_missing_structured_output_records_diagnostic() {
    // 聚合代码库 Story 但 AI 未产出 structured output（缺 tag）：
    // 约束4——不回写 involved，且不静默吞，产生可见 system 诊断消息。
    let (tmp, checkpoint_store) = setup();
    let app_paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle_store = LifecycleStore::new(app_paths.clone());
    let member_a = LogicalRepositoryId(Uuid::from_u128(0xaaaa));
    let member_b = LogicalRepositoryId(Uuid::from_u128(0xbbbb));
    let effective_member_ids = vec![member_a, member_b];
    let story = lifecycle_store
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                logical_codebase_ref: Uuid::from_u128(0x0100),
                effective_member_ids: effective_member_ids.clone(),
                involved_repository_ids: Vec::new(),
                focus_repository_id: None,
            }),
        })
        .unwrap();
    save_planning_snapshot(&app_paths, "project_0001", "issue_0001", effective_member_ids.clone());

    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput { project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: story.id.clone(),
        workspace_type: WorkspaceType::Story,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: Some(ProviderName::Codex),

        review_rounds: 2,
        superpowers_enabled: true, openspec_enabled: true, work_item_plan_options: None, })
        .unwrap();
    let session = WorkspaceSession::from_record(session_record);
    let (tx, _rx) = mpsc::channel(64);
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);

    // 完整 Story artifact 但不含 <ARIA_STRUCTURED_OUTPUT>。
    drive_author_completed(&mut engine, complete_story_artifact("生成候选。", "可验收。")).await;

    let updated = lifecycle_store
        .load_existing_spec("project_0001", "issue_0001", &story.id)
        .unwrap();
    match updated {
        ExistingSpecRecord::Story { record, .. } => {
            assert!(
                record.involved_repository_ids.is_empty(),
                "AI 未产出 involved 时不应回写"
            );
        }
        _ => panic!("expected story spec"),
    }
    // 约束4：产生可见 system 诊断消息，不静默吞。
    let diagnostics = engine
        .session()
        .messages
        .iter()
        .filter(|message| message.role == "system")
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1, "缺 tag 应记录一条可见诊断");
    assert!(
        diagnostics[0].content.contains("involved"),
        "诊断应说明 AI 未产出 involved：{}",
        diagnostics[0].content
    );
}

#[test]
fn design_artifact_constraint_negative_matrix_reports_expected_finding() {
    #[derive(Debug)]
    enum Violation { Heading(&'static str), Id(&'static str), SourceId, ForbiddenHeading(&'static str), ForbiddenToken(&'static str, &'static str) }

    let spec = artifact_constraint_spec_for(&WorkspaceType::Design);
    assert_eq!(
        (spec.required_headings.len(), spec.required_id_patterns.len(), spec.required_tokens.len(), spec.forbidden_headings.len(), spec.forbidden_tokens.len()),
        (7, 3, 1, 4, 2),
    );

    let baseline = complete_design_artifact("保留清晰的组件边界。", "公开接口保持稳定。")
        .replacen("- [DEC-001] -> [REQ-001]", "- 设计决策映射至上游需求。", 1);
    let baseline_report = validate_workspace_artifact_constraints(&baseline, &WorkspaceType::Design);
    assert!(baseline_report.passed, "Design 合法基准必须通过: {baseline_report:?}");

    let cases = [
        Violation::Heading("设计范围"),
        Violation::Heading("设计决策"),
        Violation::Heading("公共组件"),
        Violation::Heading("API 契约"),
        Violation::Heading("数据模型"),
        Violation::Heading("风险"),
        Violation::Heading("追踪关系"),
        Violation::Id("DEC"),
        Violation::Id("CMP"),
        Violation::Id("API"),
        Violation::SourceId,
        Violation::ForbiddenHeading("Work Item Plan"),
        Violation::ForbiddenHeading("任务拆分"),
        Violation::ForbiddenHeading("开发任务"),
        Violation::ForbiddenHeading("执行 checklist"),
        Violation::ForbiddenToken("[TASK-*]", "[TASK-001]"),
        Violation::ForbiddenToken("WI-*", "WI-001"),
    ];

    for violation in cases {
        let (content, field, expected) = match violation {
            Violation::Heading(label) => (
                baseline.replacen(&format!("## {label}"), label, 1),
                "missing_required_headings",
                label.to_string(),
            ),
            Violation::Id(prefix) => {
                let id = format!("[{prefix}-001]");
                (
                    baseline.replace(&id, &format!("{prefix}-001")),
                    "missing_required_ids",
                    format!("[{prefix}-*]"),
                )
            }
            Violation::SourceId => (
                baseline.replacen(
                    "source ids: Story Spec story_spec_0001, Issue issue_0001。",
                    "上游来源已关联。",
                    1,
                ),
                "missing_required_ids",
                "source id".to_string(),
            ),
            Violation::ForbiddenHeading(heading) => (
                format!("{baseline}\n## {heading}\n不应出现在 Design artifact 中。\n"),
                "forbidden_headings",
                heading.to_string(),
            ),
            Violation::ForbiddenToken(label, token) => (
                baseline.replacen(
                    "## 数据模型\n",
                    &format!("## 数据模型\n- {token}\n"),
                    1,
                ),
                "forbidden_tokens",
                format!("{label}: {token}"),
            ),
        };
        let report = validate_workspace_artifact_constraints(&content, &WorkspaceType::Design);
        let findings = match field {
            "missing_required_headings" => &report.missing_required_headings,
            "missing_required_ids" => &report.missing_required_ids,
            "forbidden_headings" => &report.forbidden_headings,
            "forbidden_tokens" => &report.forbidden_tokens,
            _ => unreachable!("unknown finding field: {field}"),
        };

        assert!(!report.passed, "{violation:?}: {report:?}");
        assert_eq!(findings.as_slice(), &[expected], "{violation:?}: {report:?}");
        let finding_count = report.missing_required_headings.len()
            + report.missing_required_ids.len()
            + report.forbidden_headings.len()
            + report.forbidden_tokens.len();
        assert_eq!(finding_count, 1, "{violation:?}: {report:?}");
    }
}
