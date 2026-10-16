//! Bug2-B(Task 4)：启动 gate-only 对账用例——自 runner_recovery.rs 拆出
//! 守 1200 行文件纪律；零语义变更纯移动（gate 不变，测试集合不变）。

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingExecutionAttempt, CodingExecutionStage, CodingProviderRole, CodingRoleRunStatus,
    CodingRoleRunTrigger,
};

use super::{StartupReconcileFixture, seed_startup_attempt};

// ---------------------------------------------------------------------------
// Bug2-B(Task 4)：启动 gate-only 对账——每 attempt 分诊前收敛门的崩溃
// 窗口；只动门、零 provider 调用；无关联旧门只告警 JSON 不动。
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum GateCrashPrefix {
    ResolvedRootOnly,
    ArchivePlusResidualRoot,
    ArchiveOnly,
    OpenUnassociated,
    OpenWithResidualArchive,
}

fn startup_gate_input(
    attempt: &CodingExecutionAttempt,
    reason_code: &str,
) -> crate::product::coding_attempt_store::CreateBlockedGateInput {
    crate::product::coding_attempt_store::CreateBlockedGateInput {
        attempt_id: attempt.id.clone(),
        stage: CodingExecutionStage::CodeReview,
        node_id: None,
        role: Some(CodingProviderRole::CodeReviewer),
        title: "代码审查中断".to_string(),
        description: "startup gate reconcile fixture".to_string(),
        reason_code: Some(reason_code.to_string()),
        evidence_refs: Vec::new(),
        raw_provider_output_ref: None,
        available_actions: Vec::new(),
    }
}

fn seed_origin_run(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    origin: crate::product::coding_models::GateOrigin,
    status: CodingRoleRunStatus,
) -> String {
    let run = store
        .create_role_run(
            attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::ManualRetry,
            None,
        )
        .expect("seed role run");
    let mut saved = store
        .get_role_run(&attempt.project_id, &attempt.issue_id, &attempt.id, &run.id)
        .expect("reload role run");
    saved.gate_origin = Some(origin);
    store
        .save_role_run(&attempt.project_id, &attempt.issue_id, &saved)
        .expect("persist gate origin");
    if status != CodingRoleRunStatus::Running {
        store
            .update_role_run_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &run.id,
                status,
                None,
            )
            .expect("terminal role run status");
    }
    run.id
}

