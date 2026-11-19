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
    use crate::product::logical_codebase::failure_copy::{
        MEMBER_SOURCE_LESS_CODE, member_source_less_evidence_copy,
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

    /// REQ-ROUTE-01：evidence mediator 经唯一 authority resolver 冻结 issue 归属，
    /// 故夹具必须落 project 记录 + 一条归属 legacy 别名 LC 的 issue。该别名子树
    /// 与本夹具已写在项目级根的 manifest/member/checkout 字节等价
    ///（`store::lc_scope_root` 契约），`ProjectStore::get` 的幂等
    /// `migrate_legacy` 负责补别名 LC record。
    fn seed_issue_authority(paths: &ProductAppPaths) {
        crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "evidence-mediator".to_string(),
                description: None,
            })
            .expect("create project");
        crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: PROJECT_ID.to_string(),
                repo_id: Some("repository_0001".to_string()),
                logical_codebase_id:
                    crate::product::logical_codebase::legacy_logical_codebase_id(PROJECT_ID),
                title: "evidence mediator issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .expect("create issue");
    }

    fn setup() -> Fixture {
        let tmp = TempDir::new().expect("tempdir");
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repo dir");
        // 真实世界已建索引的聚合根必有 `.codegraph` 库目录（codegraph init
        // 产物）；mediator 查询前置库目录守卫（④b）按此判「索引不可用」。
        // fake-runner 夹具不跑真实 CLI，显式物化该目录以对齐已建索引形态。
        std::fs::create_dir_all(repo.join(".codegraph")).expect("create codegraph library dir");
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
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
            start_claim: None,
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
            source_scan: None,
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
        let first = load_or_pin_index(
            &fx.paths,
            &fx.attempt,
            &fx.manifest,
            &crate::product::logical_codebase::legacy_logical_codebase_id(PROJECT_ID),
        )
        .expect("first pin load");
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
        let second = load_or_pin_index(
            &fx.paths,
            &fx.attempt,
            &fx.manifest,
            &crate::product::logical_codebase::legacy_logical_codebase_id(PROJECT_ID),
        )
        .expect("second pin load");
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

    /// attempt 分区级文件（预算/审计/pin）路径：按 attempt_id 参数化。
    fn attempt_partition_file(paths: &ProductAppPaths, attempt_id: &str, file: &str) -> PathBuf {
        paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join(attempt_id)
            .join(file)
    }

    /// REQ-COD-05 适配核查（multi-repo-group-coding WP4）：多 target 拆分增殖
    /// 形态下（同 issue 双 target-attempt 并存，各持不同冻结快照+独立 token），
    /// 跨仓证据链全链按 attempt 分区独立运转——
    /// ①令牌反查各自命中自身 attempt（快照承载 target 维度）；
    /// ②ACL 方向由各自冻结快照决定（A=api 只见 web/、B=web 只见 api/）；
    /// ③预算/④审计/⑤pin 每 attempt 独立分账（口径与单仓一致）。
    #[test]
    fn multi_target_attempts_keep_evidence_chain_partitioned_per_attempt() {
        let fx = setup();

        // 增殖第二 target-attempt（target=web）：WorkItemGroup scope+OQ2 命名
        // branch（worktree_path_for_attempt 按该前缀推导出 per-target worktree，
        // token 文件落各自 worktree）。
        let web_snapshot = target_snapshot(
            fx.web_id,
            fx.web_checkout_id,
            &fx.repo.join("web"),
            "rev-web",
            1,
        );
        let mut attempt_b = attempt_fixture(CodingAttemptStatus::Running, Some(web_snapshot));
        attempt_b.id = format!("coding_attempt_{}", Uuid::new_v4().simple());
        attempt_b.attempt_no = 2;
        attempt_b.scope = CodingAttemptScope::WorkItemGroup;
        attempt_b.work_item_group_id = Some("work_item_plan_0001".to_string());
        attempt_b.branch_name = format!("aria/issues/{ISSUE_ID}/{}", fx.web_id.0);
        write_json(
            &fx.paths
                .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                .join("coding-attempts")
                .join(format!("{}.json", attempt_b.id)),
            &attempt_b,
        )
        .expect("write attempt B record");
        let token_b = crate::product::logical_codebase::evidence_token::issue_evidence_token(
            &fx.paths, &fx.repo, &attempt_b,
        )
        .expect("issue evidence token for attempt B");

        // ① 令牌反查：各自命中自身 attempt，快照 target 维度互不串扰。
        let resolved_a = resolve_attempt_by_token(&fx.paths, &fx.token).expect("resolve A");
        let resolved_b = resolve_attempt_by_token(&fx.paths, &token_b).expect("resolve B");
        assert_eq!(resolved_a.id, fx.attempt.id);
        assert_eq!(resolved_b.id, attempt_b.id);
        assert_eq!(
            resolved_a
                .target_snapshot
                .as_ref()
                .expect("A snapshot")
                .logical_repository_id,
            fx.api_id
        );
        assert_eq!(
            resolved_b
                .target_snapshot
                .as_ref()
                .expect("B snapshot")
                .logical_repository_id,
            fx.web_id
        );

        // ② A（target=api）查询：跨仓命中仅 web/ 保留（ACL 排除本仓 api/）。
        let hits = serde_json::json!([
            {"node": {"name": "crossWeb", "filePath": "web/src/app.ts", "startLine": 10}},
            {"node": {"name": "selfApi", "filePath": "api/src/lib.rs", "startLine": 3}},
        ]);
        let response_a = handle_evidence_query_with_runner(
            &fx.paths,
            &query_input(&fx.token, EvidenceRole::Coder, "crossWeb"),
            Arc::new(FakeCodeGraphRunner::with_stdout(&hits.to_string())),
        )
        .expect("A query succeeds");
        assert_eq!(response_a.text, "web/src/app.ts:10 crossWeb\n");

        // ②' B（target=web）查询：跨仓命中仅 api/ 保留（方向相反——由 B 自身
        // 冻结快照决定，非全局状态）。
        let hits_b = serde_json::json!([
            {"node": {"name": "crossApi", "filePath": "api/src/lib.rs", "startLine": 7}},
            {"node": {"name": "selfWeb", "filePath": "web/src/app.ts", "startLine": 4}},
        ]);
        let response_b = handle_evidence_query_with_runner(
            &fx.paths,
            &query_input(&token_b, EvidenceRole::Reviewer, "crossApi"),
            Arc::new(FakeCodeGraphRunner::with_stdout(&hits_b.to_string())),
        )
        .expect("B query succeeds");
        assert_eq!(response_b.text, "api/src/lib.rs:7 crossApi\n");

        // ③ 预算独立：A 分区 ledger 记账、B 分区 ledger 独立——B 剩余额度 =
        // 全额 - B 自身消费（与单仓口径一致，不受 A 消费影响）。
        assert_eq!(
            response_a.budget_remaining,
            EVIDENCE_ATTEMPT_CHAR_QUOTA - response_a.text.chars().count()
        );
        assert_eq!(
            response_b.budget_remaining,
            EVIDENCE_ATTEMPT_CHAR_QUOTA - response_b.text.chars().count()
        );
        assert!(
            attempt_partition_file(&fx.paths, &fx.attempt.id, "evidence-budget.json").is_file()
        );
        assert!(attempt_partition_file(&fx.paths, &attempt_b.id, "evidence-budget.json").is_file());

        // ④ 审计独立：各自分区 jsonl 只含自身 attempt_id 的条目。
        let audit_a = std::fs::read_to_string(attempt_partition_file(
            &fx.paths,
            &fx.attempt.id,
            "evidence-audit.jsonl",
        ))
        .expect("A audit file");
        assert_eq!(audit_a.lines().count(), 1);
        assert!(audit_a.contains(&fx.attempt.id));
        assert!(!audit_a.contains(&attempt_b.id));
        let audit_b = std::fs::read_to_string(attempt_partition_file(
            &fx.paths,
            &attempt_b.id,
            "evidence-audit.jsonl",
        ))
        .expect("B audit file");
        assert_eq!(audit_b.lines().count(), 1);
        assert!(audit_b.contains(&attempt_b.id));
        assert!(!audit_b.contains(&fx.attempt.id));

        // ⑤ pin 独立：首查各自分区写 pin（同 manifest active index，但分文件）。
        assert!(
            attempt_partition_file(&fx.paths, &fx.attempt.id, EVIDENCE_INDEX_PIN_FILE).is_file()
        );
        assert!(
            attempt_partition_file(&fx.paths, &attempt_b.id, EVIDENCE_INDEX_PIN_FILE).is_file()
        );
    }

    #[test]
    fn write_back_active_pointer_unblocks_first_query_from_503() {
        let mut fx = setup();
        // 生产缺口形态：索引在册（Active）但 manifest.active_aggregate_index_id
        // 恒 None（回写缺失）⇒ 首次检索必 503，且 pin 文件无从生成。
        fx.manifest.active_aggregate_index_id = None;
        LogicalCodebaseStore::new(fx.paths.clone())
            .save_manifest(PROJECT_ID, &fx.manifest)
            .expect("save manifest without active pointer");
        let hits_json = serde_json::json!([
            {"node": {"name": "usedByWeb", "filePath": "web/src/app.ts", "startLine": 10}},
        ]);
        let runner = Arc::new(FakeCodeGraphRunner::with_stdout(&hits_json.to_string()));
        let input = query_input(&fx.token, EvidenceRole::Coder, "usedByWeb");

        let denied = handle_evidence_query_with_runner(&fx.paths, &input, runner.clone())
            .expect_err("missing pointer fails closed");
        assert!(matches!(
            denied,
            EvidenceError::QueryFailed { code, .. } if code == "evidence_index_unavailable"
        ));
        assert!(!pin_file_path(&fx.paths).exists());

        // 激活落点回写（apply_index 在 replace_active 之后经
        // set_active_aggregate_index 落盘的同一条通道）后：首查 200、pin
        // 生成并钉住该 active 世代。
        LogicalCodebaseStore::new(fx.paths.clone())
            .set_active_aggregate_index(PROJECT_ID, &fx.aggregate_index_id)
            .expect("write back active pointer");
        let response = handle_evidence_query_with_runner(&fx.paths, &input, runner)
            .expect("first query after write-back");
        assert_eq!(response.text, "web/src/app.ts:10 usedByWeb\n");
        let pin: EvidenceIndexPinRecord =
            read_json(&pin_file_path(&fx.paths)).expect("pin written on first success");
        assert_eq!(pin.pinned_aggregate_index_id, fx.aggregate_index_id);
    }

    // C2 政策读取测试段拆入子 include,共享 mod tests 作用域
    // (large_file_guard 的 1200 行上限,纯移动,不改变任何行为)。
    include!("evidence_mediator_tests_policy.inc.rs");

    // ---- 片8（REQ-COD-05 delta）：跨仓证据「未检索」语义 ----

    /// 夹具：把指定世代里某成员记为「经盘检确认无源码」（skip_reason）并落盘。
    fn skip_marked_index(fx: &Fixture, id: &str, skipped: LogicalRepositoryId) {
        let mut record = index_record(
            id,
            fx.api_id,
            fx.web_id,
            fx.api_checkout_id,
            fx.web_checkout_id,
            "rev-api",
            "rev-web",
            &fx.repo,
        );
        for member in record.member_snapshots.iter_mut() {
            if member.logical_repository_id == skipped {
                member.mark_source_less();
            }
        }
        write_json(&index_record_path(&fx.paths, id), &record).expect("overwrite index record");
    }

    fn runner_calls(runner: &FakeCodeGraphRunner) -> usize {
        runner.requests.lock().expect("fake runner requests").len()
    }

    /// 片8 共用：以 Coder 角色注入 fake runner 执行一次证据查询。
    fn run_query(
        fx: &Fixture,
        token: &str,
        symbol: &str,
        runner: Arc<FakeCodeGraphRunner>,
    ) -> Result<EvidenceQueryResponse, EvidenceError> {
        handle_evidence_query_with_runner(
            &fx.paths,
            &query_input(token, EvidenceRole::Coder, symbol),
            runner,
        )
    }

    /// 片8 共用：零命中脚本 runner（跳过支路预期不会被调用）。
    fn empty_runner() -> Arc<FakeCodeGraphRunner> {
        Arc::new(FakeCodeGraphRunner::with_stdout("[]"))
    }

    /// 片8-1/2：目标被钉住世代记为跳过 ⇒ 200 + not_searched + 零 CodeGraph 调用；
    /// text 是显式说明（与「检索无命中」的空文本可区分），message_zh 与 text
    /// 取同一码表证据面工厂（逐字同源）。
    #[test]
    fn skipped_target_returns_200_not_searched_without_codegraph_query() {
        let fx = setup();
        skip_marked_index(&fx, &fx.aggregate_index_id, fx.api_id);
        let runner = Arc::new(FakeCodeGraphRunner::with_stdout("[]"));
        let response = run_query(&fx, &fx.token, "anySymbol", runner.clone())
            .expect("skipped target returns explicit not-searched");
        let alert = response.not_searched.as_ref().expect("not_searched");
        assert_eq!(alert.code, MEMBER_SOURCE_LESS_CODE);
        assert!(!alert.message_zh.is_empty());
        assert_eq!(alert.message_zh, response.text);
        assert_eq!(response.text, member_source_less_evidence_copy("api"));
        assert!(!response.truncated);
        assert_eq!(runner_calls(&runner), 0);
    }

    /// 片8-3：跳过支路审计零命中 + 未检索原因；预算按实际回包文本核算。
    #[test]
    fn skipped_target_audit_records_zero_hits_and_reason() {
        let fx = setup();
        skip_marked_index(&fx, &fx.aggregate_index_id, fx.api_id);
        let response = run_query(&fx, &fx.token, "anySymbol", empty_runner())
            .expect("skipped target query succeeds");
        let audit = read_first_audit(&fx.paths);
        assert_eq!(audit.hit_count, 0);
        assert_eq!(audit.snapshot_refs, Vec::<String>::new());
        assert_eq!(audit.not_searched_reason.as_deref(), Some(MEMBER_SOURCE_LESS_CODE));
        assert_eq!(audit.result_chars, response.text.chars().count());
        assert_eq!(audit.budget_remaining, EVIDENCE_ATTEMPT_CHAR_QUOTA - audit.result_chars);
        assert_eq!(response.budget_remaining, audit.budget_remaining);
    }

    /// 片8-3b：跳过支路预算必须真消费——配额用尽 ⇒ 既有 429，不因跳过豁免。
    #[test]
    fn skipped_target_budget_exhaustion_returns_429_error() {
        let fx = setup();
        skip_marked_index(&fx, &fx.aggregate_index_id, fx.api_id);
        let ledger = EvidenceBudgetLedger::new(fx.paths.clone());
        assert_eq!(
            ledger
                .consume(&fx.attempt, "warmup", EVIDENCE_ATTEMPT_CHAR_QUOTA - 1)
                .unwrap(),
            BudgetOutcome::Accepted { remaining: 1 }
        );
        let err = run_query(&fx, &fx.token, "anySymbol", empty_runner())
            .expect_err("quota exhausted on skip branch");
        assert_eq!(err, EvidenceError::BudgetExhausted);
    }

    /// 片8-4：普通目标照旧检索，但结果取 ACL ∩ 钉住世代覆盖范围——被跳过
    /// 成员的命中不得出现；自身 not_searched 为 None、审计无原因。
    #[test]
    fn normal_target_results_exclude_skipped_member_paths() {
        let fx = setup();
        skip_marked_index(&fx, &fx.aggregate_index_id, fx.web_id);
        let runner = Arc::new(FakeCodeGraphRunner::with_stdout(
            r#"[{"node": {"name": "hit", "filePath": "web/x.ts", "startLine": 1}}]"#,
        ));
        let response = run_query(&fx, &fx.token, "hit", runner.clone())
            .expect("normal target query still runs");
        assert_eq!(runner_calls(&runner), 1);
        assert_eq!(response.text, "");
        assert!(response.not_searched.is_none());
        let audit = read_first_audit(&fx.paths);
        assert_eq!(audit.hit_count, 0);
        assert_eq!(audit.not_searched_reason, None);
    }

    /// 片8-5：跳过不放松凭据链——坏 token ⇒ Unauthorized；非 Running ⇒ Forbidden。
    #[test]
    fn skipped_target_still_enforces_acl_and_token() {
        let fx = setup();
        skip_marked_index(&fx, &fx.aggregate_index_id, fx.api_id);
        let err = run_query(&fx, "deadbeef", "anySymbol", empty_runner())
            .expect_err("bad token still rejected");
        assert_eq!(err, EvidenceError::Unauthorized);
        let mut completed = fx.attempt.clone();
        completed.status = CodingAttemptStatus::Completed;
        write_json(&attempt_record_path(&fx.paths), &completed).expect("write completed");
        let err = run_query(&fx, &fx.token, "anySymbol", empty_runner())
            .expect_err("non-running attempt still rejected");
        assert_eq!(err, EvidenceError::Forbidden);
    }

    /// 片8-6：老世代（无 skip_reason）⇒ 正常检索，not_searched 不出现。
    #[test]
    fn legacy_generation_without_skip_reason_searches_normally() {
        let fx = setup();
        let runner = Arc::new(FakeCodeGraphRunner::with_stdout(
            r#"[{"node": {"name": "hit", "filePath": "web/x.ts", "startLine": 1}}]"#,
        ));
        let response = run_query(&fx, &fx.token, "hit", runner.clone())
            .expect("legacy generation searches normally");
        assert_eq!(runner_calls(&runner), 1);
        assert_eq!(response.text, "web/x.ts:1 hit\n");
        assert!(response.not_searched.is_none());
    }

    /// 片8-7：输入校验在分叉之前——空串/超长查询词对跳过目标同样被拒。
    #[test]
    fn invalid_query_is_rejected_before_the_skip_branch() {
        let fx = setup();
        skip_marked_index(&fx, &fx.aggregate_index_id, fx.api_id);
        let runner = Arc::new(FakeCodeGraphRunner::with_stdout("[]"));
        let too_long = "x".repeat(513);
        for query in ["", too_long.as_str()] {
            let err = run_query(&fx, &fx.token, query, runner.clone())
                .expect_err("invalid query rejected before skip branch");
            assert!(matches!(err, EvidenceError::InvalidQuery { .. }));
        }
        assert_eq!(runner_calls(&runner), 0);
    }

    /// 片8-8/9：库目录缺失不改判（仍 200 + not_searched + 零调用）；坏 pin
    ///（指向不存在的世代）在分叉之前被拒——不是 200。
    #[test]
    fn bad_pin_rejected_before_branch_and_missing_library_dir_still_not_searched() {
        let fx = setup();
        skip_marked_index(&fx, &fx.aggregate_index_id, fx.api_id);
        std::fs::remove_dir_all(fx.repo.join(".codegraph")).expect("remove library dir");
        let runner = Arc::new(FakeCodeGraphRunner::with_stdout("[]"));
        // 片8-9：跳过支路不进库目录守卫，不落 evidence_index_unavailable。
        let response = run_query(&fx, &fx.token, "anySymbol", runner.clone())
            .expect("skip branch bypasses library dir guard");
        assert_eq!(response.not_searched.expect("not_searched").code, MEMBER_SOURCE_LESS_CODE);
        assert_eq!(runner_calls(&runner), 0);
        // 片8-8：坏 pin 仍按既有规则拒绝（分叉之前）。
        write_json(
            &pin_file_path(&fx.paths),
            &EvidenceIndexPinRecord {
                pinned_aggregate_index_id: "aggregate_index_missing".to_string(),
                pinned_at: "2026-08-14T00:00:00Z".to_string(),
            },
        )
        .expect("pre-write bad pin");
        let err = run_query(&fx, &fx.token, "anySymbol", empty_runner())
            .expect_err("bad pin rejected before skip branch");
        assert!(matches!(
            err,
            EvidenceError::QueryFailed { code, .. } if code == "evidence_index_unavailable"
        ));
    }

    /// 片8-10：普通检索与未检索的审计各自保留 query 原文与事实，互不覆盖
    ///（attempt A 先在默认世代正常命中；attempt B 钉住记 web 为跳过的新世代）。
    #[test]
    fn original_query_and_audit_reason_are_both_preserved() {
        let fx = setup();
        // A（target=api）在默认世代（无跳过）先查：跨仓命中保留。
        let runner_a = Arc::new(FakeCodeGraphRunner::with_stdout(
            r#"[{"node": {"name": "crossWeb", "filePath": "web/src/app.ts", "startLine": 10}}]"#,
        ));
        run_query(&fx, &fx.token, "aOriginalQuery", runner_a).expect("A normal query succeeds");
        let audit_a = read_first_audit(&fx.paths);
        assert_eq!(audit_a.query, "aOriginalQuery");
        assert_eq!(audit_a.hit_count, 1);
        assert_eq!(audit_a.not_searched_reason, None);
        // 激活新世代（web 被记为跳过）；增殖 attempt B（target=web）首查钉住它。
        let g2_id = format!("aggregate_index_{}", Uuid::new_v4().simple());
        skip_marked_index(&fx, &g2_id, fx.web_id);
        LogicalCodebaseStore::new(fx.paths.clone())
            .set_active_aggregate_index(PROJECT_ID, &g2_id)
            .expect("activate skip-marked generation");
        let mut attempt_b = attempt_fixture(
            CodingAttemptStatus::Running,
            Some(target_snapshot(fx.web_id, fx.web_checkout_id, &fx.repo.join("web"), "rev-web", 1)),
        );
        attempt_b.id = format!("coding_attempt_{}", Uuid::new_v4().simple());
        attempt_b.attempt_no = 2;
        attempt_b.scope = CodingAttemptScope::WorkItemGroup;
        attempt_b.work_item_group_id = Some("work_item_plan_0001".to_string());
        attempt_b.branch_name = format!("aria/issues/{ISSUE_ID}/{}", fx.web_id.0);
        let attempts_dir = fx
            .paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts");
        write_json(&attempts_dir.join(format!("{}.json", attempt_b.id)), &attempt_b)
            .expect("write attempt B record");
        let token_b = crate::product::logical_codebase::evidence_token::issue_evidence_token(
            &fx.paths,
            &fx.repo,
            &attempt_b,
        )
        .expect("issue evidence token for attempt B");
        let response_b = handle_evidence_query_with_runner(
            &fx.paths,
            &query_input(&token_b, EvidenceRole::Reviewer, "bOriginalQuery"),
            Arc::new(FakeCodeGraphRunner::with_stdout("[]")),
        )
        .expect("B skip query succeeds");
        assert_eq!(response_b.not_searched.expect("B not_searched").code, MEMBER_SOURCE_LESS_CODE);
        // B 分区审计保留自己的 query 原文与未检索事实；A 的审计不被覆盖。
        let audit_b: EvidenceAuditRecord = serde_json::from_str(
            std::fs::read_to_string(attempt_partition_file(
                &fx.paths,
                &attempt_b.id,
                "evidence-audit.jsonl",
            ))
            .expect("B audit file")
            .lines()
            .next()
            .expect("B first audit line"),
        )
        .expect("parse B audit record");
        assert_eq!(audit_b.query, "bOriginalQuery");
        assert_eq!(audit_b.hit_count, 0);
        assert_eq!(audit_b.not_searched_reason.as_deref(), Some(MEMBER_SOURCE_LESS_CODE));
        assert_eq!(read_first_audit(&fx.paths).query, "aOriginalQuery");
    }
}
