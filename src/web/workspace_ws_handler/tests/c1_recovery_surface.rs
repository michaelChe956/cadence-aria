//! C1 Task 10 综合验收串测（tasks.md §3.2）：A07/A09/A12/A13 四条真实
//! 故障链在同一测试内依序跑通「错误 → durable 事实 → 用户操作 → durable
//! 结果 → 原链续进」。每个断言读取真实 durable
//! enrollment/history/session/gate/advance retry/attempt/lease/通知结果，
//! 不读 mock 回调、不做字段非空式断言。
//!
//! - **A07**：零 WS consumer 的孤儿候选门 → REST CandidateRecovery 恢复 →
//!   同 command 幂等 / 异 payload fail-closed / 旧 gate 零副作用 → 既有
//!   approve 自动续进 Confirmed（issue 不 abandon）。
//! - **A09（lease）**：真实 worktree 锁 + attempt 的三态 fail-closed 分类
//!   （活跃等待 / 死亡待确认 / 未知停等）；死亡租约经确认接管 REST 原子清
//!   owner，同 command 重放幂等，下一次 acquire 成为新 owner（无第二
//!   attempt）。
//! - **A09（retry）**：PlanBindingSaved 注入 Failed advance → retry REST
//!   副作用门 NeedsHuman（原 Failed 审计保留）→ 确认后续做同一 attempt 到
//!   Ready → 同 command 重放 Replayed；普通 advance 不隐式 retry。
//! - **A12**：existing/create 意图合同——合法 create 编译且意图随 IR 持久
//!   化；未声明的基线外写面停等（`intent_undeclared`）；不能执行的合同
//!   拒绝（`intent_unexecutable`）且不扩大 scope。
//! - **A13**：Confirmed enrollment 显式换代 v1→v2——旧代 advance/start 回执
//!   fail-closed、binding 历史可查、无第二 attempt/provider；Manual 人工
//!   路径不被 enrollment 校验拦截（同代 durable replay 同一 attempt）。
//!
//! 各链的深层语义（缺快照禁 approve、CAS 竞态、compile child 跨代新建等）
//! 由对应定向测试持有（candidate_row / admission_tests / advance_handler /
//! c1_existing_create / part_11 / advance_plan tests）；本串测证明四链可以
//! 在同一验收序列里各自从真实故障走到用户操作后的 durable 续进。

use crate::product::advance_store::{
    AdvanceInput, AdvanceOutcome, AdvanceStatus, AdvanceStore,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::coding_attempt_store::{
    CodingAttemptStore, CreateCodingAttemptInput,
};
use crate::product::coding_models::CodingAttemptStatus;
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::json_store::write_json;
use crate::product::lifecycle_store::{
    CreateWorkspaceSessionInput, LifecycleStore, UpsertIssueSharedWorktreeInput,
};
use crate::product::logical_codebase::{EnrollmentTarget, LogicalRepositoryId};
use crate::product::models::automation::{
    EnrollmentBindingIdentityInput, EnrollmentOptions, EnrollmentRebindRequest,
    EnrollmentSource, EnrollmentWriteCommand, LeaseDisposition, SourceRevisionRef,
};
use crate::product::models::lifecycle::IssueWorkItemPlanOptions;
use crate::product::models::WorkspaceSessionStatus;
use crate::product::models::outline::{
    WorkItemDraftCandidate, WorkItemDraftRecord, WorkItemDraftStatus,
    WorkItemDraftVerificationPlan, WorkItemGenerationMode,
};
use crate::product::models::provider::ProviderName;
use crate::product::models::{
    IssueRecord, IssuePhase, IssueStatus, RepositoryProfile, RepositoryProfileConfidence,
    WorkspaceType,
};
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::product::work_item_plan_compiler::{
    PlanCandidateValidationContext, WorkItemPlanSourceContext, compile_work_item_plan,
    validate_plan_candidate_ir, validate_work_item_intent_contract,
};
use crate::product::work_item_plan_policy::RunPolicy;
use crate::product::work_item_plan_store::WorkItemPlanStore;
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::product::workspace_engine::{
    AdvanceInitializationFailpoint, AdvanceInitializationFailpointMode, WorkspaceEngine,
    WorkspaceSession, register_advance_initialization_failpoint,
};
use crate::web::handlers::automation_enrollment_test_support::create_plan_and_session;
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use std::collections::BTreeSet;
use std::sync::Arc;
use tower::ServiceExt;

/// C1 综合验收串测：四链依序执行，任何一环 durable 断言失败即整体失败。
#[tokio::test]
async fn c1_a07_a09_a12_a13_recovery_surface() {
    a07_orphan_candidate_recovery_chain().await;
    a09_lease_three_states_and_confirmed_takeover_chain().await;
    a09_failed_advance_explicit_retry_chain().await;
    a12_existing_create_intent_contract_chain();
    a13_generation_switch_chain().await;
}

// ============================================================================
// A07 —— 孤儿候选门恢复（零 WS consumer；REQ-C1-GATE-01/02）。
// 复用 campaign_stage3 的真实 HTTP fixture：无任何 WS attachment，候选事实
// 先 durable 落盘，恢复/审批全部经 REST。
// ============================================================================

async fn a07_orphan_candidate_recovery_chain() {
    let fixture = super::campaign_stage3_interactive::workspace_human_action_http_fixture(
        2,
        Vec::new(),
    )
    .await;
    let gate_id = fixture.active_gate_id().await.expect("durable gate node");

    // 错误 → 通知 → 用户操作：candidate_recovery 只读评估 + label 落盘。
    let (status, body) = fixture
        .post_human_action(serde_json::json!({
            "type": "candidate_recovery",
            "command_id": "cmd-c1-serial-a07-recover",
            "expected_gate_id": gate_id,
            "action": "recover",
        }))
        .await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    assert_eq!(body["state"], "accepted", "body: {body}");
    assert_eq!(body["complete"], true, "body: {body}");
    assert_eq!(body["gate_id"], serde_json::json!(gate_id));

    // durable 结果：会话仍 WaitingForHuman、无 provider 启动账目。
    let record = fixture.session_record().await;
    assert_eq!(record.status, WorkspaceSessionStatus::WaitingForHuman);
    assert!(record.provider_start_ledger.is_empty());

    // 同 command 同 payload 重放：同一 durable 结果（replayed）。
    let (status, body) = fixture
        .post_human_action(serde_json::json!({
            "type": "candidate_recovery",
            "command_id": "cmd-c1-serial-a07-recover",
            "expected_gate_id": gate_id,
            "action": "recover",
        }))
        .await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    assert_eq!(body["state"], "replayed", "body: {body}");

    // 同 command 异 payload：fail-closed 409（命令账本判异 payload）。
    let (status, _body) = fixture
        .post_human_action(serde_json::json!({
            "type": "candidate_recovery",
            "command_id": "cmd-c1-serial-a07-recover",
            "expected_gate_id": gate_id,
            "action": "rebuild",
        }))
        .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT);

    // 旧 gate id：门身份比对 409，durable 会话不动。
    let (status, _body) = fixture
        .post_human_action(serde_json::json!({
            "type": "candidate_recovery",
            "command_id": "cmd-c1-serial-a07-stale-gate",
            "expected_gate_id": "timeline_node_stale",
            "action": "recover",
        }))
        .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT);
    assert_eq!(
        fixture.session_record().await.status,
        WorkspaceSessionStatus::WaitingForHuman
    );

    // 恢复后原门仍可用既有 approve：自动续进到 Confirmed（issue 不 abandon）。
    let (status, body) = fixture
        .post_human_action(serde_json::json!({
            "type": "approve",
            "command_id": "cmd-c1-serial-a07-approve",
            "expected_gate_id": gate_id,
        }))
        .await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    let confirmed = fixture
        .await_session_status(WorkspaceSessionStatus::Confirmed)
        .await;
    assert_eq!(confirmed.status, WorkspaceSessionStatus::Confirmed);
}

