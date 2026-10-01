//! T10a:聚合规划 author 启动点经 gateway 的统一 helper。
//!
//! 逻辑会话(已注入 `LogicalCodebaseProviderGateway`)的 planning author run 经
//! gateway `validate` + `start_streaming` 启动并留 audit;传统单仓/未注入 gateway
//! 时保持原 `provider.start` 路径(Legacy 零变化)。
//!
//! Task 2.8（REQ-PLN-01/07，planning snapshot 贯穿）——原 B 阶段 deferred 的
//! 「run cwd=成员 checkout」已裁决纠正:
//! - provider spawn cwd 恒为 gateway 冻结的 canonical 聚合根（manifest
//!   `provider_context_root`，双工厂 root assertion 保证与登记/聚合投影一致）；
//! - target 独立传递:resolved 逻辑身份(logical_repository_id + checkout_id)成对
//!   时用 `PolicyTarget::checkout`(带真实身份的成员 checkout 锚定,享受 git-dir
//!   identity 防护);否则用 `PolicyTarget::aggregate_root(target_worktree)`。
//!   禁止以 target worktree 覆盖 root cwd,也不得回退成员 cwd。
//!
//! T3(裁决 A):planning author 的 `ValidatedSessionLaunchPolicy` 前移到 prompt 构建
//! 之前 resolve(`resolve_plan_author_launch`),使 work_item_split_engine 的 outline/
//! draft prompt 能按 `RoutingReferenceContext` 注入 Logical 路由引用;已 resolve 的
//! policy 原样透传给 `start_work_item_plan_author` 复用,避免二次 validate。
//!
//! C-2:launch request 的 `ProviderRef` 由 `session.author_provider` 经集中映射
//! `ProviderRef::from_provider_name` 派生(fail-closed:Pi/KimiCode 等无 gateway
//! 真实 dialect 的 provider 在此显式失败,不再硬编码 ClaudeCode 静默替换)。
//! `cap_managed_snapshot` capability ref 约定不变;Codex 的 REQ-ENV-05 路由阻断
//! 仍由 gateway 路由级硬门施加。

