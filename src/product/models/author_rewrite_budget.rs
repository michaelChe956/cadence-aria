//! author 重写预算域（REQ-WSC-21 共池执法）——单一权威余额的数学面。
//!
//! # 核心恒等式
//!
//! **权威余额 ≡ 门快照净余额 − 预算域自动段净扣减**
//!
//! - **门快照净余额**：快照在场 ⇒ `manual_repairs_remaining`；无快照 ⇒ 总额
//!   （`RunBudgets::default().max_manual_repairs`，唯一来源）。反馈段的
//!   预留 / 结算 / 退款 / 显式继续已全部体现在 `manual_repairs_remaining`
//!   的净变动里（每个 reserve 恰好 −1、交付不重复扣、退款回补，见
//!   `lifecycle_store/human_gate.rs` 的同 CAS 扣减与
//!   `models/workspace.rs` 的 `settle_turn_receipt` 回补），故不镜像双记。
//! - **自动段净扣减** = [`net_auto_charged`]：预算域未建 ⇒ 读侧归并 legacy
//!   B1 计数；已建 ⇒ 只算账本（legacy 建域时已冻进 baseline，不双计）。
//! - 门快照在场但 `accepted_feedback_turns == None` ⇒ 余额未知（返回
//!   `None`，保守不补满额，与 `work_item_plan_policy/types.rs` 既有
//!   「预算历史不可用」口径一致）。
//! - **门快照建立 / 重建**：快照 `manual_repairs_remaining` := 权威余额，
//!   且**同 CAS 将自动段结转清零**（[`clear_auto_segment`]：消费事实沉入
//!   快照、账本清段、`total_ever` 保留）——否则后续读会双计自动扣减。
//!
//! # 事实来源清单（「谁在读 / 写余额」盘点，2026-11-12 冻结）
//!
//! 盘点命令产物：`/tmp/quota_pool_read_write_inventory.txt`（同步进 Task 6
//! 验收记录）。要点（file:line 以盘点时 HEAD 为准）：
//!
//! - **执法读点**（B1 现状余额来源；本 change 后 MUST 改读本模块单一权威
//!   余额）：
//!   - `workspace_engine/compile_failure_surface.rs:133-135`
//!     （`max_auto_rewrites` = `RunBudgets::default().max_manual_repairs`）；
//!   - `workspace_engine/compile_failure_surface.rs:144-153`（`decide_route`
//!     读 `auto_rewrites_used` 判余额）；
//!   - `workspace_engine/compile_failure_surface.rs:292-313`
//!     （`candidate_compile_status` 派生失败事实时 `used+1` 扣减）；
//!   - `workspace_engine/compile_failure_surface.rs:353-368`
//!     （`cleared_candidate_compile_status` 编译成功清零
//!     `auto_rewrites_used: 0`）。
//! - **门快照构造点**（快照净余额写入方，建立/重建时须按上文恒等式结转）：
//!   - `workspace_engine/review/routing.rs:601-609`（SC 改道臂建快照）；
//!   - `workspace_engine/review/policy_routing.rs:96-106,128`
//!     （`route_outcome` 建快照并回填 durable 净余额）；
//!   - `workspace_engine/compile.rs:1104-1112`（失败门补建快照）；
//!   - `workspace_engine/review/routing_scope.rs:885-928`（durable 恢复重建）。
//! - **反馈扣减点**（B2 段同 CAS 写，语义不动）：
//!   - `lifecycle_store/human_gate.rs:181,195`（`manual_repairs_remaining
//!     -= 1`，同 CAS `accepted_feedback_turns += 1`）。
//! - **A 池执法点**（机械返修池；本 change 后不得再被平台自动重写挪用）：
//!   - `workspace_engine/review/routing.rs:628`（`repairs_used_delta: 1`）；
//!   - `workspace_engine/review/routing_platform_rescue.rs:124`
//!     （守卫 `cycle.repairs_used >= RunBudgets::default().max_repairs`）。
//! - **读侧余额呈现 / 上限核对**：`workspace_engine/conversational_gate.rs:530`、
//!   `workspace_engine/conversational_gate_recovery.rs:722`、
//!   `lifecycle_store/workspace_single_candidate.rs:301-304`、
//!   `models/workspace.rs:561-565`（结构退款回补净余额）、
//!   `work_item_plan_policy/evaluate.rs:466`、`session_state.rs:841`、
//!   `routing_scope.rs:914`。
//!
//! 老会话兼容（决策 6）：预算字段 `#[serde(default)]` + 不序列化 `None`；
//! 缺新事实 ⇒ 「余额未知」保守处理，不补满额、不改写历史 durable 文件。

use serde::{Deserialize, Serialize};

