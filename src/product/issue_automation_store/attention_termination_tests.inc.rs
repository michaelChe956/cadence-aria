// ---------------------------------------------------------------------------
// 批 2 N0（REQ-WIGA-09/12）：自动化待处理事实 store 与 terminated_at 纯字段。
// ---------------------------------------------------------------------------

/// N0 测试共享现场：enable＋bind 后返回当前 durable binding 与 revision。
pub(crate) fn bound_enrollment_with_binding() -> (
    tempfile::TempDir,
    IssueAutomationStore,
    EnrollmentBindingIdentity,
    u64,
) {
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
    let bound = store
        .bind_plan(
            "project_1",
            "issue_1",
            created.policy_revision,
            "plan_0001",
            "session_0001",
        )
        .unwrap();
    let binding = bound
        .binding_history
        .clone()
        .expect("binding history present after bind")
        .current;
    (tmp, store, binding, bound.policy_revision)
}

fn attention_fact(key: &str, occurred_at: &str) -> AutomationAttentionFact {
    AutomationAttentionFact {
        key: key.to_string(),
        kind: AutomationAttentionKind::AutomationFailed,
        stage: AutomationAttentionStage::Advance,
        plan_id: Some("plan_0001".to_string()),
        session_id: Some("session_0001".to_string()),
        attempt_id: None,
        reason_code: "initialization_failed".to_string(),
        occurred_at: occurred_at.to_string(),
        choice_id: None,
        gate_id: None,
    }
}

fn assert_invalid_scope(error: EnrollmentError) {
    assert!(
        matches!(error, EnrollmentError::InvalidScope(_)),
        "expected InvalidScope, got {error:?}"
    );
}

fn enrollment_json_path(tmp: &tempfile::TempDir) -> std::path::PathBuf {
    ProductAppPaths::new(tmp.path())
        .issue_root("project_1", "issue_1")
        .join("automation-enrollment.json")
}

/// 红测①：精确绑定写/清；同 key 不增项、occurred_at 首次冻结（重入
/// 保留）；无 checkpoint 不伪造 attempt；事实写不递增 policy_revision。
#[test]
fn issue_automation_store_upsert_attention_fact_writes_resolves_and_freezes_time() {
    let (_tmp, store, binding, revision) = bound_enrollment_with_binding();
    let mut fact = attention_fact("plan-q:choice-1", "2026-10-16T00:00:00Z");
    fact.kind = AutomationAttentionKind::PlanQuestion;
    fact.stage = AutomationAttentionStage::Plan;
    fact.attempt_id = Some("attempt_0001".to_string());
    fact.choice_id = Some("choice_0001".to_string());
    fact.gate_id = Some("gate_0001".to_string());
    store
        .upsert_automation_attention_fact("project_1", "issue_1", &binding, fact.clone())
        .unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(saved.attention_facts, vec![fact.clone()]);
    assert_eq!(
        saved.policy_revision, revision,
        "事实写不得递增 policy_revision"
    );

    // 同 key 重入：不增项，occurred_at 首次冻结，reason 取最新。
    let mut again = attention_fact("plan-q:choice-1", "2026-10-16T09:00:00Z");
    again.kind = AutomationAttentionKind::PlanQuestion;
    again.stage = AutomationAttentionStage::Plan;
    again.attempt_id = Some("attempt_0001".to_string());
    again.reason_code = "trust_waiting_again".to_string();
    store
        .upsert_automation_attention_fact("project_1", "issue_1", &binding, again)
        .unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(saved.attention_facts.len(), 1, "同 key 重复 upsert 不增项");
    assert_eq!(
        saved.attention_facts[0].occurred_at, "2026-10-16T00:00:00Z",
        "occurred_at 首次冻结，重入保留"
    );
    assert_eq!(saved.attention_facts[0].reason_code, "trust_waiting_again");
    assert_eq!(saved.policy_revision, revision);

    // 无 checkpoint 的 advance 失败事实：attempt 身份不伪造（None 原样）。
    let checkpointless = attention_fact("cmd-1:advance:retry", "2026-10-16T00:00:00Z");
    store
        .upsert_automation_attention_fact("project_1", "issue_1", &binding, checkpointless.clone())
        .unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(saved.attention_facts.len(), 2);
    assert_eq!(saved.attention_facts[1], checkpointless);

    // 精确绑定清 key；重复清幂等成功。
    store
        .resolve_automation_attention_fact("project_1", "issue_1", &binding, "plan-q:choice-1")
        .unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(saved.attention_facts, vec![checkpointless]);
    assert_eq!(saved.policy_revision, revision);
    store
        .resolve_automation_attention_fact("project_1", "issue_1", &binding, "plan-q:choice-1")
        .unwrap();
}

