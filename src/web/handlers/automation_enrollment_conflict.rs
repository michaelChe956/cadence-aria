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

use crate::product::models::automation::{ConflictKind, EnrollmentError};
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

/// 运行中改 payload（换 provider）的冲突（REQ-WIGA-SWAP-02 判别位类 1）：
/// 409 + 新稳定码 `automation_enrollment_payload_immutable`。
///
/// details 沿用既有 `enrollment_conflict` 的形状（`current_revision`）并
/// **追加** `kind` 判别位（旧字段不改名/不改型 ⇒ 老客户端零破坏）。
pub(crate) fn enrollment_payload_immutable(
    current_revision: Option<u64>,
    kind: ConflictKind,
) -> ApiError {
    ApiError::validation_with_details(
        "automation_enrollment_payload_immutable",
        "automation enrollment payload is immutable while running",
        json!({ "current_revision": current_revision, "kind": kind.as_str() }),
    )
}

/// 版本号过期的冲突（REQ-WIGA-SWAP-02 判别位类 2）：409 + 新稳定码
/// `automation_enrollment_revision_stale`（details 形状同上）。
pub(crate) fn enrollment_revision_stale(
    current_revision: Option<u64>,
    kind: ConflictKind,
) -> ApiError {
    ApiError::validation_with_details(
        "automation_enrollment_revision_stale",
        "automation enrollment revision is stale",
        json!({ "current_revision": current_revision, "kind": kind.as_str() }),
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
        EnrollmentError::Conflict {
            current_revision, ..
        } => {
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

    // ─── REQ-WIGA-SWAP-02：判别位 + HTTP 分流（块1 片3）────────

    /// ⑦ 终止路径 mapper 的**码 / message / details 逐字不变**（回归锁）。
    ///
    /// 终止自愈逻辑（`automationTerminationConflict.tsx`）钉住旧码
    /// `automation_enrollment_conflict`；终止路径**不按判别位分流**——
    /// 即使判别位是 `RevisionStale`，对外仍是旧码 + 同一句 message +
    /// 同一组 details。
    #[test]
    fn termination_api_error_keeps_legacy_code_message_and_details() {
        use crate::product::models::automation::{ConflictKind, EnrollmentError};
        use crate::web::handlers::automation_enrollment::AutomationPlanTerminationIdentity;

        let identity = AutomationPlanTerminationIdentity {
            enrollment_id: "enrollment-1".to_string(),
            policy_revision: 7,
            plan_id: Some("plan-1".to_string()),
            session_id: Some("session-1".to_string()),
            binding_version: Some(1),
            terminated_at: None,
            enabled: true,
        };
        let api_error = super::termination_api_error(
            EnrollmentError::Conflict {
                current_revision: Some(9),
                kind: ConflictKind::RevisionStale,
            },
            &identity,
        );
        assert_eq!(api_error.code, "automation_enrollment_conflict");
        assert_eq!(
            api_error.message,
            "automation enrollment revision conflict (current revision 9)"
        );
        assert_eq!(
            api_error.details,
            serde_json::json!({
                "current_revision": 9,
                "enrollment_id": "enrollment-1",
                "plan_id": "plan-1",
                "session_id": "session-1",
            })
        );
    }

    /// ⑧ 命令写入侧**拆 if** 后两组语义可区分（判别位层面）：
    /// 终止路径「身份不符」vs「revision 过期」、rebind 路径「未启用」
    /// vs「revision 过期」必须落到不同判别位。
    #[tokio::test]
    async fn command_write_split_ifs_produce_distinguishable_conflict_kinds() {
        use crate::product::issue_automation_store::IssueAutomationStore;
        use crate::product::logical_codebase::EnrollmentTarget;
        use crate::product::models::automation::{
            ConflictKind, EnrollmentBindingIdentityInput, EnrollmentError, EnrollmentRebindRequest,
            EnrollmentSource, EnrollmentTerminationRequest,
        };
        use crate::product::models::provider::ProviderName;

        fn conflict_kind(error: EnrollmentError) -> ConflictKind {
            match error {
                EnrollmentError::Conflict { kind, .. } => kind,
                other => panic!("expected enrollment conflict, got {other:?}"),
            }
        }

        // ── 终止路径：身份不符 vs revision 过期 ──
        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(enable.status(), StatusCode::OK);
        let enrollment = response_json(enable).await;
        let revision = enrollment["policy_revision"].as_u64().unwrap();
        let enrollment_id = enrollment["enrollment_id"].as_str().unwrap().to_string();
        let store = IssueAutomationStore::new(fixture.paths.clone());

        let identity_mismatch = store
            .terminate_plan(
                PROJECT_ID,
                ISSUE_ID,
                EnrollmentTerminationRequest {
                    command_id: "cmd-terminate-identity-mismatch".to_string(),
                    expected_enrollment_id: "enrollment-not-current".to_string(),
                    expected_policy_revision: revision,
                    reason: "conflict kind probe".to_string(),
                },
            )
            .unwrap_err();
        let stale_revision = store
            .terminate_plan(
                PROJECT_ID,
                ISSUE_ID,
                EnrollmentTerminationRequest {
                    command_id: "cmd-terminate-stale-revision".to_string(),
                    expected_enrollment_id: enrollment_id.clone(),
                    expected_policy_revision: revision + 1,
                    reason: "conflict kind probe".to_string(),
                },
            )
            .unwrap_err();

        let identity_kind = conflict_kind(identity_mismatch);
        let stale_kind = conflict_kind(stale_revision);
        assert_ne!(
            identity_kind, stale_kind,
            "终止路径的「身份不符」与「revision 过期」必须落到不同判别位"
        );
        assert_eq!(stale_kind, ConflictKind::RevisionStale);

        // ── rebind 路径：未启用 vs revision 过期 ──
        let rebind_request =
            |command_id: &str, expected_policy_revision: u64| EnrollmentRebindRequest {
                command_id: command_id.to_string(),
                expected_policy_revision,
                expected_binding_version: 0,
                binding: EnrollmentBindingIdentityInput {
                    plan_id: "plan-kind-probe".to_string(),
                    session_id: "session-kind-probe".to_string(),
                    source: EnrollmentSource {
                        stories: vec![],
                        designs: vec![],
                    },
                    target: EnrollmentTarget::SingleRepository {
                        repository_id: "repository-kind-probe".to_string(),
                    },
                    author_provider: ProviderName::Fake,
                    reviewer_provider: ProviderName::Fake,
                },
                reason: "conflict kind probe".to_string(),
            };

        let rebind_stale = store
            .rebind(
                PROJECT_ID,
                ISSUE_ID,
                rebind_request("cmd-rebind-stale-revision", revision + 1),
            )
            .unwrap_err();
        assert_eq!(conflict_kind(rebind_stale), ConflictKind::RevisionStale);

        // 禁用态 enrollment：rebind 走「未启用」分支（不是版本过期）。
        let disabled_fixture = seed_fixture(1, true);
        let disabled_app = disabled_fixture.router();
        let disabled_enable =
            put_enrollment(&disabled_app, enrollment_body(&disabled_fixture, 1, 1)).await;
        assert_eq!(disabled_enable.status(), StatusCode::OK);
        let disabled_enrollment = response_json(disabled_enable).await;
        let disabled_revision = disabled_enrollment["policy_revision"].as_u64().unwrap();
        let disable = put_enrollment(
            &disabled_app,
            serde_json::json!({
                "expected_revision": disabled_revision,
                "command": {"type": "disable"},
            }),
        )
        .await;
        assert_eq!(disable.status(), StatusCode::OK);
        let current_revision = IssueAutomationStore::new(disabled_fixture.paths.clone())
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap()
            .policy_revision;
        let not_enabled = IssueAutomationStore::new(disabled_fixture.paths.clone())
            .rebind(
                PROJECT_ID,
                ISSUE_ID,
                rebind_request("cmd-rebind-not-enabled", current_revision),
            )
            .unwrap_err();
        let not_enabled_kind = conflict_kind(not_enabled);
        assert_ne!(
            not_enabled_kind,
            ConflictKind::RevisionStale,
            "rebind 的「未启用」与「revision 过期」必须落到不同判别位"
        );
    }
}
