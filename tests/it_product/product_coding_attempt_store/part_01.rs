use std::path::PathBuf;

use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{
    CodingAttemptStore, CreateChoiceGateInput, CreateCodingAttemptInput,
    CreateCodingExecutionUnitInput, CreateGroupCodingAttemptInput,
};
use cadence_aria::product::coding_models::{
    AnalystDecisionNextStage, AnalystDecisionRecord, AnalystDecisionVerdict,
    AnalystReworkInstructions, CodeReviewReport, CodingAgentRole, CodingAttemptStatus,
    CodingChatEntry, CodingChoiceGateStatus, CodingChoiceOption, CodingContextNote,
    CodingEntryType, CodingExecutionStage, CodingExecutionUnitStatus, CodingProviderRole,
    CodingReworkInstruction, CodingRolePermissionModes, CodingRoleProviderConfigSnapshot,
    CodingRoleRunEventType, CodingRoleRunStatus, CodingRoleRunTrigger, CodingStageGateStatus,
    CodingTimelineNode, CodingTimelineNodeStatus, FindingSeverity, InternalPrReview, PushStatus,
    RemoteKind, ReviewFinding, ReviewRequest, ReviewRequestKind, ReviewVerdict, TestCommand,
    TestCommandStatus, TestingOverallStatus, TestingReport, WorkItemExecutionPlan,
    WorkItemHandoff,
};
use cadence_aria::product::models::WorkItemExecutionPlanStatus;
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
