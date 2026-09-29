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
                logical_repository_id: LogicalRepositoryId(uuid::Uuid::nil()),
                target: Some(c1_target()),
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
