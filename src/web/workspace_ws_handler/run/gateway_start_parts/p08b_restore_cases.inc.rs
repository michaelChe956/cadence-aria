    // -----------------------------------------------------------------------
    // P08b(durable restore):已 begin 的 split run 句柄在 store 重建后必须能
    // 恢复同身份 `PlanSplitRunContext`——不重新分配 seq、不 begin、不写盘;
    // 跨 session/project/issue/provider 绑定与盘上 run/session 记录篡改一律
    // fail-closed;running 轮且无 raw 输出收口时恢复合法。
    // -----------------------------------------------------------------------
    fn p08b_fixture() -> (
        tempfile::TempDir,
        crate::product::lifecycle_store::LifecycleStore,
        String,
        String,
        ProviderName,
        crate::product::work_item_split_engine::parse::WorkItemSplitProviderRunHandle,
    ) {
        use crate::product::app_paths::ProductAppPaths;
        use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
        use crate::product::models::WorkspaceType;

        let tmp = tempfile::TempDir::new().expect("p08b temp root");
        let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let session = store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_p08b".to_string(),
                issue_id: "issue_p08b".to_string(),
                entity_id: "plan_p08b".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: None,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("p08b session");
        let provider = session.author_provider.clone();
        let ctx = begin_plan_split_run(&store, "project_p08b", "issue_p08b", &provider, &session.id)
            .expect("p08b begin");
        (
            tmp,
            store,
            "project_p08b".to_string(),
            "issue_p08b".to_string(),
            provider,
            ctx.handle,
        )
    }

    #[test]
    fn p08b_restore_reuses_handle_identity_across_store_rebuild() {
        use crate::product::lifecycle_store::LifecycleStore;

        let (_tmp, store, project, issue, provider, handle) = p08b_fixture();
        let run_dir = store
            .work_item_split_provider_run_dir(&handle)
            .expect("p08b run dir");
        let run_path = run_dir.join("run.json");
        let session_path = store
            .workspace_sessions_root(&project, &issue)
            .join(format!("{}.json", handle.workspace_session_id));
        let run_before = std::fs::read(&run_path).expect("p08b run.json bytes");
        let session_before = std::fs::read(&session_path).expect("p08b session bytes");
        let seq_dirs_before = std::fs::read_dir(run_dir.parent().expect("p08b session dir"))
            .expect("p08b split runs dir")
            .count();

        // 仅凭 durable handle 在新 store 实例恢复:running 轮、无 raw 输出收口合法。
        let reopened = LifecycleStore::new(store.app_paths());
        let restored =
            plan_split_run_for_handle(&reopened, &project, &issue, &provider, handle.clone())
                .expect("p08b durable handle must restore without new allocation");

        assert_eq!(restored.handle, handle, "恢复句柄身份必须逐字段相同");
        assert_eq!(restored.project_id, project, "绑定 project 原样恢复");
        assert_eq!(restored.issue_id, issue, "绑定 issue 原样恢复");
        assert_eq!(restored.provider, provider, "绑定 provider 原样恢复");
        assert_eq!(
            restored.lifecycle.app_paths().root(),
            store.app_paths().root(),
            "恢复上下文必须克隆同一 lifecycle 根"
        );
        // 不 begin/不推进 seq/不写盘:run 记录、会话记录、run 目录集合逐字节不变。
        assert_eq!(std::fs::read(&run_path).expect("p08b run.json bytes"), run_before);
        assert_eq!(std::fs::read(&session_path).expect("p08b session bytes"), session_before);
        assert_eq!(
            std::fs::read_dir(run_dir.parent().expect("p08b session dir"))
                .expect("p08b split runs dir")
                .count(),
            seq_dirs_before,
            "restore 不得分配新 role_run_seq 目录"
        );
    }

    #[test]
    fn p08b_restore_rejects_tampered_bindings() {
        let (_tmp, store, project, issue, provider, handle) = p08b_fixture();
        let run_path = store
            .work_item_split_provider_run_dir(&handle)
            .expect("p08b run dir")
            .join("run.json");

        // ① 调用方绑定参数与 durable 事实不符:project/issue/provider 参数表。
        for (label, wrong_project, wrong_issue, wrong_provider) in [
            ("project", "project_p08b_other", issue.as_str(), provider.clone()),
            ("issue", project.as_str(), "issue_p08b_other", provider.clone()),
            ("provider", project.as_str(), issue.as_str(), ProviderName::ClaudeCode),
        ] {
            let rejected = plan_split_run_for_handle(
                &store,
                wrong_project,
                wrong_issue,
                &wrong_provider,
                handle.clone(),
            );
            assert!(rejected.is_err(), "绑定不符({label})必须拒绝");
        }

        // ② 盘上 run.json 的 role_run_seq 被改写:必须可见 Err 且不静默修复盘上事实。
        let run_original = std::fs::read(&run_path).expect("p08b run.json bytes");
        let mut run_json: serde_json::Value =
            serde_json::from_slice(&run_original).expect("p08b json");
        run_json["role_run_seq"] = serde_json::json!(handle.role_run_seq + 1);
        let run_tampered = serde_json::to_vec(&run_json).expect("p08b encode");
        std::fs::write(&run_path, &run_tampered).expect("p08b tamper run.json");
        assert!(
            plan_split_run_for_handle(&store, &project, &issue, &provider, handle.clone()).is_err(),
            "run.json role_run_seq 篡改必须拒绝"
        );
        assert_eq!(
            std::fs::read(&run_path).expect("p08b run.json bytes"),
            run_tampered,
            "拒绝对盘上事实零修复"
        );
        std::fs::write(&run_path, &run_original).expect("p08b restore run.json");

        // ③ 会话记录 author_provider 被改写:provider 绑定不再成立,必须拒绝。
        let session_path = store
            .workspace_sessions_root(&project, &issue)
            .join(format!("{}.json", handle.workspace_session_id));
        let session_original = std::fs::read(&session_path).expect("p08b session bytes");
        let mut session_json: serde_json::Value =
            serde_json::from_slice(&session_original).expect("p08b session json");
        session_json["author_provider"] = serde_json::json!("claude_code");
        std::fs::write(&session_path, serde_json::to_vec(&session_json).expect("p08b encode"))
            .expect("p08b tamper session");
        assert!(
            plan_split_run_for_handle(&store, &project, &issue, &provider, handle.clone()).is_err(),
            "session author_provider 篡改必须拒绝"
        );
        std::fs::write(&session_path, &session_original).expect("p08b restore session");
    }
