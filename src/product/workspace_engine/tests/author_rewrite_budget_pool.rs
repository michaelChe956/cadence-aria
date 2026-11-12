//! WIG 作者返修配额池（fix-author-rewrite-quota-pool）Task 3：未委托对账
//! 回退与崩溃窗两态收敛（工作包 3.3 / 3.4 / 3.5；REQ-WSC-21 共池）。
//!
//! 夹具复用 `compile_failure_auto_rewrite.rs` 的 `compile_failure_fixture`
//! 构造口径（干净候选 + 初评已过 + 编译失败事实落 durable，临时目录由
//! tempfile 夹具自管）。本文件是 Task 3/4/5/6 共享的引擎集成测试文件。
//!
//! 两态收敛（工作包 3.4）：「扣减+预领 CAS 已落、provider 未启动」的崩溃窗
//! 只允许收敛到两个终态——
//! - **已退未跑**：对账回退（`refund_author_rewrite_charge`）落 refunded 留痕；
//! - **已扣已跑**：接力消费（reservation `Reserved` → `ProviderStarted`）后
//!   扣减保留。
//!
//! 不存在第三态：净扣 > 0 且未启动且未退款 ⇒ [`assert_no_dangling_charge`]
//! 断言红（悬空扣减 = 白占额度）。

use super::compile_failure_auto_rewrite::{compile_failure_fixture, drain_engine_events};
use super::*;
use crate::product::json_store::write_json;
use crate::product::models::CandidateAutoRewritePhase;
use crate::product::models::author_rewrite_budget::{
    AuthorRewriteBudgetInputs, available_author_rewrites, net_auto_charged, with_preclaim_charge,
};
use crate::product::work_item_plan_policy::{
    ProviderStartLedgerEntry, RepairReservation, RepairReservationState,
};

/// 预领扣减 + Reserved 预留的「CAS 已落」形态预置：字段级复刻 Task 2
/// `provider_start_ledger_for_action` / `preclaimed_repair_start` 的产物
/// （attempt=0，与初评通过后的首个 TriggerAggregateRepair 预领一致）；
/// 预算域用真实纯函数 `with_preclaim_charge` 建立（legacy=1，即夹具
/// `auto_rewrites_used` 的读侧归并口径），返回预领键。
fn preland_preclaim_charge(
    lifecycle: &LifecycleStore,
    record: &mut crate::product::models::WorkspaceSessionRecord,
) -> String {
    let key = format!("single_candidate_author:{}:0", record.id);
    record.repair_reservation = Some(RepairReservation {
        token: format!("single_candidate_author_repair:{}:0", record.id),
        owner_session_id: record.id.clone(),
        owner_run_id: "review_unknown".to_string(),
        provider_start_idempotency_key: key.clone(),
        state: RepairReservationState::Reserved,
        commit_id: None,
    });
    record.provider_start_ledger.push(ProviderStartLedgerEntry {
        provider_start_idempotency_key: key.clone(),
        started: true,
        provider: None,
        started_at: None,
    });
    record.author_rewrite_budget = Some(with_preclaim_charge(
        record.author_rewrite_budget.as_ref(),
        &key,
        1,
        "2026-11-12T00:00:00Z".to_string(),
    ));
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        record,
    )
    .expect("persist prelanded preclaim charge");
    key
}

/// 两态收敛不变量（工作包 3.4）：每个未退款的扣减都必须有「真跑过」证据
/// （reservation 已 `ProviderStarted`，或该键有实际启动观察）。净扣 > 0 且
/// 未启动且未退款 = 第三态（悬空扣减）⇒ 断言红。
fn assert_no_dangling_charge(lifecycle: &LifecycleStore, session_id: &str) {
    let record = lifecycle
        .get_workspace_session(session_id)
        .expect("load session for dangling-charge invariant");
    let Some(budget) = record.author_rewrite_budget.as_ref() else {
        return;
    };
    for charge in &budget.auto_rewrite_charges {
        if charge.refunded_at.is_some() {
            continue;
        }
        let reservation_consumed = record
            .repair_reservation
            .as_ref()
            .is_some_and(|reservation| {
                reservation.provider_start_idempotency_key == charge.provider_start_idempotency_key
                    && reservation.state == RepairReservationState::ProviderStarted
            });
        let launch_observed = record.provider_start_ledger.iter().any(|entry| {
            entry.provider_start_idempotency_key == charge.provider_start_idempotency_key
                && entry.is_actual_launch_observation()
        });
        assert!(
            reservation_consumed || launch_observed,
            "悬空扣减（第三态）：键 {} 净扣 > 0 但既未启动也未退款",
            charge.provider_start_idempotency_key
        );
    }
}

