//! r2 简化批次新增断言的负测(A1/A2/B3 三面)。
//!
//! 大文件守卫:live_b45_t23.rs 已按祖父条款登记上限,新增负测整体落本
//! 文件,主体只留 `mod` 接线。每条负测都钉住「不得靠状态名/恒零计数逃避
//! 判等」的拒绝形态,正例与反例成对出现。

use super::super::harness::{EvidenceCell, r2_evidence_semantics};
use super::{
    ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, FRESH, RESUME, t23_assertion_tests,
    verify_failure_cell, verify_resume_round, verify_spawn_instrumentation,
};
use serde_json::json;

/// A1 空转格(A1 判据自证齐备的 not_applicable resume 格)。
fn idle_resume_cell() -> EvidenceCell {
    let mut cell =
        t23_assertion_tests::base_cell("coding", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, RESUME);
    cell.capability_state = "not_applicable".to_string();
    cell.execution_origin = r2_evidence_semantics::NO_PENDING_WORK_ORIGIN.to_string();
    cell.denied_reason = Some("无待恢复工作:attempt 已停 final_confirm 人工终门".to_string());
    cell.requested_resume_id = Some("native-7".to_string());
    cell.native_resume_confirmed_id = Some("native-7".to_string());
    cell.provider_spawn_count = 0;
    cell.provider_pid = None;
    cell.pid_unavailable_reason = Some("零新增 spawn(终门空转)".to_string());
    cell.provider_events = vec![json!({
        "event": {
            "type": "zero_spawn_observation",
            "provider_start_audits_before": 2,
            "provider_start_audits_after": 2,
            "audit_window": "session",
        }
    })];
    cell
}

/// 单一格的最小矩阵:空转格是合法格,断言集必须放行。
#[test]
fn a1_idle_resume_cell_passes_spawn_instrumentation() {
    let cell = idle_resume_cell();
    assert_eq!(
        verify_spawn_instrumentation(std::slice::from_ref(&cell)),
        Ok(())
    );
}

/// A1 负测:状态名不构成豁免——自证缺一维即红。
#[test]
fn a1_idle_label_without_evidence_is_rejected() {
    let mut wrong_origin = idle_resume_cell();
    wrong_origin.execution_origin = "full_chain".to_string();
    assert!(
        verify_spawn_instrumentation(std::slice::from_ref(&wrong_origin)).is_err(),
        "缺 no_pending_work 标注必须拒绝"
    );

    let mut spawned = idle_resume_cell();
    spawned.provider_spawn_count = 1;
    assert!(
        verify_spawn_instrumentation(std::slice::from_ref(&spawned)).is_err(),
        "真起了进程不得标 not_applicable"
    );

    let mut drifted = idle_resume_cell();
    drifted.provider_events = vec![json!({
        "event": {
            "type": "zero_spawn_observation",
            "provider_start_audits_before": 2,
            "provider_start_audits_after": 3,
            "audit_window": "session",
        }
    })];
    assert!(
        verify_spawn_instrumentation(std::slice::from_ref(&drifted)).is_err(),
        "审计前后不等必须拒绝(零新增不可证)"
    );

    let mut no_reason = idle_resume_cell();
    no_reason.denied_reason = Some(String::new());
    assert!(
        verify_spawn_instrumentation(std::slice::from_ref(&no_reason)).is_err(),
        "缺现场原因必须拒绝"
    );
}

/// A2/M7 负测:confirmed 的 resume 格仍然必须 ≥1 真实新增 spawn。
#[test]
fn a2_confirmed_resume_without_new_spawn_is_rejected() {
    let mut cell = idle_resume_cell();
    cell.capability_state = "confirmed".to_string();
    cell.execution_origin = "full_chain".to_string();
    let error = verify_spawn_instrumentation(std::slice::from_ref(&cell))
        .expect_err("confirmed+0 spawn 必须红(M7 进程证据不得放弃)");
    assert!(error.contains("必须 ≥1 真实 spawn"), "{error}");
}

