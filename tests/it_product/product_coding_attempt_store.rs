use std::path::PathBuf;

use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{
    CodingAttemptStore, CreateChoiceGateInput, CreateCodingAttemptInput,
};
use cadence_aria::product::coding_models::{
    AnalystDecisionNextStage, AnalystDecisionRecord, AnalystDecisionVerdict,
    AnalystReworkInstructions, CodeReviewReport, CodingAgentRole, CodingAttemptStatus,
    CodingChatEntry, CodingChoiceGateStatus, CodingChoiceOption, CodingContextNote,
    CodingEntryType, CodingExecutionStage, CodingProviderRole, CodingReworkInstruction,
    CodingRolePermissionModes, CodingRoleProviderConfigSnapshot, CodingRoleRunEventType,
    CodingRoleRunStatus, CodingRoleRunTrigger, CodingStageGateStatus, CodingTimelineNode,
    CodingTimelineNodeStatus, FindingSeverity, InternalPrReview, PushStatus, RemoteKind,
    ReviewFinding, ReviewRequest, ReviewRequestKind, ReviewVerdict, TestCommand, TestCommandStatus,
    TestingOverallStatus, TestingReport,
};
use cadence_aria::product::models::{
    ProviderConversationRef, ProviderConversationRole, ProviderName,
};
use cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot;
use tempfile::tempdir;

#[test]
fn create_attempt_assigns_attempt_number_and_blocks_active_attempts() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let first = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create first attempt");
    assert_eq!(first.id, "coding_attempt_0001");
    assert_eq!(first.attempt_no, 1);
    assert_eq!(first.status, CodingAttemptStatus::Created);
    assert_eq!(first.stage, CodingExecutionStage::PrepareContext);

    let active = store
        .get_active_attempt("project_0001", "issue_0001", "work_item_0001")
        .expect("active lookup")
        .expect("active attempt");
    assert_eq!(active.id, first.id);

    let duplicate = store.create_attempt(create_input("work_item_0001"));
    assert!(
        duplicate.is_err(),
        "active created attempt should block duplicates"
    );

    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &first.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &first.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    assert!(
        store
            .create_attempt(create_input("work_item_0001"))
            .is_err(),
        "blocked attempt is still resumable and active"
    );

    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &first.id,
            CodingAttemptStatus::Aborted,
        )
        .expect("aborted");
    let second = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create second attempt after terminal status");
    assert_eq!(second.id, "coding_attempt_0002");
    assert_eq!(second.attempt_no, 2);
}

#[test]
fn coding_attempt_provider_conversations_default_for_legacy_json() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let attempt_path = paths
        .root()
        .join("projects/project_0001/issues/issue_0001/coding-attempts")
        .join(format!("{}.json", attempt.id));
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&attempt_path).unwrap()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("provider_conversations");
    std::fs::write(&attempt_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    let reloaded = store
        .get_attempt_by_id(&attempt.id)
        .expect("reload legacy coding attempt");
    assert!(reloaded.provider_conversations.is_empty());
}

#[test]
fn updates_coding_attempt_provider_conversations() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Coder,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "coder-session-1".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        last_node_id: Some("coding-node-1".to_string()),
    }];

    let updated = store
        .replace_attempt_provider_conversations(&attempt.id, conversations.clone())
        .expect("persist coding provider conversations");

    assert_eq!(updated.provider_conversations, conversations);
    let reloaded = store
        .get_attempt_by_id(&attempt.id)
        .expect("reload attempt");
    assert_eq!(reloaded.provider_conversations, conversations);
}

#[test]
fn store_persists_reports_reviews_and_timeline_for_snapshot_recovery() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let testing = sample_testing_report(&attempt.id);
    let code_review = sample_code_review_report(&attempt.id);
    let review_request = sample_review_request(&attempt.id);
    let internal_review = sample_internal_review(&attempt.id, &review_request.id);
    let node = sample_node(&attempt.id);

    store
        .save_testing_report(&testing)
        .expect("save testing report");
    store
        .save_code_review_report(&code_review)
        .expect("save code review");
    store
        .save_review_request(&review_request)
        .expect("save review request");
    store
        .save_internal_pr_review(&internal_review)
        .expect("save internal review");
    store.save_timeline_node(node.clone()).expect("save node");

    assert_eq!(
        store
            .get_testing_report(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &testing.id
            )
            .expect("load testing"),
        testing
    );
    assert_eq!(
        store
            .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("load code reviews"),
        vec![code_review]
    );
    assert_eq!(
        store
            .get_review_request(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &review_request.id,
            )
            .expect("load review request"),
        review_request
    );
    assert_eq!(
        store
            .get_internal_pr_review(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &internal_review.id,
            )
            .expect("load internal review"),
        internal_review
    );

    store
        .update_timeline_node_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &node.id,
            CodingTimelineNodeStatus::Completed,
            Some("完成".to_string()),
            Some("2026-05-23T00:02:00Z".to_string()),
        )
        .expect("update node");
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("load nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Completed);
    assert_eq!(nodes[0].summary.as_deref(), Some("完成"));
}