/// 红测③/④：旧/别的绑定拒绝；事实身份与绑定互证失败 InvalidScope；
/// disabled 未终止阻断写/清；缺失与错误上抛不吞。
#[test]
fn issue_automation_store_attention_fact_requires_exact_binding() {
    let (_tmp, store, binding, revision) = bound_enrollment_with_binding();
    let fact = attention_fact("k1", "2026-10-16T00:00:00Z");

    // 别的 enrollment 身份：写/清均拒绝。
    let mut foreign = binding.clone();
    foreign.enrollment_id = "other_enrollment".to_string();
    assert_conflict(
        store
            .upsert_automation_attention_fact("project_1", "issue_1", &foreign, fact.clone())
            .unwrap_err(),
        revision,
    );
    assert_conflict(
        store
            .resolve_automation_attention_fact("project_1", "issue_1", &foreign, "k1")
            .unwrap_err(),
        revision,
    );

    // 事实身份与绑定互证失败：plan/session 漂移 InvalidScope。
    let mut drift = attention_fact("k2", "2026-10-16T00:00:00Z");
    drift.plan_id = Some("plan_other".to_string());
    assert_invalid_scope(
        store
            .upsert_automation_attention_fact("project_1", "issue_1", &binding, drift)
            .unwrap_err(),
    );
    let mut drift = attention_fact("k3", "2026-10-16T00:00:00Z");
    drift.session_id = Some("session_other".to_string());
    assert_invalid_scope(
        store
            .upsert_automation_attention_fact("project_1", "issue_1", &binding, drift)
            .unwrap_err(),
    );

    // 换代后旧代 binding 写/清拒绝。
    let bound = store.get("project_1", "issue_1").unwrap().unwrap();
    store
        .rebind(
            "project_1",
            "issue_1",
            rebind_request(
                "cmd-rebind-1",
                bound.policy_revision,
                binding.binding_version,
                "plan_0002",
                "session_0002",
                target_logical(),
            ),
        )
        .unwrap();
    let after = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_conflict(
        store
            .upsert_automation_attention_fact("project_1", "issue_1", &binding, fact)
            .unwrap_err(),
        after.policy_revision,
    );
    assert_conflict(
        store
            .resolve_automation_attention_fact("project_1", "issue_1", &binding, "k1")
            .unwrap_err(),
        after.policy_revision,
    );

    // disabled 未终止：同一绑定写/清一律 Conflict（终止收尾除外，见下个测试）。
    let v2 = after.binding_history.clone().unwrap().current;
    let mut fact_v2 = attention_fact("k1", "2026-10-16T00:00:00Z");
    fact_v2.plan_id = Some("plan_0002".to_string());
    fact_v2.session_id = Some("session_0002".to_string());
    store
        .upsert_automation_attention_fact("project_1", "issue_1", &v2, fact_v2.clone())
        .unwrap();
    let before_disable = store.get("project_1", "issue_1").unwrap().unwrap();
    store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(before_disable.policy_revision),
            EnrollmentWriteCommand::Disable,
        )
        .unwrap();
    let disabled = store.get("project_1", "issue_1").unwrap().unwrap();
    assert!(!disabled.enabled);
    assert_eq!(disabled.terminated_at, None);
    assert_conflict(
        store
            .upsert_automation_attention_fact("project_1", "issue_1", &v2, fact_v2)
            .unwrap_err(),
        disabled.policy_revision,
    );
    assert_conflict(
        store
            .resolve_automation_attention_fact("project_1", "issue_1", &v2, "k1")
            .unwrap_err(),
        disabled.policy_revision,
    );

    // enrollment 缺失：NotFound 上抛不吞。
    assert!(matches!(
        store
            .upsert_automation_attention_fact(
                "project_1",
                "issue_9",
                &v2,
                attention_fact("k9", "t"),
            )
            .unwrap_err(),
        EnrollmentError::NotFound
    ));
    // 非法路径 id：Store 错误上抛。
    assert!(matches!(
        store
            .upsert_automation_attention_fact(
                "../escape",
                "issue_1",
                &v2,
                attention_fact("k9", "t"),
            )
            .unwrap_err(),
        EnrollmentError::Store(_)
    ));
    assert!(matches!(
        store
            .resolve_automation_attention_fact("project_1", "../escape", &v2, "k9")
            .unwrap_err(),
        EnrollmentError::Store(_)
    ));
}

