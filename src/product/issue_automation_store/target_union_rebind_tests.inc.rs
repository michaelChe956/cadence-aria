// ---------------------------------------------------------------------------
// C1 Task 1：双载体 target union 与版本化 enrollment binding（REQ-C1-TARGET-01）。
// ---------------------------------------------------------------------------

fn target_logical_at(repository: LogicalRepositoryId) -> EnrollmentTarget {
    EnrollmentTarget::LogicalCodebase {
        logical_codebase_id: "logical_codebase_0001".into(),
        logical_repository_id: repository,
    }
}

fn target_logical() -> EnrollmentTarget {
    target_logical_at(logical_repo(1))
}

fn enable_with_target(selection_key: &str, target: EnrollmentTarget) -> EnrollmentWriteCommand {
    EnrollmentWriteCommand::Enable {
        selection_key: selection_key.into(),
        source: source(),
        options: options(),
        target,
    }
}

fn rebind_request(
    command_id: &str,
    expected_policy_revision: u64,
    expected_binding_version: u64,
    plan_id: &str,
    session_id: &str,
    target: EnrollmentTarget,
) -> EnrollmentRebindRequest {
    EnrollmentRebindRequest {
        command_id: command_id.into(),
        expected_policy_revision,
        expected_binding_version,
        binding: EnrollmentBindingIdentityInput {
            plan_id: plan_id.into(),
            session_id: session_id.into(),
            source: source(),
            target,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
        },
        reason: "recover after failed generation".into(),
    }
}

/// 单仓 target 序列化保留真实 physical id，读取后不生成 logical 替身；
/// 逻辑 target 缺任一级身份即拒绝（REQ-C1-TARGET-01）。
#[test]
fn issue_automation_store_target_union_roundtrip_preserves_physical_id() {
    let single = EnrollmentTarget::SingleRepository {
        repository_id: "repo_physical_1".into(),
    };
    let json = serde_json::to_value(&single).unwrap();
    assert_eq!(json["kind"], "single_repository");
    assert_eq!(json["repository_id"], "repo_physical_1");
    assert!(
        json.get("logical_repository_id").is_none(),
        "single repository target must not grow a logical stand-in: {json}"
    );
    let back: EnrollmentTarget = serde_json::from_value(json).unwrap();
    assert_eq!(back, single);

    let logical = target_logical();
    let json = serde_json::to_value(&logical).unwrap();
    assert_eq!(json["kind"], "logical_codebase");
    let back: EnrollmentTarget = serde_json::from_value(json).unwrap();
    assert_eq!(back, logical);

    let missing_codebase = serde_json::json!({
        "kind": "logical_codebase",
        "logical_repository_id": logical_repo(1).0.to_string(),
    });
    assert!(
        serde_json::from_value::<EnrollmentTarget>(missing_codebase).is_err(),
        "logical target without logical_codebase_id must be rejected"
    );
    let missing_repository = serde_json::json!({
        "kind": "logical_codebase",
        "logical_codebase_id": "logical_codebase_0001",
    });
    assert!(
        serde_json::from_value::<EnrollmentTarget>(missing_repository).is_err(),
        "logical target without logical_repository_id must be rejected"
    );
}

