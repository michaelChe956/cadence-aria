// REQ-WSC-25：候选生成/编译尝试的 append-only 流水（durable + 幂等 + 度量读数）。
// 从 human_gate_recovery.rs 拆出，满足 large_file_guard 1200 行红线；不压缩空行。

use crate::product::lifecycle_store::candidate_attempts::{
    CandidateAttemptFailure, CandidateAttemptOutcome, CandidateAttemptRecord,
    CandidateAttemptStage, CandidateIssueAttemptSeries, EquivalenceCanonicalization,
    compute_candidate_attempt_metrics,
};
use crate::product::models::CandidateAutoRewrite;
use crate::product::planning_failure::PlanningFailureKind;

fn attempt_record(
    attempt_key: &str,
    outcome: CandidateAttemptOutcome,
    recorded_at: &str,
) -> CandidateAttemptRecord {
    CandidateAttemptRecord {
        attempt_seq: 0,
        attempt_key: attempt_key.to_string(),
        session_id: String::new(),
        issue_id: String::new(),
        recorded_at: recorded_at.to_string(),
        stage: CandidateAttemptStage::Final,
        outcome,
        source_revision_ref: Some("source-0001".to_string()),
        plan_candidate_ir_ref: Some("ir-0001".to_string()),
        failure: match outcome {
            CandidateAttemptOutcome::Passed => None,
            CandidateAttemptOutcome::Failed => Some(CandidateAttemptFailure {
                failure_id: format!("planning_compile_failure_{attempt_key}"),
                code: "final_compile_prepare_failed".to_string(),
                typed_kinds: vec![PlanningFailureKind::Operational {
                    stable_code: "final_compile_prepare_failed".to_string(),
                    cause: "dependency_cycle: work_item_003 -> work_item_004".to_string(),
                }],
                cycle_fingerprint: Some("661e48b3".to_string()),
            }),
        },
        auto_rewrite: None,
        auto_repairs_applied: Vec::new(),
        equivalence_canonicalizations: Vec::new(),
    }
}

#[test]
fn candidate_attempt_stream_appends_records_and_is_idempotent() {
    let (_tmp, store) = setup();
    let session = create_session(
        &store,
        "work_item_plan_candidate_attempts",
        WorkspaceType::WorkItemPlan,
    );

    let first = attempt_record(
        "fail/one",
        CandidateAttemptOutcome::Failed,
        "2026-10-31T00:00:00Z",
    );
    assert!(
        store.append_candidate_attempt(&session.id, &first).unwrap(),
        "首次尝试必须落一行"
    );
    assert!(
        !store.append_candidate_attempt(&session.id, &first).unwrap(),
        "同一次尝试重复落盘必须幂等零膨胀"
    );

    let second = attempt_record(
        "fail/two",
        CandidateAttemptOutcome::Failed,
        "2026-10-31T00:01:00Z",
    );
    assert!(
        store
            .append_candidate_attempt(&session.id, &second)
            .unwrap(),
        "第二次失败必须另起一行（历史序列，不是当前状态）"
    );
    let third = attempt_record(
        "fail/three",
        CandidateAttemptOutcome::Failed,
        "2026-10-31T00:02:00Z",
    );
    assert!(
        store.append_candidate_attempt(&session.id, &third).unwrap(),
        "第三次失败必须另起一行"
    );
    let fourth = attempt_record(
        "pass/four",
        CandidateAttemptOutcome::Passed,
        "2026-10-31T00:03:00Z",
    );
    assert!(
        store
            .append_candidate_attempt(&session.id, &fourth)
            .unwrap(),
        "成功尝试也必须进历史序列（否则首次通过率无从计算）"
    );

    let records = store.list_candidate_attempts(&session.id).unwrap();
    assert_eq!(records.len(), 4, "四次尝试必须四条记录");
    assert_eq!(
        records
            .iter()
            .map(|record| record.attempt_seq)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4],
        "尝试序号必须会话内单调递增"
    );
    // 同一 issue 连续 3 次失败后仍能逐次读出（阶段/稳定码/typed 种类/指纹）。
    let failures = records
        .iter()
        .filter(|record| record.outcome == CandidateAttemptOutcome::Failed)
        .collect::<Vec<_>>();
    assert_eq!(failures.len(), 3, "三次失败必须三条可追溯记录");
    for failure in &failures {
        let failure = failure.failure.as_ref().expect("失败记录必须带失败事实");
        assert_eq!(failure.code, "final_compile_prepare_failed");
        assert_eq!(failure.cycle_fingerprint.as_deref(), Some("661e48b3"));
        assert_eq!(failure.typed_kinds.len(), 1);
    }
    assert!(
        records.iter().all(|record| record.session_id == session.id),
        "会话身份由 store 盖印，不得由调用方写错"
    );
    assert!(
        records.iter().all(|record| record.issue_id == ISSUE_ID),
        "issue 身份由 store 盖印"
    );

    // 与既有 candidate-compile-status.json 分工：后者是当前状态（覆盖写），
    // 前者是历史序列（只追加）——两者落点同族、内容互不覆盖。
    let path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("candidate-attempts.jsonl");
    let raw = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        raw.lines().count(),
        4,
        "流水必须一行一次尝试，幂等重复写不得多出行"
    );
}