/// 红测③：同一已 terminated 绑定（enabled=false）可清旧事实/写
/// termination 清理失败，不因 enabled=false 阻断终止收尾；重清成功即消失。
#[test]
fn issue_automation_store_attention_fact_terminated_binding_clears_and_writes() {
    let (tmp, store, binding, _revision) = bound_enrollment_with_binding();
    store
        .upsert_automation_attention_fact(
            "project_1",
            "issue_1",
            &binding,
            attention_fact("plan-q1", "2026-10-16T00:00:00Z"),
        )
        .unwrap();

    // 模拟 T1 终止 CAS 的 durable 结果：terminated_at 落盘＋enabled=false
    //（N0 只提供纯字段，写入归 T1）。
    let mut terminated = store.get("project_1", "issue_1").unwrap().unwrap();
    terminated.enabled = false;
    terminated.terminated_at = Some("2026-10-16T01:00:00Z".to_string());
    write_json(&enrollment_json_path(&tmp), &terminated).unwrap();

    // 同一 terminated 绑定：清旧 plan 事实＋写 termination 清理失败均放行。
    store
        .resolve_automation_attention_fact("project_1", "issue_1", &binding, "plan-q1")
        .unwrap();
    let mut cleanup = attention_fact("terminate:cleanup", "2026-10-16T02:00:00Z");
    cleanup.kind = AutomationAttentionKind::AutomationFailed;
    cleanup.stage = AutomationAttentionStage::Termination;
    store
        .upsert_automation_attention_fact("project_1", "issue_1", &binding, cleanup.clone())
        .unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(saved.attention_facts, vec![cleanup.clone()]);
    assert_eq!(saved.terminated_at.as_deref(), Some("2026-10-16T01:00:00Z"));

    // 重清成功即消失（幂等）。
    store
        .resolve_automation_attention_fact("project_1", "issue_1", &binding, "terminate:cleanup")
        .unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert!(saved.attention_facts.is_empty());
}

/// 红测②：旧 JSON 无 attention_facts/terminated_at 键（及显式空数组/
/// null、pre-C1 无 binding_history）均可读；旧代无绑定事实写 fail-closed。
#[test]
fn issue_automation_store_reads_legacy_json_without_attention_fields() {
    let (tmp, store, binding, _revision) = bound_enrollment_with_binding();
    let path = enrollment_json_path(&tmp);
    let current = store.get("project_1", "issue_1").unwrap().unwrap();

    // 批 2 前旧 wire：无 attention_facts/terminated_at 键。
    let mut legacy = serde_json::to_value(&current).unwrap();
    let object = legacy.as_object_mut().unwrap();
    object.remove("attention_facts");
    object.remove("terminated_at");
    write_json(&path, &legacy).unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert!(saved.attention_facts.is_empty());
    assert_eq!(saved.terminated_at, None);

    // 显式空数组/null 同样读为空/None。
    let mut explicit = serde_json::to_value(&current).unwrap();
    let object = explicit.as_object_mut().unwrap();
    object.insert("attention_facts".to_string(), serde_json::json!([]));
    object.insert("terminated_at".to_string(), serde_json::Value::Null);
    write_json(&path, &explicit).unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert!(saved.attention_facts.is_empty());
    assert_eq!(saved.terminated_at, None);

    // 更旧的 pre-C1 wire（无 binding_history）：读仍成功；事实写按
    // InvalidScope fail-closed（旧代无绑定不驱动自动链）。
    let mut pre_c1 = serde_json::to_value(&current).unwrap();
    pre_c1.as_object_mut().unwrap().remove("binding_history");
    write_json(&path, &pre_c1).unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert!(saved.binding_history.is_none());
    assert!(saved.attention_facts.is_empty());
    assert_eq!(saved.terminated_at, None);
    assert_invalid_scope(
        store
            .upsert_automation_attention_fact(
                "project_1",
                "issue_1",
                &binding,
                attention_fact("k9", "t"),
            )
            .unwrap_err(),
    );
}

/// 红测④：损坏 JSON fail-closed，读错误原样上抛（写/清两入口）。
#[test]
fn issue_automation_store_attention_fact_corrupt_json_propagates() {
    let (tmp, store, binding, _revision) = bound_enrollment_with_binding();
    std::fs::write(enrollment_json_path(&tmp), "{ not json").unwrap();
    assert!(matches!(
        store
            .upsert_automation_attention_fact(
                "project_1",
                "issue_1",
                &binding,
                attention_fact("k", "t"),
            )
            .unwrap_err(),
        EnrollmentError::Store(_)
    ));
    assert!(matches!(
        store
            .resolve_automation_attention_fact("project_1", "issue_1", &binding, "k")
            .unwrap_err(),
        EnrollmentError::Store(_)
    ));
}

