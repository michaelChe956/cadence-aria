// ---------------------------------------------------------------------------
// REQ-WIGA-VOID-01（删除工作项组时自动化登记一并作废）：登记作废态的模型
// 投影/旧 JSON 兼容/归属披露，与「未作废登记漂移分治」的回归锁。
//
// ★回归锁纪律★：`unvoided_terminated_drift_matrix_is_unchanged` 用同一组
// 夹具把 terminated（未作废）记录的 options/source/target 漂移组合 ×
// 悬空源两路径的全部结论钉死——CAS 放行改动（已作废前置判定）落地前后，
// 本测试的每个断言必须逐字相同（改动前先跑绿、改动后再跑绿，缺一不可）。
//
// 经 tests.rs `include!` 引入（large_file_guard 1200 行红线）。
// ---------------------------------------------------------------------------

use crate::product::models::automation::{AutomationEnrollmentState, ConflictKind, VoidOutcome};

/// tasks 1.2：四态投影——终止与作废是两个独立维度，四种组合各有确定投影
/// 值；SHALL NOT 只靠 terminated_at 布尔判断（作废态不得被读成终止态）。
#[test]
fn enrollment_state_projection_distinguishes_four_states() {
    let (_tmp, store, _binding, _) = bound_enrollment_with_binding();
    let running = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(running.automation_state(), AutomationEnrollmentState::Running);

    let mut terminated = running.clone();
    terminated.terminated_at = Some("2026-11-06T00:00:00Z".to_string());
    assert_eq!(
        terminated.automation_state(),
        AutomationEnrollmentState::Terminated
    );

    let mut voided = running.clone();
    voided.voided_at = Some("2026-11-06T00:00:00Z".to_string());
    assert_eq!(voided.automation_state(), AutomationEnrollmentState::Voided);

    let mut both = terminated.clone();
    both.voided_at = Some("2026-11-06T01:00:00Z".to_string());
    both.void_reason = Some("work_item_group_deleted".to_string());
    assert_eq!(
        both.automation_state(),
        AutomationEnrollmentState::TerminatedAndVoided
    );
    // 独立维度：作废不清除终止事实，终止也不推断作废。
    assert!(both.terminated_at.is_some() && both.voided_at.is_some());
}

/// tasks 1.1：旧 durable JSON 缺 voided_at/void_reason 字段按未作废读——
/// 反序列化零报错，两个字段读 None（序列化兼容：存量记录不变）。
#[test]
fn legacy_enrollment_json_without_void_fields_reads_as_unvoided() {
    let (tmp, store, _binding, _) = bound_enrollment_with_binding();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    let mut legacy = serde_json::to_value(&saved).unwrap();
    let object = legacy
        .as_object_mut()
        .expect("enrollment serializes to a JSON object");
    object.remove("voided_at");
    object.remove("void_reason");
    write_json(&enrollment_json_path(&tmp), &legacy).unwrap();

    let reloaded: IssueAutomationEnrollment =
        read_json(&enrollment_json_path(&tmp)).unwrap();
    assert_eq!(
        reloaded.voided_at, None,
        "旧 JSON 缺 voided_at 按未作废读"
    );
    assert_eq!(reloaded.void_reason, None);
    assert_eq!(reloaded.enrollment_id, saved.enrollment_id);
}

/// tasks 1.1/5：归属投影（REST/WS 同源）披露作废事实——读侧据此区分
/// 「已终止」与「已作废」（CodingWorkspacePage 等终止横幅消费点）。
#[test]
fn ownership_projection_carries_voided_fact() {
    let (tmp, store, _binding, _) = bound_enrollment_with_binding();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    // 直写作废态（作废命令的行为测试另行覆盖；此处只锁投影披露）。
    let mut voided = saved.clone();
    voided.voided_at = Some("2026-11-06T00:00:00Z".to_string());
    voided.void_reason = Some("work_item_group_deleted".to_string());
    write_json(&enrollment_json_path(&tmp), &voided).unwrap();

    let ownership = store
        .ownership_for_ids("project_1", "issue_1", "session_0001")
        .unwrap();
    assert_eq!(ownership.voided_at.as_deref(), Some("2026-11-06T00:00:00Z"));
    assert_eq!(
        ownership.terminated_at, None,
        "作废不推导终止——两维度独立披露"
    );
}