#[test]
fn store_persists_context_notes_in_attempt_scope() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let note = store
        .create_context_note(&attempt.id, "请优先使用 unittest".to_string())
        .expect("create context note");

    assert_eq!(
        note,
        CodingContextNote {
            id: "coding_context_note_0001".to_string(),
            attempt_id: attempt.id.clone(),
            content: "请优先使用 unittest".to_string(),
            created_at: note.created_at.clone(),
            consumed_by_rework_round: None,
        }
    );
    assert_eq!(
        store
            .list_context_notes("project_0001", "issue_0001", &attempt.id)
            .expect("list context notes"),
        vec![note]
    );
}

#[test]
fn store_lists_chat_entries_by_created_at_not_filename() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let earlier = CodingChatEntry {
        id: "coding_node_0019_analyst_verdict".to_string(),
        attempt_id: attempt.id.clone(),
        node_id: Some("coding_node_0019".to_string()),
        role: CodingAgentRole::Author,
        entry_type: CodingEntryType::UserMessage,
        content: Some("Analyst human gate".to_string()),
        metadata: None,
        created_at: "2026-06-14T15:02:43Z".to_string(),
    };
    let later_context_note = CodingChatEntry {
        id: "coding_chat_entry_0003".to_string(),
        attempt_id: attempt.id.clone(),
        node_id: Some("coding_node_0019".to_string()),
        role: CodingAgentRole::Author,
        entry_type: CodingEntryType::UserMessage,
        content: Some("请重试 Analyst，并严格只返回系统支持的 JSON schema。".to_string()),
        metadata: Some(serde_json::json!({
            "context_note_id": "coding_context_note_0003",
        })),
        created_at: "2026-06-14T15:48:40Z".to_string(),
    };
    let latest = CodingChatEntry {
        id: "coding_node_0020_analyst_verdict".to_string(),
        attempt_id: attempt.id.clone(),
        node_id: Some("coding_node_0020".to_string()),
        role: CodingAgentRole::Author,
        entry_type: CodingEntryType::UserMessage,
        content: Some("Analyst retry".to_string()),
        metadata: None,
        created_at: "2026-06-14T16:00:00Z".to_string(),
    };

    store
        .save_chat_entry(&later_context_note)
        .expect("save later context note");
    store.save_chat_entry(&latest).expect("save latest");
    store.save_chat_entry(&earlier).expect("save earlier");

    let ids = store
        .list_chat_entries("project_0001", "issue_0001", &attempt.id)
        .expect("list chat entries")
        .into_iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();

    assert_eq!(
        ids,
        vec![
            "coding_node_0019_analyst_verdict",
            "coding_chat_entry_0003",
            "coding_node_0020_analyst_verdict",
        ]
    );
}

#[test]
fn store_lists_unconsumed_context_notes_and_marks_rework_round() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let first = store
        .create_context_note(&attempt.id, "第一次补充".to_string())
        .expect("first context note");
    let second = store
        .create_context_note(&attempt.id, "第二次补充".to_string())
        .expect("second context note");

    store
        .mark_context_notes_consumed(
            "project_0001",
            "issue_0001",
            &attempt.id,
            std::slice::from_ref(&first.id),
            1,
        )
        .expect("mark first consumed");

    let unconsumed = store
        .list_unconsumed_context_notes("project_0001", "issue_0001", &attempt.id)
        .expect("list unconsumed");
    assert_eq!(unconsumed, vec![second.clone()]);

    store
        .mark_context_notes_consumed(
            "project_0001",
            "issue_0001",
            &attempt.id,
            std::slice::from_ref(&second.id),
            2,
        )
        .expect("mark second consumed");

    let notes = store
        .list_context_notes("project_0001", "issue_0001", &attempt.id)
        .expect("list notes");
    assert_eq!(notes[0].consumed_by_rework_round, Some(1));
    assert_eq!(notes[1].consumed_by_rework_round, Some(2));
    assert!(
        store
            .list_unconsumed_context_notes("project_0001", "issue_0001", &attempt.id)
            .expect("list unconsumed after all consumed")
            .is_empty()
    );
}