// ---------------------------------------------------------------------------
// 批 2 T1（REQ-WIGA-09）：终止 CAS、命令账本与不可恢复准入。
// ---------------------------------------------------------------------------

/// 红测①：terminate CAS——锁内 terminated_at＋enabled=false、revision+1、
/// 命令账本 Accepted；同 command 同 payload 重入 Replayed（零写入、
/// terminated_at 冻结）；异 payload/漂移 revision 拒绝且零副作用。
#[test]
fn issue_automation_store_terminate_plan_cas_and_idempotent_replay() {
    let (_tmp, store, _binding, revision) = bound_enrollment_with_binding();
    let request = EnrollmentTerminationRequest {
        command_id: "terminate-command-1".to_string(),
        expected_enrollment_id: store
            .get("project_1", "issue_1")
            .unwrap()
            .expect("enrollment exists")
            .enrollment_id,
        expected_policy_revision: revision,
        reason: "user requested termination".to_string(),
    };
    let result = store
        .terminate_plan("project_1", "issue_1", request.clone())
        .expect("first terminate applies");
    assert_eq!(result.state, OperationState::Accepted);
    let terminated = result.enrollment;
    assert!(!terminated.enabled, "terminated must disable enrollment");
    let terminated_at = terminated.terminated_at.clone().expect("terminated_at set");
    assert_eq!(result.terminated_at, terminated_at);
    assert_eq!(terminated.policy_revision, revision + 1);
    assert!(
        terminated
            .command_ledger
            .iter()
            .any(|entry| entry.command_id == "terminate-command-1"
                && entry.state == OperationState::Accepted),
        "termination command must be ledgered"
    );

    // 同命令同 payload 重入：Replayed，零写入（terminated_at/revision 冻结）。
    let replay = store
        .terminate_plan("project_1", "issue_1", request)
        .expect("same-command replay is idempotent");
    assert_eq!(replay.state, OperationState::Replayed);
    assert_eq!(replay.terminated_at, terminated_at);
    assert_eq!(replay.enrollment.policy_revision, revision + 1);
    assert_eq!(replay.enrollment, terminated);

    // 异 payload（同 command_id）：fail-closed Conflict。
    let divergent = EnrollmentTerminationRequest {
        command_id: "terminate-command-1".to_string(),
        expected_enrollment_id: terminated.enrollment_id.clone(),
        expected_policy_revision: revision + 1,
        reason: "divergent payload".to_string(),
    };
    assert_conflict(
        store
            .terminate_plan("project_1", "issue_1", divergent)
            .unwrap_err(),
        revision + 1,
    );

    // 预期 revision 漂移：Conflict，零副作用。
    let stale = EnrollmentTerminationRequest {
        command_id: "terminate-command-2".to_string(),
        expected_enrollment_id: terminated.enrollment_id.clone(),
        expected_policy_revision: revision,
        reason: "stale revision".to_string(),
    };
    assert_conflict(
        store
            .terminate_plan("project_1", "issue_1", stale)
            .unwrap_err(),
        revision + 1,
    );
    let after = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(after, terminated, "rejected terminate must not mutate");
}