#[tokio::test]
async fn startup_gate_reconcile_converges_crash_prefixes_without_providers() {
    for prefix in [
        GateCrashPrefix::ResolvedRootOnly,
        GateCrashPrefix::ArchivePlusResidualRoot,
        GateCrashPrefix::ArchiveOnly,
        GateCrashPrefix::OpenUnassociated,
        GateCrashPrefix::OpenWithResidualArchive,
    ] {
        let fixture = seed_startup_attempt(None, false);
        let store = fixture.store();
        let attempt = fixture.reload();
        let (p, i) = (attempt.project_id.clone(), attempt.issue_id.clone());
        let a = attempt.id.clone();
        let gate = store
            .create_blocked_gate(
                &attempt,
                startup_gate_input(&attempt, "code_review_provider_interrupted"),
            )
            .expect("seed gate");
        let gates_root = store.blocked_gates_root(&p, &i, &a);
        let root_path = gates_root.join(format!("{}.json", gate.gate_id));
        let archive_path = gates_root
            .join("resolved")
            .join(format!("{}.json", gate.gate_id));

        match prefix {
            GateCrashPrefix::ResolvedRootOnly => {
                // 崩溃于「根已写完整关闭事实、归档未写」：首关落盘后把
                // 归档内容搬回根并删归档。
                store
                    .resolve_blocked_gate_with_action(
                        &p,
                        &i,
                        &a,
                        &gate.gate_id,
                        Some("retry_review"),
                    )
                    .expect("stage first close");
                let archived = std::fs::read_to_string(&archive_path).expect("staged archive");
                std::fs::write(&root_path, archived).expect("stage resolved root");
                std::fs::remove_file(&archive_path).expect("drop archive");
            }
            GateCrashPrefix::ArchivePlusResidualRoot => {
                store
                    .resolve_blocked_gate_with_action(
                        &p,
                        &i,
                        &a,
                        &gate.gate_id,
                        Some("retry_review"),
                    )
                    .expect("stage close");
                std::fs::copy(&archive_path, &root_path).expect("stage residual root");
            }
            GateCrashPrefix::ArchiveOnly | GateCrashPrefix::OpenUnassociated => {
                if matches!(prefix, GateCrashPrefix::ArchiveOnly) {
                    store
                        .resolve_blocked_gate_with_action(
                            &p,
                            &i,
                            &a,
                            &gate.gate_id,
                            Some("retry_review"),
                        )
                        .expect("stage close");
                }
            }
            GateCrashPrefix::OpenWithResidualArchive => {
                // 重开半完成残留：open 根 + 上一代归档并存。
                store
                    .resolve_blocked_gate_with_action(
                        &p,
                        &i,
                        &a,
                        &gate.gate_id,
                        Some("retry_review"),
                    )
                    .expect("stage close");
                let archived = std::fs::read_to_string(&archive_path).expect("staged archive");
                store
                    .reopen_failed_code_review_gate_for_plan_repair(&p, &i, &a, &gate.gate_id)
                    .expect("stage reopen");
                std::fs::write(&archive_path, archived).expect("stage residual archive");
            }
        }
        let before_root = std::fs::read_to_string(&root_path).ok();
        let before_archive = std::fs::read_to_string(&archive_path).ok();

        let fresh = fixture.restart_state();
        let resumed = fixture.reconcile(&fresh).await;
        assert_eq!(
            resumed, 0,
            "{prefix:?}: gate-only reconcile must not resume runners"
        );
        assert_eq!(
            fresh.coding_runs.runner_count(&fixture.attempt_key()),
            0,
            "{prefix:?}: gate reconcile must not interact with providers"
        );

        match prefix {
            GateCrashPrefix::ResolvedRootOnly => {
                assert!(!root_path.exists(), "{prefix:?}: root removed");
                let facts = store
                    .get_archived_blocked_gate_facts(&p, &i, &a, &gate.gate_id)
                    .expect("archived facts")
                    .expect("archive completed");
                assert_eq!(facts.resolved_action.as_deref(), Some("retry_review"));
                assert!(
                    facts
                        .resolved_at
                        .as_deref()
                        .is_some_and(|at| !at.is_empty()),
                    "{prefix:?}: first-close facts preserved"
                );
            }
            GateCrashPrefix::ArchivePlusResidualRoot => {
                assert!(!root_path.exists(), "{prefix:?}: residual root cleaned");
                assert_eq!(
                    std::fs::read_to_string(&archive_path).expect("archive"),
                    before_archive.expect("staged archive"),
                    "{prefix:?}: archive first-close facts unchanged"
                );
            }
            GateCrashPrefix::ArchiveOnly => {
                assert_eq!(
                    std::fs::read_to_string(&archive_path).expect("archive"),
                    before_archive.expect("staged archive"),
                    "{prefix:?}: idempotent replay must not rewrite the archive"
                );
            }
            GateCrashPrefix::OpenUnassociated => {
                assert_eq!(
                    std::fs::read_to_string(&root_path).expect("root"),
                    before_root.expect("staged root"),
                    "{prefix:?}: unassociated gate must stay untouched (warn only)"
                );
                assert!(
                    store
                        .list_open_blocked_gates(&p, &i, &a)
                        .expect("open gates")
                        .iter()
                        .any(|open| open.gate_id == gate.gate_id),
                    "{prefix:?}: gate stays open for manual attention"
                );
            }
            GateCrashPrefix::OpenWithResidualArchive => {
                assert!(
                    root_path.exists(),
                    "{prefix:?}: open root is the current fact"
                );
                assert!(
                    !archive_path.exists(),
                    "{prefix:?}: residual previous-generation archive removed"
                );
                let reopened = std::fs::read_to_string(&root_path).expect("root");
                let value: serde_json::Value = serde_json::from_str(&reopened).unwrap();
                assert_eq!(value["status"], serde_json::json!("open"));
                assert_eq!(value["resolution_generation"], serde_json::json!(1));
            }
        }
    }
}

