//! REQ-WSC-13/14（owner 2026-10-30 反馈）：门卡的两种 durable 事实——
//!
//! 1. **已处置事实清单**（`human-dispositions.json`）：owner 逐条「排除此项」
//!    后落盘的处置事实；下一轮 reviewer prompt 携带它，声明已处置事实不得
//!    重复报为 actionable。
//! 2. **门 turn 拒收信息**（`human-gate-rejections/{turn_id}.json`）：门 turn
//!    以 `validation_reject` 失败时的可读原因、下一步动作与逐字诊断——现场
//!    此前只有 `failure_class`，全盘 grep 不到诊断，owner 无法行动。
//!
//! 两者与既有 gate turn / session 记录同族落点：都在
//! `workspace-sessions/{session_id}/` 之下，走同一 live-write 准入。

use chrono::Utc;

use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    HumanDispositionRecord, HumanGateTurnRejection, WorkspaceSessionRecord,
};

use super::LifecycleStore;

/// 已处置事实清单的落盘形态（单文件，按 finding_key 幂等去重）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HumanDispositionLedger {
    #[serde(default)]
    records: Vec<HumanDispositionRecord>,
}

impl LifecycleStore {
    fn workspace_session_root(&self, session: &WorkspaceSessionRecord) -> std::path::PathBuf {
        self.paths
            .issue_lifecycle_root(&session.project_id, &session.issue_id)
            .join("workspace-sessions")
            .join(&session.id)
    }