/// 工作包 3.3 / REQ-WSC-25：扣减已落（charges.len()==1）但路由改道人工门
/// （未委托）⇒ reconcile 回退扣减、留痕不删：
/// - durable 扣减 refunded（自动段账本净扣 0），重放幂等（只写一次）；
/// - `auto_rewrite.phase == Stopped` 留痕、剩余/重复计数保留；
/// - 诊断计数 `auto_rewrites_used` 不回退（REQ-WSC-25 尝试序列保留）；
/// - Error 事件痕迹保留。
#[tokio::test]
async fn undelegated_reconcile_refunds_charge_and_keeps_observable_trace() {
    let (_tmp, lifecycle, mut engine, mut event_rx, _cycle_key) = compile_failure_fixture(1).await;
    let session_id = engine.session().session_id.clone();

    // 崩溃窗起点：Task 2 的预领扣减 + Reserved 预留已落 durable（provider 未启动）。
    let mut record = lifecycle
        .get_workspace_session(&session_id)
        .expect("load fixture record");
    let key = preland_preclaim_charge(&lifecycle, &mut record);
    let budget = record
        .author_rewrite_budget
        .as_ref()
        .expect("prelanded budget");
    assert_eq!(
        budget.auto_rewrite_charges.len(),
        1,
        "前置：扣减已落恰好一条"
    );
    assert_eq!(budget.total_auto_rewrite_charges_ever, 1);

    // 路由改道人工门（复评预算已耗尽形态）⇒ 引擎内对账回退 + phase→Stopped。
    let routed = engine.route_compile_failure_to_rewrite().await;
    assert!(
        routed,
        "改道人工门的轮次必须被通道消费（reconcile 路径可达）"
    );

    let persisted = lifecycle
        .get_workspace_session(&session_id)
        .expect("load refunded session");
    let refunded_budget = persisted
        .author_rewrite_budget
        .as_ref()
        .expect("budget domain survives refund");
    let charge = refunded_budget
        .auto_rewrite_charges
        .iter()
        .find(|charge| charge.provider_start_idempotency_key == key)
        .expect("refunded charge record kept (留痕不删)");
    let first_refunded_at = charge
        .refunded_at
        .clone()
        .expect("undelegated preclaim must be refunded");
    assert!(
        refunded_budget
            .auto_rewrite_charges
            .iter()
            .all(|charge| charge.refunded_at.is_some()),
        "自动段账本净扣 0：唯一一条扣减已退"
    );
    assert_eq!(
        net_auto_charged(Some(refunded_budget), 0),
        refunded_budget.legacy_auto_rewrites_charged,
        "自动段净扣必须收敛回 legacy 基线（账本净扣 0，基线不动）"
    );
    assert_eq!(
        refunded_budget.total_auto_rewrite_charges_ever, 1,
        "total_ever 保留（REQ-WSC-25 可观测）"
    );

    // 幂等重放：已退的键再退 ⇒ Ok(()) 且 refunded_at 只写一次。
    lifecycle
        .refund_author_rewrite_charge(&session_id, &key, "2099-01-01T00:00:00Z".to_string())
        .expect("refund replay must be idempotent");
    let replayed = lifecycle
        .get_workspace_session(&session_id)
        .expect("reload after refund replay");
    let replayed_charge = replayed
        .author_rewrite_budget
        .as_ref()
        .and_then(|budget| {
            budget
                .auto_rewrite_charges
                .iter()
                .find(|charge| charge.provider_start_idempotency_key == key)
        })
        .expect("charge record kept after replay");
    assert_eq!(
        replayed_charge.refunded_at.as_deref(),
        Some(first_refunded_at.as_str()),
        "refunded_at 只写一次（幂等重放不得改写）"
    );

    // 留痕与可观测：phase==Stopped、诊断计数与剩余/重复计数保留、Error 事件在场。
    let status = lifecycle
        .load_candidate_compile_status(&session_id)
        .expect("load compile status")
        .expect("status present");
    assert_eq!(
        status.auto_rewrite.as_ref().expect("auto rewrite").phase,
        CandidateAutoRewritePhase::Stopped,
        "未委托必须对账为 Stopped 留痕（不谎报 running）"
    );
    assert_eq!(
        status.auto_rewrites_used, 1,
        "诊断计数不回退（REQ-WSC-25 尝试序列保留）"
    );
    assert_eq!(
        status.auto_rewrite.as_ref().unwrap().remaining,
        Some(2),
        "剩余计数保留"
    );
    assert_eq!(
        status.auto_rewrite.as_ref().unwrap().repeated,
        Some(1),
        "重复计数保留"
    );
    assert!(
        drain_engine_events(&mut event_rx)
            .iter()
            .any(|event| matches!(event, EngineEvent::Error { .. })),
        "对账必须留下 Error 事件痕迹"
    );
    // 两态不变量：未退款扣减必须有真跑证据（此处全部已退 ⇒ 无悬空）。
    assert_no_dangling_charge(&lifecycle, &session_id);
}

