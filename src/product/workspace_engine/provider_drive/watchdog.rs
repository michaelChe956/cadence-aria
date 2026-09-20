//! F-19 provider 零活动看门狗：超时常量与 legacy 流 provider start 诊断登记。
//! 自 `provider_drive.rs` 拆出（1200 行守护，Wave 3.1）：纯移动零语义变化。

use super::*;

/// F-19 看门狗：provider 会话零活动上限（拉起后无事件/命令持续至此即判楔死）。
///
/// 依据：v27 story 会话 codex app-server 楔死实测——09:16 拉起，前几分钟 9 条
/// skill 探索命令后**完全静默 27min**（CPU 时间采样零增长、出站 TCP 0 条、
/// durable streaming 恒 137 字；cadence/notes 2026-09-19 阶段4监控 F-19）。
/// 流式 provider 正常推理逐 token 产出事件，合法静默间隙远小于 10min；
/// `DEFAULT_PROVIDER_TIMEOUT_SECS=3h` 是整 run 总上限，对「子进程活着但不
/// 干活」无效（27min 楔死全程在总上限内）。取 600s：约为实测楔死确认时间
/// 的 1/3（27min→10min 即转可诊断终态），同时低于 ApprovalBridge
/// `PERMISSION_TIMEOUT=15min` 的人工等待界（权限/选择挂起期间看门狗不计时，
/// 见驱动循环挂起逻辑）。
#[cfg(not(test))]
pub(crate) const PROVIDER_IDLE_WATCHDOG_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(600);
#[cfg(test)]
pub(crate) const PROVIDER_IDLE_WATCHDOG_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(150);

/// F-22/F-19b：choice（结构化提问）悬置等待上限——provider 发出
/// ChoiceRequest（pi `ask_user`/claude AskUserQuestion/codex requestUserInput）
/// 后等待人工应答的界。
///
/// 依据：v28 实测 0482（codex 首跑 + abort 后 pi 重跑）与 0483（全新会话，无
/// abort）三连楔死——provider 经结构化提问等待用户，看门狗按设计挂起
/// （`pending_choice_requests` 非空不计时），而 choice 卡经 broadcast 的
/// try_send 送达面无重发/无恢复（丢失即无人应答），run 无人收口永久楔死
/// （12–15+ 分钟零增长零出站，看门狗零触发）。取 900s 与 ApprovalBridge
/// `PERMISSION_TIMEOUT` 的人工等待界对齐：权限悬置由 adapter 侧超时收口，
/// choice 悬置此前无任何界——本常量补齐对称语义。触发后转可诊断失败态
/// （原因码 `provider_choice_wait_timeout`），story/design 面回 prepare_context
/// 可重跑。
#[cfg(not(test))]
pub(crate) const PROVIDER_CHOICE_WAIT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(900);
#[cfg(test)]
pub(crate) const PROVIDER_CHOICE_WAIT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(150);

impl WorkspaceEngine {
    /// F-19：legacy 流（story/design/workitem）provider start 的诊断登记。
    ///
    /// 拉起 provider 子进程前在 durable `provider_start_ledger` 落一条带
    /// provider 名与时间戳的条目（key 形 `workspace_{role}:{session_id}:{n}`，
    /// 对照 SC 面 `single_candidate_author:{session}:{n}` 先例）。F-19 实测中
    /// ledger 恒空导致「start 未发生」与「start 后楔死」无法区分——本登记补齐
    /// 该诊断面（SC/门预留路径各自已有登记，不经此处）。
    ///
    /// 失败语义：诊断面 best-effort——登记失败仅告警不阻断 run（fail-closed 会
    /// 因 store 抖动杀死正常生成，与登记目的不成比例；对照 tool-policy audit
    /// 的 fail-closed 语义：那是承重契约，这是观测登记）。
    pub(crate) fn register_provider_start_in_ledger(
        &mut self,
        role: ProviderConversationRole,
        provider: ProviderName,
    ) {
        // 范围闸：WorkItemPlan 面（Legacy+SingleCandidate）的 ledger 键索引由
        // 既有预留路径按 len 派生（routing_scope/SC reserve），追加诊断条目会
        // 漂移其键算术；该面已各自登记，本登记仅覆盖 story/design/workitem
        // 聊天面（F-19 实测楔死面）。
        if matches!(self.session.workspace_type, WorkspaceType::WorkItemPlan) {
            return;
        }
        let Some(store) = self.lifecycle_store.as_ref() else {
            return;
        };
        let session_id = self.session.session_id.clone();
        let attempt = self.session.provider_start_ledger.len();
        // ProviderConversationRole/ProviderName 均为 snake_case serde 枚举：
        // key 与 provider 文本统一取 serde 投影（author/reviewer、codex/pi…）。
        let role_text = serde_json::to_value(&role)
            .ok()
            .and_then(|value| value.as_str().map(ToString::to_string))
            .unwrap_or_else(|| "provider".to_string());
        let key = format!("workspace_{role_text}:{session_id}:{attempt}");
        let provider_text = serde_json::to_value(&provider)
            .ok()
            .and_then(|value| value.as_str().map(ToString::to_string))
            .unwrap_or_else(|| "unknown".to_string());
        let started_at = chrono::Utc::now().to_rfc3339();
        match store.claim_provider_start_with_details(
            &session_id,
            &key,
            Some(provider_text.as_str()),
            Some(started_at.clone()),
        ) {
            Ok(true) => {
                self.session.provider_start_ledger.push(
                    crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
                        provider_start_idempotency_key: key,
                        started: true,
                        provider: Some(provider_text),
                        started_at: Some(started_at),
                    },
                );
            }
            Ok(false) => {
                tracing::warn!(
                    session_id = %session_id,
                    key = %key,
                    "provider start already registered; skipping duplicate ledger entry"
                );
            }
            Err(error) => {
                tracing::warn!(
                    session_id = %session_id,
                    key = %key,
                    error = %error,
                    "provider start ledger registration failed (best-effort diagnostics)"
                );
            }
        }
    }
}
