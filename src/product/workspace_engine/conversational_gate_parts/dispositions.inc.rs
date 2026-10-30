// REQ-WSC-13/14（k3 P0-2A / P0-2C / REQ-WSC-14）：排除-only 受理判据、排除
// 撤销与「最近一次拒收」只读投影。本文件由 `conversational_gate.rs` 以
// `include!` 并入同一模块作用域（large_file_guard 拆文件，不压缩空行）。

/// REQ-WSC-13（k3 P0-2A 修法 A）：排除-only 反馈的受理判据——`feedback`
/// 为空（trim 后）**且**至少一个排除 key 形态合法时，本帧只表达「owner 排除
/// 若干 finding」这一处置事实：落 durable 清单即受理，**不建修订 turn、不跑
/// `validate_feedback`**。否则保持改动前的语义（空反馈照旧
/// `INVALID_HUMAN_GATE_FEEDBACK`），旧客户端字节行为不变。
pub(crate) fn exclude_only_feedback_requested(
    feedback: &str,
    excluded_finding_ids: &[String],
) -> bool {
    feedback.trim().is_empty()
        && excluded_finding_ids.iter().any(|key| {
            crate::product::models::HumanDispositionRecord::is_valid_finding_key(key.trim())
        })
}

/// 排除 key 的平台形态归一（trim + 只留合法 key）——回执与落盘同一口径。
pub(crate) fn normalized_finding_keys(finding_keys: &[String]) -> Vec<String> {
    finding_keys
        .iter()
        .map(|key| key.trim().to_string())
        .filter(|key| crate::product::models::HumanDispositionRecord::is_valid_finding_key(key))
        .collect()
}

impl super::WorkspaceEngine {
    /// REQ-WSC-14：该会话最近一次门 turn 拒收的只读投影（与
    /// `human_gate_turn_rejected` 帧同源）——`session_state` 全量投影/重建
    /// 据此恢复门卡四要素，刷新后仍可读。
    pub(crate) fn latest_human_gate_turn_rejection_projection(
        &self,
    ) -> Option<crate::web::workspace_ws_types::HumanGateTurnRejectionDto> {
        let store = self.lifecycle_store.as_ref()?;
        store
            .latest_human_gate_turn_rejection(&self.session.session_id)
            .ok()
            .flatten()
            .map(|rejection| {
                crate::web::workspace_ws_types::HumanGateTurnRejectionDto::from(&rejection)
            })
    }

    /// REQ-WSC-13/14（k3 P0-2C）：撤销 owner 的排除——清单条目删除后，该
    /// finding 恢复为「可报为 actionable」（下一轮 prompt 不再携带、平台裁决
    /// 不再降级）。返回撤销后的剩余清单 key。
    pub(crate) async fn revoke_owner_finding_exclusions(
        &mut self,
        finding_keys: &[String],
    ) -> Result<Vec<String>, String> {
        let Some(store) = self.lifecycle_store.clone() else {
            return Ok(Vec::new());
        };
        let remaining = store
            .revoke_human_dispositions(&self.session.session_id, finding_keys)
            .map_err(|error| error.to_string())?;
        Ok(remaining
            .into_iter()
            .map(|record| record.finding_key)
            .collect())
    }
}