// ============================================================================
// A09（lease）—— 真实 worktree 锁三态 + 确认接管 REST（REQ-WIGA-03）。
// ============================================================================

async fn a09_lease_three_states_and_confirmed_takeover_chain() {
    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const ISSUE_TRANSIENT: &str = "issue_transient";
    const WORK_ITEM_ID: &str = "work_item_0001";

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let paths = ProductAppPaths::new(root.join(".aria"));
    seed_project_and_issue(&paths, PROJECT_ID, ISSUE_ID, "c1 serial lease fixture");
    seed_project_and_issue(&paths, PROJECT_ID, ISSUE_TRANSIENT, "c1 serial transient lease");

    // 真实 attempt + issue 共享 worktree 锁（owner 绑定 attempt）。
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/c1-serial-lease".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 0,
        })
        .unwrap();
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "main".to_string(),
            worktree_path: std::path::PathBuf::from("/tmp/c1-serial-lease-worktree"),
            base_branch: "main".to_string(),
        })
        .unwrap();
    let lease = lifecycle
        .try_acquire_issue_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            WORK_ITEM_ID,
            &format!("issue_worktree_lease_{}", attempt.id),
        )
        .unwrap();
    assert!(lease.acquired);
    lifecycle
        .bind_issue_worktree_lock_to_attempt(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID, &attempt.id)
        .unwrap();

    let engine = lease_engine(&store);
    // 活跃：只等待；第二 acquire 不偷 owner。
    let active = engine.classify_worktree_lease(PROJECT_ID, ISSUE_ID);
    assert_eq!(active.disposition, LeaseDisposition::ActiveWait);
    assert_eq!(active.lease_id, attempt.id);
    let stolen = lifecycle
        .try_acquire_issue_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            "work_item_0002",
            "issue_worktree_lease_second",
        )
        .map_err(|error| error.to_string())
        .expect_err("active lease must not be stolen");
    assert!(stolen.contains("issue_worktree_active"), "{stolen}");

    // 未知：瞬态 lease（owner 未绑定 attempt）与记录缺失都停等。
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_TRANSIENT.to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "main".to_string(),
            worktree_path: std::path::PathBuf::from("/tmp/c1-serial-transient-worktree"),
            base_branch: "main".to_string(),
        })
        .unwrap();
    lifecycle
        .try_acquire_issue_worktree_lock(
            PROJECT_ID,
            ISSUE_TRANSIENT,
            WORK_ITEM_ID,
            "issue_worktree_lease_transient",
        )
        .unwrap();
    assert_eq!(
        engine
            .classify_worktree_lease(PROJECT_ID, ISSUE_TRANSIENT)
            .disposition,
        LeaseDisposition::UnknownNeedsHuman
    );
    assert_eq!(
        engine
            .classify_worktree_lease(PROJECT_ID, "issue_no_worktree_record")
            .disposition,
        LeaseDisposition::UnknownNeedsHuman
    );

    // 死亡：终态 attempt → DeadNeedsTakeover（判定只读，未确认不推进）。
    set_attempt_status(&store, PROJECT_ID, ISSUE_ID, &attempt.id, CodingAttemptStatus::Failed);
    let dead = engine.classify_worktree_lease(PROJECT_ID, ISSUE_ID);
    assert_eq!(dead.disposition, LeaseDisposition::DeadNeedsTakeover);
    assert_eq!(dead.lease_id, attempt.id);

    // enrollment binding v1（显式 target；takeover REST 的身份基准）。
    let automation = IssueAutomationStore::new(paths.clone());
    automation
        .compare_and_set(
            PROJECT_ID,
            ISSUE_ID,
            None,
            EnrollmentWriteCommand::Enable {
                selection_key: "c1-serial-lease".to_string(),
                source: EnrollmentSource {
                    stories: vec![SourceRevisionRef {
                        id: "story_spec_0001".to_string(),
                        version: 1,
                    }],
                    designs: vec![],
                },
                options: serial_enrollment_options(),
                logical_repository_id: LogicalRepositoryId(uuid::Uuid::nil()),
                target: Some(EnrollmentTarget::SingleRepository {
                    repository_id: "repository_0001".to_string(),
                }),
            },
        )
        .unwrap();
    let binding = automation
        .get(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .expect("enrollment")
        .binding_history
        .expect("binding history")
        .current;

    // 确认接管 REST（真实 router）。
    let state = WebAppState::new(root.clone(), WebRuntime::new_fake(root.clone()));
    let app = crate::web::app::build_web_router(state);
    let uri = format!(
        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment/lease/takeover"
    );
    let takeover_body = |command_id: &str, lease_id: &str| {
        serde_json::json!({
            "command_id": command_id,
            "expected_binding": binding,
            "expected_lease_id": lease_id,
            "expected_attempt_id": attempt.id,
        })
    };

    // 活跃复活 → Rejected，不写任何 durable 文件（owner 不变）。
    set_attempt_status(&store, PROJECT_ID, ISSUE_ID, &attempt.id, CodingAttemptStatus::Running);
    let (status, body) = post_router_json(
        &app,
        &uri,
        &takeover_body("cmd-c1-serial-takeover-active", &attempt.id),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    assert_eq!(body["state"], "rejected", "body: {body}");
    let record = lifecycle
        .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .expect("worktree record");
    assert_eq!(
        record.current_lock_owner_id.as_deref(),
        Some(attempt.id.as_str())
    );

    // 回到死亡后：过期 lease id → IdentityMismatch 409，durable 不变。
    set_attempt_status(&store, PROJECT_ID, ISSUE_ID, &attempt.id, CodingAttemptStatus::Failed);
    let (status, body) = post_router_json(
        &app,
        &uri,
        &takeover_body("cmd-c1-serial-takeover-stale-lease", "stale-lease-id"),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "body: {body}");
    assert_eq!(body["code"], "lease_takeover_identity_mismatch", "body: {body}");

    // 合法接管：Accepted，死亡 owner 原子清出。
    let (status, body) = post_router_json(
        &app,
        &uri,
        &takeover_body("cmd-c1-serial-takeover-accept", &attempt.id),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    assert_eq!(body["state"], "accepted", "body: {body}");
    let evidence = body["lease"]["evidence"].as_array().expect("evidence");
    assert!(
        evidence
            .iter()
            .any(|line| line.as_str().unwrap_or("").contains("takeover_confirmed")),
        "body: {body}"
    );
    let record = lifecycle
        .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .expect("worktree record");
    assert_eq!(record.current_lock_owner_id, None);
    assert_eq!(record.current_active_work_item_id, None);

    // 同 command 同 payload 重放：Replayed（首次 durable 结果）。
    let (status, body) = post_router_json(
        &app,
        &uri,
        &takeover_body("cmd-c1-serial-takeover-accept", &attempt.id),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    assert_eq!(body["state"], "replayed", "body: {body}");

    // 接管后下一次 acquire 成为新 owner；attempt 总数不变（无第二 attempt）。
    let next = lifecycle
        .try_acquire_issue_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            "work_item_0002",
            "issue_worktree_lease_after_takeover",
        )
        .unwrap();
    assert!(next.acquired);
    assert_eq!(
        store.list_attempts_for_issue(PROJECT_ID, ISSUE_ID).unwrap().len(),
        1,
        "takeover must not create a second attempt"
    );
}

fn lease_engine(store: &CodingAttemptStore) -> CodingWorkspaceEngine {
    let (event_tx, _event_rx) =
        tokio::sync::mpsc::channel::<crate::web::coding_ws_handler::CodingWsOutMessage>(1);
    CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
}

fn set_attempt_status(
    store: &CodingAttemptStore,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
    status: CodingAttemptStatus,
) {
    let mut attempt = store.get_attempt(project_id, issue_id, attempt_id).unwrap();
    attempt.status = status;
    store.write_coding_attempt_for_test(&attempt).unwrap();
}

fn seed_project_and_issue(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    title: &str,
) {
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: title.to_string(),
            description: None,
        })
        .unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    write_json(
        &paths.issue_root(project_id, issue_id).join("issue.json"),
        &IssueRecord {
            id: issue_id.to_string(),
            project_id: project_id.to_string(),
            repo_id: None,
            logical_codebase_id: None,
            title: title.to_string(),
            description: None,
            change_id: format!("c1_serial_{issue_id}"),
            phase: IssuePhase::Clarification,
            status: IssueStatus::Draft,
            active_binding_id: None,
            created_at: now.clone(),
            updated_at: now,
            base_branch: None,
        },
    )
    .unwrap();
}

