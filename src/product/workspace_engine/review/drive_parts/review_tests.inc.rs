/// B1-1(波 2):规划 reviewer 开窗保留主 scope 并铸造 launch 归属。
#[cfg(test)]
mod b1_plan_launch_ownership_tests {
    use super::*;
    use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};

    fn b1_git_init(dir: &std::path::Path) {
        use std::process::Command;
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "b1@example.test"],
            vec!["config", "user.name", "B1 Plan"],
            vec!["commit", "-m", "init", "--quiet", "--allow-empty"],
        ] {
            let status = Command::new("git")
                .current_dir(dir)
                .args(&args)
                .status()
                .expect("git fixture command");
            assert!(status.success(), "git fixture {args:?} must succeed");
        }
    }

    #[tokio::test]
    async fn b1_plan_review_window_keeps_scope_and_mints_launch_ownership() {
        let root = tempfile::tempdir().expect("root");
        let paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
        let project = crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "B1-PLAN".to_string(),
                description: None,
            })
            .expect("project");
        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project.id.clone(),
                repo_id: None,
                logical_codebase_id: crate::product::logical_codebase::legacy_logical_codebase_id(
                    &project.id,
                ),
                base_branch: None,
                title: "b1 plan".to_string(),
                description: None,
                change_id: None,
            })
            .expect("issue");
        let lifecycle = LifecycleStore::new(paths.clone());
        let record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: project.id.clone(),
                issue_id: issue.id.clone(),
                entity_id: issue.id.clone(),
                workspace_type: crate::product::models::WorkspaceType::Story,
                author_provider: crate::product::models::ProviderName::ClaudeCode,
                reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("workspace session");
        let checkpoint =
            std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                paths.issue_lifecycle_root(&project.id, &issue.id),
            ));
        let (tx, _rx) = mpsc::channel(64);
        let mut engine = WorkspaceEngine::new_persistent(
            checkpoint,
            LifecycleStore::new(paths.clone()),
            tx,
            crate::product::workspace_engine::types::WorkspaceSession::from_record(record),
        );
        // 直连形态:repository_path 即带 .git 的评审目标(跳过聚合根解析)。
        let worktree = root.path().join("plan-worktree");
        std::fs::create_dir_all(&worktree).expect("worktree");
        b1_git_init(&worktree);
        engine.session.repository_path = Some(worktree.clone());

        engine
            .begin_plan_review_window(&crate::product::models::ProviderName::ClaudeCode, worktree)
            .await
            .expect("begin plan review window");

        let active = engine
            .active_review_window
            .as_ref()
            .expect("活动窗持有主 scope");
        let expected_scope_id = format!("plan-review-{}", active.handle.window_id);
        assert_eq!(active.launch_scope.key().scope_id, expected_scope_id);
        // launch 归属从同一 scope 铸造(同 window/run/boot;资源 ID 唯一)。
        let ownership = engine
            .plan_review_launch_ownership()
            .expect("活动窗铸造 launch 归属");
        assert_eq!(ownership.scope_key(), active.launch_scope.key());
        let again = engine.plan_review_launch_ownership().expect("再次铸造");
        assert_ne!(ownership.resource_id(), again.resource_id());

        engine.close_plan_review_window("b1_plan_test_exit").await;
    }
}