/// 红测②：terminated 不可恢复——Enable（同/异 payload）、Disable、rebind、
/// bind_plan 全拒且零副作用；异 enrollment_id 终止也拒。N0 口径对齐：同一
/// terminated 绑定仍可写/清终止收尾事实（不被准入拒绝阻断）。
#[test]
fn issue_automation_store_terminated_enrollment_is_irrecoverable() {
    let (_tmp, store, binding, revision) = bound_enrollment_with_binding();
    let enrollment = store.get("project_1", "issue_1").unwrap().unwrap();
    store
        .terminate_plan(
            "project_1",
            "issue_1",
            EnrollmentTerminationRequest {
                command_id: "terminate-command-1".to_string(),
                expected_enrollment_id: enrollment.enrollment_id.clone(),
                expected_policy_revision: revision,
                reason: "user requested termination".to_string(),
            },
        )
        .unwrap();
    let terminated = store.get("project_1", "issue_1").unwrap().unwrap();

    // 缺稳定 command_id 的隐式 Enable → InvalidScope（422 面）零副作用；
    // 人工显式取代（带 command_id）见取代专项红测。
    assert_invalid_scope(
        store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(terminated.policy_revision),
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap_err(),
    );
    // Disable 也拒（terminated 即终态，不再接受任何写命令）。
    assert_conflict(
        store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(terminated.policy_revision),
                EnrollmentWriteCommand::Disable,
            )
            .unwrap_err(),
        terminated.policy_revision,
    );
    // rebind → Conflict。
    assert_conflict(
        store
            .rebind(
                "project_1",
                "issue_1",
                rebind_request(
                    "rebind-command-9",
                    terminated.policy_revision,
                    binding.binding_version,
                    "plan_0002",
                    "session_0002",
                    target_logical_at(logical_repo(1)),
                ),
            )
            .unwrap_err(),
        terminated.policy_revision,
    );
    // bind_plan → Conflict。
    assert_conflict(
        store
            .bind_plan(
                "project_1",
                "issue_1",
                terminated.policy_revision,
                "plan_0003",
                "session_0003",
            )
            .unwrap_err(),
        terminated.policy_revision,
    );
    // 异 enrollment_id 的终止命令 → Conflict（身份互证）。
    assert_conflict(
        store
            .terminate_plan(
                "project_1",
                "issue_1",
                EnrollmentTerminationRequest {
                    command_id: "terminate-command-8".to_string(),
                    expected_enrollment_id: "another_enrollment".to_string(),
                    expected_policy_revision: terminated.policy_revision,
                    reason: "wrong identity".to_string(),
                },
            )
            .unwrap_err(),
        terminated.policy_revision,
    );
    // 零副作用：durable 记录未被上述任一命令改写。
    assert_eq!(
        store.get("project_1", "issue_1").unwrap().unwrap(),
        terminated
    );

    // N0 口径对齐：同一 terminated 绑定放行写/清终止收尾事实。
    store
        .upsert_automation_attention_fact(
            "project_1",
            "issue_1",
            &binding,
            AutomationAttentionFact {
                key: "termination_cleanup".to_string(),
                kind: AutomationAttentionKind::AutomationFailed,
                stage: AutomationAttentionStage::Termination,
                plan_id: Some("plan_0001".to_string()),
                session_id: Some("session_0001".to_string()),
                attempt_id: None,
                reason_code: "cleanup_failed".to_string(),
                occurred_at: "2026-10-16T00:00:00Z".to_string(),
                choice_id: None,
                gate_id: None,
            },
        )
        .expect("terminated binding may still record termination facts");
    store
        .resolve_automation_attention_fact("project_1", "issue_1", &binding, "termination_cleanup")
        .expect("terminated binding may still resolve facts");
}