/// 工作包 3.4 / 3.5：崩溃窗两态收敛——「扣减+预领 CAS 已落、provider 未启动
/// （reservation == Reserved）」之后只有两条收敛路径：
/// - 走 reconcile ⇒ 「已退未跑」（扣减退款留痕）；
/// - 走接力消费（`reserve_single_candidate_provider_start` ⇒ `ProviderStarted`）
///   ⇒ 「已扣已跑」（扣减保留）。
/// 两个终态都必须满足 [`assert_no_dangling_charge`]：净扣 > 0 且未启动且未
/// 退款 ⇒ 断言红（第三态不存在）。
#[tokio::test]
async fn crash_window_converges_to_two_states_only() {
    // ── 分支 A：委托轮落盘（真实 TAR 路径）后走对账回退 ⇒ 「已退未跑」。
    let (_tmp_a, lifecycle_a, mut engine_a, _rx_a, _cycle_a) = compile_failure_fixture(0).await;
    let session_a = engine_a.session().session_id.clone();
    assert!(
        engine_a.route_compile_failure_to_rewrite().await,
        "委托轮必须被通道消费（TAR 预领 + 扣减同 CAS 落盘）"
    );
    let landed_a = lifecycle_a
        .get_workspace_session(&session_a)
        .expect("load landed record");
    let key_a = format!("single_candidate_author:{}:0", landed_a.id);
    assert!(
        landed_a
            .repair_reservation
            .as_ref()
            .is_some_and(|reservation| {
                reservation.provider_start_idempotency_key == key_a
                    && reservation.state == RepairReservationState::Reserved
            }),
        "前置：预领仍 Reserved（扣减已落、provider 未启动的崩溃窗）"
    );
    assert!(
        landed_a
            .author_rewrite_budget
            .as_ref()
            .is_some_and(|budget| {
                budget.auto_rewrite_charges.iter().any(|charge| {
                    charge.provider_start_idempotency_key == key_a && charge.refunded_at.is_none()
                })
            }),
        "前置：预领键的扣减已同 CAS 落盘且未退"
    );

    // 恢复路径重评改道人工门（未委托）⇒ 对账回退。
    engine_a.reconcile_auto_rewrite_not_delegated().await;
    let refunded = lifecycle_a
        .get_workspace_session(&session_a)
        .expect("load refunded record");
    let charge_a = refunded
        .author_rewrite_budget
        .as_ref()
        .and_then(|budget| {
            budget
                .auto_rewrite_charges
                .iter()
                .find(|charge| charge.provider_start_idempotency_key == key_a)
        })
        .expect("「已退未跑」留痕保留");
    assert!(
        charge_a.refunded_at.is_some(),
        "分支 A 收敛为「已退未跑」：扣减必须被回退"
    );
    assert_no_dangling_charge(&lifecycle_a, &session_a);

    // ── 分支 B：同一崩溃窗起点走接力消费 ⇒ 「已扣已跑」。
    let (_tmp_b, lifecycle_b, mut engine_b, _rx_b, _cycle_b) = compile_failure_fixture(0).await;
    let session_b = engine_b.session().session_id.clone();
    assert!(
        engine_b.route_compile_failure_to_rewrite().await,
        "委托轮必须被通道消费（同一崩溃窗起点）"
    );
    let landed_b = lifecycle_b
        .get_workspace_session(&session_b)
        .expect("load landed record");
    let key_b = format!("single_candidate_author:{}:0", landed_b.id);

    let (consumed, started) = lifecycle_b
        .reserve_single_candidate_provider_start(&landed_b, &key_b)
        .expect("relay consumption must succeed");
    assert!(started, "Reserved 预领必须被接力消费放行");
    assert!(
        consumed
            .repair_reservation
            .as_ref()
            .is_some_and(|reservation| reservation.state == RepairReservationState::ProviderStarted),
        "消费后 reservation 必须推进为 ProviderStarted"
    );

    // 已启动 ⇒ 不退（真跑过）：之后的对账重放不得回退已消费扣减。
    engine_b.reconcile_auto_rewrite_not_delegated().await;
    let kept = lifecycle_b
        .get_workspace_session(&session_b)
        .expect("load kept record");
    let charge_b = kept
        .author_rewrite_budget
        .as_ref()
        .and_then(|budget| {
            budget
                .auto_rewrite_charges
                .iter()
                .find(|charge| charge.provider_start_idempotency_key == key_b)
        })
        .expect("「已扣已跑」扣减保留");
    assert!(
        charge_b.refunded_at.is_none(),
        "分支 B 收敛为「已扣已跑」：已启动的扣减不得回退"
    );
    assert_no_dangling_charge(&lifecycle_b, &session_b);
}