/// 显式 rebind 在 enrollment 文件锁内追加版本历史：previous 逐字保留旧代、
/// 同 command 同 payload 幂等重放、异 payload / 旧 expected version / 跨载体
/// target fail-closed 且 durable 不变（REQ-WIGA-01、REQ-C1-TARGET-01）。
#[test]
fn issue_automation_store_rebind_appends_version_and_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));

    // 新式 enable 显式声明 target → durable binding v1（plan/session 绑定后补全）。
    let created = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable_with_target("human-choice-1", target_logical()),
        )
        .unwrap();
    assert_eq!(created.policy_revision, 1);
    assert_eq!(
        created
            .binding_history
            .as_ref()
            .unwrap()
            .current
            .binding_version,
        1
    );
    let bound = store
        .bind_plan(
            "project_1",
            "issue_1",
            created.policy_revision,
            "plan_0001",
            "session_0001",
        )
        .unwrap();
    let v1 = bound.binding_history.as_ref().unwrap().current.clone();
    assert_eq!(v1.plan_id, "plan_0001");
    assert_eq!(v1.session_id, "session_0001");
    assert_eq!(v1.target, target_logical());
    assert_eq!(v1.author_provider, bound.options.author_provider);
    assert!(bound.binding_history.as_ref().unwrap().previous.is_empty());

    // 首次 rebind：expected policy/binding 匹配当前 → binding_version=2，
    // previous 保留 v1 的完整 plan/session/source/target/provider。
    let request = rebind_request(
        "rebind_cmd_0001",
        bound.policy_revision,
        1,
        "plan_0002",
        "session_0002",
        target_logical(),
    );
    let result = store
        .rebind("project_1", "issue_1", request.clone())
        .unwrap();
    assert_eq!(result.command_id, "rebind_cmd_0001");
    assert_eq!(result.state, OperationState::Accepted);
    let after = store.get("project_1", "issue_1").unwrap().unwrap();
    let history = after.binding_history.as_ref().unwrap();
    assert_eq!(history.current.binding_version, 2);
    assert_eq!(history.current.plan_id, "plan_0002");
    assert_eq!(history.current.session_id, "session_0002");
    assert_eq!(history.current.enrollment_id, bound.enrollment_id);
    assert_eq!(history.previous.len(), 1);
    assert_eq!(history.previous[0], v1);
    // enrollment 的当前绑定投影与 current 代一致；rebind 是 CAS 修订。
    assert_eq!(after.plan_id.as_deref(), Some("plan_0002"));
    assert_eq!(after.session_id.as_deref(), Some("session_0002"));
    assert!(after.policy_revision > bound.policy_revision);

    // 同 command 同 payload 重放：返回同一 binding_version=2，durable 不变。
    let replay = store
        .rebind("project_1", "issue_1", request.clone())
        .unwrap();
    assert_eq!(replay.state, OperationState::Replayed);
    assert_eq!(replay.enrollment, after);
    assert_eq!(store.get("project_1", "issue_1").unwrap().unwrap(), after);

    // 同 command 异 payload → Conflict，current/previous 不变。
    let mut diverged = request.clone();
    diverged.binding.plan_id = "plan_0003".into();
    let error = store
        .rebind("project_1", "issue_1", diverged)
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::Conflict { .. }), "{error:?}");
    assert_eq!(store.get("project_1", "issue_1").unwrap().unwrap(), after);

    // 旧 expected binding version → Conflict。
    let stale_binding = rebind_request(
        "rebind_cmd_0002",
        after.policy_revision,
        1,
        "plan_0003",
        "session_0003",
        target_logical(),
    );
    let error = store
        .rebind("project_1", "issue_1", stale_binding)
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::Conflict { .. }), "{error:?}");

    // 旧 expected policy revision → Conflict。
    let stale_policy = rebind_request(
        "rebind_cmd_0003",
        bound.policy_revision,
        2,
        "plan_0003",
        "session_0003",
        target_logical(),
    );
    let error = store
        .rebind("project_1", "issue_1", stale_policy)
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::Conflict { .. }), "{error:?}");

    // 跨载体 target（logical → single）→ 拒绝，durable 不变。
    let cross_carrier = rebind_request(
        "rebind_cmd_0004",
        after.policy_revision,
        2,
        "plan_0003",
        "session_0003",
        EnrollmentTarget::SingleRepository {
            repository_id: "repo_physical_1".into(),
        },
    );
    let error = store
        .rebind("project_1", "issue_1", cross_carrier)
        .unwrap_err();
    assert!(
        matches!(error, EnrollmentError::Conflict { .. })
            || matches!(error, EnrollmentError::InvalidScope(_)),
        "{error:?}"
    );
    assert_eq!(store.get("project_1", "issue_1").unwrap().unwrap(), after);
}

/// 缺新字段的旧 enrollment JSON 按 off/Manual 兼容读：不自动补
/// plan/session/binding；无声明 target 的旧 enrollment rebind fail-closed。
#[test]
fn issue_automation_store_legacy_enrollment_json_reads_without_binding() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path());
    let store = IssueAutomationStore::new(paths.clone());
    let created = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable_with_target("human-choice-1", target_logical()),
        )
        .unwrap();
    let enrollment_file = paths
        .issue_root("project_1", "issue_1")
        .join("automation-enrollment.json");

    // 剥离 C1 新字段模拟旧格式 durable JSON（空 command_ledger 本就
    // skip 序列化，与旧格式逐字节一致）。
    let mut legacy: serde_json::Value = read_json(&enrollment_file).unwrap();
    let object = legacy.as_object_mut().unwrap();
    assert!(object.remove("target").is_some());
    assert!(object.remove("binding_history").is_some());
    assert!(object.remove("command_ledger").is_none());
    write_json(&enrollment_file, &legacy).unwrap();

    let enrollment = store.get("project_1", "issue_1").unwrap().unwrap();
    assert!(enrollment.target.is_none());
    assert!(enrollment.binding_history.is_none());
    assert!(enrollment.command_ledger.is_empty());
    assert_eq!(enrollment.plan_id, None);
    assert_eq!(enrollment.session_id, None);
    assert_eq!(enrollment.enrollment_id, created.enrollment_id);
    // C5 Task 1：冗余顶层 logical 身份已删除；旧 JSON 中该字段被忽略读取。
    assert_eq!(enrollment.policy_revision, created.policy_revision);

    // 旧 enrollment 无声明 target：rebind 不猜 target，fail-closed。
    let error = store
        .rebind(
            "project_1",
            "issue_1",
            rebind_request(
                "rebind_cmd_legacy",
                enrollment.policy_revision,
                1,
                "plan_0002",
                "session_0002",
                target_logical(),
            ),
        )
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::InvalidScope(_)), "{error:?}");
    assert_eq!(store.get("project_1", "issue_1").unwrap().unwrap(), enrollment);
}

