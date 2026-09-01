#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };
    use crate::product::coding_models::{
        AttemptTargetSnapshot, CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage,
    };
    use crate::product::logical_codebase::aggregate_index::{
        AggregateIndexMemberSnapshot, AggregateIndexStatus,
    };
    use crate::product::logical_codebase::evidence_audit::EvidenceAuditRecord;
    use crate::product::logical_codebase::evidence_budget::{
        BudgetOutcome, EVIDENCE_ATTEMPT_CHAR_QUOTA, EvidenceBudgetLedger,
    };
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalRepositoryId,
        MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity,
        RepositoryType,
    };
    use crate::product::models::ProviderName;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const ATTEMPT_ID: &str = "coding_attempt_0001";

    struct Fixture {
        _tmp: TempDir,
        paths: ProductAppPaths,
        repo: PathBuf,
        attempt: CodingExecutionAttempt,
        token: String,
        manifest: LogicalCodebaseManifest,
        aggregate_index_id: String,
        api_id: LogicalRepositoryId,
        web_id: LogicalRepositoryId,
        api_checkout_id: RepositoryCheckoutId,
        web_checkout_id: RepositoryCheckoutId,
    }

    fn setup() -> Fixture {
        let tmp = TempDir::new().expect("tempdir");
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repo dir");

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
            layout: crate::product::logical_codebase::LogicalCodebaseLayout::CommonNonGitParent,
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

        let index_store = AggregateIndexStore::new(paths.clone());
        index_store
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
        let attempt = attempt_fixture(CodingAttemptStatus::Running, Some(snapshot));
        write_json(&attempt_record_path(&paths), &attempt).expect("write attempt record");

        let token = crate::product::logical_codebase::evidence_token::issue_evidence_token(
            &paths, &repo, &attempt,
        )
        .expect("issue evidence token");

        Fixture {
            _tmp: tmp,
            paths,
            repo,
            attempt,
            token,
            manifest,
            aggregate_index_id,
            api_id,
            web_id,
            api_checkout_id,
            web_checkout_id,
        }
    }

    fn attempt_fixture(
        status: CodingAttemptStatus,
        target_snapshot: Option<AttemptTargetSnapshot>,
    ) -> CodingExecutionAttempt {
        CodingExecutionAttempt {
            id: ATTEMPT_ID.to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: "work_item_0001".to_string(),
            attempt_no: 1,
            scope: CodingAttemptScope::WorkItem,
            status,
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
            target_snapshot,
            completed_at: None,
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
            codegraph_version: "1.5.0".to_string(),
            codegraph_root: codegraph_root.to_path_buf(),
            config_digest: String::new(),
            created_at: "2026-08-14T00:00:00Z".to_string(),
            updated_at: "2026-08-14T00:00:00Z".to_string(),
            supersedes_aggregate_index_id: None,
            warning: None,
        }
    }

    fn attempt_record_path(paths: &ProductAppPaths) -> PathBuf {
        paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join(format!("{ATTEMPT_ID}.json"))
    }

    fn pin_file_path(paths: &ProductAppPaths) -> PathBuf {
        paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join(ATTEMPT_ID)
            .join(EVIDENCE_INDEX_PIN_FILE)
    }

    #[test]
    fn resolve_attempt_by_token_returns_matching_attempt() {
        let fx = setup();

        let attempt = resolve_attempt_by_token(&fx.paths, &fx.token).expect("resolve attempt");
        assert_eq!(attempt.id, ATTEMPT_ID);
        assert_eq!(attempt.project_id, PROJECT_ID);
        assert_eq!(attempt.issue_id, ISSUE_ID);
    }

    #[test]
    fn resolve_attempt_by_token_rejects_unknown_token() {
        let fx = setup();

        let unknown = format!("unknown-{}-token", Uuid::new_v4());
        let err = resolve_attempt_by_token(&fx.paths, &unknown).expect_err("unknown token");
        assert_eq!(err, EvidenceError::Unauthorized);
    }

    #[test]
    fn pin_is_written_on_first_load_and_reused_on_second() {
        let mut fx = setup();

        // 首次：按 manifest.active_aggregate_index_id 写 pin 并返回该 record。
        let first =
            load_or_pin_index(&fx.paths, &fx.attempt, &fx.manifest, None).expect("first pin load");
        assert_eq!(first.aggregate_index_id, fx.aggregate_index_id);

        let pin: EvidenceIndexPinRecord =
            read_json(&pin_file_path(&fx.paths)).expect("read pin file");
        assert_eq!(pin.pinned_aggregate_index_id, fx.aggregate_index_id);
        assert!(!pin.pinned_at.is_empty());

        // 中途 active 指针被 supersede：写入第二代索引并把 manifest.active 指向它。
        let second_id = format!("aggregate_index_{}", Uuid::new_v4().simple());
        let index_store = AggregateIndexStore::new(fx.paths.clone());
        index_store
            .create(
                PROJECT_ID,
                index_record(
                    &second_id,
                    fx.api_id,
                    fx.web_id,
                    fx.api_checkout_id,
                    fx.web_checkout_id,
                    "rev-api-2",
                    "rev-web-2",
                    &fx.repo,
                ),
            )
            .expect("create second index");
        fx.manifest.active_aggregate_index_id = Some(second_id.clone());
        LogicalCodebaseStore::new(fx.paths.clone())
            .save_manifest(PROJECT_ID, &fx.manifest)
            .expect("save manifest with new active id");

        // 二次查询读钉住值（第一代 record），不重写 pin。
        let second =
            load_or_pin_index(&fx.paths, &fx.attempt, &fx.manifest, None).expect("second pin load");
        assert_eq!(second.aggregate_index_id, fx.aggregate_index_id);

        let pin: EvidenceIndexPinRecord =
            read_json(&pin_file_path(&fx.paths)).expect("re-read pin file");
        assert_eq!(pin.pinned_aggregate_index_id, fx.aggregate_index_id);
    }

    #[test]
    fn stale_detects_membership_revision_and_revision_mismatch() {
        let fx = setup();
        let snapshot = target_snapshot(
            fx.api_id,
            fx.api_checkout_id,
            &fx.repo.join("api"),
            "rev-api",
            1,
        );

        let matching = index_record(
            &fx.aggregate_index_id,
            fx.api_id,
            fx.web_id,
            fx.api_checkout_id,
            fx.web_checkout_id,
            "rev-api",
            "rev-web",
            &fx.repo,
        );
        assert!(!is_index_stale(&snapshot, &matching));

        // 方向一：membership_revision 不一致 → stale。
        let mut membership_advanced = matching.clone();
        membership_advanced.membership_revision = 2;
        assert!(is_index_stale(&snapshot, &membership_advanced));

        // 方向二：目标成员 revision 不一致 → stale。
        let mut revision_drifted = matching.clone();
        revision_drifted.member_snapshots[0].revision = "rev-api-other".to_string();
        assert!(is_index_stale(&snapshot, &revision_drifted));
    }

    struct FakeCodeGraphRunner {
        results: std::sync::Mutex<
            std::collections::VecDeque<Result<BoundedCommandResult, BoundedCommandError>>,
        >,
        requests: std::sync::Mutex<Vec<BoundedCommandRequest>>,
    }

    impl FakeCodeGraphRunner {
        fn with_stdout(stdout: &str) -> Self {
            Self {
                results: std::sync::Mutex::new(std::collections::VecDeque::from([Ok(
                    BoundedCommandResult {
                        exit_code: Some(0),
                        stdout: stdout.to_string(),
                        stderr: String::new(),
                        timed_out: false,
                        cancelled: false,
                        stdout_truncated: false,
                        stderr_truncated: false,
                        duration_ms: 1,
                    },
                )])),
                requests: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for FakeCodeGraphRunner {
        async fn run(
            &self,
            request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            self.requests
                .lock()
                .expect("fake runner requests")
                .push(request);
            self.results
                .lock()
                .expect("fake runner results")
                .pop_front()
                .expect("scripted result")
        }
    }

    fn audit_file_path(paths: &ProductAppPaths) -> PathBuf {
        paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join(ATTEMPT_ID)
            .join("evidence-audit.jsonl")
    }

    fn read_first_audit(paths: &ProductAppPaths) -> EvidenceAuditRecord {
        let content = std::fs::read_to_string(audit_file_path(paths)).expect("read audit file");
        let first = content.lines().next().expect("first audit line");
        serde_json::from_str(first).expect("parse audit record")
    }

    fn index_record_path(paths: &ProductAppPaths, id: &str) -> PathBuf {
        paths
            .aggregate_indexes_root(PROJECT_ID)
            .join(format!("{id}.json"))
    }

    fn query_input(token: &str, role: EvidenceRole, query: &str) -> EvidenceQueryInput {
        EvidenceQueryInput {
            token: token.to_string(),
            role,
            query: query.to_string(),
        }
    }

    #[test]
    fn full_chain_coder_cross_member_hit_returns_text_budget_and_audit() {
        let fx = setup();
        // 目标仓命中（api/）与非成员命中（other/）均应被过滤，仅 web/ 跨仓命中保留。
        let hits_json = serde_json::json!([
            {"node": {"name": "usedByWeb", "filePath": "web/src/app.ts", "startLine": 10}},
            {"node": {"name": "targetSelf", "filePath": "api/src/lib.rs", "startLine": 3}},
            {"node": {"name": "nonMember", "filePath": "other/foo.ts", "startLine": 5}},
        ]);
        let runner = Arc::new(FakeCodeGraphRunner::with_stdout(&hits_json.to_string()));
        let input = query_input(&fx.token, EvidenceRole::Coder, "usedByWeb");

        let response = handle_evidence_query_with_runner(&fx.paths, &input, runner.clone())
            .expect("full chain succeeds");

        assert_eq!(response.text, "web/src/app.ts:10 usedByWeb\n");
        assert!(!response.truncated);
        assert!(!response.index_stale);
        assert_eq!(
            response.budget_remaining,
            EVIDENCE_ATTEMPT_CHAR_QUOTA - response.text.chars().count()
        );

        // 审计落盘：role 含 role_self_reported，hit_count/snapshot_refs/result_chars/budget_remaining 对齐。
        let audit = read_first_audit(&fx.paths);
        assert_eq!(audit.attempt_id, ATTEMPT_ID);
        assert_eq!(audit.role, "coder(role_self_reported)");
        assert_eq!(audit.query, "usedByWeb");
        assert_eq!(audit.hit_count, 1);
        assert_eq!(audit.result_chars, response.text.chars().count());
        assert_eq!(audit.snapshot_refs, vec!["web/src/app.ts".to_string()]);
        assert_eq!(audit.budget_remaining, response.budget_remaining);
    }

    #[test]
    fn unauthorized_for_empty_and_wrong_token() {
        let fx = setup();
        let runner = || Arc::new(FakeCodeGraphRunner::with_stdout("[]"));

        let empty = handle_evidence_query_with_runner(
            &fx.paths,
            &query_input("", EvidenceRole::Coder, "x"),
            runner(),
        )
        .expect_err("empty token");
        assert_eq!(empty, EvidenceError::Unauthorized);

        let wrong = handle_evidence_query_with_runner(
            &fx.paths,
            &query_input("deadbeef", EvidenceRole::Coder, "x"),
            runner(),
        )
        .expect_err("wrong token");
        assert_eq!(wrong, EvidenceError::Unauthorized);
    }

    #[test]
    fn forbidden_when_attempt_not_running() {
        let fx = setup();
        let mut completed = fx.attempt.clone();
        completed.status = CodingAttemptStatus::Completed;
        write_json(&attempt_record_path(&fx.paths), &completed).expect("write completed attempt");

        let input = query_input(&fx.token, EvidenceRole::Coder, "x");
        let err = handle_evidence_query_with_runner(
            &fx.paths,
            &input,
            Arc::new(FakeCodeGraphRunner::with_stdout("[]")),
        )
        .expect_err("non-running attempt");
        assert_eq!(err, EvidenceError::Forbidden);
    }

    #[test]
    fn not_available_when_target_snapshot_none() {
        let fx = setup();
        let mut no_snapshot = fx.attempt.clone();
        no_snapshot.target_snapshot = None;
        write_json(&attempt_record_path(&fx.paths), &no_snapshot)
            .expect("write attempt without snapshot");

        let input = query_input(&fx.token, EvidenceRole::Reviewer, "x");
        let err = handle_evidence_query_with_runner(
            &fx.paths,
            &input,
            Arc::new(FakeCodeGraphRunner::with_stdout("[]")),
        )
        .expect_err("legacy attempt");
        assert_eq!(err, EvidenceError::NotAvailable);
    }

    #[test]
    fn non_member_and_target_hits_are_filtered_to_empty() {
        let fx = setup();
        let hits_json = serde_json::json!([
            {"node": {"name": "self", "filePath": "api/src/lib.rs", "startLine": 1}},
            {"node": {"name": "other", "filePath": "other/x.ts", "startLine": 2}},
        ]);
        let runner = Arc::new(FakeCodeGraphRunner::with_stdout(&hits_json.to_string()));
        let input = query_input(&fx.token, EvidenceRole::Coder, "x");

        let response = handle_evidence_query_with_runner(&fx.paths, &input, runner)
            .expect("filtered-empty query succeeds");
        assert_eq!(response.text, "");
        assert_eq!(response.budget_remaining, EVIDENCE_ATTEMPT_CHAR_QUOTA);
        assert_eq!(read_first_audit(&fx.paths).hit_count, 0);
    }

    #[test]
    fn budget_exhausted_returns_error() {
        let fx = setup();
        let ledger = EvidenceBudgetLedger::new(fx.paths.clone());
        assert_eq!(
            ledger.consume(&fx.attempt, "warmup", 119_999).unwrap(),
            BudgetOutcome::Accepted { remaining: 1 }
        );

        let hits_json = serde_json::json!([
            {"node": {"name": "hit", "filePath": "web/x.ts", "startLine": 1}},
        ]);
        let runner = Arc::new(FakeCodeGraphRunner::with_stdout(&hits_json.to_string()));
        let input = query_input(&fx.token, EvidenceRole::Coder, "hit");

        let err = handle_evidence_query_with_runner(&fx.paths, &input, runner)
            .expect_err("quota exceeded");
        assert_eq!(err, EvidenceError::BudgetExhausted);
    }

    #[test]
    fn stale_marks_response_when_index_drifted() {
        let fx = setup();
        // 覆盖钉住索引为目标成员 revision 漂移版本。
        let drifted = index_record(
            &fx.aggregate_index_id,
            fx.api_id,
            fx.web_id,
            fx.api_checkout_id,
            fx.web_checkout_id,
            "rev-api-drifted",
            "rev-web",
            &fx.repo,
        );
        write_json(
            &index_record_path(&fx.paths, &fx.aggregate_index_id),
            &drifted,
        )
        .expect("overwrite index record");

        let hits_json = serde_json::json!([
            {"node": {"name": "hit", "filePath": "web/x.ts", "startLine": 1}},
        ]);
        let runner = Arc::new(FakeCodeGraphRunner::with_stdout(&hits_json.to_string()));
        let input = query_input(&fx.token, EvidenceRole::Coder, "hit");

        let response = handle_evidence_query_with_runner(&fx.paths, &input, runner)
            .expect("stale query succeeds");
        assert!(response.index_stale);
        assert_eq!(response.text, "web/x.ts:1 hit\n");
    }

    #[test]
    fn truncation_marks_response_and_appends_marker() {
        let fx = setup();
        // 200 条命中（MAX_EVIDENCE_HITS 上限）且每行 > 100 字符，总长 > 12k → 截断。
        let entries: Vec<serde_json::Value> = (0..200)
            .map(|index| {
                serde_json::json!({
                    "node": {
                        "name": format!("symbol_{}", "x".repeat(80)),
                        "filePath": format!("web/src/file{index}.ts"),
                        "startLine": index,
                    }
                })
            })
            .collect();
        let runner = Arc::new(FakeCodeGraphRunner::with_stdout(
            &serde_json::json!(entries).to_string(),
        ));
        let input = query_input(&fx.token, EvidenceRole::Coder, "symbol");

        let response = handle_evidence_query_with_runner(&fx.paths, &input, runner)
            .expect("truncated query succeeds");
        assert!(response.truncated);
        assert!(response.text.ends_with(TRUNCATION_MARKER));
        assert_eq!(
            response.text.chars().count(),
            EVIDENCE_QUERY_RESULT_CHAR_LIMIT + TRUNCATION_MARKER.chars().count()
        );
        assert_eq!(
            response.budget_remaining,
            EVIDENCE_ATTEMPT_CHAR_QUOTA - response.text.chars().count()
        );
    }

    #[test]
    fn reviewer_role_is_distinguished_in_audit() {
        let fx = setup();
        let hits_json = serde_json::json!([
            {"node": {"name": "hit", "filePath": "web/x.ts", "startLine": 1}},
        ]);
        let runner = Arc::new(FakeCodeGraphRunner::with_stdout(&hits_json.to_string()));
        let input = query_input(&fx.token, EvidenceRole::Reviewer, "hit");

        handle_evidence_query_with_runner(&fx.paths, &input, runner)
            .expect("reviewer query succeeds");
        assert_eq!(
            read_first_audit(&fx.paths).role,
            "reviewer(role_self_reported)"
        );
    }
}
