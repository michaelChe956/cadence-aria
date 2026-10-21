//! 从 mod.rs 拆出以满足 large_file_guard 的登记上限（纯移动，无行为变化）。
//! Task 9a（REQ-ENV-04）：LC 显式 resume 的 durable provider_start 审计门——
//! spawn 前比较完整 LC 审计字面量（冻结投影摘要 + v2 resume 材料），漂移或
//! legacy 记录一律 superseded 终止审计 + 零 child 拒绝。
use super::*;

use crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink;
use crate::product::logical_codebase::policy::SessionPolicyEnvelope;
use crate::product::logical_codebase::provider_projection::ProviderPolicyProjection;

/// Task 9a(REQ-ENV-04):LC 启动的完整 resume 审计字面量(投影摘要 + v2
/// resume 材料),与 codex/pi/claude 三家的同名装配逐字段同源。action
/// evidence 摘要与 `capability_projection_digest` 同源;resume 指纹为五参
/// `from_envelope` 全上下文计算(policy/authority/cwd/target/git identity/
/// exact version/语义 tool policy/trust/config/MCP 都在投影摘要内)。
fn lc_start_audit_literal(
    envelope: &crate::product::logical_codebase::policy::SessionPolicyEnvelope,
    lc_projection: &crate::product::logical_codebase::provider_projection::ProviderPolicyProjection,
) -> crate::cross_cutting::tool_policy_audit::LcProviderStartAudit {
    let git_identity =
        crate::product::logical_codebase::provider_gateway::canonical_target_git_identity(
            &envelope.target.worktree,
        );
    let action_evidence = lc_projection.capability_projection_digest().to_string();
    crate::cross_cutting::tool_policy_audit::LcProviderStartAudit {
        action: projection::action_text(envelope.action).to_string(),
        wire_dialect: projection::wire_dialect_text(lc_projection.wire_dialect()).to_string(),
        capability_projection_digest: action_evidence.clone(),
        projection_digest: lc_projection.projection_digest().to_string(),
        boundary_evidence_ref: lc_projection.boundary_evidence_ref().to_string(),
        resume_fingerprint:
            crate::product::logical_codebase::provider_gateway::SessionResumeFingerprint::from_envelope(
                envelope,
                lc_projection,
                &action_evidence,
                &git_identity,
            )
            .digest,
        action_evidence_digest: action_evidence,
        target_git_identity: git_identity,
    }
}

/// Task 9a(REQ-ENV-04):LC 显式 resume 在 child 前比较全 LC
/// audit(完整字面量含冻结投影 + v2 resume 材料),与
/// codex/pi/claude 三家同形。存档缺 `lc_projection`(旧/direct
/// 形态)或任一投影摘要漂移 → superseded 终止审计写被取代旧
/// run、零 child 拒绝,绝不清 resume id 静默 fresh。无存档记录
/// 时与三家同为「依赖原生握手确认」,本层不额外拒绝(共享口径,
/// 不在此单方改变)。门位于版本/投影复验之后、预留与 spawn 之前
/// ——拒绝路径零 child、零登记、零新 provider_start。生产构造
/// (`KimiCodeProvider::new`,无 Aria MCP bundle)同样必经此门,
/// 不因 MCP digest 惰性而退化。
pub(super) fn ensure_lc_resume_audit(
    sink: &dyn ToolPolicyAuditSink,
    envelope: &SessionPolicyEnvelope,
    lc_projection: &ProviderPolicyProjection,
    input: &StreamingProviderInput,
) -> Result<(), ProviderAdapterError> {
    if let Some(resume_id) = input
        .resume_provider_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        let stored = sink.find_provider_start(resume_id).map_err(|error| {
            ProviderAdapterError::parse_error(
                format!("kimi lc validated start: resume lookup failed: {error}"),
                String::new(),
                String::new(),
            )
        })?;
        if let Some(stored) = stored.as_ref() {
            let current = lc_start_audit_literal(envelope, lc_projection);
            if matches!(
                crate::cross_cutting::tool_policy_audit::resume_with_lc_start_record(
                    &stored.record,
                    &current
                ),
                crate::cross_cutting::tool_policy_audit::ResumeDecision::RejectSupersedeAndStartNew
            ) {
                // P1-4 同源裁决:superseded 终止审计写入被取代旧 run。
                crate::cross_cutting::tool_policy_audit::append_superseded_policy_drift(
                    sink, stored,
                )
                .map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("kimi lc validated start: superseded audit append failed: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?;
                return Err(ProviderAdapterError::parse_error(
                    "kimi lc validated start: resume audit drifted or legacy record lacks lc projection; refusing to spawn a fresh child for an explicit LC resume (session NOT started; no child was spawned)",
                    String::new(),
                    String::new(),
                ));
            }
        }
    }
    Ok(())
}

impl KimiCodeProvider {
    /// LC MCP 来源事实:Aria 注入 bundle digest 或 native 项目配置标记
    /// (单独标来源,不冒充 Aria bundle digest)。LC 审计字面量的投影来源
    /// 事实,随本模块同置。
    pub(super) fn lc_mcp_bundle_digest(&self) -> String {
        match self.mcp_injection.as_ref() {
            Some(config) => config.bundle.digest().to_string(),
            None => projection::KIMI_NATIVE_MCP_SOURCE.to_string(),
        }
    }
}
