#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use parking_lot::Mutex;

    use sha2::Digest;

    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };
    use crate::product::logical_codebase::{
        CheckoutAvailability, LogicalRepositoryId, RepositoryCheckoutId, RepositorySourceIdentity,
        RepositoryType,
    };

    use super::*;
    include!("operation_sdk_coverage_tests.inc.rs");

    #[test]
    fn build_persists_building_before_cli_and_marks_first_build_drift_failed() {
        let fixture = aggregate_index_fixture();
        fixture.cli.files_return(["api/src/A.java", "web/src/B.ts"]);
        for query in EXCLUDED_QUERIES {
            fixture.cli.query_returns(query, serde_json::json!([]));
        }
        let building_seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let building_seen_by_hook = Arc::clone(&building_seen);
        let records_root = fixture.paths.aggregate_indexes_root("project_0001");
        fixture.cli.observe_init(Arc::new(move || {
            let seen = std::fs::read_dir(&records_root)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter_map(|entry| std::fs::read(entry.path()).ok())
                .filter_map(|bytes| serde_json::from_slice::<AggregateIndexRecord>(&bytes).ok())
                .any(|record| record.status == AggregateIndexStatus::Building);
            building_seen_by_hook.store(seen, std::sync::atomic::Ordering::SeqCst);
        }));
        fixture.cli.drift_on_init();

        let error = fixture.operation().build("project_0001", 3).unwrap_err();
        assert!(matches!(
            error,
            AggregateIndexError::Failed { code: "aggregate_index_member_drifted", .. }
        ));
        assert!(building_seen.load(std::sync::atomic::Ordering::SeqCst));
        let records = fixture.records("project_0001");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, AggregateIndexStatus::Failed);
        assert_eq!(records[0].member_snapshots[0].revision, "a".repeat(40));
        assert_eq!(
            records[0].observed_after_member_snapshots[0].revision,
            "b".repeat(40)
        );
    }

    #[test]
    fn build_requires_member_coverage_and_negative_exclusion() {
        let fixture = aggregate_index_fixture();
        fixture.cli.files_return(["api/src/A.java", "web/src/B.ts"]);
        fixture
            .cli
            .query_returns("SHOULD_NOT_INDEX_WORKTREE", serde_json::json!([]));
        fixture
            .cli
            .query_returns("SHOULD_NOT_INDEX_ARIA", serde_json::json!([]));
        fixture
            .cli
            .query_returns("SHOULD_NOT_INDEX_BUILD", serde_json::json!([]));
        fixture
            .cli
            .query_returns("SHOULD_NOT_INDEX_NONMEMBER", serde_json::json!([]));

        let record = fixture.operation().build("project_0001", 3).unwrap();
        assert_eq!(record.status, AggregateIndexStatus::Active);
        assert_eq!(record.membership_revision, 3);
        assert_eq!(record.config_digest, fixture.config_digest());
        // ③ 版本透传：codegraph_version 必须是 CLI 实测值（fake 输出
        // 1.6.2），不得沿用 types.rs 硬编码默认值误导排障。
        assert_eq!(record.codegraph_version, "1.6.2");
        assert_eq!(record.member_snapshots.len(), 2);
        assert_eq!(record.observed_after_member_snapshots.len(), 2);
        assert_eq!(
            record.member_snapshots[0].revision,
            record.observed_after_member_snapshots[0].revision
        );
        assert!(
            record
                .member_snapshots
                .iter()
                .all(|snapshot| !snapshot.dirty)
        );
        assert_eq!(
            fixture.store.active("project_0001").unwrap().unwrap(),
            record
        );

        fixture.cli.files_return(["api/src/A.java", "web/src/B.ts"]);
        fixture
            .cli
            .query_returns("SHOULD_NOT_INDEX_WORKTREE", serde_json::json!([]));
        fixture
            .cli
            .query_returns("SHOULD_NOT_INDEX_ARIA", serde_json::json!([]));
        fixture
            .cli
            .query_returns("SHOULD_NOT_INDEX_BUILD", serde_json::json!([]));
        fixture.cli.query_returns(
            "SHOULD_NOT_INDEX_NONMEMBER",
            serde_json::json!([{"file":"not-a-repo/src/Leak.java"}]),
        );
        assert!(matches!(
            fixture.operation().build("project_0001", 3),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_exclusion_failed"
        ));
    }

    #[test]
    fn build_rejects_missing_member_coverage() {
        let fixture = aggregate_index_fixture();
        fixture.cli.files_return(["api/src/A.java"]);
        assert!(matches!(
            fixture.operation().build("project_0001", 3),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_member_coverage_failed"
        ));
    }

    /// ① 现场（/tmp/notify-center）：成员仓仅 .git+README，init 建出
    /// 0 文件库，codegraph 1.6 的 `files --json` 以 exit 0 输出人读提示。
    /// 该形态必须归并为业务语义的覆盖不足错误并点名全部成员，而不是
    /// 把提示文本当 JSON 解析出 `expected value at line 1 column 1` 天书。
    #[test]
    fn build_on_empty_index_reports_member_coverage_naming_all_members() {
        let fixture = aggregate_index_fixture();
        fixture.cli.files_return_empty_index_notice();

        let error = fixture.operation().build("project_0001", 3).unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "aggregate_index_member_coverage_failed"),
            "empty index must surface business semantics, got: {error}"
        );
        let message = error.to_string();
        assert!(message.contains("api"), "must name member api: {message}");
        assert!(message.contains("web"), "must name member web: {message}");
    }

    /// ① 部分覆盖：点名全部无可索引文件的成员清单（不止第一个），
    /// 供 bootstrap 失败卡呈现「每成员至少 1 个可索引代码文件」指引。
    #[test]
    fn build_names_all_uncovered_members_in_coverage_failure() {
        let fixture = aggregate_index_fixture();
        fixture.cli.files_return(["api/src/A.java"]);

        let error = fixture.operation().build("project_0001", 3).unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "aggregate_index_member_coverage_failed"),
            "unexpected error: {error}"
        );
        assert!(
            error.to_string().contains("members [web]"),
            "must name the uncovered member list, got: {error}"
        );
    }

    /// ② 防漂移（探测路径）：durable Active 记录与磁盘库漂移——
    /// `.codegraph` 被 清理/tmp 重建）后，stale→sync 对未初始化库是死路；
    /// Sync 模式必须探测库目录缺失并回退 init＋sync 自愈。
    #[test]
    fn sync_reinitializes_when_index_library_drifted_away() {
        let fixture = aggregate_index_fixture();
        let active = fixture.persist_active_index();
        fixture.cli.requests(); // drain 建库期请求

        let refreshed = fixture
            .operation()
            .sync_and_verify("project_0001", active)
            .expect("sync must self-heal via init when the library drifted away");
        assert_eq!(refreshed.status, AggregateIndexStatus::Active);
        let commands = codegraph_index_commands(
            fixture
                .cli
                .requests()
                .into_iter()
                .map(|request| request.argv)
                .collect(),
        );
        // 探测到 `.codegraph` 缺失：直接 init＋sync，不先跑必败的 sync。
        assert_eq!(
            commands,
            vec![
                vec!["init".to_string(), ".".to_string()],
                vec!["sync".to_string(), ".".to_string()],
            ]
        );
    }

    /// ② 防漂移（捕获路径）：`.codegraph` 目录存在但库未初始化时，
    /// 捕获 `✗ CodeGraph not initialized` 失败回退 init＋sync。
    #[test]
    fn sync_recovers_from_not_initialized_failure() {
        let fixture = aggregate_index_fixture();
        let root = fixture
            .logical
            .load_manifest("project_0001")
            .unwrap()
            .unwrap()
            .provider_context_root;
        std::fs::create_dir_all(root.join(".codegraph")).unwrap();
        let active = fixture.persist_active_index();
        fixture.cli.requests();
        fixture
            .cli
            .fail_next_sync_not_initialized("✗ CodeGraph not initialized in /aggregate");

        let refreshed = fixture
            .operation()
            .sync_and_verify("project_0001", active)
            .expect("sync must fall back to init after a not-initialized failure");
        assert_eq!(refreshed.status, AggregateIndexStatus::Active);
        let commands = codegraph_index_commands(
            fixture
                .cli
                .requests()
                .into_iter()
                .map(|request| request.argv)
                .collect(),
        );
        assert_eq!(
            commands,
            vec![
                vec!["sync".to_string(), ".".to_string()],
                vec!["init".to_string(), ".".to_string()],
                vec!["sync".to_string(), ".".to_string()],
            ]
        );
    }

    fn codegraph_index_commands(argvs: Vec<Vec<String>>) -> Vec<Vec<String>> {
        argvs
            .into_iter()
            .filter(|argv| matches!(argv.first().map(String::as_str), Some("init" | "sync")))
            .collect()
    }

    /// G4（终局关闸缺口）：真实业务仓不含 canonical fixture 预置的
    /// `crossRepoGreeting` 语料——激活不得依赖任何硬编码符号命中。
    /// 真实聚合语义＝成员覆盖（每个 included member 均有索引文件）＋
    /// 负向排除查询（越界作用域零命中）；无符号语料即激活。
    #[test]
    fn build_succeeds_without_canonical_fixture_symbol() {
        let fixture = aggregate_index_fixture();
        fixture.cli.files_return(["api/src/A.java", "web/src/B.ts"]);
        for query in EXCLUDED_QUERIES {
            fixture.cli.query_returns(query, serde_json::json!([]));
        }
        // 刻意不 script crossRepoGreeting（fake 对未 script 查询返回 []，
        // 等价于真实业务仓无该符号）。
        let record = fixture.operation().build("project_0001", 3).unwrap();
        assert_eq!(record.status, AggregateIndexStatus::Active);
    }

    /// G4：codegraph 1.6 实证——对已存在的索引库重复 `init` 不重扫新增
    /// 成员目录，`sync` 才做增量重扫。Initialize/Rebuild 的命令序列必须
    /// 以增量 `sync` 收尾，保证「库已存在＋成员集变化」（补注册成员后的
    /// rebuild / 失败后的 bootstrap retry）能把新成员纳入覆盖。
    #[test]
    fn build_and_rebuild_rescan_members_with_incremental_sync() {
        fn index_commands(argvs: Vec<Vec<String>>) -> Vec<Vec<String>> {
            argvs
                .into_iter()
                .filter(|argv| matches!(argv.first().map(String::as_str), Some("init" | "sync")))
                .collect()
        }

        let fixture = aggregate_index_fixture();
        fixture.cli.files_return(["api/src/A.java", "web/src/B.ts"]);
        for query in EXCLUDED_QUERIES {
            fixture.cli.query_returns(query, serde_json::json!([]));
        }
        let record = fixture.operation().build("project_0001", 3).unwrap();
        assert_eq!(record.status, AggregateIndexStatus::Active);
        let commands = index_commands(
            fixture
                .cli
                .requests()
                .into_iter()
                .map(|request| request.argv)
                .collect(),
        );
        assert_eq!(
            commands,
            vec![
                vec!["init".to_string(), ".".to_string()],
                vec!["sync".to_string(), ".".to_string()],
            ]
        );

        fixture.cli.files_return(["api/src/A.java", "web/src/B.ts", "zeta/src/Z.rs"]);
        let rebuilt = fixture.operation().rebuild("project_0001").unwrap();
        assert_eq!(rebuilt.status, AggregateIndexStatus::Active);
        let rebuild_commands = index_commands(
            fixture
                .cli
                .requests()
                .into_iter()
                .map(|request| request.argv)
                .collect(),
        );
        assert_eq!(
            rebuild_commands,
            vec![
                vec!["init".to_string(), ".".to_string()],
                vec!["sync".to_string(), ".".to_string()],
            ]
        );
    }

    #[test]
    fn build_rejects_invalid_project_id_and_membership_revision_before_cli() {
        let fixture = aggregate_index_fixture();
        assert!(matches!(
            fixture.operation().build("../project_0001", 3),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_store_error"
        ));
        assert!(matches!(
            fixture.operation().build("project_0001", 2),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_membership_revision_mismatch"
        ));
        assert!(fixture.cli.requests().is_empty());
    }

    #[test]
    fn build_requires_an_available_main_checkout() {
        let fixture = aggregate_index_fixture();
        let mut checkout = fixture
            .logical
            .list_checkouts("project_0001")
            .unwrap()
            .remove(0);
        checkout.availability = CheckoutAvailability::Missing;
        fixture
            .logical
            .save_checkout("project_0001", &checkout)
            .unwrap();

        assert!(matches!(
            fixture.operation().build("project_0001", 3),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_member_invalid"
        ));
        let requests = fixture.cli.requests();
        assert!(requests.is_empty());
    }

    #[test]
    fn rebuild_drift_marks_new_record_stale_and_keeps_before_active() {
        let fixture = aggregate_index_fixture();
        let active = fixture.persist_active_index();
        fixture.cli.drift_on_init();

        let error = fixture.operation().rebuild("project_0001").unwrap_err();
        assert!(matches!(
            error,
            AggregateIndexError::Failed { code: "aggregate_index_member_drifted", .. }
        ));
        assert_eq!(
            fixture
                .store()
                .active_required("project_0001")
                .unwrap()
                .aggregate_index_id,
            active.aggregate_index_id
        );
        let stale = fixture
            .records("project_0001")
            .into_iter()
            .find(|record| record.aggregate_index_id != active.aggregate_index_id)
            .unwrap();
        assert_eq!(stale.status, AggregateIndexStatus::Stale);
        assert_eq!(stale.member_snapshots[0].revision, "a".repeat(40));
        assert_eq!(
            stale.observed_after_member_snapshots[0].revision,
            "b".repeat(40)
        );
    }

    #[test]
    fn sync_drift_marks_new_record_stale_and_keeps_before_active() {
        let fixture = aggregate_index_fixture();
        let active = fixture.persist_active_index();
        fixture.cli.drift_on_sync();

        let error = fixture
            .operation()
            .sync_and_verify("project_0001", active.clone())
            .unwrap_err();
        assert!(matches!(
            error,
            AggregateIndexError::Failed { code: "aggregate_index_member_drifted", .. }
        ));
        assert_eq!(
            fixture
                .store()
                .active_required("project_0001")
                .unwrap()
                .aggregate_index_id,
            active.aggregate_index_id
        );
        let stale = fixture
            .records("project_0001")
            .into_iter()
            .find(|record| record.aggregate_index_id != active.aggregate_index_id)
            .unwrap();
        assert_eq!(stale.status, AggregateIndexStatus::Stale);
        assert_eq!(stale.member_snapshots[0].revision, "a".repeat(40));
        assert_eq!(
            stale.observed_after_member_snapshots[0].revision,
            "b".repeat(40)
        );
    }

    #[test]
    fn failed_rebuild_keeps_last_known_good_readable_and_marks_new_record_stale() {
        let fixture = aggregate_index_fixture();
        let active = fixture.persist_active_index();
        fixture.cli.fail_next_init("parser crashed");

        let error = fixture.operation().rebuild("project_0001").unwrap_err();
        assert!(
            matches!(error, AggregateIndexError::Failed { code, .. } if code == "codegraph_init_failed")
        );
        let preserved = fixture.store().active_required("project_0001").unwrap();
        assert_eq!(preserved.aggregate_index_id, active.aggregate_index_id);
        // C4 Task 5：CLI/工具面刷新失败保留 LKG 并标记 degraded（可读 + 审计 warning）。
        assert_eq!(preserved.status, AggregateIndexStatus::Degraded);
        assert!(preserved
            .warning
            .as_deref()
            .unwrap_or_default()
            .contains("parser crashed"));
        let records = fixture.records("project_0001");
        assert!(records.iter().any(|record| {
            record.aggregate_index_id != active.aggregate_index_id
                && record.status == AggregateIndexStatus::Stale
        }));
        assert!(fixture.read_only_planner_can_read("project_0001"));
    }

    #[test]
    fn rebuild_succeeds_and_supersedes_the_prior_active_record() {
        let fixture = aggregate_index_fixture();
        let first = fixture.persist_active_index();

        // The single-writer rebuild path publishes a new active record that
        // supersedes the prior one; the old generation becomes superseded.
        let rebuilt = fixture.operation().rebuild("project_0001").unwrap();
        assert_eq!(rebuilt.status, AggregateIndexStatus::Active);
        assert_ne!(rebuilt.aggregate_index_id, first.aggregate_index_id);
        assert_eq!(
            rebuilt.supersedes_aggregate_index_id.as_deref(),
            Some(first.aggregate_index_id.as_str())
        );
        let prior = fixture
            .store()
            .get("project_0001", &first.aggregate_index_id)
            .unwrap()
            .unwrap();
        assert_eq!(prior.status, AggregateIndexStatus::Superseded);
    }

    #[test]
    fn rebuild_rejects_invalid_project_id_before_acquiring_the_single_writer_lock() {
        let fixture = aggregate_index_fixture();

        assert!(matches!(
            fixture.operation().rebuild("../project_0001"),
            Err(AggregateIndexError::Failed { code, .. }) if code == "aggregate_index_store_error"
        ));
        // The lock path validation happens before any CodeGraph request.
        assert!(fixture.cli.requests().is_empty());
    }

    #[test]
    fn concurrent_rebuilds_are_serialized_under_the_single_writer_lock() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Condvar, Mutex};

        let fixture = aggregate_index_fixture();
        fixture.persist_active_index();

        // Hold the single-writer lock on the exact path the rebuild uses so the
        // in-flight rebuild must wait for it to release rather than racing the
        // active pointer. `with_exact_exclusive_lock` locks the path verbatim
        // (unlike the target-derived `ExclusiveFileLock`), so the holder must
        // use the same primitive to actually contend on the same flock.
        use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
        let lock_path = fixture.store().lock_path("project_0001").unwrap();

        // Synchronization pair so the holder signals once it holds the flock
        // and the main thread signals back when it may release.
        let held = Arc::new((Mutex::new(false), Condvar::new()));
        let release = Arc::new((Mutex::new(false), Condvar::new()));

        let holder_held = Arc::clone(&held);
        let holder_release = Arc::clone(&release);
        let holder_path = lock_path.clone();
        let holder = std::thread::spawn(move || {
            with_exact_exclusive_lock(&holder_path, || {
                let (lock, cvar) = &*holder_held;
                *lock.lock().unwrap() = true;
                cvar.notify_one();
                // Wait for the main thread to confirm the rebuild is blocked.
                let (rlock, rcvar) = &*holder_release;
                let mut released = rlock.lock().unwrap();
                while !*released {
                    released = rcvar.wait(released).unwrap();
                }
                Ok::<(), ProductStoreError>(())
            })
            .unwrap();
        });

        // Wait until the holder reports it holds the flock.
        {
            let (lock, cvar) = &*held;
            let mut held_flag = lock.lock().unwrap();
            while !*held_flag {
                held_flag = cvar.wait(held_flag).unwrap();
            }
        }

        let rebuild_done = Arc::new(AtomicBool::new(false));
        let fixture_cli = fixture.cli.clone();
        let fixture_store = fixture.store.clone();
        let fixture_logical = fixture.logical.clone();
        let snapshot_logical = fixture.logical.clone();
        let snapshot_runner = fixture.cli.clone();
        let cli_executable = "codegraph".to_string();
        let done_flag = Arc::clone(&rebuild_done);
        std::thread::spawn(move || {
            let operation = AggregateIndexOperation::with_snapshot_dependencies(
                fixture_logical,
                fixture_store,
                CodeGraphCli::new(fixture_cli, cli_executable),
                CodeGraphExcludeGenerator,
                AggregateIndexSnapshotCollector::with_dependencies(
                    snapshot_logical,
                    snapshot_runner,
                ),
            );
            let _ = operation.rebuild("project_0001");
            done_flag.store(true, Ordering::SeqCst);
        });

        // The rebuild cannot complete while the sibling thread holds the lock.
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(
            !rebuild_done.load(Ordering::SeqCst),
            "rebuild completed while the single-writer lock was held elsewhere"
        );

        // Release the holder; the rebuild proceeds and reports completion.
        {
            let (lock, cvar) = &*release;
            *lock.lock().unwrap() = true;
            cvar.notify_one();
        }
        for _ in 0..40 {
            if rebuild_done.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            rebuild_done.load(Ordering::SeqCst),
            "rebuild never completed after the single-writer lock was released"
        );
        holder.join().unwrap();
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        logical: LogicalCodebaseStore,
        store: AggregateIndexStore,
        cli: Arc<FakeCodeGraphRunner>,
    }

    impl Fixture {
        fn operation(&self) -> AggregateIndexOperation {
            AggregateIndexOperation::with_snapshot_dependencies(
                self.logical.clone(),
                self.store.clone(),
                CodeGraphCli::new(self.cli.clone(), "codegraph".to_string()),
                CodeGraphExcludeGenerator,
                AggregateIndexSnapshotCollector::with_dependencies(
                    self.logical.clone(),
                    self.cli.clone(),
                ),
            )
        }

        fn config_digest(&self) -> String {
            let config: super::super::CodeGraphConfig = serde_json::from_slice(
                &std::fs::read(
                    self.logical
                        .load_manifest("project_0001")
                        .unwrap()
                        .unwrap()
                        .provider_context_root
                        .join("codegraph.json"),
                )
                .unwrap(),
            )
            .unwrap();
            format!(
                "sha256:{:x}",
                sha2::Sha256::digest(serde_json::to_vec_pretty(&config).unwrap())
            )
        }

        /// Handle to the durable store for direct assertions in failure-recovery tests.
        fn store(&self) -> &AggregateIndexStore {
            &self.store
        }

        fn records(&self, project_id: &str) -> Vec<AggregateIndexRecord> {
            let root = self.paths.aggregate_indexes_root(project_id);
            let mut records = std::fs::read_dir(root)
                .unwrap()
                .filter_map(Result::ok)
                .filter_map(|entry| std::fs::read(entry.path()).ok())
                .map(|bytes| serde_json::from_slice(&bytes).unwrap())
                .collect::<Vec<AggregateIndexRecord>>();
            records.sort_by(|left, right| left.aggregate_index_id.cmp(&right.aggregate_index_id));
            records
        }

        /// Publishes a verified active record so a subsequent failing rebuild has a
        /// last-known-good generation to preserve.
        fn persist_active_index(&self) -> AggregateIndexRecord {
            self.cli.files_return(["api/src/A.java", "web/src/B.ts"]);
            self.cli.query_returns(
                "crossRepoGreeting",
                serde_json::json!([{"file":"api/src/A.java"}, {"file":"web/src/B.ts"}]),
            );
            self.cli
                .query_returns("SHOULD_NOT_INDEX_NONMEMBER", serde_json::json!([]));
            self.cli
                .query_returns("SHOULD_NOT_INDEX_WORKTREE", serde_json::json!([]));
            self.cli
                .query_returns("SHOULD_NOT_INDEX_ARIA", serde_json::json!([]));
            self.cli
                .query_returns("SHOULD_NOT_INDEX_BUILD", serde_json::json!([]));
            self.operation().build("project_0001", 3).unwrap()
        }

        /// Mirrors the read-only planner entry point: a degraded/stale last-known-good
        /// remains readable (with its warning) while building/failed/superseded
        /// generations are never served.
        fn read_only_planner_can_read(&self, project_id: &str) -> bool {
            self.store().active_required(project_id).is_ok()
        }
    }

    fn aggregate_index_fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("aggregate");
        std::fs::create_dir_all(root.join("api/src")).unwrap();
        std::fs::create_dir_all(root.join("web/src")).unwrap();
        std::fs::create_dir_all(root.join("not-a-repo/src")).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let logical = LogicalCodebaseStore::new(paths.clone());
        let store = AggregateIndexStore::new(paths.clone());
        let api = member_with_checkout(&root, "api", "a".repeat(40));
        let web = member_with_checkout(&root, "web", "b".repeat(40));
        let mut manifest = LogicalCodebaseManifest::new(
            "project_0001",
            root,
            vec![api.0.logical_repository_id, web.0.logical_repository_id],
        );
        manifest.membership_revision = 3;
        logical.save_manifest("project_0001", &manifest).unwrap();
        logical.save_member("project_0001", &api.0).unwrap();
        logical.save_member("project_0001", &web.0).unwrap();
        logical.save_checkout("project_0001", &api.1).unwrap();
        logical.save_checkout("project_0001", &web.1).unwrap();

        Fixture {
            _temp: temp,
            paths: paths.clone(),
            logical,
            store,
            cli: Arc::new(FakeCodeGraphRunner::default()),
        }
    }

    fn member_with_checkout(
        root: &Path,
        name: &str,
        revision: String,
    ) -> (CodebaseMemberRecord, RepositoryCheckoutRecord) {
        let logical_repository_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let canonical_path = root.join(name);
        let now = "2026-08-09T00:00:00Z".to_string();
        let source_identity = RepositorySourceIdentity {
            scheme: "test".to_string(),
            key_digest: format!("sha256:source-{name}"),
            canonical_git_dir: canonical_path.join(".git"),
            canonical_origin: None,
            first_seen_path_hash: format!("sha256:path-{name}"),
        };
        let member = CodebaseMemberRecord {
            logical_repository_id,
            physical_repository_id: format!("repository_{name}"),
            alias: name.to_string(),
            role: "repository".to_string(),
            ordinal: 1,
            source_identity: source_identity.clone(),
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![checkout_id],
            status: MemberStatus::Active,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let checkout = RepositoryCheckoutRecord {
            checkout_id,
            logical_repository_id,
            physical_repository_id: member.physical_repository_id.clone(),
            kind: CheckoutKind::Main,
            canonical_path,
            checkout_path_hash: format!("sha256:checkout-{name}"),
            git_dir_identity: source_identity.git_dir_identity(),
            revision: Some(revision),
            availability: CheckoutAvailability::Available,
            observed_at: now.clone(),
            created_at: now.clone(),
            updated_at: now,
        };
        (member, checkout)
    }

    #[derive(Default)]
    struct FakeCodeGraphRunner {
        state: Mutex<FakeCodeGraphState>,
    }

    #[derive(Default)]
    struct FakeCodeGraphState {
        files: Vec<PathBuf>,
        /// 复刻 codegraph 1.6 契约缺陷：空索引（0 文件）时 `files --json`
        /// 以 exit 0 输出人读提示而非 JSON 数组（现场 /tmp/notify-center）。
        files_empty_index_notice: bool,
        queries: BTreeMap<String, Value>,
        requests: Vec<BoundedCommandRequest>,
        /// When set, the next `init` invocation fails with this stderr and a
        /// non-zero exit, simulating a CodeGraph parser crash mid-rebuild.
        fail_next_init: Option<String>,
        /// When set, the next `sync` invocation exits 1 with this stderr,
        /// simulating `✗ CodeGraph not initialized` on a drifted-away library.
        fail_next_sync_not_initialized: Option<String>,
        drift_on_init: bool,
        drift_on_sync: bool,
        revision_overrides: BTreeMap<String, String>,
        init_observer: Option<Arc<dyn Fn() + Send + Sync>>,
    }

    impl FakeCodeGraphRunner {
        fn files_return<I, P>(&self, paths: I)
        where
            I: IntoIterator<Item = P>,
            P: AsRef<Path>,
        {
            let mut state = self.state.lock();
            state.files_empty_index_notice = false;
            state.files = paths
                .into_iter()
                .map(|path| path.as_ref().to_path_buf())
                .collect();
        }

        /// README-only 空仓形态：init 建出 0 文件库，`files --json` 输出
        /// 上游人读提示。
        fn files_return_empty_index_notice(&self) {
            let mut state = self.state.lock();
            state.files_empty_index_notice = true;
            state.files.clear();
        }

        fn query_returns(&self, query: &str, value: Value) {
            self.state.lock().queries.insert(query.to_string(), value);
        }

        fn fail_next_init(&self, stderr: &str) {
            self.state.lock().fail_next_init = Some(stderr.to_string());
        }

        fn fail_next_sync_not_initialized(&self, stderr: &str) {
            self.state.lock().fail_next_sync_not_initialized = Some(stderr.to_string());
        }

        fn drift_on_init(&self) {
            self.state.lock().drift_on_init = true;
        }

        fn drift_on_sync(&self) {
            self.state.lock().drift_on_sync = true;
        }

        fn observe_init(&self, observer: Arc<dyn Fn() + Send + Sync>) {
            self.state.lock().init_observer = Some(observer);
        }

        fn requests(&self) -> Vec<BoundedCommandRequest> {
            std::mem::take(&mut self.state.lock().requests)
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for FakeCodeGraphRunner {
        async fn run(
            &self,
            request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            let mut state = self.state.lock();
            state.requests.push(request.clone());
            // Bug45/T4 修复轮:平台 git argv 以受信禁桥参数开头(见
            // snapshot.rs 生产 git()),codegraph 请求不受影响;先断言并
            // 剥离 git 禁桥前缀,再按同一 argv 匹配面分发。
            let bridge = crate::cross_cutting::git_command::TRUSTED_GIT_BRIDGE_ARGS;
            let bridge_len = bridge.len();
            let argv: &[String] = if request.executable == "git" {
                let prefixed = request.argv.len() > bridge_len
                    && request.argv[..bridge_len]
                        .iter()
                        .zip(bridge.iter())
                        .all(|(actual, expected)| actual == expected);
                assert!(
                    prefixed,
                    "git argv missing trusted bridge prefix: {:?}",
                    request.argv
                );
                &request.argv[bridge_len..]
            } else {
                request.argv.as_slice()
            };
            if argv == ["init", "."] {
                if state.drift_on_init {
                    state
                        .revision_overrides
                        .insert("api".to_string(), "b".repeat(40));
                    state.drift_on_init = false;
                }
                if let Some(observer) = state.init_observer.as_ref() {
                    observer();
                }
            }
            if argv == ["sync", "."] && state.drift_on_sync {
                state
                    .revision_overrides
                    .insert("api".to_string(), "b".repeat(40));
                state.drift_on_sync = false;
            }
            // A scripted init failure short-circuits before any other argv match
            // so the rebuild path observes a non-zero `codegraph init` exit.
            if let [init, dot] = argv
                && init == "init"
                && dot == "."
                && let Some(stderr) = state.fail_next_init.take()
            {
                return Ok(BoundedCommandResult {
                    exit_code: Some(2),
                    stdout: String::new(),
                    stderr,
                    timed_out: false,
                    cancelled: false,
                    stdout_truncated: false,
                    stderr_truncated: false,
                    duration_ms: 1,
                });
            }
            // 漂移态现场：库不存在时 `sync` exit 1（`✗ CodeGraph not
            // initialized …`，stderr）。
            if let [sync, dot] = argv
                && sync == "sync"
                && dot == "."
                && let Some(stderr) = state.fail_next_sync_not_initialized.take()
            {
                return Ok(BoundedCommandResult {
                    exit_code: Some(1),
                    stdout: String::new(),
                    stderr,
                    timed_out: false,
                    cancelled: false,
                    stdout_truncated: false,
                    stderr_truncated: false,
                    duration_ms: 1,
                });
            }
            let stdout = match argv {
                [version] if version == "--version" => "1.6.2\n".to_string(),
                [rev_parse, head] if rev_parse == "rev-parse" && head == "HEAD" => {
                    match request
                        .working_dir
                        .file_name()
                        .and_then(|name| name.to_str())
                    {
                        Some(name) if state.revision_overrides.contains_key(name) => state
                            .revision_overrides
                            .get(name)
                            .cloned()
                            .unwrap(),
                        Some("api") => "a".repeat(40),
                        Some("web") => "b".repeat(40),
                        name => panic!("unexpected fake Git checkout: {name:?}"),
                    }
                }
                [status, porcelain] if status == "status" && porcelain == "--porcelain=v1" => {
                    String::new()
                }
                [init, dot] if init == "init" && dot == "." => "Indexed 2 files\n".to_string(),
                [sync, dot] if sync == "sync" && dot == "." => "Synced 2 files\n".to_string(),
                [files, json] if files == "files" && json == "--json" => {
                    // 空索引形态复刻上游契约缺陷：exit 0 + 人读提示（非 JSON）。
                    if state.files_empty_index_notice {
                        "ℹ No files indexed. Run \"codegraph index\" first.".to_string()
                    } else {
                        serde_json::to_string(
                            &state
                                .files
                                .iter()
                                .map(|path| serde_json::json!({"path": path}))
                                .collect::<Vec<_>>(),
                        )
                        .unwrap()
                    }
                }
                [query, symbol, json] if query == "query" && json == "--json" => state
                    .queries
                    .get(symbol)
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([]))
                    .to_string(),
                argv => panic!("unexpected fake CodeGraph argv: {argv:?}"),
            };
            Ok(BoundedCommandResult {
                exit_code: Some(0),
                stdout,
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                duration_ms: 1,
            })
        }
    }

    #[test]
    fn same_index_command_replays_active_record_and_conflicting_revision_fails_closed() {
        let fixture = aggregate_index_fixture();
        fixture.cli.files_return(["api/src/A.java", "web/src/B.ts"]);
        for query in [
            "SHOULD_NOT_INDEX_NONMEMBER",
            "SHOULD_NOT_INDEX_WORKTREE",
            "SHOULD_NOT_INDEX_ARIA",
            "SHOULD_NOT_INDEX_BUILD",
        ] {
            fixture.cli.query_returns(query, serde_json::json!([]));
        }
        let command_id = "aggregate_index_cmd_boot_0002";

        let first = fixture
            .operation()
            .build_with_command_id("project_0001", 3, command_id)
            .unwrap();
        assert_eq!(first.status, AggregateIndexStatus::Active);
        assert_eq!(first.command_id.as_deref(), Some(command_id));

        // 同 command 重放：返回同一 durable record，不再发任何 CLI 请求
        // （`requests()` 为 drain 型：先清空再断言 replay 零新增）。
        fixture.cli.requests();
        let replayed = fixture
            .operation()
            .build_with_command_id("project_0001", 3, command_id)
            .unwrap();
        assert_eq!(replayed.aggregate_index_id, first.aggregate_index_id);
        assert!(fixture.cli.requests().is_empty());

        // 同 command 不同 expected revision → fail-closed，不落新 generation。
        let records_before = fixture.records("project_0001").len();
        let error = fixture
            .operation()
            .build_with_command_id("project_0001", 4, command_id)
            .unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "aggregate_index_command_revision_mismatch"),
            "unexpected error: {error:?}"
        );
        assert_eq!(fixture.records("project_0001").len(), records_before);
    }

    #[test]
    fn first_build_failure_persists_failed_generation_and_command_replay_returns_same_failure() {
        let fixture = aggregate_index_fixture();
        fixture.cli.fail_next_init("cli exploded on first build");
        let command_id = "aggregate_index_cmd_first_0001";

        let error = fixture
            .operation()
            .build_with_command_id("project_0001", 3, command_id)
            .unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "codegraph_init_failed"),
            "unexpected error: {error:?}"
        );

        // durable：Building→Failed generation 带 command identity，active 指针为空。
        let failed = fixture
            .records("project_0001")
            .into_iter()
            .find(|record| record.status == AggregateIndexStatus::Failed)
            .expect("failed first-build generation must be durable");
        assert_eq!(failed.command_id.as_deref(), Some(command_id));
        assert!(fixture.store().active("project_0001").unwrap().is_none());

        // 同 command 重放：不再发 CLI，返回同一失败事实。
        fixture.cli.requests();
        let replayed = fixture
            .operation()
            .build_with_command_id("project_0001", 3, command_id)
            .unwrap_err();
        assert!(
            matches!(&replayed, AggregateIndexError::Failed { code, .. }
                if *code == "aggregate_index_command_replayed_failure"),
            "unexpected replay error: {replayed:?}"
        );
        assert!(replayed.to_string().contains("codegraph_init_failed"));
        assert!(fixture.cli.requests().is_empty());
    }

    #[test]
    fn rebuild_failure_keeps_last_known_good_as_degraded() {
        let fixture = aggregate_index_fixture();
        let active = fixture.persist_active_index();
        fixture.cli.fail_next_init("cli exploded during refresh");

        let error = fixture.operation().rebuild("project_0001").unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "codegraph_init_failed"),
            "unexpected error: {error:?}"
        );

        // LKG 保留并标记 degraded，warning 含失败原因。
        let preserved = fixture.store().active_required("project_0001").unwrap();
        assert_eq!(preserved.aggregate_index_id, active.aggregate_index_id);
        assert_eq!(preserved.status, AggregateIndexStatus::Degraded);
        assert!(preserved
            .warning
            .as_deref()
            .unwrap_or_default()
            .contains("cli exploded during refresh"));
        assert!(fixture.read_only_planner_can_read("project_0001"));

        // freshness 不把 degraded LKG 重新评估为 active。
        let service = crate::product::logical_codebase::aggregate_index::AggregateIndexFreshnessService::new(
            fixture.operation(),
        );
        let assessed = service.assess("project_0001").unwrap();
        assert_eq!(assessed.status, AggregateIndexStatus::Degraded);
        assert!(assessed.reason.contains("cli exploded during refresh"));
    }

    #[test]
    fn activation_writes_back_manifest_active_aggregate_index_pointer() {
        let fixture = aggregate_index_fixture();
        // 前置＝生产缺口形态：激活成功前 manifest 指针为 None。
        let before = fixture
            .logical
            .load_manifest("project_0001")
            .unwrap()
            .unwrap();
        assert_eq!(before.active_aggregate_index_id, None);

        // 首次激活：指针在该世代成为 active 的那一刻回写为它的 id。
        let first = fixture.persist_active_index();
        let after_first = fixture
            .logical
            .load_manifest("project_0001")
            .unwrap()
            .unwrap();
        assert_eq!(
            after_first.active_aggregate_index_id.as_deref(),
            Some(first.aggregate_index_id.as_str())
        );
        // 回写不动成员面：membership_revision / member_ids 原样。
        assert_eq!(after_first.membership_revision, before.membership_revision);
        assert_eq!(after_first.member_ids, before.member_ids);

        // rebuild 换代：旧世代被翻 Superseded，指针随最新 active 更新，
        // 与 durable active 世代一致（patrol 引用巡检口径可解析）。
        fixture.cli.files_return(["api/src/A.java", "web/src/B.ts"]);
        for query in EXCLUDED_QUERIES {
            fixture.cli.query_returns(query, serde_json::json!([]));
        }
        let second = fixture.operation().rebuild("project_0001").unwrap();
        assert_eq!(second.status, AggregateIndexStatus::Active);
        assert_ne!(second.aggregate_index_id, first.aggregate_index_id);
        assert_eq!(
            second.supersedes_aggregate_index_id.as_deref(),
            Some(first.aggregate_index_id.as_str())
        );
        let after_second = fixture
            .logical
            .load_manifest("project_0001")
            .unwrap()
            .unwrap();
        assert_eq!(
            after_second.active_aggregate_index_id.as_deref(),
            Some(second.aggregate_index_id.as_str())
        );
        assert_eq!(
            fixture
                .store
                .active("project_0001")
                .unwrap()
                .unwrap()
                .aggregate_index_id,
            second.aggregate_index_id
        );
    }
}