// ============ Task 4（工作包 1.3 读侧 / 1.5 / 2.1 / 2.2 / 2.5）============

/// Task 4 夹具：改写 durable 编译状态的诊断计数（预算域未建 ⇒ 读侧归并
/// 口径 net = used，余额 = total − used）。
fn preset_legacy_auto_rewrites_used(lifecycle: &LifecycleStore, session_id: &str, used: u32) {
    let mut status = lifecycle
        .load_candidate_compile_status(session_id)
        .expect("load fixture compile status")
        .expect("status present");
    status.auto_rewrites_used = used;
    lifecycle
        .save_candidate_compile_status(session_id, &status)
        .expect("persist preset compile status");
}

/// Task 4 夹具：预置轮内 A 池计数（cycle.repairs_used）。
fn preset_cycle_repairs_used(
    lifecycle: &LifecycleStore,
    session_id: &str,
    cycle_key: &str,
    repairs_used: u32,
) {
    let mut record = lifecycle
        .get_workspace_session(session_id)
        .expect("load fixture record");
    record
        .run_history
        .review_cycles
        .entry(cycle_key.to_string())
        .and_modify(|cycle| cycle.repairs_used = repairs_used)
        .or_insert_with(|| crate::product::work_item_plan_policy::ReviewCycleState {
            repairs_used,
            ..Default::default()
        });
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist preset cycle repairs_used");
}

/// durable 读侧组装权威余额（与生产 `author_rewrite_balance` 同口径；
/// 测试侧直接从 record 组装，便于断言 durable 事实本身）。
fn durable_balance(
    record: &crate::product::models::WorkspaceSessionRecord,
    legacy_used: u32,
) -> Option<u32> {
    available_author_rewrites(&AuthorRewriteBudgetInputs {
        total: crate::product::work_item_plan_policy::RunBudgets::default().max_manual_repairs,
        human_gate_snapshot: record.human_gate_snapshot.as_ref(),
        author_rewrite_budget: record.author_rewrite_budget.as_ref(),
        legacy_auto_rewrites_used: legacy_used,
    })
}

