//! ① 层容错接入产品读路径（REQ-DRG-24）。
//!
//! 本模块把 ① 层的「**跳过该条目 + 记一条告警**」容错接到**产品里真的读这些
//! 旁支记录、并顺着 id 去查目标**的路径上：
//!
//! 1. **实体身份台账按 owner 反查**（`src/product/lifecycle_store/plan.rs` 的
//!    `entity_owned_session_ids`）：台账绑定指向**已删实体**（L1 墓碑四条全成
//!    立）→ **跳过该绑定**，**不把已删实体的 session 算进待清理集合** + 记一条
//!    ①层告警。台账反查本体（`EntityOperationStore::list_workspace_session_ids_by_owner`）
//!    的 fail-closed 语义**一字不改**——容错加在**调用方**。
//!
//! 2. **reviewer 窗口面**：**不接入**「跳过该条目」语义。启动补偿扫描
//!    （`code_review.rs` 的 `compensate_open_reviewer_windows_at_startup`）是
//!    **收口面**而非投影面：窗口的 attempt 记录已删时，该窗口 MUST 继续进入
//!    补偿收口（否则它永远停在非终态、租约永不释放、③ 层的窗口簇清理因
//!    「非终态」而永不删它）。既有 F12 用例
//!    （`code_review_hygiene_tests.inc.rs` 的 `f12_startup_compensation_cleans_unpublished_residue`
//!    / `..._keeps_published_manifest`）以「attempt 记录不存在的孤儿窗口」为
//!    夹具，断言它**必须**被补偿——跳过会打破该不变量。attempt 依赖的补偿子
//!    步骤沿用既有优雅 no-op（`project_window_manual_attention` 记日志、
//!    `resume_pending_review_compensation` 早退），**不抛错、不使整批失败**。
//!
//! 3. **自动化 intent / reviewer 快照面**：产品侧无「读旁支记录并顺着 id 去
//!    查目标」的路径（intent 只按固定文件名单条读、只比较**身份**；快照按
//!    `window_id` 单条读、无枚举 API）→ **无容错可接**，维持既有语义。
//!
//! 边界（与 REQ-DRG-05 / 06 / 23 一致，MUST NOT 放宽）：
//!
//! - 只跳过「**已确认目标不存在**」：读失败（IO / 解析 / 权限）照旧上抛，
//!   MUST NOT 被当成「目标已删」；
//! - 存在性判为「不可判定」（`Unknown`）的引用**既不跳过也不告警**
//!   （REQ-DRG-23）；
//! - 主记录缺失语义不变（`story_spec_not_found` 等既有错误码与文案一字不动，
//!   REQ-DRG-04）；
//! - fail-closed 读路径（N5–N12）保持不变。

use std::collections::BTreeSet;

use crate::product::app_paths::ProductAppPaths;
use crate::product::dangling_reference::listing::{
    entity_kind_target_kind, field_path_for, record_path_of,
};
use crate::product::dangling_reference::reference_table::{
    DanglingRecordType, DanglingTargetKind, reference_rule,
};
use crate::product::dangling_reference::resolve::{DanglingResolver, EvidenceLevel};
use crate::product::dangling_reference::{
    DANGLING_REFERENCE_WARNING_CODE, DanglingReferenceWarning, DanglingReferenceWarnings,
};
use crate::product::entity_deletion_store::EntityScope;
use crate::product::json_store::ProductStoreError;

/// 被跳过条目的处置动作（告警 `action` 列取值，与①层列举面同口径）。
const ACTION_SKIPPED: &str = "skipped";

/// 实体身份台账反查的容错接入（REQ-DRG-24）。
///
/// 输入 = 台账按 owner 反查得到的 session id 集合（`EntityOperationStore`
/// 的 fail-closed 反查本体**不改**）；输出 = 跳过「绑定指向已删实体」的条目后
/// 的集合 + 逐条①层告警。判据：绑定实体的**已删证据为 L1**（墓碑存在 +
/// `phase == deleted` + `deleted_at` 有值 + 无待人工处置的删除失败事实，
/// REQ-DRG-02）——只有**已确认已删**才跳过；删除进行中（`purging`）/ 缺完成
/// 时间 / 带 pending failure 一律**不跳过**（证据不足 ≠ 已删）。
///
/// 读失败（IO / 解析 / 权限）照旧上抛，MUST NOT 被当成「已删」。
pub(crate) fn live_ledger_session_ids(
    paths: &ProductAppPaths,
    scope: &EntityScope,
    session_ids: BTreeSet<String>,
) -> Result<(BTreeSet<String>, DanglingReferenceWarnings), ProductStoreError> {
    if session_ids.is_empty() {
        return Ok((session_ids, DanglingReferenceWarnings::default()));
    }
    let Some(target_kind) = ledger_target_kind(scope) else {
        return Ok((session_ids, DanglingReferenceWarnings::default()));
    };
    // 已删证据必须为 L1；证据不足（purging / 缺完成时间 / 带失败事实）→ 不跳过。
    let empty_session_ids: BTreeSet<String> = BTreeSet::new();
    let resolver = DanglingResolver::new(paths, &empty_session_ids);
    if resolver.evidence_level(target_kind, &scope.entity_id, Some(scope))? != EvidenceLevel::L1 {
        return Ok((session_ids, DanglingReferenceWarnings::default()));
    }
    let rule = reference_rule(DanglingRecordType::EntityIdentitySessionBinding)
        .expect("判据表必须登记实体身份台账");
    let field_path = field_path_for(rule, target_kind);
    let mut warnings = DanglingReferenceWarnings::default();
    for session_id in &session_ids {
        warnings.push(DanglingReferenceWarning {
            code: DANGLING_REFERENCE_WARNING_CODE.to_string(),
            record_path: ledger_record_path(paths, session_id),
            record_type: DanglingRecordType::EntityIdentitySessionBinding
                .as_str()
                .to_string(),
            reference_field: field_path.to_string(),
            target_kind: target_kind.as_str().to_string(),
            target_id: scope.entity_id.clone(),
            action: ACTION_SKIPPED.to_string(),
        });
    }
    // 告警落盘：产品读路径没有可挂告警的 store → 用既有 tracing::warn! 带
    // 结构化字段（不新造告警体系）。
    for warning in warnings.items() {
        log_warning(warning);
    }
    Ok((BTreeSet::new(), warnings))
}

/// ①层告警落盘（结构化字段；REQ-DRG-07 的七个字段齐备）。
fn log_warning(warning: &DanglingReferenceWarning) {
    tracing::warn!(
        code = %warning.code,
        record_path = %warning.record_path,
        record_type = %warning.record_type,
        reference_field = %warning.reference_field,
        target_kind = %warning.target_kind,
        target_id = %warning.target_id,
        action = %warning.action,
        "悬空引用已跳过：目标已确认不存在"
    );
}

/// 台账绑定实体 → 目标类型（`story` / `design` / `plan`；表外值不猜引用语义）。
fn ledger_target_kind(scope: &EntityScope) -> Option<DanglingTargetKind> {
    entity_kind_target_kind(scope.kind.path_segment())
}

/// 台账条目路径（相对应用状态根）。
fn ledger_record_path(paths: &ProductAppPaths, session_id: &str) -> String {
    record_path_of(
        paths,
        &paths
            .state_root()
            .join("entity-identity")
            .join("sessions")
            .join(format!("{session_id}.json")),
    )
}

#[cfg(test)]
#[path = "tolerance_tests.rs"]
mod tolerance_tests;