/// 漂移组合命令：`drift` 含 'o'＝options 漂移（换 provider）、's'＝source
/// 漂移（story 版本 +1）、't'＝target 漂移（换逻辑仓）。与 `reenable` 同源
/// 的稳定 command_id/新 selection_key。
fn drifted_reenable(command_id: &str, drift: &str) -> EnrollmentWriteCommand {
    let mut command = reenable(command_id, "human-choice-drift");
    if let EnrollmentWriteCommand::Enable {
        source, options, target, ..
    } = &mut command
    {
        if drift.contains('o') {
            options.author_provider = ProviderName::Codex;
            options.reviewer_provider = ProviderName::Codex;
        }
        if drift.contains('s') {
            source.stories[0].version += 1;
        }
        if drift.contains('t') {
            *target = target_logical_at(logical_repo(9));
        }
    }
    command
}

/// terminated（未作废）夹具：enable＋bind＋同源终止 CAS。
fn terminated_unvoided_fixture()
-> (tempfile::TempDir, IssueAutomationStore, IssueAutomationEnrollment) {
    let (tmp, store, _binding, _) = bound_enrollment_with_binding();
    let current = store.get("project_1", "issue_1").unwrap().unwrap();
    store
        .terminate_plan(
            "project_1",
            "issue_1",
            EnrollmentTerminationRequest {
                command_id: "void-tests-terminate".to_string(),
                expected_enrollment_id: current.enrollment_id.clone(),
                expected_policy_revision: current.policy_revision,
                reason: "fixture termination".to_string(),
            },
        )
        .unwrap();
    let terminated = store.get("project_1", "issue_1").unwrap().unwrap();
    (tmp, store, terminated)
}

fn assert_conflict_kind(error: EnrollmentError, expected_kind: ConflictKind, revision: u64) {
    match error {
        EnrollmentError::Conflict {
            current_revision: Some(rev),
            kind,
        } => {
            assert_eq!(rev, revision);
            assert_eq!(kind, expected_kind);
        }
        other => panic!("expected Conflict({expected_kind:?}) at revision {revision}, got {other:?}"),
    }
}