use std::path::PathBuf;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::{
    PROVIDER_ERROR_STDERR_TAIL_BYTES, ProviderAdapterError,
};
use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use crate::cross_cutting::streaming_provider::{
    ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::cadence_skills::routing_reference::{
    RoutingReferenceContext, routing_reference_context_from_policy,
};
use crate::product::logical_codebase::{
    LogicalCodebaseProviderGateway, PolicyTarget, ProviderGatewayError, ProviderRef,
    SessionLaunchRequest, SessionPolicyAction, ValidatedSessionLaunchPolicy,
};
use crate::product::models::ProviderName;
use crate::product::workspace_engine::WorkspaceEngine;

/// 逻辑会话经 gateway 启动所需的已解析 launch 信息。
/// `Clone`：F2-B SC compile 教学重驱需在同一 run 内复用已 resolve 的 launch
/// （避免二次 gateway validate）；字段均为不可变快照，克隆无副作用。
#[derive(Clone)]
pub(crate) struct LogicalPlanLaunch {
    pub gateway: Arc<LogicalCodebaseProviderGateway>,
    pub project_id: String,
    /// provider spawn cwd：gateway 冻结的 canonical 聚合根（Task 2.8，
    /// REQ-PLN-07 唯一来源；禁止以成员 checkout/target worktree 覆盖）。
    pub working_dir: PathBuf,
    /// target worktree（成员 checkout，`session.repository_path` 注入）。
    /// cwd 与 target 显式分离；身份不可得时 target 锚仍取该路径。
    pub target_worktree: PathBuf,
    /// resolved 逻辑仓库身份(可选)。两者都有时用 checkout target,否则 aggregate_root。
    pub logical_repository_id: Option<String>,
    pub checkout_id: Option<String>,
    /// session 配置的 author provider(C-2:launch request provider ref 的唯一
    /// 来源,勿从 input.provider_type 或 UI 默认推导)。
    pub author_provider: ProviderName,
}

impl LogicalPlanLaunch {
    /// 组装 planning 只读 `SessionLaunchRequest`（cwd=root、target=成员锚定，
    /// 见文件头 Task 2.8 说明）;provider ref 由 `author_provider` 经集中映射派生,
    /// 不支持的 provider 显式返回 `UnsupportedCapability` 而非静默回退 Claude(C-2)。
    pub(crate) fn planning_request(&self) -> Result<SessionLaunchRequest, ProviderGatewayError> {
        let target = match (&self.logical_repository_id, &self.checkout_id) {
            (Some(logical_repository_id), Some(checkout_id)) => PolicyTarget::checkout(
                logical_repository_id.clone(),
                checkout_id.clone(),
                self.target_worktree.clone(),
            ),
            _ => PolicyTarget::aggregate_root(self.target_worktree.clone()),
        };

        Ok(SessionLaunchRequest {
            project_id: self.project_id.clone(),
            provider: ProviderRef::from_provider_name(
                &self.author_provider,
                "cap_managed_snapshot",
            )?,
            action: SessionPolicyAction::PlanningReadOnly,
            target,
            // Task 2.8（REQ-PLN-01/07）：cwd 恒为 canonical root（≠成员
            // target），由结构体字面量显式提供——`SessionLaunchRequest::planning`
            // 的 cwd==target 默认仅适用单仓形态。
            working_directory: self.working_dir.clone(),
            readable_roots: vec![self.working_dir.clone()],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        })
    }

    /// 经 gateway 校验并冻结为 `ValidatedSessionLaunchPolicy`。
    pub(crate) fn validate(&self) -> Result<ValidatedSessionLaunchPolicy, ProviderGatewayError> {
        self.gateway.validate(self.planning_request()?)
    }
}

/// 已冻结政策的逻辑会话启动:launch + validated policy 成对保存,供 prompt 构建
/// (取 `RoutingReferenceContext::Logical`)与 provider 启动(复用 validated policy)共用。
#[derive(Clone)]
pub(crate) struct ValidatedPlanLaunch {
    pub launch: LogicalPlanLaunch,
    pub validated: ValidatedSessionLaunchPolicy,
}

/// planning author 启动选择:Legacy(直接 `provider.start`)或 Logical(经 gateway)。
#[derive(Clone)]
pub(crate) enum PlanAuthorLaunch {
    Legacy,
    Logical(Box<ValidatedPlanLaunch>),
}

impl PlanAuthorLaunch {
    /// 由此启动派生 prompt 注入用的路由引用上下文:Legacy 用 `Legacy`,
    /// Logical 用 validated policy 的 envelope 字段构造 `LogicalPolicyReference`。
    pub(crate) fn routing_context(&self) -> RoutingReferenceContext {
        match self {
            PlanAuthorLaunch::Legacy => RoutingReferenceContext::Legacy,
            PlanAuthorLaunch::Logical(plan) => {
                routing_reference_context_from_policy(&plan.validated)
            }
        }
    }
}

/// 组装逻辑会话 launch(gateway 已注入时);非逻辑会话/未注入 gateway 时 `None`。
/// Task 2.8：cwd 取 gateway 冻结的 canonical root（`authority_root()`，与
/// manifest/登记投影 canonical 一致由双工厂 assertion 保证），成员 checkout
/// （`logical_planning_launch` 的 repository_path）只作 target 锚。
pub(crate) fn logical_plan_launch_for(
    engine: &WorkspaceEngine,
    logical_repository_id: Option<String>,
    checkout_id: Option<String>,
) -> Option<LogicalPlanLaunch> {
    let gateway = engine.logical_provider_gateway()?;
    let (project_id, target_worktree) = engine.logical_planning_launch()?;
    Some(LogicalPlanLaunch {
        // Task 2.8：cwd 唯一来源是 gateway 冻结的 canonical root——不读
        // session.repository_path（成员 checkout 只作 target），杜绝成员 cwd
        // fallback（REQ-PLN-07）。
        working_dir: gateway.authority_root().to_path_buf(),
        gateway,
        project_id,
        target_worktree,
        logical_repository_id,
        checkout_id,
        // C-2:provider 身份唯一来源于 session 配置。
        author_provider: engine.session().author_provider.clone(),
    })
}

/// 在 prompt 构建之前 resolve 出 planning author 启动:非逻辑会话 → `Legacy`;
/// 逻辑会话 → 经 gateway `validate` 冻结 policy 并返回 `Logical`。
/// gateway 校验失败映射为 `ProviderAdapterError`,与启动路径的错误形态一致。
pub(crate) fn resolve_plan_author_launch(
    engine: &WorkspaceEngine,
    logical_repository_id: Option<String>,
    checkout_id: Option<String>,
) -> Result<PlanAuthorLaunch, ProviderAdapterError> {
    let Some(launch) = logical_plan_launch_for(engine, logical_repository_id, checkout_id) else {
        return Ok(PlanAuthorLaunch::Legacy);
    };
    let validated = launch.validate().map_err(map_gateway_error_to_adapter)?;
    Ok(PlanAuthorLaunch::Logical(Box::new(ValidatedPlanLaunch {
        launch,
        validated,
    })))
}

/// 逻辑会话经 gateway 启动,否则原 `provider.start`。返回 `ProviderSession`。
///
/// `launch` 为 `Logical` 时复用已 resolve 的 validated policy 经
/// `gateway.start_streaming` 启动并留 audit;gateway 错误映射为
/// `ProviderAdapterError`(与 `drive_author_provider_session_via_gateway` 相同形态),
/// 使调用点后续对 `Err` 的处理方式与直接 `provider.start` 完全一致。
/// `launch` 为 `Legacy` 时原样透传 `provider.start(input, cancel)`(Legacy 零变化)。
///
pub(crate) async fn start_work_item_plan_author(
    launch: PlanAuthorLaunch,
    provider: Arc<dyn StreamingProviderAdapter>,
    mut input: StreamingProviderInput,
    cancel: CancellationToken,
) -> Result<ProviderSession, ProviderAdapterError> {
    let PlanAuthorLaunch::Logical(plan) = launch else {
        return provider.start(input, cancel).await;
    };
    let plan = *plan;

    // Task 2.8（REQ-PLN-03，planning snapshot 贯穿）：input 的独立 cwd 重绑
    // envelope 冻结的 canonical root——cwd/target 分离贯穿正常 run（worktree=
    // 成员 checkout）与 B3 StaleContext 重建 run（worktree=rebuilt.cwd=root），
    // spawn 前复验恒以 envelope root 为准，`input.working_dir` 保持 target
    // 语义原样透传（与 2.1 author 链 `provider_drive.rs` 的重绑同型）。
    input.working_directory = Some(plan.validated.envelope().working_directory.clone());
    let validated_input = ValidatedStreamingProviderInput::new(input, plan.validated);

    plan.launch
        .gateway
        .start_streaming(validated_input, cancel)
        .await
        .map_err(map_gateway_error_to_adapter)
}

fn map_gateway_error_to_adapter(error: ProviderGatewayError) -> ProviderAdapterError {
    // 诊断直通（claude×轻 握手谜团第 2 轮）:Adapter 变体不再丢弃 stderr 字段——
    // stderr 尾部（有界）并入 details（随驱动 Err → EngineEvent::Error → WS error
    // 消息上浮），stderr 字段同源保留；其余 gateway 校验错误维持 Display 文案不变。
    match &error {
        ProviderGatewayError::Adapter(inner) => {
            let mut mapped = ProviderAdapterError::provider_unavailable(error.to_string());
            ProviderAdapterError::append_bounded_stderr_tail(
                &mut mapped.details,
                &inner.stderr,
                PROVIDER_ERROR_STDERR_TAIL_BYTES,
            );
            mapped.stderr = inner.stderr.clone();
            mapped
        }
        _ => ProviderAdapterError::provider_unavailable(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 诊断直通（claude×轻 握手谜团第 2 轮）：workitem author 启动路径的 gateway
    /// 错误映射同样不得丢弃 adapter stderr——stderr 尾部（有界）并入 details，
    /// stderr 字段同源保留（随驱动 Err → EngineEvent::Error → WS error 消息上浮）。
    #[test]
    fn gateway_error_mapping_carries_bounded_stderr_tail() {
        let stderr_head = "NOISE_HEAD_MARKER".to_string() + &"c".repeat(700);
        let stderr = format!("{stderr_head}\nSENTINEL_WS_GW_STDERR_TAIL");
        let inner = ProviderAdapterError::parse_error(
            "claude policy session: handshake failed: claude policy handshake cancelled",
            String::new(),
            stderr,
        );
        let mapped = map_gateway_error_to_adapter(ProviderGatewayError::Adapter(inner));
        assert!(
            mapped.details.contains("SENTINEL_WS_GW_STDERR_TAIL"),
            "gateway 映射应携带 adapter stderr 尾部，got: {}",
            mapped.details
        );
        assert!(
            !mapped.details.contains("NOISE_HEAD_MARKER"),
            "stderr 尾部应有界（丢头保尾），got: {}",
            mapped.details
        );
        assert!(
            mapped.stderr.contains("SENTINEL_WS_GW_STDERR_TAIL"),
            "stderr 字段应同源保留，got: {}",
            mapped.stderr
        );
    }
}
