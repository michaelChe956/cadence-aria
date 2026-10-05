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

/// r27 问题2 根修:SC plan 会话的成员锚(logical/checkout id 对)。
/// fresh 生成 turn(single_candidate.rs)与 SC 门修订 turn
/// (provider_run.rs HumanGateScManualRevision 臂)必须**同源**取锚——
/// 修订臂此前传 (None,None),`planning_request` 退 aggregate_root 锚,
/// envelope target 三元组与 fresh 轮漂移,9a 显式 resume 审计门
/// (`resume_with_lc_start_record` 五字段比对)按投影漂移恒拒
/// (r26 现场:"resume audit drifted or legacy record lacks lc
/// projection")。两调用点统一经本 helper 从同一 repository 解析取锚。
pub(crate) fn plan_member_anchor(
    repository: &crate::product::models::RepositoryRecord,
) -> (Option<String>, Option<String>) {
    (
        repository
            .logical_repository_id
            .as_ref()
            .map(|id| id.0.to_string()),
        repository
            .primary_checkout_id
            .as_ref()
            .map(|id| id.0.to_string()),
    )
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

/// Task 1b 段②:WS streaming Plan/split 的 run 身份绑定——caller 先 begin,
/// `start_work_item_plan_author` 消费它做 prepare 前 sink 绑定,成功 complete、
/// 失败 fail;retry 每次新 handle(计划冻结「WS Plan/split run identity」)。
pub(crate) struct PlanSplitRunContext {
    pub handle: crate::product::work_item_split_engine::parse::WorkItemSplitProviderRunHandle,
    lifecycle: crate::product::lifecycle_store::LifecycleStore,
}

/// begin handle:分配 run-bound 身份(sink 绑定与审计文件 key 同源)。
pub(crate) fn begin_plan_split_run(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    project_id: &str,
    issue_id: &str,
    provider: &crate::product::models::ProviderName,
    workspace_session_id: &str,
) -> Result<PlanSplitRunContext, String> {
    let handle = lifecycle
        .begin_work_item_split_provider_run(project_id, issue_id, provider, workspace_session_id)
        .map_err(|error| error.to_string())?;
    Ok(PlanSplitRunContext {
        handle,
        lifecycle: lifecycle.clone(),
    })
}

impl PlanSplitRunContext {
    /// prepare 前 run-bound audit 上下文(sink=LifecycleStore 生产 sink)。
    pub(crate) fn audit_context(
        &self,
    ) -> crate::product::logical_codebase::provider_gateway::ProviderLaunchAuditContext {
        crate::product::logical_codebase::provider_gateway::ProviderLaunchAuditContext {
            workspace_session_id: self.handle.workspace_session_id.clone(),
            role_run_seq: self.handle.role_run_seq,
            audit_sink: std::sync::Arc::new(self.lifecycle.clone()),
        }
    }

    /// 成功收口:complete 消费已有 handle,不再 `save_work_item_split_provider_run`。
    pub(crate) fn complete(
        &self,
        prompt: &str,
        structured_output: &serde_json::Value,
    ) -> Result<(), String> {
        self.lifecycle
            .complete_work_item_split_provider_run(&self.handle, prompt, structured_output)
            .map_err(|error| error.to_string())
    }

    /// 失败收口:handle 以 status=failed 落盘(reason 可观测)。
    pub(crate) fn fail(&self, reason: &str) -> Result<(), String> {
        self.lifecycle
            .fail_work_item_split_provider_run(&self.handle, reason)
            .map_err(|error| error.to_string())
    }
}

/// Task 1b 段②生产臂:Logical 启动才分配 split run 身份(begin→Some);
/// Legacy(直连 `provider.start`)保持 `None`,run 记录与 sink 绑定零变化。
/// begin 失败(durable 存储故障)fail-closed 上浮。
pub(crate) fn begin_plan_split_run_if_logical(
    launch: &PlanAuthorLaunch,
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    project_id: &str,
    issue_id: &str,
    provider: &ProviderName,
    workspace_session_id: &str,
) -> Result<Option<PlanSplitRunContext>, String> {
    match launch {
        PlanAuthorLaunch::Legacy => Ok(None),
        PlanAuthorLaunch::Logical(_) => begin_plan_split_run(
            lifecycle,
            project_id,
            issue_id,
            provider,
            workspace_session_id,
        )
        .map(Some),
    }
}

/// Task 1b 段②生产臂:驱动成功后的 run 收口——structured 输出可解析则
/// `complete`(存档该 run 的结构化产物),不可解析则 `fail`。收口落盘失败
/// 不阻断业务流(`tracing::warn`,与 `commit_rebuilt_snapshot_after_provider_start`
/// 同语义:split run 记录是审计身份,不是主产物)。
pub(crate) fn close_plan_split_run(
    run: Option<&PlanSplitRunContext>,
    prompt: &str,
    full_output: &str,
) {
    let Some(run) = run else { return };
    match crate::web::workspace_ws_handler::run::parse_work_item_split_structured_output(
        full_output,
    ) {
        Ok(structured_output) => {
            if let Err(error) = run.complete(prompt, &structured_output) {
                tracing::warn!(%error, "complete plan split provider run failed");
            }
        }
        Err(message) => {
            if let Err(error) = run.fail(&format!("structured output parse failed: {message}")) {
                tracing::warn!(%error, "fail plan split provider run failed");
            }
        }
    }
}

/// Task 1b 段②生产臂:Markdown 产物(无 structured sentinel,如 SC plan
/// markdown/human-gate 修订)的 run 收口——原始全文按 JSON 字符串存档。
pub(crate) fn close_plan_split_run_with_markdown(
    run: Option<&PlanSplitRunContext>,
    prompt: &str,
    full_output: &str,
) {
    let Some(run) = run else { return };
    let structured_output = serde_json::Value::String(full_output.to_string());
    if let Err(error) = run.complete(prompt, &structured_output) {
        tracing::warn!(%error, "complete plan split provider run failed");
    }
}

/// Task 1b 段②生产臂:驱动失败(run 未产出可用输出)的 run 收口。
pub(crate) fn fail_plan_split_run(run: Option<&PlanSplitRunContext>, reason: &str) {
    let Some(run) = run else { return };
    if let Err(error) = run.fail(reason) {
        tracing::warn!(%error, "fail plan split provider run failed");
    }
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
    plan_split_run: Option<&PlanSplitRunContext>,
) -> Result<ProviderSession, ProviderAdapterError> {
    let PlanAuthorLaunch::Logical(plan) = launch else {
        return provider.start(input, cancel).await;
    };
    let plan = *plan;
    // Task 2.8(REQ-PLN-03,planning snapshot 贯穿):input 的独立 cwd 重绑
    // envelope 冻结的 canonical root——cwd/target 分离贯穿正常 run(worktree=
    // 成员 checkout)与 B3 StaleContext 重建 run(worktree=rebuilt.cwd=root),
    // spawn 前复验恒以 envelope root 为准,`input.working_dir` 保持 target
    // 语义原样透传(与 2.1 author 链 `provider_drive.rs` 的重绑同型)。
    input.working_directory = Some(plan.validated.envelope().working_directory.clone());
    // Task 1b 段②:WS plan/split caller 携 run handle 时,经 gateway
    // `prepare_streaming_launch` 在 prepare 前绑定 run-bound sink(含无通用
    // tool_policy 角色)并以冻结接口组装 validated input;无 handle 的存量
    // 调用面保持原 validate 复用路径(后续 caller 迁移逐步收敛)。
    let validated_input = match plan_split_run {
        Some(run_ctx) => plan
            .launch
            .gateway
            .prepare_streaming_launch(
                input,
                plan.launch
                    .planning_request()
                    .map_err(map_gateway_error_to_adapter)?,
                run_ctx.audit_context(),
            )
            .map_err(map_gateway_error_to_adapter)?,
        None => ValidatedStreamingProviderInput::new(input, plan.validated),
    };

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
