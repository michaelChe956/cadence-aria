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

    /// REQ-WSC-18/19：门卡上的「当前候选编译失败」只读投影。
    ///
    /// 判据全部在平台侧：读 durable 候选编译事实 + 按会话当前候选绑定
    /// （source revision + candidate IR）比较；只有「这个候选确实编译不过」
    /// 才投影，失败属于旧候选或任一侧 ref 缺席时整体省略。
    pub(crate) fn candidate_compile_failure_projection(
        &self,
    ) -> Option<crate::web::workspace_ws_types::CandidateCompileFailureDto> {
        let status = self.current_candidate_compile_status()?;
        let failure = status.failure.as_ref()?;
        // 只有「候选自身无效」（编译类）才上牌；store/IO/failpoint 这类
        // operational 失败不是「这个候选无效」，把它标成无效会制造新的死路。
        if !crate::product::workspace_engine::compile_failure_surface::is_candidate_invalid_code(
            &failure.code,
        ) {
            return None;
        }
        Some(
            crate::web::workspace_ws_types::CandidateCompileFailureDto::from_status(
                &status, failure,
            ),
        )
    }

    /// REQ-WSC-19：当前候选的完整候选编译事实（durable 读取 + 候选绑定比较）。
    pub(crate) fn current_candidate_compile_status(
        &self,
    ) -> Option<crate::product::models::CandidateCompileStatus> {
        let store = self.lifecycle_store.as_ref()?;
        let record = store
            .get_workspace_session(&self.session.session_id)
            .ok()?;
        store.current_candidate_compile_status(&record).ok().flatten()
    }

    /// REQ-WSC-19：durable 判定——该会话**当前候选**是否已确定编译不过。
    pub(crate) fn current_candidate_compile_failure(
        &self,
    ) -> Option<crate::product::models::CandidateCompileFailure> {
        let store = self.lifecycle_store.as_ref()?;
        let record = store
            .get_workspace_session(&self.session.session_id)
            .ok()?;
        store.current_candidate_compile_failure(&record).ok().flatten()
    }

    /// REQ-WSC-19：确认动作的平台侧 fail-closed 判据——当前候选编译不过时
    /// 一律拒绝放行（确认必然重跑同一次编译、必然再失败，是死路）。
    /// 判据只认 durable 事实，前端禁用按钮只是同一判据的呈现面。
    pub(crate) fn ensure_current_candidate_compile_not_failed(&self) -> Result<(), String> {
        match self.current_candidate_compile_failure() {
            // 只有「候选自身无效」才拦确认；operational 失败（store/IO/failpoint）
            // 允许重试——否则一次瞬态错误会把门永久锁死（新的死路）。
            Some(failure)
                if crate::product::workspace_engine::compile_failure_surface::is_candidate_invalid_code(
                    &failure.code,
                ) =>
            {
                Err(crate::product::workspace_engine::compile_failure_surface::approve_rejected_message(
                    &failure,
                ))
            }
            _ => Ok(()),
        }
    }

    /// REQ-WSC-19：给 author 的教学块（进入 SC 门内修订提示词）——当前候选
    /// 已确定编译不过时携带具体重写要点；无失败时 None，既有 prompt 逐字不变。
    pub(crate) fn candidate_compile_failure_author_teaching(&self) -> Option<String> {
        self.current_candidate_compile_failure().map(|failure| {
            crate::product::workspace_engine::compile_failure_surface::author_teaching_block(
                &failure,
            )
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