    /// REQ-WSC-13：读该会话的已处置事实清单（无文件＝空清单，不报错）。
    pub fn list_human_dispositions(
        &self,
        session_id: &str,
    ) -> Result<Vec<HumanDispositionRecord>, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session = self.get_workspace_session(session_id)?;
        let path = self
            .workspace_session_root(&session)
            .join("human-dispositions.json");
        if !super::path_exists(&path)? {
            return Ok(Vec::new());
        }
        let ledger: HumanDispositionLedger = read_json(&path)?;
        Ok(ledger.records)
    }

    /// REQ-WSC-13：把 owner 的处置事实并入 durable 清单（按 finding_key
    /// 幂等：同 key 重复提交零写；返回合并后的完整清单）。写入口走该会话
    /// 的 live-write 准入，与 gate turn / session 记录同一权威。
    pub fn record_human_dispositions(
        &self,
        session_id: &str,
        incoming: &[HumanDispositionRecord],
    ) -> Result<Vec<HumanDispositionRecord>, ProductStoreError> {
        validate_relative_id(session_id)?;
        for record in incoming {
            if !HumanDispositionRecord::is_valid_finding_key(&record.finding_key) {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "human_disposition",
                    reason: "finding_key must be 16 lowercase hex characters".to_string(),
                });
            }
        }
        let (_, scope) = self.resolve_session_scope(session_id)?;
        let incoming = incoming.to_vec();
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let session = self.get_workspace_session(session_id)?;
            let path = self
                .workspace_session_root(&session)
                .join("human-dispositions.json");
            let mut ledger: HumanDispositionLedger = if super::path_exists(&path)? {
                read_json(&path)?
            } else {
                HumanDispositionLedger::default()
            };
            let mut changed = false;
            for record in incoming {
                if ledger
                    .records
                    .iter()
                    .any(|existing| existing.finding_key == record.finding_key)
                {
                    continue;
                }
                ledger.records.push(record);
                changed = true;
            }
            if changed {
                write_json(&path, &ledger)?;
            }
            Ok(ledger.records)
        })
    }

    /// REQ-WSC-13/14（k3 P0-2C）：**撤销** owner 的排除（清单条目删除）。
    ///
    /// 处置事实是 owner 的裁定，排错后必须能撤回——否则该 finding 永久只能
    /// 以 advisory 出现，没有「重新提出为 actionable」的路径。撤销后下一轮
    /// reviewer prompt 不再携带该条，平台裁决也不再把它降级。返回撤销后的
    /// 剩余清单（幂等：无匹配 key 零写）。
    pub fn revoke_human_dispositions(
        &self,
        session_id: &str,
        finding_keys: &[String],
    ) -> Result<Vec<HumanDispositionRecord>, ProductStoreError> {
        validate_relative_id(session_id)?;
        let (_, scope) = self.resolve_session_scope(session_id)?;
        let revoked = finding_keys
            .iter()
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty())
            .collect::<Vec<_>>();
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let session = self.get_workspace_session(session_id)?;
            let path = self
                .workspace_session_root(&session)
                .join("human-dispositions.json");
            if !super::path_exists(&path)? {
                return Ok(Vec::new());
            }
            let mut ledger: HumanDispositionLedger = read_json(&path)?;
            let before = ledger.records.len();
            ledger
                .records
                .retain(|record| !revoked.iter().any(|key| key == &record.finding_key));
            if ledger.records.len() != before {
                write_json(&path, &ledger)?;
            }
            Ok(ledger.records)
        })
    }

    /// REQ-WSC-14（根因 E）：落盘门 turn 的 validation_reject 可操作信息。
    /// 同 turn 幂等（已存在同 turn 记录时零写），durable 可复盘。
    pub fn save_human_gate_turn_rejection(
        &self,
        session_id: &str,
        rejection: &HumanGateTurnRejection,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(session_id)?;
        validate_relative_id(&rejection.turn_id)?;
        let (_, scope) = self.resolve_session_scope(session_id)?;
        let rejection = rejection.clone();
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let session = self.get_workspace_session(session_id)?;
            let dir = self
                .workspace_session_root(&session)
                .join("human-gate-rejections");
            let path = dir.join(format!("{}.json", rejection.turn_id));
            if super::path_exists(&path)? {
                return Ok(());
            }
            write_json(&path, &rejection)
        })
    }

    /// REQ-WSC-14：读某 turn 的拒收信息（缺席＝None，不报错）。
    pub fn load_human_gate_turn_rejection(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<Option<HumanGateTurnRejection>, ProductStoreError> {
        validate_relative_id(session_id)?;
        validate_relative_id(turn_id)?;
        let session = self.get_workspace_session(session_id)?;
        let path = self
            .workspace_session_root(&session)
            .join("human-gate-rejections")
            .join(format!("{turn_id}.json"));
        if !super::path_exists(&path)? {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    /// 该会话最近一次（按落盘时间）门 turn 拒收信息——供呈现层复用。
    pub fn latest_human_gate_turn_rejection(
        &self,
        session_id: &str,
    ) -> Result<Option<HumanGateTurnRejection>, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session = self.get_workspace_session(session_id)?;
        let dir = self
            .workspace_session_root(&session)
            .join("human-gate-rejections");
        let mut latest: Option<HumanGateTurnRejection> = None;
        for path in super::json_file_paths(&dir)? {
            let Ok(rejection) = read_json::<HumanGateTurnRejection>(&path) else {
                continue;
            };
            if latest
                .as_ref()
                .is_none_or(|current| rejection.recorded_at >= current.recorded_at)
            {
                latest = Some(rejection);
            }
        }
        Ok(latest)
    }
}

/// 可读原因与下一步动作的单一事实源：由 typed 诊断稳定码映射到人话。
/// 稳定码缺省时给出通用可行动文案（绝不把错误码直接抛给 owner）。
pub(crate) fn human_gate_rejection_guidance(diagnostics: &[String]) -> (String, Vec<String>) {
    let has = |needle: &str| diagnostics.iter().any(|line| line.contains(needle));
    if has("STRUCTURAL_ATTEMPTS_EXHAUSTED") {
        return (
            "本轮反馈已用完结构性自动修复次数，仍未通过校验；本轮反馈没有被采纳。".to_string(),
            vec![
                "把反馈收窄成一条最小、可执行的修改，再提交一次".to_string(),
                "如果当前版本可以接受，直接在门卡上确认当前版本".to_string(),
            ],
        );
    }
    if has("STRUCTURAL_REPAIR_UNPRODUCIBLE") {
        return (
            "本轮反馈被判为需要机器修复，但平台无法生成可执行的修复指令；门仍停在原处。"
                .to_string(),
            vec![
                "把反馈改写为针对当前候选的具体改动（指出要改哪个字段/哪条验收标准）".to_string(),
                "如果问题需要范围或产品判断，改为在门卡上选择确认或终止".to_string(),
            ],
        );
    }
    if has("STRUCTURAL_EXISTING_GATE") {
        return (
            "本轮反馈涉及真实业务矛盾（缺少事实或契约未定），不能由平台自动返修，需要你决定。"
                .to_string(),
            vec![
                "在门卡上对这条矛盾做出选择（确认当前版本或终止）".to_string(),
                "补齐缺失事实后，用一条针对当前候选的反馈重新提交".to_string(),
            ],
        );
    }
    if has("STRUCTURAL_OPERATIONAL_WAIT") {
        return (
            "自动返修等待条件未满足（授权或机械校验窗口还没就绪），本轮反馈没有驱动修订。"
                .to_string(),
            vec![
                "稍后原样重试提交同一条反馈".to_string(),
                "先完成门卡上的授权/确认动作，再提交反馈".to_string(),
            ],
        );
    }
    if has("CANDIDATE_DRIFTED") || has("HUMAN_GATE_REVISION_CANDIDATE_DRIFTED") {
        return (
            "你提交反馈之后候选已经变化，本轮反馈对不上当前候选，因此没有被采纳。".to_string(),
            vec![
                "刷新后基于最新候选重新提交反馈".to_string(),
                "如果最新候选已经可以接受，直接在门卡上确认当前版本".to_string(),
            ],
        );
    }
    (
        "本轮反馈未通过平台校验，没有被采纳；门仍停在原处。".to_string(),
        vec![
            "查看下方诊断原文，把反馈收窄成一条可执行的修改后重新提交".to_string(),
            "如果当前版本可以接受，直接在门卡上确认当前版本".to_string(),
        ],
    )
}

/// 构造一条 durable 拒收记录（`turn_id` 由调用方给出）。
pub(crate) fn build_human_gate_turn_rejection(
    turn_id: &str,
    rejected_feedback: &str,
    diagnostics: &[String],
) -> HumanGateTurnRejection {
    let (reason, next_actions) = human_gate_rejection_guidance(diagnostics);
    HumanGateTurnRejection {
        turn_id: turn_id.to_string(),
        rejected_feedback: rejected_feedback.to_string(),
        reason,
        next_actions,
        diagnostics: diagnostics.to_vec(),
        recorded_at: Utc::now().to_rfc3339(),
    }
}
