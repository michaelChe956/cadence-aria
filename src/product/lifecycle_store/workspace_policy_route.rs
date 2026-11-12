//! policy route / policy failure 的 durable 持久化(从 `workspace.rs` 拆出,
//! 保持文件行数守卫)。两者共用同一 policy_diagnostics 落盘机制。

use chrono::Utc;

use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{WorkspaceSessionRecord, WorkspaceSessionStatus};

use super::LifecycleStore;

pub struct PolicyRoutePersist {
    pub status: WorkspaceSessionStatus,
    /// SingleCandidate 由同一 CAS 与策略结果一起推进；legacy 始终保持 `None`。
    pub single_candidate_phase: Option<crate::product::models::SingleCandidatePhase>,
    pub run_history: crate::product::work_item_plan_policy::RunHistory,
    pub scope: Option<crate::product::work_item_plan_policy::ReviewInvocationScope>,
    pub gate: Option<crate::product::work_item_plan_policy::HumanGateSnapshot>,
    pub diagnostics: Vec<crate::product::work_item_plan_policy::PolicyDiagnostic>,
    pub repair_reservation: Option<crate::product::work_item_plan_policy::RepairReservation>,
    pub provider_start_ledger: Vec<crate::product::work_item_plan_policy::ProviderStartLedgerEntry>,
    /// author 重写预算域（Task 2）：与预领 / ledger 同 CAS 落盘——扣减键即预领的
    /// provider-start 幂等键，重放不重复扣。
    pub author_rewrite_budget:
        Option<crate::product::models::author_rewrite_budget::AuthorRewriteBudgetState>,
}

