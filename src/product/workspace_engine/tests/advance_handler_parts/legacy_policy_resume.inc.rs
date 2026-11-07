/// k3 波2 F1（controller fix loop 第 1 轮）：升级前已 durable 的单 target
/// enrolled advance——旧 record 无 `start_run_policy` 字段（serde 读
/// Manual）、journal 当时已穿透 AutoStartOnce。修复前：编排器（传
/// AutoStartOnce）在 record 冻结互证处 Err、retry（从 record 取 Manual）
/// 在 journal 互证处 Err——双双永久卡死且无热修路径。修复后：record 已
/// 绑定 attempt 视为「策略未指定」，以 journal 冻结值为权威续做
///（AutoStartOnce 保持，不升格不降级）；显式新 record 行为不变。
#[tokio::test]
async fn legacy_record_without_policy_field_resumes_with_journal_authority() {
    use crate::product::coding_models::CodingStartRunPolicy;

    let auto_policy = CodingStartRunPolicy::AutoStartOnce {
        enrollment_id: "enrollment_legacy_policy".to_string(),
        policy_revision: 1,
        source_plan_revision: "revision_advance".to_string(),
    };
    let root = tempfile::tempdir().unwrap();
    crate::web::test_controls::PlanRepairFixtureRuntime::seed(
        root.path(),
        crate::web::test_controls::PlanRepairFixtureControl::default(),
    )
    .await
    .expect("seed complete authoritative advance fixture");

    let app_paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
    let coding_store =
        crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone());
    let seeded_attempt = coding_store
        // 基座 attempt 带冻结快照 ⇒ 桶键 Some(logical_repository_id)，
        // None 桶检索落空 ⇒ 桶无关取数（见 group.rs `attempt_target_bucket`）。
        .list_attempts_for_work_item_group("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .unwrap()
        .into_iter()
        .next()
        .expect("seeded group attempt");
    coding_store
        .delete_attempt("project_0001", "issue_plan_0001", &seeded_attempt.id)
        .unwrap();
    seed_advance_draft_records(&app_paths);

    let lifecycle = LifecycleStore::new(app_paths.clone());
    let session_record = lifecycle
        .list_workspace_sessions("project_0001", "issue_plan_0001")
        .unwrap()
        .into_iter()
        .find(|session| {
            session.workspace_type == WorkspaceType::WorkItemPlan
                && session.entity_id == "work_item_plan_0001"
        })
        .expect("seeded plan workspace session");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(session_record),
    );
    let input = |command_id: &str| AdvanceInput {
        command_id: command_id.to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_plan_0001".to_string(),
        plan_id: "work_item_plan_0001".to_string(),
    };
    let admission = || {
        crate::product::workspace_engine::AdvancePlanAdmission::acquire(
            &app_paths,
            "project_0001",
            "issue_plan_0001",
            "work_item_plan_0001",
        )
        .expect("plan admission")
    };

    // 首轮：AutoStartOnce 穿透到 Ready（journal attempt 冻结 AutoStartOnce）。
    let first = engine
        .handle_advance_with_start_policy(
            input("command_legacy_policy"),
            auto_policy.clone(),
            &admission(),
        )
        .await
        .expect("first auto advance must complete");
    let (record_id, first_attempt) = match &first {
        AdvanceOutcome::Completed {
            record, attempt_id, ..
        } => (record.id.clone(), attempt_id.clone()),
        other => panic!("first advance must complete, got {other:?}"),
    };

    // 模拟「升级前已 durable」旧形态：磁盘 record 删除 start_run_policy 字段
    // （serde 读 Manual）并回 Initializing（retry 重开后续做的 durable 形态）。
    let record_path = app_paths
        .issue_root("project_0001", "issue_plan_0001")
        .join("advance-records")
        .join(format!("{record_id}.json"));
    let rewrite_legacy_initializing = || {
        let mut json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&record_path).unwrap()).unwrap();
        let object = json.as_object_mut().unwrap();
        assert!(object.remove("start_run_policy").is_some());
        object.insert("status".to_string(), serde_json::json!("initializing"));
        std::fs::write(&record_path, serde_json::to_string(&json).unwrap()).unwrap();
    };

    // A（编排器链路，修复前红）：旧 record + 传 AutoStartOnce → 以 journal
    // 冻结值为权威续做，不再 Err；同一 journal/attempt 复用。
    rewrite_legacy_initializing();
    let replay_auto = engine
        .handle_advance_with_start_policy(
            input("command_legacy_policy"),
            auto_policy.clone(),
            &admission(),
        )
        .await
        .expect("legacy record must resume under journal authority (auto)");
    if let AdvanceOutcome::Completed { attempt_id, .. } = &replay_auto {
        assert_eq!(
            attempt_id, &first_attempt,
            "续做必须复用同一 journal attempt"
        );
    }

    // B（retry 链路，修复前红）：record 读 Manual（旧形态）+ 传 Manual →
    // journal(AutoStartOnce) 以其为权威放行；attempt 策略保持 AutoStartOnce
    //（当时真实授权的 durable 事实，不升格不降级）。
    rewrite_legacy_initializing();
    engine
        .handle_advance_with_start_policy(
            input("command_legacy_policy"),
            CodingStartRunPolicy::Manual,
            &admission(),
        )
        .await
        .expect("legacy record must resume under journal authority (manual/retry)");
    let journal = coding_store
        .get_group_initialization("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .expect("group journal");
    assert_eq!(journal.attempt.start_run_policy, auto_policy);

    // C（显式新 record 行为不变）：独立 fixture 走显式 Manual 完成 →
    // 新 command 传 AutoStartOnce → record(Manual)==journal(Manual) 与传入
    // 冲突＝真·漂移，仍 fail-closed（不升格）。
    let root_c = tempfile::tempdir().unwrap();
    crate::web::test_controls::PlanRepairFixtureRuntime::seed(
        root_c.path(),
        crate::web::test_controls::PlanRepairFixtureControl::default(),
    )
    .await
    .expect("seed fixture for explicit policy case");
    let app_paths_c = crate::product::app_paths::ProductAppPaths::new(root_c.path().join(".aria"));
    let coding_store_c =
        crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths_c.clone());
    let seeded_c = coding_store_c
        .list_attempts_for_work_item_group("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .unwrap()
        .into_iter()
        .next()
        .expect("seeded group attempt");
    coding_store_c
        .delete_attempt("project_0001", "issue_plan_0001", &seeded_c.id)
        .unwrap();
    seed_advance_draft_records(&app_paths_c);
    let lifecycle_c = LifecycleStore::new(app_paths_c.clone());
    let session_c = lifecycle_c
        .list_workspace_sessions("project_0001", "issue_plan_0001")
        .unwrap()
        .into_iter()
        .find(|session| {
            session.workspace_type == WorkspaceType::WorkItemPlan
                && session.entity_id == "work_item_plan_0001"
        })
        .expect("seeded plan workspace session");
    let (event_tx_c, _event_rx_c) = mpsc::channel(8);
    let mut engine_c = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root_c.path().join("checkpoints"))),
        lifecycle_c,
        event_tx_c,
        WorkspaceSession::from_record(session_c),
    );
    engine_c
        .handle_advance(input("command_explicit_manual"))
        .await
        .expect("explicit manual advance must complete");
    // 显式 record 保持 start_run_policy 字段、仅回 Initializing（retry 重开
    // 后的续做形态）——新 command 传 AutoStartOnce：record(Manual)==journal
    // (Manual) 显式一致而与传入冲突＝真·漂移，仍 fail-closed（不升格）。
    let explicit_record = AdvanceStore::new(app_paths_c.clone())
        .get_advance_for_plan("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .unwrap()
        .expect("explicit record");
    let explicit_path = app_paths_c
        .issue_root("project_0001", "issue_plan_0001")
        .join("advance-records")
        .join(format!("{}.json", explicit_record.id));
    let mut explicit_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&explicit_path).unwrap()).unwrap();
    assert!(
        explicit_json
            .as_object()
            .unwrap()
            .contains_key("start_run_policy"),
        "新 record 显式携带策略字段"
    );
    explicit_json
        .as_object_mut()
        .unwrap()
        .insert("status".to_string(), serde_json::json!("initializing"));
    std::fs::write(
        &explicit_path,
        serde_json::to_string(&explicit_json).unwrap(),
    )
    .unwrap();
    let admission_c = crate::product::workspace_engine::AdvancePlanAdmission::acquire(
        &app_paths_c,
        "project_0001",
        "issue_plan_0001",
        "work_item_plan_0001",
    )
    .expect("plan admission");
    let error = engine_c
        .handle_advance_with_start_policy(input("command_explicit_auto"), auto_policy, &admission_c)
        .await
        .expect_err("explicit manual record must reject auto escalation");
    assert!(
        error.contains("froze start policy"),
        "explicit record drift error: {error}"
    );
}
