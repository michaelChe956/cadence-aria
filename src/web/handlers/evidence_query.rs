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
use axum::response::IntoResponse;
use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::evidence_audit::{
    EvidenceAuditRecord, append_evidence_audit,
};
use crate::product::logical_codebase::evidence_budget::EvidenceBudgetLedger;
use crate::product::logical_codebase::evidence_index::EvidenceError;
use crate::product::logical_codebase::evidence_mediator::evidence_role_label;
use crate::product::logical_codebase::{
    EvidenceQueryInput, handle_evidence_query, resolve_attempt_by_token,
};
use crate::web::error::ApiResult;
use crate::web::handlers::evidence_error_mapping::evidence_api_error;
use crate::web::state::WebAppState;

/// C2 oracle C-2（任务二）：政策正文查询保留字——在跑 coder／reviewer 经
/// 注入脚本 `--query policy` 读取受限政策正文时，路由到
/// `handle_policy_text_query`（Running＋claims 校验链：令牌反查→Running→
/// claims role／attempt／digest／过期→resolver 冻结一致性），不走路由仓
/// 证据链、不消耗 evidence 配额、不做本地路径 fallback。
pub(crate) const POLICY_TEXT_QUERY_SENTINEL: &str = "policy";

pub async fn evidence_query(
    State(state): State<WebAppState>,
    Json(input): Json<EvidenceQueryInput>,
) -> ApiResult<axum::response::Response> {
    let paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    if input.query.trim() == POLICY_TEXT_QUERY_SENTINEL {
        let result = crate::product::logical_codebase::handle_policy_text_query(
            &paths,
            &crate::product::logical_codebase::evidence_mediator::PolicyTextQueryInput {
                token: input.token.clone(),
                role: input.role,
            },
        )
        .map_err(crate::web::handlers::evidence_error_mapping::policy_api_error)?;
        return Ok(axum::Json(result).into_response());
    }
    match handle_evidence_query(&paths, &input) {
        Ok(response) => Ok(Json(response).into_response()),
        Err(error) => {
            // 控制器裁决 1（T6 Minor-1）：被拒查询也补审计（query 留痕；
            // result_chars=0、budget_remaining 取当前 ledger 或 0）。
            // M1（fix round 1）：`EvidenceError::Io` 可能来自 T6 成功路径自身
            // 审计 append 失败（结果已服务），此时再补零结果「被拒」审计会失真，
            // 故 Io 不补记（见 should_record_rejected_audit）。
            if should_record_rejected_audit(&error) {
                record_rejected_evidence_query(&paths, &input);
            }
            Err(evidence_api_error(&error))
        }
    }
}