/// 红测⑧（REQ-WIGA-09 修订）：自动路径（`ensure_plan_binding`＝reconcile
/// 派生 plan/session 的唯一入口）不得复活 terminated enrollment——即使
/// 人工取代入口已放开，自动链仍 Conflict 零副作用：create 回调不被调用
///（无自愈循环）、不落绑定、revision 不动。
#[test]
fn issue_automation_store_terminated_enrollment_blocks_automatic_chain() {
    let (_tmp, store, _binding, revision) = bound_enrollment_with_binding();
    let enrollment = store.get("project_1", "issue_1").unwrap().unwrap();
    store
        .terminate_plan(
            "project_1",
            "issue_1",
            EnrollmentTerminationRequest {
                command_id: "terminate-command-auto".to_string(),
                expected_enrollment_id: enrollment.enrollment_id.clone(),
                expected_policy_revision: revision,
                reason: "user requested termination".to_string(),
            },
        )
        .unwrap();
    let terminated = store.get("project_1", "issue_1").unwrap().unwrap();

    let creates = std::sync::atomic::AtomicUsize::new(0);
    assert_conflict(
        store
            .ensure_plan_binding("project_1", "issue_1", &terminated.enrollment_id, |_, _| {
                creates.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
            .unwrap_err(),
        terminated.policy_revision,
    );
    assert_eq!(
        creates.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "automatic chain must not derive a fresh plan for a terminated enrollment"
    );
    assert_eq!(
        store.get("project_1", "issue_1").unwrap().unwrap(),
        terminated,
        "automatic reconcile leaves the terminated record untouched"
    );
}

/// 红测⑤（REQ-WIGA-09 修订）：terminated 的人工显式 Enable 取代——同
/// enrollment_id、policy_revision+1、terminated_at 清空、plan/session 解绑、
/// prepare_intent_id 轮换（新链绝不复用被终止 plan）、binding 换代（旧代
/// current 进 previous、binding_version+1、plan/session 空串占位）、命令
/// 账本 Accepted 留痕；旧代伴生意图文件随旧授权退役。
#[test]
fn issue_automation_store_terminated_human_enable_supersedes_and_ledgers() {
    let (tmp, store, binding, revision) = bound_enrollment_with_binding();
    let enrollment = store.get("project_1", "issue_1").unwrap().unwrap();
    // 终止前留一个旧代伴生意图文件：取代后必须退役（否则 ensure 身份
    // 比对 fail-closed 卡死新链）。
    let issue_root = ProductAppPaths::new(tmp.path()).issue_root("project_1", "issue_1");
    let target = enrollment.target.clone().expect("fixture declares target");
    write_json(
        &issue_root.join("automation-plan-intent.json"),
        &crate::product::models::automation::PreparedPlanIntent::from_enrollment(
            &enrollment,
            &target,
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
            target: target.clone(),
            phase: crate::product::models::automation::PlanGenerationPhase::Claimed,
        },
    )
    .unwrap();
    store
        .terminate_plan(
            "project_1",
            "issue_1",
            EnrollmentTerminationRequest {
                command_id: "terminate-command-1".to_string(),
                expected_enrollment_id: enrollment.enrollment_id.clone(),
                expected_policy_revision: revision,
                reason: "user requested termination".to_string(),
            },
        )
        .unwrap();
    let terminated = store.get("project_1", "issue_1").unwrap().unwrap();
    assert!(terminated.terminated_at.is_some());

    // 人工显式取代：稳定 command_id＋新 selection_key＋当前 revision＋
    // 与终止记录逐字段相等的 source/options/target。
    let replaced = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(terminated.policy_revision),
            reenable("reenable-command-1", "human-choice-2"),
        )
        .expect("human enable supersedes the terminated enrollment");
    assert!(replaced.enabled);
    assert_eq!(replaced.terminated_at, None);
    assert_eq!(replaced.policy_revision, terminated.policy_revision + 1);
    assert_eq!(
        replaced.enrollment_id, terminated.enrollment_id,
        "supersede keeps the enrollment identity"
    );
    assert_ne!(
        replaced.prepare_intent_id, terminated.prepare_intent_id,
        "prepare intent must rotate so the fresh chain never reuses the terminated plan"
    );
    assert_eq!(replaced.plan_id, None, "superseded plan binding is cleared");
    assert_eq!(replaced.session_id, None);
    assert!(
        replaced.attention_facts.is_empty(),
        "superseded attention facts are retired with the old authorization"
    );
    let history = replaced
        .binding_history
        .as_ref()
        .expect("binding history survives the supersede");
    assert_eq!(history.current.binding_version, binding.binding_version + 1);
    assert_eq!(history.current.enrollment_id, terminated.enrollment_id);
    assert_eq!(history.current.plan_id, "");
    assert_eq!(history.current.session_id, "");
    assert!(
        history
            .previous
            .iter()
            .any(|old| old.binding_version == binding.binding_version
                && old.plan_id == binding.plan_id),
        "the superseded generation stays read-only in previous"
    );
    let ledger = replaced
        .command_ledger
        .iter()
        .find(|entry| entry.command_id == "reenable-command-1")
        .expect("the re-enable is ledgered");
    assert_eq!(ledger.state, OperationState::Accepted);
    assert_eq!(ledger.binding_version, binding.binding_version + 1);
    // 旧代伴生意图/生成检查点退役：新链由 ensure 重新派生。
    assert!(!issue_root.join("automation-plan-intent.json").exists());
    assert!(
        !issue_root
            .join("automation-generation-intent.json")
            .exists()
    );
    // durable 落盘核对。
    assert_eq!(
        store.get("project_1", "issue_1").unwrap().unwrap(),
        replaced
    );
}

/// 红测⑥（REQ-WIGA-09 修订）：取代的命令身份幂等与拒绝面——同
/// command_id＋同 payload 重放返回首次 durable 结果（零写入、revision
/// 不再 +1、账本不新增）；同 command_id 异 payload/异 expected_revision
/// → Conflict；缺 command_id → InvalidScope；expected_revision None/stale
/// 或 source/options/target 不等或 Disable → Conflict 零副作用。
#[test]
fn issue_automation_store_terminated_reenable_replay_and_rejections() {
    let (_tmp, store, _binding, revision) = bound_enrollment_with_binding();
    let enrollment = store.get("project_1", "issue_1").unwrap().unwrap();
    store
        .terminate_plan(
            "project_1",
            "issue_1",
            EnrollmentTerminationRequest {
                command_id: "terminate-command-1".to_string(),
                expected_enrollment_id: enrollment.enrollment_id.clone(),
                expected_policy_revision: revision,
                reason: "user requested termination".to_string(),
            },
        )
        .unwrap();
    let terminated = store.get("project_1", "issue_1").unwrap().unwrap();

    // expected_revision=None → Conflict（人工显式必须携带当前 revision）。
    assert_conflict(
        store
            .compare_and_set(
                "project_1",
                "issue_1",
                None,
                reenable("reenable-command-1", "human-choice-2"),
            )
            .unwrap_err(),
        terminated.policy_revision,
    );
    // stale revision → Conflict。
    assert_conflict(
        store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(terminated.policy_revision - 1),
                reenable("reenable-command-1", "human-choice-2"),
            )
            .unwrap_err(),
        terminated.policy_revision,
    );
    // source 漂移 → Conflict（改授权须先同 payload 取代再 Disable→Enable）。
    let mut drifted_source = reenable("reenable-command-1", "human-choice-2");
    if let EnrollmentWriteCommand::Enable { source, .. } = &mut drifted_source {
        source.stories.push(SourceRevisionRef {
            id: "story_spec_0002".into(),
            version: 1,
        });
    }
    assert_conflict(
        store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(terminated.policy_revision),
                drifted_source,
            )
            .unwrap_err(),
        terminated.policy_revision,
    );
    // options 漂移 → Conflict。
    let mut drifted_options = reenable("reenable-command-1", "human-choice-2");
    if let EnrollmentWriteCommand::Enable { options, .. } = &mut drifted_options {
        options.review_rounds = 9;
    }
    assert_conflict(
        store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(terminated.policy_revision),
                drifted_options,
            )
            .unwrap_err(),
        terminated.policy_revision,
    );
    // target 漂移 → Conflict。
    let mut drifted_target = reenable("reenable-command-1", "human-choice-2");
    if let EnrollmentWriteCommand::Enable { target, .. } = &mut drifted_target {
        *target = target_logical_at(logical_repo(2));
    }
    assert_conflict(
        store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(terminated.policy_revision),
                drifted_target,
            )
            .unwrap_err(),
        terminated.policy_revision,
    );
    // 上述拒绝全部零副作用。
    assert_eq!(
        store.get("project_1", "issue_1").unwrap().unwrap(),
        terminated
    );

    // 首次取代成功。
    let replaced = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(terminated.policy_revision),
            reenable("reenable-command-1", "human-choice-2"),
        )
        .expect("first supersede applies");
    // 同 command_id＋同 payload＋同 expected_revision 重放 → 首次 durable
    // 结果的当前投影（零写入；不再 +1、账本不新增）。
    let replay = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(terminated.policy_revision),
            reenable("reenable-command-1", "human-choice-2"),
        )
        .expect("same-command replay is idempotent");
    assert_eq!(replay, replaced);
    assert_eq!(
        store.get("project_1", "issue_1").unwrap().unwrap(),
        replaced,
        "replay must not mutate the durable record"
    );
    // 同 command_id 异 payload（selection_key 属 payload）→ Conflict。
    assert_conflict(
        store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(terminated.policy_revision),
                reenable("reenable-command-1", "human-choice-3"),
            )
            .unwrap_err(),
        replaced.policy_revision,
    );
    // 同 command_id 异 expected_revision → Conflict。
    assert_conflict(
        store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(replaced.policy_revision),
                reenable("reenable-command-1", "human-choice-2"),
            )
            .unwrap_err(),
        replaced.policy_revision,
    );
    // durable 仍未被改写。
    assert_eq!(
        store.get("project_1", "issue_1").unwrap().unwrap(),
        replaced
    );
}

