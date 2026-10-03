// Task 2（aggregate-policy-root-publication，REQ-BOOT-06）：唯一桩等待
// 谓词的专用测试。本文件经 bootstrap_tests.inc.rs 在 mod tests 内 include，
// 复用同模块的 readiness fixture 与审计链 helper。

/// 构造 revision 1 的合法真正文 artifact（根配方聚合政策，非自举桩）：
/// digest 按 store 的 canonical 规则计算（policy_text 原字节 SHA-256，
/// `sha256:` 前缀），与 `AggregatePolicyArtifact::compute_digest` 一致；
/// 正文与 `BOOTSTRAP_POLICY_TEXT` 完全不同。仅供原先必须 Completed/ready
/// 的对照分支显式准备真正文——负例 fixture 一律保留桩正文。
fn real_root_policy_revision_one(
    project_id: &str,
    logical_codebase_id: &str,
) -> crate::product::logical_codebase::policy::AggregatePolicyArtifact {
    use sha2::{Digest, Sha256};
    let policy_text =
        "# Aggregate policy (root recipe)\n\nRoot-recipe aggregated policy for planning read-only and coding target-write sessions.\n"
            .to_string();
    crate::product::logical_codebase::policy::AggregatePolicyArtifact {
        policy_id: format!("policy/{project_id}/{logical_codebase_id}/1"),
        project_id: project_id.to_string(),
        logical_codebase_id: logical_codebase_id.to_string(),
        revision: 1,
        digest: format!("sha256:{:x}", Sha256::digest(policy_text.as_bytes())),
        policy_text,
        created_at: "2026-10-02T00:00:00Z".to_string(),
    }
}

/// 补齐 aggregate index：使 AggregateIndexActive 步 Completed（另四步由
/// readiness fixture 满足），五步中仅 RulesPolicy 可能等待。
fn seed_active_aggregate_index(fixture: &ReadinessFixture, index_id: &str) {
    let index_store = AggregateIndexStore::for_lc(fixture.paths.clone(), &fixture.lc_id);
    index_store
        .create(
            "project_0001",
            AggregateIndexRecord::building(
                index_id.to_string(),
                "project_0001".to_string(),
                fixture.manifest.membership_revision,
                Vec::new(),
                "2026-10-02T00:20:00Z".to_string(),
            ),
        )
        .unwrap();
    index_store
        .mark_status("project_0001", index_id, AggregateIndexStatus::Active, None)
        .unwrap();
}

/// 写入一致 AGENTS.md 并以桩/真正文 digest finalize 最终 receipt（四条
/// 命令审计全部 Allowed），补齐三源一致的全部材料。
fn freeze_consistent_receipt(fixture: &ReadinessFixture) {
    std::fs::write(
        fixture.aggregate_root.join("AGENTS.md"),
        "# aggregate root rules\n",
    )
    .unwrap();
    let rule_digest = crate::product::logical_codebase::root_recipe_receipt::root_rule_digest(
        &std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
    )
    .unwrap()
    .expect("root rule digest");
    let receipt = finalize_root_receipt(
        &fixture.paths,
        &fixture.lc_id,
        &fixture.operation_id,
        &fixture.aggregate_root,
        &fixture.policy.digest,
        &rule_digest,
    );
    assert_eq!(receipt.policy_digest, fixture.policy.digest);
    assert_eq!(receipt.rule_digest, rule_digest);
}

