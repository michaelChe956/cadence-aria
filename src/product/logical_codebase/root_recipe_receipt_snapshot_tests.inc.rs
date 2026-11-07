// 块2 片6（REQ-AUD-01/02/03/04）：审计快照的元数据差分 / `.git` 跳过 /
// 流式读 —— 新增用例片段（单独成文件以守住 1200 行硬线；由
// root_recipe_receipt_tests.inc.rs 的 `mod tests` 内 include 进来，与既有
// 用例共享同一批 fixture 与 helper）。
//
// 本文件被 `include!` 进 `mod tests`（不是独立模块），因此可直接使用
// 同模块的 `ReceiptFixture` / `GitignoreFixture` / `command_spec` 等夹具。

    // ---- 块2 片6（REQ-AUD-01/02/03/04）：元数据差分 + .git 跳过 + 流式读 --

    /// T1：`.git/HEAD` **不在**快照 entries；`.git` 字节不计入预算。
    #[test]
    fn git_subtree_stays_out_of_snapshot_entries_and_budget() {
        let fixture = GitignoreFixture::new(|root| {
            std::fs::create_dir_all(root.join("members/repo-a/.git/objects")).unwrap();
            std::fs::write(
                root.join("members/repo-a/.git/HEAD"),
                "ref: refs/heads/main\n",
            )
            .unwrap();
            std::fs::write(root.join("members/repo-a/.git/config"), "[core]\n").unwrap();
            std::fs::write(
                root.join("members/repo-a/.git/objects/blob"),
                vec![0u8; 4096],
            )
            .unwrap();
            std::fs::write(root.join("members/repo-a/README.md"), "# member\n").unwrap();
        });
        let auditor = RootRecipeFilesystemAuditor::new();
        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        let paths = snapshot_paths(&watch.before);
        assert!(
            !paths.iter().any(|path| path.contains(".git")),
            ".git subtree must stay out of the snapshot entries: {paths:?}"
        );
        assert!(
            paths.contains(&"members/repo-a/README.md"),
            "member content stays observable: {paths:?}"
        );

        // `.git` 的 4 KiB 字节不计入预算：16 字节的小预算也能完成一次审计
        // （窗口内只改了 3 字节的 allowlist 内文件）。
        let tight = GitignoreFixture::byte_capped_auditor(16);
        let watch = tight
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        std::fs::create_dir_all(fixture.root.join(".aria/aggregate")).unwrap();
        std::fs::write(fixture.root.join(".aria/aggregate/small.txt"), "ok\n").unwrap();
        let receipt = tight
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap();
        assert_eq!(receipt.verdict, RootRecipeCommandVerdict::Allowed);
    }

    /// T2：两遍之间只改一个文件 → 变更清单只含它，且**只对该文件**读内容
    /// （读取探针计数）。
    #[test]
    fn metadata_diff_reads_content_only_for_changed_paths() {
        let fixture = GitignoreFixture::new(|root| {
            std::fs::create_dir_all(root.join("members/repo-a")).unwrap();
            std::fs::write(root.join("members/repo-a/a.txt"), "a\n").unwrap();
            std::fs::write(root.join("members/repo-a/b.txt"), "b\n").unwrap();
            std::fs::write(root.join("members/repo-a/c.txt"), "c\n").unwrap();
        });
        let auditor = RootRecipeFilesystemAuditor::new();
        let (step, command) = command_spec(1);
        read_probe::reset();
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        assert!(
            read_probe::reads().is_empty(),
            "the metadata scan must not read file contents: {:?}",
            read_probe::reads()
        );
        // 只改一个文件（大小同时变化 → 不依赖 mtime 精度）。
        std::fs::write(fixture.root.join("members/repo-a/b.txt"), "bbbb\n").unwrap();
        let receipt = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap();
        let reads = read_probe::reads();
        assert_eq!(
            reads.len(),
            1,
            "only the changed file's content may be read: {reads:?}"
        );
        assert!(
            reads[0].path.ends_with("b.txt"),
            "the wrong file was read: {reads:?}"
        );
        // 变更清单只含它。
        assert_eq!(receipt.observed_changes.len(), 1, "{:?}", receipt.observed_changes);
        assert_eq!(receipt.observed_changes[0].path, "members/repo-a/b.txt");
        assert_eq!(
            receipt.observed_changes[0].change_kind,
            RootRecipeChangeKind::Modified
        );
    }

    /// T3：recipe 写到 allowlist 之外 → **仍被检出**为 `UnknownPath`
    /// （元数据差分法的检测能力回归锁）。
    #[test]
    fn out_of_allowlist_write_is_still_detected_by_metadata_diff() {
        let fixture = GitignoreFixture::new(|root| {
            std::fs::create_dir_all(root.join("members/repo-a")).unwrap();
            std::fs::write(root.join("members/repo-a/README.md"), "# member\n").unwrap();
        });
        let auditor = RootRecipeFilesystemAuditor::new();
        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        std::fs::write(fixture.root.join("members/repo-a/rogue.md"), "rogue\n").unwrap();
        let receipt = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap();
        assert_eq!(receipt.verdict, RootRecipeCommandVerdict::Rejected);
        let (class, allowed) = change_of(&receipt, "members/repo-a/rogue.md");
        assert_eq!(class, &RootRecipeChangeClass::UnknownPath);
        assert!(!allowed, "out-of-allowlist write must be rejected");
    }

    /// T4：存量 1 GiB 未忽略产物 + 窗口内零改动 → **不失败**（预算按变更量
    /// 计：bytes 与 entries 均 ≈ 0）。
    #[test]
    fn existing_bulk_artifacts_do_not_fail_when_nothing_changed() {
        let fixture = GitignoreFixture::new(|root| {
            std::fs::create_dir_all(root.join("pms/target")).unwrap();
            let fat = std::fs::File::create(root.join("pms/target/app.jar")).unwrap();
            fat.set_len(1024 * 1024 * 1024).unwrap();
            drop(fat);
        });
        // 64 KiB 字节预算 + 1 条目预算：存量 1 GiB 完全不触发。
        let auditor = RootRecipeFilesystemAuditor::with_snapshot_budget(RootRecipeSnapshotBudget {
            max_entries: 1,
            max_bytes: 64 * 1024,
        });
        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        let receipt = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap();
        assert_eq!(receipt.verdict, RootRecipeCommandVerdict::Allowed);
        assert!(
            receipt.observed_changes.is_empty(),
            "no change must mean no observed change: {:?}",
            receipt.observed_changes
        );
        // 计费口径：变更清单为空 → bytes 与 entries 都是 0。
        let metas = scan_meta(&fixture.root).unwrap();
        let changes = diff_meta(&metas, &metas);
        assert!(changes.is_empty());
        let (_, charge) = describe_changes(
            &fixture.root,
            &changes,
            &auditor.snapshot_budget(),
            &default_allowlist(),
        )
        .unwrap();
        assert_eq!(charge.bytes, 0);
        assert_eq!(charge.entries, 0);
    }

    /// T4b：**条目口径回归**——20 001 个未变化文件（fake walker 造元数据）+
    /// 窗口内零改动 → 不失败（证明 `entries` 也按变更量计；否则会命中
    /// `max_entries = 20 000`）。
    #[test]
    fn unchanged_entries_are_not_charged_against_max_entries() {
        let metas: Vec<SnapshotMeta> = (0..20_001)
            .map(|index| SnapshotMeta {
                path: PathBuf::from(format!("members/repo-a/file-{index}.bin")),
                kind: RootRecipeSnapshotEntryKind::File,
                size: 4096,
                mtime_ns: 1,
                link_target: None,
                special_type: None,
                escapes_root: false,
                in_git: false,
            })
            .collect();
        let changes = diff_meta(&metas, &metas);
        assert!(
            changes.is_empty(),
            "identical metadata must not yield any change"
        );
        let budget = RootRecipeSnapshotBudget {
            max_entries: 20_000,
            max_bytes: 64 * 1024 * 1024,
        };
        let (observed, charge) = describe_changes(
            Path::new("/nonexistent-audit-root"),
            &changes,
            &budget,
            &default_allowlist(),
        )
        .unwrap();
        assert!(observed.is_empty());
        assert_eq!(charge.entries, 0, "unchanged paths must not be charged");
        assert_eq!(charge.bytes, 0, "unchanged paths must not be charged");
    }

    /// T5：越限 → 载荷含 `max_bytes`/`cumulative_bytes`/`over_line_path`/
    /// `max_entries`/`cumulative_entries`/`top_groups`(非空)/
    /// `member_summaries`(非空)，且 `top_groups` 里**不出现**任何 `.git` 键。
    #[test]
    fn budget_failure_payload_carries_groups_and_member_summaries() {
        let fixture = GitignoreFixture::new(|root| {
            std::fs::create_dir_all(root.join("pms/.git")).unwrap();
            std::fs::write(root.join("pms/.git/HEAD"), "ref: refs/heads/main\n").unwrap();
            std::fs::create_dir_all(root.join("pms/target")).unwrap();
            std::fs::create_dir_all(root.join("busi/target")).unwrap();
        });
        let auditor = GitignoreFixture::byte_capped_auditor(1024);
        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        std::fs::write(fixture.root.join("pms/target/app.jar"), vec![0u8; 4096]).unwrap();
        std::fs::write(fixture.root.join("busi/target/app.jar"), vec![0u8; 4096]).unwrap();
        let error = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap_err();
        let failure = budget_failure_of(&error);
        assert_eq!(failure.max_bytes, 1024);
        assert!(failure.cumulative_bytes > failure.max_bytes, "{failure:?}");
        assert!(!failure.over_line_path.is_empty(), "{failure:?}");
        assert_eq!(failure.max_entries, usize::MAX);
        assert!(failure.cumulative_entries > 0, "{failure:?}");
        assert!(!failure.top_groups.is_empty(), "{failure:?}");
        assert!(!failure.member_summaries.is_empty(), "{failure:?}");
        assert!(
            failure
                .top_groups
                .iter()
                .all(|group| !group.key.contains(".git")),
            ".git must never enter the group ranking: {failure:?}"
        );
        assert!(
            failure
                .top_groups
                .iter()
                .any(|group| group.key == "pms/target"),
            "{failure:?}"
        );
        // 大项分组按字节降序。
        let bytes: Vec<u64> = failure.top_groups.iter().map(|group| group.bytes).collect();
        let mut sorted = bytes.clone();
        sorted.sort_by(|left, right| right.cmp(left));
        assert_eq!(bytes, sorted, "{failure:?}");
        assert!(
            failure
                .member_summaries
                .iter()
                .any(|member| member.member == "pms" && member.bytes > 0),
            "{failure:?}"
        );
    }

    /// T6：4 GiB 级稀疏文件 → **提前中止**、读取循环次数有上限（内存峰值在
    /// 块大小量级，绝不整读 4 GiB）。
    #[test]
    fn huge_file_read_stops_at_budget_without_reading_it_whole() {
        let fixture = GitignoreFixture::new(|root| {
            std::fs::create_dir_all(root.join("pms")).unwrap();
        });
        let auditor = GitignoreFixture::byte_capped_auditor(64 * 1024);
        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        let fat = std::fs::File::create(fixture.root.join("pms/fat.bin")).unwrap();
        fat.set_len(4 * 1024 * 1024 * 1024).unwrap();
        drop(fat);
        read_probe::reset();
        let error = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap_err();
        let failure = budget_failure_of(&error);
        assert!(failure.cumulative_bytes > failure.max_bytes, "{failure:?}");
        assert_eq!(failure.over_line_path, "pms/fat.bin");
        let reads = read_probe::reads();
        assert_eq!(reads.len(), 1, "{reads:?}");
        // 读满预算即停：64 KiB 预算 / 64 KiB 块 → 至多 3 次读取循环，
        // 且**读到的字节数**不超过「预算 + 一个块」（证明没有整读 4 GiB）。
        assert!(reads[0].chunks <= 3, "read loop must stop at the budget: {reads:?}");
        assert!(
            reads[0].bytes <= 2 * 64 * 1024,
            "the oversized file must not be read whole: {reads:?}"
        );
    }

    /// T7：旧 receipt（带 before/after 全量快照 + 内容摘要）**仍可反序列化**。
    #[test]
    fn legacy_receipt_with_content_digests_still_deserializes() {
        let legacy = r#"{
          "operation_id": "aggregate_initialization_0001",
          "canonical_root": "/tmp/aggregate-root",
          "step": "pre_check",
          "command_index": 1,
          "command": "/pre-check --no-interrupt --upgrade 用大陆镜像",
          "allowlist": ["AGENTS.md"],
          "before_snapshot": {
            "canonical_root": "/tmp/aggregate-root",
            "entries": [
              {
                "path": "AGENTS.md",
                "kind": "file",
                "content_digest": "sha256:legacy-before",
                "link_target": null,
                "special_type": null,
                "escapes_root": false
              }
            ],
            "snapshot_digest": "sha256:legacy-before-digest"
          },
          "after_snapshot": {
            "canonical_root": "/tmp/aggregate-root",
            "entries": [
              {
                "path": "AGENTS.md",
                "kind": "file",
                "content_digest": "sha256:legacy-after",
                "link_target": null,
                "special_type": null,
                "escapes_root": false
              }
            ],
            "snapshot_digest": "sha256:legacy-after-digest"
          },
          "observed_changes": [
            {
              "path": "AGENTS.md",
              "change_kind": "modified",
              "classification": "allowlisted_artifact",
              "allowed": true,
              "before_digest": "sha256:legacy-before",
              "after_digest": "sha256:legacy-after",
              "link_target": null,
              "escapes_root": false
            }
          ],
          "escape_evidence": [],
          "verdict": "allowed",
          "rejection_reason": null,
          "recorded_at": "2026-10-01T00:00:00Z"
        }"#;
        let receipt: RootRecipeCommandReceipt = serde_json::from_str(legacy).unwrap();
        assert_eq!(receipt.operation_id, OPERATION_ID);
        assert_eq!(receipt.verdict, RootRecipeCommandVerdict::Allowed);
        assert_eq!(
            receipt.before_snapshot.entries[0].content_digest.as_deref(),
            Some("sha256:legacy-before")
        );
        assert_eq!(
            receipt.observed_changes[0].before_digest.as_deref(),
            Some("sha256:legacy-before")
        );
    }

    /// T8：`classify_member_git` 对 `.git/config` 变化仍判 `MemberGit`
    /// （分类保留）；端到端的「窗口内写 `.git` 仍判越界」由
    /// `receipt_auditor_rejects_unknown_paths_and_symlink_escape` 锁住。
    #[test]
    fn member_git_classification_is_retained_for_git_paths() {
        assert_eq!(
            classify_member_git("members/repo-a/.git/config"),
            Some(RootRecipeChangeClass::MemberGit)
        );
        assert_eq!(
            classify_member_git("members/repo-a/.git/HEAD"),
            Some(RootRecipeChangeClass::MemberGit)
        );
        assert_eq!(
            classify_member_git("members/repo-a/.git/worktrees/wt/HEAD"),
            Some(RootRecipeChangeClass::MemberWorktree)
        );
        assert_eq!(classify_member_git("members/repo-a/README.md"), None);
    }

    /// T10（10.4 用户面出口）：预算超限的**诊断文本**里必须真的带上
    /// 「大项分组」与「成员仓摘要」——码表写的是「大项排行与成员仓见下方
    /// 诊断」，下方不能什么都没有。
    #[test]
    fn budget_diagnostic_carries_groups_and_member_summaries() {
        let fixture = GitignoreFixture::new(|root| {
            std::fs::create_dir_all(root.join("pms/.git")).unwrap();
            std::fs::write(root.join("pms/.git/HEAD"), "ref: refs/heads/main\n").unwrap();
            std::fs::create_dir_all(root.join("pms/target")).unwrap();
            std::fs::create_dir_all(root.join("busi/target")).unwrap();
        });
        let auditor = GitignoreFixture::byte_capped_auditor(1024);
        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        std::fs::write(fixture.root.join("pms/target/app.jar"), vec![0u8; 4096]).unwrap();
        std::fs::write(fixture.root.join("busi/target/app.jar"), vec![0u8; 4096]).unwrap();
        let error = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap_err();
        let failure = budget_failure_of(&error);
        // 诊断文本由载荷类型自己渲染（`AuditSnapshotBudgetFailure::diagnostic`，
        // 片1 的模块）：码表那句「大项排行与成员仓见下方诊断」指的就是它。
        let diagnostic = failure.diagnostic();
        // 上限、累计、越线文件都在。
        assert!(
            diagnostic.contains("audit_snapshot_budget_exceeded"),
            "{diagnostic}"
        );
        assert!(diagnostic.contains("1024"), "{diagnostic}");
        // 越线文件 = 变更清单里**第一个**被读到的文件（路径升序 ⇒ busi 在 pms 前）。
        assert!(
            diagnostic.contains("busi/target/app.jar"),
            "{diagnostic}"
        );
        // 大项分组与成员仓摘要是**真的**有内容（不是空、不是占位）。
        assert!(diagnostic.contains("top_groups:"), "{diagnostic}");
        assert!(diagnostic.contains("pms/target=4096 bytes"), "{diagnostic}");
        assert!(diagnostic.contains("member_summaries:"), "{diagnostic}");
        assert!(diagnostic.contains("pms=4096 bytes"), "{diagnostic}");
        // `.git` 永不入列（用户面也不该看到 `.git` 大项）。
        assert!(!diagnostic.contains(".git"), "{diagnostic}");
    }

    /// T9（成本口径回归）：扫描成本与**本次变更量**成正比，而不是与整仓
    /// 存量成正比——300 个文件 + 100 个 `.git` 文件的仓，窗口内只改一个
    /// 文件时，**只读那一个文件的内容**（读探针计数），其余路径只取元数据。
    #[test]
    fn scan_cost_is_proportional_to_the_change_set_not_to_the_tree() {
        const MEMBER_FILES: usize = 300;
        const GIT_FILES: usize = 100;
        const FILE_BYTES: usize = 64 * 1024;
        let fixture = GitignoreFixture::new(|root| {
            std::fs::create_dir_all(root.join("members/repo-a/.git/objects")).unwrap();
            for index in 0..GIT_FILES {
                std::fs::write(
                    root.join(format!("members/repo-a/.git/objects/o{index}")),
                    vec![0u8; 1024],
                )
                .unwrap();
            }
            std::fs::create_dir_all(root.join("members/repo-a/src")).unwrap();
            for index in 0..MEMBER_FILES {
                std::fs::write(
                    root.join(format!("members/repo-a/src/f{index}.bin")),
                    vec![0u8; FILE_BYTES],
                )
                .unwrap();
            }
        });
        let auditor = RootRecipeFilesystemAuditor::new();
        let (step, command) = command_spec(1);
        read_probe::reset();
        let started = std::time::Instant::now();
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        let before_elapsed = started.elapsed();
        std::fs::write(fixture.root.join("members/repo-a/src/f7.bin"), vec![0u8; 2048]).unwrap();
        let started = std::time::Instant::now();
        let receipt = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap();
        let after_elapsed = started.elapsed();
        let reads = read_probe::reads();
        assert_eq!(
            reads.len(),
            1,
            "only the changed file's content may be read: {reads:?}"
        );
        assert!(reads[0].path.ends_with("f7.bin"), "{reads:?}");
        assert_eq!(receipt.observed_changes.len(), 1, "{:?}", receipt.observed_changes);
        // 实测数字（报告用）：条目数、读内容次数、耗时。
        eprintln!(
            "[audit-cost] entries={} git_face_entries=0 changed_paths={} content_reads={} \
             read_bytes=2048 tree_bytes={} before_ms={} after_ms={}",
            receipt.after_snapshot.entries.len(),
            receipt.observed_changes.len(),
            reads.len(),
            (MEMBER_FILES * FILE_BYTES + GIT_FILES * 1024) as u64,
            before_elapsed.as_millis(),
            after_elapsed.as_millis(),
        );
    }