#[test]
fn candidate_attempt_stream_keeps_typed_facts_and_auto_rewrite_phase() {
    let (_tmp, store) = setup();
    let session = create_session(
        &store,
        "work_item_plan_candidate_attempts_typed",
        WorkspaceType::WorkItemPlan,
    );

    let mut record = attempt_record(
        "fail/typed",
        CandidateAttemptOutcome::Failed,
        "2026-10-31T00:00:00Z",
    );
    record.stage = CandidateAttemptStage::Structure;
    record.auto_rewrite = Some(CandidateAutoRewrite {
        phase: crate::product::models::CandidateAutoRewritePhase::Running,
        remaining: Some(2),
        repeated: Some(1),
    });
    record.auto_repairs_applied = vec!["required_capability_missing:contract-001".to_string()];
    record.equivalence_canonicalizations = vec![EquivalenceCanonicalization {
        kind: "intent_target_repository".to_string(),
        original_key: "intent_target_repo".to_string(),
        value: "repository_1b3f3080".to_string(),
        resolution_basis: "member_targets.physical_by_logical".to_string(),
    }];
    store
        .append_candidate_attempt(&session.id, &record)
        .unwrap();

    let loaded = store
        .list_candidate_attempts(&session.id)
        .unwrap()
        .into_iter()
        .next()
        .expect("记录必须可读");
    assert_eq!(loaded.stage, CandidateAttemptStage::Structure);
    let failure = loaded.failure.expect("失败尝试必须带失败事实");
    assert_eq!(failure.code, "final_compile_prepare_failed");
    assert_eq!(
        failure.failure_id, "planning_compile_failure_fail/typed",
        "failure_id 必须逐字保留（内容寻址身份）"
    );
    assert_eq!(failure.cycle_fingerprint.as_deref(), Some("661e48b3"));
    assert_eq!(
        failure.typed_kinds.len(),
        1,
        "typed 种类必须复用既有 PlanningFailureKind，不得另造同义结构"
    );
    let auto_rewrite = loaded.auto_rewrite.expect("自动重写阶段必须落盘");
    assert_eq!(
        auto_rewrite.phase,
        crate::product::models::CandidateAutoRewritePhase::Running
    );
    assert_eq!(auto_rewrite.remaining, Some(2));
    assert_eq!(loaded.auto_repairs_applied.len(), 1);
    assert_eq!(loaded.equivalence_canonicalizations.len(), 1);
}