#[test]
fn saves_reads_and_consumes_latest_coding_rework_instruction() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let first = CodingReworkInstruction {
        id: "coding_rework_instruction_0001".to_string(),
        attempt_id: attempt.id.clone(),
        source_stage: CodingExecutionStage::Testing,
        rework_round: 1,
        summary: "测试失败".to_string(),
        fix_hints: vec!["修复 failing test".to_string()],
        questions: Vec::new(),
        created_at: "2026-05-29T00:00:00Z".to_string(),
        consumed_by_node_id: None,
        consumed_at: None,
    };
    let second = CodingReworkInstruction {
        id: "coding_rework_instruction_0002".to_string(),
        attempt_id: attempt.id.clone(),
        source_stage: CodingExecutionStage::CodeReview,
        rework_round: 2,
        summary: "移除运行产物".to_string(),
        fix_hints: vec!["不要提交 __pycache__".to_string()],
        questions: vec!["确认 diff 只包含业务文件".to_string()],
        created_at: "2026-05-29T00:01:00Z".to_string(),
        consumed_by_node_id: None,
        consumed_at: None,
    };

    store
        .save_rework_instruction(&first)
        .expect("save first instruction");
    assert_eq!(
        store
            .latest_unconsumed_rework_instruction("project_0001", "issue_0001", &attempt.id)
            .expect("latest first"),
        Some(first.clone())
    );
    store
        .mark_rework_instruction_consumed(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &first.id,
            "coding_node_0002",
        )
        .expect("consume first instruction");
    assert_eq!(
        store
            .latest_unconsumed_rework_instruction("project_0001", "issue_0001", &attempt.id)
            .expect("latest after first consume"),
        None
    );
    store
        .save_rework_instruction(&second)
        .expect("save second instruction");

    assert_eq!(
        store
            .latest_unconsumed_rework_instruction("project_0001", "issue_0001", &attempt.id)
            .expect("latest unconsumed"),
        Some(second.clone())
    );

    let consumed = store
        .mark_rework_instruction_consumed(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &second.id,
            "coding_node_0003",
        )
        .expect("consume instruction");

    assert_eq!(
        consumed.consumed_by_node_id.as_deref(),
        Some("coding_node_0003")
    );
    assert!(consumed.consumed_at.is_some());
    assert_eq!(
        store
            .latest_unconsumed_rework_instruction("project_0001", "issue_0001", &attempt.id)
            .expect("latest after consume"),
        None
    );
}

#[test]
fn saves_reads_and_lists_latest_analyst_decision() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let first = AnalystDecisionRecord {
        id: "analyst_decision_0001".to_string(),
        attempt_id: attempt.id.clone(),
        source_stage: CodingExecutionStage::Testing,
        rework_round: 1,
        verdict: AnalystDecisionVerdict::NeedsFix,
        next_stage: AnalystDecisionNextStage::Coding,
        reason: "测试失败，需要返修".to_string(),
        evidence_refs: vec!["testing_report_0001.json".to_string()],
        raw_provider_output_refs: Vec::new(),
        rework_instructions: Some(AnalystReworkInstructions {
            summary: "修复 failing test".to_string(),
            required_changes: vec!["补充边界输入处理".to_string()],
            verification_expectations: vec!["cargo test --locked --test it_product".to_string()],
        }),
        human_gate: None,
        created_at: "2026-06-12T00:00:00Z".to_string(),
        parse_error: None,
        role_run_id: None,
        run_no: None,
    };
    let second = AnalystDecisionRecord {
        id: "analyst_decision_0002".to_string(),
        attempt_id: attempt.id.clone(),
        source_stage: CodingExecutionStage::CodeReview,
        rework_round: 2,
        verdict: AnalystDecisionVerdict::Proceed,
        next_stage: AnalystDecisionNextStage::ReviewRequest,
        reason: "审查通过，可以创建 review request".to_string(),
        evidence_refs: vec!["code_review_0001.json".to_string()],
        raw_provider_output_refs: vec!["provider-raw/code_review/code_review_0001.txt".to_string()],
        rework_instructions: None,
        human_gate: None,
        created_at: "2026-06-12T00:01:00Z".to_string(),
        parse_error: None,
        role_run_id: None,
        run_no: None,
    };

    store
        .save_analyst_decision(&first)
        .expect("save first decision");
    store
        .save_analyst_decision(&second)
        .expect("save second decision");

    let decisions = store
        .list_analyst_decisions("project_0001", "issue_0001", &attempt.id)
        .expect("list decisions");
    assert_eq!(decisions, vec![first.clone(), second.clone()]);
    assert_eq!(
        store
            .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
            .expect("latest decision"),
        Some(second)
    );
}