/// 工作包 2.1 / 2.5（Task 4）：平台编译失败轮的 author 重跑摘出机械返修
/// 池 A——轮内 `cycle.repairs_used` 与会话观测 `run_history.repairs_used`
/// 均不递增；B 池（author 重写预算域）净扣恰好 1（预领键一条未退扣减）。
/// 对照改前基线：cycle.repairs_used == 1（平台轮挪用 A 池扣次）。
#[tokio::test]
async fn platform_round_consumes_no_a_pool_delta() {
    let (_tmp, lifecycle, mut engine, _event_rx, cycle_key) = compile_failure_fixture(0).await;
    let session_id = engine.session().session_id.clone();
    assert!(
        engine.route_compile_failure_to_rewrite().await,
        "平台轮必须被通道消费（真正委托 author 重跑）"
    );
    let persisted = lifecycle
        .get_workspace_session(&session_id)
        .expect("load persisted session");
    let cycle = persisted
        .run_history
        .review_cycles
        .get(&cycle_key)
        .expect("cycle recorded");
    assert_eq!(
        cycle.repairs_used, 0,
        "A 池轮内计数不递增（平台轮摘出 A 池）"
    );
    assert_eq!(
        cycle.verification_count, 1,
        "复评观测计数保持递增（REQ-WSC-25 可序列观测不回归）"
    );
    assert_eq!(
        persisted.run_history.repairs_used, 0,
        "A 池会话观测计数不递增"
    );
    let budget = persisted
        .author_rewrite_budget
        .as_ref()
        .expect("B 池预领扣减已随委托同 CAS 落盘");
    assert_eq!(
        budget
            .auto_rewrite_charges
            .iter()
            .filter(|charge| charge.refunded_at.is_none())
            .count(),
        1,
        "B 池净扣恰好 1（本轮 author 重跑的预领扣减）"
    );
    // 委托即「已扣待跑」——接力消费预领（ProviderStarted）后收敛为
    // 「已扣已跑」，此时才满足两态不变量（净扣 > 0 必有真跑证据）。
    let key = format!("single_candidate_author:{}:0", persisted.id);
    let (_consumed, started) = lifecycle
        .reserve_single_candidate_provider_start(&persisted, &key)
        .expect("relay consumption must succeed");
    assert!(started, "Reserved 预领必须被接力消费放行（已扣已跑）");
    assert_no_dangling_charge(&lifecycle, &session_id);
}

/// 工作包 2.2 / 2.5（Task 4）：rescue 守卫改读 B 池余额——A 池耗尽
/// （cycle.repairs_used == 1 == max_repairs）不得阻断平台自动重写；B 池
/// 余额 0 才是真耗尽（维持人工门）。
#[tokio::test]
async fn a_pool_exhausted_still_allows_auto_rewrite() {
    // A 池耗尽 + B 余额 Some(1) ⇒ rescue 复原为 Repairable，委托照常。
    let (_tmp, lifecycle, mut engine, _event_rx, cycle_key) = compile_failure_fixture(0).await;
    let session_id = engine.session().session_id.clone();
    preset_cycle_repairs_used(&lifecycle, &session_id, &cycle_key, 1);
    preset_legacy_auto_rewrites_used(&lifecycle, &session_id, 2); // 余额 = 3 − 2 = 1
    assert!(
        engine.route_compile_failure_to_rewrite().await,
        "A 池耗尽的平台轮仍必须被通道消费"
    );
    assert!(
        engine.sc_author_rerun_delegated(),
        "A 池耗尽不得阻断平台自动重写（只要 B 池余额 > 0）——守卫改读 B 池"
    );

    // 对照：B 余额 Some(0) ⇒ rescue 不复原 ⇒ 维持 HumanRequired（真耗尽交门）。
    let (_tmp2, lifecycle2, mut engine2, _event_rx2, _cycle_key2) =
        compile_failure_fixture(0).await;
    let session_id2 = engine2.session().session_id.clone();
    preset_legacy_auto_rewrites_used(&lifecycle2, &session_id2, 3); // 余额 = 3 − 3 = 0
    assert!(
        engine2.route_compile_failure_to_rewrite().await,
        "B 池耗尽的升级交门轮也必须被通道消费（单门语义）"
    );
    assert!(
        !engine2.sc_author_rerun_delegated(),
        "B 池余额 0 ⇒ 真实耗尽结论，维持人工门"
    );
    let persisted2 = lifecycle2
        .get_workspace_session(&session_id2)
        .expect("load gate session");
    assert!(
        persisted2.human_gate_snapshot.is_some(),
        "B 池耗尽 ⇒ 升级交门（EnterHumanGate/StopNeedsHuman 快照在场）"
    );
}