// ---------------------------------------------------------------------------
// C5 Task 1：Enable 收敛必填 target；旧 JSON／旧冻结意图读侧 fail-closed
//（REQ-WIGA-01、Review Focus 1）。
// ---------------------------------------------------------------------------

/// Enable 缺 target 的 JSON 反序列化直接拒绝（HTTP 层即 422、零写入）；
/// 旧 enrollment JSON（含已废弃 `logical_repository_id`、无 `target`）读入
/// 不炸：`target` 读为 `None`（自动链经 `load_current_enrollment_binding`
/// 按旧代拒绝），Disable→重新 Enable 携带 target 后进入新链；幂等与
/// 序列化不含冗余 logical 身份零回归。
#[test]
fn enable_requires_target_and_old_enrollment_json_reads_fail_closed() {
    // 1) Enable 必带 target：缺字段 JSON 反序列化失败，多余旧字段被忽略。
    let missing_target = serde_json::json!({
        "type": "enable",
        "selection_key": "human-choice-1",
        "source": serde_json::to_value(source()).unwrap(),
        "options": serde_json::to_value(options()).unwrap(),
        "logical_repository_id": logical_repo(1).0.to_string(),
    });
    let error = serde_json::from_value::<EnrollmentWriteCommand>(missing_target)
        .expect_err("enable without target must fail deserialization");
    assert!(error.to_string().contains("target"), "{error}");

    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path());
    let store = IssueAutomationStore::new(paths.clone());
    let created = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();

    // 2) 旧 enrollment JSON：剥离 target/binding 后注入已废弃的
    //    logical_repository_id，模拟旧代 durable 记录——读入不炸，target None。
    let enrollment_file = paths
        .issue_root("project_1", "issue_1")
        .join("automation-enrollment.json");
    let mut legacy: serde_json::Value = read_json(&enrollment_file).unwrap();
    let object = legacy.as_object_mut().unwrap();
    assert!(object.remove("target").is_some());
    assert!(object.remove("binding_history").is_some());
    object.insert(
        "logical_repository_id".to_string(),
        serde_json::json!(logical_repo(1).0.to_string()),
    );
    write_json(&enrollment_file, &legacy).unwrap();
    let enrollment = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(enrollment.target, None);
    assert_eq!(enrollment.binding_history, None);
    assert_eq!(enrollment.enrollment_id, created.enrollment_id);
    assert_eq!(enrollment.policy_revision, created.policy_revision);

    // 3) 旧记录只读、不迁移；Disable 后重新 Enable 携带 target 进入新链。
    assert_eq!(
        read_json::<serde_json::Value>(&enrollment_file).unwrap(),
        legacy
    );
    store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(enrollment.policy_revision),
            EnrollmentWriteCommand::Disable,
        )
        .unwrap();
    let disabled = store.get("project_1", "issue_1").unwrap().unwrap();
    assert!(!disabled.enabled);
    let reopened = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(disabled.policy_revision),
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();
    assert!(reopened.enabled);
    assert_eq!(reopened.target.as_ref(), Some(&target_logical()));
    assert_eq!(
        reopened
            .binding_history
            .as_ref()
            .unwrap()
            .current
            .binding_version,
        1
    );
    assert_eq!(reopened.enrollment_id, created.enrollment_id);

    // 4) 幂等零回归：同选择键重复 Enable 返回同一 enrollment；序列化 JSON
    //    不再含 logical_repository_id。
    let replay = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();
    assert_eq!(replay, reopened);
    let serialized = serde_json::to_value(&replay).unwrap();
    assert!(
        serialized.get("logical_repository_id").is_none(),
        "enrollment JSON must not carry a redundant logical identity: {serialized}"
    );
    assert_eq!(serialized["target"]["kind"], "logical_codebase");
    assert_eq!(
        serialized["target"]["logical_repository_id"],
        logical_repo(1).0.to_string()
    );
}

