//! T10a:聚合规划 author 启动点经 gateway 的统一 helper。
//!
//! 逻辑会话(已注入 `LogicalCodebaseProviderGateway`)的 planning author run 经
//! gateway `validate` + `start_streaming` 启动并留 audit;传统单仓/未注入 gateway
//! 时保持原 `provider.start` 路径(Legacy 零变化)。
//!
//! 关于 target 语义(controller 裁决 3,deferred 说明):
//! - 正常逻辑 WorkItemPlan run 的 cwd 是选中成员 checkout(不是 `provider_context_root`
//!   聚合根)——这是 B 阶段既有行为。gateway spawn 前复验强制 `input.working_dir ==
//!   target.worktree`,因此 launch target 必须锚定 run 的实际 working_dir。
//! - 能便捷取得 resolved 逻辑仓库身份(logical_repository_id + checkout_id)时用
//!   `PolicyTarget::checkout`(带真实身份的成员 checkout 锚定,享受 git-dir identity
//!   防护);否则用 `PolicyTarget::aggregate_root(working_dir)`。
//! - 「聚合根只读启动」的契约语义偏差是 B 阶段遗留,不在 T10a 范围纠正,记 deferred。
//!
//! T3(裁决 A):planning author 的 `ValidatedSessionLaunchPolicy` 前移到 prompt 构建
//! 之前 resolve(`resolve_plan_author_launch`),使 work_item_split_engine 的 outline/
//! draft prompt 能按 `RoutingReferenceContext` 注入 Logical 路由引用;已 resolve 的
//! policy 原样透传给 `start_work_item_plan_author` 复用,避免二次 validate。

use std::path::PathBuf;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
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
use crate::product::workspace_engine::WorkspaceEngine;

/// 逻辑会话经 gateway 启动所需的已解析 launch 信息。
pub(crate) struct LogicalPlanLaunch {
    pub gateway: Arc<LogicalCodebaseProviderGateway>,
    pub project_id: String,
    /// run 的实际 cwd(也是 `input.working_dir`)。gateway target 必须等于它。
    pub working_dir: PathBuf,
    /// resolved 逻辑仓库身份(可选)。两者都有时用 checkout target,否则 aggregate_root。
    pub logical_repository_id: Option<String>,
    pub checkout_id: Option<String>,
}

impl LogicalPlanLaunch {
    /// 组装 planning 只读 `SessionLaunchRequest`。target 语义见文件头说明。
    pub(crate) fn planning_request(&self) -> SessionLaunchRequest {
        let target = match (&self.logical_repository_id, &self.checkout_id) {
            (Some(logical_repository_id), Some(checkout_id)) => PolicyTarget::checkout(
                logical_repository_id.clone(),
                checkout_id.clone(),
                self.working_dir.clone(),
            ),
            _ => PolicyTarget::aggregate_root(self.working_dir.clone()),
        };

        SessionLaunchRequest {
            project_id: self.project_id.clone(),
            provider: ProviderRef::claude_code("cap_managed_snapshot"),
            action: SessionPolicyAction::PlanningReadOnly,
            target,
            readable_roots: vec![self.working_dir.clone()],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        }
    }

    /// 经 gateway 校验并冻结为 `ValidatedSessionLaunchPolicy`。
    pub(crate) fn validate(&self) -> Result<ValidatedSessionLaunchPolicy, ProviderGatewayError> {
        self.gateway.validate(self.planning_request())
    }
}

/// 已冻结政策的逻辑会话启动:launch + validated policy 成对保存,供 prompt 构建
/// (取 `RoutingReferenceContext::Logical`)与 provider 启动(复用 validated policy)共用。
pub(crate) struct ValidatedPlanLaunch {
    pub launch: LogicalPlanLaunch,
    pub validated: ValidatedSessionLaunchPolicy,
}

/// planning author 启动选择:Legacy(直接 `provider.start`)或 Logical(经 gateway)。
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
pub(crate) fn logical_plan_launch_for(
    engine: &WorkspaceEngine,
    logical_repository_id: Option<String>,
    checkout_id: Option<String>,
) -> Option<LogicalPlanLaunch> {
    engine.logical_provider_gateway().and_then(|gateway| {
        engine
            .logical_planning_launch()
            .map(|(project_id, working_dir)| LogicalPlanLaunch {
                gateway,
                project_id,
                working_dir,
                logical_repository_id,
                checkout_id,
            })
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
pub(crate) async fn start_work_item_plan_author(
    launch: PlanAuthorLaunch,
    provider: Arc<dyn StreamingProviderAdapter>,
    input: StreamingProviderInput,
    cancel: CancellationToken,
) -> Result<ProviderSession, ProviderAdapterError> {
    let PlanAuthorLaunch::Logical(plan) = launch else {
        return provider.start(input, cancel).await;
    };
    let plan = *plan;

    let validated_input = ValidatedStreamingProviderInput::new(input, plan.validated);

    plan.launch
        .gateway
        .start_streaming(validated_input, cancel)
        .await
        .map_err(map_gateway_error_to_adapter)
}

fn map_gateway_error_to_adapter(error: ProviderGatewayError) -> ProviderAdapterError {
    ProviderAdapterError::provider_unavailable(error.to_string())
}