/// 自动重写的一次预领扣减（按 provider-start 幂等键去重、可回退留痕）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuthorRewriteCharge {
    /// 幂等键 = 预领的 provider-start 键
    pub provider_start_idempotency_key: String,
    /// RFC3339
    pub charged_at: String,
    /// 对账回退留痕（幂等：只写一次）
    pub refunded_at: Option<String>,
}

/// author 重写预算域的 durable 状态（挂在 session 记录上，与门反馈同 CAS）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuthorRewriteBudgetState {
    /// 建域（首次扣减 CAS）时一次性归并的遗留 B1 计数（老会话读侧事实，不改历史文件）
    pub legacy_auto_rewrites_charged: u32,
    /// 未结转/未清零的自动重写扣减（按 provider_start_idempotency_key 去重）
    pub auto_rewrite_charges: Vec<AuthorRewriteCharge>,
    /// 历史累计扣减（编译成功/门结转都不清零；REQ-WSC-25 可观测）
    pub total_auto_rewrite_charges_ever: u32,
}

/// 余额输入（从 durable session 记录读侧组装，不写任何文件）
pub struct AuthorRewriteBudgetInputs<'a> {
    /// 调用方传 RunBudgets::default().max_manual_repairs
    pub total: u32,
    pub human_gate_snapshot: Option<&'a crate::product::work_item_plan_policy::HumanGateSnapshot>,
    pub author_rewrite_budget: Option<&'a AuthorRewriteBudgetState>,
    /// CandidateCompileStatus.auto_rewrites_used（读侧）
    pub legacy_auto_rewrites_used: u32,
}

/// 单一权威余额（纯函数）：
/// - `None` = 余额未知（门快照在场但 `accepted_feedback_turns == None` ⇒
///   缺 gate-local 事实，保守不补满额，与 types.rs 既有「预算历史不可用」
///   口径一致）；
/// - `Some(n)` = 快照净余额（无快照 ⇒ total）saturating_sub(净自动扣减)。
pub fn available_author_rewrites(inputs: &AuthorRewriteBudgetInputs<'_>) -> Option<u32> {
    let snapshot_net = match inputs.human_gate_snapshot {
        Some(snapshot) => match snapshot.accepted_feedback_turns {
            None => return None,
            Some(_) => snapshot.manual_repairs_remaining,
        },
        None => inputs.total,
    };
    let net_auto = net_auto_charged(
        inputs.author_rewrite_budget,
        inputs.legacy_auto_rewrites_used,
    );
    Some(snapshot_net.saturating_sub(net_auto))
}

/// 自动段净扣减：预算域未建立（None）⇒ 读侧归并 legacy 计数；
/// 已建立 ⇒ 只算账本（baseline 已在建域时并入，不双计）。
pub fn net_auto_charged(
    state: Option<&AuthorRewriteBudgetState>,
    legacy_auto_rewrites_used: u32,
) -> u32 {
    match state {
        None => legacy_auto_rewrites_used,
        Some(state) => state.legacy_auto_rewrites_charged.saturating_add(
            state
                .auto_rewrite_charges
                .iter()
                .filter(|charge| charge.refunded_at.is_none())
                .count() as u32,
        ),
    }
}

/// 预领扣减（纯状态转移，落盘归同 CAS 调用方）：
/// 幂等：键已存在 ⇒ 原样返回（不重复扣）；建域时把 legacy 计数冻进
/// baseline；`total_ever` 只增。
pub fn with_preclaim_charge(
    state: Option<&AuthorRewriteBudgetState>,
    provider_start_idempotency_key: &str,
    legacy_auto_rewrites_used: u32,
    charged_at: String,
) -> AuthorRewriteBudgetState {
    let charge = AuthorRewriteCharge {
        provider_start_idempotency_key: provider_start_idempotency_key.to_string(),
        charged_at,
        refunded_at: None,
    };
    match state {
        None => AuthorRewriteBudgetState {
            legacy_auto_rewrites_charged: legacy_auto_rewrites_used,
            auto_rewrite_charges: vec![charge],
            total_auto_rewrite_charges_ever: 1,
        },
        Some(state) => {
            if state.auto_rewrite_charges.iter().any(|existing| {
                existing.provider_start_idempotency_key == charge.provider_start_idempotency_key
            }) {
                return state.clone();
            }
            let mut next = state.clone();
            next.auto_rewrite_charges.push(charge);
            next.total_auto_rewrite_charges_ever =
                next.total_auto_rewrite_charges_ever.saturating_add(1);
            next
        }
    }
}

