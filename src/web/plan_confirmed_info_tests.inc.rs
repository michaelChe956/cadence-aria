use super::*;
use crate::web::wiga_gate_fixture::EnrolledGateFixture;
use crate::web::wiga_gate_fixture::{ISSUE_ID, PROJECT_ID};

struct CompiledPlanFixture {
    gate: EnrolledGateFixture,
}

impl CompiledPlanFixture {
    async fn new() -> Self {
        Self {
            gate: EnrolledGateFixture::new().await,
        }
    }

    fn bound_enrollment(&self) -> IssueAutomationEnrollment {
        IssueAutomationStore::new(self.gate.inner.paths.clone())
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap()
    }

    fn paths(&self) -> &ProductAppPaths {
        &self.gate.inner.paths
    }

    async fn approve_and_persist_failed_compile(&mut self) {
        self.gate.fail_compile_after_human_approve().await;
    }

    /// 人工 CompileRecovery Continue + Approve 关门（fixture 共享面）。
    async fn recover_and_commit_compile(&mut self) {
        self.gate.recover_and_confirm_compile().await;
    }
}

#[tokio::test]
async fn plan_confirmed_info_requires_published_compile_and_confirmed_session() {
    let mut fixture = Box::new(CompiledPlanFixture::new().await);
    let enrollment = fixture.bound_enrollment();
    assert!(
        plan_confirmed_info(fixture.paths(), &enrollment)
            .unwrap()
            .is_none(),
        "未编译/未确认不得产生成功信息"
    );
    fixture.approve_and_persist_failed_compile().await;
    assert!(
        plan_confirmed_info(fixture.paths(), &enrollment)
            .unwrap()
            .is_none(),
        "failpoint 失败 compile（门保持开启）不得产生成功信息"
    );
    fixture.recover_and_commit_compile().await;
    let first = plan_confirmed_info(fixture.paths(), &enrollment)
        .unwrap()
        .expect("恢复并确认后必须有且仅有一条成功信息");
    let second = plan_confirmed_info(fixture.paths(), &fixture.bound_enrollment())
        .unwrap()
        .expect("重复读取幂等");
    assert_eq!(first.key, second.key);
    assert_eq!(first.occurred_at, second.occurred_at);
    assert!(first.key.contains(&first.plan_id));
    assert!(first.key.contains("plan_confirmed:"));
    assert_eq!(first.session_id, fixture.gate.session_id);
    assert_eq!(first.title, PLAN_CONFIRMED_INFO_TITLE);
    assert!(
        !first.occurred_at.is_empty(),
        "occurred_at 取 tx.committed_at"
    );
}

/// 禁用 enrollment 后历史成功不投影为当前信息（禁用不是假成功）。
#[tokio::test]
async fn plan_confirmed_info_hidden_after_enrollment_disabled() {
    let mut fixture = Box::new(CompiledPlanFixture::new().await);
    fixture.approve_and_persist_failed_compile().await;
    fixture.recover_and_commit_compile().await;
    let enrollment = fixture.bound_enrollment();
    assert!(
        plan_confirmed_info(fixture.paths(), &enrollment)
            .unwrap()
            .is_some()
    );
    let disabled = IssueAutomationEnrollment {
        enabled: false,
        ..enrollment
    };
    assert!(
        plan_confirmed_info(fixture.paths(), &disabled)
            .unwrap()
            .is_none(),
        "禁用后不得把历史成功当当前信息"
    );
}

// ── F-A（owner 2026-10-25 裁定，cadence/notes/2026-10-25_UI复验发现）：
//    等待项可操作性分类——lease_wait 归自动等待，generation_history 按
//    当前绑定有效性分流。──