#[test]
fn saves_reads_and_supersedes_coding_role_runs() {
    let root = tempfile::tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let first = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("first role run");
    assert_eq!(first.id, "coding_role_run_0001");
    assert_eq!(first.run_no, 1);
    assert_eq!(first.status, CodingRoleRunStatus::Running);
    assert_eq!(first.role, CodingProviderRole::Tester);

    let second = store
        .supersede_latest_role_run_and_create(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::RetryTestPlan,
            Some("coding_node_0004".to_string()),
            Some("plan_tests_timeout".to_string()),
        )
        .expect("second role run");

    assert_eq!(second.id, "coding_role_run_0002");
    assert_eq!(second.run_no, 2);
    assert_eq!(
        second.supersedes_run_id.as_deref(),
        Some("coding_role_run_0001")
    );

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Superseded);
    assert_eq!(
        runs[0].superseded_by_run_id.as_deref(),
        Some("coding_role_run_0002")
    );
    assert_eq!(runs[1].status, CodingRoleRunStatus::Running);

    let latest = store
        .latest_role_run(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
        )
        .expect("latest")
        .expect("latest role run");
    assert_eq!(latest.id, "coding_role_run_0002");
}

#[test]
fn store_persists_role_provider_config_snapshot_in_attempt_scope() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let initial = store
        .get_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id)
        .expect("initial role provider snapshot");
    assert_eq!(initial.coder, ProviderName::Fake);
    assert_eq!(initial.tester, ProviderName::Fake);
    assert_eq!(initial.code_reviewer, ProviderName::Fake);

    let updated = CodingRoleProviderConfigSnapshot {
        coder: ProviderName::Fake,
        tester: ProviderName::Codex,
        analyst: ProviderName::Fake,
        code_reviewer: ProviderName::Codex,
        internal_reviewer: ProviderName::Fake,
        review_rounds: 1,
        permission_modes: CodingRolePermissionModes::default(),
    };
    store
        .update_role_provider_config_snapshot(
            "project_0001",
            "issue_0001",
            &attempt.id,
            updated.clone(),
        )
        .expect("update role provider snapshot");

    assert_eq!(
        store
            .get_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id)
            .expect("updated role provider snapshot"),
        updated
    );
}

#[test]
fn store_persists_and_resolves_stage_gates_in_attempt_scope() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let provider_snapshot = CodingRoleProviderConfigSnapshot::from(ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: Some(ProviderName::Fake),
        review_rounds: 1,
    });
    let gate = store
        .create_stage_gate(
            &attempt.id,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            "2026-05-28T00:00:05Z".to_string(),
            provider_snapshot.clone(),
        )
        .expect("create stage gate");

    assert_eq!(gate.gate_id, "coding_stage_gate_0001");
    assert_eq!(gate.attempt_id, attempt.id);
    assert_eq!(gate.stage, CodingExecutionStage::Testing);
    assert_eq!(gate.role, CodingProviderRole::Tester);
    assert_eq!(gate.provider_snapshot, provider_snapshot);
    assert_eq!(gate.status, CodingStageGateStatus::Open);
    assert_eq!(
        store
            .list_open_stage_gates("project_0001", "issue_0001", &attempt.id)
            .expect("open stage gates")
            .len(),
        1
    );

    let confirmed = store
        .update_stage_gate_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &gate.gate_id,
            CodingStageGateStatus::Confirmed,
        )
        .expect("confirm stage gate");

    assert_eq!(confirmed.status, CodingStageGateStatus::Confirmed);
    assert!(
        store
            .list_open_stage_gates("project_0001", "issue_0001", &attempt.id)
            .expect("open stage gates")
            .is_empty()
    );
}

