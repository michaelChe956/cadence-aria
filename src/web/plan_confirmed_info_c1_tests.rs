//! C1 Task 9 定向测试：`list_c1_waiting_items` 的 durable 等待项投影。

use crate::product::advance_store::{AdvanceRecord, AdvanceStatus, AdvanceStore};
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::json_store::write_json;
use crate::product::lifecycle_store::{
    CreateWorkspaceSessionInput, LifecycleStore, UpsertRepoSharedWorktreeInput,
};
use crate::product::logical_codebase::{EnrollmentTarget, LogicalRepositoryId};
use crate::product::models::automation::{
    EnrollmentBindingIdentityInput, EnrollmentOptions, EnrollmentRebindRequest,
    EnrollmentSource, EnrollmentWriteCommand,
};
use crate::product::models::lifecycle::{
    IssueWorkItemPlan, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus,
};
use crate::product::models::outline::{WorkItemPlanCompileStatus, WorkItemPlanCommitState};
use crate::product::models::provider::ProviderName;
use crate::product::models::{
    IssueWorkItemDependencyEdge, WorkItemSplitFinding, WorkItemSplitFindingSeverity,
    WorkItemPlanCompileTransaction, WorkspaceSessionRecord, WorkspaceType,
};
use crate::product::work_item_plan_policy::{CandidateSnapshotRecovery, HumanGateSnapshot};
use crate::product::work_item_plan_store::WorkItemPlanStore;
use crate::web::plan_confirmed_info::list_c1_waiting_items;

use std::collections::BTreeMap;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";

fn c1_target() -> EnrollmentTarget {
    EnrollmentTarget::SingleRepository {
        repository_id: "repo_physical_c1".to_string(),
    }
}

fn enrollment_source() -> EnrollmentSource {
    EnrollmentSource {
        stories: vec![],
        designs: vec![],
    }
}