/// 波3 F11:close_plan_review_window 的用户可见人工注意投影。
/// 红线:store 失效/恢复人工门时仅 tracing::warn 不可接受——必须经
/// 既有 EngineEvent 事件流发用户可见事件(带 window_id/原因/人工入口),
/// 且不改变锁与 generation 语义(不解锁、不丢窗口引用、状态机原样)。
#[cfg(test)]
mod f11_close_attention_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::reviewer_window_store::{
        REVIEWER_WINDOW_STORE_DIR, ReviewerWindowState, ReviewerWindowStore, ViolationJournalEntry,
        tree_key_of,
    };

    fn f11_git_init(dir: &std::path::Path) {
        use std::process::Command;
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "f11@example.test"],
            vec!["config", "user.name", "F11 Plan"],
            vec!["commit", "-m", "init", "--quiet", "--allow-empty"],
        ] {
            let status = Command::new("git")
                .current_dir(dir)
                .args(&args)
                .status()
                .expect("git fixture command");
            assert!(status.success(), "git fixture {args:?} must succeed");
        }
    }

    fn f11_engine(
        root: &std::path::Path,
    ) -> (
        WorkspaceEngine,
        mpsc::Receiver<EngineEvent>,
        ProductAppPaths,
        std::path::PathBuf,
    ) {
        let paths = ProductAppPaths::new(root.join(".aria"));
        let project = crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "F11-PLAN".to_string(),
                description: None,
            })
            .expect("project");
        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project.id.clone(),
                repo_id: None,
                logical_codebase_id: crate::product::logical_codebase::legacy_logical_codebase_id(
                    &project.id,
                ),
                base_branch: None,
                title: "f11 plan".to_string(),
                description: None,
                change_id: None,
            })
            .expect("issue");
        let lifecycle = LifecycleStore::new(paths.clone());
        let record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: project.id.clone(),
                issue_id: issue.id.clone(),
                entity_id: issue.id.clone(),
                workspace_type: crate::product::models::WorkspaceType::Story,
                author_provider: crate::product::models::ProviderName::ClaudeCode,
                reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("workspace session");
        let checkpoint =
            std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                paths.issue_lifecycle_root(&project.id, &issue.id),
            ));
        let (tx, rx) = mpsc::channel(64);
        let mut engine = WorkspaceEngine::new_persistent(
            checkpoint,
            LifecycleStore::new(paths.clone()),
            tx,
            crate::product::workspace_engine::types::WorkspaceSession::from_record(record),
        );
        let worktree = root.join("f11-worktree");
        std::fs::create_dir_all(&worktree).expect("worktree");
        f11_git_init(&worktree);
        engine.session.repository_path = Some(worktree.clone());
        (engine, rx, paths, worktree)
    }

    async fn f11_next_attention(
        rx: &mut mpsc::Receiver<EngineEvent>,
        prefix: &str,
    ) -> Option<crate::cross_cutting::streaming_provider::ProviderExecutionEvent> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while let Ok(Some(event)) = tokio::time::timeout_at(deadline, rx.recv()).await {
            if let EngineEvent::ExecutionEvent { event, .. } = event
                && event.event_id.starts_with(prefix)
            {
                return Some(event);
            }
        }
        None
    }

    /// store 失效:仅 warn 不可接受——必须发用户可见注意事件,且窗口
    /// 记录(锁与 generation)原样保持。
    #[tokio::test]
    async fn f11_store_unavailable_close_emits_attention_and_keeps_window() {
        let root = tempfile::tempdir().expect("root");
        let (mut engine, mut rx, paths, worktree) = f11_engine(root.path());
        engine
            .begin_plan_review_window(&crate::product::models::ProviderName::ClaudeCode, worktree)
            .await
            .expect("begin plan review window");
        let active = engine.active_review_window.as_ref().expect("active window");
        let window_id = active.handle.window_id.clone();
        let generation = active.handle.generation;
        let window_store = ReviewerWindowStore::new(paths.root().join(REVIEWER_WINDOW_STORE_DIR));
        let before = window_store.load_window(&window_id).expect("record before");

        engine.lifecycle_store = None;
        engine.close_plan_review_window("f11_store_gone").await;

        let attention = f11_next_attention(
            &mut rx,
            "plan_review_window_attention_close_store_unavailable",
        )
        .await
        .expect("F11 红:store 失效收口必须发用户可见注意事件");
        let detail = attention.detail.as_deref().unwrap_or("");
        assert!(detail.contains(&window_id), "注意事件必须带 window_id");
        assert!(!detail.is_empty(), "注意事件必须带原因/人工入口");

        let after = window_store.load_window(&window_id).expect("record after");
        assert_eq!(after.state, before.state, "锁与状态保持");
        assert_eq!(after.generation, before.generation, "generation 保持");
        assert_eq!(after.generation, generation);
    }

    /// 恢复人工门:必须发用户可见注意事件(含人工裁决入口),锁保持
    /// (ManualGate 非终态、租约不释放)。
    #[tokio::test]
    async fn f11_manual_gate_close_emits_attention_and_keeps_lock() {
        let root = tempfile::tempdir().expect("root");
        let (mut engine, mut rx, paths, worktree) = f11_engine(root.path());
        engine
            .begin_plan_review_window(&crate::product::models::ProviderName::ClaudeCode, worktree)
            .await
            .expect("begin plan review window");
        let active = engine.active_review_window.as_ref().expect("active window");
        let window_id = active.handle.window_id.clone();
        let window_store = ReviewerWindowStore::new(paths.root().join(REVIEWER_WINDOW_STORE_DIR));
        let record = window_store.load_window(&window_id).expect("record");
        window_store
            .append_violation(
                &window_id,
                ViolationJournalEntry {
                    seq: 0,
                    entry_id: "f11-violation-entry-1".to_string(),
                    at: chrono::Utc::now().to_rfc3339(),
                    window_id: window_id.clone(),
                    attempt_id: record.attempt_id.clone(),
                    run_id: record.run_id.clone(),
                    report_id: None,
                    kind: "worktree_mutation".to_string(),
                    path: "reviewer-edit.txt".to_string(),
                    before_digest: None,
                    after_digest: None,
                    detail: "F11 夹具:预置违规 journal 触发人工门".to_string(),
                    restore_cursor: "started+0".to_string(),
                },
            )
            .expect("append violation");

        engine.close_plan_review_window("f11_violation").await;

        let attention = f11_next_attention(&mut rx, "plan_review_window_attention_manual_gate")
            .await
            .expect("F11 红:人工门收口必须发用户可见注意事件");
        let detail = attention.detail.as_deref().unwrap_or("");
        assert!(detail.contains(&window_id), "注意事件必须带 window_id");
        assert!(
            detail.contains("Invalidate") && detail.contains("RequireNewReview"),
            "注意事件必须披露人工裁决入口: {detail}"
        );

        let after = window_store.load_window(&window_id).expect("record after");
        assert_eq!(
            after.state,
            ReviewerWindowState::ManualGate,
            "锁保持:人工门非终态"
        );
        let tree_key = tree_key_of(&after.identity.canonical_worktree);
        let lease = window_store
            .load_lease(&tree_key)
            .expect("lease read")
            .expect("锁保持:租约未释放");
        assert_eq!(lease.window_id, window_id);
    }
}

