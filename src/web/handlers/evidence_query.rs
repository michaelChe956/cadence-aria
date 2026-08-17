// C-4 跨仓只读证据中介：HTTP 端点 handler（Task 7）。
//
// 契约 REQ-COD-05「受控接口」；设计 §3.3/§5.2。`POST /api/evidence-query`
// 接收 body `{token, role, query}`（serde snake_case），调用 T6
// `handle_evidence_query`，成功返回 `{text, truncated, index_stale, budget_remaining}`
// （200）；失败走稳定码 JSON（见 `evidence_error_mapping` 与 `web/error.rs`）。
//
// 控制器裁决 1（T6 Minor-1）：对 `handle_evidence_query` 返回 `Err` 的查询也追加
// 一条 evidence_query 审计条目（query 文本留痕；result_chars=0、budget_remaining
// 取当前 ledger 值或 0）。成功路径仍由 T6 落审计，本层不重复。

use axum::Json;
use axum::extract::State;
use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::evidence_audit::{
    EvidenceAuditRecord, append_evidence_audit,
};
use crate::product::logical_codebase::evidence_budget::EvidenceBudgetLedger;
use crate::product::logical_codebase::evidence_mediator::evidence_role_label;
use crate::product::logical_codebase::{
    EvidenceQueryInput, EvidenceQueryResponse, handle_evidence_query, resolve_attempt_by_token,
};
use crate::web::error::ApiResult;
use crate::web::handlers::evidence_error_mapping::evidence_api_error;
use crate::web::state::WebAppState;

pub async fn evidence_query(
    State(state): State<WebAppState>,
    Json(input): Json<EvidenceQueryInput>,
) -> ApiResult<Json<EvidenceQueryResponse>> {
    let paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    match handle_evidence_query(&paths, &input) {
        Ok(response) => Ok(Json(response)),
        Err(error) => {
            // 控制器裁决 1（T6 Minor-1）：被拒查询也补审计（query 留痕；
            // result_chars=0、budget_remaining 取当前 ledger 或 0）。
            record_rejected_evidence_query(&paths, &input);
            Err(evidence_api_error(&error))
        }
    }
}

/// 被拒查询审计补记：令牌可反查 attempt 时追加一条零结果审计条目；
/// 令牌无效（无法反查 attempt）时跳过（无从定位 attempt 分区）。
fn record_rejected_evidence_query(paths: &ProductAppPaths, input: &EvidenceQueryInput) {
    let Ok(attempt) = resolve_attempt_by_token(paths, &input.token) else {
        return;
    };
    let budget_remaining = EvidenceBudgetLedger::new(paths.clone())
        .remaining(&attempt)
        .unwrap_or(0);
    let _ = append_evidence_audit(
        paths,
        &attempt,
        &EvidenceAuditRecord {
            attempt_id: attempt.id.clone(),
            role: evidence_role_label(input.role),
            query: input.query.clone(),
            hit_count: 0,
            result_chars: 0,
            snapshot_refs: Vec::new(),
            budget_remaining,
            timestamp: Utc::now().to_rfc3339(),
        },
    );
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;
    use crate::product::coding_models::{
        CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage,
    };
    use crate::product::json_store::write_json;
    use crate::product::logical_codebase::evidence_audit::EvidenceAuditRecord;
    use crate::product::logical_codebase::evidence_budget::EVIDENCE_ATTEMPT_CHAR_QUOTA;
    use crate::product::logical_codebase::evidence_token::issue_evidence_token;
    use crate::product::logical_codebase::{EvidenceQueryInput, EvidenceRole};
    use crate::product::models::ProviderName;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const ATTEMPT_ID: &str = "coding_attempt_0001";

    fn attempt_fixture() -> crate::product::coding_models::CodingExecutionAttempt {
        crate::product::coding_models::CodingExecutionAttempt {
            id: ATTEMPT_ID.to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: "work_item_0001".to_string(),
            attempt_no: 1,
            scope: CodingAttemptScope::WorkItem,
            status: CodingAttemptStatus::Running,
            version: 0,
            manual_recovery_reason: None,
            admission_ticket_consumed_at: None,
            stage: CodingExecutionStage::Coding,
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: Default::default(),
            },
            rework_count: 0,
            max_auto_rework: 0,
            work_item_group_id: None,
            current_work_item_id: Some("work_item_0001".to_string()),
            active_unit_id: None,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: "2026-08-11T00:00:00Z".to_string(),
            updated_at: "2026-08-11T00:00:00Z".to_string(),
            target_snapshot: None,
            completed_at: None,
        }
    }

    fn attempt_record_path(paths: &ProductAppPaths) -> PathBuf {
        paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join(format!("{ATTEMPT_ID}.json"))
    }

    fn audit_file_path(paths: &ProductAppPaths) -> PathBuf {
        paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join(ATTEMPT_ID)
            .join("evidence-audit.jsonl")
    }

    fn fixture() -> (TempDir, ProductAppPaths, PathBuf, String) {
        let tmp = TempDir::new().expect("tempdir");
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo dir");

        let attempt = attempt_fixture();
        write_json(&attempt_record_path(&paths), &attempt).expect("write attempt record");
        let token = issue_evidence_token(&paths, &repo, &attempt).expect("issue token");

        (tmp, paths, repo, token)
    }

    fn rejected_input(token: &str) -> EvidenceQueryInput {
        EvidenceQueryInput {
            token: token.to_string(),
            role: EvidenceRole::Coder,
            query: "RejectedSymbol".to_string(),
        }
    }

    #[test]
    fn web_evidence_rejected_query_appends_audit_entry_with_query_and_zero_chars() {
        let (_tmp, paths, _repo, token) = fixture();
        record_rejected_evidence_query(&paths, &rejected_input(&token));

        let content = fs::read_to_string(audit_file_path(&paths)).expect("read audit file");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(
            lines.len(),
            1,
            "rejected query must append exactly one audit line"
        );

        let value: serde_json::Value =
            serde_json::from_str(lines[0]).expect("audit line must be valid JSON");
        let record: EvidenceAuditRecord =
            serde_json::from_value(value.clone()).expect("audit record deserializes");
        assert_eq!(record.attempt_id, ATTEMPT_ID);
        assert_eq!(record.role, "coder(role_self_reported)");
        assert_eq!(record.query, "RejectedSymbol");
        assert_eq!(record.hit_count, 0);
        assert_eq!(record.result_chars, 0);
        assert!(record.snapshot_refs.is_empty());
        // budget_remaining 取当前 ledger 值（无消费时=全配额）。
        assert_eq!(record.budget_remaining, EVIDENCE_ATTEMPT_CHAR_QUOTA);
    }

    #[test]
    fn web_evidence_rejected_query_skips_audit_when_token_unresolvable() {
        let (_tmp, paths, _repo, _token) = fixture();
        record_rejected_evidence_query(&paths, &rejected_input("no-such-token"));

        let path = audit_file_path(&paths);
        assert!(
            !path.exists(),
            "unresolvable token must not append an audit entry"
        );
    }
}