/// 旧冻结意图（仅有 `logical_repository_id`、无 `target`）从 store 读入
/// 不炸：ensure/claim 一律 fail-closed 拒绝且绝不覆盖既有文件；新格式
/// 意图 target 与 enrollment target 不一致同样拒绝（身份漂移）。
#[test]
fn legacy_frozen_intents_read_fail_closed_without_target() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path());
    let store = IssueAutomationStore::new(paths.clone());
    let enrolled = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();
    let issue_root = paths.issue_root("project_1", "issue_1");

    // 1) 旧格式 prepared intent：读入不炸，ensure fail-closed、不覆盖。
    let legacy_prepared = serde_json::json!({
        "enrollment_id": enrolled.enrollment_id,
        "prepare_intent_id": enrolled.prepare_intent_id,
        "project_id": "project_1",
        "issue_id": "issue_1",
        "source": serde_json::to_value(source()).unwrap(),
        "options": serde_json::to_value(options()).unwrap(),
        "logical_repository_id": logical_repo(1).0.to_string(),
        "plan_id": format!("issue_work_item_plan_auto_{}", enrolled.prepare_intent_id),
        "session_id": format!("workspace_session_auto_{}", enrolled.prepare_intent_id),
    });
    let prepared_path = issue_root.join("automation-plan-intent.json");
    write_json(&prepared_path, &legacy_prepared).unwrap();
    let creates = std::cell::Cell::new(0u32);
    let error = store
        .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| {
            creates.set(creates.get() + 1);
            Ok(())
        })
        .unwrap_err();
    assert!(
        matches!(error, EnrollmentError::Conflict { .. })
            || matches!(error, EnrollmentError::InvalidScope(_)),
        "legacy prepared intent must fail closed: {error:?}"
    );
    assert_eq!(creates.get(), 0, "create callback must not run for legacy intents");
    assert_eq!(
        read_json::<serde_json::Value>(&prepared_path).unwrap(),
        legacy_prepared,
        "legacy intent file must stay untouched"
    );

    // 2) 旧格式 generation intent：读入不炸，claim fail-closed、不覆盖。
    store
        .bind_plan(
            "project_1",
            "issue_1",
            enrolled.policy_revision,
            "plan_bound",
            "session_bound",
        )
        .unwrap();
    let legacy_generation = serde_json::json!({
        "enrollment_id": enrolled.enrollment_id,
        "plan_id": "plan_bound",
        "session_id": "session_bound",
        "action_key": crate::product::models::automation::PlanGenerationIntent::action_key_for(
            &enrolled.enrollment_id,
            "plan_bound",
        ),
        "source": serde_json::to_value(source()).unwrap(),
        "options": serde_json::to_value(options()).unwrap(),
        "logical_repository_id": logical_repo(1).0.to_string(),
        "phase": "claimed",
    });
    let generation_path = issue_root.join("automation-generation-intent.json");
    write_json(&generation_path, &legacy_generation).unwrap();
    let error = store
        .claim_plan_generation("project_1", "issue_1", &enrolled.enrollment_id)
        .unwrap_err();
    assert!(
        matches!(error, EnrollmentError::Conflict { .. })
            || matches!(error, EnrollmentError::InvalidScope(_)),
        "legacy generation intent must fail closed: {error:?}"
    );
    assert_eq!(
        read_json::<serde_json::Value>(&generation_path).unwrap(),
        legacy_generation,
        "legacy generation intent must stay untouched"
    );

    // 3) 新格式意图 target 与 enrollment target 不一致：身份漂移拒绝。
    let mut drifted = legacy_generation.clone();
    let drifted_object = drifted.as_object_mut().unwrap();
    assert!(drifted_object.remove("logical_repository_id").is_some());
    drifted_object.insert(
        "target".to_string(),
        serde_json::json!({
            "kind": "logical_codebase",
            "logical_codebase_id": "logical_codebase_0001",
            "logical_repository_id": logical_repo(2).0.to_string(),
        }),
    );
    write_json(&generation_path, &drifted).unwrap();
    let error = store
        .claim_plan_generation("project_1", "issue_1", &enrolled.enrollment_id)
        .unwrap_err();
    assert!(
        matches!(error, EnrollmentError::Conflict { .. }),
        "drifted intent target must be rejected: {error:?}"
    );
    assert_eq!(
        read_json::<serde_json::Value>(&generation_path).unwrap(),
        drifted
    );
}
