//! `evidence_*` 稳定码映射（两段式：错误 variant → 稳定码 → HTTP 状态）。
//!
//! 与 `gateway_error_mapping.rs` 同构：生产活跃路径把 `EvidenceError` 归一为
//! 稳定码 `ApiError`；`web/error.rs` 的 HttpStatus 段负责「稳定码 → HTTP 状态」
//! 第二段集中映射。覆盖设计 §5.2 的 6 个稳定码 + `evidence_io`：
//! `evidence_unauthorized`(401)、`evidence_forbidden`(403)、
//! `evidence_not_available`(404)、`evidence_invalid_query`(422)、
//! `evidence_budget_exhausted`(429)、`evidence_query_failed`(503)、
//! `evidence_io`(500)。

use serde_json::json;

use crate::product::logical_codebase::evidence_index::EvidenceError;
use crate::web::error::ApiError;

/// `EvidenceError` → 稳定码前缀表（设计 §5.2）。
pub(crate) fn evidence_error_code(error: &EvidenceError) -> &'static str {
    match error {
        EvidenceError::Unauthorized => "evidence_unauthorized",
        EvidenceError::Forbidden => "evidence_forbidden",
        EvidenceError::NotAvailable => "evidence_not_available",
        EvidenceError::InvalidQuery { .. } => "evidence_invalid_query",
        EvidenceError::BudgetExhausted => "evidence_budget_exhausted",
        EvidenceError::QueryFailed { .. } => "evidence_query_failed",
        EvidenceError::Io { .. } => "evidence_io",
    }
}

/// `EvidenceError` → 稳定码 `ApiError`。第二段「稳定码 → HTTP 状态」在
/// `web/error.rs` 集中登记。
pub(crate) fn evidence_api_error(error: &EvidenceError) -> ApiError {
    match error {
        EvidenceError::InvalidQuery { reason } => {
            ApiError::validation(evidence_error_code(error), reason.clone())
        }
        EvidenceError::QueryFailed { code, message } => ApiError::runtime(
            evidence_error_code(error),
            message.clone(),
            json!({ "reason_code": code }),
        ),
        EvidenceError::Io { message } => {
            ApiError::runtime(evidence_error_code(error), message.clone(), json!({}))
        }
        _ => ApiError::runtime(evidence_error_code(error), error.to_string(), json!({})),
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    use super::*;

    #[test]
    fn web_evidence_error_codes_map_to_expected_http_status() {
        // 设计 §5.2 稳定码契约：401/403/404/422/429/503 + evidence_io → 500。
        let cases = [
            (
                evidence_api_error(&EvidenceError::Unauthorized).code,
                StatusCode::UNAUTHORIZED,
            ),
            (
                evidence_api_error(&EvidenceError::Forbidden).code,
                StatusCode::FORBIDDEN,
            ),
            (
                evidence_api_error(&EvidenceError::NotAvailable).code,
                StatusCode::NOT_FOUND,
            ),
            (
                evidence_api_error(&EvidenceError::InvalidQuery {
                    reason: "query symbol must not be empty".to_string(),
                })
                .code,
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                evidence_api_error(&EvidenceError::BudgetExhausted).code,
                StatusCode::TOO_MANY_REQUESTS,
            ),
            (
                evidence_api_error(&EvidenceError::QueryFailed {
                    code: "codegraph_query_invalid_json",
                    message: "bad json".to_string(),
                })
                .code,
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                evidence_api_error(&EvidenceError::Io {
                    message: "write failed".to_string(),
                })
                .code,
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];

        for (code, expected) in cases {
            let response = ApiError::validation(code.clone(), "contract").into_response();
            assert_eq!(response.status(), expected, "{code} status mapping");
        }
    }

    #[test]
    fn web_evidence_query_failed_carries_underlying_reason_code() {
        let error = evidence_api_error(&EvidenceError::QueryFailed {
            code: "evidence_acl_manifest_missing",
            message: "manifest missing".to_string(),
        });
        assert_eq!(error.code, "evidence_query_failed");
        assert_eq!(
            error.details["reason_code"],
            "evidence_acl_manifest_missing"
        );
    }
}