/// 规划面窗口 ID 全局唯一(跨会话撞号回归)。
///
/// 窗口 store 目录跨会话共享(全局 `.aria` 目录):规划面此前自持
/// `plan-<run>-w01` 格式化副本,不同会话的同号 plan run 会算出同一
/// 窗口 ID → 后一个会话开窗命中旧记录被拒
/// (`reviewer_window_conflict[reviewer_window]`),plan review
/// fail-closed 走不下去。窗口 ID 必须带会话前缀
/// (`role_run_window_id(session_id, run_id)`)。
#[cfg(test)]
mod plan_window_id_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};

    fn git_init(dir: &std::path::Path) {
        use std::process::Command;
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "plan-window@example.test"],
            vec!["config", "user.name", "Plan Window"],
            vec!["commit", "-m", "init", "--quiet", "--allow-empty"],
        ] {
            let status = Command::new("git")
                .current_dir(dir)
                .args(&args)
                .status()
                .expect("git fixture command");
            assert!(status.success(), "git fixture {args:?} must succeed");
        }
    }

    /// 同项目同 issue 下的两个会话 + 各自的评审工作树(直连形态:
    /// repository_path 即评审目标)。
    fn two_session_engines(
        root: &std::path::Path,
    ) -> (WorkspaceEngine, WorkspaceEngine, ProductAppPaths) {
        let paths = ProductAppPaths::new(root.join(".aria"));
        let project = crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "PLAN-WINDOW-ID".to_string(),
                description: None,
            })
            .expect("project");
        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project.id.clone(),
                repo_id: None,
                logical_codebase_id: crate::product::logical_codebase::legacy_logical_codebase_id(
                    &project.id,
                ),
                base_branch: None,
                title: "plan window id".to_string(),
                description: None,
                change_id: None,
            })
            .expect("issue");
        let lifecycle = LifecycleStore::new(paths.clone());
        let mut engines = Vec::new();
        for index in 0..2 {
            let record = lifecycle
                .create_workspace_session(CreateWorkspaceSessionInput {
                    project_id: project.id.clone(),
                    issue_id: issue.id.clone(),
                    entity_id: issue.id.clone(),
                    workspace_type: crate::product::models::WorkspaceType::Story,
                    author_provider: crate::product::models::ProviderName::ClaudeCode,
                    reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                    review_rounds: 1,
                    superpowers_enabled: false,
                    openspec_enabled: false,
                    work_item_plan_options: None,
                })
                .expect("workspace session");
            let checkpoint =
                std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                    paths.issue_lifecycle_root(&project.id, &issue.id),
                ));
            let (tx, _rx) = mpsc::channel(64);
            let mut engine = WorkspaceEngine::new_persistent(
                checkpoint,
                LifecycleStore::new(paths.clone()),
                tx,
                crate::product::workspace_engine::types::WorkspaceSession::from_record(record),
            );
            let worktree = root.join(format!("plan-window-worktree-{index}"));
            std::fs::create_dir_all(&worktree).expect("worktree");
            git_init(&worktree);
            engine.session.repository_path = Some(worktree.clone());
            engines.push(engine);
        }
        let mut engines = engines.into_iter();
        (
            engines.next().expect("first engine"),
            engines.next().expect("second engine"),
            paths,
        )
    }

    /// 回归:两个不同会话的同号 plan run → 窗口 ID 不撞,两窗都能开。
    #[tokio::test]
    async fn plan_window_ids_are_unique_across_sessions() {
        let root = tempfile::tempdir().expect("root");
        let (mut first, mut second, paths) = two_session_engines(root.path());
        // 两个会话跑同一个 run id(生产上跨会话可撞同一 run 号)。
        first.mark_active_run_started("run-2");
        second.mark_active_run_started("run-2");

        let worktree_a = first.session.repository_path.clone().expect("worktree a");
        let worktree_b = second.session.repository_path.clone().expect("worktree b");
        first
            .begin_plan_review_window(
                &crate::product::models::ProviderName::ClaudeCode,
                worktree_a,
            )
            .await
            .expect("first plan review window");
        second
            .begin_plan_review_window(
                &crate::product::models::ProviderName::ClaudeCode,
                worktree_b,
            )
            .await
            .expect("second plan review window must not collide");

        let window_a = first
            .active_review_window
            .as_ref()
            .expect("window a")
            .handle
            .window_id
            .clone();
        let window_b = second
            .active_review_window
            .as_ref()
            .expect("window b")
            .handle
            .window_id
            .clone();
        assert_ne!(
            window_a, window_b,
            "不同会话的同号 plan run 不得撞同一窗口 id"
        );
        assert_eq!(
            window_a,
            crate::product::reviewer_window_store::role_run_window_id(
                &first.session.session_id,
                "run-2"
            ),
            "窗口 id 必须带会话前缀(role_run_window_id)"
        );
        assert_eq!(
            window_b,
            crate::product::reviewer_window_store::role_run_window_id(
                &second.session.session_id,
                "run-2"
            )
        );

        // 两个窗口记录都在 store 里(后者没有顶掉前者)。
        let store = crate::product::reviewer_window_store::ReviewerWindowStore::new(
            paths
                .root()
                .join(crate::product::reviewer_window_store::REVIEWER_WINDOW_STORE_DIR),
        );
        assert_eq!(
            store.load_window(&window_a).expect("record a").state,
            crate::product::reviewer_window_store::ReviewerWindowState::Running
        );
        assert_eq!(
            store.load_window(&window_b).expect("record b").state,
            crate::product::reviewer_window_store::ReviewerWindowState::Running
        );
    }
}

