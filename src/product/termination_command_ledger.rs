//! 终止命令的账本身份（REQ-WIGA-09 修订；owner 2026-10-26 真机「终止整个
//! 自动化」永久 409 根修）。
//!
//! # 契约要点
//!
//! - **终止命令的身份不含乐观前置条件**：`expected_policy_revision` 是 CAS
//!   前置条件，不是命令意图。服务端自己的冲突投影
//!   （`termination_identity_conflict_details`）就把当前 revision 交回客户端
//!   并要求「刷新确认面身份再确认一次」；若把 revision 计入命令 payload 的
//!   身份摘要，这条被服务端指引的重试必然被判「异 payload」，终止入口从此
//!   永久 409——初始身份越用越旧，用户再点也无法收敛。
//! - **终止没有第二种意图**：终止 payload 只表达「终止哪个 enrollment、为何」
//!   （`command_id`／`expected_enrollment_id`／`reason`）；不像 Enable 的
//!   `source`/`options`/`target` 会改授权。因此同 `command_id` 的「仅前置条件
//!   刷新」重试是同一命令的续做，不是异命令。
//! - **CAS 语义不动**：账本判定只决定「是否重放首次 durable 结果」，其余一律
//!   落到既有精确 revision CAS（漂移仍 Conflict 零副作用）。REQ-WIGA-09 的
//!   并发保护不因本模块放宽。
//! - **旧账本条目兼容**：历史条目记的是**含 revision 的全量 payload 摘要**
//!   （`sha256:` 前缀）。字节全等仍重放；仅前置条件漂移时按「同命令续做」
//!   处理——否则修复前落盘的现场（例如前端稳定 command_id 的终止条目）永久
//!   死锁，无法用任何用户动作解开。新条目记 intent 摘要（`intent-sha256:`
//!   前缀），异 intent 仍 fail-closed。
//!
//! 从 `issue_automation_store.rs` 独立成文件：该文件已在 large_file_guard
//! 登记表内（禁止继续增长），终止账本判定与新逻辑统一放这里。

use sha2::{Digest, Sha256};

use crate::product::models::automation::{
    EnrollmentCommandResult, EnrollmentTerminationRequest, OperationState,
};

/// 新口径：终止命令的 intent 摘要前缀（不含 `expected_policy_revision`）。
pub(crate) const INTENT_DIGEST_PREFIX: &str = "intent-sha256:";

/// 终止命令的 intent 摘要：`command_id` ＋ `expected_enrollment_id` ＋ `reason`。
///
/// **不含** `expected_policy_revision`（乐观前置条件）——它就是本 endpoint
/// 冲突投影里交给客户端刷新的那个值，同一命令的重试必然携带新值。
pub(crate) fn termination_intent_digest(request: &EnrollmentTerminationRequest) -> String {
    let intent = format!(
        "{}\u{1f}{}\u{1f}{}",
        request.command_id, request.expected_enrollment_id, request.reason
    );
    format!(
        "{INTENT_DIGEST_PREFIX}{:x}",
        Sha256::digest(intent.as_bytes())
    )
}

/// 账本命中判定（同 `command_id` 的既有条目 vs 本次请求）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminationLedgerVerdict {
    /// 同一条终止命令（含仅 `expected_policy_revision` 刷新的续做）：已终止
    /// 即回报首次 durable 结果（零写入、revision 不涨）；未终止（曾被重新
    /// 开启）则落到下方精确 CAS 当作新一次尝试。
    SameCommand,
    /// 无法证明同命令（新条目异 intent）：fail-closed Conflict，零副作用。
    Divergent,
}

/// 判定同 `command_id` 的既有账本条目与本次终止请求是否同一条命令。
pub(crate) fn termination_ledger_verdict(
    entry: &EnrollmentCommandResult,
    request: &EnrollmentTerminationRequest,
) -> TerminationLedgerVerdict {
    let recorded = entry.payload_digest.as_str();
    if recorded == termination_intent_digest(request) {
        return TerminationLedgerVerdict::SameCommand;
    }
    // 旧口径条目：字节全等即同一命令的完全重放。
    if recorded == request.payload_digest() {
        return TerminationLedgerVerdict::SameCommand;
    }
    if recorded.starts_with(INTENT_DIGEST_PREFIX) {
        // 新口径条目可证 intent 不同 —— 异命令复用 command_id，拒绝。
        return TerminationLedgerVerdict::Divergent;
    }
    // 旧口径条目（含 revision 的 `sha256:` 摘要）+ 前置条件漂移：无法据摘要
    // 证明 intent 相同，但终止无第二种意图且后续仍受精确 revision CAS 约束，
    // 按同命令续做处理（否则修复前落盘的现场永久死锁）。
    TerminationLedgerVerdict::SameCommand
}

/// 终止 accepted 落盘时写账本：一条命令一个条目（同 `command_id` 就地升级为
/// intent 摘要，不追加重复项——账本按 `command_id` 查找）。
pub(crate) fn record_termination_ledger_accept(
    ledger: &mut Vec<EnrollmentCommandResult>,
    request: &EnrollmentTerminationRequest,
    binding_version: u64,
) {
    let digest = termination_intent_digest(request);
    match ledger
        .iter_mut()
        .find(|entry| entry.command_id == request.command_id)
    {
        Some(entry) => {
            entry.payload_digest = digest;
            entry.state = OperationState::Accepted;
            entry.binding_version = binding_version;
        }
        None => ledger.push(EnrollmentCommandResult {
            command_id: request.command_id.clone(),
            payload_digest: digest,
            state: OperationState::Accepted,
            binding_version,
        }),
    }
}

#[cfg(test)]
mod tests;
