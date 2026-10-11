//! trust 指引卡「登记并重试」按钮（owner 2026-10-12 直批，免双审裁定已
//! 入账）：`RegisterTrustEntry` WS 动作的 decisions 处理臂。
//!
//! 语义边界：
//! - root **服务端解析**（引擎 `prepare_explicit_trust_registration`：LC
//!   冻结根漂移重验/单仓 repository 根），**不接受客户端传路径**；
//! - 复用现有 ensure 登记链（`ensure_ready_cancellable`：幂等/审计
//!   ensure·registered|replay|reuse_user_entry/归属隔离/home sidecar 锁），
//!   场景=`ExplicitRegistration`（显式按钮授权），operation_id=
//!   `register_trust_entry:<session>` 关联审计；
//! - 成功仅回 typed 回执 `trust_entry_registered`（action/trust_key/
//!   canonical_root），**不自动触发重新生成**；失败以稳定码
//!   `trust_registration_waiting` 投影（reason_code/retry_action/
//!   manual_guidance 原样上浮，不折叠）；
//! - 无需 workspace trust 的 provider 显式 typed 拒收（零 ensure 调用）；
//! - 引擎锁只覆盖解析快照，home IO 在锁外执行（不楔死引擎/run）。

use super::*;

/// 稳定等待码：登记失败（home 侧等待或解析上下文 fail-closed）的统一
/// WS ProtocolError code；reason_code 原样保留在 message 与 context。
pub(crate) const TRUST_REGISTRATION_WAITING_CODE: &str = "trust_registration_waiting";

/// 无需 workspace trust 的 provider 拒收码（Claude/Pi/Fake 无用户级 trust
/// 工件，按钮不该出现；出现即协议面诚实拒绝，不进 ensure）。
pub(crate) const PROVIDER_TRUST_NOT_REQUIRED_CODE: &str = "provider_trust_not_required";

pub(crate) async fn handle_register_trust_entry_from_handler(
    app_state: WebAppState,
    engine: Arc<Mutex<WorkspaceEngine>>,
    outbound_tx: mpsc::Sender<OutboundControl>,
    provider: ProviderName,
) {
    if !crate::product::logical_codebase::requires_workspace_trust(&provider) {
        let err = WsOutMessage::ProtocolError {
            code: PROVIDER_TRUST_NOT_REQUIRED_CODE.to_string(),
            message: format!(
                "provider {provider:?} 不需要用户级 workspace trust；「登记并重试」仅适用于 \
                 codex/kimi_code 的 trust 缺失指引卡"
            ),
            context: Some(serde_json::json!({
                "provider": crate::product::logical_codebase::provider_trust::provider_wire_label(&provider),
                "received": "register_trust_entry",
            })),
        };
        let _ = send_json_outbound(&outbound_tx, &err).await;
        return;
    }

    // 解析快照（引擎锁短持有）：root 服务端解析 + precondition 归位。
    let target = {
        let locked_engine = engine.lock().await;
        locked_engine
            .prepare_explicit_trust_registration(app_state.aggregate_initialization_trust())
    };
    let target = match target {
        Ok(target) => target,
        Err(waiting) => {
            let err = WsOutMessage::ProtocolError {
                code: TRUST_REGISTRATION_WAITING_CODE.to_string(),
                message: waiting.message.clone(),
                context: Some(serde_json::json!({
                    "reason_code": waiting.reason_code,
                    "retryable": true,
                })),
            };
            let _ = send_json_outbound(&outbound_tx, &err).await;
            return;
        }
    };

    let operation_id = format!("register_trust_entry:{}", target.session_id);
    let outcome = target
        .precondition
        .ensure_ready_cancellable(
            target.project_id.clone(),
            operation_id,
            target.registry_lc_id.clone(),
            target.canonical_root.clone(),
            vec![provider.clone()],
            crate::product::logical_codebase::provider_trust::ProviderTrustScenario::ExplicitRegistration,
        )
        .await;
    let registration = match outcome {
        crate::product::logical_codebase::ProviderTrustPreparationResult::Ready {
            registrations,
        } => match registrations.first() {
            Some(registration) => registration.clone(),
            // 过滤后为空=provider 无需 trust（上方已拒）；防御性诚实回执，
            // 不伪称登记成功。
            None => {
                let err = WsOutMessage::ProtocolError {
                    code: PROVIDER_TRUST_NOT_REQUIRED_CODE.to_string(),
                    message: format!(
                        "provider {provider:?} 经 trust 过滤后无需登记；「登记并重试」不适用"
                    ),
                    context: None,
                };
                let _ = send_json_outbound(&outbound_tx, &err).await;
                return;
            }
        },
        crate::product::logical_codebase::ProviderTrustPreparationResult::Waiting { waiting } => {
            let err = WsOutMessage::ProtocolError {
                code: TRUST_REGISTRATION_WAITING_CODE.to_string(),
                message: format!(
                    "{}；人工登记指导：{}（retry_action：{}）；登记停等，修复后可重新点击「登记并重试」，不自动重试",
                    waiting.message, waiting.manual_guidance, waiting.retry_action,
                ),
                context: Some(serde_json::json!({
                    "reason_code": waiting.reason_code,
                    "provider": crate::product::logical_codebase::provider_trust::provider_wire_label(&waiting.provider),
                    "trust_key": waiting.trust_key,
                    "canonical_root": waiting.canonical_root.display().to_string(),
                    "retry_action": waiting.retry_action,
                    "retryable": true,
                })),
            };
            let _ = send_json_outbound(&outbound_tx, &err).await;
            return;
        }
    };

    let action = match registration.action {
        crate::product::logical_codebase::ProviderTrustAction::Register => {
            TrustEntryRegistrationActionDto::Registered
        }
        crate::product::logical_codebase::ProviderTrustAction::Replay => {
            TrustEntryRegistrationActionDto::Replay
        }
        crate::product::logical_codebase::ProviderTrustAction::ReuseUserEntry => {
            TrustEntryRegistrationActionDto::ReuseUserEntry
        }
    };
    let receipt = WsOutMessage::TrustEntryRegistered {
        provider: registration.provider,
        action,
        trust_key: registration.trust_key,
        canonical_root: registration.canonical_root.display().to_string(),
        registered_at: registration.registered_at,
    };
    // 仅回执：不自动触发重新生成（成功后卡片态提示「已登记，请显式重新
    // 生成」，由用户显式重新发起）。
    let _ = send_json_outbound(&outbound_tx, &receipt).await;
}