fn serial_enrollment_options() -> EnrollmentOptions {
    EnrollmentOptions {
        author_provider: ProviderName::Fake,
        reviewer_provider: ProviderName::Fake,
        review_rounds: 1,
        superpowers_enabled: false,
        openspec_enabled: false,
        plan_options: IssueWorkItemPlanOptions {
            include_integration_tests: true,
            include_e2e_tests: false,
            force_frontend_backend_split: false,
            require_execution_plan_confirm: false,
        },
    }
}

async fn post_router_json(
    app: &axum::Router,
    uri: &str,
    body: &serde_json::Value,
) -> (axum::http::StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

// ============================================================================
// A09（retry）—— Failed advance 显式 retry REST（REQ-ADV-C1-RETRY）。
// 铺底与 advance_handler 定向测试同源（PlanRepairFixtureRuntime 真实 fixture
// + accepted drafts + 手工 engine 注入 PlanBindingSaved 失败）。
// ============================================================================

async fn a09_failed_advance_explicit_retry_chain() {
    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_plan_0001";
    const PLAN_ID: &str = "work_item_plan_0001";

    let root = tempfile::tempdir().unwrap();
    crate::web::test_controls::PlanRepairFixtureRuntime::seed(
        root.path(),
        crate::web::test_controls::PlanRepairFixtureControl::default(),
    )
    .await
    .expect("seed authoritative advance fixture");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let seeded_attempt = coding_store
        .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID, None)
        .unwrap()
        .expect("seeded group attempt");
    coding_store
        .delete_attempt(PROJECT_ID, ISSUE_ID, &seeded_attempt.id)
        .unwrap();
    seed_serial_advance_drafts(&app_paths);

    let lifecycle = LifecycleStore::new(app_paths.clone());
    let session_record = lifecycle
        .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .into_iter()
        .find(|record| {
            record.workspace_type == WorkspaceType::WorkItemPlan && record.entity_id == PLAN_ID
        })
        .expect("seeded plan workspace session");
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("advance-checkpoints"))),
        lifecycle,
        event_tx,
        WorkspaceSession::from_record(session_record),
    );

    // 注入 PlanBindingSaved（≥ WorktreeBound）失败：可能存在外部副作用。
    let request = AdvanceInput {
        command_id: "cmd-c1-serial-a09-advance".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: PLAN_ID.to_string(),
    };
    let _failpoint = register_advance_initialization_failpoint(
        &request,
        AdvanceInitializationFailpoint::PlanBindingSaved,
        AdvanceInitializationFailpointMode::Error,
    );
    engine
        .handle_advance(request)
        .await
        .expect_err("injected engine failure must be returned");

    let advance_store = AdvanceStore::new(app_paths.clone());
    let record = advance_store
        .get_advance_for_plan(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .expect("failed record");
    assert_eq!(record.status, AdvanceStatus::Failed);
    let journal = advance_store
        .get_advance_initialization(&record)
        .unwrap()
        .expect("failed initialization journal");
    let failed_error = journal.error.clone().expect("journal failure fact");
    let attempt_id = journal.attempt_id.clone();
    let checkpoint = journal.phase;
    let attempts_before = coding_store
        .list_attempts_for_issue(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .len();

    // 普通 advance（异 command）不隐式 retry：Failed 事实按 Replayed 返回。
    let replay = engine
        .handle_advance(AdvanceInput {
            command_id: "cmd-c1-serial-a09-plain-advance".to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: PLAN_ID.to_string(),
        })
        .await
        .expect("plain advance replays the failed fact");
    assert!(
        matches!(replay, AdvanceOutcome::Replayed { ref record } if record.status == AdvanceStatus::Failed),
        "plain advance must not implicitly retry: {replay:?}"
    );

    // enrollment binding v1（retry REST 的身份基准）。
    let automation = IssueAutomationStore::new(app_paths.clone());
    automation
        .compare_and_set(
            PROJECT_ID,
            ISSUE_ID,
            None,
            EnrollmentWriteCommand::Enable {
                selection_key: "c1-serial-retry".to_string(),
                source: EnrollmentSource {
                    stories: vec![SourceRevisionRef {
                        id: "story_spec_0001".to_string(),
                        version: 1,
                    }],
                    designs: vec![],
                },
                options: serial_enrollment_options(),
                logical_repository_id: LogicalRepositoryId(uuid::Uuid::nil()),
                target: Some(EnrollmentTarget::SingleRepository {
                    repository_id: "repository_0001".to_string(),
                }),
            },
        )
        .unwrap();
    let binding = automation
        .get(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .expect("enrollment")
        .binding_history
        .expect("binding history")
        .current;

    // retry REST（真实 router）。
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = crate::web::app::build_web_router(state);
    let uri = format!(
        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/work-item-plans/{PLAN_ID}/advance/retry-initialization"
    );
    let retry_body = |command_id: &str, checkpoint: serde_json::Value, confirm: bool| {
        serde_json::json!({
            "command_id": command_id,
            "expected_binding": binding,
            "expected_attempt_id": attempt_id,
            "expected_checkpoint": checkpoint,
            "confirm_unknown_side_effect": confirm,
        })
    };

    // 副作用门：未确认 → NeedsHuman（outcome=None，只写 retry 事实）。
    let (status, body) = post_router_json(
        &app,
        &uri,
        &retry_body(
            "cmd-c1-serial-a09-retry-unconfirmed",
            serde_json::to_value(checkpoint).unwrap(),
            false,
        ),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    assert_eq!(body["state"], "needs_human", "body: {body}");
    assert!(body["outcome"].is_null(), "body: {body}");
    let unchanged = advance_store
        .get_advance_for_plan(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .expect("record");
    assert_eq!(unchanged.status, AdvanceStatus::Failed);
    let unchanged_journal = advance_store
        .get_advance_initialization(&unchanged)
        .unwrap()
        .expect("journal");
    assert_eq!(
        unchanged_journal.error.as_deref(),
        Some(failed_error.as_str()),
        "原 Failed 审计必须保留"
    );

    // 过期 checkpoint：拒绝且 durable 不变（不写 retry 事实）。
    let (status, body) = post_router_json(
        &app,
        &uri,
        &retry_body(
            "cmd-c1-serial-a09-retry-stale",
            serde_json::json!("journal_prepared"),
            true,
        ),
    )
    .await;
    assert!(!status.is_success(), "body: {body}");
    assert_eq!(body["code"], "retry_initialization_rejected", "body: {body}");

    // 确认未知副作用后续做：Accepted，同一 attempt 到 Ready。
    let (status, body) = post_router_json(
        &app,
        &uri,
        &retry_body(
            "cmd-c1-serial-a09-retry-confirmed",
            serde_json::to_value(checkpoint).unwrap(),
            true,
        ),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    assert_eq!(body["state"], "accepted", "body: {body}");
    assert_eq!(body["retry"]["attempt_id"], serde_json::json!(attempt_id));
    let ready = advance_store
        .get_advance_for_plan(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .unwrap()
        .expect("record");
    assert_eq!(ready.status, AdvanceStatus::Ready);
    assert_eq!(ready.attempt_id.as_deref(), Some(attempt_id.as_str()));
    assert_eq!(
        coding_store
            .list_attempts_for_issue(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .len(),
        attempts_before,
        "retry 续做同一 attempt，不建第二 attempt"
    );

    // 同 command 同 payload 重放：Replayed（Ready 幂等投影）。
    let (status, body) = post_router_json(
        &app,
        &uri,
        &retry_body(
            "cmd-c1-serial-a09-retry-confirmed",
            serde_json::to_value(checkpoint).unwrap(),
            true,
        ),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    assert_eq!(body["state"], "replayed", "body: {body}");
    assert_eq!(
        coding_store
            .list_attempts_for_issue(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .len(),
        attempts_before
    );
}

/// advance 铺底：与 advance_handler 定向测试同款（accepted drafts 经真实
/// store 面），保证 handle_advance 能推进到 PlanBindingSaved。
fn seed_serial_advance_drafts(app_paths: &ProductAppPaths) {
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    let lineage = revision_store
        .get_plan_lineage("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .unwrap();
    let plan_store = WorkItemPlanStore::new(app_paths.clone());
    for (logical_id, revision_id) in [
        ("wi_core", "work_item_revision_wi_core_0001"),
        (
            "wi_registration",
            "work_item_revision_wi_registration_0001",
        ),
        ("wi_unrelated", "work_item_revision_wi_unrelated_0001"),
    ] {
        let revision = revision_store
            .get_work_item_revision(&lineage, logical_id, revision_id)
            .unwrap();
        plan_store
            .put_draft_record(&WorkItemDraftRecord {
                project_id: "project_0001".to_string(),
                issue_id: "issue_plan_0001".to_string(),
                plan_id: "work_item_plan_0001".to_string(),
                draft_id: revision.source_draft_revision_id.clone(),
                outline_id: format!("outline_{logical_id}"),
                generation_round_id: "round_advance".to_string(),
                batch_id: None,
                attempt_index: 1,
                outline_version_ref: "outline_version_advance".to_string(),
                generation_mode: WorkItemGenerationMode::Serial,
                generation_diagnostics: None,
                candidate: WorkItemDraftCandidate {
                    target_repository_id: None,
                    outline_id: format!("outline_{logical_id}"),
                    logical_work_item_id: logical_id.to_string(),
                    canonical_contract_candidate: revision.canonical_contract,
                    verification_plan: WorkItemDraftVerificationPlan { checks: Vec::new() },
                },
                status: WorkItemDraftStatus::Accepted,
                active: true,
                superseded_by_draft_id: None,
                supersede_reason: None,
                copied_from_draft_id: None,
                review_node_id: None,
                review_verdict_ref: None,
                generated_from_node_id: "c1_serial_advance".to_string(),
                accepted_at: Some("2026-08-31T00:00:00Z".to_string()),
                superseded_at: None,
                created_at: "2026-08-31T00:00:00Z".to_string(),
                updated_at: "2026-08-31T00:00:00Z".to_string(),
            })
            .unwrap();
    }
}

// ============================================================================
// A12 —— existing/create 意图合同（REQ-C1-PLAN-01）。
// 模板与编译器定向测试（work_item_plan_compiler/tests/c1_existing_create.rs）
// 同源；串测聚焦三种用户可见结局：合法 create 过、未声明停等、不能执行拒绝。
// ============================================================================

const C1_SERIAL_TARGET_REPO: &str = "repo-c1";

const C1_SERIAL_CREATE_INTENT: &str = r#"
### Plan Intent
- intent: create
- provider_work_item_id: WI-002
- intent_target_kind: single_repository
- intent_target_repo: repo-c1
"#;

fn c1_serial_source(intent_section: &str, write_scopes: (&str, &str), depends_on: &str) -> String {
    format!(
        r#"# Work Item Plan

## Work Item WI-001: Levels API

### Identity
- schema_version: 1
- logical_work_item_id: WI-001
- title: Levels API
- kind: backend

### Goal
- summary: WHEN a levels request arrives THE SYSTEM SHALL return configured levels.

### Non Goals
- non_goals: Rendering is out of scope.

### Dependencies
- depends_on: {depends_on}
{intent_section}
### Inputs

### Outputs
- contract_id: contract.levels-api
- capabilities: api.levels.read

### Tasks
- task_id: TASK-001
- statement: WHEN the levels API starts THE SYSTEM SHALL serve configured levels.
- requirement_refs: REQ-C1-01
- done_when_refs: AC-001

### Write Policy
- exclusive_scopes: {exclusive}
- forbidden_scopes: {forbidden}

### Acceptance Criteria
- criterion_id: AC-001
- statement: WHEN GET /levels is requested THE SYSTEM SHALL return configured levels.
- required_evidence: source_diff

### Verification
- check_id: CHECK-001
- command: cargo test --locked --lib levels
- manual_instruction: Confirm the levels endpoint returns configured levels.
- required: true
- non_zero_test_execution_required: true

### Handoff Schema
- required_fields: commit_sha
- provided_contract_refs: contract.levels-api
- reviewer_check_refs: AC-001

### Blockers
- reason_code: levels_invalid
- route: plan_repair_current
- target_contract_refs: contract.levels-api

### Traceability
- source_type: design_spec
- source_id: design_c1_0001
- requirement_id: REQ-C1-01

## Work Item WI-002: Levels store

### Identity
- schema_version: 1
- logical_work_item_id: WI-002
- title: Levels store
- kind: backend

### Goal
- summary: WHEN levels are read THE SYSTEM SHALL return persisted rows.

### Non Goals
- non_goals: API surface is out of scope.

### Dependencies
- depends_on: []

### Plan Intent
- intent: create
- provider_work_item_id: WI-002
- intent_target_kind: single_repository
- intent_target_repo: {C1_SERIAL_TARGET_REPO}

### Inputs

### Outputs
- contract_id: contract.levels-store
- capabilities: store.levels.read

### Tasks
- task_id: TASK-002
- statement: WHEN the store loads THE SYSTEM SHALL expose persisted levels.
- requirement_refs: REQ-C1-02
- done_when_refs: AC-002

### Write Policy
- exclusive_scopes: src/levels_store/**
- forbidden_scopes: web/**

### Acceptance Criteria
- criterion_id: AC-002
- statement: WHEN the store is queried THE SYSTEM SHALL return persisted rows.
- required_evidence: source_diff

### Verification
- check_id: CHECK-002
- command: cargo test --locked --lib levels_store
- manual_instruction: Confirm the store returns persisted rows.
- required: true
- non_zero_test_execution_required: true

### Handoff Schema
- required_fields: commit_sha
- provided_contract_refs: contract.levels-store
- reviewer_check_refs: AC-002

### Blockers
- reason_code: levels_store_invalid
- route: plan_repair_current
- target_contract_refs: contract.levels-store

### Traceability
- source_type: design_spec
- source_id: design_c1_0001
- requirement_id: REQ-C1-02
"#,
        exclusive = write_scopes.0,
        forbidden = write_scopes.1,
    )
}

fn c1_serial_profile() -> RepositoryProfile {
    RepositoryProfile {
        id: "repository_profile_c1_serial".to_string(),
        project_id: "project_c1_serial".to_string(),
        issue_id: "issue_c1_serial".to_string(),
        repository_id: C1_SERIAL_TARGET_REPO.to_string(),
        logical_repository_id: None,
        membership_revision: 0,
        provider_run_ref: None,
        languages: vec!["rust".to_string()],
        frameworks: vec!["axum".to_string()],
        package_managers: vec!["cargo".to_string()],
        test_frameworks: vec!["cargo-test".to_string()],
        build_systems: vec!["cargo".to_string()],
        verification_capabilities: vec!["unit".to_string()],
        detected_layers: vec!["backend".to_string()],
        split_recommendation: "single_layer".to_string(),
        confidence: RepositoryProfileConfidence::High,
        uncertainties: Vec::new(),
        created_at: "2026-09-29T00:00:00Z".to_string(),
        updated_at: "2026-09-29T00:00:00Z".to_string(),
    }
}

/// 基线树只含既有写面；src/levels_new/** 完全在基线外＝新建写面。
fn c1_serial_baseline_tree() -> BTreeSet<String> {
    BTreeSet::from([
        "src/levels_store/mod.rs".to_string(),
        "web/src/app.ts".to_string(),
    ])
}

fn c1_serial_context<'a>(
    existing_ids: &'a [String],
    baseline: Option<&'a BTreeSet<String>>,
) -> PlanCandidateValidationContext<'a> {
    static BOUND_TARGET: std::sync::LazyLock<EnrollmentTarget> =
        std::sync::LazyLock::new(|| {
            EnrollmentTarget::SingleRepository {
                repository_id: C1_SERIAL_TARGET_REPO.to_string(),
            }
        });
    static PROFILE: std::sync::LazyLock<RepositoryProfile> =
        std::sync::LazyLock::new(c1_serial_profile);
    static OPTIONS: std::sync::LazyLock<IssueWorkItemPlanOptions> =
        std::sync::LazyLock::new(|| IssueWorkItemPlanOptions {
            include_integration_tests: false,
            include_e2e_tests: false,
            force_frontend_backend_split: false,
            require_execution_plan_confirm: false,
        });
    static STORY: std::sync::LazyLock<Vec<String>> =
        std::sync::LazyLock::new(|| vec!["story_spec_c1_0001".to_string()]);
    static DESIGN: std::sync::LazyLock<Vec<String>> =
        std::sync::LazyLock::new(|| vec!["design_spec_c1_0001".to_string()]);
    PlanCandidateValidationContext {
        project_id: "project_c1_serial",
        issue_id: "issue_c1_serial",
        plan_id: "plan_c1_serial",
        source_story_spec_ids: &STORY,
        source_design_spec_ids: &DESIGN,
        repository_profile: Some(&PROFILE),
        plan_options: &OPTIONS,
        baseline_tree: baseline,
        existing_work_item_ids: existing_ids,
        enrollment_target: Some(&BOUND_TARGET),
        now: "2026-09-29T00:00:00Z",
    }
}

fn c1_serial_compile(intent_section: &str) -> crate::product::work_item_plan_compiler::PlanCandidateIr {
    compile_work_item_plan(
        &c1_serial_source(intent_section, ("src/levels_new/**", "web/**"), "WI-002"),
        &WorkItemPlanSourceContext {
            target_repository_id: C1_SERIAL_TARGET_REPO.to_string(),
        },
    )
    .expect("plan must lower")
}

fn a12_existing_create_intent_contract_chain() {
    let baseline = c1_serial_baseline_tree();

    // 1. 合法 create：编译通过 + 意图随 canonical contract 持久化。
    let ir = c1_serial_compile(C1_SERIAL_CREATE_INTENT);
    let intent = ir.items[0]
        .contract
        .intent_contract
        .as_ref()
        .expect("intent contract lowered onto canonical contract");
    assert_eq!(
        intent.intent,
        crate::product::work_item_contract::WorkItemIntent::Create
    );
    assert_eq!(intent.provider_work_item_id, "WI-002");
    assert_eq!(intent.exclusive_scopes, vec!["src/levels_new/**".to_string()]);
    let report = validate_plan_candidate_ir(&ir, &c1_serial_context(&[], Some(&baseline)))
        .expect("declared create must validate");
    assert!(
        !report.has_errors(),
        "declared create findings must be Error-free: {:#?}",
        report.findings
    );

    // 2. 未声明：基线外新建写面 + 无 Plan Intent → intent_undeclared 停等
    //    （Error 保留在机械报告里等修订回灌，不产生 work item/child）。
    let ir = c1_serial_compile("");
    assert!(
        ir.items[0].contract.intent_contract.is_none(),
        "未声明意图时 contract 不得伪造 intent"
    );
    let diagnostics = validate_work_item_intent_contract(
        &ir,
        &c1_serial_context(&[], Some(&baseline)),
    )
    .expect_err("undeclared baseline writes must fail intent validation");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "intent_undeclared"),
        "expected intent_undeclared, got {diagnostics:#?}"
    );
    let report = validate_plan_candidate_ir(&ir, &c1_serial_context(&[], Some(&baseline)))
        .expect("preflight-family findings stay in the mechanical report");
    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "intent_undeclared"
                && finding.work_item_ids.contains(&"WI-001".to_string())),
        "report must carry intent_undeclared for WI-001: {:#?}",
        report.findings
    );

    // 3. 不能执行：错 provider Work Item → intent_unexecutable 拒绝（既有
    //    finding 不清空，不扩大 scope）。
    let ir = c1_serial_compile(
        r#"
### Plan Intent
- intent: create
- provider_work_item_id: WI-404
- intent_target_kind: single_repository
- intent_target_repo: repo-c1
"#,
    );
    let diagnostics = validate_work_item_intent_contract(
        &ir,
        &c1_serial_context(&[], Some(&baseline)),
    )
    .expect_err("unknown provider must be unexecutable");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "intent_unexecutable" && d.message.contains("WI-404")),
        "got {diagnostics:#?}"
    );
}

// ============================================================================
// A13 —— 显式换代（REQ-WIGA-01/03、REQ-C1-CHILD-01 的换代面）。
// 复用 confirmed_enrolled_fixture（真实 enrollment + Confirmed 链）：
// v1 唯一 attempt → 显式 rebind v2 → 旧代 advance/start 回执 fail-closed、
// 历史可查、无第二 attempt/provider；Manual 人工路径 replay 同一 attempt。
// compile child 跨代新建/旧 child 只读由 part_11 定向测试持有。
// ============================================================================

async fn a13_generation_switch_chain() {
    let fixture = crate::web::wiga_gate_fixture::confirmed_enrolled_fixture().await;
    let enrollment = fixture.enrollment();
    let plan_1 = enrollment.plan_id.clone().expect("bound plan");
    let binding_v1 = enrollment
        .binding_history
        .as_ref()
        .expect("versioned binding v1")
        .current
        .clone();
    assert_eq!(binding_v1.binding_version, 1);

    // v1 advance 成功：journal 冻结 v1 身份（唯一 attempt）。
    let v1_advance = crate::web::advance_plan::advance_plan(
        &fixture.state,
        AdvanceInput {
            command_id: format!("wiga-advance-{}-{plan_1}", enrollment.enrollment_id),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            plan_id: plan_1.clone(),
        },
        crate::web::advance_plan::AdvancePlanOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision,
        },
    )
    .await
    .expect("v1 advance under current binding");
    assert!(matches!(v1_advance, AdvanceOutcome::Completed { .. }));
    assert_eq!(fixture.coding_attempts().len(), 1);
    let v1_attempt_id = fixture.attempt().id.clone();

    // 显式换代 v2：新 plan/session 属于同一 issue。
    let (plan_2, session_2) = create_plan_and_session(&fixture.inner, RunPolicy::Interactive);
    let store = IssueAutomationStore::new(ProductAppPaths::new(
        fixture.state.workspace_root.join(".aria"),
    ));
    let rebind = store
        .rebind(
            &enrollment.project_id,
            &enrollment.issue_id,
            EnrollmentRebindRequest {
                command_id: "cmd-c1-serial-a13-rebind".to_string(),
                expected_policy_revision: enrollment.policy_revision,
                expected_binding_version: binding_v1.binding_version,
                binding: EnrollmentBindingIdentityInput {
                    plan_id: plan_2.clone(),
                    session_id: session_2,
                    source: enrollment.source.clone(),
                    target: binding_v1.target.clone(),
                    author_provider: enrollment.options.author_provider.clone(),
                    reviewer_provider: enrollment.options.reviewer_provider.clone(),
                },
                reason: "c1 serial generation switch".to_string(),
            },
        )
        .expect("rebind to v2");
    let after = rebind.enrollment;
    let history = after.binding_history.as_ref().expect("binding history");
    assert_eq!(history.current.binding_version, 2);
    // 旧 binding/历史可查：previous 保留完整 v1 身份。
    assert_eq!(history.previous.len(), 1);
    assert_eq!(history.previous[0].binding_version, 1);
    assert_eq!(history.previous[0].plan_id, plan_1);

    // 旧代 advance 回执 fail-closed：无第二 attempt。
    let v1_receipt = crate::web::advance_plan::advance_plan(
        &fixture.state,
        AdvanceInput {
            command_id: format!("wiga-advance-{}-{plan_1}", enrollment.enrollment_id),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            plan_id: plan_1.clone(),
        },
        crate::web::advance_plan::AdvancePlanOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision,
        },
    )
    .await
    .unwrap_err();
    assert!(
        v1_receipt.contains("mismatch"),
        "v1 advance receipt must fail closed on identity, got: {v1_receipt}"
    );
    assert_eq!(fixture.coding_attempts().len(), 1);

    // 旧代 start 回执 fail-closed：零 runner、不改当前代。
    let v1_start = crate::web::coding_start::start_coding_once(
        &fixture.state,
        &enrollment.project_id,
        &enrollment.issue_id,
        crate::web::coding_start::StartCodingCommand {
            attempt_id: v1_attempt_id,
            command_id: "cmd-c1-serial-a13-start-v1".to_string(),
            origin: crate::product::coding_models::CodingStartOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
                binding_version: Some(binding_v1.binding_version),
                target: Some(binding_v1.target.clone()),
            },
        },
    )
    .await;
    assert!(v1_start.is_err(), "v1 start receipt must fail closed");
    assert_eq!(fixture.coding_runner_count(), 0);
    assert_eq!(fixture.coding_attempts().len(), 1);

    // Manual 人工路径不被 enrollment 校验拦截：同代 durable replay 返回
    // 同一 attempt（不建第二链）。
    let manual = crate::web::advance_plan::advance_plan(
        &fixture.state,
        AdvanceInput {
            command_id: format!("wiga-advance-{}-{plan_1}", enrollment.enrollment_id),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            plan_id: plan_1.clone(),
        },
        crate::web::advance_plan::AdvancePlanOrigin::Manual,
    )
    .await
    .expect("manual advance must not be blocked by enrollment binding checks");
    assert!(matches!(manual, AdvanceOutcome::Replayed { .. }));
    assert_eq!(fixture.coding_attempts().len(), 1);
    assert_eq!(fixture.coding_runner_count(), 0);
}