/// 对账回退：幂等——已退（或键不存在）⇒ false；退成 ⇒ true
/// （账面 −1，total_ever 不减、留痕不删）。
pub fn refund_charge(
    state: &mut AuthorRewriteBudgetState,
    provider_start_idempotency_key: &str,
    refunded_at: String,
) -> bool {
    let Some(charge) = state.auto_rewrite_charges.iter_mut().find(|charge| {
        charge.provider_start_idempotency_key == provider_start_idempotency_key
            && charge.refunded_at.is_none()
    }) else {
        return false;
    };
    charge.refunded_at = Some(refunded_at);
    true
}

/// 编译成功清零 / 门快照结转共用：charges 清空、baseline 归零、
/// total_ever 保留。
pub fn clear_auto_segment(state: &mut AuthorRewriteBudgetState) {
    state.auto_rewrite_charges.clear();
    state.legacy_auto_rewrites_charged = 0;
}

/// 门快照建立/重建的结转（Task 4）：快照净余额 := 权威余额后，自动段消费
/// 事实已沉入快照 ⇒ 预算域同 CAS 清段（charges 清空、baseline 归零、
/// `total_ever` 保留）。预算域未建（老会话）时先把 legacy 读侧计数冻进
/// baseline 再清零——否则快照建立后的读侧会再次归并 legacy 计数（双计）。
pub fn carried_forward_budget_state(
    state: Option<&AuthorRewriteBudgetState>,
    legacy_auto_rewrites_used: u32,
) -> AuthorRewriteBudgetState {
    let mut carried = state.cloned().unwrap_or_else(|| AuthorRewriteBudgetState {
        legacy_auto_rewrites_charged: legacy_auto_rewrites_used,
        auto_rewrite_charges: Vec::new(),
        total_auto_rewrite_charges_ever: legacy_auto_rewrites_used,
    });
    clear_auto_segment(&mut carried);
    carried
}