/// A1:resume 轮的 coding/review 允许「自证齐备的空转格」代替 executed-resume。
#[test]
fn a1_resume_round_accepts_honest_idle_cell() {
    let manifest = t23_assertion_tests::test_manifest("/tmp/unused-source-root");
    let mut cells = t23_assertion_tests::resume_round_cells(&manifest);
    for cell in &mut cells {
        if cell.stage == "coding" && cell.fresh_or_resume == RESUME {
            let idle = idle_resume_cell();
            cell.capability_state = idle.capability_state;
            cell.execution_origin = idle.execution_origin;
            cell.denied_reason = idle.denied_reason;
            cell.requested_resume_id = idle.requested_resume_id;
            cell.native_resume_confirmed_id = idle.native_resume_confirmed_id;
            cell.provider_spawn_count = 0;
            cell.provider_events = idle.provider_events;
        }
    }
    assert_eq!(verify_resume_round(&cells), Ok(()));

    // 空转格自证不足(缺 zero_spawn_observation)→ 红。
    let mut unproven = cells.clone();
    for cell in &mut unproven {
        if cell.stage == "coding" && cell.fresh_or_resume == RESUME {
            cell.provider_events.clear();
        }
    }
    assert!(
        verify_resume_round(&unproven).is_err(),
        "空转格缺自证必须拒绝"
    );

    // carried 冒充 executed-resume 仍红(A1 不放宽该红线)。
    let mut masquerade = cells;
    for cell in &mut masquerade {
        if cell.stage == "review" && cell.fresh_or_resume == RESUME {
            cell.execution_origin = "carried:snap-1".to_string();
        }
    }
    let error = verify_resume_round(&masquerade).expect_err("carried 冒充必须拒绝");
    assert!(error.contains("冒充"), "{error}");
}

/// 失败格基座:真实拒绝探针 + 审计前后计数(可按用例改写)。
fn failure_cell(scenario: &str) -> EvidenceCell {
    let mut cell =
        t23_assertion_tests::base_cell("coding", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, FRESH);
    cell.capability_state = "denied".to_string();
    cell.denied_reason = Some("spawn 前复验拒绝".to_string());
    cell.provider_spawn_count = 0;
    cell.provider_pid = None;
    cell.pid_unavailable_reason = Some("零 spawn 被拒:无 provider 子进程".to_string());
    cell.native_session_id = String::new();
    cell.provider_events = vec![
        json!({
            "event": {
                "type": "boundary_matrix_evidence",
                "failure_scenario": scenario,
            }
        }),
        json!({
            "event": {
                "type": "launch_rejected_probe",
                "scenario": scenario,
                "refusal_gate": "spawn 前复验(resume_fingerprint)",
                "refusal_text": "provider_gateway_policy_drift: resume_fingerprint",
                "refused": true,
            }
        }),
        json!({
            "event": {
                "type": "zero_spawn_observation",
                "provider_start_audits_before": 0,
                "provider_start_audits_after": 0,
                "audit_window": "session",
            }
        }),
    ];
    cell
}

/// B3 正例:拒绝格计数与审计 delta 同口径(=0)。
#[test]
fn b3_failure_cell_consumes_audit_delta() {
    let cell = failure_cell("fingerprint_drift");
    assert_eq!(verify_failure_cell(&cell), Ok(()));
}

/// B3 负测:格内计数恒 0 而自身观测 0→1(真 spawn)——必须红。
#[test]
fn b3_failure_cell_count_must_match_audit_delta() {
    let mut drifted = failure_cell("missing_body");
    drifted.provider_events = vec![
        drifted.provider_events[0].clone(),
        drifted.provider_events[1].clone(),
        json!({
            "event": {
                "type": "zero_spawn_observation",
                "provider_start_audits_before": 0,
                "provider_start_audits_after": 1,
                "audit_window": "not_applicable",
            }
        }),
    ];
    let error = verify_failure_cell(&drifted)
        .expect_err("provider_spawn_count 恒 0 与审计 delta 1 自相矛盾必须红");
    assert!(error.contains("不自洽"), "{error}");

    let mut spawned = drifted;
    spawned.provider_spawn_count = 1;
    let error =
        verify_failure_cell(&spawned).expect_err("拒绝格真起进程即零 spawn 负向证明失效,必须红");
    assert!(error.contains("零 spawn 负向证明不成立"), "{error}");
}

/// R1:评审 resume 格「无可续评审会话」形态(reviewer 恒 fresh spawn)。
fn review_idle_resume_cell() -> EvidenceCell {
    let mut cell = idle_resume_cell();
    cell.stage = "review".to_string();
    cell.role = "reviewer".to_string();
    cell.action = "review_read_only".to_string();
    cell.denied_reason = Some(format!(
        "无可续评审会话:{}——评审阶段不存在可续的原生会话",
        r2_evidence_semantics::REVIEWER_FRESH_ONLY_REASON_MARKER
    ));
    cell
}

