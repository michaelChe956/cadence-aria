//! Provider policy projection 的纯 DTO 与 projector 合同(Task 1a,REQ-LCG-03)。
//!
//! 投影(projection)是 gateway 从不可由调用方伪造的输入出发,为某次
//! provider/action 会话冻结的完整权限画像:provider/dialect/wire、exact
//! version、role、permission/tool/approval/sandbox、cwd/target/roots、
//! trust/config/MCP digest 与 boundary 证据引用。adapter 只能消费 projection,
//! 不能自造或改写;`ProviderProjectionInput` 字段私有、只能由 gateway 构造,
//! 是该不可伪造性的承载点。
//!
//! 阶段说明:本文件随 Task 1a 冻结 trait/DTO 形状;真实 provider projector
//! 实现归 Task 4/5(计划批次 D),digest 计算/校验归 gateway 装配(1b/1c)。

/// gateway 构造的投影输入。字段私有:只能由 gateway 装配,provider、
/// 调用方与测试 fixture 均不能自造投影输入。
///
/// 🔴 Task 1a 阶段 1 桩:冻结字段集(envelope、provider/action row、role、
/// permission/tool/MCP/config/trust/boundary 输入)在阶段 2 落地。
#[derive(Debug, Clone)]
pub struct ProviderProjectionInput {
    _reserved: (),
}

/// provider policy projector 合同:把 gateway 构造的输入投影为某次会话的
/// 固定权限画像。实现归 provider projector owner(Task 4/5);registry 以
/// `Arc<dyn ProviderPolicyProjector>` 原子保存。
pub trait ProviderPolicyProjector: Send + Sync {
    /// 计算投影。输入由 gateway 构造;投影字段固定只读,消费方只能经
    /// `pub(crate)` getter/view 读取。
    fn project(
        &self,
        input: &ProviderProjectionInput,
    ) -> Result<ProviderPolicyProjection, ProviderProjectionError>;
}

/// 投影结果:固定只读字段集(provider_type/provider_dialect/wire_dialect/
/// exact_version/action/role/permission_mode/tool_policy/approval_policy/
/// sandbox/working_directory/protocol_working_directory/target/readable_roots/
/// writable_roots/trust_digest/config_digest/mcp_bundle_digest/
/// boundary_evidence_ref/capability_projection_digest/projection_digest)。
///
/// 🔴 Task 1a 阶段 1 桩:字段与 `pub(crate)` getter/View 在阶段 2 冻结落地。
#[derive(Debug, Clone)]
pub struct ProviderPolicyProjection {
    _reserved: (),
}

/// 投影失败:不可伪造输入校验失败、provider 不支持该 action/role 的投影,
/// 或投影材料缺失时 fail-closed。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderProjectionError {
    /// 该 provider 尚未接入 projector(或投影能力未交付),一律拒绝。
    #[error("provider_projection_unsupported: {0}")]
    Unsupported(String),
    /// 投影输入不合法(缺字段/角色不匹配等)。
    #[error("provider_projection_invalid: {0}")]
    Invalid(String),
}

/// 未接入 projector 的显式拒绝实现:对一切 `project` 调用返回
/// `ProviderProjectionError::Unsupported`。
///
/// 1a 合同期占位(裁决 A1):`ProviderRegistry::register_gated` 冻结为
/// (name, adapter, projector, gate) 四参,而真实 provider projector 归
/// Task 4/5 交付;在此之前,生产装配(`state.rs::real_provider_registry`)
/// 以本类型占第四参,行为=显式拒绝(与 `start_validated`/`run_validated`
/// 默认 unsupported 只供未接入 adapter 拒绝同构)。1c-factory 落真实
/// projector 时整体替换,不留占位。
#[derive(Debug, Clone, Copy, Default)]
pub struct UnprovisionedProviderPolicyProjector;

impl ProviderPolicyProjector for UnprovisionedProviderPolicyProjector {
    fn project(
        &self,
        _input: &ProviderProjectionInput,
    ) -> Result<ProviderPolicyProjection, ProviderProjectionError> {
        Err(ProviderProjectionError::Unsupported(
            "provider policy projector 未接入(1a 合同期占位,等待 Task 4/5 真实 projector)"
                .to_string(),
        ))
    }
}
