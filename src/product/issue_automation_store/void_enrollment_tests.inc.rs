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

use crate::product::models::automation::{AutomationEnrollmentState, ConflictKind};

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
