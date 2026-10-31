// REQ-WSC-18：候选编译失败事实的 durable 存储回归（从 human_gate_recovery.rs
// 拆出，满足 large_file_guard 1200 行红线；不压缩空行）。

// ================= REQ-WSC-18：候选编译失败事实（durable + 完整） =================

fn candidate_compile_failure_fixture(detail: &str) -> crate::product::models::CandidateCompileFailure
{
    crate::product::models::CandidateCompileFailure {
        failure_id: "planning_compile_failure_abc123".to_string(),
        code: "final_compile_prepare_failed".to_string(),
        message: "依赖环：work_item_..._004 把 work_item_..._003 写成 provider".to_string(),
        detail: detail.to_string(),
        recorded_at: "2026-10-31T00:00:00Z".to_string(),
    }
}

#[test]
fn candidate_compile_status_is_durable_and_scoped_to_current_candidate() {
    let (_tmp, store) = setup();
    let session = create_session(
        &store,
        "work_item_plan_compile_status",
        WorkspaceType::WorkItemPlan,
    );

    // 从未运行过 SC compile：读 None，不报错（旧会话零回归）。
    assert!(
        store
            .load_candidate_compile_status(&session.id)
            .unwrap()
            .is_none()
    );

    // detail 完整落盘：真实诊断远超 timeline summary 的可见长度，逐字保留。
    let mut long_detail =
        "failure_id: planning_compile_failure_abc123\ncode: final_compile_prepare_failed\n"
            .to_string();
    for index in 0..64 {
        long_detail.push_str(&format!(
            "dependency_cycle 诊断第 {index} 段：work_item_..._003 ↔ work_item_..._004\n"
        ));
    }
    let failure = candidate_compile_failure_fixture(&long_detail);
    let status = crate::product::models::CandidateCompileStatus {
        source_revision_ref: Some("source-0001".to_string()),
        plan_candidate_ir_ref: Some("ir-0001".to_string()),
        failure: Some(failure.clone()),
        cycle_fingerprint: None,
        repeat_count: None,
        auto_rewrites_used: 0,
        auto_rewrite: None,
        recorded_at: "2026-10-31T00:00:00Z".to_string(),
    };
    store
        .save_candidate_compile_status(&session.id, &status)
        .unwrap();

    let loaded = store
        .load_candidate_compile_status(&session.id)
        .unwrap()
        .expect("candidate compile status must be durable");
    assert_eq!(loaded, status);
    assert_eq!(
        loaded.failure.as_ref().expect("failure").detail,
        failure.detail,
        "诊断原文必须逐字落盘，不得截断"
    );

    // 当前候选判定：两侧 ref 逐字相等才算「当前候选」。
    assert_eq!(
        loaded
            .current_failure(Some("source-0001"), Some("ir-0001"))
            .map(|current| current.failure_id.as_str()),
        Some("planning_compile_failure_abc123")
    );
    assert!(
        loaded
            .current_failure(Some("source-0002"), Some("ir-0001"))
            .is_none(),
        "候选 source 变了 → 旧失败不属于当前候选"
    );
    assert!(
        loaded
            .current_failure(Some("source-0001"), Some("ir-0009"))
            .is_none(),
        "候选 IR 变了 → 旧失败不属于当前候选"
    );
    assert!(
        loaded.current_failure(None, Some("ir-0001")).is_none(),
        "任一侧缺席即无法证明同属一个候选，不得据此拦确认"
    );

    // 编译成功覆盖（failure=None）后，同一候选不再被呈现为「编译不过」。
    let recovered = crate::product::models::CandidateCompileStatus {
        source_revision_ref: Some("source-0001".to_string()),
        plan_candidate_ir_ref: Some("ir-0001".to_string()),
        failure: None,
        cycle_fingerprint: None,
        repeat_count: None,
        auto_rewrites_used: 0,
        auto_rewrite: None,
        recorded_at: "2026-10-31T00:10:00Z".to_string(),
    };
    store
        .save_candidate_compile_status(&session.id, &recovered)
        .unwrap();
    let reloaded = store
        .load_candidate_compile_status(&session.id)
        .unwrap()
        .expect("status survives the success overwrite");
    assert!(
        reloaded
            .current_failure(Some("source-0001"), Some("ir-0001"))
            .is_none(),
        "瞬态失败恢复后不得残留「当前候选编译不过」（否则确认被永久挡死）"
    );
}