/// 工作包 1.5 / 2.2（Task 4）：B 余额 0 升级交门 ⇒ 新门快照
/// manual_repairs_remaining == 0（统一权威余额，不是重置公式给 3），且同
/// CAS 自动段已结转清零（再读权威余额 == 快照 0，无双计；total_ever 保留）。
#[tokio::test]
async fn escalation_gate_snapshot_carries_unified_balance_and_settles_segment() {
    let (_tmp, lifecycle, mut engine, _event_rx, _cycle_key) = compile_failure_fixture(0).await;
    let session_id = engine.session().session_id.clone();
    preset_legacy_auto_rewrites_used(&lifecycle, &session_id, 3); // 余额 = 3 − 3 = 0
    assert!(
        engine.route_compile_failure_to_rewrite().await,
        "升级交门轮必须被通道消费"
    );
    let persisted = lifecycle
        .get_workspace_session(&session_id)
        .expect("load gated session");
    let snapshot = persisted
        .human_gate_snapshot
        .as_ref()
        .expect("escalation gate snapshot");
    assert_eq!(
        snapshot.manual_repairs_remaining, 0,
        "门快照携带统一权威余额 0（不是重置公式的 3）"
    );
    let budget = persisted
        .author_rewrite_budget
        .as_ref()
        .expect("预算域随门快照同 CAS 结转");
    assert!(
        budget.auto_rewrite_charges.is_empty(),
        "自动段已结转清零（无双计）"
    );
    assert_eq!(
        budget.legacy_auto_rewrites_charged, 0,
        "baseline 冻结后归零（legacy 读侧不再重复归并）"
    );
    assert_eq!(
        budget.total_auto_rewrite_charges_ever, 3,
        "total_ever 保留（REQ-WSC-25 可观测）"
    );
    let legacy_used = lifecycle
        .load_candidate_compile_status(&session_id)
        .expect("load status")
        .expect("present")
        .auto_rewrites_used;
    assert_eq!(
        durable_balance(&persisted, legacy_used),
        Some(0),
        "结转后再读权威余额 == 快照净余额 0（无双计）"
    );
}

// ============ Task 5（工作包 4.1 / 4.2 / 4.3 / 4.4 / 5.5）============

/// Task 5 夹具：预置「门反馈段 + 预算域自动段」的 durable 形态——快照
/// `manual_repairs_remaining = total − accepted_feedback_turns`（反馈段净
/// 余额，gate-local 事实在场 ⇒ 余额可知）；自动段用真实纯函数
/// `with_preclaim_charge` 建 N 条未退扣减（首条建域 legacy=1 ⇒ baseline=0
/// ⇒ 净扣恰好 = 条数，与 `preland_preclaim_charge` 同口径）。
fn preset_feedback_gate_and_auto_charges(
    lifecycle: &LifecycleStore,
    session_id: &str,
    accepted_feedback_turns: u32,
    auto_charges: u32,
) {
    let mut record = lifecycle
        .get_workspace_session(session_id)
        .expect("load fixture record");
    let total = crate::product::work_item_plan_policy::RunBudgets::default().max_manual_repairs;
    record.human_gate_snapshot = Some(crate::product::work_item_plan_policy::HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: accepted_feedback_turns,
        manual_repairs_remaining: total - accepted_feedback_turns,
        accepted_feedback_turns: Some(accepted_feedback_turns),
        candidate_recovery: None,
        trigger: crate::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
        resumable: true,
    });
    for attempt in 0..auto_charges {
        record.author_rewrite_budget = Some(with_preclaim_charge(
            record.author_rewrite_budget.as_ref(),
            &format!("single_candidate_author:{}:{}", record.id, attempt),
            1,
            "2026-11-12T00:00:00Z".to_string(),
        ));
    }
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist gate + auto segment fixture");
}

/// Task 5 夹具：同门再接受一轮 owner 反馈——镜像 `lifecycle_store/
/// human_gate.rs` 的同 CAS 扣减语义（`manual_repairs_remaining −1` 且
/// `accepted_feedback_turns +1` 原子同落），此处直接落 durable 形态。
fn accept_one_more_feedback_turn(lifecycle: &LifecycleStore, session_id: &str) {
    let mut record = lifecycle
        .get_workspace_session(session_id)
        .expect("load fixture record");
    let snapshot = record
        .human_gate_snapshot
        .as_mut()
        .expect("gate snapshot present");
    snapshot.manual_repairs_remaining -= 1;
    snapshot.accepted_feedback_turns = Some(snapshot.accepted_feedback_turns.unwrap_or(0) + 1);
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist accepted feedback turn");
}

