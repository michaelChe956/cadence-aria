//! pointer_publish_* 稳定码映射（两段式：错误 variant → 稳定码 → HTTP 状态）。
//!
//! 与 `gateway_error_mapping.rs` 同构：生产活跃路径把 `PointerPublishError` 归一为
//! 稳定码 `ApiError`；`web/error.rs` 的 HttpStatus 段负责「稳定码 → HTTP 状态」第二段
//! 集中映射。覆盖 Task 10 契约的 6 个稳定码：`pointer_publish_busy`(409)、
//! `pointer_conflict_unresolved`(409)、`pointer_push_failed`(503)、
//! `pointer_revoke_failed`(503)、`pointer_not_found`(404)、参数非法 422。

use serde_json::json;

use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::PointerPublishError;
use crate::web::error::ApiError;

/// `PointerPublishError` → 稳定码 `ApiError`。`Store`/`Git` 包裹的底层错误按
/// kind 分流到 pointer 稳定码或既有 store 稳定码。
pub(crate) fn pointer_publish_api_error(error: PointerPublishError) -> ApiError {
    match error {
        PointerPublishError::Busy(message) => {
            ApiError::runtime("pointer_publish_busy", message, json!({}))
        }
        PointerPublishError::NotFound(message) => {
            ApiError::runtime("pointer_not_found", message, json!({}))
        }
        PointerPublishError::ConflictUnresolved(message) => {
            ApiError::runtime("pointer_conflict_unresolved", message, json!({}))
        }
        PointerPublishError::PushFailed(message) => {
            ApiError::runtime("pointer_push_failed", message, json!({}))
        }
        PointerPublishError::RevokeFailed(message) => {
            ApiError::runtime("pointer_revoke_failed", message, json!({}))
        }
        PointerPublishError::Validation(message) => {
            ApiError::validation("invalid_pointer_request", message)
        }
        PointerPublishError::Store(store_error) => pointer_store_api_error(store_error),
        PointerPublishError::Git(git_error) => {
            ApiError::runtime("pointer_push_failed", git_error.to_string(), json!({}))
        }
    }
}

/// `ProductStoreError` → pointer 稳定码或既有 store 稳定码。pointer 分区自身的
/// NotFound/Conflict 归一为 `pointer_not_found`/`pointer_publish_busy`，避免与
/// 既有业务稳定码漂移。
pub(crate) fn pointer_store_api_error(error: ProductStoreError) -> ApiError {
    let code = match &error {
        ProductStoreError::NotFound { kind, .. }
            if *kind == "pointer_publication" || *kind == "pointer_publication_entry" =>
        {
            Some("pointer_not_found")
        }
        ProductStoreError::Conflict { kind, .. } if *kind == "pointer_publication" => {
            Some("pointer_publish_busy")
        }
        _ => None,
    };
    match code {
        Some(code) => ApiError::runtime(code, error.to_string(), json!({})),
        None => super::support::product_store_api_error(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    #[test]
    fn pointer_publish_error_codes_map_to_expected_http_status() {
        // Task 10 稳定码契约：busy/conflict 409、push/revoke 失败 503、not_found 404、
        // 参数非法 422。
        let cases = [
            (
                pointer_publish_api_error(PointerPublishError::Busy("busy".to_string())).code,
                StatusCode::CONFLICT,
            ),
            (
                pointer_publish_api_error(PointerPublishError::ConflictUnresolved(
                    "conflict".to_string(),
                ))
                .code,
                StatusCode::CONFLICT,
            ),
            (
                pointer_publish_api_error(PointerPublishError::PushFailed("push".to_string())).code,
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                pointer_publish_api_error(PointerPublishError::RevokeFailed("revoke".to_string()))
                    .code,
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                pointer_publish_api_error(PointerPublishError::NotFound("missing".to_string()))
                    .code,
                StatusCode::NOT_FOUND,
            ),
            (
                pointer_publish_api_error(PointerPublishError::Validation("bad".to_string())).code,
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
        ];

        for (code, expected) in cases {
            let response = ApiError::validation(code.clone(), "contract").into_response();
            assert_eq!(response.status(), expected, "{code} status mapping");
        }
    }

    #[test]
    fn pointer_store_not_found_and_conflict_map_to_pointer_stable_codes() {
        let not_found = pointer_store_api_error(ProductStoreError::NotFound {
            kind: "pointer_publication",
            id: "pub_1".to_string(),
        });
        assert_eq!(not_found.code, "pointer_not_found");

        let busy = pointer_store_api_error(ProductStoreError::Conflict {
            kind: "pointer_publication",
            id: "logical_codebase:lc_1".to_string(),
        });
        assert_eq!(busy.code, "pointer_publish_busy");
        assert_eq!(busy.into_response().status(), StatusCode::CONFLICT);
    }
}