#[test]
fn candidate_attempt_stream_survives_restart_and_skips_broken_lines() {
    let (tmp, store) = setup();
    let session = create_session(
        &store,
        "work_item_plan_candidate_attempts_restart",
        WorkspaceType::WorkItemPlan,
    );
    store
        .append_candidate_attempt(
            &session.id,
            &attempt_record(
                "fail/one",
                CandidateAttemptOutcome::Failed,
                "2026-10-31T00:00:00Z",
            ),
        )
        .unwrap();

    // 重启（新 store 实例读同一 durable 目录）后仍可读。
    let reopened = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    assert_eq!(
        reopened.list_candidate_attempts(&session.id).unwrap().len(),
        1,
        "流水必须 durable：重启后仍可读"
    );

    // 坏行跳过（不因一行损坏丢掉其余历史）。
    let path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("candidate-attempts.jsonl");
    let mut raw = std::fs::read_to_string(&path).unwrap();
    raw.push_str("{not-json\n");
    std::fs::write(&path, raw).unwrap();
    assert_eq!(
        store.list_candidate_attempts(&session.id).unwrap().len(),
        1,
        "坏行必须跳过，不得让整条流水不可读"
    );
}

#[test]
fn candidate_attempt_append_failure_does_not_corrupt_existing_stream() {
    let (_tmp, store) = setup();
    let session = create_session(
        &store,
        "work_item_plan_candidate_attempts_write_fail",
        WorkspaceType::WorkItemPlan,
    );
    store
        .append_candidate_attempt(
            &session.id,
            &attempt_record(
                "fail/one",
                CandidateAttemptOutcome::Failed,
                "2026-10-31T00:00:00Z",
            ),
        )
        .unwrap();

    // 制造写失败：把流水文件占成目录。
    let path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("candidate-attempts.jsonl");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir_all(&path).unwrap();

    let error = store
        .append_candidate_attempt(
            &session.id,
            &attempt_record(
                "fail/two",
                CandidateAttemptOutcome::Failed,
                "2026-10-31T00:01:00Z",
            ),
        )
        .expect_err("写失败必须以错误返回，由调用点降级为静默");
    assert!(
        !error.to_string().is_empty(),
        "写失败必须带可诊断信息（调用点据此记入运行日志）"
    );
}

#[test]
fn candidate_attempt_metrics_report_pass_rate_percentiles_and_code_top_n() {
    // issue A：首次即通过 → 计入首次通过率，连续失败 0 次。
    let issue_a = CandidateIssueAttemptSeries {
        issue_id: "issue_a".to_string(),
        records: vec![attempt_record(
            "pass/a1",
            CandidateAttemptOutcome::Passed,
            "2026-10-31T00:00:00Z",
        )],
    };
    // issue B：两次失败后通过 → 连续失败 2 次。
    let issue_b = CandidateIssueAttemptSeries {
        issue_id: "issue_b".to_string(),
        records: vec![
            attempt_record(
                "fail/b1",
                CandidateAttemptOutcome::Failed,
                "2026-10-31T00:00:00Z",
            ),
            attempt_record(
                "fail/b2",
                CandidateAttemptOutcome::Failed,
                "2026-10-31T00:01:00Z",
            ),
            attempt_record(
                "pass/b3",
                CandidateAttemptOutcome::Passed,
                "2026-10-31T00:02:00Z",
            ),
        ],
    };
    // issue C：终态失败（lowering 类）→ 连续失败 1 次，终态 lowering 计数 +1。
    let mut lowering = attempt_record(
        "fail/c1",
        CandidateAttemptOutcome::Failed,
        "2026-10-31T00:00:00Z",
    );
    lowering.failure.as_mut().expect("失败事实").code = "lowering_error".to_string();
    let issue_c = CandidateIssueAttemptSeries {
        issue_id: "issue_c".to_string(),
        records: vec![lowering],
    };

    let metrics = compute_candidate_attempt_metrics(&[issue_a, issue_b, issue_c]);
    assert_eq!(metrics.issue_count, 3);
    assert_eq!(metrics.first_pass_issue_count, 1);
    assert_eq!(metrics.first_pass_rate, Some(1.0 / 3.0));
    assert_eq!(metrics.consecutive_failure_p50, Some(1.0));
    assert_eq!(metrics.consecutive_failure_p90, Some(2.0));
    assert_eq!(
        metrics
            .failure_code_ranking
            .first()
            .map(|item| item.code.as_str()),
        Some("final_compile_prepare_failed"),
        "失败码分布必须按次数降序"
    );
    assert_eq!(metrics.failure_code_ranking[0].count, 2);
    assert_eq!(
        metrics.lowering_terminal_failure_count, 1,
        "lowering 类终态失败必须单独计数"
    );
}

