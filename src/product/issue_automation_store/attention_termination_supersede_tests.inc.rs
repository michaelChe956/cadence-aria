// ---------------------------------------------------------------------------
// 块1 片2（REQ-WIGA-SWAP-01、REQ-WIGA-02 修订、REQ-WIGA-09 修订）：
// terminated 重新开启的 options 漂移分路径判定——正常重开按换代放行
// options 漂移（source/target 漂移仍拒）；悬空源重授权路径维持
// options/target 逐字段相等（REQ-DRG-18）；disabled 记录不走取代。
// 经 tests.rs `include!` 引入（large_file_guard 1200 行红线）。
// ---------------------------------------------------------------------------

/// ① 正常重开路径：terminated 记录上带新 provider 的 Enable（options 漂移）
/// 被接受并完成换代——新 options 生效、policy_revision+1、prepare_intent_id
/// 轮换、binding_version+1、旧代进 previous、旧代伴生意图文件退役。
#[test]
fn issue_automation_store_terminated_reenable_allows_provider_options_drift_and_supersedes() {
    let (tmp, store, binding, revision) = bound_enrollment_with_binding();
    let enrollment = store.get("project_1", "issue_1").unwrap().unwrap();
    // 旧代伴生意图文件：取代后必须退役。
    let issue_root = ProductAppPaths::new(tmp.path()).issue_root("project_1", "issue_1");
    let declared_target = enrollment.target.clone().expect("fixture declares target");
    write_json(
        &issue_root.join("automation-plan-intent.json"),
        &crate::product::models::automation::PreparedPlanIntent::from_enrollment(
            &enrollment,
            &declared_target,
        ),
    )
    .unwrap();
    write_json(
        &issue_root.join("automation-generation-intent.json"),
        &crate::product::models::automation::PlanGenerationIntent {
            enrollment_id: enrollment.enrollment_id.clone(),
            action_key: crate::product::models::automation::PlanGenerationIntent::action_key_for(
                &enrollment.enrollment_id,
                "plan_0001",
            ),
            plan_id: "plan_0001".to_string(),
            session_id: "session_0001".to_string(),
            source: enrollment.source.clone(),
            options: enrollment.options.clone(),
            target: declared_target.clone(),
            phase: crate::product::models::automation::PlanGenerationPhase::Claimed,
        },
    )
    .unwrap();
    store
        .terminate_plan(
            "project_1",
            "issue_1",
            EnrollmentTerminationRequest {
                command_id: "terminate-swap-1".to_string(),
                expected_enrollment_id: enrollment.enrollment_id.clone(),
                expected_policy_revision: revision,
                reason: "user requested termination".to_string(),
            },
        )
        .unwrap();
    let terminated = store.get("project_1", "issue_1").unwrap().unwrap();

    // 正常重开（source 未悬空）＋换 provider：取代生效并完成换代留痕。
    let mut swapped = reenable("reenable-swap-1", "human-choice-2");
    if let EnrollmentWriteCommand::Enable { options, .. } = &mut swapped {
        options.author_provider = ProviderName::Codex;
        options.reviewer_provider = ProviderName::Codex;
    }
    let replaced = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(terminated.policy_revision),
            swapped,
        )
        .expect("options drift supersedes on the normal re-enable path");
    assert!(replaced.enabled && replaced.terminated_at.is_none());
    assert_eq!(replaced.policy_revision, terminated.policy_revision + 1);
    assert_eq!(replaced.options.author_provider, ProviderName::Codex);
    assert_eq!(replaced.options.reviewer_provider, ProviderName::Codex);
    assert_ne!(
        replaced.prepare_intent_id, terminated.prepare_intent_id,
        "supersede rotates the prepare intent so the fresh chain never reuses the old plan"
    );
    let history = replaced
        .binding_history
        .as_ref()
        .expect("binding history survives the supersede");
    assert_eq!(history.current.binding_version, binding.binding_version + 1);
    assert_eq!(history.current.author_provider, ProviderName::Codex);
    assert!(
        history
            .previous
            .iter()
            .any(|old| old.binding_version == binding.binding_version
                && old.author_provider == ProviderName::Fake),
        "the superseded fake-provider generation stays read-only in previous"
    );
    // 旧代伴生意图/生成检查点随旧授权退役。
    assert!(!issue_root.join("automation-plan-intent.json").exists());
    assert!(
        !issue_root
            .join("automation-generation-intent.json")
            .exists()
    );
    assert_eq!(
        store.get("project_1", "issue_1").unwrap().unwrap(),
        replaced,
        "durable record matches the supersede projection"
    );
}

/// ② 悬空源重授权路径：options 漂移（换 provider）仍被拒——
/// `compare_and_set_reenable(..., true)` 维持 options/target 逐字段相等
/// （REQ-DRG-18 执行证据），拒绝零副作用。
#[test]
fn issue_automation_store_terminated_reenable_dangling_source_still_rejects_options_drift() {
    let tmp = tempfile::tempdir().unwrap();
    let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
    let created = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();
    store
        .terminate_plan(
            "project_1",
            "issue_1",
            EnrollmentTerminationRequest {
                command_id: "terminate-dangling-options".to_string(),
                expected_enrollment_id: created.enrollment_id.clone(),
                expected_policy_revision: created.policy_revision,
                reason: "cascade entity deletion".to_string(),
            },
        )
        .unwrap();
    let terminated = store.get("project_1", "issue_1").unwrap().unwrap();

    let mut drifted_options = reenable("reenable-dangling-options-1", "human-choice-2");
    if let EnrollmentWriteCommand::Enable { options, .. } = &mut drifted_options {
        options.author_provider = ProviderName::Codex;
        options.reviewer_provider = ProviderName::Codex;
    }
    assert_conflict(
        store
            .compare_and_set_reenable(
                "project_1",
                "issue_1",
                Some(terminated.policy_revision),
                drifted_options,
                true,
            )
            .unwrap_err(),
        terminated.policy_revision,
    );
    assert_eq!(
        store.get("project_1", "issue_1").unwrap().unwrap(),
        terminated,
        "the dangling-path rejection must be side-effect free"
    );
}