/// 红测⑦（REQ-WIGA-09 修订）：非 terminated 的 Enable 携带稳定
/// command_id → 命令账本留痕且同命令同 payload 重放幂等（零写入）；
/// 缺 command_id → 旧行为（成功、不进账本），旧客户端不受影响。
#[test]
fn issue_automation_store_enable_with_command_id_ledgers_outside_terminated() {
    let tmp = tempfile::tempdir().unwrap();
    // 首次 Enable（另一 issue）携带 command_id → 成功且账本留痕。
    let fresh = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
    let created = fresh
        .compare_and_set(
            "project_1",
            "issue_2",
            None,
            reenable("enable-command-2", "human-choice-9"),
        )
        .expect("plain enable with a command_id still succeeds");
    assert!(created.enabled);
    assert!(
        created
            .command_ledger
            .iter()
            .any(|entry| entry.command_id == "enable-command-2"
                && entry.state == OperationState::Accepted),
        "explicit command_id is ledgered outside the terminated path"
    );
    // 同命令同 payload 重放 → 幂等返回首次 durable 结果，零写入。
    let replay = fresh
        .compare_and_set(
            "project_1",
            "issue_2",
            None,
            reenable("enable-command-2", "human-choice-9"),
        )
        .expect("same-command replay is idempotent");
    assert_eq!(replay, created);
    // 同命令异 payload → Conflict。
    assert_conflict(
        fresh
            .compare_and_set(
                "project_1",
                "issue_2",
                None,
                reenable("enable-command-2", "human-choice-10"),
            )
            .unwrap_err(),
        created.policy_revision,
    );
    // 缺 command_id 的既有路径（另一 issue）→ 成功且账本为空。
    let legacy = fresh
        .compare_and_set(
            "project_1",
            "issue_3",
            None,
            enable("human-choice-11", logical_repo(1)),
        )
        .expect("enable without command_id keeps the legacy behavior");
    assert!(legacy.command_ledger.is_empty());
}

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