/// 评审 Important-1（2026-11-12 整分支评审）：generate 段等价审计行
/// （`attempt_key` 以 `equiv/` 开头，写入点 `record_equivalence_canonicalizations`）
/// 与尝试流水同序列落库，但**不是编译尝试结果**——首过判定与连续失败判定
/// 都必须剔除它，否则「审计行先落、Final Compile 后败」的 issue 会被计成
/// 首次通过、连续失败被截为 0（系统性抬高 first_pass_rate、压低 P50/P90）。
#[test]
fn candidate_attempt_metrics_exclude_equivalence_audit_rows() {
    // Case A：先一条等价审计行（Passed、无失败事实），后一条真实 Final
    // Compile 失败 ⇒ 不得计为首次通过；连续失败 = 1（不是被审计行截成的 0）。
    let mut audit = attempt_record(
        "equiv/node-1/0123456789abcdef",
        CandidateAttemptOutcome::Passed,
        "2026-11-11T00:00:00Z",
    );
    audit.equivalence_canonicalizations = vec![EquivalenceCanonicalization {
        kind: "intent_target_repository".to_string(),
        original_key: "intent_target_codebase".to_string(),
        value: "aria".to_string(),
        resolution_basis: "member_table/physical_by_logical".to_string(),
    }];
    let issue_a = CandidateIssueAttemptSeries {
        issue_id: "issue_audited".to_string(),
        records: vec![
            audit,
            attempt_record(
                "fail/after-equiv",
                CandidateAttemptOutcome::Failed,
                "2026-11-11T00:01:00Z",
            ),
        ],
    };

    let metrics = compute_candidate_attempt_metrics(&[issue_a]);
    assert_eq!(metrics.issue_count, 1);
    assert_eq!(
        metrics.first_pass_issue_count, 0,
        "等价审计行不得把随后编译失败的 issue 计成首次通过"
    );
    assert_eq!(metrics.first_pass_rate, Some(0.0));
    assert_eq!(
        metrics.consecutive_failure_p50,
        Some(1.0),
        "审计行后的真实失败必须计入连续失败（不得被审计行截断为 0）"
    );
    assert_eq!(metrics.consecutive_failure_p90, Some(1.0));

    // Case B（对照）：无审计行的真实失败序列 ⇒ 过滤不得把正常路径也滤掉。
    let issue_b = CandidateIssueAttemptSeries {
        issue_id: "issue_plain".to_string(),
        records: vec![attempt_record(
            "fail/no-audit",
            CandidateAttemptOutcome::Failed,
            "2026-11-11T00:00:00Z",
        )],
    };
    let metrics = compute_candidate_attempt_metrics(&[issue_b]);
    assert_eq!(metrics.issue_count, 1);
    assert_eq!(metrics.first_pass_issue_count, 0);
    assert_eq!(metrics.first_pass_rate, Some(0.0));
    assert_eq!(metrics.consecutive_failure_p50, Some(1.0));
    assert_eq!(metrics.consecutive_failure_p90, Some(1.0));
}

