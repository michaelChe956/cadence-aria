#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::json_store::{ProductStoreError, read_json};
    use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepKind;
    use crate::product::logical_codebase::aggregate_initialization_store::root_recipe_command_index;

    const PROJECT_ID: &str = "project_0001";
    const LC_ID: &str = "logical_codebase_0001";
    const OPERATION_ID: &str = "aggregate_initialization_0001";
    const RECORDED_AT: &str = "2026-10-01T00:00:00Z";
    const POLICY_DIGEST: &str = "sha256:policy-1";
    const RULE_DIGEST: &str = "sha256:rule-1";

    /// 非 Git 聚合根 + 一个成员仓（含 `.git/HEAD`）的最小 fixture；receipt
    /// store 以 per-LC scope 构造（与 1.4 的 store 同一布局规则）。
    struct ReceiptFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        store: RootRecipeReceiptStore,
        root: std::path::PathBuf,
    }

    impl ReceiptFixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let paths = ProductAppPaths::new(temp.path());
            let store = RootRecipeReceiptStore::for_lc(paths.clone(), LC_ID);
            let raw_root = temp.path().join("aggregate-root");
            std::fs::create_dir_all(raw_root.join(".aria/aggregate")).unwrap();
            std::fs::create_dir_all(raw_root.join("members/repo-a/.git")).unwrap();
            std::fs::write(
                raw_root.join("members/repo-a/.git/HEAD"),
                "ref: refs/heads/main\n",
            )
            .unwrap();
            std::fs::write(raw_root.join("members/repo-a/README.md"), "# member\n").unwrap();
            let root = raw_root.canonicalize().unwrap();
            Self {
                _temp: temp,
                paths,
                store,
                root,
            }
        }

        /// 以「只写 allowlist 内一个新 artifact」的方式跑一条命令的完整审计。
        fn run_allowed_command(
            &self,
            auditor: &RootRecipeFilesystemAuditor,
            command_index: usize,
            artifact_relative: &str,
            artifact_content: &str,
        ) -> RootRecipeCommandReceipt {
            let (step, command) = command_spec(command_index);
            let watch =
                auditor.before_command(OPERATION_ID, &self.root, step, command_index, command);
            let watch = watch.unwrap();
            std::fs::write(
                self.root.join(".aria/aggregate").join(artifact_relative),
                artifact_content,
            )
            .unwrap();
            auditor
                .after_command(watch, RECORDED_AT.to_string())
                .unwrap()
        }
    }

    /// 固定命令索引的第 command_index 条（1..=4）。
    fn command_spec(command_index: usize) -> (AggregateInitializationStepKind, &'static str) {
        let (step, _, command) = root_recipe_command_index()[command_index - 1];
        (step, command)
    }

    fn change_of<'a>(
        receipt: &'a RootRecipeCommandReceipt,
        path: &str,
    ) -> (&'a RootRecipeChangeClass, bool) {
        let change = receipt
            .observed_changes
            .iter()
            .find(|change| change.path == path)
            .unwrap_or_else(|| panic!("no observed change for {path}"));
        (&change.classification, change.allowed)
    }

    #[test]
    fn receipt_auditor_rejects_unknown_paths_and_symlink_escape() {
        let fixture = ReceiptFixture::new();
        let auditor = RootRecipeFilesystemAuditor::new();

        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();

        // allowlist 内新 artifact：唯一合法变更。
        std::fs::write(
            fixture.root.join(".aria/aggregate/precheck-report.json"),
            "{\"ok\":true}",
        )
        .unwrap();
        // 根级未知路径（allowlist 之外）。
        std::fs::write(fixture.root.join("AGENTS.md"), "# user file\n").unwrap();
        // 成员 `.git` 变化。
        std::fs::write(
            fixture.root.join("members/repo-a/.git/HEAD"),
            "ref: refs/heads/changed\n",
        )
        .unwrap();
        // 成员 worktree 变化。
        std::fs::create_dir_all(fixture.root.join("members/repo-a/.git/worktrees/wt")).unwrap();
        std::fs::write(
            fixture.root.join("members/repo-a/.git/worktrees/wt/HEAD"),
            "abc123\n",
        )
        .unwrap();
        // allowlist 内指向根之外的 symlink（逃逸）。
        std::os::unix::fs::symlink("../../..", fixture.root.join(".aria/aggregate/escape-link"))
            .unwrap();

        let receipt = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap();
        assert_eq!(receipt.verdict, RootRecipeCommandVerdict::Rejected);
        assert!(
            !receipt
                .rejection_reason
                .as_deref()
                .unwrap_or_default()
                .is_empty()
        );

        let (class, allowed) = change_of(&receipt, ".aria/aggregate/precheck-report.json");
        assert_eq!(class, &RootRecipeChangeClass::AllowlistedArtifact);
        assert!(allowed, "allowlisted artifact change must be allowed");

        let (class, allowed) = change_of(&receipt, "AGENTS.md");
        assert_eq!(class, &RootRecipeChangeClass::UnknownPath);
        assert!(!allowed, "unknown root-level path must be rejected");

        let (class, allowed) = change_of(&receipt, "members/repo-a/.git/HEAD");
        assert_eq!(class, &RootRecipeChangeClass::MemberGit);
        assert!(!allowed, "member .git change must be rejected");

        let (class, allowed) = change_of(&receipt, "members/repo-a/.git/worktrees/wt/HEAD");
        assert_eq!(class, &RootRecipeChangeClass::MemberWorktree);
        assert!(!allowed, "member worktree change must be rejected");

        let (class, allowed) = change_of(&receipt, ".aria/aggregate/escape-link");
        assert_eq!(class, &RootRecipeChangeClass::SymlinkEscape);
        assert!(
            !allowed,
            "symlink escaping the canonical root must be rejected"
        );
        assert!(
            receipt
                .escape_evidence
                .iter()
                .any(|path| path == ".aria/aggregate/escape-link")
        );

        // allowlist 不扩大为整个 root。
        assert_eq!(receipt.allowlist, vec![".aria/aggregate".to_string()]);
        // auditor 只读：用户/成员文件原样保留。
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("members/repo-a/README.md")).unwrap(),
            "# member\n"
        );
        assert!(fixture.root.join("AGENTS.md").exists());

        // 拒绝事实 durable 保留（允许记录，不允许静默）。
        fixture
            .store
            .append_command(PROJECT_ID, receipt.clone())
            .unwrap();
        // 任一命令被拒绝时 finalize fail-closed。
        let error = fixture
            .store
            .finalize(
                PROJECT_ID,
                OPERATION_ID,
                POLICY_DIGEST,
                RULE_DIGEST,
                RECORDED_AT.to_string(),
            )
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::InvalidRecord { .. }),
            "finalize must fail closed on a rejected command: {error:?}"
        );
    }

    #[test]
    fn receipt_records_before_after_snapshot_and_policy_rule_identity() {
        let fixture = ReceiptFixture::new();
        let auditor = RootRecipeFilesystemAuditor::new();

        let artifacts = [
            ("precheck-report.json", "{\"ok\":true}"),
            ("claude-rules.md", "# rules"),
            ("mcp.json", "{\"mcp\":true}"),
            ("openspec-examples.json", "[]"),
        ];
        for (offset, (artifact, content)) in artifacts.iter().enumerate() {
            let command_index = offset + 1;
            let (expected_step, expected_command) = command_spec(command_index);
            let receipt = fixture.run_allowed_command(&auditor, command_index, artifact, content);

            assert_eq!(receipt.verdict, RootRecipeCommandVerdict::Allowed);
            assert!(receipt.rejection_reason.is_none());
            assert_eq!(receipt.operation_id, OPERATION_ID);
            assert_eq!(receipt.canonical_root, fixture.root);
            assert_eq!(receipt.step, expected_step);
            assert_eq!(receipt.command, expected_command);
            assert_eq!(receipt.command_index, command_index);
            assert_eq!(receipt.allowlist, vec![".aria/aggregate".to_string()]);

            // before/after 快照 + digest 均被冻结。
            assert!(!receipt.before_snapshot.entries.is_empty());
            assert!(!receipt.after_snapshot.entries.is_empty());
            assert!(
                receipt
                    .before_snapshot
                    .entries
                    .iter()
                    .any(|entry| entry.path == "members/repo-a/.git/HEAD")
            );
            assert!(
                receipt
                    .before_snapshot
                    .snapshot_digest
                    .starts_with("sha256:")
            );
            assert_ne!(
                receipt.before_snapshot.snapshot_digest,
                receipt.after_snapshot.snapshot_digest
            );
            assert!(receipt.observed_changes.iter().all(|change| change.allowed));

            fixture.store.append_command(PROJECT_ID, receipt).unwrap();
        }

        let finalized = fixture
            .store
            .finalize(
                PROJECT_ID,
                OPERATION_ID,
                POLICY_DIGEST,
                RULE_DIGEST,
                RECORDED_AT.to_string(),
            )
            .unwrap();
        assert_eq!(finalized.operation_id, OPERATION_ID);
        assert_eq!(finalized.canonical_root, fixture.root);
        assert_eq!(finalized.policy_digest, POLICY_DIGEST);
        assert_eq!(finalized.rule_digest, RULE_DIGEST);
        assert_eq!(finalized.commands.len(), 4);
        let commands: Vec<&str> = finalized
            .commands
            .iter()
            .map(|summary| summary.command.as_str())
            .collect();
        assert_eq!(
            commands,
            vec![
                "/pre-check --no-interrupt --upgrade 用大陆镜像",
                "/rule-config --no-interrupt",
                "/mcp-configuration --no-interrupt",
                "/project-rules-examples --no-interrupt",
            ]
        );
        assert!(
            finalized
                .commands
                .iter()
                .all(|summary| summary.verdict == RootRecipeCommandVerdict::Allowed)
        );

        // receipt durable 写入 per-LC scope 的 aggregate 路径族。
        let receipt_path = fixture
            .paths
            .logical_codebases_root(PROJECT_ID)
            .join(LC_ID)
            .join("aggregate-recipe-receipts")
            .join(format!("{OPERATION_ID}.json"));
        assert!(receipt_path.exists());

        // get 往返 + finalize 幂等重放。
        assert_eq!(
            fixture
                .store
                .get(PROJECT_ID, OPERATION_ID)
                .unwrap()
                .unwrap(),
            finalized
        );
        let replay = fixture
            .store
            .finalize(
                PROJECT_ID,
                OPERATION_ID,
                POLICY_DIGEST,
                RULE_DIGEST,
                RECORDED_AT.to_string(),
            )
            .unwrap();
        assert_eq!(replay, finalized);
    }

    #[test]
    fn receipt_does_not_overwrite_user_conflict() {
        let fixture = ReceiptFixture::new();
        let auditor = RootRecipeFilesystemAuditor::new();

        // 命令 1 首次审计落盘。
        let first = fixture.run_allowed_command(&auditor, 1, "precheck-report.json", "v1");
        fixture
            .store
            .append_command(PROJECT_ID, first.clone())
            .unwrap();
        // 同内容重放幂等。
        fixture
            .store
            .append_command(PROJECT_ID, first.clone())
            .unwrap();

        // 同 command_index 的不同审计事实：冲突不覆盖。
        std::fs::write(
            fixture.root.join(".aria/aggregate/precheck-report.json"),
            "v2",
        )
        .unwrap();
        std::fs::write(fixture.root.join("CLAUDE-user.md"), "user content").unwrap();
        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        let conflicting = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap();
        assert_ne!(conflicting, first);
        let error = fixture
            .store
            .append_command(PROJECT_ID, conflicting)
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::Conflict { .. }),
            "conflicting append must fail closed: {error:?}"
        );
        // 磁盘上的既有记录保持首次审计事实。
        let persisted: RootRecipeCommandReceipt = read_json(
            &fixture
                .store
                .command_receipt_path(PROJECT_ID, OPERATION_ID, 1)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(persisted, first);

        // 其余三命令补齐并 finalize。
        for command_index in 2..=4 {
            let receipt = fixture.run_allowed_command(
                &auditor,
                command_index,
                &format!("artifact-{command_index}.json"),
                "ok",
            );
            fixture.store.append_command(PROJECT_ID, receipt).unwrap();
        }
        fixture
            .store
            .finalize(
                PROJECT_ID,
                OPERATION_ID,
                POLICY_DIGEST,
                RULE_DIGEST,
                RECORDED_AT.to_string(),
            )
            .unwrap();

        // 用户手工修改最终 receipt：再次 finalize 冲突且用户内容原样保留。
        let receipt_path = fixture
            .store
            .receipt_path(PROJECT_ID, OPERATION_ID)
            .unwrap();
        let user_text = {
            let value: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&receipt_path).unwrap()).unwrap();
            let mut user_value = value.clone();
            user_value["policy_digest"] = serde_json::json!("sha256:user-edited");
            let text = serde_json::to_string_pretty(&user_value).unwrap();
            std::fs::write(&receipt_path, &text).unwrap();
            text
        };
        let error = fixture
            .store
            .finalize(
                PROJECT_ID,
                OPERATION_ID,
                POLICY_DIGEST,
                RULE_DIGEST,
                RECORDED_AT.to_string(),
            )
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::Conflict { .. }),
            "finalize over a user-modified receipt must fail closed: {error:?}"
        );
        assert_eq!(std::fs::read_to_string(&receipt_path).unwrap(), user_text);
        // get 返回用户修改后的事实（不覆盖、不回滚）。
        let current = fixture
            .store
            .get(PROJECT_ID, OPERATION_ID)
            .unwrap()
            .unwrap();
        assert_eq!(current.policy_digest, "sha256:user-edited");
    }

    /// Task 1.5 carry → Task 3.4：快照全量递归规模预算门。条目/字节超限
    /// 必须 fail-closed 报错（绝不静默截断观测面），且对 before/after 两次
    /// 快照对称生效；预算内不改变审计语义。
    #[test]
    fn receipt_snapshot_budget_gate_fails_closed_beyond_entry_and_byte_caps() {
        let fixture = ReceiptFixture::new();
        let (step, command) = command_spec(1);

        // 条目上限：预算 4 条 < fixture 实际条目数（root/.aria/members 与
        // 成员仓文件等）→ before_command fail-closed。
        let tight_entries =
            RootRecipeFilesystemAuditor::with_snapshot_budget(RootRecipeSnapshotBudget {
                max_entries: 4,
                max_bytes: u64::MAX,
            });
        let error = tight_entries
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap_err();
        let message = format!("{error:?}");
        assert!(message.contains("snapshot budget"), "{message}");
        assert!(message.contains("max_entries"), "{message}");

        // 字节上限：预算 8 字节 < 成员 .git/HEAD（17 字节）→ fail-closed。
        let tight_bytes =
            RootRecipeFilesystemAuditor::with_snapshot_budget(RootRecipeSnapshotBudget {
                max_entries: usize::MAX,
                max_bytes: 8,
            });
        let error = tight_bytes
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap_err();
        let message = format!("{error:?}");
        assert!(message.contains("snapshot budget"), "{message}");
        assert!(message.contains("max_bytes"), "{message}");

        // 预算内：同一 fixture 上默认 auditor 正常产出 Allowed receipt，
        // 预算门不改变观测语义；默认预算非零可经 snapshot_budget() 复核。
        let auditor = RootRecipeFilesystemAuditor::new();
        let budget = auditor.snapshot_budget();
        assert!(budget.max_entries > 0, "default budget must pin entries");
        assert!(budget.max_bytes > 0, "default budget must pin bytes");
        let receipt = fixture.run_allowed_command(&auditor, 2, "budget-ok.txt", "ok\n");
        assert_eq!(receipt.verdict, RootRecipeCommandVerdict::Allowed);

        // 命令前预算内、命令后超出（写入超预算大文件）→ after_command 同样
        // fail-closed——预算门对前后两次快照对称生效。
        let growing = RootRecipeFilesystemAuditor::with_snapshot_budget(RootRecipeSnapshotBudget {
            max_entries: usize::MAX,
            max_bytes: 64,
        });
        let watch = growing
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        std::fs::write(
            fixture.root.join(".aria/aggregate/oversize.txt"),
            vec![b'x'; 128],
        )
        .unwrap();
        let error = growing
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap_err();
        let message = format!("{error:?}");
        assert!(message.contains("snapshot budget"), "{message}");
        assert!(message.contains("max_bytes"), "{message}");
    }
}