/// 悬空源重授权（store 级）：terminated 记录的 source 引用实体已被级联删除
/// 时，`compare_and_set_reenable` 放行 **仅 source** 的漂移（人工按当前已确认
/// source 显式重新授权；binding 换代留痕）；options/target 漂移仍 409；
/// 未带悬空标记的普通 `compare_and_set` 维持逐字段相等 409。
#[test]
fn issue_automation_store_reenable_dangling_source_allows_only_source_drift() {
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
                command_id: "terminate-dangling".to_string(),
                expected_enrollment_id: created.enrollment_id.clone(),
                expected_policy_revision: created.policy_revision,
                reason: "cascade entity deletion".to_string(),
            },
        )
        .unwrap();
    let terminated = store.get("project_1", "issue_1").unwrap().unwrap();
    let binding_version = terminated
        .binding_history
        .as_ref()
        .map(|history| history.current.binding_version)
        .unwrap_or(0);

    // 新 source（当前已确认的另一对实体）。
    let resigned_source = EnrollmentSource {
        stories: vec![SourceRevisionRef {
            id: "story_spec_0002".into(),
            version: 3,
        }],
        designs: vec![SourceRevisionRef {
            id: "design_spec_0002".into(),
            version: 2,
        }],
    };
    let mut resign = reenable("reenable-dangling-1", "human-choice-2");
    if let EnrollmentWriteCommand::Enable { source: slot, .. } = &mut resign {
        *slot = resigned_source.clone();
    }

    // 未声明悬空（普通 CAS 语义）→ source 漂移维持 409。
    assert_conflict(
        store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(terminated.policy_revision),
                resign.clone(),
            )
            .unwrap_err(),
        terminated.policy_revision,
    );

    // 悬空标记放行 source 漂移：取代生效＋binding 换代留痕。
    let resigned = store
        .compare_and_set_reenable(
            "project_1",
            "issue_1",
            Some(terminated.policy_revision),
            resign,
            true,
        )
        .unwrap();
    assert!(resigned.enabled && resigned.terminated_at.is_none());
    assert_eq!(resigned.source, resigned_source);
    let history = resigned.binding_history.as_ref().unwrap();
    assert_eq!(history.current.binding_version, binding_version + 1);
    assert_eq!(history.previous.last().unwrap().source, terminated.source);

    // 再终止后：悬空标记不放宽 options/target 漂移。
    store
        .terminate_plan(
            "project_1",
            "issue_1",
            EnrollmentTerminationRequest {
                command_id: "terminate-dangling-2".to_string(),
                expected_enrollment_id: created.enrollment_id.clone(),
                expected_policy_revision: resigned.policy_revision,
                reason: "second cascade entity deletion".to_string(),
            },
        )
        .unwrap();
    let reterminated = store.get("project_1", "issue_1").unwrap().unwrap();
    let mut drifted_options = reenable("reenable-dangling-2", "human-choice-3");
    if let EnrollmentWriteCommand::Enable { options: slot, .. } = &mut drifted_options {
        slot.review_rounds = 7;
    }
    assert_conflict(
        store
            .compare_and_set_reenable(
                "project_1",
                "issue_1",
                Some(reterminated.policy_revision),
                drifted_options,
                true,
            )
            .unwrap_err(),
        reterminated.policy_revision,
    );
}