#[test]
fn candidate_attempt_metrics_aggregate_issue_level_streams() {
    let (_tmp, store) = setup();
    let session = create_session(
        &store,
        "work_item_plan_candidate_attempts_aggregate",
        WorkspaceType::WorkItemPlan,
    );
    store
        .append_candidate_attempt(
            &session.id,
            &attempt_record(
                "fail/one",
                CandidateAttemptOutcome::Failed,
                "2026-10-31T00:00:00Z",
            ),
        )
        .unwrap();
    store
        .append_candidate_attempt(
            &session.id,
            &attempt_record(
                "pass/two",
                CandidateAttemptOutcome::Passed,
                "2026-10-31T00:01:00Z",
            ),
        )
        .unwrap();

    let records = store
        .list_issue_candidate_attempts(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(records.len(), 2, "issue 级读数必须跨会话汇总");
    let series = vec![CandidateIssueAttemptSeries {
        issue_id: ISSUE_ID.to_string(),
        records,
    }];
    let metrics = compute_candidate_attempt_metrics(&series);
    assert_eq!(metrics.issue_count, 1);
    assert_eq!(metrics.first_pass_issue_count, 0);
    assert_eq!(metrics.first_pass_rate, Some(0.0));
    assert_eq!(metrics.consecutive_failure_p50, Some(1.0));
}

/// REQ-WSC-25 修订（2026-10-31 真机）：「终止自动化 + 重开」会**删掉旧会话
/// 目录**（连带其流水）——落会话目录的读数会被重跑擦掉：同一个 issue 上一次
/// 跑 `first_pass_rate=1.0`，重跑一次后变成 `0.0`，上一次那条
/// `outcome=passed` 的记录被删没了。流水必须耐得住会话清理：
///
/// 1. issue 级读数仍包含**已删会话**的尝试（含旧的 `outcome=passed`）；
/// 2. `first_pass_rate` 不因重跑回退；
/// 3. 会话级读（`list_candidate_attempts`）语义不变：只返回该会话的记录。
#[test]
fn candidate_attempt_stream_survives_session_directory_cleanup() {
    let (_tmp, store) = setup();
    // 第一次跑：通过（`first_pass_rate=1.0` 的来源）。
    let passed_session = create_session(
        &store,
        "work_item_plan_candidate_attempts_cleanup_passed",
        WorkspaceType::WorkItemPlan,
    );
    // 第二次跑：失败。
    let failed_session = create_session(
        &store,
        "work_item_plan_candidate_attempts_cleanup_failed",
        WorkspaceType::WorkItemPlan,
    );
    store
        .append_candidate_attempt(
            &passed_session.id,
            &attempt_record(
                "pass/first-run",
                CandidateAttemptOutcome::Passed,
                "2026-10-31T00:00:00Z",
            ),
        )
        .unwrap();
    store
        .append_candidate_attempt(
            &failed_session.id,
            &attempt_record(
                "fail/second-run",
                CandidateAttemptOutcome::Failed,
                "2026-10-31T00:01:00Z",
            ),
        )
        .unwrap();

    // 「终止 + 重开」：旧会话目录（连同其流水）被删除。
    let session_dir = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions")
        .join(&passed_session.id);
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("session-marker.txt"), "old session").unwrap();
    std::fs::remove_dir_all(&session_dir).unwrap();
    assert!(
        !session_dir.exists(),
        "旧会话目录必须已被删除（模拟「终止+重开」）"
    );

    // ① issue 级读数：旧会话的尝试仍在（流水不随会话清理消失）。
    let records = store
        .list_issue_candidate_attempts(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(
        records.len(),
        2,
        "会话目录被删后旧会话的尝试仍须可读：{records:?}"
    );
    assert!(
        records.iter().any(|record| {
            record.session_id == passed_session.id
                && record.outcome == CandidateAttemptOutcome::Passed
        }),
        "旧会话的 passed 记录必须仍在（读数不得被重跑擦掉）：{records:?}"
    );

    // ② 读数不回退：首次通过率仍是 1.0（重跑不得把它拉回 0.0）。
    let metrics = compute_candidate_attempt_metrics(&[CandidateIssueAttemptSeries {
        issue_id: ISSUE_ID.to_string(),
        records,
    }]);
    assert_eq!(
        metrics.first_pass_rate,
        Some(1.0),
        "重跑后首次通过率不得回退"
    );
    assert_eq!(
        metrics
            .failure_code_ranking
            .first()
            .map(|item| item.code.as_str()),
        Some("final_compile_prepare_failed"),
        "第二次跑的失败事实仍须进失败码分布"
    );

    // ③ 会话级读语义不变：只返回该会话自己的记录。
    let failed_rows = store.list_candidate_attempts(&failed_session.id).unwrap();
    assert_eq!(failed_rows.len(), 1);
    assert_eq!(failed_rows[0].outcome, CandidateAttemptOutcome::Failed);
    let passed_rows = store.list_candidate_attempts(&passed_session.id).unwrap();
    assert_eq!(passed_rows.len(), 1, "会话级读仍只返回该会话的记录");
    assert_eq!(passed_rows[0].outcome, CandidateAttemptOutcome::Passed);
}

/// 升级兼容面：旧版落点（会话目录内的 `candidate-attempts.jsonl`）里**已落盘**
/// 的历史行不得从读数里消失（升级前写下的记录不能因为改了落点就看不见），
/// 也不得与 issue 级权威文件里的同键行双计；序号跨两个落点连续。
#[test]
fn candidate_attempt_stream_keeps_legacy_session_level_rows_without_double_counting() {
    let (_tmp, store) = setup();
    let session = create_session(
        &store,
        "work_item_plan_candidate_attempts_legacy",
        WorkspaceType::WorkItemPlan,
    );

    // 升级前：流水落会话目录（store 盖印的会话/issue 身份照旧）。
    let legacy_path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions")
        .join(&session.id)
        .join("candidate-attempts.jsonl");
    std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
    let mut legacy = attempt_record(
        "fail/legacy",
        CandidateAttemptOutcome::Failed,
        "2026-10-31T00:00:00Z",
    );
    legacy.session_id = session.id.clone();
    legacy.issue_id = ISSUE_ID.to_string();
    legacy.attempt_seq = 1;
    std::fs::write(
        &legacy_path,
        format!("{}\n", serde_json::to_string(&legacy).unwrap()),
    )
    .unwrap();

    // 升级后：新的尝试落 issue 级权威文件，序号跨两个落点连续（2，不是 1）。
    store
        .append_candidate_attempt(
            &session.id,
            &attempt_record(
                "fail/new",
                CandidateAttemptOutcome::Failed,
                "2026-10-31T00:01:00Z",
            ),
        )
        .unwrap();

    let issue_records = store
        .list_issue_candidate_attempts(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(
        issue_records.len(),
        2,
        "旧版落点的历史行不得从读数里消失，也不得双计：{issue_records:?}"
    );
    let session_records = store.list_candidate_attempts(&session.id).unwrap();
    assert_eq!(session_records.len(), 2, "会话级读同样并入旧版落点的行");
    assert_eq!(
        session_records
            .iter()
            .map(|record| record.attempt_seq)
            .collect::<Vec<_>>(),
        vec![1, 2],
        "尝试序号必须跨两个落点连续（不撞号）"
    );

    // 同一尝试（旧版落点已有同键行）不得在读数里出现两次。
    store
        .append_candidate_attempt(
            &session.id,
            &attempt_record(
                "fail/legacy",
                CandidateAttemptOutcome::Failed,
                "2026-10-31T00:00:00Z",
            ),
        )
        .unwrap();
    let merged = store
        .list_issue_candidate_attempts(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(
        merged.len(),
        2,
        "同键行只算一次（issue 级权威文件优先）：{merged:?}"
    );
}
