#![allow(
    clippy::large_enum_variant,
    clippy::manual_async_fn,
    clippy::too_many_arguments
)]

pub mod clarification;
pub mod code_review;
pub mod coding;
pub mod design_authoring;
pub mod design_review;
pub mod design_revision;
pub mod execution_setup;
pub mod final_review;
pub mod final_summary;
pub mod intake_capture;
pub mod integration_execute;
pub mod integration_prepare;
pub mod integration_verify;
pub mod patch_followup_dispatch;
pub mod plan_dispatch;
pub mod prompt_template_registry;
pub mod rework;
pub mod session_bootstrap;
pub mod session_closeout;
pub mod spec_authoring;
pub mod spec_gate_review;
pub mod task_init;

use serde_json::Value;
use std::future::Future;

#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalNodeInput {
    pub session_id: String,
    pub task_id: Option<String>,
    pub node_id: String,
    pub risk_registry_ref: Option<String>,
    pub payload: Value,
}

/// 批2 Task 11（硬约束1，T10 方案A 等价包装）：runtime unit 裸
/// `provider.run` 的受信载体。Direct 链的 prompt 装配段仍是
/// `build_provider_context`（T10 裁定：它不能作为铸造 validated input 的
/// 边界），合同铸造收口在此——以节点 durable run 事实构造
/// `TrustedRunIdentity` 并按权威装配矩阵冻结执行合同，任何矩阵失配都在
/// `provider.run` 之前零副作用拒绝（裸 run 不再存在）。
///
/// 职责口径（T10 已裁定并合同测试锁定；**Direct 恒 Coder=受信裁定**）：
/// `runtime_role` 参数仅作为 durable run 身份事实记录，不参与 duty 派生——
/// legacy Direct 形态 identity 恒 `HostCoding/Coder`，本链的 review-report
/// 节点是 Direct 链的报告工人
/// （D §3.2：Direct Coder/Reviewer 不注入策略、danger 原生 approval），
/// B 档 `ReviewerVerify+窗口` 职责未经真窗（T20）不可 claim；Author 岗位
/// 经 LC author root launch 独立入口（`bind_author_execution_contract`）
/// 接线，不从本链按 RuntimeRole 提升。
pub(crate) fn freeze_runtime_unit_execution(
    session_id: &str,
    node_id: &str,
    provider_type: crate::protocol::contracts::ProviderType,
    runtime_role: crate::protocol::contracts::RuntimeRole,
    provider_run_id: &str,
) -> Result<
    crate::cross_cutting::provider_execution::FrozenProviderExecution,
    crate::cross_cutting::provider_adapter::ProviderAdapterError,
> {
    use crate::cross_cutting::provider_execution::{
        ProviderExecutionProfile, TrustedProviderRole, TrustedRunIdentity,
        freeze_provider_execution,
    };
    if session_id.trim().is_empty() || provider_run_id.trim().is_empty() {
        return Err(crate::cross_cutting::provider_adapter::ProviderAdapterError::provider_unavailable(
            "runtime unit execution contract requires durable session/run identity facts"
                .to_string(),
        ));
    }
    let run_record = crate::protocol::contracts::ProviderRunRecord {
        provider_run_id: provider_run_id.to_string(),
        node_id: node_id.to_string(),
        provider_type,
        runtime_role,
        // T11 矩阵：legacy Direct 形态 identity 恒 Coder（承载=执行工人）。
        adapter_role: crate::protocol::contracts::RuntimeRole::Executor.adapter_role(),
        provider_capability_ref: "cap_runtime_unit_direct".to_string(),
        adapter_compatibility_ref: "compat_1".to_string(),
        context_package_ref: format!("ctx_{node_id}"),
        adapter_input_ref: format!("ain_{node_id}"),
        adapter_output_ref: format!("aout_{node_id}"),
        raw_artifact_refs: Vec::new(),
        exit_code: None,
        error_code: None,
        error_details: None,
        stdout_ref: None,
        stderr_ref: None,
        structured_output_ref: None,
        files_modified: Vec::new(),
        status: crate::protocol::contracts::ProviderRunStatus::Running,
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        duration_ms: None,
        timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
        retry_count: 0,
        approval_policy: crate::protocol::contracts::ApprovalPolicy::OnRequest,
        sandbox_mode: crate::protocol::contracts::SandboxMode::FullAccess,
        constraint_check_ref: None,
        traceability_binding_refs: Vec::new(),
    };
    let identity =
        TrustedRunIdentity::from_durable_run(&run_record, session_id, TrustedProviderRole::Coder);
    freeze_provider_execution(
        &identity,
        ProviderExecutionProfile::HostCoding,
        None,
        format!("runtime_unit_{node_id}"),
        None,
    )
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonContext {
    pub workspace_root: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeProtocolStep {
    pub node_id: String,
    pub status: RuntimeStepStatus,
    pub node_specific_fields: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeStepStatus {
    Completed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeUnitResult {
    pub protocol_steps: Vec<RuntimeProtocolStep>,
    pub produced_artifact_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeUnitError {
    pub code: String,
    pub message: String,
}

pub trait RuntimeUnit {
    fn unit_id(&self) -> &'static str;

    fn covered_protocol_nodes(&self) -> Vec<&'static str>;

    fn execute(
        &self,
        input: CanonicalNodeInput,
        ctx: &DaemonContext,
    ) -> impl Future<Output = Result<RuntimeUnitResult, RuntimeUnitError>> + Send;
}

pub fn completed_step(node_id: &str) -> RuntimeProtocolStep {
    RuntimeProtocolStep {
        node_id: node_id.to_string(),
        status: RuntimeStepStatus::Completed,
        node_specific_fields: serde_json::json!({}),
    }
}

#[cfg(test)]
mod t11_trusted_carrier_tests {
    use super::freeze_runtime_unit_execution;
    use crate::cross_cutting::provider_execution::{
        ProviderExecutionProfile, TrustedProviderRole,
    };

    /// 批2 Task 11（硬约束1）：runtime unit 受信载体冻结 Coder/HostCoding
    /// 合同（T10 裁定 legacy Direct 形态 identity）；durable 身份事实缺失
    /// 在 run 之前零副作用拒绝。
    #[test]
    fn runtime_unit_carrier_freezes_coder_host_coding_contract() {
        let contract = freeze_runtime_unit_execution(
            "session_t11_0001",
            "N16",
            crate::protocol::contracts::ProviderType::Codex,
            crate::protocol::contracts::RuntimeRole::Executor,
            "run_t11_0001",
        )
        .expect("runtime unit carrier freezes the trusted contract");
        assert_eq!(contract.duty(), TrustedProviderRole::Coder);
        assert_eq!(contract.profile(), ProviderExecutionProfile::HostCoding);
        assert_eq!(
            contract.author_policy_digest(),
            None,
            "Coder duty carries no author discipline"
        );
        assert_eq!(contract.review_window_id(), None);
    }

    #[test]
    fn runtime_unit_carrier_rejects_missing_identity_facts() {
        assert!(
            freeze_runtime_unit_execution(
                "  ",
                "N16",
                crate::protocol::contracts::ProviderType::Codex,
                crate::protocol::contracts::RuntimeRole::Executor,
                "run_t11_0002",
            )
            .is_err(),
            "missing durable session identity must fail closed before the run"
        );
        assert!(
            freeze_runtime_unit_execution(
                "session_t11_0001",
                "N18",
                crate::protocol::contracts::ProviderType::Codex,
                crate::protocol::contracts::RuntimeRole::Reviewer,
                "",
            )
            .is_err(),
            "missing durable run identity must fail closed before the run"
        );
    }
}