#[test]
fn store_persists_and_resolves_choice_gates_in_attempt_scope() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let gate = store
        .create_choice_gate(CreateChoiceGateInput {
            attempt_id: attempt.id.clone(),
            choice_id: "choice_0001".to_string(),
            stage: CodingExecutionStage::Coding,
            node_id: Some("coding_node_0001".to_string()),
            role: CodingProviderRole::Coder,
            provider: ProviderName::Codex,
            source: "request_user_input".to_string(),
            prompt: "请选择实现范围".to_string(),
            options: vec![CodingChoiceOption {
                id: "backend_first".to_string(),
                label: "先做后端".to_string(),
                description: Some("TASK-001 到 TASK-009".to_string()),
            }],
            allow_multiple: false,
            allow_free_text: true,
        })
        .expect("create choice gate");

    assert_eq!(gate.gate_id, "coding_choice_gate_0001");
    assert_eq!(gate.choice_id, "choice_0001");
    assert_eq!(gate.attempt_id, attempt.id);
    assert_eq!(gate.status, CodingChoiceGateStatus::Open);
    assert_eq!(gate.provider, ProviderName::Codex);
    assert_eq!(gate.source, "request_user_input");
    assert_eq!(
        store
            .list_open_choice_gates("project_0001", "issue_0001", &attempt.id)
            .expect("open choice gates")
            .len(),
        1
    );

    let resolved = store
        .resolve_choice_gate(
            "project_0001",
            "issue_0001",
            &attempt.id,
            "choice_0001",
            vec!["backend_first".to_string()],
            Some("先控制范围".to_string()),
        )
        .expect("resolve choice gate");

    assert_eq!(resolved.status, CodingChoiceGateStatus::Resolved);
    assert_eq!(
        resolved
            .response
            .as_ref()
            .expect("response")
            .selected_option_ids,
        vec!["backend_first"]
    );
    assert!(
        store
            .list_open_choice_gates("project_0001", "issue_0001", &attempt.id)
            .expect("open choice gates")
            .is_empty()
    );
}

#[test]
fn status_and_stage_transitions_reject_invalid_backwards_moves() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    assert!(
        store
            .update_attempt_status(
                "project_0001",
                "issue_0001",
                &attempt.id,
                CodingAttemptStatus::Completed,
            )
            .is_err(),
        "created cannot jump directly to completed"
    );

    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("forward stage");
    assert!(
        store
            .update_attempt_stage(
                "project_0001",
                "issue_0001",
                &attempt.id,
                CodingExecutionStage::Coding,
            )
            .is_err(),
        "stage cannot move backwards outside rework"
    );
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Rework,
        )
        .expect("enter rework");
}

fn create_input(work_item_id: &str) -> CreateCodingAttemptInput {
    CreateCodingAttemptInput {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: work_item_id.to_string(),
        base_branch: "main".to_string(),
        branch_name: format!("aria/work-items/{work_item_id}/attempt-1"),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: Some(ProviderName::Fake),
            review_rounds: 1,
        },
        max_auto_rework: 2,
    }
}

fn sample_testing_report(attempt_id: &str) -> TestingReport {
    TestingReport {
        id: "testing_report_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        role_run_id: None,
        run_no: None,
        commands: vec![TestCommand {
            command: vec!["cargo".to_string(), "test".to_string()],
            cwd: PathBuf::from("/tmp/worktree"),
            exit_code: Some(0),
            duration_ms: 100,
            stdout_ref: "artifacts/stdout.txt".to_string(),
            stderr_ref: "artifacts/stderr.txt".to_string(),
            status: TestCommandStatus::Passed,
        }],
        overall_status: TestingOverallStatus::Passed,
        provider_claim: None,
        backend_verified: true,
        started_at: "2026-05-23T00:00:00Z".to_string(),
        completed_at: Some("2026-05-23T00:01:00Z".to_string()),
        plan_id: None,
        plan_summary: None,
        steps: Vec::new(),
        unplanned_commands: Vec::new(),
        unplanned_evidence: Vec::new(),
        missing_required_steps: Vec::new(),
        skipped_required_steps: Vec::new(),
        context_warnings: Vec::new(),
        raw_provider_output_ref: None,
    }
}