/// F-A 夹具：project/issue + enabled enrollment 绑定 plan/session 并
/// rebind 一次制造 `history.previous`；`with_plan_record` 控制当前绑定
/// 指向的 plan 记录是否落盘（false = 悬空绑定，如单 plan 级联删除后）。
fn f_a_waiting_fixture(
    with_plan_record: bool,
) -> (tempfile::TempDir, ProductAppPaths, String, String) {
    use crate::product::issue_automation_store::IssueAutomationStore;
    use crate::product::json_store::write_json;
    use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
    use crate::product::logical_codebase::EnrollmentTarget;
    use crate::product::models::automation::{
        EnrollmentBindingIdentityInput, EnrollmentOptions, EnrollmentRebindRequest,
        EnrollmentSource, EnrollmentWriteCommand, PlanApprovalPolicy,
    };
    use crate::product::models::lifecycle::{
        IssueWorkItemPlan, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus,
    };
    use crate::product::models::provider::ProviderName;
    use crate::product::models::{IssueRecord, IssueWorkItemDependencyEdge, WorkspaceType};
    use crate::product::project_store::{CreateProjectInput, ProjectStore};

    let tmp = tempfile::TempDir::new().unwrap();
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "f-a waiting fixture".to_string(),
            description: None,
        })
        .unwrap();
    let lifecycle = LifecycleStore::new(paths.clone());
    let now = chrono::Utc::now().to_rfc3339();
    write_json(
        &paths.issue_root(PROJECT_ID, ISSUE_ID).join("issue.json"),
        &IssueRecord {
            id: ISSUE_ID.to_string(),
            project_id: PROJECT_ID.to_string(),
            repo_id: None,
            logical_codebase_id: None,
            title: "f-a waiting fixture".to_string(),
            description: None,
            change_id: "f_a_waiting".to_string(),
            phase: crate::product::models::IssuePhase::Clarification,
            status: crate::product::models::IssueStatus::Draft,
            active_binding_id: None,
            created_at: now.clone(),
            updated_at: now,
            base_branch: None,
        },
    )
    .unwrap();

    let plan_id = "plan_fa_0001".to_string();
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: plan_id.clone(),
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

    let target = EnrollmentTarget::SingleRepository {
        repository_id: "repo_physical_fa".to_string(),
    };
    let source = EnrollmentSource {
        stories: vec![],
        designs: vec![],
    };
    let options = EnrollmentOptions {
        author_provider: ProviderName::Fake,
        reviewer_provider: ProviderName::Fake,
        review_rounds: 1,
        superpowers_enabled: false,
        openspec_enabled: false,
        plan_approval: PlanApprovalPolicy::Manual,
        plan_options: IssueWorkItemPlanOptions {
            include_integration_tests: true,
            include_e2e_tests: false,
            force_frontend_backend_split: false,
            require_execution_plan_confirm: false,
        },
    };
    let automation = IssueAutomationStore::new(paths.clone());
    automation
        .compare_and_set(
            PROJECT_ID,
            ISSUE_ID,
            None,
            EnrollmentWriteCommand::Enable {
                command_id: None,
                selection_key: "f_a_waiting_fixture".to_string(),
                source: source.clone(),
                options: options.clone(),
                target: target.clone(),
            },
        )
        .unwrap();
    automation
        .bind_plan(PROJECT_ID, ISSUE_ID, 1, &plan_id, &session_id)
        .unwrap();
    // rebind 一次：同 plan/session 换代，history.previous 恰有一条。
    automation
        .rebind(
            PROJECT_ID,
            ISSUE_ID,
            EnrollmentRebindRequest {
                command_id: "cmd-f-a-rebind-v2".to_string(),
                expected_policy_revision: 2,
                expected_binding_version: 1,
                binding: EnrollmentBindingIdentityInput {
                    plan_id: plan_id.clone(),
                    session_id: session_id.clone(),
                    source,
                    target,
                    author_provider: ProviderName::Codex,
                    reviewer_provider: ProviderName::Fake,
                },
                reason: "f-a fixture rebind".to_string(),
            },
        )
        .unwrap();

    if with_plan_record {
        write_json(
            &paths
                .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                .join("issue-work-item-plans")
                .join(format!("{plan_id}.json")),
            &IssueWorkItemPlan {
                id: plan_id.clone(),
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
                ever_confirmed: None,
                created_at: "2026-10-25T00:00:00Z".to_string(),
                updated_at: "2026-10-25T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    }
    (tmp, paths, plan_id, session_id)
}

/// F-A：当前绑定健在（plan 记录与 session 均存在）时，换代历史降级为
/// 只读信息——无 rebind 动作、requires_human=false（不进「需要人工
/// 处理」与待处理计数）。
#[test]
fn f_a_generation_history_demotes_to_read_only_info_while_binding_live() {
    let (_tmp, paths, plan_id, session_id) = f_a_waiting_fixture(true);
    let items = list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID).unwrap();
    let generation = items
        .iter()
        .find(|item| item.kind == "generation_history")
        .expect("generation history stays queryable");
    assert!(
        generation.reason.contains("current binding active"),
        "reason must state the live binding: {generation:?}"
    );
    assert!(
        generation.actions.is_empty(),
        "live binding must not offer rebind: {generation:?}"
    );
    assert_eq!(generation.next_phase, None);
    assert!(!generation.requires_human);
    assert_eq!(generation.plan_id.as_deref(), Some(plan_id.as_str()));
    assert_eq!(generation.session_id.as_deref(), Some(session_id.as_str()));
}

/// F-A：当前绑定失效（plan 记录缺失，如单 plan 级联删除）时，换代历史
/// 保持换代待办——rebind 动作 + requires_human=true。
#[test]
fn f_a_generation_history_stays_actionable_when_binding_dangling() {
    let (_tmp, paths, _plan_id, _session_id) = f_a_waiting_fixture(false);
    let items = list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID).unwrap();
    let generation = items
        .iter()
        .find(|item| item.kind == "generation_history")
        .expect("generation history stays queryable");
    assert!(
        generation
            .reason
            .contains("current binding dangling (plan)"),
        "reason must name the dangling leg: {generation:?}"
    );
    assert_eq!(generation.actions, vec!["rebind".to_string()]);
    assert_eq!(generation.next_phase.as_deref(), Some("rebind"));
    assert!(generation.requires_human);
}