/// Task 2（REQ-BOOT-06）：桩正文 + 已完成旧 operation + 四 Allowed
/// receipts + 一致 AGENTS 的三源一致证据，在升级后不再冒充就绪——唯一
/// 桩 guard 命中时 RulesPolicy 等待显式迁移。另四个 readiness 步骤保持
/// Completed；detail/notice 指向完整实际项目/LC 的初始化 API 并提醒新
/// idempotency_key、Completed 无可见启动按钮；GET 零写入、零 provider
/// 启动、零重建派发。
#[test]
fn bootstrap_placeholder_with_valid_receipt_waits_for_explicit_recipe_api() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let fixture = readiness_fixture("aggregate_initialization_placeholder_0001", false);
    assert!(
        fixture.policy.is_bootstrap_placeholder(),
        "precondition: the fixture policy must stay the bootstrap placeholder"
    );
    freeze_consistent_receipt(&fixture);
    seed_active_aggregate_index(&fixture, "aggregate_index_placeholder_0001");

    // provider/rebuild 通道计数：GET 投影不得触发 run 探针或重建派发。
    let probe_calls = std::sync::Arc::new(AtomicUsize::new(0));
    let rebuilds = std::sync::Arc::new(AtomicUsize::new(0));
    let _service = LogicalCodebaseBootstrapService::new(fixture.paths.clone())
        .with_member_index_run_probe({
            let probe_calls = probe_calls.clone();
            std::sync::Arc::new(move |_project: &str, _lc: &str, _operation: &str| {
                probe_calls.fetch_add(1, Ordering::SeqCst);
                true
            })
        })
        .with_aggregate_index_rebuild({
            let rebuilds = rebuilds.clone();
            std::sync::Arc::new(move |_project: &str, _command: &str, _revision: u64| {
                rebuilds.fetch_add(1, Ordering::SeqCst);
                Err(AggregateIndexError::Failed {
                    code: "unexpected_rebuild",
                    message: "placeholder waiting must not dispatch rebuilds".to_string(),
                })
            })
        });

    let before_inventory = full_tree_inventory(fixture.temp.path());
    let projection = fixture.project();
    let after_inventory = full_tree_inventory(fixture.temp.path());

    // 另四个 readiness 步骤全部 Completed：唯一缺口是桩 guard。
    for step in &projection.steps {
        if step.step != LogicalCodebaseBootstrapStep::RulesPolicy {
            assert_eq!(
                step.status,
                LogicalCodebaseBootstrapStepStatus::Completed,
                "step {} must stay completed",
                step.step.as_str()
            );
        }
    }
    let rules = ReadinessFixture::rules_step(&projection);
    assert_eq!(
        rules.status,
        LogicalCodebaseBootstrapStepStatus::WaitingForHuman
    );
    assert_eq!(
        rules.failure.as_ref().unwrap().reason_code,
        "aggregate_policy_bootstrap_placeholder"
    );
    assert!(!projection.planning_ready);
    assert!(rules.allowed_actions.contains(&BootstrapActionKind::Retry));
    assert!(
        rules
            .failure
            .as_ref()
            .unwrap()
            .detail
            .contains("/initializations")
    );
    assert!(
        rules
            .failure
            .as_ref()
            .unwrap()
            .detail
            .contains("idempotency_key")
    );
    assert_eq!(before_inventory, after_inventory);

    // detail/notice 指向完整实际项目/LC API，提醒新 key 且 Completed 无
    // UI 启动按钮；RulesPolicy 通用 Retry 不是根配方入口。
    let detail = &rules.failure.as_ref().unwrap().detail;
    let api = format!(
        "/api/projects/project_0001/logical-codebases/{}/initializations",
        fixture.lc_id
    );
    assert!(
        detail.contains(&api),
        "detail must cite the real API: {detail}"
    );
    assert!(detail.contains("新的 idempotency_key"));
    assert!(detail.contains("没有可见的启动按钮"));
    let notice = projection
        .notices
        .iter()
        .find(|notice| notice.step == LogicalCodebaseBootstrapStep::RulesPolicy)
        .expect("waiting rules_policy step must surface a notice");
    assert_eq!(notice.reason_code, "aggregate_policy_bootstrap_placeholder");
    assert!(notice.summary.contains(&api));
    assert_eq!(notice.external_side_effect, "none");

    assert_eq!(probe_calls.load(Ordering::SeqCst), 0);
    assert_eq!(rebuilds.load(Ordering::SeqCst), 0);
}

/// Task 2（REQ-BOOT-06）：合法真正文 revision 1 沿用原有全部判定——
/// 相同 receipt/root/索引事实下 RulesPolicy Completed 且 planning_ready
/// =true；系统不以 revision 是否为 1、receipt 年龄或额外迁移标记改变
/// 就绪结论（durable 事实零改动，无迁移标记写入）。
#[test]
fn bootstrap_non_placeholder_revision_one_keeps_existing_ready_predicates() {
    let fixture = readiness_fixture("aggregate_initialization_real_policy_0001", true);
    assert_eq!(fixture.policy.revision, 1);
    assert!(!fixture.policy.is_bootstrap_placeholder());
    freeze_consistent_receipt(&fixture);
    seed_active_aggregate_index(&fixture, "aggregate_index_real_policy_0001");

    let before_inventory = full_tree_inventory(fixture.temp.path());
    let projection = fixture.project();
    let after_inventory = full_tree_inventory(fixture.temp.path());

    let rules = ReadinessFixture::rules_step(&projection);
    assert_eq!(rules.status, LogicalCodebaseBootstrapStepStatus::Completed);
    assert!(rules.failure.is_none());
    assert_eq!(rules.object_id, fixture.policy.policy_id);
    for step in &projection.steps {
        assert_eq!(
            step.status,
            LogicalCodebaseBootstrapStepStatus::Completed,
            "step {} must complete for the readiness loop",
            step.step.as_str()
        );
    }
    assert!(projection.planning_ready);

    // 不添加迁移标记：投影零写入，真正文 revision 1 的 durable 事实
    // 原样保留，不产生 RulesPolicy 等待通知。
    assert_eq!(before_inventory, after_inventory);
    assert!(
        projection
            .notices
            .iter()
            .all(|notice| notice.step != LogicalCodebaseBootstrapStep::RulesPolicy)
    );
}

