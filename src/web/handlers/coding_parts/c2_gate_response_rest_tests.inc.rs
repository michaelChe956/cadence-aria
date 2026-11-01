/// C2 Task 12（REQ-CRO-06）：gate-responses REST 契约——同 command 同
/// payload 重放首次 durable 结果（不重复副作用）、同 command 异 payload
/// fail-closed"请刷新"、旧版本 Rejected 不改 attempt 不启动 provider；
/// 与 WS `GateResponse` 同一应用服务（`handle_blocked_gate_response`）。
#[cfg(test)]
mod c2_gate_response_rest_tests {
    use axum::extract::{Path, State};
    use axum::Json;

    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::CodingAttemptStore;
    use crate::product::coding_attempt_store::CreateCodingAttemptInput;
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::models::automation::OperationState;
    use crate::web::handlers::coding::scope::CodingAttemptRoutePath;
    use crate::web::handlers::coding::{
        CodingGateResponseRestRequest, post_coding_gate_response,
    };
    use crate::web::state::WebAppState;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    async fn fixture() -> (tempfile::TempDir, WebAppState, CodingAttemptStore, crate::product::coding_models::CodingExecutionAttempt) {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().to_path_buf();
        let state = WebAppState::new(
            root.clone(),
            crate::web::runtime::WebRuntime::new_fake(root.clone()),
        );
        let paths = ProductAppPaths::new(root.join(".aria"));
        IssueStore::new(paths.clone())
            .create(CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some("repository_0001".to_string()),
                logical_codebase_id: None,
                title: "gate response issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .expect("issue");
        let store = CodingAttemptStore::new(paths.clone());
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/c2-gate-response".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: crate::product::models::ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 0,
            })
            .expect("attempt");
        (temp, state, store, attempt)
    }

    fn request(
        command_id: &str,
        gate_id: &str,
        action_id: &str,
        expected_version: u64,
        extra_context: Option<&str>,
    ) -> Json<CodingGateResponseRestRequest> {
        Json(CodingGateResponseRestRequest {
            command_id: command_id.to_string(),
            gate_id: gate_id.to_string(),
            action_id: action_id.to_string(),
            extra_context: extra_context.map(str::to_string),
            expected_version,
        })
    }

    fn route(attempt: &crate::product::coding_models::CodingExecutionAttempt) -> Path<CodingAttemptRoutePath> {
        Path(CodingAttemptRoutePath {
            project_id: Some(attempt.project_id.clone()),
            issue_id: Some(attempt.issue_id.clone()),
            attempt_id: attempt.id.clone(),
        })
    }

    #[tokio::test]
    async fn c2_gate_response_rest_replays_same_command_and_rejects_stale_version() {
        let (_tmp, state, store, attempt) = fixture().await;

        // 旧页面过期版本：Rejected"请刷新"，不改 attempt、不启动 provider。
        let (status, body) = post_coding_gate_response(
            State(state.clone()),
            route(&attempt),
            request("cmd-gate-0001", "gate_missing", "manual_continue", 99, None),
        )
        .await
        .expect("stale version handled as conflict body");
        assert_eq!(status, axum::http::StatusCode::CONFLICT);
        assert_eq!(body.state, OperationState::Rejected);
        assert!(body.reason.as_deref().unwrap_or("").contains("请刷新"));
        let after = store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap();
        assert_eq!(after.version, attempt.version, "stale version must not mutate");
        assert_eq!(after.status, attempt.status, "stale version must not advance status");
        let ledger = store
            .find_attempt_command_result(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                "cmd-gate-0001",
            )
            .unwrap()
            .expect("rejected result recorded");
        assert_eq!(ledger.state, OperationState::Rejected);

        // 正确版本（门不存在 → 引擎按 WS 语义 no-op 返回 attempt）：
        // Accepted 落账，200。
        let (status, body) = post_coding_gate_response(
            State(state.clone()),
            route(&attempt),
            request("cmd-gate-0002", "gate_missing", "manual_continue", attempt.version, None),
        )
        .await
        .expect("accepted");
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body.state, OperationState::Accepted);

        // 同 command 同 payload：重放首次 durable 结果（Replayed），零副作用。
        let (status, replay) = post_coding_gate_response(
            State(state.clone()),
            route(&attempt),
            request("cmd-gate-0002", "gate_missing", "manual_continue", attempt.version, None),
        )
        .await
        .expect("replayed");
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(replay.state, OperationState::Replayed);

        // 同 command 异 payload（extra_context 变更）：fail-closed"请刷新"。
        let conflict = post_coding_gate_response(
            State(state.clone()),
            route(&attempt),
            request(
                "cmd-gate-0002",
                "gate_missing",
                "manual_continue",
                attempt.version,
                Some("different payload"),
            ),
        )
        .await
        .expect_err("same command with different payload must fail closed");
        assert_eq!(
            conflict.code, "coding_gate_response_command_conflict",
            "conflict surfaces a refresh prompt, not a provider start"
        );
    }
}