impl LifecycleStore {
    /// Atomically projects a policy failure terminal state (status=Failed +
    /// diagnostics) into the durable session record. `finish_policy_failure`
    /// previously went through `update_workspace_session_status`, which persists
    /// only the status — fail-closed AbortFatal diagnostics stayed memory-only.
    /// This keeps the same durable diagnostics mechanism as
    /// `compare_and_save_policy_route` (I-1 round3 F-C).
    pub fn fail_workspace_session_policy(
        &self,
        session_id: &str,
        diagnostics: &[crate::product::work_item_plan_policy::PolicyDiagnostic],
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        // D1.3b（fix-cascade-deletion plan §0.4）：最外层 scope 准入——在
        // session flock 外先解析归属再取 scope；锁内失败投影零变化。
        let (session_path, scope) = self.resolve_session_scope(session_id)?;
        let locked_session_path = session_path.clone();
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            with_exclusive_lock(&session_path, move || {
                let mut session: WorkspaceSessionRecord = read_json(&locked_session_path)?;
                session.status = WorkspaceSessionStatus::Failed;
                session.human_gate_snapshot = None;
                session.policy_diagnostics = diagnostics.to_vec();
                session.updated_at = Utc::now().to_rfc3339();
                write_json(&locked_session_path, &session)?;
                Ok(session)
            })
        })
    }

    /// Atomically projects a policy route into the durable session record.
    /// The expected record check prevents stale websocket workers from
    /// overwriting a newer route; callers must reload and re-evaluate on clash.
    pub fn compare_and_save_policy_route(
        &self,
        expected: &WorkspaceSessionRecord,
        persist: PolicyRoutePersist,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        // D1.3b（fix-cascade-deletion plan §0.4）：最外层 scope 准入——expected
        // 记录域在 session flock 外先取 scope；锁内 route CAS 零变化。
        let entity_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(entity_scope.as_ref(), move || {
            self.compare_and_save_policy_route_in_admission(expected, persist)
        })
    }

    /// `compare_and_save_policy_route` 的已准入内层（D1.3b：外层已持 scope
    /// 共享准入/已按删除事实拒；session flock 内原 CAS 语义不变）。
    fn compare_and_save_policy_route_in_admission(
        &self,
        expected: &WorkspaceSessionRecord,
        persist: PolicyRoutePersist,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let PolicyRoutePersist {
            status,
            single_candidate_phase,
            run_history,
            scope,
            gate,
            diagnostics,
            repair_reservation,
            provider_start_ledger,
            author_rewrite_budget,
        } = persist;
        validate_relative_id(&expected.id)?;
        let session_path = self.find_workspace_session_path(&expected.id)?;
        let locked_session_path = session_path.clone();
        with_exclusive_lock(&session_path, move || {
            let mut stored: WorkspaceSessionRecord = read_json(&locked_session_path)?;
            if stored != *expected {
                return Err(ProductStoreError::Conflict {
                    kind: "workspace_session",
                    id: expected.id.clone(),
                });
            }
            stored.status = status;
            if stored.flow_kind
                == crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
            {
                stored.single_candidate_phase = single_candidate_phase;
            }
            stored.run_history = run_history;
            stored.review_invocation_scope = scope;
            stored.human_gate_snapshot = gate;
            stored.policy_diagnostics = diagnostics;
            stored.repair_reservation = repair_reservation;
            stored.provider_start_ledger = provider_start_ledger;
            // Task 2：预算域扣减与预领 / ledger 同 CAS 落盘（expected 全量比对
            // 不动——并发写在此冲突）。
            stored.author_rewrite_budget = author_rewrite_budget;
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(&locked_session_path, &stored)?;
            Ok(stored)
        })
    }

    /// 对账回退（Task 3，工作包 3.3/3.4）：把指定 provider-start 键的自动
    /// 重写扣减幂等退款（`refunded_at` 只写一次）。算法在
    /// `models::author_rewrite_budget::refund_charge`（L1 纯函数层），此处
    /// 只做 durable 读改写——与门反馈同一条 session flock（粒度 = 单
    /// session 记录，同 `compare_and_save_policy_route` 的 CAS 域），不改写
    /// 历史事实（total_ever 与 refunded 留痕保留）。预算域为 None / 键不
    /// 存在 / 已退 ⇒ 无事可退，幂等成功（不动 `updated_at`）。
    pub fn refund_author_rewrite_charge(
        &self,
        session_id: &str,
        provider_start_idempotency_key: &str,
        refunded_at: String,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(session_id)?;
        // D1.3b（fix-cascade-deletion plan §0.4）：最外层 scope 准入——在
        // session flock 外先解析归属再取 scope；锁内回退零变化。
        let (session_path, scope) = self.resolve_session_scope(session_id)?;
        let locked_session_path = session_path.clone();
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            with_exclusive_lock(&session_path, move || {
                let mut session: WorkspaceSessionRecord = read_json(&locked_session_path)?;
                let Some(budget) = session.author_rewrite_budget.as_mut() else {
                    // 预算域未建立（老会话）⇒ 无事可退。
                    return Ok(());
                };
                if !crate::product::models::author_rewrite_budget::refund_charge(
                    budget,
                    provider_start_idempotency_key,
                    refunded_at,
                ) {
                    // 已退 / 键不存在 ⇒ 幂等：不重复写、不动 updated_at。
                    return Ok(());
                }
                session.updated_at = Utc::now().to_rfc3339();
                write_json(&locked_session_path, &session)?;
                Ok(())
            })
        })
    }

    /// 编译成功：清零预算域自动段（charges 清空、baseline 归零、total_ever
    /// 保留）。门反馈段（快照 manual_repairs_remaining /
    /// accepted_feedback_turns）一律不动——本 CAS 只写 `author_rewrite_budget`
    /// 一个字段，反馈段事实沉在门快照里（REQ-HGC-01/REQ-CG-02 接续语义）。
    /// 幂等：预算域未建（None）或已清（charges 空且 baseline==0）⇒ Ok、
    /// 不建域、不动 updated_at。
    pub fn clear_author_rewrite_auto_segment(
        &self,
        session_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(session_id)?;
        // D1.3b（fix-cascade-deletion plan §0.4）：最外层 scope 准入——在
        // session flock 外先解析归属再取 scope；锁内清段零变化。
        let (session_path, scope) = self.resolve_session_scope(session_id)?;
        let locked_session_path = session_path.clone();
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            with_exclusive_lock(&session_path, move || {
                let mut session: WorkspaceSessionRecord = read_json(&locked_session_path)?;
                let Some(budget) = session.author_rewrite_budget.as_mut() else {
                    // 预算域未建立（老会话）⇒ 无可清段，幂等成功（不建域、不写盘）。
                    return Ok(());
                };
                if budget.auto_rewrite_charges.is_empty()
                    && budget.legacy_auto_rewrites_charged == 0
                {
                    // 已清段重放 ⇒ 幂等：不重复写、不动 updated_at。
                    return Ok(());
                }
                crate::product::models::author_rewrite_budget::clear_auto_segment(budget);
                session.updated_at = Utc::now().to_rfc3339();
                write_json(&locked_session_path, &session)?;
                Ok(())
            })
        })
    }
}