/// M1（fix round 1）：`EvidenceError::Io` 可能来自 T6 成功路径自身审计 append 失败
/// （结果已服务但审计落盘失败），此时再补一条零结果「被拒」审计会失真，故 Io 不补记。
fn should_record_rejected_audit(error: &EvidenceError) -> bool {
    !matches!(error, EvidenceError::Io { .. })
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
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };
    use crate::product::coding_models::{
        AttemptTargetSnapshot, CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage,
    };
    use crate::product::json_store::write_json;
    use crate::product::logical_codebase::aggregate_index::{
        AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus,
        AggregateIndexStore,
    };
    use crate::product::logical_codebase::store::{
        LogicalCodebaseManifest, LogicalCodebaseStore,
    };
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseLayout,
        LogicalRepositoryId, MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord,
        RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::logical_codebase::evidence_audit::EvidenceAuditRecord;
    use crate::product::logical_codebase::evidence_budget::EVIDENCE_ATTEMPT_CHAR_QUOTA;
    use crate::product::logical_codebase::evidence_index::EvidenceError;
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
            admission_kind: crate::product::coding_models::CodingAdmissionKind::LegacyGroup,
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
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
            start_claim: None,
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
    fn web_evidence_io_error_skips_rejected_audit_but_other_errors_do_not() {
        assert!(
            !should_record_rejected_audit(&EvidenceError::Io {
                message: "audit append failed".to_string()
            }),
            "Io must not trigger a rejected-audit record"
        );
        assert!(
            should_record_rejected_audit(&EvidenceError::InvalidQuery {
                reason: "empty".to_string()
            }),
            "InvalidQuery must still trigger a rejected-audit record"
        );
        assert!(
            should_record_rejected_audit(&EvidenceError::BudgetExhausted),
            "BudgetExhausted must still trigger a rejected-audit record"
        );
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

    /// C2 oracle C-2（任务二）：在跑证据查询按政策类型路由——`--query policy`
    /// 进入 handle_policy_text_query 的 Running＋claims 校验链：Running 且
    /// 无 claims 时 403 policy_forbidden（提示重新授权），不落 evidence 配额。
    #[tokio::test]
    async fn web_evidence_policy_query_routes_to_claims_chain() {
        let (tmp, paths, _repo, token) = fixture();
        let state = crate::web::state::WebAppState::new(
            tmp.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(tmp.path().to_path_buf()),
        );
        let response = evidence_query(
            axum::extract::State(state),
            axum::Json(EvidenceQueryInput {
                token: token.clone(),
                role: EvidenceRole::Coder,
                query: "policy".to_string(),
            }),
        )
        .await
        .expect_err("policy query without claims must fail closed into the claims chain");
        assert_eq!(
            response.code, "policy_forbidden",
            "policy sentinel must reach the claims verification chain"
        );
        assert!(
            response
                .message
                .contains("policy_reauthorization_required_missing_claims"),
            "rejection must point to reauthorization, got: {}",
            response.message
        );
        // 不消耗 evidence 配额（政策读取不在 budget ledger 内）。
        let budget = crate::product::logical_codebase::evidence_budget::EvidenceBudgetLedger::new(
            paths.clone(),
        )
        .remaining(
            &crate::product::logical_codebase::resolve_attempt_by_token(&paths, &token)
                .expect("attempt resolves"),
        )
        .expect("budget read");
        assert_eq!(
            budget,
            crate::product::logical_codebase::evidence_budget::EVIDENCE_ATTEMPT_CHAR_QUOTA
        );
    }

    /// 非 policy 查询保持既有路由仓证据链（无 target snapshot → 404
    /// evidence_not_available），不进入政策校验链。
    #[tokio::test]
    async fn web_evidence_non_policy_query_stays_on_evidence_chain() {
        let (tmp, _paths, _repo, token) = fixture();
        let state = crate::web::state::WebAppState::new(
            tmp.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(tmp.path().to_path_buf()),
        );
        let response = evidence_query(
            axum::extract::State(state),
            axum::Json(EvidenceQueryInput {
                token,
                role: EvidenceRole::Coder,
                query: "policy_text_for_other_symbol".to_string(),
            }),
        )
        .await
        .expect_err("non-policy query without target snapshot fails on the evidence chain");
        assert_eq!(response.code, "evidence_not_available");
    }

    // ---- 片8A：state.evidence_command_runner（片7W 覆盖点）接线断言 ----

    /// 与 mediator 全链夹具（evidence_mediator_tests.inc.rs）同构：issue
    /// authority + manifest + 两成员 checkout + active 聚合索引 + `.codegraph`
    /// 库目录 + Running attempt（含 target_snapshot）+ 令牌——查询可达第⑤步
    ///（CodeGraph 派发），注入 runner 才会被调用。
    fn seed_issue_authority(paths: &ProductAppPaths) {
        crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "evidence-query-handler".to_string(),
                description: None,
            })
            .expect("create project");
        crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: PROJECT_ID.to_string(),
                repo_id: Some("repository_0001".to_string()),
                logical_codebase_id:
                    crate::product::logical_codebase::legacy_logical_codebase_id(PROJECT_ID),
                title: "evidence query handler issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .expect("create issue");
    }

    fn full_chain_attempt(
        target_snapshot: AttemptTargetSnapshot,
    ) -> crate::product::coding_models::CodingExecutionAttempt {
        crate::product::coding_models::CodingExecutionAttempt {
            target_snapshot: Some(target_snapshot),
            ..attempt_fixture()
        }
    }

    fn target_snapshot(
        logical_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
        canonical_path: &Path,
        revision: &str,
        membership_revision: u64,
    ) -> AttemptTargetSnapshot {
        AttemptTargetSnapshot {
            logical_repository_id: logical_id,
            checkout_id,
            physical_repository_id: format!("repo_{}", logical_id.0),
            canonical_path: canonical_path.to_path_buf(),
            git_dir_identity: "git-dir-id".to_string(),
            revision: Some(revision.to_string()),
            policy_digest: "policy-digest".to_string(),
            membership_revision,
            captured_at: "2026-08-14T00:00:00Z".to_string(),
            capture_source: "test".to_string(),
        }
    }

    fn member_record(
        id: LogicalRepositoryId,
        alias: &str,
        checkout_id: RepositoryCheckoutId,
        physical_id: &str,
    ) -> CodebaseMemberRecord {
        CodebaseMemberRecord {
            logical_repository_id: id,
            physical_repository_id: physical_id.to_string(),
            alias: alias.to_string(),
            role: "backend".to_string(),
            ordinal: 0,
            source_identity: RepositorySourceIdentity {
                scheme: "git_dir_only_v1".to_string(),
                key_digest: format!("sha256:{alias}"),
                canonical_git_dir: PathBuf::from(format!("/workspace/{alias}/.git")),
                canonical_origin: None,
                first_seen_path_hash: format!("hash:{alias}"),
            },
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![checkout_id],
            status: MemberStatus::Active,
            created_at: "2026-08-14T00:00:00Z".to_string(),
            updated_at: "2026-08-14T00:00:00Z".to_string(),
        }
    }

    fn checkout_record(
        logical_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
        canonical_path: PathBuf,
    ) -> RepositoryCheckoutRecord {
        RepositoryCheckoutRecord {
            checkout_id,
            logical_repository_id: logical_id,
            physical_repository_id: format!("repo_{}", logical_id.0),
            kind: CheckoutKind::Main,
            canonical_path,
            checkout_path_hash: "checkout-hash".to_string(),
            git_dir_identity: "git-dir-id".to_string(),
            revision: None,
            availability: CheckoutAvailability::Available,
            observed_at: "2026-08-14T00:00:00Z".to_string(),
            created_at: "2026-08-14T00:00:00Z".to_string(),
            updated_at: "2026-08-14T00:00:00Z".to_string(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn index_record(
        id: &str,
        api_id: LogicalRepositoryId,
        web_id: LogicalRepositoryId,
        api_checkout_id: RepositoryCheckoutId,
        web_checkout_id: RepositoryCheckoutId,
        api_revision: &str,
        web_revision: &str,
        codegraph_root: &Path,
    ) -> AggregateIndexRecord {
        AggregateIndexRecord {
            aggregate_index_id: id.to_string(),
            project_id: PROJECT_ID.to_string(),
            membership_revision: 1,
            status: AggregateIndexStatus::Active,
            command_id: None,
            member_snapshots: vec![
                AggregateIndexMemberSnapshot::indexed(
                    api_id,
                    api_checkout_id,
                    api_revision.to_string(),
                    false,
                    "2026-08-14T00:00:00Z".to_string(),
                ),
                AggregateIndexMemberSnapshot::indexed(
                    web_id,
                    web_checkout_id,
                    web_revision.to_string(),
                    false,
                    "2026-08-14T00:00:00Z".to_string(),
                ),
            ],
            observed_after_member_snapshots: Vec::new(),
            codegraph_version: "1.6.0".to_string(),
            codegraph_root: codegraph_root.to_path_buf(),
            config_digest: String::new(),
            created_at: "2026-08-14T00:00:00Z".to_string(),
            updated_at: "2026-08-14T00:00:00Z".to_string(),
            supersedes_aggregate_index_id: None,
            warning: None,
        }
    }

    fn full_chain_fixture() -> (TempDir, ProductAppPaths, PathBuf, String) {
        let tmp = TempDir::new().expect("tempdir");
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo dir");
        // fake-runner 夹具不跑真实 CLI，显式物化 `.codegraph` 对齐已建索引
        // 形态（mediator ④b 库目录守卫按此判「索引不可用」）。
        fs::create_dir_all(repo.join(".codegraph")).expect("create codegraph library dir");
        seed_issue_authority(&paths);

        let api_id = LogicalRepositoryId(Uuid::new_v4());
        let web_id = LogicalRepositoryId(Uuid::new_v4());
        let api_checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let web_checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let aggregate_index_id = format!("aggregate_index_{}", Uuid::new_v4().simple());

        let logical = LogicalCodebaseStore::new(paths.clone());
        let manifest = LogicalCodebaseManifest {
            schema_version: 1,
            project_id: PROJECT_ID.to_string(),
            logical_codebase_id: Uuid::new_v4(),
            provider_context_root: repo.clone(),
            layout: LogicalCodebaseLayout::CommonNonGitParent,
            membership_revision: 1,
            member_ids: vec![api_id, web_id],
            active_aggregate_index_id: Some(aggregate_index_id.clone()),
            context_policy_digest: String::new(),
            created_at: "2026-08-14T00:00:00Z".to_string(),
            updated_at: "2026-08-14T00:00:00Z".to_string(),
        };
        logical
            .save_manifest(PROJECT_ID, &manifest)
            .expect("save manifest");
        logical
            .save_member(
                PROJECT_ID,
                &member_record(api_id, "api", api_checkout_id, "repo_api"),
            )
            .expect("save api member");
        logical
            .save_member(
                PROJECT_ID,
                &member_record(web_id, "web", web_checkout_id, "repo_web"),
            )
            .expect("save web member");
        logical
            .save_checkout(
                PROJECT_ID,
                &checkout_record(api_id, api_checkout_id, repo.join("api")),
            )
            .expect("save api checkout");
        logical
            .save_checkout(
                PROJECT_ID,
                &checkout_record(web_id, web_checkout_id, repo.join("web")),
            )
            .expect("save web checkout");

        AggregateIndexStore::new(paths.clone())
            .create(
                PROJECT_ID,
                index_record(
                    &aggregate_index_id,
                    api_id,
                    web_id,
                    api_checkout_id,
                    web_checkout_id,
                    "rev-api",
                    "rev-web",
                    &repo,
                ),
            )
            .expect("create aggregate index record");

        let snapshot = target_snapshot(api_id, api_checkout_id, &repo.join("api"), "rev-api", 1);
        let attempt = full_chain_attempt(snapshot);
        write_json(&attempt_record_path(&paths), &attempt).expect("write attempt record");

        let token = issue_evidence_token(&paths, &repo, &attempt).expect("issue token");

        (tmp, paths, repo, token)
    }

    /// 计数 runner：每次调用 +1 并返回固定 stdout（与 mediator 测试的
    /// FakeCodeGraphRunner 同协议，另加调用计数供注入断言）。
    struct CountingCodeGraphRunner {
        calls: AtomicUsize,
        stdout: String,
    }

    impl CountingCodeGraphRunner {
        fn new(stdout: &str) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                stdout: stdout.to_string(),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for CountingCodeGraphRunner {
        async fn run(
            &self,
            _request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(BoundedCommandResult {
                exit_code: Some(0),
                stdout: self.stdout.clone(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                duration_ms: 1,
            })
        }
    }

    /// 片8A：片7W 的 `state.evidence_command_runner` 覆盖点必须被真实接到证据
    /// 查询路径——注入计数 runner 后，一次全链查询必须经它派发 CodeGraph 调用。
    #[tokio::test]
    async fn web_evidence_injected_runner_is_used_for_query() {
        let (tmp, paths, _repo, token) = full_chain_fixture();
        let hits_json = serde_json::json!([
            {"node": {"name": "usedByWeb", "filePath": "web/src/app.ts", "startLine": 10}},
        ]);
        let runner = Arc::new(CountingCodeGraphRunner::new(&hits_json.to_string()));
        let state = crate::web::state::WebAppState::new(
            tmp.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(tmp.path().to_path_buf()),
        )
        .with_evidence_command_runner(runner.clone());

        let _response = evidence_query(
            axum::extract::State(state),
            axum::Json(EvidenceQueryInput {
                token,
                role: EvidenceRole::Coder,
                query: "usedByWeb".to_string(),
            }),
        )
        .await
        .expect("full chain query via injected runner succeeds");

        // 注入的 runner 被真实用于证据查询（≥1 次 CodeGraph 派发）。
        assert!(
            runner.calls() > 0,
            "injected runner must be dispatched for the codegraph query"
        );
        // 全链成功佐证：审计落盘 1 条命中（结果确经注入 runner 返回）。
        let content = fs::read_to_string(audit_file_path(&paths)).expect("read audit file");
        let record: EvidenceAuditRecord = serde_json::from_str(
            content.lines().next().expect("audit line"),
        )
        .expect("audit record deserializes");
        assert_eq!(record.hit_count, 1);
    }
}