/// R1 正例:自证齐备的 review 空转格替代 executed-resume,两项断言集均放行。
#[test]
fn r1_resume_round_accepts_review_idle_cell() {
    let manifest = t23_assertion_tests::test_manifest("/tmp/unused-source-root");
    let mut cells = t23_assertion_tests::resume_round_cells(&manifest);
    for cell in &mut cells {
        if cell.stage == "review" && cell.fresh_or_resume == RESUME {
            let idle = review_idle_resume_cell();
            cell.capability_state = idle.capability_state;
            cell.execution_origin = idle.execution_origin;
            cell.denied_reason = idle.denied_reason;
            cell.requested_resume_id = idle.requested_resume_id;
            cell.native_resume_confirmed_id = idle.native_resume_confirmed_id;
            cell.provider_spawn_count = 0;
            cell.provider_events = idle.provider_events;
        }
    }
    assert_eq!(verify_resume_round(&cells), Ok(()));
    assert_eq!(verify_spawn_instrumentation(&cells), Ok(()));
}

/// R1 负测:review 空转格缺任一自证要素即红(状态名不构成豁免)。
#[test]
fn r1_review_idle_cell_without_evidence_is_rejected() {
    let manifest = t23_assertion_tests::test_manifest("/tmp/unused-source-root");

    // (a) 缺 zero_spawn_observation:零新增不可证。
    let mut unproven = t23_assertion_tests::resume_round_cells(&manifest);
    for cell in &mut unproven {
        if cell.stage == "review" && cell.fresh_or_resume == RESUME {
            let idle = review_idle_resume_cell();
            cell.capability_state = idle.capability_state;
            cell.execution_origin = idle.execution_origin;
            cell.denied_reason = idle.denied_reason;
            cell.requested_resume_id = idle.requested_resume_id;
            cell.native_resume_confirmed_id = idle.native_resume_confirmed_id;
            cell.provider_spawn_count = 0;
            cell.provider_events.clear();
        }
    }
    assert!(
        verify_resume_round(&unproven).is_err(),
        "review 空转格缺 zero_spawn_observation 必须拒绝"
    );
    assert!(
        verify_spawn_instrumentation(&unproven).is_err(),
        "spawn 仪器侧同样不得放行"
    );

    // (b) 真起了进程却标 not_applicable。
    let mut spawned = t23_assertion_tests::resume_round_cells(&manifest);
    for cell in &mut spawned {
        if cell.stage == "review" && cell.fresh_or_resume == RESUME {
            *cell = review_idle_resume_cell();
            cell.provider_spawn_count = 1;
        }
    }
    assert!(
        verify_spawn_instrumentation(&spawned).is_err(),
        "review 格真起进程不得标 not_applicable"
    );

    // (c) 原因复用 coding 终门口径(阶段语义冒充)。
    let mut wrong_reason = t23_assertion_tests::resume_round_cells(&manifest);
    for cell in &mut wrong_reason {
        if cell.stage == "review" && cell.fresh_or_resume == RESUME {
            *cell = review_idle_resume_cell();
            cell.denied_reason =
                Some("无待恢复工作:attempt 已停 final_confirm 人工终门".to_string());
        }
    }
    assert!(
        verify_resume_round(&wrong_reason).is_err(),
        "review 空转格复用 coding 原因必须拒绝"
    );
}

/// R1/M7 红线:空转格的 native ID 一致判据不放宽(请求≠实测仍红)。
#[test]
fn r1_review_idle_cell_keeps_native_id_equality() {
    let manifest = t23_assertion_tests::test_manifest("/tmp/unused-source-root");
    let mut cells = t23_assertion_tests::resume_round_cells(&manifest);
    for cell in &mut cells {
        if cell.stage == "review" && cell.fresh_or_resume == RESUME {
            *cell = review_idle_resume_cell();
            cell.native_resume_confirmed_id = Some("native-other".to_string());
        }
    }
    let error = verify_resume_round(&cells).expect_err("空转格 native ID 不一致必须拒绝");
    assert!(error.contains("原生会话一致证据"), "{error}");
    assert!(error.contains("native-other"), "{error}");
}

/// R1:review 空转格不得借 carried 来源冒充(承继格不重新计票)。
#[test]
fn r1_review_idle_cell_rejects_carried_masquerade() {
    let manifest = t23_assertion_tests::test_manifest("/tmp/unused-source-root");
    let mut cells = t23_assertion_tests::resume_round_cells(&manifest);
    for cell in &mut cells {
        if cell.stage == "review" && cell.fresh_or_resume == RESUME {
            *cell = review_idle_resume_cell();
            cell.execution_origin = "carried:snap-1".to_string();
        }
    }
    let error = verify_resume_round(&cells).expect_err("carried 冒充必须拒绝");
    assert!(error.contains("carried"), "{error}");
}
