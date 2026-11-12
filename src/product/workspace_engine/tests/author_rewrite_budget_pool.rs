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
use crate::product::models::author_rewrite_budget::{net_auto_charged, with_preclaim_charge};
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