/// REQ-ENV-13（Review Focus 1）：单仓「无窗口」路径 MUST 保持
/// `reviewer_verify_policy() == None`——多仓（lc）会话身份 UUID 策略
/// 不得外溢到单仓形态，否则本该被四家启动检查/网关复验拒绝的无窗
/// 单仓调用会借 UUID 合同通过。
#[cfg(test)]
mod single_repo_review_identity_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};

    #[test]
    fn single_repo_without_window_yields_no_reviewer_verify_policy() {
        let root = tempfile::tempdir().expect("root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());
        let record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "issue_0001".to_string(),
                workspace_type: crate::product::models::WorkspaceType::Story,
                author_provider: crate::product::models::ProviderName::ClaudeCode,
                reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("workspace session");
        let checkpoint =
            std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                paths.issue_lifecycle_root(&record.project_id, &record.issue_id),
            ));
        let (tx, _rx) = mpsc::channel(64);
        let engine = WorkspaceEngine::new_persistent(
            checkpoint,
            LifecycleStore::new(paths.clone()),
            tx,
            crate::product::workspace_engine::types::WorkspaceSession::from_record(record),
        );

        // 单仓形态：未注入 logical gateway；未开评审窗。
        assert!(
            !engine.is_logical_codebase_issue(),
            "前置：无 gateway 注入即单仓形态"
        );
        assert!(engine.active_review_window.is_none(), "前置：未开评审窗");
        assert!(
            engine.reviewer_verify_policy().is_none(),
            "单仓无窗口路径必须保持 None，不得借用会话 UUID 身份铸合同"
        );
    }
}