fn sample_code_review_report(attempt_id: &str) -> CodeReviewReport {
    CodeReviewReport {
        id: "code_review_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        round: 1,
        verdict: ReviewVerdict::Approve,
        findings: vec![sample_finding()],
        tested_evidence_refs: vec!["testing_report_0001".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "通过".to_string(),
        created_at: "2026-05-23T00:01:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
    }
}

fn sample_review_request(attempt_id: &str) -> ReviewRequest {
    ReviewRequest {
        id: "review_request_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: "main".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        commit_sha: "abc123".to_string(),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: vec![],
        created_at: "2026-05-23T00:02:00Z".to_string(),
        updated_at: "2026-05-23T00:02:00Z".to_string(),
    }
}

fn sample_internal_review(attempt_id: &str, review_request_id: &str) -> InternalPrReview {
    InternalPrReview {
        id: "internal_review_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        review_request_id: review_request_id.to_string(),
        verdict: ReviewVerdict::Approve,
        findings: vec![sample_finding()],
        impact_scope: vec!["src/lib.rs".to_string()],
        pr_description: "实现 work item".to_string(),
        commit_message_suggestion: "feat: implement work item".to_string(),
        tested_evidence_refs: vec!["testing_report_0001".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "最终审查通过".to_string(),
        created_at: "2026-05-23T00:03:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
    }
}

fn sample_finding() -> ReviewFinding {
    ReviewFinding {
        severity: FindingSeverity::Info,
        file_path: Some("src/lib.rs".to_string()),
        line: Some(1),
        message: "ok".to_string(),
        required_action: None,
        source_stage: CodingExecutionStage::CodeReview,
        evidence: Vec::new(),
        related_requirements: Vec::new(),
        related_design_constraints: Vec::new(),
        related_work_item_tasks: Vec::new(),
    }
}

fn sample_node(attempt_id: &str) -> CodingTimelineNode {
    CodingTimelineNode {
        id: "coding_node_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        stage: CodingExecutionStage::Testing,
        title: "测试".to_string(),
        status: CodingTimelineNodeStatus::Running,
        agent_role: Some(CodingAgentRole::Tester),
        summary: None,
        started_at: "2026-05-23T00:01:00Z".to_string(),
        completed_at: None,
        artifact_refs: vec![],
    }
}

#[test]
fn updates_coding_role_run_refs_without_duplicates() {
    let root = tempfile::tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: None,
            ..create_input("work_item_0001")
        })
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Rework,
            CodingProviderRole::Analyst,
            CodingRoleRunTrigger::Initial,
            None,
        )
        .expect("role run");

    let updated = store
        .update_role_run_refs(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            vec!["provider-raw/rework/analyst_decision_0001.txt".to_string()],
            vec!["provider-raw/rework/analyst_evidence_0001.txt".to_string()],
        )
        .expect("update refs");
    assert_eq!(updated.raw_provider_output_refs.len(), 1);
    assert_eq!(updated.artifact_refs.len(), 1);

    let updated = store
        .update_role_run_refs(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            vec!["provider-raw/rework/analyst_decision_0001.txt".to_string()],
            vec!["provider-raw/rework/analyst_evidence_0001.txt".to_string()],
        )
        .expect("update refs again");
    assert_eq!(updated.raw_provider_output_refs.len(), 1);
    assert_eq!(updated.artifact_refs.len(), 1);
}

#[test]
fn appends_and_lists_coding_role_run_events_in_sequence() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("role run");

    let first = store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ProviderPrompt,
            serde_json::json!({
                "mode": "plan_tests",
                "prompt": "plan tests as JSON"
            }),
        )
        .expect("append first event");
    let second = store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::TextDelta,
            serde_json::json!({
                "content": "No tasks found"
            }),
        )
        .expect("append second event");

    assert_eq!(first.sequence, 1);
    assert_eq!(second.sequence, 2);
    assert_eq!(first.attempt_id, attempt.id);
    assert_eq!(first.role_run_id, run.id);
    assert_eq!(first.node_id.as_deref(), Some("coding_node_0003"));
    assert_eq!(first.stage, CodingExecutionStage::Testing);
    assert_eq!(first.role, CodingProviderRole::Tester);
    assert_eq!(second.node_id.as_deref(), Some("coding_node_0003"));

    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &run.id)
        .expect("events");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].attempt_id, attempt.id);
    assert_eq!(events[0].role_run_id, run.id);
    assert_eq!(events[0].node_id.as_deref(), Some("coding_node_0003"));
    assert_eq!(events[0].stage, CodingExecutionStage::Testing);
    assert_eq!(events[0].role, CodingProviderRole::Tester);
    assert_eq!(events[0].event_type, CodingRoleRunEventType::ProviderPrompt);
    assert_eq!(events[1].event_type, CodingRoleRunEventType::TextDelta);
    assert_eq!(events[1].payload["content"], "No tasks found");
}

