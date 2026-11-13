//! P05 lookup：按 scope + 业务键 + 原始节点 + 轮节点 + ordinal 找回已绑定的真实轮。
//!
//! 只读：枚举 store 私有证据分区（不扫描全项目盘），逐分区复用既有严格读回
//! （坏行 / 断号 / 跨 chain 停下并带已验证前缀），再按链身份与轮身份精确过滤；
//! 命中唯一才返回 `Some`，多命中报错（不挑"最新"）。
//! 不写、不建目录、不分配 seq、不读 provider 产物、不解析不透明 id、不猜 hash。

use std::path::Path;

use super::{
    CODE_LOG_CORRUPT, CODE_SCOPE_MISMATCH, SC_FAILURE_EVIDENCE_DIR, SC_FAILURE_EVIDENCE_LOG,
    ScFailureEvidenceEntry, ScFailureEvidenceError, ScFailureEvidenceEvent, ScFailureEvidenceRead,
    error, io_error, read_sc_failure_log, sc_failure_log_failure, scope_mismatch_error,
    session_not_live_error, session_not_sc_error,
};
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::WorkspaceType;
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::web::workspace_ws_types::{ScFailureChainIdentity, ScFailureScope, ScRoundIdentity};

/// 多命中：同一 scope 下存在多个匹配的已绑定轮（不得挑"最新"）。
const CODE_LOOKUP_AMBIGUOUS: &str = "sc_failure_lookup_ambiguous";

impl LifecycleStore {
    /// P05 lookup（Plan §2.4）：按 scope + 业务键 + 原始节点 + 轮节点 + ordinal
    /// 找回已 RoundBound 的真实轮身份。
    ///
    /// 读门与既有读路径同口径：会话必须是真实的 SC single-candidate
    /// WorkItemPlan，且 scope 的 project/issue/plan 与真实会话身份一致。
    pub(crate) fn find_sc_failure_round(
        &self,
        scope: &ScFailureScope,
        business_key: &str,
        original_node_id: &str,
        round_node_id: &str,
        ordinal: u32,
    ) -> Result<Option<(ScFailureChainIdentity, ScRoundIdentity)>, ScFailureEvidenceError> {
        const STEP: &str = "find_sc_failure_round";
        self.sc_failure_lookup_session(scope, STEP)?;
        let root = self
            .workspace_timeline_root_for_session(&scope.session_id)
            .map_err(|failure| {
                io_error(
                    STEP,
                    None,
                    format!("定位 workspace timeline 失败: {failure}"),
                )
            })?
            .join(SC_FAILURE_EVIDENCE_DIR);
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(failure) => {
                return Err(io_error(STEP, None, format!("读取证据分区失败: {failure}")));
            }
        };
        let mut partitions: Vec<String> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|failure| {
                io_error(STEP, None, format!("读取证据分区条目失败: {failure}"))
            })?;
            partitions.push(entry.file_name().to_string_lossy().into_owned());
        }
        partitions.sort();
        let mut found: Option<(ScFailureChainIdentity, ScRoundIdentity)> = None;
        for partition in partitions {
            let log_path = root.join(&partition).join(SC_FAILURE_EVIDENCE_LOG);
            let Some(chain) =
                self.sc_failure_lookup_partition_chain(&log_path, &partition, scope, STEP)?
            else {
                continue;
            };
            // 每个分区都严格读回：坏行 / 断号 / 跨 chain 立即报错（不静默跳过）。
            let read: ScFailureEvidenceRead = read_sc_failure_log(&log_path, &chain, STEP)?;
            if chain.business_key != business_key || chain.original_node_id != original_node_id {
                continue;
            }
            for entry in &read.entries {
                let ScFailureEvidenceEvent::RoundBound { round } = &entry.event else {
                    continue;
                };
                if round.node_id != round_node_id || round.round_ordinal != ordinal {
                    continue;
                }
                if found.is_some() {
                    // 多命中时轮身份不唯一：不报某一个 round_id（不挑"最新"）。
                    return Err(error(
                        CODE_LOOKUP_AMBIGUOUS,
                        STEP,
                        None,
                        "同一 scope 下存在多个匹配的已绑定轮",
                        false,
                    ));
                }
                found = Some((chain.clone(), round.clone()));
            }
        }
        Ok(found)
    }

    /// 读门：会话必须是真实的 SC single-candidate WorkItemPlan，且 scope 的
    /// project/issue/plan 与真实会话身份一致（与既有读路径同口径）。
    fn sc_failure_lookup_session(
        &self,
        scope: &ScFailureScope,
        step: &str,
    ) -> Result<(), ScFailureEvidenceError> {
        let session =
            self.get_workspace_session(&scope.session_id)
                .map_err(|failure| match failure {
                    ProductStoreError::NotFound { .. } => {
                        session_not_live_error(step, "会话不存在或已不可用")
                    }
                    other => io_error(step, None, format!("读取会话失败: {other}")),
                })?;
        if session.workspace_type != WorkspaceType::WorkItemPlan
            || session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
        {
            return Err(session_not_sc_error(
                step,
                "会话不是 SC single-candidate WorkItemPlan",
            ));
        }
        if scope.project_id != session.project_id
            || scope.issue_id != session.issue_id
            || scope.plan_id != session.entity_id
        {
            return Err(scope_mismatch_error(
                step,
                None,
                "scope 与真实会话身份不一致",
            ));
        }
        Ok(())
    }

    /// 分区链身份：日志首条 entry 的 chain（分区目录名即 chain_id，scope 必须
    /// 与本会话一致）。日志不存在或为空 → `None`（该分区没有事实）。
    fn sc_failure_lookup_partition_chain(
        &self,
        log_path: &Path,
        partition_chain_id: &str,
        scope: &ScFailureScope,
        step: &str,
    ) -> Result<Option<ScFailureChainIdentity>, ScFailureEvidenceError> {
        let bytes = match std::fs::read(log_path) {
            Ok(bytes) => bytes,
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(failure) => {
                return Err(io_error(step, None, format!("读取证据日志失败: {failure}")));
            }
        };
        let Some(line) = bytes
            .split(|byte| *byte == b'\n')
            .find(|line| !line.is_empty())
        else {
            return Ok(None);
        };
        let text = std::str::from_utf8(line).map_err(|failure| {
            sc_failure_log_failure(
                CODE_LOG_CORRUPT,
                step,
                None,
                &[],
                &[],
                format!("证据日志行不是 UTF-8: {failure}"),
            )
        })?;
        let entry: ScFailureEvidenceEntry = serde_json::from_str(text).map_err(|failure| {
            sc_failure_log_failure(
                CODE_LOG_CORRUPT,
                step,
                None,
                &[],
                &[],
                format!("证据日志行不是合法条目: {failure}"),
            )
        })?;
        if entry.chain.chain_id != partition_chain_id {
            return Err(sc_failure_log_failure(
                CODE_SCOPE_MISMATCH,
                step,
                None,
                &[],
                &[],
                "证据日志条目不属于本 chain 分区".to_string(),
            ));
        }
        if entry.chain.scope != *scope {
            return Err(sc_failure_log_failure(
                CODE_SCOPE_MISMATCH,
                step,
                None,
                &[],
                &[],
                "证据日志条目属于其他 scope（跨 scope 注入）".to_string(),
            ));
        }
        Ok(Some(entry.chain))
    }
}
