    // ---------- 审查修复轮 1(F1-F5)钉测 ----------

    /// F1:派生违规后 journal 写失败/崩溃 → 重入恢复仍观察得到自提交
    /// 事实(extra ref 留仓),不丢失、不误放行 Accepted。
    #[test]
    fn f1_derived_git_facts_survive_journal_failure_and_restore_reentry() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-f1r", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");

        // Reviewer 自提交(新分支+commit,extra ref 留仓)。
        fs::write(fx.worktree.join("r.txt"), "x\n").unwrap();
        git_ok(&fx.worktree, &["checkout", "-q", "-b", "evil"]);
        git_ok(&fx.worktree, &["add", "-A"]);
        git_ok(&fx.worktree, &["commit", "-qm", "self"]);
        let quiescing = begin_quiesce(&store, "w-f1r", handle.generation, "end").unwrap();

        // 注入 journal 写失败:恢复后派生收口落盘失败,停等标记残留。
        let journal_path = fx.window_root.join("violations").join("w-f1r.json");
        fs::create_dir_all(&journal_path).unwrap();
        let err = block_on(restore_window(
            &store,
            &snapshot,
            "w-f1r",
            quiescing.generation,
        ))
        .expect_err("journal 阻断时恢复收口必须失败");
        assert!(
            matches!(
                &err,
                crate::product::reviewer_window::ReviewerWindowError::Store(_)
            ),
            "{err:?}"
        );
        let record = store.load_window("w-f1r").unwrap();
        assert!(record.violation_pending.is_some());
        assert!(
            !record
                .restore
                .as_ref()
                .is_some_and(|binding| binding.completed),
            "journal 未落盘不得宣称恢复完整待采信"
        );

        // 崩溃重入(修复 journal 可写):独立观察重算自提交事实 → 落
        // journal → ManualGate;即使 T16 游标已推进,事实不丢。
        fs::remove_dir(&journal_path).unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-f1r",
            record.generation,
        ))
        .expect("reentry restore");
        assert!(summary.manual_gate, "重入后自提交事实不得丢失");
        let journal = store.load_violations("w-f1r").unwrap();
        assert!(
            journal.iter().any(|entry| entry.kind == "git_extra_ref"),
            "{journal:?}"
        );
    }

    /// F4:直写 .git/config → annex 补充面观察 → 整轮作废进人工门。
    #[test]
    fn f4_direct_git_config_write_detected_via_annex_observation() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-f4", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");

        // 通道冻结窗口内直写 .git/config(hooks/fsmonitor 类逃逸面)。
        let config_path = fx.worktree.join(".git").join("config");
        let original = fs::read_to_string(&config_path).unwrap();
        fs::write(
            &config_path,
            format!("{original}[core]\n\tfsmonitor = evil\n"),
        )
        .unwrap();

        let quiescing = begin_quiesce(&store, "w-f4", handle.generation, "end").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-f4",
            quiescing.generation,
        ))
        .expect("restore");
        assert!(summary.manual_gate, "直写 .git/config 必须进人工门");
        let journal = store.load_violations("w-f4").unwrap();
        assert!(
            journal
                .iter()
                .any(|entry| entry.kind == "git_annex_drift" && entry.path.contains("config")),
            "{journal:?}"
        );
        // 对账:整轮 Invalidated,Git 域不通过。
        let outcome = reconcile_from_stores(&store, &snapshot, "w-f4").unwrap();
        assert!(!outcome.git.passed);
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Invalidated { .. }
        ));
    }

    /// F2:pending 属他人 entry 时 record_violation fail-closed 拒绝,
    /// 不得覆盖;同 entry 重试可推进并清除。
    #[test]
    fn f2_record_violation_never_overwrites_foreign_pending() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-f2", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");
        let quiescing = begin_quiesce(&store, "w-f2", handle.generation, "end").unwrap();

        // 造 pending(A):journal 阻断写入失败但标记已持久化。
        let journal_path = fx.window_root.join("violations").join("w-f2.json");
        fs::create_dir_all(&journal_path).unwrap();
        let violation_a = ViolationRecord {
            kind: "platform_domain_write",
            path: ".aria/a.json",
            before_digest: None,
            after_digest: None,
            detail: "A",
            report_id: None,
        };
        assert!(
            record_violation(&store, &snapshot, "w-f2", quiescing.generation, violation_a).is_err()
        );
        let record = store.load_window("w-f2").unwrap();
        assert!(record.violation_pending.is_some());

        // pending(A) 存在时记 B → ViolationJournalPending 拒绝(A 不被覆盖)。
        let violation_b = ViolationRecord {
            kind: "git_domain_write",
            path: ".git/x",
            before_digest: None,
            after_digest: None,
            detail: "B",
            report_id: None,
        };
        let err = record_violation(&store, &snapshot, "w-f2", record.generation, violation_b)
            .expect_err("不得覆盖他人 pending");
        assert!(
            matches!(
                err,
                crate::product::reviewer_window::ReviewerWindowError::ViolationJournalPending { .. }
            ),
            "{err:?}"
        );
        assert_eq!(
            store
                .load_window("w-f2")
                .unwrap()
                .violation_pending
                .as_ref()
                .unwrap()
                .kind,
            "platform_domain_write"
        );

        // 同 entry(A)重试 → journal 落盘、标记清除、迁移 ViolationRecorded。
        fs::remove_dir(&journal_path).unwrap();
        let violation_a = ViolationRecord {
            kind: "platform_domain_write",
            path: ".aria/a.json",
            before_digest: None,
            after_digest: None,
            detail: "A",
            report_id: None,
        };
        let violated = record_violation(&store, &snapshot, "w-f2", record.generation, violation_a)
            .expect("retry A");
        assert_eq!(violated.state, ReviewerWindowState::ViolationRecorded);
        assert!(violated.violation_pending.is_none());
    }

    /// F3:本窗重入态(pending 残留)下 journal 非空不构成洗白指控。
    #[test]
    fn f3_reentry_pending_journal_not_accused_as_whitewash() {
        let mut record = facts_record("w-f3");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(restored_binding());
        record.violation_pending = Some(crate::product::reviewer_window_store::ViolationPending {
            entry_id: "half-appended".to_string(),
            kind: "git_extra_ref".to_string(),
            detail: "append 崩溃残留".to_string(),
        });
        // journal 非空(部分落盘的本窗派生条目)+ pending 残留 →
        // reconcile 不产生「洗白」指控(那是 journal_tamper 虚假污染)。
        let outcome = reconcile(&ReconciliationFacts {
            journal: vec![journal_entry(1, "w-f3", "git_extra_ref")],
            record,
            manifest: ManifestFacts::Missing,
        });
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated { reasons, .. } => {
                assert!(
                    !reasons.iter().any(|reason| reason.contains("洗白")),
                    "重入态不得误判洗白: {reasons:?}"
                );
            }
            other => panic!("期望 Invalidated(pending fail-closed),得到 {other:?}"),
        }
    }

    /// F5:终态投影按实际绑定;ClosedAccepted 遇非空 journal 倒填作废。
    #[test]
    fn f5_terminal_projection_uses_binding_and_backfill_guard() {
        let manifest = minimal_manifest("w-f5");
        let mut record = facts_record("w-f5");
        record.state = ReviewerWindowState::ClosedAccepted;
        record.report_acceptance = ReportAcceptance::Accepted;
        record.restore = Some(restored_binding());
        record.baseline = Some(baseline_binding(&manifest_fingerprint(&manifest)));
        let outcome = reconcile(&ReconciliationFacts {
            record: record.clone(),
            journal: Vec::new(),
            manifest: ManifestFacts::Loaded(manifest),
        });
        assert!(outcome.git.passed, "终态投影按绑定事实,不再恒 false");
        assert!(outcome.platform.passed);
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Accepted { .. }
        ));

        // 倒填防线:终态 Accepted + 非空 journal → Invalidated fail-closed。
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: vec![journal_entry(1, "w-f5", "channel_bypass")],
            manifest: ManifestFacts::Missing,
        });
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Invalidated { .. }
        ));
    }

    // ---------- 审查修复轮 2(P1/P2/P3)钉测 ----------

    /// P1:index-only 直写(回写后不可再观)经平台漂移停等续接后
    /// 仍 ManualGate——恢复前持久化观察兜底,不静默放行。
    #[test]
    fn p1_index_only_drift_survives_platform_recovery_detour() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let platform_dir = fx.base.join("platform");
        fs::create_dir_all(&platform_dir).unwrap();
        let platform_file = platform_dir.join("report.json");
        fs::write(&platform_file, "v0\n").unwrap();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let mut request = begin_request("w-p1", "attempt-1", &fx.worktree, &targets);
        request.platform_write_paths = vec![PlatformWritePath {
            path: platform_file.clone(),
            ownership: "report-store".to_string(),
        }];
        let handle = block_on(begin_review_window(&store, &snapshot, &request)).expect("begin");

        // Reviewer 直写 .git/index(追加字节:无新对象/无 ref 痕迹——
        // 恢复回写后不可再观)+ 平台域漂移(逼 RecoveryRequired 分支)。
        // index-only 直写(N4 语义可检出形态):写对象 + 直接改 staging
        // (无 ref/HEAD 变化;OID 变 → 语义摘要漂移)。
        fs::write(fx.worktree.join("evil.txt"), "evil\n").unwrap();
        let blob = git_sha(&fx.worktree, &["hash-object", "-w", "evil.txt"]);
        git_ok(
            &fx.worktree,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("100644,{blob},evil.txt"),
            ],
        );
        fs::write(&platform_file, "v1-drift\n").unwrap();

        let quiescing = begin_quiesce(&store, "w-p1", handle.generation, "end").unwrap();
        let err = block_on(restore_window(
            &store,
            &snapshot,
            "w-p1",
            quiescing.generation,
        ))
        .expect_err("平台漂移按 T16 停等");
        assert!(
            matches!(
                &err,
                crate::product::reviewer_window::ReviewerWindowError::RecoveryStopped { reason, .. }
                    if reason.contains("platform")
            ),
            "{err:?}"
        );
        // 恢复前观察已持久化(index 事实在,虽 index 已被 T16 回写)。
        let record = store.load_window("w-p1").unwrap();
        assert!(
            record
                .pre_restore_git_facts
                .iter()
                .any(|entry| entry.path == ".git/index"),
            "pre_restore 必须含 index 事实: {:?}",
            record.pre_restore_git_facts
        );

        // 人工稳定平台 → 重入:内存报告空(index 已回写),持久化恢复
        // 前观察兜底 → ManualGate,不 Accepted。
        fs::write(&platform_file, "v0\n").unwrap();
        let summary = block_on(restore_window(&store, &snapshot, "w-p1", record.generation))
            .expect("reentry");
        assert!(summary.manual_gate, "index-only 直写不得被洗白");
        assert_eq!(summary.record.state, ReviewerWindowState::ManualGate);
        let journal = store.load_violations("w-p1").unwrap();
        assert!(
            journal.iter().any(|entry| entry.path == ".git/index"),
            "{journal:?}"
        );
        assert!(
            record_report_acceptance(
                &store,
                &snapshot,
                "w-p1",
                summary.record.generation,
                ReportAcceptance::Accepted,
            )
            .is_err()
        );
    }

    /// P1 崩溃变体:恢复入口观察+in_progress 落盘后、gate 收口前 kill
    /// → 重入以持久化恢复前观察续接,仍 ManualGate。
    #[test]
    fn p1_crash_variant_persisted_pre_restore_gates_on_reentry() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-p1c", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");

        // index-only 直写(N4 语义可检出形态,同 p1 平台绕行)。
        fs::write(fx.worktree.join("evil.txt"), "evil\n").unwrap();
        let blob = git_sha(&fx.worktree, &["hash-object", "-w", "evil.txt"]);
        git_ok(
            &fx.worktree,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("100644,{blob},evil.txt"),
            ],
        );
        let quiescing = begin_quiesce(&store, "w-p1c", handle.generation, "end").unwrap();

        // 第一次入口:观察+in_progress 已持久化;journal 阻断使收口
        // 失败(等价 kill 现场:Restoring+pre_restore 已在 durable 态)。
        let journal_path = fx.window_root.join("violations").join("w-p1c.json");
        fs::create_dir_all(&journal_path).unwrap();
        let err = block_on(restore_window(
            &store,
            &snapshot,
            "w-p1c",
            quiescing.generation,
        ))
        .expect_err("journal 阻断,收口必须失败");
        assert!(
            matches!(
                &err,
                crate::product::reviewer_window::ReviewerWindowError::Store(_)
            ),
            "{err:?}"
        );
        let record = store.load_window("w-p1c").unwrap();
        assert!(
            record
                .pre_restore_git_facts
                .iter()
                .any(|entry| entry.path == ".git/index")
        );

        // 崩溃后重入(durable 现场续接)→ 仍 ManualGate。
        fs::remove_dir(&journal_path).unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-p1c",
            record.generation,
        ))
        .expect("reentry");
        assert!(summary.manual_gate, "崩溃重入后持久化观察必须兜底");
    }

    /// P2-a:pending/journal 存在但无可复观事实 → 仍 gate,不永久卡
    /// Restoring 也不被采信。
    #[test]
    fn p2a_pending_without_reobservable_facts_still_gates() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-p2a", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");
        let quiescing = begin_quiesce(&store, "w-p2a", handle.generation, "end").unwrap();

        // 造 pending(A)+ journal 阻断(无可复观 git 事实的纯净仓库)。
        let journal_path = fx.window_root.join("violations").join("w-p2a.json");
        fs::create_dir_all(&journal_path).unwrap();
        assert!(
            record_violation(
                &store,
                &snapshot,
                "w-p2a",
                quiescing.generation,
                ViolationRecord {
                    kind: "channel_bypass",
                    path: "channels/push",
                    before_digest: None,
                    after_digest: None,
                    detail: "push 绕过",
                    report_id: None,
                },
            )
            .is_err()
        );
        fs::remove_dir(&journal_path).unwrap();
        let mut record = store.load_window("w-p2a").unwrap();
        assert!(record.violation_pending.is_some());

        // 白盒构造 P2-a 现场:恢复入口已落(Restoring+in_progress)后、
        // journal 落盘前 kill——pending 残留且无可复观事实(journal 空、
        // 仓库干净)。重入必须 gate 而非永久卡 Restoring/被采信。
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(crate::product::reviewer_window_store::RestoreBinding {
            completed: false,
            from_violation: false,
            outcome: "in_progress".to_string(),
            cursor_phase: None,
            entry_index: None,
            detail: String::new(),
            git_restored_mutation_paths: Vec::new(),
            git_extra_refs_observed: Vec::new(),
            platform_verified: Vec::new(),
        });
        record.bump_transition(ReviewerWindowState::Restoring, "白盒:恢复入口后 kill 现场");
        let expected = record.generation;
        let record = store.save_window_transition(record, expected).unwrap();

        // 重入恢复 → P2-a gate → 清 pending → ManualGate。
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-p2a",
            record.generation,
        ))
        .expect("reentry");
        assert!(summary.manual_gate, "pending 存在即 gate,不得静默放行");
        assert_eq!(summary.record.state, ReviewerWindowState::ManualGate);
        assert!(summary.record.violation_pending.is_none());
    }

    /// P2-b:嵌套路径子模块(modules/vendor/foo)capture/复观不报错。
    #[test]
    fn p2b_nested_submodule_gitdir_observed_without_error() {
        let fx = fixture();
        let nested = fx
            .worktree
            .join(".git")
            .join("modules")
            .join("vendor")
            .join("foo");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("config"), "[core]\n\tbare = false\n").unwrap();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-p2b", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("嵌套子模块 capture 不得失败");
        let quiescing = begin_quiesce(&store, "w-p2b", handle.generation, "end").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-p2b",
            quiescing.generation,
        ))
        .expect("restore");
        assert!(!summary.manual_gate, "无漂移不得误报");
    }

    /// P3-2:git_annex_observation=None 不序列化(旧 manifest JSON 形状不变)。
    #[test]
    fn p3_2_none_annex_omitted_from_json() {
        let manifest = minimal_manifest("w-p32");
        let serialized = serde_json::to_string(&manifest).expect("serialize");
        assert!(!serialized.contains("git_annex_observation"));
        let deserialized: crate::product::reviewer_snapshot::ReviewerBaselineManifest =
            serde_json::from_str(&serialized).expect("deserialize");
        assert!(deserialized.git_annex_observation.is_none());
    }

    /// P3-4:journal 已含 pending.entry_id(陈旧标记)→ 先清再处理新违规。
    #[test]
    fn p3_4_stale_pending_cleared_before_new_violation() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-p34", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");
        let quiescing = begin_quiesce(&store, "w-p34", handle.generation, "end").unwrap();

        // 造 pending(A)+ journal 阻断。
        let journal_path = fx.window_root.join("violations").join("w-p34.json");
        fs::create_dir_all(&journal_path).unwrap();
        assert!(
            record_violation(
                &store,
                &snapshot,
                "w-p34",
                quiescing.generation,
                ViolationRecord {
                    kind: "platform_domain_write",
                    path: ".aria/a.json",
                    before_digest: None,
                    after_digest: None,
                    detail: "A",
                    report_id: None,
                },
            )
            .is_err()
        );
        let record = store.load_window("w-p34").unwrap();
        let stale_entry_id = record.violation_pending.as_ref().unwrap().entry_id.clone();

        // 白盒补落 A 条目(模拟「落盘成功但清除前崩溃」),再记 B:
        // 陈旧 pending 先清,B 正常推进(不再被挡)。
        fs::remove_dir(&journal_path).unwrap();
        store
            .append_violation(
                "w-p34",
                ViolationJournalEntry {
                    seq: 0,
                    entry_id: stale_entry_id,
                    at: "2026-10-17T00:00:00Z".to_string(),
                    window_id: "w-p34".to_string(),
                    attempt_id: "attempt-1".to_string(),
                    run_id: "run-1".to_string(),
                    report_id: None,
                    kind: "platform_domain_write".to_string(),
                    path: ".aria/a.json".to_string(),
                    before_digest: None,
                    after_digest: None,
                    detail: "A".to_string(),
                    restore_cursor: "started+0".to_string(),
                },
            )
            .unwrap();
        let violated = record_violation(
            &store,
            &snapshot,
            "w-p34",
            record.generation,
            ViolationRecord {
                kind: "git_domain_write",
                path: ".git/x",
                before_digest: None,
                after_digest: None,
                detail: "B",
                report_id: None,
            },
        )
        .expect("陈旧 pending 已清,B 必须可推进");
        assert_eq!(violated.state, ReviewerWindowState::ViolationRecorded);
        assert_eq!(store.load_violations("w-p34").unwrap().len(), 2);
    }

    // ---------- 审查修复轮 3(N1-N4+顺带)钉测 ----------

    /// N2:直写 .git/config.worktree → git_annex_drift → ManualGate。
    #[test]
    fn n2_config_worktree_direct_write_detected() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-n2", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");

        // 窗口内直写 per-worktree 配置面。
        fs::write(
            fx.worktree.join(".git").join("config.worktree"),
            "[core]\n\tfoo\n",
        )
        .unwrap();
        let quiescing = begin_quiesce(&store, "w-n2", handle.generation, "end").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-n2",
            quiescing.generation,
        ))
        .expect("restore");
        assert!(summary.manual_gate, "直写 config.worktree 必须进人工门");
        let journal = store.load_violations("w-n2").unwrap();
        assert!(
            journal
                .iter()
                .any(|entry| entry.kind == "git_annex_drift"
                    && entry.path.contains("worktree_config")),
            "{journal:?}"
        );
    }

    /// N3:修复轮 1 形态 manifest(annex 无 worktree_config_sha256 字段)
    /// 可反序列化,观察跳过该面不误报。
    #[test]
    fn n3_round1_manifest_deserializes_and_skips_missing_face() {
        let mut manifest = minimal_manifest("w-n3");
        manifest.git_annex_observation =
            Some(crate::product::reviewer_snapshot::GitAnnexObservation {
                config_sha256: "a".to_string(),
                common_config_sha256: "b".to_string(),
                // 修复轮 1 形态:无此字段 → JSON 缺失 → 反序列化为空串。
                worktree_config_sha256: String::new(),
                objects_digest: "c".to_string(),
                reflogs_digest: "d".to_string(),
                submodule_gitdirs_digest: "e".to_string(),
            });
        // 手工剔除 worktree_config_sha256 键模拟 round-1 JSON。
        let mut value = serde_json::to_value(&manifest).unwrap();
        let annex = value
            .get_mut("git_annex_observation")
            .unwrap()
            .as_object_mut()
            .unwrap();
        annex.remove("worktree_config_sha256");
        let manifest: crate::product::reviewer_snapshot::ReviewerBaselineManifest =
            serde_json::from_value(value).expect("round-1 manifest 必须可反序列化");
        assert_eq!(
            manifest
                .git_annex_observation
                .as_ref()
                .unwrap()
                .worktree_config_sha256,
            ""
        );

        // 空 before → 观察跳过该面(不把旧窗口当漂移):直接断言比对守卫。
        let mut record = facts_record("w-n3");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(restored_binding());
        let facts =
            crate::product::reviewer_window::observed_git_violation_facts(&record, &manifest);
        assert!(
            !facts
                .iter()
                .any(|fact| fact.path.contains("worktree_config")),
            "空 before 必须跳过该面: {facts:?}"
        );
    }

    /// N4:常规 git status(stat-refresh)不改语义摘要——不整轮作废;
    /// git add(OID 变)仍被检出。
    #[test]
    fn n4_index_semantic_digest_ignores_stat_refresh_detects_add() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-n4a", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");

        // 场景 1(B2 修复轮 4):touch 已跟踪文件(同内容新 mtime)→
        // git status 触发真实 stat-refresh(index 字节重写)。
        let baseline_index_sha = snapshot
            .load_manifest("w-n4a")
            .unwrap()
            .git_observation
            .index_sha256
            .clone();
        let touched = fx.worktree.join("tracked.txt");
        let content = fs::read_to_string(&touched).unwrap();
        // 触发 stat 过期:同内容经临时文件 rename 回(必然更新 mtime)。
        let tmp = fx.worktree.join("tracked.txt.tmp");
        fs::write(&tmp, &content).unwrap();
        fs::rename(&tmp, &touched).unwrap();
        git_ok(&fx.worktree, &["status", "--porcelain"]);
        // 证明 stat-refresh 确实发生:raw index 字节已≠基线。
        let raw_now = sha256_of_file(&fx.worktree.join(".git").join("index"));
        assert_ne!(
            raw_now, baseline_index_sha,
            "前置失效:git status 未触发 stat-refresh(需 mtime 变化)"
        );
        let quiescing = begin_quiesce(&store, "w-n4a", handle.generation, "end").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-n4a",
            quiescing.generation,
        ))
        .expect("restore");
        assert!(
            !summary.manual_gate,
            "stat-refresh(raw 变、语义不变)不得整轮作废: {:?}",
            store.load_violations("w-n4a").unwrap()
        );
        // 回写后 raw 恢复基线字节(B1:回写仍按 raw 差异执行)。
        assert_eq!(
            sha256_of_file(&fx.worktree.join(".git").join("index")),
            baseline_index_sha
        );

        // 场景 2:git add(新对象入 index,OID 变)仍被检出。
        let fx2 = fixture();
        let store2 = fx2.store();
        let snapshot2 = fx2.snapshot();
        let scope2 = open_scope(&fx2.base, "coder");
        let targets2: [&ProviderRunResourceScope; 1] = [&scope2];
        let handle2 = block_on(begin_review_window(
            &store2,
            &snapshot2,
            &begin_request("w-n4b", "attempt-1", &fx2.worktree, &targets2),
        ))
        .expect("begin");
        fs::write(fx2.worktree.join("smuggled.txt"), "x\n").unwrap();
        git_ok(&fx2.worktree, &["add", "smuggled.txt"]);
        let quiescing2 = begin_quiesce(&store2, "w-n4b", handle2.generation, "end").unwrap();
        let summary2 = block_on(restore_window(
            &store2,
            &snapshot2,
            "w-n4b",
            quiescing2.generation,
        ))
        .expect("restore");
        assert!(summary2.manual_gate, "git add 漂移必须被检出");
        let journal = store2.load_violations("w-n4b").unwrap();
        assert!(
            journal.iter().any(|entry| entry.path == ".git/index"),
            "{journal:?}"
        );
    }

    /// 顺带 2:零事实 pending 清除时落占位条目——reconcile 不误报
    /// journal_tamper(from_violation 真而 journal 空的矛盾)。
    #[test]
    fn incidental2_pending_placeholder_entry_prevents_tamper_false_positive() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-i2", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");
        let quiescing = begin_quiesce(&store, "w-i2", handle.generation, "end").unwrap();

        // 造 pending + 白盒构造 Restoring 现场(同 p2a)。
        let journal_path = fx.window_root.join("violations").join("w-i2.json");
        fs::create_dir_all(&journal_path).unwrap();
        assert!(
            record_violation(
                &store,
                &snapshot,
                "w-i2",
                quiescing.generation,
                ViolationRecord {
                    kind: "channel_bypass",
                    path: "channels/commit",
                    before_digest: None,
                    after_digest: None,
                    detail: "绕过",
                    report_id: None,
                },
            )
            .is_err()
        );
        fs::remove_dir(&journal_path).unwrap();
        let mut record = store.load_window("w-i2").unwrap();
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(crate::product::reviewer_window_store::RestoreBinding {
            completed: false,
            from_violation: false,
            outcome: "in_progress".to_string(),
            cursor_phase: None,
            entry_index: None,
            detail: String::new(),
            git_restored_mutation_paths: Vec::new(),
            git_extra_refs_observed: Vec::new(),
            platform_verified: Vec::new(),
        });
        record.bump_transition(ReviewerWindowState::Restoring, "白盒 kill 现场");
        let expected = record.generation;
        let record = store.save_window_transition(record, expected).unwrap();

        let summary = block_on(restore_window(&store, &snapshot, "w-i2", record.generation))
            .expect("reentry");
        assert!(summary.manual_gate);
        // 占位条目已落:reconcile 无 journal_tamper 误报。
        let journal = store.load_violations("w-i2").unwrap();
        assert_eq!(journal.len(), 1, "pending 占位条目: {journal:?}");
        let outcome = reconcile_from_stores(&store, &snapshot, "w-i2").unwrap();
        match &outcome.recommendation {
            AcceptanceRecommendation::Invalidated { reasons, .. } => {
                assert!(
                    !reasons.iter().any(
                        |reason| reason.contains("journal 为空") || reason.contains("序号断裂")
                    ),
                    "不得误报篡改: {reasons:?}"
                );
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    /// P1(修复轮 5):index 损坏(直写垃圾)不可被 git 解析——恢复不
    /// 卡死:基线 blob 回写修复 + 违规事实在 + raw 归位基线。
    #[test]
    fn p1r_corrupt_index_restored_not_stuck() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-p1r", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");
        let baseline_index_sha = snapshot
            .load_manifest("w-p1r")
            .unwrap()
            .git_observation
            .index_sha256
            .clone();

        // 直写垃圾字节:index 不可被 git 解析。
        fs::write(
            fx.worktree.join(".git").join("index"),
            b"garbage-not-an-index",
        )
        .unwrap();
        let quiescing = begin_quiesce(&store, "w-p1r", handle.generation, "end").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-p1r",
            quiescing.generation,
        ))
        .expect("损坏 index 不得中止恢复(基线 blob 是修复源)");

        // index 已被基线字节修复(raw 归位)。
        assert_eq!(
            sha256_of_file(&fx.worktree.join(".git").join("index")),
            baseline_index_sha
        );
        // 违规事实在(语义不可解析保守视为漂移→恢复事实→人工门)。
        assert!(summary.manual_gate, "损坏 index 必须按漂移收口");
        let journal = store.load_violations("w-p1r").unwrap();
        assert!(
            journal.iter().any(|entry| entry.path == ".git/index"),
            "{journal:?}"
        );
    }