#[test]
fn role_run_event_large_string_payload_is_moved_to_artifact() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0007".to_string()),
        )
        .expect("role run");
    let long_prompt = "review this diff\n".repeat(2_000);

    let event = store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ProviderPrompt,
            serde_json::json!({
                "mode": "full_conversation",
                "prompt": long_prompt
            }),
        )
        .expect("append event");

    assert!(event.truncated);
    assert_eq!(
        event.artifact_ref.as_deref(),
        Some("artifacts/role-run-events/coding_role_run_0001/0001_prompt.txt")
    );
    assert_eq!(
        event.payload["prompt"]["artifact_ref"],
        "artifacts/role-run-events/coding_role_run_0001/0001_prompt.txt"
    );
    assert_eq!(event.payload["prompt"]["truncated"], true);
    let preview = event.payload["prompt"]["preview"]
        .as_str()
        .expect("preview string");
    assert!(preview.starts_with("review this diff"));
    assert!(preview.len() <= 16_384);

    let artifact = store
        .read_attempt_artifact_text(&attempt.id, event.artifact_ref.as_deref().expect("ref"))
        .expect("artifact text");
    assert_eq!(artifact, long_prompt);
}

#[test]
fn role_run_event_truncates_each_large_payload_field() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0011".to_string()),
        )
        .expect("role run");
    let long_stdout = "stdout line\n".repeat(2_000);
    let long_stderr = "stderr line\n".repeat(2_000);

    let event = store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "command": "cargo test --locked",
                "stdout": long_stdout,
                "stderr": long_stderr
            }),
        )
        .expect("append event");

    assert!(event.truncated);
    assert_eq!(
        event.artifact_ref.as_deref(),
        Some("artifacts/role-run-events/coding_role_run_0001/0001_stdout.txt")
    );

    let stdout_payload = event.payload["stdout"].as_object().expect("stdout object");
    let stderr_payload = event.payload["stderr"].as_object().expect("stderr object");
    assert!(
        stdout_payload["preview"]
            .as_str()
            .expect("stdout preview")
            .len()
            <= 16_384
    );
    assert!(
        stderr_payload["preview"]
            .as_str()
            .expect("stderr preview")
            .len()
            <= 16_384
    );
    assert_eq!(stdout_payload["truncated"], true);
    assert_eq!(stderr_payload["truncated"], true);
    let stdout_ref = stdout_payload["artifact_ref"]
        .as_str()
        .expect("stdout artifact ref");
    let stderr_ref = stderr_payload["artifact_ref"]
        .as_str()
        .expect("stderr artifact ref");
    assert_ne!(stdout_ref, stderr_ref);

    let stdout_artifact = store
        .read_attempt_artifact_text(&attempt.id, stdout_ref)
        .expect("stdout artifact text");
    let stderr_artifact = store
        .read_attempt_artifact_text(&attempt.id, stderr_ref)
        .expect("stderr artifact text");
    assert_eq!(stdout_artifact, long_stdout);
    assert_eq!(stderr_artifact, long_stderr);
}