/// 读侧 legacy B1 自动重写计数（`auto_rewrites_used`）：读不到 ⇒ 0
/// （老会话 / 从未跑过 SC compile 的情形）。
pub fn legacy_b1_used(store: Option<&LifecycleStore>, session_id: &str) -> u32 {
    store
        .and_then(|store| {
            store
                .load_candidate_compile_status(session_id)
                .ok()
                .flatten()
        })
        .map_or(0, |status| status.auto_rewrites_used)
}

/// 预领扣减（Task 2）：按预领的 provider-start 幂等键扣减 author 重写预算域
/// ——算法在 `models::author_rewrite_budget`，此处只做参数组装与时间戳。
pub fn preclaim_budget(
    state: &Option<crate::product::models::author_rewrite_budget::AuthorRewriteBudgetState>,
    reservation: &crate::product::work_item_plan_policy::RepairReservation,
    legacy_auto_rewrites_used: u32,
) -> crate::product::models::author_rewrite_budget::AuthorRewriteBudgetState {
    crate::product::models::author_rewrite_budget::with_preclaim_charge(
        state.as_ref(),
        &reservation.provider_start_idempotency_key,
        legacy_auto_rewrites_used,
        Utc::now().to_rfc3339(),
    )
}

/// Task 4（工作包 1.5）：author 重写**单一权威余额**的 durable 读侧组装——
/// 全部执法点（`decide_route` 供参 / rescue 守卫 / 门快照构造）MUST 经此同
/// 一事实源，不得一处读诊断计数、一处读预算域。输入全部取自 durable
/// session 记录 + durable 编译状态读侧（不读 engine 内存镜像：reconcile
/// 退款只写 durable、不刷镜像，读镜像会少算可用额度——Task 3 评审 Minor-3）。
pub fn author_rewrite_balance(
    store: &LifecycleStore,
    record: &WorkspaceSessionRecord,
    total: u32,
) -> Option<u32> {
    author_rewrite_balance_from_record(record, legacy_b1_used(Some(store), &record.id), total)
}

/// Task 4 评审 Minor-4：余额组装的**单一入口**——从「记录 + 遗留计数」组装
/// `AuthorRewriteBudgetInputs`。`author_rewrite_balance` 与
/// `platform_gate_carried_budget` 共用本函数 ⇒ 仓内**不再存在第二份**余额
/// 算术副本（此前 `platform_gate_carried_budget` 内联重组装，有漂移风险）。
pub fn author_rewrite_balance_from_record(
    record: &WorkspaceSessionRecord,
    legacy_auto_rewrites_used: u32,
    total: u32,
) -> Option<u32> {
    crate::product::models::author_rewrite_budget::available_author_rewrites(
        &crate::product::models::author_rewrite_budget::AuthorRewriteBudgetInputs {
            total,
            human_gate_snapshot: record.human_gate_snapshot.as_ref(),
            author_rewrite_budget: record.author_rewrite_budget.as_ref(),
            legacy_auto_rewrites_used,
        },
    )
}

/// Task 4：门快照建立/重建（快照净余额 := 权威余额）时的预算域结转——
/// 自动段消费事实沉入快照 ⇒ 同 CAS 清段防后续读双计（老会话先冻 legacy
/// 计数再清）。算法在 `models::author_rewrite_budget`，此处只做 durable 读侧组装。
pub fn carried_author_rewrite_budget(
    store: &LifecycleStore,
    record: &WorkspaceSessionRecord,
) -> Option<crate::product::models::author_rewrite_budget::AuthorRewriteBudgetState> {
    Some(
        crate::product::models::author_rewrite_budget::carried_forward_budget_state(
            record.author_rewrite_budget.as_ref(),
            legacy_b1_used(Some(store), &record.id),
        ),
    )
}
