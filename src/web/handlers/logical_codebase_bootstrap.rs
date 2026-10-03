//! C4 Task 6：LC 冷启动 bootstrap 的 canonical HTTP 面。
//!
//! - `GET /logical-codebases/{lc_id}/bootstrap`：纯投影，只组合既有 durable
//!   facts（resolver 冻结 authority 后调用 projector），零写入。
//! - `POST /logical-codebases/{lc_id}/bootstrap/actions`：唯一显式动作入口，
//!   先落 durable 事实（service.apply → 既有 batch/initialization/index
//!   记录），再发布 `ProjectionUpdated` 通知；同 command 重放返回同一
//!   durable 结果。
//! - 旧 `/logical-codebase/bootstrap*` 路径保持为默认第一个 LC 的兼容
//!   alias，alias 与 canonical 走同一 projector/service。

use super::support::{
    default_logical_codebase_id, product_app_paths, product_store_api_error, resolve_lc_authority,
};
use super::*;

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::product::logical_codebase::bootstrap::LogicalCodebaseBootstrapProjector;
use crate::product::logical_codebase::bootstrap::LogicalCodebaseBootstrapService;
use crate::product::logical_codebase::{BootstrapActionError, BootstrapActionRequest};
use crate::web::error::ApiError;
use crate::web::state::{InitializationRunKey, WebAppState};
use crate::web::types::{
    BootstrapActionRequestDto, BootstrapActionResultDto, LogicalCodebaseBootstrapProjectionDto,
};

/// Legacy `/logical-codebase/bootstrap` compatibility alias for the project's
/// default first logical codebase.
pub async fn get_logical_codebase_bootstrap(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    get_bootstrap_projection_for_lc(&state, project_id, logical_codebase_id)
}

/// v1.3 canonical endpoint: the cold-start bootstrap projection is resolved
/// per logical codebase.
pub async fn get_lc_bootstrap_projection(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    get_bootstrap_projection_for_lc(&state, project_id, logical_codebase_id)
}

fn get_bootstrap_projection_for_lc(
    state: &WebAppState,
    project_id: String,
    logical_codebase_id: String,
) -> ApiResult<Response> {
    let paths = product_app_paths(state);
    // canonical 路由先经唯一 authority resolver 冻结 LC 身份（conflict
    // fail-closed），alias 经 default_logical_codebase_id 定位后同样进入
    // 同一 projector——两条路径不得有不同的 authority 语义。
    resolve_lc_authority(&paths, &project_id, &logical_codebase_id)?;
    let projection = LogicalCodebaseBootstrapProjector::new(paths)
        .project(&project_id, &logical_codebase_id)
        .map_err(product_store_api_error)?;
    Ok((
        StatusCode::OK,
        Json(LogicalCodebaseBootstrapProjectionDto::from(&projection)),
    )
        .into_response())
}

/// Legacy `/logical-codebase/bootstrap/actions` compatibility alias.
pub async fn post_logical_codebase_bootstrap_action(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<BootstrapActionRequestDto>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    post_bootstrap_action_for_lc(state, project_id, logical_codebase_id, request).await
}

/// v1.3 canonical endpoint: the single explicit bootstrap action surface.
pub async fn post_lc_bootstrap_action(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
    Json(request): Json<BootstrapActionRequestDto>,
) -> ApiResult<Response> {
    post_bootstrap_action_for_lc(state, project_id, logical_codebase_id, request).await
}