#[cfg(test)]
mod tests {
    use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason};

    use super::{
        AuthorRewriteBudgetInputs, AuthorRewriteBudgetState, AuthorRewriteCharge,
        available_author_rewrites, carried_forward_budget_state, clear_auto_segment,
        net_auto_charged, refund_charge, with_preclaim_charge,
    };

    /// 最小门快照夹具：净余额与 gate-local 事实（accepted_feedback_turns）显式传入。
    fn gate_snapshot(
        manual_repairs_remaining: u32,
        accepted_feedback_turns: Option<u32>,
    ) -> HumanGateSnapshot {
        HumanGateSnapshot {
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            attempts_used: 0,
            manual_repairs_remaining,
            accepted_feedback_turns,
            candidate_recovery: None,
            trigger: HumanReason::NativeHumanRequired,
            resumable: true,
        }
    }

    /// 一条未回退的自动重写扣减（时间戳仅留痕，不参与运算）。
    fn charge(key: &str) -> AuthorRewriteCharge {
        AuthorRewriteCharge {
            provider_start_idempotency_key: key.to_string(),
            charged_at: "2026-11-12T00:00:00Z".to_string(),
            refunded_at: None,
        }
    }

    #[test]
    fn balance_pools_auto_and_feedback_into_one_sequence() {
        // 反馈净余额 2（3 − 1 次反馈）、自动净扣 1 ⇒ Some(1)。不是 2、不是各自 2。
        let state = AuthorRewriteBudgetState {
            auto_rewrite_charges: vec![charge("provider_start_0001")],
            ..Default::default()
        };
        let inputs = AuthorRewriteBudgetInputs {
            total: 3,
            human_gate_snapshot: Some(&gate_snapshot(2, Some(1))),
            author_rewrite_budget: Some(&state),
            legacy_auto_rewrites_used: 0,
        };
        // 核心恒等式：快照净余额 2 − 自动净扣 1 = 1（同一个池、同一条扣减序列）。
        assert_eq!(available_author_rewrites(&inputs), Some(1));
    }

    #[test]
    fn balance_unknown_when_gate_snapshot_lacks_gate_local_facts() {
        // snapshot.accepted_feedback_turns == None ⇒ None（保守，不补满额 3）
        let state = AuthorRewriteBudgetState::default();
        let inputs = AuthorRewriteBudgetInputs {
            total: 3,
            human_gate_snapshot: Some(&gate_snapshot(3, None)),
            author_rewrite_budget: Some(&state),
            legacy_auto_rewrites_used: 0,
        };
        // 即使净余额看似满额 3、预算域无扣减，缺 gate-local 事实 ⇒ 余额未知。
        assert_eq!(available_author_rewrites(&inputs), None);
    }

    #[test]
    fn no_snapshot_means_full_pool_minus_auto() {
        // None 快照 + 自动 1 ⇒ Some(2)
        let state = AuthorRewriteBudgetState {
            auto_rewrite_charges: vec![charge("provider_start_0001")],
            ..Default::default()
        };
        let inputs = AuthorRewriteBudgetInputs {
            total: 3,
            human_gate_snapshot: None,
            author_rewrite_budget: Some(&state),
            legacy_auto_rewrites_used: 0,
        };
        assert_eq!(available_author_rewrites(&inputs), Some(2));
    }

    #[test]
    fn legacy_b1_merged_read_side_only_until_domain_established() {
        // 预算域 None ⇒ net = legacy(2)；建域后再读 ⇒ net 只算账本（不双计）
        assert_eq!(net_auto_charged(None, 2), 2);
        let established = with_preclaim_charge(
            None,
            "provider_start_0001",
            2,
            "2026-11-12T00:00:00Z".to_string(),
        );
        // 建域时 legacy 冻进 baseline，而不是丢掉。
        assert_eq!(established.legacy_auto_rewrites_charged, 2);
        // 只算账本：baseline(2) + 活动扣减(1) = 3；双计会给出 2+2+1=5。
        assert_eq!(net_auto_charged(Some(&established), 2), 3);
    }

    #[test]
    fn preclaim_charge_is_idempotent_by_key() {
        // 同键两次 ⇒ charges.len()==1、total_ever==1
        let first = with_preclaim_charge(
            None,
            "provider_start_0001",
            0,
            "2026-11-12T00:00:00Z".to_string(),
        );
        let replay = with_preclaim_charge(
            Some(&first),
            "provider_start_0001",
            0,
            "2026-11-12T00:01:00Z".to_string(),
        );
        assert_eq!(replay.auto_rewrite_charges.len(), 1);
        assert_eq!(replay.total_auto_rewrite_charges_ever, 1);
        // 键已存在 ⇒ 原样返回（charged_at 也不被重放覆盖）。
        assert_eq!(replay, first);
    }

    #[test]
    fn refund_charge_is_idempotent() {
        // 第二次退 ⇒ false，账面只退一次
        let mut state = with_preclaim_charge(
            None,
            "provider_start_0001",
            0,
            "2026-11-12T00:00:00Z".to_string(),
        );
        assert_eq!(net_auto_charged(Some(&state), 0), 1);
        assert!(refund_charge(
            &mut state,
            "provider_start_0001",
            "2026-11-12T00:02:00Z".to_string()
        ));
        // 重放退款：幂等失败，不二次回补。
        assert!(!refund_charge(
            &mut state,
            "provider_start_0001",
            "2026-11-12T00:03:00Z".to_string()
        ));
        assert_eq!(net_auto_charged(Some(&state), 0), 0);
        // total_ever 不减、留痕不删（REQ-WSC-25 可观测）。
        assert_eq!(state.total_auto_rewrite_charges_ever, 1);
        assert_eq!(state.auto_rewrite_charges.len(), 1);
    }

    #[test]
    fn clear_auto_segment_keeps_total_ever_and_zeroes_baseline() {
        // 清段后 net==0、total_ever 保留
        let state = with_preclaim_charge(
            None,
            "provider_start_0001",
            2,
            "2026-11-12T00:00:00Z".to_string(),
        );
        let mut state = with_preclaim_charge(
            Some(&state),
            "provider_start_0002",
            0,
            "2026-11-12T00:01:00Z".to_string(),
        );
        assert_eq!(state.total_auto_rewrite_charges_ever, 2);
        clear_auto_segment(&mut state);
        assert!(state.auto_rewrite_charges.is_empty());
        assert_eq!(state.legacy_auto_rewrites_charged, 0);
        assert_eq!(net_auto_charged(Some(&state), 0), 0);
        // 历史累计不清零：编译成功 / 门结转后仍可观测总扣减次数。
        assert_eq!(state.total_auto_rewrite_charges_ever, 2);
    }

    #[test]
    fn carried_forward_state_freezes_legacy_then_clears_segment() {
        // 预算域未建（老会话）：legacy 冻进 baseline 后清零、total_ever 保留
        // ——快照建立后读侧净扣 0（不再重复归并 legacy，无双计）。
        let carried = carried_forward_budget_state(None, 2);
        assert_eq!(carried.legacy_auto_rewrites_charged, 0);
        assert!(carried.auto_rewrite_charges.is_empty());
        assert_eq!(carried.total_auto_rewrite_charges_ever, 2);
        assert_eq!(net_auto_charged(Some(&carried), 2), 0);
        // 预算域已建：消费事实沉入快照 ⇒ 清段（幂等），total_ever 不减。
        let charged = with_preclaim_charge(None, "key", 1, "t".to_string());
        let carried = carried_forward_budget_state(Some(&charged), 1);
        assert_eq!(net_auto_charged(Some(&carried), 2), 0);
        assert_eq!(
            carried.total_auto_rewrite_charges_ever,
            charged.total_auto_rewrite_charges_ever
        );
    }
}