/// ★回归锁★：未作废 terminated 记录的漂移分治矩阵（design §3 硬边界——
/// 「未作废的登记分治逻辑一个字都不许改」）。每个组合独立现场，结论与
/// REQ-WIGA-SWAP-01/REQ-DRG-18 现行语义逐条一致；CAS 作废放行改动落地
/// 前后本测试必须同绿（夹具与断言一字不改）。
#[test]
fn unvoided_terminated_drift_matrix_is_unchanged() {
    struct Case {
        drift: &'static str,
        dangling: bool,
        /// None＝接受（换代落盘）；Some(kind)＝以该判别位拒绝（零副作用）。
        expect: Option<ConflictKind>,
    }
    // 正常重开（dangling=false）：options 漂移按换代放行；source/target
    // 任一漂移即 SourceTargetDrift。
    // 悬空源重授权（dangling=true）：options 漂移 DefensiveStateMismatch
    //（先判）；无 options 漂移时 target 漂移 SourceTargetDrift；仅 source
    // 漂移放行。
    let matrix = [
        Case { drift: "", dangling: false, expect: None },
        Case { drift: "o", dangling: false, expect: None },
        Case { drift: "s", dangling: false, expect: Some(ConflictKind::SourceTargetDrift) },
        Case { drift: "t", dangling: false, expect: Some(ConflictKind::SourceTargetDrift) },
        Case { drift: "os", dangling: false, expect: Some(ConflictKind::SourceTargetDrift) },
        Case { drift: "ot", dangling: false, expect: Some(ConflictKind::SourceTargetDrift) },
        Case { drift: "st", dangling: false, expect: Some(ConflictKind::SourceTargetDrift) },
        Case { drift: "ost", dangling: false, expect: Some(ConflictKind::SourceTargetDrift) },
        Case { drift: "", dangling: true, expect: None },
        Case { drift: "o", dangling: true, expect: Some(ConflictKind::DefensiveStateMismatch) },
        Case { drift: "s", dangling: true, expect: None },
        Case { drift: "t", dangling: true, expect: Some(ConflictKind::SourceTargetDrift) },
        Case { drift: "os", dangling: true, expect: Some(ConflictKind::DefensiveStateMismatch) },
        Case { drift: "ot", dangling: true, expect: Some(ConflictKind::DefensiveStateMismatch) },
        Case { drift: "st", dangling: true, expect: Some(ConflictKind::SourceTargetDrift) },
        Case { drift: "ost", dangling: true, expect: Some(ConflictKind::DefensiveStateMismatch) },
    ];
    for case in matrix {
        let (_tmp, store, terminated) = terminated_unvoided_fixture();
        let label = format!("drift={:?} dangling={}", case.drift, case.dangling);
        let command = drifted_reenable(
            &format!("matrix-{}-{}", case.drift, case.dangling),
            case.drift,
        );
        let outcome = if case.dangling {
            store.compare_and_set_reenable(
                "project_1",
                "issue_1",
                Some(terminated.policy_revision),
                command,
                true,
            )
        } else {
            store.compare_and_set(
                "project_1",
                "issue_1",
                Some(terminated.policy_revision),
                command,
            )
        };
        match (case.expect, outcome) {
            (None, Ok(replaced)) => {
                assert!(replaced.enabled, "{label}: superseded record is enabled");
                assert!(replaced.terminated_at.is_none(), "{label}");
                assert_eq!(
                    replaced.policy_revision,
                    terminated.policy_revision + 1,
                    "{label}: 取代 revision+1"
                );
                assert_ne!(
                    replaced.prepare_intent_id, terminated.prepare_intent_id,
                    "{label}: 换代轮换 prepare_intent_id"
                );
                let history = replaced
                    .binding_history
                    .as_ref()
                    .expect("binding history survives the supersede");
                assert_eq!(
                    history.current.binding_version,
                    terminated
                        .binding_history
                        .as_ref()
                        .unwrap()
                        .current
                        .binding_version
                        + 1,
                    "{label}: binding_version+1"
                );
                assert!(
                    history
                        .previous
                        .iter()
                        .any(|binding| binding.plan_id == "plan_0001"),
                    "{label}: 旧代进 previous"
                );
            }
            (Some(kind), Err(error)) => {
                assert_conflict_kind(error, kind, terminated.policy_revision);
                let after = store.get("project_1", "issue_1").unwrap().unwrap();
                assert_eq!(
                    after.policy_revision, terminated.policy_revision,
                    "{label}: 拒绝零副作用"
                );
            }
            (expected, actual) => panic!(
                "{label}: unexpected outcome — expected {expected:?}, got {actual:?}",
                expected = expected.map(|kind| format!("{kind:?}")),
                actual = actual.map(|enrollment| enrollment.automation_state())
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// REQ-WIGA-VOID-01（tasks 2.1）：作废命令四分支——不存在⇒NotFound（幂等）、
// 已作废⇒AlreadyVoided（不重复追加账本）、绑定 plan 不匹配⇒NotBoundToPlan
// （不得作废别人的）、成功⇒写字段+revision+1+绑定前代进 previous+账本追加
// 一条（旧条目逐字不变）且不物理删除登记文件。
// ---------------------------------------------------------------------------

/// 分支①：该 issue 无登记 ⇒ NotFound（幂等成功），且不反向创建登记文件。
#[test]
fn void_enrollment_for_missing_returns_not_found_without_creating_file() {
    let tmp = tempfile::tempdir().unwrap();
    let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
    let outcome = store
        .void_enrollment_for_deleted_plan(
            "project_1",
            "issue_1",
            "plan_0001",
            "work_item_group_deleted",
        )
        .expect("无登记是幂等成功，不报错");
    assert_eq!(outcome, VoidOutcome::NotFound);
    assert!(
        !enrollment_json_path(&tmp).exists(),
        "不得反向创建登记文件"
    );
}

/// 分支②：已作废 ⇒ AlreadyVoided 幂等返回——不重复追加账本、revision 不再
/// +1（整条记录逐字段不变）。
#[test]
fn void_enrollment_already_voided_is_idempotent_without_second_ledger_entry() {
    let (_tmp, store, _terminated) = terminated_unvoided_fixture();
    match store
        .void_enrollment_for_deleted_plan(
            "project_1",
            "issue_1",
            "plan_0001",
            "work_item_group_deleted",
        )
        .unwrap()
    {
        VoidOutcome::Voided(_) => {}
        other => panic!("首次作废应成功，got {other:?}"),
    }
    let after_first = store.get("project_1", "issue_1").unwrap().unwrap();

    let second = store
        .void_enrollment_for_deleted_plan(
            "project_1",
            "issue_1",
            "plan_0001",
            "work_item_group_deleted",
        )
        .unwrap();
    assert_eq!(second, VoidOutcome::AlreadyVoided);
    let after_second = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(
        after_second, after_first,
        "已作废的重复作废不得再追加账本或递增 revision"
    );
}

/// 分支③：登记绑定别的 plan（或从未绑定 plan）⇒ NotBoundToPlan 零副作用。
#[test]
fn void_enrollment_not_bound_to_plan_refuses_to_void_others() {
    // (a) 绑定在别的 plan 上。
    let (tmp, store, _binding, _) = bound_enrollment_with_binding();
    let before = store.get("project_1", "issue_1").unwrap().unwrap();
    let outcome = store
        .void_enrollment_for_deleted_plan(
            "project_1",
            "issue_1",
            "plan_9999",
            "work_item_group_deleted",
        )
        .unwrap();
    assert_eq!(outcome, VoidOutcome::NotBoundToPlan);
    assert_eq!(
        store.get("project_1", "issue_1").unwrap().unwrap(),
        before,
        "NotBoundToPlan 零副作用"
    );
    drop(tmp);

    // (b) 从未绑定 plan（plan_id=None）：不能证明登记属于被删 plan ⇒ 不作废。
    let tmp = tempfile::tempdir().unwrap();
    let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
    store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-unbound", logical_repo(1)),
        )
        .unwrap();
    let outcome = store
        .void_enrollment_for_deleted_plan(
            "project_1",
            "issue_1",
            "plan_0001",
            "work_item_group_deleted",
        )
        .unwrap();
    assert_eq!(
        outcome,
        VoidOutcome::NotBoundToPlan,
        "未绑定 plan 的登记不得被作废"
    );
}

/// 分支④：成功 ⇒ 写 voided_at/void_reason、revision+1、绑定前代进
/// previous（current 换空占位新代）、账本追加一条（旧条目逐字不变）；
/// terminated_at/plan_id 原样保留（独立维度、不抹绑定事实）；
/// SHALL NOT 物理删除 `automation-enrollment.json`。
#[test]
fn void_enrollment_voids_writes_fields_binds_predecessor_and_appends_ledger() {
    let (tmp, store, binding, _) = bound_enrollment_with_binding();
    let current = store.get("project_1", "issue_1").unwrap().unwrap();
    store
        .terminate_plan(
            "project_1",
            "issue_1",
            EnrollmentTerminationRequest {
                command_id: "void-tests-terminate".to_string(),
                expected_enrollment_id: current.enrollment_id.clone(),
                expected_policy_revision: current.policy_revision,
                reason: "fixture termination".to_string(),
            },
        )
        .unwrap();
    let terminated = store.get("project_1", "issue_1").unwrap().unwrap();

    let voided = match store
        .void_enrollment_for_deleted_plan(
            "project_1",
            "issue_1",
            "plan_0001",
            "work_item_group_deleted",
        )
        .unwrap()
    {
        VoidOutcome::Voided(voided) => voided,
        other => panic!("expected Voided, got {other:?}"),
    };
    assert!(voided.voided_at.as_deref().is_some_and(|at| !at.is_empty()));
    assert_eq!(
        voided.void_reason.as_deref(),
        Some("work_item_group_deleted")
    );
    assert_eq!(voided.policy_revision, terminated.policy_revision + 1);
    // 独立维度：作废不清终止；绑定事实不被抹掉。
    assert_eq!(voided.terminated_at, terminated.terminated_at);
    assert_eq!(voided.plan_id, terminated.plan_id);
    // 绑定前代进 previous；current 换空占位新代（与换代同构）。
    let history = voided
        .binding_history
        .as_ref()
        .expect("binding history survives the void");
    assert_eq!(
        history.previous.last(),
        Some(&binding),
        "被作废代进 previous（只读可追溯）"
    );
    assert_eq!(history.current.binding_version, binding.binding_version + 1);
    assert_eq!(history.current.plan_id, "");
    assert_eq!(history.current.session_id, "");
    // 账本：旧条目逐字不变 + 一条作废条目（Accepted、binding_version 新代）。
    assert_eq!(
        voided.command_ledger.len(),
        terminated.command_ledger.len() + 1
    );
    assert_eq!(
        &voided.command_ledger[..terminated.command_ledger.len()],
        &terminated.command_ledger[..],
        "旧账本条目逐字不变（只追加不改写）"
    );
    let entry = voided.command_ledger.last().unwrap();
    assert_eq!(entry.command_id, "void-enrollment-plan_0001");
    assert_eq!(entry.state, OperationState::Accepted);
    assert_eq!(entry.binding_version, binding.binding_version + 1);
    // SHALL NOT 物理删除登记文件。
    assert!(enrollment_json_path(&tmp).exists());
}

// ---------------------------------------------------------------------------
// REQ-WIGA-VOID-01（tasks 3.1/3.2）：已作废 ⇒ 重新开启放行全部漂移（换代
// 语义）；作废态下重新开启不复活旧 plan；作废+未终止（Disable 后随组删除）
// 的 Enable 同样走换代取代。
// ---------------------------------------------------------------------------

/// 作废态直写夹具：terminated（未作废）之后叠加作废字段（磁盘实证形态）。
fn voided_enrollment_fixture()
-> (tempfile::TempDir, IssueAutomationStore, IssueAutomationEnrollment) {
    let (tmp, store, terminated) = terminated_unvoided_fixture();
    let mut voided = terminated.clone();
    voided.voided_at = Some("2026-11-06T00:00:00Z".to_string());
    voided.void_reason = Some("work_item_group_deleted".to_string());
    write_json(&enrollment_json_path(&tmp), &voided).unwrap();
    (tmp, store, voided)
}

/// tasks 3.1/3.2：已作废 ⇒ 正常重开路径（original_source_dangling=false）
/// 放行 options/source/target 全部漂移，一次 Enable 完成换代；新代未作废
/// （voided_at/void_reason 随换代清除）；旧 plan 不复活（plan/session 解绑、
/// prepare_intent_id 轮换、旧代进 previous）。
#[test]
fn voided_enrollment_reenable_releases_all_drift_on_normal_path() {
    let (_tmp, store, voided) = voided_enrollment_fixture();
    let old_binding = voided.binding_history.clone().unwrap().current;
    let old_prepare_intent = voided.prepare_intent_id.clone();
    let drifted = drifted_reenable("voided-drift-normal", "ost");
    let replaced = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(voided.policy_revision),
            drifted,
        )
        .expect("已作废的登记放行 options/source/target 全部漂移（REQ-WIGA-VOID-01）");
    assert!(replaced.enabled);
    assert!(replaced.terminated_at.is_none());
    assert!(
        replaced.voided_at.is_none(),
        "作废属于被取代的旧代——新代未作废"
    );
    assert!(replaced.void_reason.is_none());
    assert_eq!(replaced.policy_revision, voided.policy_revision + 1);
    assert_ne!(
        replaced.prepare_intent_id, old_prepare_intent,
        "换代轮换 prepare_intent_id"
    );
    assert!(
        replaced.plan_id.is_none() && replaced.session_id.is_none(),
        "旧 plan 不复活（supersede 解绑，新链由 ensure 重新派生）"
    );
    assert_eq!(replaced.options.author_provider, ProviderName::Codex);
    let history = replaced
        .binding_history
        .as_ref()
        .expect("binding history survives the supersede");
    assert_eq!(
        history.current.binding_version,
        old_binding.binding_version + 1
    );
    assert!(
        history
            .previous
            .iter()
            .any(|bound| bound.plan_id == "plan_0001"),
        "旧代进 previous"
    );
}

/// tasks 3.1：悬空源重授权路径（original_source_dangling=true）同样放行全部
/// 漂移——作废记录视为无约束力的历史记录，分路径差异不复存在。
#[test]
fn voided_enrollment_reenable_releases_all_drift_on_dangling_path() {
    let (_tmp, store, voided) = voided_enrollment_fixture();
    let drifted = drifted_reenable("voided-drift-dangling", "ost");
    let replaced = store
        .compare_and_set_reenable(
            "project_1",
            "issue_1",
            Some(voided.policy_revision),
            drifted,
            true,
        )
        .expect("悬空源重授权路径同样放行全部漂移（作废记录不再约束）");
    assert!(replaced.enabled);
    assert!(replaced.terminated_at.is_none());
    assert!(replaced.voided_at.is_none());
}

/// 作废且未终止（先 Disable 再随工作项组删除）的记录：Enable 走换代取代
///（binding_version+1、prepare_intent_id 轮换）——target 漂移不再被普通
/// 重开路径拒绝。
#[test]
fn voided_disabled_enrollment_enable_walks_generation_supersede() {
    let (tmp, store, _binding, revision) = bound_enrollment_with_binding();
    store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(revision),
            EnrollmentWriteCommand::Disable,
        )
        .unwrap();
    let mut disabled = store.get("project_1", "issue_1").unwrap().unwrap();
    disabled.voided_at = Some("2026-11-06T00:00:00Z".to_string());
    disabled.void_reason = Some("work_item_group_deleted".to_string());
    write_json(&enrollment_json_path(&tmp), &disabled).unwrap();
    let old_prepare_intent = disabled.prepare_intent_id.clone();
    let old_binding = disabled.binding_history.clone().unwrap().current;

    let drifted = drifted_reenable("voided-disabled-enable", "t");
    let replaced = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(disabled.policy_revision),
            drifted,
        )
        .expect("作废且未终止的记录：Enable 走换代取代（target 漂移放行）");
    assert!(replaced.enabled);
    assert!(replaced.terminated_at.is_none());
    assert!(replaced.voided_at.is_none());
    assert_eq!(replaced.policy_revision, disabled.policy_revision + 1);
    assert_ne!(replaced.prepare_intent_id, old_prepare_intent);
    assert_eq!(
        replaced
            .binding_history
            .as_ref()
            .unwrap()
            .current
            .binding_version,
        old_binding.binding_version + 1
    );
}