/// ③ disabled 记录（非 terminated）上的 Enable 不走取代——binding_version
/// 不变、prepare_intent_id 不轮换、旧 plan 绑定保留、旧代伴生意图文件
/// 不删（options 漂移在普通重开路径本就放行，但那是 revision+1 重开，
/// 不是换代——钉住「取代只发生在 terminated 记录上」）。
#[test]
fn issue_automation_store_terminated_reenable_on_disabled_record_does_not_supersede() {
    let (tmp, store, binding, revision) = bound_enrollment_with_binding();
    let enrollment = store.get("project_1", "issue_1").unwrap().unwrap();
    let issue_root = ProductAppPaths::new(tmp.path()).issue_root("project_1", "issue_1");
    let declared_target = enrollment.target.clone().expect("fixture declares target");
    write_json(
        &issue_root.join("automation-plan-intent.json"),
        &crate::product::models::automation::PreparedPlanIntent::from_enrollment(
            &enrollment,
            &declared_target,
        ),
    )
    .unwrap();
    write_json(
        &issue_root.join("automation-generation-intent.json"),
        &crate::product::models::automation::PlanGenerationIntent {
            enrollment_id: enrollment.enrollment_id.clone(),
            action_key: crate::product::models::automation::PlanGenerationIntent::action_key_for(
                &enrollment.enrollment_id,
                "plan_0001",
            ),
            plan_id: "plan_0001".to_string(),
            session_id: "session_0001".to_string(),
            source: enrollment.source.clone(),
            options: enrollment.options.clone(),
            target: declared_target,
            phase: crate::product::models::automation::PlanGenerationPhase::Claimed,
        },
    )
    .unwrap();

    // 普通 Disable（非 terminated），随后带 provider 漂移的 Enable＝普通重开。
    let disabled = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(revision),
            EnrollmentWriteCommand::Disable,
        )
        .unwrap();
    assert!(!disabled.enabled && disabled.terminated_at.is_none());
    let mut swapped = reenable("reenable-disabled-1", "human-choice-2");
    if let EnrollmentWriteCommand::Enable { options, .. } = &mut swapped {
        options.author_provider = ProviderName::Codex;
        options.reviewer_provider = ProviderName::Codex;
    }
    let reopened = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(disabled.policy_revision),
            swapped,
        )
        .expect("a disabled record re-enables without supersede");
    assert!(reopened.enabled && reopened.terminated_at.is_none());
    assert_eq!(reopened.policy_revision, disabled.policy_revision + 1);
    assert_eq!(reopened.options.author_provider, ProviderName::Codex);
    assert_eq!(
        reopened.prepare_intent_id, enrollment.prepare_intent_id,
        "plain reopen must not rotate the prepare intent"
    );
    assert_eq!(
        reopened.plan_id, enrollment.plan_id,
        "plain reopen keeps the bound plan"
    );
    assert_eq!(reopened.session_id, enrollment.session_id);
    let history = reopened
        .binding_history
        .as_ref()
        .expect("binding history survives the plain reopen");
    assert_eq!(
        history.current.binding_version, binding.binding_version,
        "plain reopen must not supersede the binding generation"
    );
    assert!(
        history.previous.is_empty(),
        "no generation is archived by a plain reopen"
    );
    assert!(
        issue_root.join("automation-plan-intent.json").exists(),
        "plain reopen must not retire the intent files"
    );
    assert!(
        issue_root
            .join("automation-generation-intent.json")
            .exists()
    );
}

// 以下一条为行数门禁溢出处置（块1 片2）：自 attention_termination_tests.inc.rs
// 整段零改写迁入（与 supersede 无关；同模块 include! 语义等价）。
/// 红测③：缺失 enrollment 的终止 NotFound；普通 Disable→Enable 重开回归
/// 保持（终止语义不波及非终止路径）。
#[test]
fn issue_automation_store_termination_missing_and_manual_disable_regression() {
    let tmp = tempfile::tempdir().unwrap();
    let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
    assert!(matches!(
        store
            .terminate_plan(
                "project_1",
                "issue_1",
                EnrollmentTerminationRequest {
                    command_id: "c".to_string(),
                    expected_enrollment_id: "e".to_string(),
                    expected_policy_revision: 1,
                    reason: String::new(),
                },
            )
            .unwrap_err(),
        EnrollmentError::NotFound
    ));
    drop(store);

    // 普通 Disable 后 Enable 重开（无 terminated_at）保持既有行为。
    let (_tmp2, store2, _binding2, revision2) = bound_enrollment_with_binding();
    let disabled = store2
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(revision2),
            EnrollmentWriteCommand::Disable,
        )
        .unwrap();
    assert!(!disabled.enabled);
    assert!(disabled.terminated_at.is_none());
    let reopened = store2
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(disabled.policy_revision),
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();
    assert!(reopened.enabled, "plain disable still allows re-enable");
}
