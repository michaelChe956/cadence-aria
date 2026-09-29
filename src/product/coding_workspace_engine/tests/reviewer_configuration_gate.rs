use super::*;

// C2 Task 5（REQ-CRO-05）：reviewer 三值——code/internal reviewer 缺失时，在
// 建 role run／timeline node 之前被拦：落 reason_code `reviewer_configuration_missing`
// 的 blocked gate（重试＋终止动作），attempt 转 Blocked，绝不以 author 顶替。

/// 断言用桩：任何 start 都视为违约（reviewer 缺失时 provider 不得启动）。
struct NeverStartedProvider {
    starts: std::sync::atomic::AtomicUsize,
}

impl NeverStartedProvider {
    fn default() -> Self {
        Self {
            starts: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn starts(&self) -> usize {
        self.starts.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for NeverStartedProvider {
    async fn start(
        &self,
        _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<crate::cross_cutting::streaming_provider::ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
        self.starts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(
            crate::cross_cutting::provider_adapter::ProviderAdapterError::provider_unavailable(
                "never_started_provider".to_string(),
            ),
        )
    }
}

fn running_attempt_with_reviewer(
    reviewer: Option<ProviderName>,
) -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
) {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id: None,
            title: "issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("issue");
    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer,
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("running attempt");
    (root, store, attempt)
}

fn running_group_attempt_with_reviewer(
    reviewer: Option<ProviderName>,
) -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
) {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id: None,
            title: "issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("issue");
    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_group_attempt(crate::product::coding_attempt_store::CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/groups/plan_0001/attempt-1".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer,
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        })
        .expect("create group attempt");
    let attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("running group attempt");
    // group attempt 的 admission lineage 校验需要 plan lineage 在场。
    let revision_store = crate::product::work_item_revision_store::WorkItemRevisionStore::new(
        store.paths(),
    );
    let lineage = crate::product::models::WorkItemPlanLineage {
        id: "plan_0001".to_string(),
        project_id: attempt.project_id.clone(),
        issue_id: attempt.issue_id.clone(),
        story_spec_refs: Vec::new(),
        design_spec_refs: Vec::new(),
        active_revision_id: None,
        active_amendment_id: None,
        created_at: "2026-09-29T00:00:00Z".to_string(),
        updated_at: "2026-09-29T00:00:00Z".to_string(),
    };
    revision_store
        .put_plan_lineage(&lineage)
        .expect("plan lineage");
    (root, store, attempt)
}

#[tokio::test]
async fn missing_reviewer_lands_configuration_gate_without_author_fallback() {
    let (_root, store, attempt) = running_attempt_with_reviewer(None);
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = NeverStartedProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("missing reviewer must land the configuration gate");

    assert!(
        matches!(
            error,
            CodingWorkspaceEngineError::ReviewerConfigurationMissing { ref attempt_id, .. }
                if attempt_id == &attempt.id
        ),
        "expected ReviewerConfigurationMissing, got: {error:?}"
    );
    assert_eq!(provider.starts(), 0, "provider must not start");

    // 快照保持空 effective：coder=author 是 coder 自身语义，reviewer 绝不被 author 顶替。
    let snapshot = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role snapshot");
    assert_eq!(snapshot.coder, ProviderName::Codex);
    assert_eq!(snapshot.code_reviewer, None);
    assert_eq!(snapshot.internal_reviewer, None);
    let reviewer_config = snapshot.reviewer_config();
    assert_eq!(reviewer_config.effective, None);
    assert!(!reviewer_config.enabled);

    // 未建 role run／CodeReview timeline node。
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs");
    assert!(runs.is_empty(), "no role run may be created before the gate");
    let nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline nodes");
    assert!(
        nodes
            .iter()
            .all(|node| node.stage != CodingExecutionStage::CodeReview),
        "no CodeReview timeline node may be created before the gate"
    );

    // attempt 转 Blocked，落唯一 open gate：reason_code＋重试/终止动作。
    let current = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(current.status, CodingAttemptStatus::Blocked);
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates");
    assert_eq!(gates.len(), 1, "exactly one configuration gate");
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("reviewer_configuration_missing")
    );
    let action_ids: Vec<&str> = gates[0]
        .available_actions
        .iter()
        .map(|action| action.action_id.as_str())
        .collect();
    assert!(
        action_ids.contains(&"retry_review"),
        "actions: {action_ids:?}"
    );
    assert!(action_ids.contains(&"abort"), "actions: {action_ids:?}");

    // 重复进入同阶段：幂等命中同一 gate，不堆积第二条，也不启动 provider。
    let again = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("missing reviewer keeps failing closed");
    assert!(matches!(
        again,
        CodingWorkspaceEngineError::ReviewerConfigurationMissing { .. }
    ));
    assert_eq!(provider.starts(), 0);
    let gates_after = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates after retry");
    assert_eq!(gates_after.len(), 1, "gate creation is idempotent");
    assert_eq!(gates_after[0].gate_id, gates[0].gate_id);
}

#[tokio::test]
async fn missing_internal_reviewer_lands_configuration_gate_before_role_run() {
    let (_root, store, attempt) = running_group_attempt_with_reviewer(None);
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = NeverStartedProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_internal_pr_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("missing internal reviewer must land the configuration gate");

    assert!(
        matches!(
            error,
            CodingWorkspaceEngineError::ReviewerConfigurationMissing { ref attempt_id, .. }
                if attempt_id == &attempt.id
        ),
        "expected ReviewerConfigurationMissing, got: {error:?}"
    );
    assert_eq!(provider.starts(), 0);

    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs");
    assert!(runs.is_empty());
    let nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline nodes");
    assert!(
        nodes
            .iter()
            .all(|node| node.stage != CodingExecutionStage::InternalPrReview)
    );

    let current = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(current.status, CodingAttemptStatus::Blocked);
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("reviewer_configuration_missing")
    );
    let action_ids: Vec<&str> = gates[0]
        .available_actions
        .iter()
        .map(|action| action.action_id.as_str())
        .collect();
    assert!(
        action_ids.contains(&"retry_internal_review"),
        "actions: {action_ids:?}"
    );
    assert!(action_ids.contains(&"abort"), "actions: {action_ids:?}");
}

#[tokio::test]
async fn configured_reviewer_continues_without_author_substitution() {
    let (_root, store, attempt) = running_attempt_with_reviewer(None);
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = NeverStartedProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("missing reviewer lands the gate first");
    assert!(matches!(
        error,
        CodingWorkspaceEngineError::ReviewerConfigurationMissing { .. }
    ));

    // 用户在等待项配置合法 reviewer 后重试：以新配置从原阶段继续，历史不被改写。
    let mut configured = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role snapshot");
    configured.code_reviewer = Some(ProviderName::Fake);
    store
        .update_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            configured,
        )
        .expect("configure reviewer");

    // attempt 仍是 Blocked：配置不自动续跑，重试动作才续跑（人工门语义）。
    let current = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(current.status, CodingAttemptStatus::Blocked);
    let snapshot = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role snapshot after configure");
    assert_eq!(snapshot.code_reviewer, Some(ProviderName::Fake));
    assert_ne!(
        snapshot.code_reviewer,
        Some(snapshot.coder.clone()),
        "reviewer must never silently become the author provider"
    );
    let reviewer_config = snapshot.reviewer_config();
    assert_eq!(reviewer_config.effective, Some(ProviderName::Fake));
    assert!(reviewer_config.enabled);
}