/// 作废→重新开启：新代是未作废登记（后续语义回到现状）——再次绑定、终止、
/// 删除作废可再次发生（同一条命令、账本再 +1）。钉住「supersede 清除作废
/// 字段」不把新代永久钉在作废态。
#[test]
fn superseded_generation_after_void_starts_unvoided_and_can_be_voided_again() {
    let (_tmp, store, voided) = voided_enrollment_fixture();
    let same_payload = reenable("voided-reopen", "human-choice-2");
    let reopened = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(voided.policy_revision),
            same_payload,
        )
        .expect("作废记录的等 payload 重新开启走同一条取代路径");
    assert!(
        reopened.voided_at.is_none() && reopened.void_reason.is_none(),
        "新代未作废"
    );

    // 新代绑定新 plan → 终止 → 删除作废：可再次产生作废态（账本再 +1）。
    store
        .bind_plan(
            "project_1",
            "issue_1",
            reopened.policy_revision,
            "plan_0002",
            "session_0002",
        )
        .unwrap();
    let bound = store.get("project_1", "issue_1").unwrap().unwrap();
    store
        .terminate_plan(
            "project_1",
            "issue_1",
            EnrollmentTerminationRequest {
                command_id: "void-tests-terminate-2".to_string(),
                expected_enrollment_id: bound.enrollment_id.clone(),
                expected_policy_revision: bound.policy_revision,
                reason: "fixture termination".to_string(),
            },
        )
        .unwrap();
    match store
        .void_enrollment_for_deleted_plan(
            "project_1",
            "issue_1",
            "plan_0002",
            "work_item_group_deleted",
        )
        .unwrap()
    {
        VoidOutcome::Voided(revoided) => {
            assert!(revoided.voided_at.is_some());
            assert_eq!(revoided.void_reason.as_deref(), Some("work_item_group_deleted"));
            assert_eq!(revoided.policy_revision, bound.policy_revision + 2);
        }
        other => panic!("新代可再次作废，got {other:?}"),
    }
}
