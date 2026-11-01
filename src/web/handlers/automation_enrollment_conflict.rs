//! `automation_enrollment_conflict`（409）的 details 构造（批 2 后续，
//! owner 2026-10-25 真机「终止整个自动化」死路根修）。
//!
//! 契约面：details 是 **additive 扩展**——`current_revision` 保持既有语义与
//! 位置，终止路径额外带上 `enrollment_id` / `plan_id` / `session_id`（可得
//! 时），前端据此把确认面身份刷新到服务端当前代再让用户确认一次；旧客户端
//! 忽略未知字段即可，零破坏。
//!
//! 从 `automation_enrollment.rs` 净搬出：该文件已在 large_file_guard 登记表
//! 内（禁止继续增长），冲突构造与新逻辑统一放这里。

use serde_json::{Value, json};

use crate::product::models::automation::EnrollmentError;
use crate::web::error::ApiError;

use super::automation_enrollment::{AutomationPlanTerminationIdentity, enrollment_api_error};

/// 既有冲突构造（details 只有 `current_revision`；PUT/绑定/续链路径语义不变）。
pub(crate) fn enrollment_conflict(current_revision: Option<u64>, message: &str) -> ApiError {
    ApiError::validation_with_details(
        "automation_enrollment_conflict",
        message,
        json!({ "current_revision": current_revision }),
    )
}

/// 终止身份冲突的 details（additive）：既有 `current_revision` + 刷新确认面
/// 所需的 enrollment/plan/session 身份。plan/session 未绑定时为 null。
pub(crate) fn termination_identity_conflict_details(
    identity: &AutomationPlanTerminationIdentity,
    current_revision: u64,
) -> Value {
    json!({
        "current_revision": current_revision,
        "enrollment_id": identity.enrollment_id.clone(),
        "plan_id": identity.plan_id.clone(),
        "session_id": identity.session_id.clone(),
    })
}

/// 终止端点的身份互证失败（expected enrollment/binding 与当前代不符）：
/// 409 但 details 带全身份，前端刷新确认面后可继续。
pub(crate) fn termination_identity_conflict(
    identity: &AutomationPlanTerminationIdentity,
    message: &str,
) -> ApiError {
    ApiError::validation_with_details(
        "automation_enrollment_conflict",
        message,
        termination_identity_conflict_details(identity, identity.policy_revision),
    )
}

/// 终止 store CAS 错误：revision 漂移（enrollment 被反复重开）仍是 409，
/// 但 message 与 details 都带上服务端当前 revision 并补全身份——前端据此
/// 刷新确认面再确认一次，而不是只能停在错误提示上。
pub(crate) fn termination_api_error(
    error: EnrollmentError,
    identity: &AutomationPlanTerminationIdentity,
) -> ApiError {
    match error {
        EnrollmentError::Conflict { current_revision } => {
            let revision = current_revision.unwrap_or(identity.policy_revision);
            ApiError::validation_with_details(
                "automation_enrollment_conflict",
                format!("automation enrollment revision conflict (current revision {revision})"),
                termination_identity_conflict_details(identity, revision),
            )
        }
        other => enrollment_api_error(other),
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    use crate::product::work_item_plan_policy::RunPolicy;
    use crate::web::handlers::automation_enrollment_test_support::{
        Fixture, ISSUE_ID, PROJECT_ID, create_plan_and_session, enrollment_body, post_binding,
        put_enrollment, response_json, seed_fixture,
    };

    /// 统一终止 REST 的 POST helper（与 automation_enrollment.rs 测试同形）。
    async fn post_terminate(
        app: &axum::Router,
        body: &serde_json::Value,
    ) -> axum::http::Response<Body> {
        app.clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment/terminate"
                    ))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    /// 播种到「已 enable + 已绑定 plan/session」的状态。`Fixture` 必须随结果
    /// 返回并在用例内存活——它持有 TempDir，先 drop 会让后续请求读到空工作区
    /// （表现为 `issue_not_found`）。绑定会推进 policy_revision，故返回绑定后的值。
    async fn seed_bound_plan() -> (
        Fixture,
        axum::Router,
        serde_json::Value,
        u64,
        String,
        String,
    ) {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(enable.status(), StatusCode::OK);
        let enrollment = response_json(enable).await;
        let revision = enrollment["policy_revision"].as_u64().unwrap();
        let (plan_id, session_id) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let bound = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": plan_id,
                "session_id": session_id,
            }),
        )
        .await;
        assert_eq!(bound.status(), StatusCode::OK);
        let bound_revision = response_json(bound).await["policy_revision"]
            .as_u64()
            .unwrap();
        (
            fixture,
            app,
            enrollment,
            bound_revision,
            plan_id,
            session_id,
        )
    }

    /// owner 2026-10-25 真机死路：页面持有旧 revision 确认终止 → 409 必须带
    /// `current_revision` + 完整身份（enrollment/plan/session）与可读 message，
    /// 前端才能刷新确认面让用户再确认一次。
    #[tokio::test]
    async fn terminate_revision_drift_conflict_details_carry_full_identity() {
        let (_fixture, app, enrollment, bound_revision, plan_id, session_id) =
            seed_bound_plan().await;

        let response = post_terminate(
            &app,
            &serde_json::json!({
                "command_id": "cmd-terminate-stale-revision",
                "expected_enrollment_id": enrollment["enrollment_id"],
                "expected_policy_revision": bound_revision - 1,
                "confirmed": true,
            }),
        )
        .await;

        let status = response.status();
        let payload = response_json(response).await;
        assert_eq!(status, StatusCode::CONFLICT, "{payload}");
        assert_eq!(payload["code"], "automation_enrollment_conflict");
        // 既有字段保持（additive 扩展，老字段位置/语义不变）。
        assert_eq!(payload["details"]["current_revision"], bound_revision);
        // 新增字段：刷新确认面所需的完整身份。
        assert_eq!(
            payload["details"]["enrollment_id"],
            enrollment["enrollment_id"]
        );
        assert_eq!(payload["details"]["plan_id"], plan_id.as_str());
        assert_eq!(payload["details"]["session_id"], session_id.as_str());
        // message 也写明当前 revision（不只有 details）。
        assert!(
            payload["message"]
                .as_str()
                .unwrap()
                .contains(&format!("current revision {bound_revision}")),
            "message 必须带 current_revision：{payload}"
        );
    }

    /// expected_enrollment_id 不符（页面绑到旧一代 enrollment）同样 409 带身份，
    /// 前端据此刷新；零副作用（enrollment 未终止）。
    #[tokio::test]
    async fn terminate_identity_mismatch_conflict_details_carry_full_identity() {
        let (_fixture, app, enrollment, bound_revision, plan_id, session_id) =
            seed_bound_plan().await;

        let response = post_terminate(
            &app,
            &serde_json::json!({
                "command_id": "cmd-terminate-stale-enrollment",
                "expected_enrollment_id": "enrollment_stale_generation",
                "expected_policy_revision": bound_revision,
                "confirmed": true,
            }),
        )
        .await;

        let status = response.status();
        let payload = response_json(response).await;
        assert_eq!(status, StatusCode::CONFLICT, "{payload}");
        assert_eq!(payload["code"], "automation_enrollment_conflict");
        assert_eq!(
            payload["details"]["enrollment_id"],
            enrollment["enrollment_id"]
        );
        assert_eq!(payload["details"]["plan_id"], plan_id.as_str());
        assert_eq!(payload["details"]["session_id"], session_id.as_str());
        assert_eq!(payload["details"]["current_revision"], bound_revision);
    }
}
