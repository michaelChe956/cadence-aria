// ---------------------------------------------------------------------------
// C5 Task 6（REQ-INIT-C5-RESUME）：project 级初始化失败等待项投影——
// 同一 `C1WaitingItemDto`，按 parent→后继链计算展示/消隐。
// ---------------------------------------------------------------------------

use crate::product::repository_store::{
    CadenceSkillsPreparationSummary, RepositoryInitializationCommandSummary,
    RepositoryInitializationOperation, RepositoryInitializationOperationInput,
    RepositoryInitializationOperationStore, RepositoryInitializationStepKind,
    RepositoryInitializationSummary, RepositoryRegistrationError, RepositoryRegistrationSuccess,
};
use crate::web::plan_confirmed_info::list_project_c1_waiting_items;

fn init_input() -> RepositoryInitializationOperationInput {
    RepositoryInitializationOperationInput {
        name: "Aria".to_string(),
        git_root: std::path::PathBuf::from("/tmp/aria-c5"),
        default_policy_preset: Some("manual-write".to_string()),
        default_provider_mode: Some("claude_code".to_string()),
    }
}

fn provider_unavailable_error() -> RepositoryRegistrationError {
    let mut error = RepositoryRegistrationError::new(
        "provider_gate",
        "provider_unavailable",
        Some("claude gateway unavailable".to_string()),
        true,
        "Restore Claude Code availability, then resume repository registration.",
    );
    error.provider = Some("claude_code".to_string());
    error.changed_paths = Some(vec![".claude/rules".to_string()]);
    error
}