/// F-A：租约活跃（自动链到期自动恢复）是自动等待——requires_human=false，
/// 无动作；死租约的接管项不在此夹具出现（见 c1 测试既有覆盖）。
#[test]
fn f_a_lease_wait_is_auto_waiting_not_needs_human() {
    use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
    use crate::product::coding_models::CodingAttemptStatus;
    use crate::product::json_store::write_json;
    use crate::product::models::lifecycle::IssueSharedWorktree;
    use crate::product::models::project::IssueSharedWorktreeStatus;
    use crate::product::models::provider::ProviderName;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    let (_tmp, paths, _plan_id, _session_id) = f_a_waiting_fixture(true);
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: "work_item_fa_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/fa-running".to_string(),
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
    let mut running = attempt.clone();
    running.status = CodingAttemptStatus::Running;
    store.write_coding_attempt_for_test(&running).unwrap();
    write_json(
        &paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("issue-shared-worktree.json"),
        &IssueSharedWorktree {
            id: "issue_shared_worktree_fa".to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: "repo_physical_fa".to_string(),
            target_repository_id: None,
            checkout_id: None,
            path_schema_version: 0,
            branch_name: "aria/issue-shared-fa".to_string(),
            worktree_path: std::path::PathBuf::from("/tmp/fa-shared-worktree"),
            base_branch: "main".to_string(),
            status: IssueSharedWorktreeStatus::Running,
            current_active_work_item_id: Some("work_item_fa_0001".to_string()),
            current_lock_owner_id: Some(running.id.clone()),
            last_completed_work_item_id: None,
            created_at: "2026-10-25T00:00:00Z".to_string(),
            updated_at: "2026-10-25T00:00:00Z".to_string(),
        },
    )
    .unwrap();

    let items = list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID).unwrap();
    let lease_wait = items
        .iter()
        .find(|item| item.kind == "lease_wait")
        .expect("active lease must project lease_wait");
    assert!(
        lease_wait.reason.contains("lease active"),
        "reason must carry the holder lease: {lease_wait:?}"
    );
    assert!(lease_wait.actions.is_empty());
    assert!(!lease_wait.requires_human);
    assert!(
        !items
            .iter()
            .any(|item| item.kind == "lease_takeover" || item.kind == "lease_unknown"),
        "active lease must not project takeover/unknown noise: {items:?}"
    );
}

/// F-A additive 契约：旧响应缺 `requires_human` 字段按需要人工解释
/// （fail-safe 不静默降级真待办）；新响应显式携带该字段。
#[test]
fn f_a_requires_human_defaults_true_for_old_responses() {
    let old_response = r#"{
            "id": "c1:lease_wait:issue_0001:lease_0001",
            "kind": "lease_wait",
            "reason": "lease active; automation keeps waiting",
            "completed_steps": [],
            "target": null,
            "plan_id": null,
            "session_id": null,
            "attempt_id": null,
            "gate_id": null,
            "possible_side_effect": null,
            "actions": [],
            "next_phase": null
        }"#;
    let parsed: C1WaitingItemDto = serde_json::from_str(old_response).unwrap();
    assert!(parsed.requires_human, "旧响应缺省按需要人工解释");
    let serialized = serde_json::to_value(&parsed).unwrap();
    assert_eq!(serialized["requires_human"], serde_json::json!(true));
}