/// 工作包 4.1 / 4.2 / 4.3 / 4.4 / 5.5（Task 5）：编译成功只清该候选编译链
/// 的『平台自动重写』段——自动 2 + 反馈 1（快照 remaining 2）⇒ 编译成功 ⇒
/// 权威余额 `Some(2)`（自动段归零、反馈段保留）；同门再反馈一次 ⇒
/// `Some(1)`（carry-forward 未被重置，不回满额）；`total_ever` 保留
/// （REQ-WSC-25 可观测）。红线对照：若把门反馈段一并清零 / 整池回满，
/// 余额会假读 3（REQ-HGC-01 被静默推翻）。
#[tokio::test]
async fn compile_success_clears_auto_segment_only_not_feedback() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, lifecycle, mut engine, _event_rx, _cycle_key) = compile_failure_fixture(0).await;
    let session_id = engine.session().session_id.clone();
    // 幂等边界：预算域未建立（老会话）⇒ clear CAS 幂等 Ok 且不建域。
    lifecycle
        .clear_author_rewrite_auto_segment(&session_id)
        .expect("预算域未建 ⇒ 幂等 Ok");
    assert!(
        lifecycle
            .get_workspace_session(&session_id)
            .expect("reload after idempotent clear")
            .author_rewrite_budget
            .is_none(),
        "None ⇒ 幂等清段不得建域"
    );
    // 自动 2 + 反馈 1：快照 remaining 2、自动段净扣 2 ⇒ 编译前余额 Some(0)。
    preset_feedback_gate_and_auto_charges(&lifecycle, &session_id, 1, 2);
    let before = lifecycle
        .get_workspace_session(&session_id)
        .expect("reload preset record");
    assert_eq!(
        durable_balance(&before, 1),
        Some(0),
        "编译前权威余额 = 快照 2 − 自动段 2（预算域已建 ⇒ legacy 不归并）"
    );
    // 编译成功走真实成功分支（run_work_item_plan_compile ⇒ record_candidate_
    // compile_success：先清段 CAS、再落诊断计数面）。
    engine
        .run_work_item_plan_compile()
        .await
        .expect("干净候选必须编译成功");
    let persisted = lifecycle
        .get_workspace_session(&session_id)
        .expect("load post-success session");
    let budget = persisted
        .author_rewrite_budget
        .as_ref()
        .expect("预算域在场（清段不拆域）");
    assert!(
        budget.auto_rewrite_charges.is_empty(),
        "自动段归零（charges 清空）"
    );
    assert_eq!(
        budget.legacy_auto_rewrites_charged, 0,
        "自动段归零（baseline 归零）"
    );
    assert_eq!(
        budget.total_auto_rewrite_charges_ever, 2,
        "total_ever 保留（REQ-WSC-25 可观测）"
    );
    let snapshot = persisted
        .human_gate_snapshot
        .as_ref()
        .expect("门快照保留（反馈段不清）");
    assert_eq!(
        snapshot.manual_repairs_remaining, 2,
        "反馈段净余额不动（清段只动预算域）"
    );
    assert_eq!(
        snapshot.accepted_feedback_turns,
        Some(1),
        "gate-local 反馈计数不动"
    );
    let legacy_used = lifecycle
        .load_candidate_compile_status(&session_id)
        .expect("load compile status")
        .expect("status present")
        .auto_rewrites_used;
    assert_eq!(legacy_used, 0, "诊断计数随成功归零（cleared 状态）");
    assert_eq!(
        durable_balance(&persisted, legacy_used),
        Some(2),
        "编译成功后权威余额 = 快照 2 − 自动段 0（自动段清、反馈段留）"
    );
    // 同门再反馈一次 ⇒ Some(1)：carry-forward 接续（编译成功不重置门预算）。
    accept_one_more_feedback_turn(&lifecycle, &session_id);
    let after_feedback = lifecycle
        .get_workspace_session(&session_id)
        .expect("reload after feedback turn");
    assert_eq!(
        durable_balance(&after_feedback, 0),
        Some(1),
        "同门再反馈一次 ⇒ Some(1)（carry-forward 未被重置，非回满额 3）"
    );
    // 幂等重放：已清段再 clear ⇒ Ok 且零状态变化。
    lifecycle
        .clear_author_rewrite_auto_segment(&session_id)
        .expect("已清段 ⇒ 幂等 Ok");
    let replayed = lifecycle
        .get_workspace_session(&session_id)
        .expect("reload after replay clear");
    assert_eq!(
        replayed
            .author_rewrite_budget
            .as_ref()
            .expect("budget domain")
            .total_auto_rewrite_charges_ever,
        2,
        "幂等重放不动 total_ever"
    );
    assert_eq!(
        durable_balance(&replayed, 0),
        Some(1),
        "幂等重放不改变权威余额"
    );
}