fn init_success() -> RepositoryRegistrationSuccess {
    RepositoryRegistrationSuccess {
        repository: crate::product::models::RepositoryRecord {
            id: "repository_c5".to_string(),
            project_id: PROJECT_ID.to_string(),
            name: "Aria".to_string(),
            path: std::path::PathBuf::from("/tmp/aria-c5"),
            repo_hash: "repo_hash".to_string(),
            runtime_root: std::path::PathBuf::from("/tmp/aria-c5/.aria/runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "claude_code".to_string(),
            created_at: "2026-09-30T00:10:00Z".to_string(),
            updated_at: "2026-09-30T00:10:00Z".to_string(),
            logical_repository_id: None,
            primary_checkout_id: None,
            identity_schema_version: 0,
        },
        cadence_skills: CadenceSkillsPreparationSummary {
            source_mode: "cached".to_string(),
            source_root: std::path::PathBuf::from("/tmp/cadence-skills"),
            skills_root: std::path::PathBuf::from("/tmp/aria-c5/.claude/skills"),
            git_updated: false,
            link_sync_status: "synchronized".to_string(),
            warnings: Vec::new(),
        },
        initialization: RepositoryInitializationSummary {
            provider: "claude_code".to_string(),
            source: std::path::PathBuf::from("/tmp/cadence-skills"),
            source_mode: "cached".to_string(),
            skills_root: std::path::PathBuf::from("/tmp/aria-c5/.claude/skills"),
            git_updated: false,
            link_sync_status: "synchronized".to_string(),
            commands: vec![RepositoryInitializationCommandSummary {
                command_index: 1,
                command: "/pre-check --no-interrupt --upgrade 用大陆镜像".to_string(),
                status: "completed".to_string(),
                output_summary: Some("ok".to_string()),
            }],
        },
        warnings: Vec::new(),
        changed_paths: Vec::new(),
        git_finalize_warning: None,
        completed_at: "2026-09-30T00:10:00Z".to_string(),
    }
}

fn create_initialization_operation(
    store: &RepositoryInitializationOperationStore,
    operation_id: &str,
    parent: Option<&str>,
    created_at: &str,
) {
    let mut operation = RepositoryInitializationOperation::new(
        operation_id.to_string(),
        PROJECT_ID.to_string(),
        init_input(),
        created_at.to_string(),
    );
    operation.parent_operation_id = parent.map(str::to_string);
    store.create(operation).unwrap();
}

/// 网关不可用失败：cadence_skills 完成、pre_check 失败。
fn fail_initialization_at_pre_check(
    store: &RepositoryInitializationOperationStore,
    operation_id: &str,
    parent: Option<&str>,
    created_at: &str,
) {
    create_initialization_operation(store, operation_id, parent, created_at);
    store
        .mark_running(PROJECT_ID, operation_id, "2026-09-30T00:00:01Z".to_string())
        .unwrap();
    store
        .mark_step_running(
            PROJECT_ID,
            operation_id,
            RepositoryInitializationStepKind::CadenceSkills,
            "2026-09-30T00:00:02Z".to_string(),
        )
        .unwrap();
    store
        .mark_step_completed(
            PROJECT_ID,
            operation_id,
            RepositoryInitializationStepKind::CadenceSkills,
            "2026-09-30T00:00:03Z".to_string(),
        )
        .unwrap();
    store
        .mark_step_running(
            PROJECT_ID,
            operation_id,
            RepositoryInitializationStepKind::PreCheck,
            "2026-09-30T00:00:04Z".to_string(),
        )
        .unwrap();
    store
        .finish_failed(
            PROJECT_ID,
            operation_id,
            Some(RepositoryInitializationStepKind::PreCheck),
            provider_unavailable_error(),
            "2026-09-30T00:00:05Z".to_string(),
        )
        .unwrap();
}

fn complete_initialization(
    store: &RepositoryInitializationOperationStore,
    operation_id: &str,
    parent: Option<&str>,
    created_at: &str,
) {
    create_initialization_operation(store, operation_id, parent, created_at);
    store
        .mark_running(PROJECT_ID, operation_id, "2026-09-30T01:00:01Z".to_string())
        .unwrap();
    let steps = [
        RepositoryInitializationStepKind::CadenceSkills,
        RepositoryInitializationStepKind::PreCheck,
        RepositoryInitializationStepKind::RuleConfig,
        RepositoryInitializationStepKind::McpConfiguration,
        RepositoryInitializationStepKind::ProjectRulesExamples,
    ];
    for (index, step) in steps.iter().enumerate() {
        store
            .mark_step_running(
                PROJECT_ID,
                operation_id,
                *step,
                format!("2026-09-30T01:00:{:02}Z", index * 2 + 2),
            )
            .unwrap();
        store
            .mark_step_completed(
                PROJECT_ID,
                operation_id,
                *step,
                format!("2026-09-30T01:00:{:02}Z", index * 2 + 3),
            )
            .unwrap();
    }
    store
        .mark_step_running(
            PROJECT_ID,
            operation_id,
            RepositoryInitializationStepKind::GitFinalize,
            "2026-09-30T01:00:20Z".to_string(),
        )
        .unwrap();
    store
        .checkpoint_git_finalize_result(PROJECT_ID, operation_id, init_success())
        .unwrap();
    store
        .mark_step_completed(
            PROJECT_ID,
            operation_id,
            RepositoryInitializationStepKind::GitFinalize,
            "2026-09-30T01:00:21Z".to_string(),
        )
        .unwrap();
    store
        .finish_completed(
            PROJECT_ID,
            operation_id,
            init_success(),
            "2026-09-30T01:00:22Z".to_string(),
        )
        .unwrap();
}

#[test]
fn project_c1_waiting_items_surface_failed_repository_initialization_once() {
    // 阶段 1：网关不可用失败 → project 级恰一个等待项，含结构化诊断与
    // “恢复后继续”动作；issue 级投影零变化。
    let (_tmp, paths, _lifecycle) = fixture_root();
    let store = RepositoryInitializationOperationStore::new(paths.clone());
    fail_initialization_at_pre_check(
        &store,
        "repository_initialization_orig",
        None,
        "2026-09-30T00:00:00Z",
    );

    let items = list_project_c1_waiting_items(&paths, PROJECT_ID).unwrap();
    assert_eq!(items.len(), 1, "{items:?}");
    let item = &items[0];
    assert_eq!(
        item.id,
        "c1:project:project_0001:repository_init:repository_initialization_orig"
    );
    assert_eq!(item.kind, "repository_initialization_failed");
    assert_eq!(
        item.operation_id.as_deref(),
        Some("repository_initialization_orig")
    );
    assert_eq!(item.project_id.as_deref(), Some(PROJECT_ID));
    assert!(item.issue_id.is_none());
    assert_eq!(item.completed_steps, vec!["cadence_skills".to_string()]);
    assert_eq!(
        item.actions,
        vec!["resume_repository_initialization".to_string()]
    );
    let diagnostics = item.diagnostics.as_ref().expect("diagnostics projected");
    assert_eq!(diagnostics.failed_step, "pre_check");
    assert_eq!(diagnostics.reason_code, "provider_unavailable");
    assert_eq!(diagnostics.provider.as_deref(), Some("claude_code"));
    assert_eq!(diagnostics.changed_paths, vec![".claude/rules".to_string()]);
    assert!(diagnostics.retryable);
    assert!(item.parent_operation_id.is_none());
    assert!(item.superseded_by.is_none());
    let wire = serde_json::to_value(item).unwrap();
    assert_eq!(wire["diagnostics"]["reason_code"], "provider_unavailable");
    assert_eq!(wire["operation_id"], "repository_initialization_orig");
    assert_eq!(wire["project_id"], PROJECT_ID);

    // issue 级消费零变化：无 enrollment 的 issue 不新增任何条目。
    assert!(
        list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty()
    );

    // 阶段 2：successor Completed → 原 waiting item 稳定消隐，记录只读可查。
    let (_tmp, paths, _lifecycle) = fixture_root();
    let store = RepositoryInitializationOperationStore::new(paths.clone());
    fail_initialization_at_pre_check(
        &store,
        "repository_initialization_orig",
        None,
        "2026-09-30T00:00:00Z",
    );
    complete_initialization(
        &store,
        "repository_initialization_resume_done",
        Some("repository_initialization_orig"),
        "2026-09-30T01:00:00Z",
    );
    assert!(
        list_project_c1_waiting_items(&paths, PROJECT_ID)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .get(PROJECT_ID, "repository_initialization_orig")
            .unwrap()
            .status,
        crate::product::repository_store::RepositoryInitializationOperationStatus::Failed
    );

    // 阶段 3：successor 再次 Failed → 展示最新失败链叶并保留 parent 关联；
    // Created 后继（未执行）→ 运行中只读项，无 resume 动作。
    let (_tmp, paths, _lifecycle) = fixture_root();
    let store = RepositoryInitializationOperationStore::new(paths.clone());
    fail_initialization_at_pre_check(
        &store,
        "repository_initialization_chain_orig",
        None,
        "2026-09-30T00:00:00Z",
    );
    fail_initialization_at_pre_check(
        &store,
        "repository_initialization_chain_retry",
        Some("repository_initialization_chain_orig"),
        "2026-09-30T01:00:00Z",
    );
    let items = list_project_c1_waiting_items(&paths, PROJECT_ID).unwrap();
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(
        items[0].operation_id.as_deref(),
        Some("repository_initialization_chain_retry")
    );
    assert_eq!(
        items[0].parent_operation_id.as_deref(),
        Some("repository_initialization_chain_orig")
    );
    assert_eq!(
        items[0].id,
        "c1:project:project_0001:repository_init:repository_initialization_chain_retry"
    );

    let (_tmp, paths, _lifecycle) = fixture_root();
    let store = RepositoryInitializationOperationStore::new(paths.clone());
    fail_initialization_at_pre_check(
        &store,
        "repository_initialization_pending_orig",
        None,
        "2026-09-30T00:00:00Z",
    );
    create_initialization_operation(
        &store,
        "repository_initialization_pending_resume",
        Some("repository_initialization_pending_orig"),
        "2026-09-30T02:00:00Z",
    );
    let items = list_project_c1_waiting_items(&paths, PROJECT_ID).unwrap();
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(
        items[0].operation_id.as_deref(),
        Some("repository_initialization_pending_resume")
    );
    assert!(
        items[0].actions.is_empty(),
        "running successor must be read-only without resume action"
    );
    assert!(items[0].diagnostics.is_none());
}

/// G1（终局关闸缺口）：plan approve（REST human-actions）成功后 session
/// 推进 confirmed、timeline 门节点 Completed，但 human_gate_snapshot 的
/// candidate_recovery 不随之清除——投影只看快照存在性，等待项不消隐且
/// gate id 陈旧（现场 session_auto_bd69a84a/timeline_node_003）。
/// 修复语义：投影派生按当前门状态过滤——gate 节点已终态
/// （Completed/Failed/Skipped）即视为已处理不再投影；节点 Active/Paused
/// 仍投影；节点缺失（无法证明已处理）保持投影（fail-safe）。
#[test]
fn c1_candidate_recovery_item_clears_once_gate_is_resolved() {
    use crate::web::workspace_ws_types::common::ProviderConfigSnapshot as TimelineProviderSnapshot;
    use crate::web::workspace_ws_types::stage::WorkspaceStage;
    use crate::web::workspace_ws_types::timeline::{
        TimelineNode, TimelineNodeStatus, TimelineNodeType,
    };

    let (_tmp, paths, lifecycle) = fixture_root();
    let plan_id = "plan_g1";
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: plan_id.to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Fake,
            reviewer_provider: Some(ProviderName::Fake),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .unwrap();
    enable_enrollment(&paths, plan_id, &session.id);
    let mut durable: WorkspaceSessionRecord =
        lifecycle.get_workspace_session(&session.id).unwrap();
    durable.status = crate::product::models::WorkspaceSessionStatus::Confirmed;
    durable.human_gate_snapshot = Some(HumanGateSnapshot {
        findings: vec![],
        repeated_fingerprints: vec![],
        attempts_used: 0,
        manual_repairs_remaining: 2,
        accepted_feedback_turns: None,
        candidate_recovery: Some(CandidateSnapshotRecovery {
            complete: true,
            gate_id: "timeline_node_003".to_string(),
            source_revision_ref: None,
            source_revision_hash: None,
            plan_candidate_ir_ref: Some("ir_ref".to_string()),
            mechanical_report_ref: Some("report_ref".to_string()),
            budget_remaining: Some(3),
            missing: vec![],
            completed_steps: vec![
                "candidate_source_persisted".to_string(),
                "mechanical_report_persisted".to_string(),
            ],
            commands: vec![],
            assessed_at: "2026-09-29T00:00:00Z".to_string(),
        }),
        trigger: crate::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
        resumable: true,
    });
    write_json(
        &paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("workspace-sessions")
            .join(format!("{}.json", session.id)),
        &durable,
    )
    .unwrap();

    let gate_node = |status: TimelineNodeStatus| TimelineNode {
        node_id: "timeline_node_003".to_string(),
        node_type: TimelineNodeType::HumanConfirm,
        agent: None,
        stage: WorkspaceStage::HumanConfirm,
        round: None,
        status,
        title: "人工确认".to_string(),
        summary: None,
        started_at: "2026-09-29T00:00:00Z".to_string(),
        completed_at: Some("2026-09-29T00:01:00Z".to_string()),
        duration_ms: Some(60000),
        artifact_ref: None,
        provider_config_snapshot: TimelineProviderSnapshot {
            author: ProviderName::Fake,
            reviewer: Some(ProviderName::Fake),
            review_rounds: 1,
            permission_modes: Default::default(),
        },
        retry: None,
    };
    let timeline_path = paths
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-timelines")
        .join(&session.id)
        .join("timeline_nodes.json");

    // 门已 approve（节点 Completed）：等待项必须消隐。
    write_json(
        &timeline_path,
        &vec![gate_node(TimelineNodeStatus::Completed)],
    )
    .unwrap();
    let items = list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID).unwrap();
    assert!(
        !items.iter().any(|item| item.kind == "candidate_recovery"),
        "resolved gate must clear the stale candidate_recovery item: {items:?}"
    );

    // 门仍开放（节点 Active）：等待项保持投影（真实等待语义不变）。
    write_json(
        &timeline_path,
        &vec![gate_node(TimelineNodeStatus::Active)],
    )
    .unwrap();
    let items = list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID).unwrap();
    assert!(
        items.iter().any(|item| item.kind == "candidate_recovery"),
        "open gate must keep projecting the recovery item"
    );
}