async fn post_bootstrap_action_for_lc(
    state: WebAppState,
    project_id: String,
    logical_codebase_id: String,
    request: BootstrapActionRequestDto,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    resolve_lc_authority(&paths, &project_id, &logical_codebase_id)?;
    if request.command_id.trim().is_empty() {
        return Err(ApiError::validation(
            "invalid_command_id",
            "command_id must be a non-empty string",
        ));
    }
    if request.expected_object_id.trim().is_empty() {
        return Err(ApiError::validation(
            "bootstrap_action_invalid_object",
            "expected_object_id must be a non-empty string",
        ));
    }

    let runs = state
        .aggregate_initialization_dependencies()
        .for_lc(logical_codebase_id.clone())
        .runs()
        .clone();
    // G3（终局关闸缺口）：注入 aggregate_index_active 步 Retry 的重建
    // 派发器（LC 隔离 index operation 的 build_with_command_id——同
    // command 幂等、revision 冲突 fail-closed），替代「仅查重放记录、
    // 找不到即 NotFound 恒 500」的断链。
    let index_operation = state
        .aggregate_initialization_dependencies()
        .for_lc(logical_codebase_id.clone())
        .index
        .clone();
    let service = LogicalCodebaseBootstrapService::new(paths)
        .with_member_index_run_probe(Arc::new(
            move |project_id: &str, lc_id: &str, operation_id: &str| {
                runs.is_active(&InitializationRunKey::aggregate(
                    project_id,
                    lc_id,
                    operation_id,
                ))
            },
        ))
        .with_aggregate_index_rebuild(Arc::new(
            move |project_id: &str, command_id: &str, expected_revision: u64| {
                index_operation.build_with_command_id(project_id, expected_revision, command_id)
            },
        ));
    let result = service
        .apply(BootstrapActionRequest {
            command_id: request.command_id.clone(),
            project_id: project_id.clone(),
            logical_codebase_id: logical_codebase_id.clone(),
            step: request.step,
            action: request.action,
            expected_revision: request.expected_revision,
            expected_object_id: request.expected_object_id.clone(),
        })
        .await
        .map_err(|error| match error {
            BootstrapActionError::Conflict { code, detail } => ApiError::runtime(
                code,
                "logical codebase bootstrap action was rejected",
                json!({
                    "project_id": project_id,
                    "logical_codebase_id": logical_codebase_id,
                    "detail": detail,
                }),
            ),
            BootstrapActionError::Store(error) => product_store_api_error(error),
        })?;
    // C4 Task 10（A03 续跑接缝）：member_index 的显式 Continue/Retry 被
    // service 接受（Failed → reopen，durable 状态 Running）后，由原
    // aggregate-initialization 编排链从 checkpoint 续跑——复用 create
    // handler 的 run-lease + spawn + detached index build 模式；已完成
    // provider turn 不重跑（execute_remaining 只执行未完成步骤）。
    // recover_interrupted 分支（落盘 Failed + Retry 等待面）不在其列——
    // 投影为 failed，续跑留给用户的下一次显式 Retry。
    let member_index_reopened = matches!(
        result.outcome,
        crate::product::logical_codebase::BootstrapActionOutcome::Accepted
    ) && request.step
        == crate::product::logical_codebase::LogicalCodebaseBootstrapStep::MemberIndex
        && result.projection.steps.iter().any(|step| {
            step.step == crate::product::logical_codebase::LogicalCodebaseBootstrapStep::MemberIndex
                && step.status
                    == crate::product::logical_codebase::LogicalCodebaseBootstrapStepStatus::Running
        });
    if member_index_reopened {
        let dependencies = state
            .aggregate_initialization_dependencies()
            .for_lc(logical_codebase_id.clone());
        let key = InitializationRunKey::aggregate(
            &project_id,
            &logical_codebase_id,
            &request.expected_object_id,
        );
        let operation_id_for_worker = request.expected_object_id.clone();
        if let Some(lease) = dependencies.runs().register(key) {
            let token = lease.cancellation_token();
            let coordinator = dependencies.coordinator.clone();
            let index = dependencies.index.clone();
            let project_id_for_worker = project_id.clone();
            tokio::spawn(async move {
                // 与 create handler 相同：lease 覆盖整个续跑执行。
                let _lease = lease;
                match coordinator
                    .execute_remaining(&project_id_for_worker, &operation_id_for_worker, token)
                    .await
                {
                    Ok(operation) => {
                        // 续跑完成后的首建/索引推进仍走既有 detached
                        // index build（失败独立可观测，不回滚初始化）。
                        let manifest_revision = operation.input.manifest_revision;
                        tokio::spawn(async move {
                            let build = tokio::task::spawn_blocking(move || {
                                index.build(&project_id_for_worker, manifest_revision)
                            })
                            .await;
                            match build {
                                Ok(Ok(_)) => {}
                                Ok(Err(error)) => {
                                    tracing::warn!(
                                        operation_id = %operation_id_for_worker,
                                        error = %error,
                                        "bootstrap resume aggregate index build stopped"
                                    );
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        operation_id = %operation_id_for_worker,
                                        error = %error,
                                        "bootstrap resume aggregate index worker panicked"
                                    );
                                }
                            }
                        });
                    }
                    Err(error) => {
                        tracing::warn!(
                            project_id = %project_id_for_worker,
                            operation_id = %operation_id_for_worker,
                            error = %error,
                            "bootstrap resume worker stopped"
                        );
                    }
                }
            });
        }
    }
    // 事实已先落盘；EventHub 只承担通知/补读触发，发布失败不回滚。
    // Task 9：payload 携带 durable notice 上下文（step/object/reason/next
    // step），全部从 action 后重新投影的 notices 派生——通知不宣称任何
    // 未持久化的成功。
    let acted_notice = result
        .projection
        .notices
        .iter()
        .find(|notice| notice.step == request.step)
        .or_else(|| result.projection.notices.first());
    state.events.publish(
        crate::web::events::WebEventType::ProjectionUpdated.as_str(),
        None,
        json!({
            "scope": "logical_codebase_bootstrap",
            "project_id": result.projection.project_id,
            "logical_codebase_id": result.projection.logical_codebase_id,
            "command_id": result.command_id,
            "outcome": match result.outcome {
                crate::product::logical_codebase::BootstrapActionOutcome::Accepted => "accepted",
                crate::product::logical_codebase::BootstrapActionOutcome::Replayed => "replayed",
                crate::product::logical_codebase::BootstrapActionOutcome::WaitingForHuman => {
                    "waiting_for_human"
                }
                crate::product::logical_codebase::BootstrapActionOutcome::Completed => "completed",
            },
            "planning_ready": result.projection.planning_ready,
            "notice": acted_notice.map(|notice| json!({
                "key": notice.key,
                "step": notice.step.as_str(),
                "object_id": notice.object_id,
                "reason_code": notice.reason_code,
                "external_side_effect": notice.external_side_effect,
                "allowed_actions": notice
                    .allowed_actions
                    .iter()
                    .map(|action| action.as_str())
                    .collect::<Vec<_>>(),
                "next_step": notice.next_step.map(|step| step.as_str()),
            })),
        }),
    );

    Ok((
        StatusCode::OK,
        Json(BootstrapActionResultDto::from(&result)),
    )
        .into_response())
}