#[test]
fn role_run_retry_diagnostic_summary_compacts_events_and_refs() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("role run");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Task update",
                "status": "running",
                "detail": "No tasks found"
            }),
        )
        .expect("event");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::Timeout,
            serde_json::json!({
                "reason_code": "plan_tests_timeout",
                "message": "Tester provider timed out"
            }),
        )
        .expect("timeout");
    store
        .update_role_run_refs(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            vec!["provider-raw/testing/plan_tests_0001.txt".to_string()],
            vec!["artifacts/role-run-events/coding_role_run_0001/0001_output.txt".to_string()],
        )
        .expect("refs");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            CodingRoleRunStatus::Blocked,
            Some("plan_tests_timeout".to_string()),
        )
        .expect("blocked");

    let summary = store
        .role_run_retry_diagnostic_summary("project_0001", "issue_0001", &attempt.id, &run.id)
        .expect("summary")
        .expect("summary text");

    assert!(summary.contains("role_run_id: coding_role_run_0001"));
    assert!(summary.contains("reason_code: plan_tests_timeout"));
    assert!(summary.contains("terminal_event: timeout"));
    assert!(summary.contains("Task update"));
    assert!(summary.contains("No tasks found"));
    assert!(summary.contains("provider-raw/testing/plan_tests_0001.txt"));
    assert!(summary.contains("artifacts/role-run-events/coding_role_run_0001/0001_output.txt"));
    assert!(
        summary.len() < 8_000,
        "retry diagnostic summary must stay prompt-safe"
    );
}

#[test]
fn role_run_retry_diagnostic_summary_keeps_recent_metadata_and_payload_refs() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0004".to_string()),
        )
        .expect("role run");

    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Old event",
                "status": "running",
                "detail": "Dropped old event"
            }),
        )
        .expect("old event");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::TextDelta,
            serde_json::json!({
                "content": "DROPPED_TEXT_DELTA_BODY"
            }),
        )
        .expect("old text delta");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Recent setup",
                "status": "running",
                "detail": "Preparing test run"
            }),
        )
        .expect("recent setup");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::TextDelta,
            serde_json::json!({
                "content": "DO_NOT_INJECT_TEXT_DELTA_BODY"
            }),
        )
        .expect("recent text delta");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Recent event 5",
                "status": "running",
                "detail": "Still running"
            }),
        )
        .expect("recent event 5");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Recent event 6",
                "status": "running",
                "detail": "Almost done"
            }),
        )
        .expect("recent event 6");
    let long_stdout = "stdout line\n".repeat(2_000);
    let long_stderr = "stderr line\n".repeat(2_000);
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Cargo test",
                "status": "failed",
                "detail": "Captured command output",
                "stdout": long_stdout,
                "stderr": long_stderr
            }),
        )
        .expect("captured output");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            CodingRoleRunStatus::Blocked,
            Some("tests_failed".to_string()),
        )
        .expect("blocked");

    let summary = store
        .role_run_retry_diagnostic_summary("project_0001", "issue_0001", &attempt.id, &run.id)
        .expect("summary")
        .expect("summary text");

    assert!(!summary.contains("DO_NOT_INJECT_TEXT_DELTA_BODY"));
    assert!(!summary.contains("Dropped old event"));
    assert!(summary.contains("artifacts/role-run-events/coding_role_run_0001/0007_stdout.txt"));
    assert!(summary.contains("artifacts/role-run-events/coding_role_run_0001/0007_stderr.txt"));
    assert!(
        summary.len() <= 8_000,
        "retry diagnostic summary must stay prompt-safe"
    );
}

#[test]
fn role_run_retry_diagnostic_summary_preserves_refs_when_inline_detail_is_long() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0005".to_string()),
        )
        .expect("role run");
    let long_detail = format!("{}DETAIL_SHOULD_BE_TRUNCATED", "x".repeat(10_000));
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Long diagnostic detail",
                "status": "blocked",
                "detail": long_detail
            }),
        )
        .expect("event");
    store
        .update_role_run_refs(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            vec!["provider-raw/testing/long_detail_0001.txt".to_string()],
            vec!["artifacts/role-run-events/coding_role_run_0001/0001_detail.txt".to_string()],
        )
        .expect("refs");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            CodingRoleRunStatus::Blocked,
            Some("long_detail_blocked".to_string()),
        )
        .expect("blocked");

    let summary = store
        .role_run_retry_diagnostic_summary("project_0001", "issue_0001", &attempt.id, &run.id)
        .expect("summary")
        .expect("summary text");

    assert!(summary.contains("Long diagnostic detail"));
    assert!(summary.contains("reason_code: long_detail_blocked"));
    assert!(summary.contains("provider-raw/testing/long_detail_0001.txt"));
    assert!(summary.contains("artifacts/role-run-events/coding_role_run_0001/0001_detail.txt"));
    assert!(!summary.contains("DETAIL_SHOULD_BE_TRUNCATED"));
    assert!(
        summary.len() <= 8_000,
        "retry diagnostic summary must stay prompt-safe"
    );
}