/// Task 2（REQ-BOOT-06）：桩正文不遮盖既有检查的优先级——生命周期分流
/// （Running/Failed/Cancelled）、缺件（receipt/根规则缺失）与漂移
/// （policy/rule/root）在桩在场时逐项保持原 status/reason_code。
#[test]
fn bootstrap_placeholder_preserves_existing_failure_precedence() {
    // 生命周期：Running 保持 Running（无 failure、无动作）。
    {
        let fixture = readiness_fixture_with_outcome(
            "aggregate_initialization_placeholder_running",
            false,
            FixtureOutcome::Running,
        );
        let projection = fixture.project();
        let rules = ReadinessFixture::rules_step(&projection);
        assert_eq!(rules.status, LogicalCodebaseBootstrapStepStatus::Running);
        assert!(rules.failure.is_none());
        assert!(!projection.planning_ready);
    }

    // 生命周期：Failed 保持 Failed 与 operation 的错误 reason。
    {
        let fixture = readiness_fixture_with_outcome(
            "aggregate_initialization_placeholder_failed",
            false,
            FixtureOutcome::Failed,
        );
        let projection = fixture.project();
        let rules = ReadinessFixture::rules_step(&projection);
        assert_eq!(rules.status, LogicalCodebaseBootstrapStepStatus::Failed);
        assert_eq!(
            rules.failure.as_ref().unwrap().reason_code,
            "root_recipe_probe_failed"
        );
        assert!(rules.allowed_actions.contains(&BootstrapActionKind::Retry));
        assert!(!projection.planning_ready);
    }

    // 生命周期：Cancelled 保持等待与 root_recipe_cancelled。
    {
        let fixture = readiness_fixture_with_outcome(
            "aggregate_initialization_placeholder_cancelled",
            false,
            FixtureOutcome::Cancelled,
        );
        let projection = fixture.project();
        let rules = ReadinessFixture::rules_step(&projection);
        assert_eq!(
            rules.status,
            LogicalCodebaseBootstrapStepStatus::WaitingForHuman
        );
        assert_eq!(
            rules.failure.as_ref().unwrap().reason_code,
            "root_recipe_cancelled"
        );
        assert!(!projection.planning_ready);
    }

    // 缺件：operation Completed 但最终 receipt 缺失 → root_receipt_missing。
    {
        let fixture = readiness_fixture("aggregate_initialization_placeholder_no_receipt", false);
        std::fs::write(
            fixture.aggregate_root.join("AGENTS.md"),
            "# aggregate root rules\n",
        )
        .unwrap();
        let projection = fixture.project();
        let rules = ReadinessFixture::rules_step(&projection);
        assert_eq!(
            rules.failure.as_ref().unwrap().reason_code,
            "root_receipt_missing"
        );
        assert!(!projection.planning_ready);
    }

    // 缺件：receipt 冻结后根规则被移除 → root_rule_missing。
    {
        let fixture = readiness_fixture("aggregate_initialization_placeholder_no_rule", false);
        freeze_consistent_receipt(&fixture);
        std::fs::remove_file(fixture.aggregate_root.join("AGENTS.md")).unwrap();
        let projection = fixture.project();
        let rules = ReadinessFixture::rules_step(&projection);
        assert_eq!(
            rules.failure.as_ref().unwrap().reason_code,
            "root_rule_missing"
        );
        assert!(!projection.planning_ready);
    }

    // 漂移：正文升级后 receipt 冻结摘要失配 → policy_digest_drift。
    {
        let fixture = readiness_fixture("aggregate_initialization_placeholder_policy_drift", false);
        freeze_consistent_receipt(&fixture);
        let revised = fixture.policy.with_revised_policy(
            "# Aggregate policy (revised)\n",
            "2026-10-02T00:30:00Z".to_string(),
        );
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            fixture.paths.clone(),
            &fixture.lc_id,
        )
        .save("project_0001", &revised)
        .unwrap();
        let projection = fixture.project();
        let rules = ReadinessFixture::rules_step(&projection);
        assert_eq!(
            rules.failure.as_ref().unwrap().reason_code,
            "policy_digest_drift"
        );
        assert!(!projection.planning_ready);
    }

    // 漂移：根规则内容漂移 → rule_digest_drift。
    {
        let fixture = readiness_fixture("aggregate_initialization_placeholder_rule_drift", false);
        freeze_consistent_receipt(&fixture);
        std::fs::write(
            fixture.aggregate_root.join("AGENTS.md"),
            "# aggregate root rules (drifted)\n",
        )
        .unwrap();
        let projection = fixture.project();
        let rules = ReadinessFixture::rules_step(&projection);
        assert_eq!(
            rules.failure.as_ref().unwrap().reason_code,
            "rule_digest_drift"
        );
        assert!(!projection.planning_ready);
    }

    // 漂移：manifest 指向别的 root → root_receipt_authority_drift。
    {
        let fixture = readiness_fixture("aggregate_initialization_placeholder_root_drift", false);
        freeze_consistent_receipt(&fixture);
        let other_root = fixture.temp.path().join("other-root");
        std::fs::create_dir_all(&other_root).unwrap();
        let mut moved = fixture.manifest.clone();
        moved.provider_context_root = other_root;
        LogicalCodebaseStore::for_lc(fixture.paths.clone(), &fixture.lc_id)
            .save_manifest("project_0001", &moved)
            .unwrap();
        let projection = fixture.project();
        let rules = ReadinessFixture::rules_step(&projection);
        assert_eq!(
            rules.failure.as_ref().unwrap().reason_code,
            "root_receipt_authority_drift"
        );
        assert!(!projection.planning_ready);
    }
}
