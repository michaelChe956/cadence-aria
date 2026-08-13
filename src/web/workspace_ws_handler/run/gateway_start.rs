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

use std::path::PathBuf;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use crate::cross_cutting::streaming_provider::{
    ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::logical_codebase::{
    LogicalCodebaseProviderGateway, PolicyTarget, ProviderGatewayError, ProviderRef,
    SessionLaunchRequest, SessionPolicyAction,
};

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

/// 逻辑会话经 gateway 启动,否则原 `provider.start`。返回 `ProviderSession`。
///
/// `logical` 为 `Some` 时组装 planning 只读 launch 请求,经
/// `gateway.validate` + `gateway.start_streaming` 启动;gateway 错误映射为
/// `ProviderAdapterError`(与 `drive_author_provider_session_via_gateway` 相同形态),
/// 使调用点后续对 `Err` 的处理方式与直接 `provider.start` 完全一致。
/// `logical` 为 `None` 时原样透传 `provider.start(input, cancel)`(Legacy 零变化)。
pub(crate) async fn start_work_item_plan_author(
    logical: Option<LogicalPlanLaunch>,
    provider: Arc<dyn StreamingProviderAdapter>,
    input: StreamingProviderInput,
    cancel: CancellationToken,
) -> Result<ProviderSession, ProviderAdapterError> {
    let Some(launch) = logical else {
        return provider.start(input, cancel).await;
    };

    let target = match (launch.logical_repository_id, launch.checkout_id) {
        (Some(logical_repository_id), Some(checkout_id)) => PolicyTarget::checkout(
            logical_repository_id,
            checkout_id,
            launch.working_dir.clone(),
        ),
        _ => PolicyTarget::aggregate_root(launch.working_dir.clone()),
    };

    let request = SessionLaunchRequest {
        project_id: launch.project_id,
        provider: ProviderRef::claude_code("cap_managed_snapshot"),
        action: SessionPolicyAction::PlanningReadOnly,
        target,
        readable_roots: vec![launch.working_dir.clone()],
        writable_roots: Vec::new(),
        config_artifact_ref: "sha256:managed-config-artifact".to_string(),
    };

    let validated = launch
        .gateway
        .validate(request)
        .map_err(map_gateway_error_to_adapter)?;

    let validated_input = ValidatedStreamingProviderInput::new(input, validated);

    launch
        .gateway
        .start_streaming(validated_input, cancel)
        .await
        .map_err(map_gateway_error_to_adapter)
}

fn map_gateway_error_to_adapter(error: ProviderGatewayError) -> ProviderAdapterError {
    ProviderAdapterError::provider_unavailable(error.to_string())
}