#[tokio::test]
async fn startup_gate_reconcile_compensates_only_terminal_origin_matched_runs() {
    let fixture = seed_startup_attempt(None, false);
    let store = fixture.store();
    let attempt = fixture.reload();
    let (p, i) = (attempt.project_id.clone(), attempt.issue_id.clone());
    let a = attempt.id.clone();

    // ① open 门 × 终态 run × origin 精确匹配（代次一致）→ 幂等补偿关。
    let gate_a = store
        .create_blocked_gate(
            &attempt,
            startup_gate_input(&attempt, "code_review_provider_interrupted_a"),
        )
        .expect("gate a");
    seed_origin_run(
        &store,
        &attempt,
        crate::product::coding_models::GateOrigin {
            gate_id: gate_a.gate_id.clone(),
            generation: 0,
            action_id: "retry_review".to_string(),
        },
        CodingRoleRunStatus::Failed,
    );

    // ② 重开后的新门（gen 1）：旧 run 的 origin 停留 gen 0 → 不关。
    let gate_b = store
        .create_blocked_gate(
            &attempt,
            startup_gate_input(&attempt, "code_review_provider_interrupted_b"),
        )
        .expect("gate b");
    store
        .resolve_blocked_gate_with_action(&p, &i, &a, &gate_b.gate_id, Some("retry_review"))
        .expect("close gate b");
    store
        .reopen_failed_code_review_gate_for_plan_repair(&p, &i, &a, &gate_b.gate_id)
        .expect("reopen gate b");
    seed_origin_run(
        &store,
        &attempt,
        crate::product::coding_models::GateOrigin {
            gate_id: gate_b.gate_id.clone(),
            generation: 0,
            action_id: "retry_review".to_string(),
        },
        CodingRoleRunStatus::Failed,
    );

    // ③ open 门 × origin 匹配但 run 仍 Running → 不关（终态先持久化）。
    let gate_c = store
        .create_blocked_gate(
            &attempt,
            startup_gate_input(&attempt, "code_review_provider_interrupted_c"),
        )
        .expect("gate c");
    seed_origin_run(
        &store,
        &attempt,
        crate::product::coding_models::GateOrigin {
            gate_id: gate_c.gate_id.clone(),
            generation: 0,
            action_id: "abort".to_string(),
        },
        CodingRoleRunStatus::Running,
    );

    let fresh = fixture.restart_state();
    let resumed = fixture.reconcile(&fresh).await;
    assert_eq!(resumed, 0);

    let compensated = store
        .get_archived_blocked_gate_facts(&p, &i, &a, &gate_a.gate_id)
        .expect("gate a facts")
        .expect("gate a compensated at startup");
    assert_eq!(compensated.resolved_action.as_deref(), Some("retry_review"));
    assert!(
        compensated
            .resolved_at
            .as_deref()
            .is_some_and(|at| !at.is_empty())
    );
    let open_ids: Vec<String> = store
        .list_open_blocked_gates(&p, &i, &a)
        .expect("open gates")
        .into_iter()
        .map(|gate| gate.gate_id)
        .collect();
    assert!(
        open_ids.contains(&gate_b.gate_id),
        "stale-generation origin must not close the reopened gate: {open_ids:?}"
    );
    assert!(
        open_ids.contains(&gate_c.gate_id),
        "non-terminal run must not close its gate: {open_ids:?}"
    );
    assert!(!open_ids.contains(&gate_a.gate_id));
}

