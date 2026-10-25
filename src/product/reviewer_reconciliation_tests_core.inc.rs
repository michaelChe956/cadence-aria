    use super::*;
    use crate::cross_cutting::provider_resources::{
        ProviderRunResourceScope, ResourceScopeKey, new_boot_identity,
    };
    use crate::product::coding_models::ReviewReportAcceptance;
    use crate::product::reviewer_snapshot::{
        PlatformWritePath, REVIEWER_BASELINE_SCHEMA, RestoreCursor, RestorePhase,
        ReviewWorktreeLockIdentity, ReviewerSnapshotStore, SnapshotBudget,
    };
    use crate::product::reviewer_window::{
        ViolationRecord, begin_quiesce, begin_review_window, close_accepted,
        record_report_acceptance, record_violation, resolve_manual_gate, restore_window,
    };
    use crate::product::reviewer_window_store::RestoreBinding;
    use crate::product::reviewer_window_store::{
        BaselineBinding, ReportAcceptance, ReviewerWindowStore, ViolationJournalEntry,
        wire_acceptance,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    // ---------- fixtures ----------

    fn t18_dir() -> tempfile::TempDir {
        let root = Path::new("/tmp/aria-bug45/t18");
        fs::create_dir_all(root).expect("create /tmp/aria-bug45/t18");
        tempfile::tempdir_in(root).expect("t18 fixture dir")
    }

    fn git_ok(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {:?} in {} failed: {}",
            args,
            dir.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn git_sha(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("spawn git");
        assert!(out.status.success(), "git {args:?} failed");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn sha256_of_file(path: &Path) -> String {
        use sha2::{Digest, Sha256};
        let bytes = fs::read(path).unwrap_or_default();
        hex::encode(Sha256::digest(&bytes))
    }

    struct Fixture {
        worktree: PathBuf,
        window_root: PathBuf,
        snapshot_root: PathBuf,
        base: PathBuf,
        _keep: Vec<tempfile::TempDir>,
    }

    impl Fixture {
        fn store(&self) -> ReviewerWindowStore {
            ReviewerWindowStore::new(&self.window_root)
        }

        fn snapshot(&self) -> ReviewerSnapshotStore {
            ReviewerSnapshotStore::new(&self.snapshot_root)
        }
    }

    fn init_repo(worktree: &Path) {
        fs::write(worktree.join("tracked.txt"), "base\n").unwrap();
        git_ok(worktree, &["init", "-q", "-b", "main"]);
        git_ok(worktree, &["config", "user.email", "t18@aria.invalid"]);
        git_ok(worktree, &["config", "user.name", "t18-fixture"]);
        git_ok(worktree, &["add", "-A"]);
        git_ok(worktree, &["commit", "-qm", "base"]);
    }

    fn fixture() -> Fixture {
        let base = t18_dir();
        let win = t18_dir();
        let snap = t18_dir();
        let worktree = base.path().join("repo");
        fs::create_dir_all(&worktree).unwrap();
        init_repo(&worktree);
        Fixture {
            worktree,
            window_root: win.path().to_path_buf(),
            snapshot_root: snap.path().to_path_buf(),
            base: base.path().to_path_buf(),
            _keep: vec![base, win, snap],
        }
    }

    fn open_scope(base: &Path, scope_id: &str) -> ProviderRunResourceScope {
        let store_root = base.join("scope-store");
        let private = base.join("private").join(scope_id);
        fs::create_dir_all(&private).unwrap();
        let key = ResourceScopeKey {
            scope_id: scope_id.to_string(),
            run_id: format!("run-{scope_id}"),
            boot_identity: new_boot_identity(),
        };
        ProviderRunResourceScope::open_for_recovery(
            &key,
            Some("w-t18"),
            &private,
            &store_root,
            None,
        )
        .expect("open scope")
    }

    fn begin_request<'a>(
        window_id: &'a str,
        attempt: &'a str,
        worktree: &'a Path,
        targets: &'a [&'a ProviderRunResourceScope],
    ) -> crate::product::reviewer_window::ReviewerWindowBeginRequest<'a> {
        crate::product::reviewer_window::ReviewerWindowBeginRequest {
            window_id,
            attempt_id: attempt,
            run_id: "run-1",
            provider: "claude_code",
            worktree,
            quiesce_targets: targets,
            platform_write_paths: Vec::new(),
            budget: SnapshotBudget::generous(),
            drain_timeout: Duration::from_secs(5),
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(future)
    }

    fn minimal_manifest(
        window_id: &str,
    ) -> crate::product::reviewer_snapshot::ReviewerBaselineManifest {
        crate::product::reviewer_snapshot::ReviewerBaselineManifest {
            schema: REVIEWER_BASELINE_SCHEMA.to_string(),
            window_id: window_id.to_string(),
            attempt_id: "attempt-1".to_string(),
            run_id: "run-1".to_string(),
            lock_identity: ReviewWorktreeLockIdentity {
                canonical_worktree: PathBuf::from("/tmp/aria-bug45/t18/repo"),
                gitdir: PathBuf::from("/tmp/aria-bug45/t18/repo/.git"),
                common_dir: PathBuf::from("/tmp/aria-bug45/t18/repo/.git"),
                lease_id: "lease-1".to_string(),
            },
            entries: Vec::new(),
            git_observation: crate::product::reviewer_snapshot::GitObservationBaseline {
                head_symref: Some("refs/heads/main".to_string()),
                head_sha: "0".repeat(40),
                index_blob: "blobs/0".to_string(),
                index_sha256: "0".repeat(64),
                managed_refs: Vec::new(),
                refs_digest: "0".repeat(64),
                index_semantic_sha256: None,
            },
            platform_write_baseline: Vec::new(),
            scope_key: ResourceScopeKey {
                scope_id: "scope".to_string(),
                run_id: "run-1".to_string(),
                boot_identity: "boot".to_string(),
            },
            scope_manifest: Vec::new(),
            quiesced_at: "2026-10-17T00:00:00Z".to_string(),
            captured_at: "2026-10-17T00:00:00Z".to_string(),
            git_annex_observation: None,
            restore_cursor: RestoreCursor {
                phase: RestorePhase::Completed,
                entry_index: 0,
            },
            ignore_rules_freeze:
                crate::product::reviewer_snapshot_domain::IgnoreRulesFreeze::synthetic(),
            domain_boundary: crate::product::reviewer_snapshot_domain::DOMAIN_BOUNDARY_DISCLOSURE
                .to_string(),
        }
    }

    fn facts_record(window_id: &str) -> ReviewerWindowRecord {
        ReviewerWindowRecord::new(
            window_id,
            ReviewWorktreeLockIdentity {
                canonical_worktree: PathBuf::from("/tmp/aria-bug45/t18/repo"),
                gitdir: PathBuf::from("/tmp/aria-bug45/t18/repo/.git"),
                common_dir: PathBuf::from("/tmp/aria-bug45/t18/repo/.git"),
                lease_id: "lease-1".to_string(),
            },
            "attempt-1",
            "run-1",
            "claude_code",
            Vec::new(),
        )
    }

    fn restored_binding() -> RestoreBinding {
        RestoreBinding {
            completed: true,
            from_violation: false,
            outcome: "restored".to_string(),
            cursor_phase: Some("completed".to_string()),
            entry_index: Some(0),
            detail: "ok".to_string(),
            git_restored_mutation_paths: Vec::new(),
            git_extra_refs_observed: Vec::new(),
            platform_verified: vec!["/platform/report.json".to_string()],
        }
    }

    fn baseline_binding(sha: &str) -> BaselineBinding {
        BaselineBinding {
            manifest_path: "windows/w-1/manifest.json".to_string(),
            manifest_sha256: sha.to_string(),
            entry_count: 0,
            total_bytes: 0,
            captured_at: "2026-10-17T00:00:00Z".to_string(),
        }
    }

    fn journal_entry(seq: u64, window_id: &str, kind: &str) -> ViolationJournalEntry {
        ViolationJournalEntry {
            seq,
            entry_id: format!("entry-{seq}"),
            at: "2026-10-17T00:00:00Z".to_string(),
            window_id: window_id.to_string(),
            attempt_id: "attempt-1".to_string(),
            run_id: "run-1".to_string(),
            report_id: Some("code_review_report_0001".to_string()),
            kind: kind.to_string(),
            path: ".aria/direct.json".to_string(),
            before_digest: None,
            after_digest: Some("sha256:ff".to_string()),
            detail: "直写".to_string(),
            restore_cursor: "started+0".to_string(),
        }
    }

    // ---------- 纯判定矩阵 ----------

    #[test]
    fn pure_clean_completed_facts_recommend_accepted() {
        let manifest = minimal_manifest("w-1");
        let mut record = facts_record("w-1");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(restored_binding());
        record.baseline = Some(baseline_binding(&manifest_fingerprint(&manifest)));
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Loaded(manifest),
        });
        assert!(outcome.worktree.passed);
        assert!(outcome.git.passed);
        assert!(outcome.platform.passed);
        assert!(outcome.records_consistent);
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Accepted { .. }
        ));
    }

    #[test]
    fn pure_restore_error_binding_not_ready_with_cursor() {
        // T16 ⚠2:条目级 Io Err 计入非通过,last_error+游标带出。
        let mut record = facts_record("w-err");
        record.state = ReviewerWindowState::Restoring;
        record.last_error = Some("Io: read blob".to_string());
        record.restore = Some(RestoreBinding {
            completed: false,
            from_violation: false,
            outcome: "error".to_string(),
            cursor_phase: Some("worktree_restore".to_string()),
            entry_index: Some(3),
            detail: "Io: read blob".to_string(),
            git_restored_mutation_paths: Vec::new(),
            git_extra_refs_observed: Vec::new(),
            platform_verified: Vec::new(),
        });
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Missing,
        });
        match outcome.recommendation {
            AcceptanceRecommendation::NotReady { blockers, .. } => {
                assert!(
                    blockers
                        .iter()
                        .any(|blocker| blocker.contains("worktree_restore")),
                    "blockers 须带出游标: {blockers:?}"
                );
            }
            other => panic!("期望 NotReady,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_git_mutation_and_extra_refs_invalidate_round() {
        let mut record = facts_record("w-git");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(RestoreBinding {
            completed: true,
            from_violation: false,
            outcome: "restored".to_string(),
            cursor_phase: Some("completed".to_string()),
            entry_index: Some(0),
            detail: "ok".to_string(),
            git_restored_mutation_paths: vec![".git/HEAD".to_string()],
            git_extra_refs_observed: vec!["refs/heads/evil".to_string()],
            platform_verified: Vec::new(),
        });
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Missing,
        });
        assert!(!outcome.git.passed, "Git 观察域必须不通过");
        assert!(outcome.worktree.passed, "worktree 域独立通过");
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated {
                manual_gate,
                reasons,
                ..
            } => {
                assert!(manual_gate);
                assert!(reasons.iter().any(|reason| reason.contains("自提交")));
                assert!(reasons.iter().any(|reason| reason.contains("改 Git")));
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_journal_seq_gap_tamper_fail_closed() {
        let mut record = facts_record("w-gap");
        record.state = ReviewerWindowState::ManualGate;
        record.restore = Some(RestoreBinding {
            completed: true,
            from_violation: true,
            outcome: "restored".to_string(),
            cursor_phase: Some("completed".to_string()),
            entry_index: Some(0),
            detail: "ok".to_string(),
            git_restored_mutation_paths: Vec::new(),
            git_extra_refs_observed: Vec::new(),
            platform_verified: Vec::new(),
        });
        // seq 断裂(1,3):可观察篡改 → fail-closed 作废。
        let journal = vec![
            journal_entry(1, "w-gap", "platform_domain_write"),
            journal_entry(3, "w-gap", "platform_domain_write"),
        ];
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal,
            manifest: ManifestFacts::Missing,
        });
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated { reasons, .. } => {
                assert!(reasons.iter().any(|reason| reason.contains("序号断裂")));
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_from_violation_with_empty_journal_fail_closed() {
        let mut record = facts_record("w-miss");
        record.state = ReviewerWindowState::ManualGate;
        record.restore = Some(RestoreBinding {
            completed: true,
            from_violation: true,
            outcome: "restored".to_string(),
            cursor_phase: Some("completed".to_string()),
            entry_index: Some(0),
            detail: "ok".to_string(),
            git_restored_mutation_paths: Vec::new(),
            git_extra_refs_observed: Vec::new(),
            platform_verified: Vec::new(),
        });
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Missing,
        });
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated { reasons, .. } => {
                assert!(reasons.iter().any(|reason| reason.contains("journal 为空")));
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_journal_without_binding_whitewash_fail_closed() {
        let mut record = facts_record("w-white");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(restored_binding());
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: vec![journal_entry(1, "w-white", "channel_bypass")],
            manifest: ManifestFacts::Missing,
        });
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated { reasons, .. } => {
                assert!(reasons.iter().any(|reason| reason.contains("洗白")));
                assert!(
                    reasons
                        .iter()
                        .any(|reason| reason.contains("channel_bypass"))
                );
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_fingerprint_mismatch_invalidated() {
        let manifest = minimal_manifest("w-hash");
        let mut record = facts_record("w-hash");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(restored_binding());
        // 故意给错指纹:hash 不一致 → 整轮作废(同源核验 fail-closed)。
        record.baseline = Some(baseline_binding(&"f".repeat(64)));
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Loaded(manifest),
        });
        assert!(!outcome.records_consistent);
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated { reasons, .. } => {
                assert!(reasons.iter().any(|reason| reason.contains("指纹不一致")));
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_platform_allowlist_mismatch_invalidated() {
        let mut manifest = minimal_manifest("w-plat");
        manifest.platform_write_baseline = vec![
            crate::product::reviewer_snapshot::PlatformWriteBaselineEntry {
                path: "/platform/report.json".to_string(),
                ownership: "report-store".to_string(),
                state: crate::product::reviewer_snapshot::PlatformWriteState::File {
                    content_sha256: "0".repeat(64),
                    mode: 0o644,
                },
            },
        ];
        let mut record = facts_record("w-plat");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(restored_binding());
        record.baseline = Some(baseline_binding(&manifest_fingerprint(&manifest)));
        // 记录 allowlist 与基线不一致 → 记录不一致 → 作废。
        record.platform_write_paths = vec![PlatformWritePath {
            path: PathBuf::from("/platform/other.json"),
            ownership: "report-store".to_string(),
        }];
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Loaded(manifest),
        });
        assert!(!outcome.records_consistent);
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated { reasons, .. } => {
                assert!(
                    reasons
                        .iter()
                        .any(|reason| reason.contains("allowlist 不一致"))
                );
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_pending_violation_fail_closed_invalidated() {
        let mut record = facts_record("w-pend");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(restored_binding());
        record.violation_pending = Some(crate::product::reviewer_window_store::ViolationPending {
            entry_id: "e".to_string(),
            kind: "platform_domain_write".to_string(),
            detail: "journal 写失败".to_string(),
        });
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Missing,
        });
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated {
                manual_gate,
                reasons,
                ..
            } => {
                assert!(manual_gate);
                assert!(
                    reasons
                        .iter()
                        .any(|reason| reason.contains("journal 未落盘"))
                );
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_terminal_window_projects_final_acceptance() {
        let mut record = facts_record("w-done");
        record.state = ReviewerWindowState::ClosedAccepted;
        record.report_acceptance = ReportAcceptance::Accepted;
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Missing,
        });
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Accepted { .. }
        ));

        let mut record = facts_record("w-done2");
        record.state = ReviewerWindowState::ClosedInvalidated;
        record.report_acceptance = ReportAcceptance::Invalidated;
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Missing,
        });
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Invalidated { .. }
        ));
    }

    // ---------- E2E(store 实链路) ----------

    #[test]
    fn e2e_build_residue_and_coder_dirty_recovers_to_accepted() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-acc", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");

        // Coder dirty(改 tracked)+ 临时构建残留(untracked)。
        fs::write(fx.worktree.join("tracked.txt"), "coder dirty\n").unwrap();
        fs::create_dir_all(fx.worktree.join("target/debug")).unwrap();
        fs::write(fx.worktree.join("target/debug/residue.o"), "junk\n").unwrap();

        let quiescing = begin_quiesce(&store, "w-acc", handle.generation, "review end").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-acc",
            quiescing.generation,
        ))
        .expect("restore");
        assert!(!summary.manual_gate);

        // 三域对账:可采信。
        let outcome = reconcile_from_stores(&store, &snapshot, "w-acc").expect("reconcile");
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Accepted { .. }
        ));

        // 先 Pending(初值)→ 受信裁决 Accepted → 采信关闭。
        assert_eq!(
            summary.record.report_acceptance,
            ReportAcceptance::PendingAcceptance
        );
        let accepted = record_report_acceptance(
            &store,
            &snapshot,
            "w-acc",
            summary.record.generation,
            ReportAcceptance::Accepted,
        )
        .expect("record accepted");
        assert_eq!(
            wire_acceptance(ReportAcceptance::Accepted),
            ReviewReportAcceptance::Accepted
        );
        let terminal = close_accepted(&store, "w-acc", accepted.generation).expect("close");
        assert_eq!(terminal.state, ReviewerWindowState::ClosedAccepted);
        assert!(store.load_violations("w-acc").unwrap().is_empty());
        assert_eq!(terminal.report_acceptance, ReportAcceptance::Accepted);
    }

    #[test]
    fn e2e_reviewer_self_commit_invalidated_even_after_full_restore() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-selfcommit", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");
        let baseline_head = git_sha(&fx.worktree, &["rev-parse", "HEAD"]);

        // Reviewer 自提交:新分支 + commit(HEAD symref/ref/index/新对象)。
        fs::write(fx.worktree.join("reviewer.txt"), "self commit\n").unwrap();
        git_ok(&fx.worktree, &["checkout", "-q", "-b", "evil"]);
        git_ok(&fx.worktree, &["add", "-A"]);
        git_ok(&fx.worktree, &["commit", "-qm", "reviewer self commit"]);

        let quiescing = begin_quiesce(&store, "w-selfcommit", handle.generation, "end").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-selfcommit",
            quiescing.generation,
        ))
        .expect("restore");
        // 恢复成功也不能洗白:自提交对象存在 → 人工门。
        assert!(summary.manual_gate, "自提交必须进入人工门");
        assert_eq!(summary.record.state, ReviewerWindowState::ManualGate);
        // HEAD 已恢复基线。
        assert_eq!(git_sha(&fx.worktree, &["rev-parse", "HEAD"]), baseline_head);
        let binding = summary.record.restore.as_ref().expect("binding");
        assert!(
            !binding.git_extra_refs_observed.is_empty()
                || !binding.git_restored_mutation_paths.is_empty(),
            "自提交事实必须结构化入绑定: {binding:?}"
        );

        // journal:恢复前事实已落盘且恢复不删除/覆盖。
        let journal = store.load_violations("w-selfcommit").unwrap();
        assert!(!journal.is_empty(), "自提交证据必须入 journal");
        assert!(
            journal
                .iter()
                .any(|entry| entry.kind == "git_extra_ref" || entry.kind == "git_domain_mutation")
        );

        // 三域对账:整轮 Invalidated。
        let outcome = reconcile_from_stores(&store, &snapshot, "w-selfcommit").expect("reconcile");
        match &outcome.recommendation {
            AcceptanceRecommendation::Invalidated { manual_gate, .. } => assert!(manual_gate),
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
        assert!(!outcome.git.passed);

        // 采信关闭被拒;人工作废关闭 → 旧报告永久 Invalidated。
        assert!(close_accepted(&store, "w-selfcommit", summary.record.generation).is_err());
        let terminal = resolve_manual_gate(
            &store,
            "w-selfcommit",
            summary.record.generation,
            crate::product::reviewer_window::ManualGateDecision::Invalidate,
        )
        .expect("manual invalidate");
        assert_eq!(terminal.state, ReviewerWindowState::ClosedInvalidated);
        assert_eq!(terminal.report_acceptance, ReportAcceptance::Invalidated);
        // 恢复后 journal 仍在(不可删除/覆盖)。
        assert!(!store.load_violations("w-selfcommit").unwrap().is_empty());
    }

    #[test]
    fn e2e_legal_platform_report_write_not_flagged() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let platform_dir = fx.base.join("platform");
        fs::create_dir_all(&platform_dir).unwrap();
        let platform_file = platform_dir.join("report.json");
        fs::write(&platform_file, "v0\n").unwrap();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let mut request = begin_request("w-plat-ok", "attempt-1", &fx.worktree, &targets);
        request.platform_write_paths = vec![PlatformWritePath {
            path: platform_file.clone(),
            ownership: "report-store".to_string(),
        }];
        let handle = block_on(begin_review_window(&store, &snapshot, &request)).expect("begin");

        // 合法平台报告写(allowlist 内,平台 actor):内容漂移按 T16 契约
        // 停等(不伪称三域通过),但 T18 不得误判为违规——不落 journal、
        // 对账为 NotReady 而非 Invalidated;平台稳定后重入恢复完成采信。
        fs::write(&platform_file, "v1-legal-report\n").unwrap();
        // 顺带 worktree 脏(可恢复)。
        fs::write(fx.worktree.join("tracked.txt"), "dirty\n").unwrap();

        let quiescing = begin_quiesce(&store, "w-plat-ok", handle.generation, "end").unwrap();
        let err = block_on(restore_window(
            &store,
            &snapshot,
            "w-plat-ok",
            quiescing.generation,
        ))
        .expect_err("平台域漂移按 T16 停等,不伪称完成");
        assert!(
            matches!(
                &err,
                crate::product::reviewer_window::ReviewerWindowError::RecoveryStopped { reason, .. }
                    if reason.contains("platform")
            ),
            "{err:?}"
        );
        // 不误判:无违规 journal、对账 NotReady(停等)而非 Invalidated。
        assert!(store.load_violations("w-plat-ok").unwrap().is_empty());
        let record = store.load_window("w-plat-ok").unwrap();
        assert_eq!(record.state, ReviewerWindowState::RecoveryRequired);
        let outcome = reconcile_from_stores(&store, &snapshot, "w-plat-ok").expect("reconcile");
        match &outcome.recommendation {
            AcceptanceRecommendation::NotReady { blockers, .. } => {
                assert!(
                    blockers
                        .iter()
                        .any(|blocker| blocker.contains("platform_domain")),
                    "blockers: {blockers:?}"
                );
            }
            other => panic!("合法平台写不得判 Invalidated/Accepted: {other:?}"),
        }

        // 平台侧稳定(归属方落定基线态)→ 恢复重入完成 → 三域通过。
        fs::write(&platform_file, "v0\n").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-plat-ok",
            record.generation,
        ))
        .expect("restore retry");
        assert!(!summary.manual_gate, "合法平台写不得误判为违规");
        let binding = summary.record.restore.as_ref().expect("binding");
        assert!(
            binding
                .platform_verified
                .contains(&platform_file.to_string_lossy().to_string())
        );

        let outcome = reconcile_from_stores(&store, &snapshot, "w-plat-ok").expect("reconcile");
        assert!(outcome.platform.passed, "平台合法写入域必须通过");
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Accepted { .. }
        ));
        let accepted = record_report_acceptance(
            &store,
            &snapshot,
            "w-plat-ok",
            summary.record.generation,
            ReportAcceptance::Accepted,
        )
        .expect("accepted");
        assert_eq!(
            close_accepted(&store, "w-plat-ok", accepted.generation)
                .expect("close")
                .state,
            ReviewerWindowState::ClosedAccepted
        );
    }

    #[test]
    fn e2e_observed_violations_journal_fields_and_gate_flow() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-viol2", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");

        let quiescing = begin_quiesce(&store, "w-viol2", handle.generation, "end").unwrap();
        let violated = record_violation(
            &store,
            &snapshot,
            "w-viol2",
            quiescing.generation,
            ViolationRecord {
                kind: "git_domain_write",
                path: ".git/refs/heads/main",
                before_digest: Some("sha256:base"),
                after_digest: Some("sha256:evil"),
                detail: "直接写 .git 受管 ref",
                report_id: Some("code_review_report_0002"),
            },
        )
        .expect("record violation 1");
        assert_eq!(violated.state, ReviewerWindowState::ViolationRecorded);

        // 同一违规重复收口幂等:journal 只有一条。
        let again = record_violation(
            &store,
            &snapshot,
            "w-viol2",
            violated.generation,
            ViolationRecord {
                kind: "git_domain_write",
                path: ".git/refs/heads/main",
                before_digest: Some("sha256:base"),
                after_digest: Some("sha256:evil"),
                detail: "直接写 .git 受管 ref",
                report_id: Some("code_review_report_0002"),
            },
        )
        .expect("record violation idempotent");
        let journal = store.load_violations("w-viol2").unwrap();
        assert_eq!(journal.len(), 1, "同一违规重复收口只落一条: {journal:?}");
        // journal 条目字段完备(brief:路径/前后摘要/时间/window/attempt/
        // run/report/restore_cursor)。
        let entry = &journal[0];
        assert_eq!(entry.seq, 1);
        assert_eq!(entry.window_id, "w-viol2");
        assert_eq!(entry.attempt_id, "attempt-1");
        assert_eq!(entry.run_id, "run-1");
        assert_eq!(entry.report_id.as_deref(), Some("code_review_report_0002"));
        assert_eq!(entry.path, ".git/refs/heads/main");
        assert_eq!(entry.before_digest.as_deref(), Some("sha256:base"));
        assert_eq!(entry.after_digest.as_deref(), Some("sha256:evil"));
        assert!(!entry.at.is_empty());
        assert!(!entry.restore_cursor.is_empty());

        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-viol2",
            again.generation,
        ))
        .expect("restore");
        assert!(summary.manual_gate);
        assert_eq!(summary.record.state, ReviewerWindowState::ManualGate);

        // 人工门期间:补偿通道被冻结拒绝 + 租约保持。
        let common_dir =
            crate::product::reviewer_window::resolve_worktree_git_identity(&fx.worktree)
                .expect("identity")
                .2;
        let err = store
            .enter_write_channel(
                &common_dir,
                crate::product::reviewer_window_store::WriteChannelKind::Compensation,
                crate::product::reviewer_window_store::ChannelWriter {
                    writer_id: "comp-1".to_string(),
                    boot_identity: "boot".to_string(),
                    pid: std::process::id(),
                    start_time_ticks: 0,
                    entered_at: "2026-10-17T00:00:00Z".to_string(),
                },
            )
            .expect_err("补偿通道必须被冻结拒绝");
        assert!(matches!(
            err,
            crate::product::reviewer_window_store::ChannelGateError::Blocked { .. }
        ));
        let empty = open_scope(&fx.base, "coder-2");
        let empty_targets: [&ProviderRunResourceScope; 1] = [&empty];
        assert!(
            block_on(begin_review_window(
                &store,
                &snapshot,
                &begin_request("w-second", "attempt-2", &fx.worktree, &empty_targets),
            ))
            .is_err()
        );

        // 采信写入被拒(违规流只能人工门)。
        assert!(
            record_report_acceptance(
                &store,
                &snapshot,
                "w-viol2",
                summary.record.generation,
                ReportAcceptance::Accepted,
            )
            .is_err()
        );

        let outcome = reconcile_from_stores(&store, &snapshot, "w-viol2").expect("reconcile");
        match &outcome.recommendation {
            AcceptanceRecommendation::Invalidated {
                manual_gate,
                reasons,
                ..
            } => {
                assert!(manual_gate);
                assert!(
                    reasons
                        .iter()
                        .any(|reason| reason.contains("git_domain_write"))
                );
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }

        let terminal = resolve_manual_gate(
            &store,
            "w-viol2",
            summary.record.generation,
            crate::product::reviewer_window::ManualGateDecision::RequireNewReview,
        )
        .expect("require new review");
        assert_eq!(terminal.state, ReviewerWindowState::NewReviewRequired);
        assert_eq!(terminal.report_acceptance, ReportAcceptance::Invalidated);

        // 新轮:同树可开新窗口,新 baseline/新 PendingAcceptance;
        // 旧窗口记录与 journal 永久保留。
        let new_handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-new2", "attempt-2", &fx.worktree, &empty_targets),
        ))
        .expect("new round window");
        let new_record = store.load_window("w-new2").unwrap();
        assert_eq!(
            new_record.report_acceptance,
            ReportAcceptance::PendingAcceptance
        );
        let old = store.load_window("w-viol2").unwrap();
        assert_ne!(
            new_handle.manifest_sha256,
            old.baseline.as_ref().unwrap().manifest_sha256,
            "新轮不得沿用旧 baseline 指纹"
        );
        assert_ne!(
            new_handle.manifest_path,
            old.baseline.as_ref().unwrap().manifest_path,
            "新轮不得沿用旧 manifest"
        );
        assert_eq!(old.report_acceptance, ReportAcceptance::Invalidated);
        assert!(!store.load_violations("w-viol2").unwrap().is_empty());
    }

    #[test]
    fn e2e_journal_write_failure_blocks_restore_until_durable() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-jfail", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");
        let quiescing = begin_quiesce(&store, "w-jfail", handle.generation, "end").unwrap();

        // 注入 journal 写失败:在 journal 文件路径放置目录。
        let journal_path = fx.window_root.join("violations").join("w-jfail.json");
        fs::create_dir_all(&journal_path).unwrap();
        let err = record_violation(
            &store,
            &snapshot,
            "w-jfail",
            quiescing.generation,
            ViolationRecord {
                kind: "platform_domain_write",
                path: ".aria/direct.json",
                before_digest: None,
                after_digest: None,
                detail: "直写 .aria",
                report_id: None,
            },
        )
        .expect_err("journal 写失败必须报错");
        assert!(
            matches!(
                err,
                crate::product::reviewer_window::ReviewerWindowError::Store(_)
            ),
            "{err:?}"
        );

        // 停等标记已持久化;恢复必须不开始;采信关闭被拒。
        let record = store.load_window("w-jfail").unwrap();
        assert!(
            record.violation_pending.is_some(),
            "violation_pending 必须持久化"
        );
        let err = block_on(restore_window(
            &store,
            &snapshot,
            "w-jfail",
            record.generation,
        ))
        .expect_err("journal 未落盘不得开始恢复");
        assert!(
            matches!(
                err,
                crate::product::reviewer_window::ReviewerWindowError::ViolationJournalPending { .. }
            ),
            "{err:?}"
        );
        assert!(close_accepted(&store, "w-jfail", record.generation).is_err());

        // 恢复 journal 可写后重入:幂等续接 → 违规流正常收口。
        fs::remove_dir(&journal_path).unwrap();
        let violated = record_violation(
            &store,
            &snapshot,
            "w-jfail",
            record.generation,
            ViolationRecord {
                kind: "platform_domain_write",
                path: ".aria/direct.json",
                before_digest: None,
                after_digest: None,
                detail: "直写 .aria",
                report_id: None,
            },
        )
        .expect("retry record violation");
        assert_eq!(violated.state, ReviewerWindowState::ViolationRecorded);
        assert!(violated.violation_pending.is_none());
        assert_eq!(store.load_violations("w-jfail").unwrap().len(), 1);
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-jfail",
            violated.generation,
        ))
        .expect("restore");
        assert!(summary.manual_gate);
    }

