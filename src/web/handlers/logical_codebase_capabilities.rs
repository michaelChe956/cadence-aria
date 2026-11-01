//! add-provider-revalidate-probe：LC provider capability 的用户显式重
//! 验证 HTTP 面（canonical v1.3 路由）。
//!
//! - `GET /logical-codebases/{lc_id}/capabilities`：只读投影四家 durable
//!   capability 行（零探针、零写入）。
//! - `POST /logical-codebases/{lc_id}/capability-revalidate`：对该
//!   provider 执行一次真实现场探针并原子导入 durable Confirmed（产品层
//!   `ProviderCapabilityRevalidateService` owner；同步等待——与 aggregate
//!   index rebuild 分钟级同步动作同先例）。全行已验证同版本秒回
//!   `already_confirmed`；Fake/未知 provider 稳定 422；探针失败如实
//!   `provider_capability_probe_failed`。

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use super::support::{product_app_paths, product_store_api_error, require_logical_codebase};
use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use crate::product::logical_codebase::SessionPolicyAction;
use crate::product::logical_codebase::provider_capability_revalidate::{
    ProviderCapabilityProviderSnapshot, ProviderCapabilityRevalidateService, RevalidateError,
    RevalidateOutcome, revalidate_action_text,
};
use crate::product::models::ProviderName;
use crate::web::error::{ApiError, ApiResult};
use crate::web::state::WebAppState;

/// 请求体：`provider_type`（四家冻结名之外一律 unsupported，不冒充探测）。
#[derive(Debug, serde::Deserialize)]
pub struct CapabilityRevalidateRequestDto {
    pub provider_type: String,
}

/// capability 重验证服务装配：state 注入（it_web fake seam）优先，缺省按
/// 请求 paths 构造真实服务（真实 `cli --version` 版本源 + 真实
/// `run_cli_boundary_probe` runner）。
fn capability_service(state: &WebAppState) -> ProviderCapabilityRevalidateService {
    state
        .capability_revalidate_service
        .as_ref()
        .map(std::sync::Arc::as_ref)
        .cloned()
        .unwrap_or_else(|| ProviderCapabilityRevalidateService::new(product_app_paths(state)))
}

fn evidence_text(evidence: &ProviderCapabilityEvidence) -> &'static str {
    match evidence {
        ProviderCapabilityEvidence::Confirmed => "confirmed",
        ProviderCapabilityEvidence::Denied { .. } => "denied",
        ProviderCapabilityEvidence::Unknown => "unknown",
    }
}

fn action_text(action: SessionPolicyAction) -> &'static str {
    revalidate_action_text(action)
}

/// provider_type 稳定名（与 capability store 的 DTO 冻结名一致）。
fn provider_type_name(snapshot: &ProviderCapabilityProviderSnapshot) -> &'static str {
    match snapshot.provider_type {
        crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode => {
            "claude_code"
        }
        crate::product::logical_codebase::provider_gateway::ProviderRefType::Codex => "codex",
        crate::product::logical_codebase::provider_gateway::ProviderRefType::Pi => "pi",
        crate::product::logical_codebase::provider_gateway::ProviderRefType::KimiCode => {
            "kimi_code"
        }
    }
}

/// GET `/logical-codebases/{lc_id}/capabilities`：只读 durable 投影。
pub async fn get_lc_capabilities(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    require_logical_codebase(&paths, &project_id, &logical_codebase_id)?;
    let providers = capability_service(&state)
        .capability_snapshot(&project_id, &logical_codebase_id)
        .map_err(product_store_api_error)?;
    let providers: Vec<Value> = providers
        .iter()
        .map(|snapshot| {
            json!({
                "provider_type": provider_type_name(snapshot),
                "cli_program": snapshot.cli_program,
                "version": snapshot.version,
                "probed_at": snapshot.probed_at,
                "probe_artifact_ref": snapshot.probe_artifact_ref,
                "rows": snapshot.rows.iter().map(|row| json!({
                    "action": action_text(row.action),
                    "launch": evidence_text(&row.launch),
                    "resume": evidence_text(&row.resume),
                    "write_boundary": evidence_text(&row.write_boundary),
                    "evidence_ref": row.evidence_ref,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(Json(json!({ "providers": providers })).into_response())
}

/// POST `/logical-codebases/{lc_id}/capability-revalidate`：显式重验证。
pub async fn post_lc_capability_revalidate(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
    Json(request): Json<CapabilityRevalidateRequestDto>,
) -> ApiResult<Response> {
    let provider_type = request.provider_type.trim();
    let provider = match provider_type {
        "claude_code" => ProviderName::ClaudeCode,
        "codex" => ProviderName::Codex,
        "pi" => ProviderName::Pi,
        "kimi_code" => ProviderName::KimiCode,
        "" => {
            return Err(ApiError::validation(
                "provider_capability_probe_unsupported",
                "provider_type must be a non-empty string",
            ));
        }
        other => {
            return Err(ApiError::validation(
                "provider_capability_probe_unsupported",
                format!("provider {other:?} has no real probe channel"),
            ));
        }
    };
    let paths = product_app_paths(&state);
    require_logical_codebase(&paths, &project_id, &logical_codebase_id)?;
    let service = capability_service(&state);
    match service
        .revalidate(&project_id, &logical_codebase_id, provider)
        .await
    {
        Ok(outcome) => Ok((
            StatusCode::OK,
            Json(match outcome {
                RevalidateOutcome::AlreadyConfirmed { version } => json!({
                    "provider_type": provider_type,
                    "outcome": "already_confirmed",
                    "version": version,
                }),
                RevalidateOutcome::Revalidated {
                    version,
                    imported_actions,
                } => json!({
                    "provider_type": provider_type,
                    "outcome": "revalidated",
                    "version": version,
                    "imported_actions": imported_actions
                        .iter()
                        .map(|action| action_text(*action))
                        .collect::<Vec<_>>(),
                }),
            }),
        )
            .into_response()),
        Err(RevalidateError::Unsupported { provider }) => Err(ApiError::validation(
            "provider_capability_probe_unsupported",
            format!("provider {provider} has no real probe channel"),
        )),
        Err(RevalidateError::ProbeFailed { action, detail }) => Err(ApiError::runtime(
            "provider_capability_probe_failed",
            "provider capability revalidation probe failed; no Confirmed was fabricated",
            json!({
                "provider_type": provider_type,
                "action": action.map(action_text),
                "detail": detail,
            }),
        )),
        Err(RevalidateError::Store(error)) => Err(product_store_api_error(error)),
    }
}