#[tokio::test]
async fn startup_gate_reconcile_backfills_archived_action_from_journal_evidence() {
    let fixture = seed_startup_attempt(None, false);
    let store = fixture.store();
    let attempt = fixture.reload();
    let (p, i) = (attempt.project_id.clone(), attempt.issue_id.clone());
    let a = attempt.id.clone();

    // ① 旧档：无 action 归档 + 恢复 journal 可证 retry_review → 按证据回填。
    let gate_a = store
        .create_blocked_gate(
            &attempt,
            startup_gate_input(&attempt, "code_review_provider_interrupted_a"),
        )
        .expect("gate a");
    // 旧档模拟：无 action 归档(剥除 resolved_action/resolved_at 字段,
    // 复现 Task 3 前的 legacy 形态——历史档缺时间,回填不得伪造)。
    let strip_legacy_closure = |gate_id: &str| {
        let archive_path = store
            .blocked_gates_root(&p, &i, &a)
            .join("resolved")
            .join(format!("{gate_id}.json"));
        let mut record: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&archive_path).expect("archive to strip"),
        )
        .expect("archive json");
        record["resolved_action"].take();
        record["resolved_at"].take();
        std::fs::write(
            &archive_path,
            serde_json::to_string_pretty(&record).expect("stripped archive"),
        )
        .expect("write stripped archive");
    };
    store
        .resolve_blocked_gate(&p, &i, &a, &gate_a.gate_id)
        .expect("close gate a");
    strip_legacy_closure(&gate_a.gate_id);
    let journal = store
        .prepare_failed_code_review_recovery_journal(
            &attempt,
            &gate_a.gate_id,
            "coding_node_0009",
            "coding_role_run_0001",
        )
        .expect("recovery journal");
    let retry_run = seed_origin_run(
        &store,
        &attempt,
        crate::product::coding_models::GateOrigin {
            gate_id: gate_a.gate_id.clone(),
            generation: 0,
            action_id: "retry_review".to_string(),
        },
        CodingRoleRunStatus::Running,
    );
    store
        .advance_failed_code_review_recovery_journal(
            &journal,
            crate::product::coding_attempt_store::FailedCodeReviewRecoveryPhase::RetryRunCreated,
            Some(&retry_run),
        )
        .expect("journal proves retry acceptance");

    // ② 无 journal 证据的缺 action 归档 → 不动。
    let gate_b = store
        .create_blocked_gate(
            &attempt,
            startup_gate_input(&attempt, "code_review_provider_interrupted_b"),
        )
        .expect("gate b");
    store
        .resolve_blocked_gate(&p, &i, &a, &gate_b.gate_id)
        .expect("close gate b");
    strip_legacy_closure(&gate_b.gate_id);

    let fresh = fixture.restart_state();
    fixture.reconcile(&fresh).await;

    let a_facts = store
        .get_archived_blocked_gate_facts(&p, &i, &a, &gate_a.gate_id)
        .expect("gate a facts")
        .expect("gate a archived");
    assert_eq!(
        a_facts.resolved_action.as_deref(),
        Some("retry_review"),
        "journal-proven retry_review must backfill the missing action"
    );
    assert_eq!(
        a_facts.resolved_at, None,
        "历史时间不伪造：回填只补动作，不补时间"
    );
    let b_facts = store
        .get_archived_blocked_gate_facts(&p, &i, &a, &gate_b.gate_id)
        .expect("gate b facts")
        .expect("gate b archived");
    assert_eq!(
        b_facts.resolved_action, None,
        "no journal evidence → archive untouched"
    );
}

#[tokio::test]
async fn startup_gate_reconcile_skips_gates_under_recovery_arbitration() {
    let fixture = seed_startup_attempt(None, false);
    let store = fixture.store();
    let attempt = fixture.reload();
    let (p, i) = (attempt.project_id.clone(), attempt.issue_id.clone());
    let a = attempt.id.clone();
    let gate = store
        .create_blocked_gate(
            &attempt,
            startup_gate_input(&attempt, "code_review_provider_interrupted"),
        )
        .expect("gate");
    // 仲裁中的恢复 journal（未 completed、无 retry run）→ 不抢先关。
    store
        .prepare_failed_code_review_recovery_journal(
            &attempt,
            &gate.gate_id,
            "coding_node_0009",
            "coding_role_run_0001",
        )
        .expect("arbitration journal");
    // 即便存在带 origin 的终态 run（罕见竞态残留），仲裁优先于补偿。
    seed_origin_run(
        &store,
        &attempt,
        crate::product::coding_models::GateOrigin {
            gate_id: gate.gate_id.clone(),
            generation: 0,
            action_id: "retry_review".to_string(),
        },
        CodingRoleRunStatus::Failed,
    );

    let fresh = fixture.restart_state();
    fixture.reconcile(&fresh).await;
    assert!(
        store
            .list_open_blocked_gates(&p, &i, &a)
            .expect("open gates")
            .iter()
            .any(|open| open.gate_id == gate.gate_id),
        "arbitration in flight: startup reconcile must not preempt the gate"
    );
}