fn enrollment_options() -> EnrollmentOptions {
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

fn fixture_root() -> (tempfile::TempDir, ProductAppPaths, LifecycleStore) {
    let tmp = tempfile::TempDir::new().unwrap();
    let app_paths = ProductAppPaths::new(tmp.path().join(".aria"));
    crate::product::project_store::ProjectStore::new(app_paths.clone())
        .create(crate::product::project_store::CreateProjectInput {
            name: "c1 waiting items fixture".to_string(),
            description: None,
        })
        .unwrap();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let now = chrono::Utc::now().to_rfc3339();
    write_json(
        &app_paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("issue.json"),
        &crate::product::models::IssueRecord {
            id: ISSUE_ID.to_string(),
            project_id: PROJECT_ID.to_string(),
            repo_id: None,
            logical_codebase_id: None,
            title: "C1 waiting items fixture".to_string(),
            description: None,
            change_id: "c1_waiting".to_string(),
            phase: crate::product::models::IssuePhase::Clarification,
            status: crate::product::models::IssueStatus::Draft,
            active_binding_id: None,
            created_at: now.clone(),
            updated_at: now,
            base_branch: None,
        },
    )
    .unwrap();
    (tmp, app_paths, lifecycle)
}

fn enable_enrollment(paths: &ProductAppPaths, plan_id: &str, session_id: &str) {
    enable_enrollment_only(paths);
    IssueAutomationStore::new(paths.clone())
        .bind_plan(PROJECT_ID, ISSUE_ID, 1, plan_id, session_id)
        .unwrap();
}

fn enable_enrollment_only(paths: &ProductAppPaths) {
    IssueAutomationStore::new(paths.clone())
        .compare_and_set(
            PROJECT_ID,
            ISSUE_ID,
            None,
            EnrollmentWriteCommand::Enable {
                selection_key: "c1_waiting_fixture".to_string(),
                source: enrollment_source(),
                options: enrollment_options(),
                target: c1_target(),
            },
        )
        .unwrap();
}

fn previous_plan_snapshot(plan_id: &str) -> IssueWorkItemPlan {
    IssueWorkItemPlan {
        id: plan_id.to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        source_story_spec_ids: vec![],
        source_design_spec_ids: vec![],
        options: IssueWorkItemPlanOptions {
            include_integration_tests: true,
            include_e2e_tests: false,
            force_frontend_backend_split: false,
            require_execution_plan_confirm: false,
        },
        status: IssueWorkItemPlanStatus::Draft,
        work_item_ids: vec![],
        repository_profile_ref: None,
        verification_plan_ids: vec![],
        dependency_graph: vec![IssueWorkItemDependencyEdge {
            from_work_item_id: "a".to_string(),
            to_work_item_id: "b".to_string(),
        }],
        created_from_provider_run: None,
        validator_findings: vec![],
        review_summary: None,
        created_at: "2026-09-29T00:00:00Z".to_string(),
        updated_at: "2026-09-29T00:00:00Z".to_string(),
    }
}

/// C1 Task 9：四类 durable waiting facts（孤儿候选、死租约、Failed advance、
/// intent 停等）＋换代历史必须完整投影身份（target/plan/session/attempt/
/// gate/副作用）与可用动作，供驾驶舱补读闭环。
#[test]
fn c1_waiting_items_include_identity_and_actions() {
    let (_tmp, paths, lifecycle) = fixture_root();
    let plan_id = "plan_0001";

    // 事实 1（A07）：plan session 上的孤儿候选快照（source revision 缺失）。
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
    let session_id = session.id.clone();
    // enrolled 链：binding v1 精确指向真实 plan/session。
    enable_enrollment(&paths, plan_id, &session_id);
    let mut durable: WorkspaceSessionRecord = lifecycle
        .get_workspace_session(&session.id)
        .unwrap();
    durable.human_gate_snapshot = Some(HumanGateSnapshot {
        findings: vec![],
        repeated_fingerprints: vec![],
        attempts_used: 0,
        manual_repairs_remaining: 2,
        accepted_feedback_turns: None,
        candidate_recovery: Some(CandidateSnapshotRecovery {
            complete: false,
            gate_id: "gate_0001".to_string(),
            source_revision_ref: None,
            source_revision_hash: None,
            plan_candidate_ir_ref: Some("ir_ref".to_string()),
            mechanical_report_ref: Some("report_ref".to_string()),
            budget_remaining: Some(3),
            missing: vec!["source_revision_missing".to_string()],
            completed_steps: vec!["candidate_source_persisted".to_string()],
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

    // 事实 2（A09）：Failed advance（外部副作用未知）。
    let advance_store = AdvanceStore::new(paths.clone());
    advance_store
        .put_record(&AdvanceRecord {
            id: "advance_0001".to_string(),
            command_id: "cmd_advance_0001".to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: plan_id.to_string(),
            plan_revision_id: "plan_rev_0001".to_string(),
            attempt_id: Some("attempt_0001".to_string()),
            target_attempts: vec![],
            status: AdvanceStatus::Failed,
            workspace_entry: None,
            error: Some("provider start outcome unknown".to_string()),
            created_at: "2026-09-29T00:00:00Z".to_string(),
            updated_at: "2026-09-29T00:00:00Z".to_string(),
        })
        .unwrap();

    // 事实 3（A12）：compile 事务携带 intent_undeclared finding（停等修订）。
    let plan_store = WorkItemPlanStore::new(paths.clone());
    plan_store
        .put_compile_transaction(&WorkItemPlanCompileTransaction {
            compile_id: "compile_c1_intent".to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: plan_id.to_string(),
            flow_kind: None,
            source_revision_id: None,
            source_revision_ref: None,
            plan_candidate_ir_ref: None,
            mechanical_report_ref: None,
            publication_provenance_ref: None,
            publication_provenance_content_hash: None,
            generation_round_id: "round_0001".to_string(),
            outline_version_ref: "outline_0001".to_string(),
            active_draft_ids: vec![],
            status: WorkItemPlanCompileStatus::Failed,
            plan_commit_state: WorkItemPlanCommitState::NotStarted,
            step_cursor: "validating".to_string(),
            outline_to_work_item_id: BTreeMap::new(),
            outline_to_verification_plan_id: BTreeMap::new(),
            created_work_item_ids: vec![],
            created_verification_plan_ids: vec![],
            child_session_ids: vec![],
            validator_findings: vec![WorkItemSplitFinding {
                severity: WorkItemSplitFindingSeverity::Error,
                code: "intent_undeclared".to_string(),
                message: "provider outputs paths outside baseline without create intent"
                    .to_string(),
                work_item_ids: vec![],
            }],
            abort_requested_at: None,
            failure_reason: Some("intent_undeclared".to_string()),
            previous_plan_snapshot: previous_plan_snapshot(plan_id),
            created_at: "2026-09-29T00:00:00Z".to_string(),
            updated_at: "2026-09-29T00:00:00Z".to_string(),
            committed_at: None,
        })
        .unwrap();

    // 事实 4（A09）：已释放租约（dead → 需确认接管）。
    lifecycle
        .upsert_repo_shared_worktree(UpsertRepoSharedWorktreeInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: LogicalRepositoryId(uuid::Uuid::nil()),
            branch_name: "c1-waiting".to_string(),
            worktree_path: std::path::PathBuf::from("/tmp/c1-waiting-worktree"),
            base_branch: "main".to_string(),
        })
        .unwrap();

    // 事实 5（A13）：显式换代 → 旧代 binding 历史可查。
    IssueAutomationStore::new(paths.clone())
        .rebind(
            PROJECT_ID,
            ISSUE_ID,
            EnrollmentRebindRequest {
                command_id: "cmd_rebind_v2".to_string(),
                expected_policy_revision: 2,
                expected_binding_version: 1,
                binding: EnrollmentBindingIdentityInput {
                    plan_id: plan_id.to_string(),
                    session_id: session_id.to_string(),
                    source: enrollment_source(),
                    target: c1_target(),
                    author_provider: ProviderName::Codex,
                    reviewer_provider: ProviderName::Fake,
                },
                reason: "c1 waiting fixture rebind".to_string(),
            },
        )
        .unwrap();

    let items = list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID).unwrap();
    assert!(
        !items.is_empty(),
        "durable waiting facts must project to inbox items"
    );

    let candidate = items
        .iter()
        .find(|item| item.kind == "candidate_recovery")
        .expect("orphan candidate snapshot must project");
    assert_eq!(candidate.reason, "candidate snapshot incomplete: source_revision_missing");
    assert_eq!(candidate.completed_steps, vec!["candidate_source_persisted".to_string()]);
    assert_eq!(candidate.gate_id.as_deref(), Some("gate_0001"));
    assert_eq!(candidate.session_id.as_deref(), Some(session.id.as_str()));
    assert_eq!(candidate.plan_id.as_deref(), Some(plan_id));
    assert_eq!(candidate.actions, vec!["recover_candidate".to_string()]);
    assert_eq!(candidate.target, Some(c1_target()));
    assert_eq!(candidate.next_phase.as_deref(), Some("candidate_recovered"));

    let lease = items
        .iter()
        .find(|item| item.kind == "lease_takeover")
        .expect("dead lease must project a takeover item");
    assert_eq!(lease.actions, vec!["confirm_takeover".to_string()]);
    assert!(
        lease.reason.contains("released") || lease.reason.contains("dead"),
        "dead lease reason must carry durable evidence: {lease:?}"
    );

    let advance = items
        .iter()
        .find(|item| item.kind == "advance_retry_failed")
        .expect("failed advance must project a retry item");
    assert_eq!(advance.attempt_id.as_deref(), Some("attempt_0001"));
    assert_eq!(
        advance.possible_side_effect.as_deref(),
        Some("provider start outcome unknown")
    );
    assert_eq!(advance.actions, vec!["retry_initialization".to_string()]);
    assert_eq!(advance.plan_id.as_deref(), Some(plan_id));

    let intent = items
        .iter()
        .find(|item| item.kind == "intent_blocked")
        .expect("intent undeclared compile failure must project");
    assert!(intent.reason.contains("intent_undeclared"));
    assert!(intent.actions.is_empty(), "intent stop waits for revision, no REST action");
    assert_eq!(intent.next_phase.as_deref(), Some("plan_revision"));

    let generation = items
        .iter()
        .find(|item| item.kind == "generation_history")
        .expect("previous binding generations must stay queryable");
    assert!(generation.reason.contains('1'), "history reason must count previous generations");
    assert_eq!(generation.actions, vec!["rebind".to_string()]);
}

/// 非 enrolled / 链路未开始（无 worktree 事实）不产生 C1 等待项——
/// 不以猜测的“最新 lease”制造噪声或越权操作面。
#[test]
fn c1_waiting_items_stay_silent_without_enrollment_or_lease_facts() {
    let (_tmp, paths, _lifecycle) = fixture_root();
    assert!(
        list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty(),
        "no enrollment → no C1 waiting items"
    );

    // enable 但尚未绑定 plan/session：无孤儿候选投影，也不制造 lease 噪声。
    enable_enrollment_only(&paths);
    let items = list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID).unwrap();
    assert!(
        items.iter().all(|item| item.kind != "lease_takeover" && item.kind != "lease_unknown"),
        "unstarted chain must not fabricate lease items: {items:?}"
    );
    assert!(
        items.is_empty(),
        "no plan binding → no waiting items at all: {items:?}"
    );
}
/// C2 Task 12（REQ-CRO-06）：十类 coding durable 等待事实必须经
/// `list_c1_waiting_items` additive 投影（kind 前缀 `coding_`／C2 常量），
/// 携带 attempt／gate／session 身份、expected 版本与 action_context
/// （稳定 command_id）；C1 既有七种 kind 与字段零变化（additive 缺省）。
/// 只读派生可重复补读（通知失败不回滚业务事实）。
#[test]
fn c2_waiting_items_project_coding_run_facts() {
    use crate::product::coding_attempt_store::{
        CodingAttemptCommandRecord, CreateBlockedGateInput, CreateCodingAttemptInput,
        EnterVerificationTriageInput,
    };
    use crate::product::coding_models::{
        CodingExecutionStage, CodingGateAction, CodingGateActionType, CodingProviderRole,
    };
    use crate::product::json_store::read_json;
    use crate::product::models::automation::OperationState;
    use crate::product::models::project::IssueSharedWorktreeStatus;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    let (_tmp, paths, lifecycle) = fixture_root();
    let plan_id = "plan_0001";
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
    let store = CodingAttemptStore::new(paths.clone());
    let snapshot = ProviderConfigSnapshot {
        author: ProviderName::Fake,
        reviewer: None,
        review_rounds: 0,
        permission_modes: Default::default(),
    };
    let create = |work_item_id: &str, branch: &str| {
        store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: work_item_id.to_string(),
                base_branch: "main".to_string(),
                branch_name: branch.to_string(),
                worktree_path: None,
                provider_config_snapshot: snapshot.clone(),
                target_snapshot: None,
                max_auto_rework: 0,
            })
            .unwrap()
    };

    // 事实 1（restart 可用）：终态 Aborted attempt（版本已推进到 3）。
    let attempt_a = create("work_item_0001", "aria/c2-restart");
    let mut aborted = attempt_a.clone();
    aborted.status = crate::product::coding_models::CodingAttemptStatus::Aborted;
    aborted.version = 3;
    store.write_coding_attempt_for_test(&aborted).unwrap();

    // 事实 2（完成状态待确认）：AwaitingManualRecovery＋manual_recovery_reason。
    let attempt_b = create("work_item_0002", "aria/c2-unconfirmed");
    let mut unconfirmed = attempt_b.clone();
    unconfirmed.status = crate::product::coding_models::CodingAttemptStatus::AwaitingManualRecovery;
    unconfirmed.manual_recovery_reason = Some("runner_died_before_provider_start".to_string());
    unconfirmed.version = 7;
    store.write_coding_attempt_for_test(&unconfirmed).unwrap();

    // 事实 3（admission 停等账本）：attempt_b 被 Task 2 互斥拦下（NeedsHuman）。
    store
        .append_attempt_command_result(
            PROJECT_ID,
            ISSUE_ID,
            &attempt_b.id,
            &CodingAttemptCommandRecord {
                command_id: "cmd-c2-kick-0001".to_string(),
                payload_digest: "kick|coding".to_string(),
                state: OperationState::NeedsHuman,
                recorded_at: "2026-09-29T00:00:00Z".to_string(),
            },
        )
        .unwrap();

    // 事实 4（政策核验等待事实）：attempt_b 分区 policy-verification.json。
    let policy_fact = serde_json::json!({
        "attempt_id": attempt_b.id,
        "reason_code": "policy_resolver_unavailable",
        "detail": "resolver cannot uniquely resolve the frozen policy envelope",
        "policy_digest": null,
        "created_at": "2026-09-29T00:00:00Z",
    });
    write_json(
        &paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join(&attempt_b.id)
            .join("policy-verification.json"),
        &policy_fact,
    )
    .unwrap();

    // 事实 5（验证处理未决）：store 级转入（finding 沿 report_id#index 约定）。
    store
        .enter_verification_triage(
            &unconfirmed,
            EnterVerificationTriageInput {
                attempt_id: attempt_b.id.clone(),
                finding_id: "code_review_report_0001#2".to_string(),
                check_id: "check_run_tests".to_string(),
                plan_revision_id: "plan_rev_0001".to_string(),
                original_command: Some("pnpm -C web exec vitest run".to_string()),
                alternative_command: None,
                cwd: None,
                outcome: None,
                test_execution_count: None,
                environment: None,
                scope: vec!["check_run_tests".to_string()],
                expires_at: "2026-10-06T00:00:00Z".to_string(),
            },
        )
        .unwrap();

    // 事实 6（指令消费中断，C-1b 真实对账驱动）：认领事务真实落 journal
    //（消费标记已齐、node 已绑定）而其 node 无任何 ProviderPrompt →
    // 对账判中断、落 instruction-claim-interrupted 等待事实（消费标记后、
    // spawn 前中断；重驱以 claim.instruction_ids 强制入渲染）。
    let interrupted_instruction = crate::product::coding_models::CodingReworkInstruction {
        id: "coding_rework_instruction_0001".to_string(),
        attempt_id: attempt_b.id.clone(),
        source_stage: CodingExecutionStage::CodeReview,
        rework_round: 1,
        summary: "中断认领的返修指令".to_string(),
        fix_hints: vec!["按 finding 修复 src/lib.rs".to_string()],
        questions: Vec::new(),
        created_at: "2026-09-29T00:00:00Z".to_string(),
        consumed_by_node_id: None,
        consumed_at: None,
    };
    store
        .save_rework_instruction(&attempt_b, &interrupted_instruction)
        .unwrap();
    store
        .claim_and_consume_rework_instructions(
            &attempt_b,
            "coding_node_0009",
            1,
            "上一轮完整 prompt（渲染后中断，未发出）",
            None,
            &["coding_rework_instruction_0001".to_string()],
        )
        .unwrap();
    let interrupted_renders = store
        .reconcile_interrupted_rework_claims(PROJECT_ID, ISSUE_ID, &attempt_b.id)
        .unwrap();
    assert_eq!(
        interrupted_renders.len(),
        1,
        "对账判中断并返回强制回放指令：{interrupted_renders:?}"
    );
    let interrupted_claim_id = interrupted_renders[0].claim.claim_id.clone();

    // 事实 7（reviewer 配置缺失）：reason_code 定格的开放 blocked gate。
    store
        .create_blocked_gate_with_diagnostic(
            &unconfirmed,
            CreateBlockedGateInput {
                attempt_id: attempt_b.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: None,
                role: Some(CodingProviderRole::CodeReviewer),
                title: "reviewer 未配置".to_string(),
                description: "reviewer provider is missing; pick a reviewer and retry"
                    .to_string(),
                reason_code: Some("reviewer_configuration_missing".to_string()),
                evidence_refs: vec![],
                raw_provider_output_ref: None,
                available_actions: vec![CodingGateAction {
                    action_id: "retry_review".to_string(),
                    label: "重试代码审查".to_string(),
                    action_type: CodingGateActionType::RetryReview,
                }],
            },
            None,
        )
        .unwrap();

    // 事实 8（大候选停等）：绑定 plan session 分区 sc-revision-blocked.json。
    write_json(
        &paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("workspace-sessions")
            .join(&session.id)
            .join("sc-revision-blocked.json"),
        &serde_json::json!({
            "session_id": session.id,
            "command_id": "cmd-sc-revision-0001",
            "reason_code": "HUMAN_GATE_REVISION_INPUT_OVER_HARD_LIMIT",
            "detail": "assembled revision input 48000 bytes exceeds hard limit 32000",
            "total_bytes": 48_000,
            "hard_limit_bytes": 32_000,
            "actions": ["segmented_revision", "retry"],
            "created_at": "2026-09-29T00:00:00Z",
        }),
    )
    .unwrap();

    // 事实 9（已在运行）：活跃 attempt_c 持有 issue worktree 锁。
    let attempt_c = create("work_item_0003", "aria/c2-running");
    let mut running = attempt_c.clone();
    running.status = crate::product::coding_models::CodingAttemptStatus::Running;
    running.admission_ticket_consumed_at = Some("2026-09-29T00:00:00Z".to_string());
    running.version = 11;
    store.write_coding_attempt_for_test(&running).unwrap();
    let worktree_path = paths
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("issue-shared-worktree.json");
    let lock_worktree = |owner: &str, active_item: Option<&str>| {
        write_json(
            &worktree_path,
            &crate::product::models::lifecycle::IssueSharedWorktree {
                id: "issue_shared_worktree_0001".to_string(),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: "repo_physical_c1".to_string(),
                target_repository_id: None,
                checkout_id: None,
                path_schema_version: 0,
                branch_name: "aria/issue-shared".to_string(),
                worktree_path: std::path::PathBuf::from("/tmp/c2-shared-worktree"),
                base_branch: "main".to_string(),
                status: IssueSharedWorktreeStatus::Running,
                current_active_work_item_id: active_item.map(str::to_string),
                current_lock_owner_id: Some(owner.to_string()),
                last_completed_work_item_id: None,
                created_at: "2026-09-29T00:00:00Z".to_string(),
                updated_at: "2026-09-29T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    };
    lock_worktree(&attempt_c.id, Some("work_item_0003"));

    let items = list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID).unwrap();
    let find = |kind: &str| {
        items
            .iter()
            .find(|item| item.kind == kind)
            .unwrap_or_else(|| panic!("{kind} must project from durable facts: {items:?}"))
    };

    // 完成状态待确认：AwaitingManualRecovery＋诊断 reason；无 REST 动作
    //（恢复操作在 coding workspace 的显式 RecoverCoding 面）。
    let completion = find("coding_completion_unconfirmed");
    assert_eq!(completion.attempt_id.as_deref(), Some(attempt_b.id.as_str()));
    assert!(
        completion.reason.contains("runner_died_before_provider_start"),
        "completion reason carries the durable diagnostic: {completion:?}"
    );
    assert!(completion.actions.is_empty());
    assert_eq!(completion.expected_version, Some(7));
    assert!(completion.action_context.is_empty());

    // restart 可用：expected 版本＋action_context（稳定 command_id）。
    let restart = find("coding_restart_available");
    assert_eq!(restart.attempt_id.as_deref(), Some(attempt_a.id.as_str()));
    assert_eq!(restart.actions, vec!["restart_coding".to_string()]);
    assert_eq!(restart.expected_version, Some(3));
    assert_eq!(restart.action_context.len(), 1);
    assert_eq!(restart.action_context[0].action, "restart_coding");
    assert_eq!(restart.action_context[0].expected_version, 3);
    assert!(!restart.action_context[0].command_id.is_empty());

    // reviewer 配置缺失：gate 身份＋gate 动作经 action_context 携带版本。
    let reviewer = find("reviewer_configuration_missing");
    assert_eq!(reviewer.attempt_id.as_deref(), Some(attempt_b.id.as_str()));
    assert!(reviewer.gate_id.is_some());
    assert_eq!(reviewer.actions, vec!["retry_review".to_string()]);
    assert_eq!(reviewer.expected_version, Some(7));
    assert_eq!(reviewer.action_context[0].action, "retry_review");
    assert_eq!(reviewer.action_context[0].expected_version, 7);

    // 验证处理：未决记录投影（finding/report#index、check 绑定入 reason）。
    let triage = find("verification_triage");
    assert_eq!(triage.attempt_id.as_deref(), Some(attempt_b.id.as_str()));
    assert!(triage.reason.contains("code_review_report_0001#2"));
    assert!(triage.reason.contains("check_run_tests"));
    assert!(triage.actions.is_empty());

    // 政策核验：reason_code＋detail 直达（停等呈现，动作面在重新授权链）。
    let policy = find("policy_verification");
    assert_eq!(policy.attempt_id.as_deref(), Some(attempt_b.id.as_str()));
    assert!(policy.reason.contains("policy_resolver_unavailable"));
    assert!(policy.actions.is_empty());

    // 指令消费中断：认领身份与强制回放语义入 reason（真实对账落账的事实）。
    let claim = find("instruction_claim_interrupted");
    assert_eq!(claim.attempt_id.as_deref(), Some(attempt_b.id.as_str()));
    assert!(claim.reason.contains(&interrupted_claim_id));
    assert!(
        claim.reason.contains("coding_rework_instruction_0001"),
        "instruction ids surface in reason: {claim:?}"
    );
    assert!(claim.actions.is_empty());

    // 大候选停等：session 身份＋预算事实入 reason。
    let large = find("large_candidate_blocked");
    assert_eq!(large.session_id.as_deref(), Some(session.id.as_str()));
    assert!(large.reason.contains("48000"));
    assert!(large.reason.contains("HUMAN_GATE_REVISION_INPUT_OVER_HARD_LIMIT"));

    // 已在运行：活跃持有者身份；无抢占动作。
    let already = find("coding_already_running");
    assert_eq!(already.attempt_id.as_deref(), Some(attempt_c.id.as_str()));
    assert!(already.actions.is_empty());
    assert_eq!(already.next_phase.as_deref(), Some("coding_run_settles"));

    // C1 既有投影零变化：lease_wait 仍在且不携带 C2 additive 字段。
    let lease_wait = items
        .iter()
        .find(|item| item.kind == "lease_wait")
        .expect("C1 lease_wait stays unchanged");
    assert_eq!(lease_wait.expected_version, None);
    assert!(lease_wait.action_context.is_empty());

    // 阶段 2（确认接管）：持有者终态 → coding_takeover_required＋confirm_takeover。
    let mut dead = running.clone();
    dead.status = crate::product::coding_models::CodingAttemptStatus::Aborted;
    store.write_coding_attempt_for_test(&dead).unwrap();
    let items = list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID).unwrap();
    let takeover = items
        .iter()
        .find(|item| item.kind == "coding_takeover_required")
        .unwrap_or_else(|| panic!("dead foreign holder must project takeover: {items:?}"));
    assert_eq!(takeover.attempt_id.as_deref(), Some(attempt_c.id.as_str()));
    assert_eq!(takeover.actions, vec!["confirm_takeover".to_string()]);
    assert_eq!(takeover.expected_version, Some(11));
    assert_eq!(takeover.action_context[0].action, "confirm_takeover");
    assert_eq!(takeover.action_context[0].expected_version, 11);
    assert!(
        !items.iter().any(|item| item.kind == "coding_already_running"),
        "dead holder no longer projects already_running"
    );

    // 阶段 3（活性未知）：owner 无法解析为 issue attempt → 停等。
    lock_worktree("coding_attempt_ghost_9999", Some("work_item_0003"));
    let items = list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID).unwrap();
    let unknown = items
        .iter()
        .find(|item| item.kind == "coding_lease_unknown")
        .unwrap_or_else(|| panic!("unresolvable owner must project lease_unknown: {items:?}"));
    assert!(unknown.actions.is_empty());
    assert!(
        !items.iter().any(|item| item.kind == "coding_takeover_required"),
        "unknown holder no longer projects takeover"
    );

    // 只读补读幂等：同事实再次 GET 得到同形条目（通知失败不回滚业务事实）。
    let reread = list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID).unwrap();
    assert_eq!(
        reread
            .iter()
            .find(|item| item.kind == "coding_lease_unknown")
            .map(|item| (item.id.clone(), item.reason.clone())),
        Some((unknown.id.clone(), unknown.reason.clone())),
    );

    // journal 落盘事实未被投影读取改动（只读派生）；真实认领的 consumed_at
    // 保持 Some（对账不重写 claim、不二次消费——等待事实在独立分区）。
    let raw: serde_json::Value = read_json(
        &paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join(&attempt_b.id)
            .join("rework-claims.json"),
    )
    .unwrap();
    assert_eq!(
        raw["records"][0]["claim_id"].as_str(),
        Some(interrupted_claim_id.as_str())
    );
    assert!(
        raw["records"][0]["consumed_at"].as_str().is_some(),
        "reconciliation must not rewrite the claim journal"
    );
}
