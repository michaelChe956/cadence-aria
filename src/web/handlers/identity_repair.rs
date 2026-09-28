//! C4 Task 7：canonical identity repair HTTP 面。
//!
//! - `GET /logical-codebases/{lc_id}/identity-repair`：只读诊断（journal/
//!   registry/repos.json 低层事实；不经过普通成员列表与成功身份解析）。
//! - `POST /logical-codebases/{lc_id}/identity-repair`：唯一显式 repair
//!   动作入口；事实先落盘，再发布 `ProjectionUpdated`；被拒动作零写入。

use super::support::{product_app_paths, product_store_api_error};
use super::*;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::product::logical_codebase::identity_repair::{
    IdentityMappingSubmission, IdentityRepairActionKind, IdentityRepairActionRequest,
    IdentityRepairService,
};
use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
use crate::product::json_store::ProductStoreError;
use crate::web::error::{ApiError, ApiResult};
use crate::web::state::WebAppState;
use crate::web::types::{
    IdentityJournalDiagnosticDto, IdentityRepairActionRequestDto,
};

pub async fn get_lc_identity_repair(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    let diagnostic = IdentityRepairService::new(paths)
        .diagnostic(&project_id, &logical_codebase_id)
        .map_err(identity_repair_api_error)?;
    Ok((
        StatusCode::OK,
        Json(IdentityJournalDiagnosticDto::from(&diagnostic)),
    )
        .into_response())
}

pub async fn post_lc_identity_repair_action(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
    Json(request): Json<IdentityRepairActionRequestDto>,
) -> ApiResult<Response> {
    if request.command_id.trim().is_empty() {
        return Err(ApiError::validation(
            "invalid_command_id",
            "command_id must be a non-empty string",
        ));
    }
    let action = match request.action.as_str() {
        "continue_safe_prefix" => IdentityRepairActionKind::ContinueSafePrefix,
        "submit_mapping" => IdentityRepairActionKind::SubmitMapping,
        "revalidate" => IdentityRepairActionKind::Revalidate,
        other => {
            return Err(ApiError::validation(
                "identity_repair_invalid_action",
                format!("unknown identity repair action: {other}"),
            ));
        }
    };
    let mapping = match (&action, &request.mapping) {
        (IdentityRepairActionKind::SubmitMapping, None) => {
            return Err(ApiError::validation(
                "identity_repair_mapping_required",
                "submit_mapping requires a mapping submission",
            ));
        }
        (_, Some(_)) => {
            return Err(ApiError::validation(
                "identity_repair_mapping_unexpected",
                "mapping is only accepted for submit_mapping",
            ));
        }
        _ => None,
    };
    let mapping = match mapping {
        None => None,
        Some(submission) => Some(mapping_submission_from_dto(submission)?),
    };

    let service = IdentityRepairService::new(product_app_paths(&state));
    let diagnostic = service
        .apply(IdentityRepairActionRequest {
            command_id: request.command_id.clone(),
            project_id: project_id.clone(),
            logical_codebase_id: logical_codebase_id.clone(),
            expected_journal_updated_at: request.expected_journal_updated_at.clone(),
            action,
            mapping,
        })
        .map_err(identity_repair_api_error)?;

    // 事实已先落盘；EventHub 只承担通知/补读触发，发布失败不回滚。
    // Task 9：payload 与 bootstrap 通知同构（step/object/reason/next step），
    // repair 只作用于 identity 步骤，字段全部来自 durable diagnostic。
    state.events.publish(
        crate::web::events::WebEventType::ProjectionUpdated.as_str(),
        None,
        json!({
            "scope": "logical_codebase_identity_repair",
            "project_id": diagnostic.project_id,
            "logical_codebase_id": diagnostic.logical_codebase_id,
            "migration_id": diagnostic.migration_id,
            "phase": format!("{:?}", diagnostic.phase).to_lowercase(),
            "command_id": request.command_id,
            "conflicts": diagnostic.conflicts,
            "notice": {
                "key": format!(
                    "identity_repair:{}:{}",
                    diagnostic.migration_id,
                    diagnostic
                        .conflicts
                        .first()
                        .cloned()
                        .unwrap_or_else(|| format!("{:?}", diagnostic.phase).to_lowercase())
                ),
                "step": "identity",
                "object_id": diagnostic.migration_id,
                "reason_code": if diagnostic.phase
                    == crate::product::logical_codebase::IdentityMigrationPhase::Failed
                {
                    "identity_migration_failed"
                } else {
                    "identity_repair_applied"
                },
                "external_side_effect": "none",
                "allowed_actions": diagnostic
                    .allowed_actions
                    .iter()
                    .map(|action| match action {
                        crate::product::logical_codebase::identity_repair::IdentityRepairActionKind::ContinueSafePrefix => "continue_safe_prefix",
                        crate::product::logical_codebase::identity_repair::IdentityRepairActionKind::SubmitMapping => "submit_mapping",
                        crate::product::logical_codebase::identity_repair::IdentityRepairActionKind::Revalidate => "revalidate",
                    })
                    .collect::<Vec<_>>(),
                "next_step": "manifest_checkout",
            },
        }),
    );

    Ok((
        StatusCode::OK,
        Json(IdentityJournalDiagnosticDto::from(&diagnostic)),
    )
        .into_response())
}

fn mapping_submission_from_dto(
    submission: &crate::web::types::IdentityMappingSubmissionDto,
) -> ApiResult<IdentityMappingSubmission> {
    Ok(IdentityMappingSubmission {
        legacy_repository_id: submission.legacy_repository_id.clone(),
        source_identity_digest: submission.source_identity_digest.clone(),
        logical_repository_id: LogicalRepositoryId(
            submission
            .logical_repository_id
            .parse()
            .map_err(|_| invalid_mapping_uuid("logical_repository_id"))?,
        ),
        primary_checkout_id: RepositoryCheckoutId(
            submission
            .primary_checkout_id
            .parse()
            .map_err(|_| invalid_mapping_uuid("primary_checkout_id"))?,
        ),
        physical_repository_id: submission.physical_repository_id.clone(),
        idempotency_key: submission.idempotency_key.clone(),
    })
}

fn invalid_mapping_uuid(field: &str) -> ApiError {
    ApiError::validation(
        "identity_repair_invalid_mapping",
        format!("{field} must be a uuid"),
    )
}

/// repair 稳定码映射：journal 缺失 404、stale/冲突 409、语义拒绝 422。
fn identity_repair_api_error(error: ProductStoreError) -> ApiError {
    match error {
        ProductStoreError::NotFound {
            kind: "identity_migration_journal",
            id,
        } => ApiError::runtime(
            "identity_repair_journal_not_found",
            "no identity migration journal exists for this project",
            json!({ "project_id": id }),
        ),
        ProductStoreError::Conflict { kind, id }
            if kind.starts_with("identity_repair_") =>
        {
            match kind {
                "identity_repair_mapping_unknown_repository"
                | "identity_repair_mapping_digest_mismatch"
                | "identity_repair_mapping_physical_mismatch"
                | "identity_repair_mapping_key_mismatch"
                | "identity_repair_mapping_candidate_mismatch"
                | "identity_repair_mapping_no_candidate"
                | "identity_repair_mapping_required"
                | "identity_repair_invalid_command" => ApiError::runtime(
                    "identity_repair_rejected",
                    "identity repair action was rejected",
                    json!({ "kind": kind, "detail": id }),
                ),
                other => ApiError::runtime(
                    other,
                    "identity repair action conflicted with durable facts",
                    json!({ "detail": id }),
                ),
            }
        }
        other => product_store_api_error(other),
    }
}
