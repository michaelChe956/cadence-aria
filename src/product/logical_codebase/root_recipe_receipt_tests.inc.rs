#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::audit_snapshot_budget::AuditSnapshotBudgetFailure;
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

    /// 从 `ProductStoreError` 里取出结构化的审计预算失败载荷（块2 片1 的
    /// `AuditSnapshotBudgetExceeded` 变体）。
    fn budget_failure_of(error: &ProductStoreError) -> AuditSnapshotBudgetFailure {
        match error {
            ProductStoreError::AuditSnapshotBudgetExceeded(failure) => failure.clone(),
            other => panic!("expected an audit snapshot budget failure, got {other:?}"),
        }
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
        std::fs::write(fixture.root.join("rogue-root-file.md"), "# user file\n").unwrap();
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

        let (class, allowed) = change_of(&receipt, "rogue-root-file.md");
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
        assert_eq!(
            receipt.allowlist,
            ROOT_RECIPE_ALLOWLIST.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
        // auditor 只读：用户/成员文件原样保留。
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("members/repo-a/README.md")).unwrap(),
            "# member\n"
        );
        assert!(fixture.root.join("rogue-root-file.md").exists());

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
            assert_eq!(
                receipt.allowlist,
                ROOT_RECIPE_ALLOWLIST.iter().map(|s| s.to_string()).collect::<Vec<_>>()
            );

            // before/after 快照 + digest 均被冻结。
            assert!(!receipt.before_snapshot.entries.is_empty());
            assert!(!receipt.after_snapshot.entries.is_empty());
            // `.git` 子树不进快照观测面（REQ-AUD-02，2026-11-03 块2）：成员仓
            // 的 `.git/HEAD` 既不在条目里，其字节也不计入预算。
            assert!(
                !receipt
                    .before_snapshot
                    .entries
                    .iter()
                    .any(|entry| entry.path == "members/repo-a/.git"
                        || entry.path.starts_with("members/repo-a/.git/")),
                "member .git subtree must stay out of the snapshot: {:?}",
                receipt
                    .before_snapshot
                    .entries
                    .iter()
                    .map(|entry| entry.path.as_str())
                    .collect::<Vec<&str>>()
            );
            assert!(
                receipt
                    .before_snapshot
                    .entries
                    .iter()
                    .any(|entry| entry.path == "members/repo-a/README.md"),
                "member content stays observable"
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

    /// Task 3.5 carry ②（Task 1.8 §五.2，design Open Question「provider-native
    /// 根文件 allowlist」）：真实 Claude Code 四命令 recipe 在空的非 Git
    /// canonical root 的实测落盘前缀（2026-10-02 spike，claude 2.1.283，
    /// 见 cadence/reports/2026-10-02_验收报告_LC根初始化全链E2E）。
    /// 逐项冻结、绝不扩大为整个 root；成员目录不进 allowlist（成员 `.git`
    /// 分类优先于 allowlist 的既有锁继续生效）。2026-10-02 E2E 增补：
    /// `.codegraph`/`codegraph.json` 为产品自管聚合索引面（exclude.rs 根
    /// 扫描白名单同款），spike 冻结时无 codegraph daemon 在场属信息不全，
    /// 按真实部署事实（索引建立后 daemon 与 recipe 共存）补齐。
    #[test]
    fn root_recipe_allowlist_freezes_real_provider_root_artifacts() {
        assert_eq!(
            ROOT_RECIPE_ALLOWLIST,
            &[
                ".aria/aggregate",
                ".codegraph",
                "codegraph.json",
                "AGENTS.md",
                "CLAUDE.md",
                ".mcp.json",
                ".mcp.json.cadence-backup-*",
                ".gitignore",
                ".claude",
                ".agents",
                ".omp",
                ".codex",
                ".kimi-code",
                ".pi",
                "cadence",
                "openspec",
            ]
        );
    }

    /// 行为锁：冻结前缀内的真实根产物写入判 `AllowlistedArtifact`；
    /// 前缀外的未知路径仍 `UnknownPath` 拒绝；成员 `.git` 分类优先于
    /// allowlist（前缀内出现 `.git` 段仍拒绝）。
    #[test]
    fn receipt_auditor_allows_real_root_artifacts_and_keeps_boundaries() {
        let fixture = ReceiptFixture::new();
        let auditor = RootRecipeFilesystemAuditor::new();

        let (step, command) = command_spec(2);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 2, command)
            .unwrap();

        // 真实 recipe 的代表性根产物（每前缀至少一条实测形态）。
        std::fs::write(fixture.root.join("AGENTS.md"), "# entry\n").unwrap();
        std::fs::write(fixture.root.join("CLAUDE.md"), "# entry\n").unwrap();
        std::fs::write(fixture.root.join(".mcp.json"), "{}").unwrap();
        std::fs::write(fixture.root.join(".gitignore"), ".worktrees/\n").unwrap();
        std::fs::create_dir_all(fixture.root.join(".claude/rules")).unwrap();
        std::fs::write(fixture.root.join(".claude/rules/language.md"), "# lang\n").unwrap();
        std::fs::create_dir_all(fixture.root.join(".agents/rules")).unwrap();
        std::os::unix::fs::symlink(
            "../../.claude/rules/language.md",
            fixture.root.join(".agents/rules/language.md"),
        )
        .unwrap();
        std::fs::create_dir_all(fixture.root.join(".omp")).unwrap();
        std::fs::write(fixture.root.join(".omp/AGENTS.md"), "# bridge\n").unwrap();
        std::fs::create_dir_all(fixture.root.join(".codex")).unwrap();
        std::fs::write(fixture.root.join(".codex/config.toml"), "[mcp_servers.x]\n").unwrap();
        std::fs::create_dir_all(fixture.root.join(".kimi-code/skills/a")).unwrap();
        std::fs::write(fixture.root.join(".kimi-code/skills/a/SKILL.md"), "# a\n").unwrap();
        std::fs::create_dir_all(fixture.root.join(".pi/prompts")).unwrap();
        std::fs::write(fixture.root.join(".pi/prompts/opsx-apply.md"), "# p\n").unwrap();
        std::fs::create_dir_all(fixture.root.join("cadence/project-rules")).unwrap();
        std::fs::write(fixture.root.join("cadence/project-rules/README.md"), "# r\n").unwrap();
        std::fs::create_dir_all(fixture.root.join("openspec")).unwrap();
        std::fs::write(fixture.root.join("openspec/config.yaml"), "scope: root\n").unwrap();
        std::fs::create_dir_all(fixture.root.join(".aria/aggregate")).unwrap();
        std::fs::write(fixture.root.join(".aria/aggregate/mcp.json"), "{}").unwrap();
        // 未知根级路径（实测 recipe 从不产出）：拒绝。
        std::fs::write(fixture.root.join("rogue-root-file.md"), "# rogue\n").unwrap();
        // 成员目录（非 allowlist 前缀）：拒绝。
        std::fs::write(fixture.root.join("members/repo-a/rogue.txt"), "x").unwrap();

        let receipt = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap();

        for path in [
            "AGENTS.md",
            "CLAUDE.md",
            ".mcp.json",
            ".gitignore",
            ".claude/rules/language.md",
            ".agents/rules/language.md",
            ".omp/AGENTS.md",
            ".codex/config.toml",
            ".kimi-code/skills/a/SKILL.md",
            ".pi/prompts/opsx-apply.md",
            "cadence/project-rules/README.md",
            "openspec/config.yaml",
            ".aria/aggregate/mcp.json",
            ".claude",
            ".agents",
            ".omp",
            ".codex",
            ".kimi-code",
            ".pi",
            "cadence",
            "openspec",
        ] {
            let (class, allowed) = change_of(&receipt, path);
            assert_eq!(
                class,
                &RootRecipeChangeClass::AllowlistedArtifact,
                "{path} must be an allowlisted real root artifact"
            );
            assert!(allowed, "{path} must be allowed");
        }

        let (class, allowed) = change_of(&receipt, "rogue-root-file.md");
        assert_eq!(class, &RootRecipeChangeClass::UnknownPath);
        assert!(!allowed, "unknown root-level path must stay rejected");
        let (class, allowed) = change_of(&receipt, "members/repo-a/rogue.txt");
        assert_eq!(class, &RootRecipeChangeClass::UnknownPath);
        assert!(!allowed, "member paths must stay outside the allowlist");

        assert_eq!(
            receipt.verdict,
            RootRecipeCommandVerdict::Rejected,
            "any unknown-path write keeps the command rejected"
        );
    }

    /// r49(claude r48 现场,8 组 receipts 诊断):Claude CLI 修改 `.mcp.json`
    /// 时自动落 `.mcp.json.cadence-backup-<时间戳>` 备份——CLI 工具自身
    /// 管理性文件(与 `.codegraph` daemon 共存同类),不在精确 allowlist →
    /// 02/03 步 unknown_path 拒;同 workspace 重试时上次 `.mcp.json` 残留
    /// 必触发备份,重试循环 7200s 超时。修:allowlist 增 `-` 尾缀通配条目
    /// `.mcp.json.cadence-backup-*`,只匹配根级单文件(`*` 不跨 `/`)。
    #[test]
    fn receipt_auditor_allows_claude_cli_mcp_backup_artifacts() {
        let fixture = ReceiptFixture::new();
        let auditor = RootRecipeFilesystemAuditor::new();

        let (step, command) = command_spec(3);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 3, command)
            .unwrap();

        // CLI 真实备份形态(r48 receipts 实测):.mcp.json 修改 + 同批备份。
        std::fs::write(fixture.root.join(".mcp.json"), "{\"mcpServers\":{}}\n").unwrap();
        std::fs::write(
            fixture.root.join(".mcp.json.cadence-backup-20261007T081530-012345"),
            "{\"mcpServers\":{}}\n",
        )
        .unwrap();
        // 通配不得跨 `/`:目录化伪装仍拒。
        std::fs::create_dir_all(fixture.root.join(".mcp.json.cadence-backup-evil")).unwrap();
        std::fs::write(
            fixture.root.join(".mcp.json.cadence-backup-evil/payload.md"),
            "# rogue\n",
        )
        .unwrap();
        // 前缀不完整(`-` 后缀未对齐)的伪装仍拒。
        std::fs::write(fixture.root.join(".mcp.json.cadence-backupX"), "x").unwrap();

        let receipt = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap();

        let (class, allowed) = change_of(&receipt, ".mcp.json");
        assert_eq!(class, &RootRecipeChangeClass::AllowlistedArtifact);
        assert!(allowed, ".mcp.json itself stays allowlisted");
        let (class, allowed) = change_of(&receipt, ".mcp.json.cadence-backup-20261007T081530-012345");
        assert_eq!(
            class, &RootRecipeChangeClass::AllowlistedArtifact,
            "CLI 自身管理性备份文件必须按通配 allowlist 放行(r48 现场根因)"
        );
        assert!(allowed, "backup artifact must be allowed");
        let (class, allowed) = change_of(&receipt, ".mcp.json.cadence-backup-evil");
        assert_eq!(class, &RootRecipeChangeClass::AllowlistedArtifact);
        assert!(allowed, "pattern-matching directory component itself matches the single-level glob");
        let (class, allowed) = change_of(&receipt, ".mcp.json.cadence-backup-evil/payload.md");
        assert_eq!(
            class, &RootRecipeChangeClass::UnknownPath,
            "glob must not cross '/' into subdirectories"
        );
        assert!(!allowed);
        let (class, allowed) = change_of(&receipt, ".mcp.json.cadence-backupX");
        assert_eq!(
            class, &RootRecipeChangeClass::UnknownPath,
            "incomplete prefix must not match"
        );
        assert!(!allowed);

        assert_eq!(
            receipt.verdict,
            RootRecipeCommandVerdict::Rejected,
            "subdirectory payload keeps the command rejected"
        );
    }

    /// 缺陷回归（2026-10-02 全链 E2E，op aggregate_initialization_873c…同因）：
    /// 产品自身 sanction 的聚合索引面（`.codegraph`/`codegraph.json`，见
    /// aggregate_index/exclude.rs 根扫描白名单）在 recipe 期间承载 codegraph
    /// daemon 的 IPC 产物（Unix socket 等）。审计必须可观测地记录非常规
    /// 条目（special：路径+类型、无内容 digest），不得 fail-closed 断审；
    /// `.codegraph`/`codegraph.json` 变更按产品自管聚合面放行；未知路径
    /// 的 special 条目仍拒绝（可观测≠放行）。
    #[test]
    fn receipt_auditor_observes_special_entries_and_allows_codegraph_surface() {
        let fixture = ReceiptFixture::new();
        let auditor = RootRecipeFilesystemAuditor::new();

        // daemon 在场：before 快照前已存在的稳定 IPC 面目。
        std::fs::create_dir_all(fixture.root.join(".codegraph")).unwrap();
        let _daemon = std::os::unix::net::UnixListener::bind(
            fixture.root.join(".codegraph/listener.sock"),
        )
        .unwrap();

        let (step, command) = command_spec(4);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 4, command)
            .unwrap();

        // 命令窗内的真实 daemon 副作用形态：IPC socket 新增 + pid/log 文件、
        // codegraph.json（exclude.rs 根扫描白名单的根级配置）。
        let _sock2 = std::os::unix::net::UnixListener::bind(
            fixture.root.join(".codegraph/daemon.sock"),
        )
        .unwrap();
        std::fs::write(fixture.root.join(".codegraph/daemon.pid"), "42\n").unwrap();
        std::fs::write(fixture.root.join(".codegraph/daemon.log"), "listening\n").unwrap();
        std::fs::write(fixture.root.join("codegraph.json"), "{\"exclude\":[]}\n").unwrap();
        // 越界对照：未知根级 socket 必须仍被拒绝。
        let _rogue = std::os::unix::net::UnixListener::bind(fixture.root.join("rogue.sock"))
            .unwrap();

        let receipt = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap();

        // special 条目可观测：路径+类型入快照，无内容 digest。
        let sock = receipt
            .after_snapshot
            .entries
            .iter()
            .find(|entry| entry.path == ".codegraph/daemon.sock")
            .expect("daemon.sock must be observed in the after snapshot");
        assert_eq!(sock.kind, RootRecipeSnapshotEntryKind::Special);
        assert_eq!(sock.special_type.as_deref(), Some("socket"));
        assert!(sock.content_digest.is_none());
        // before 面里预先存在的 listener.sock 同为稳定 special 条目。
        assert!(receipt
            .before_snapshot
            .entries
            .iter()
            .any(|entry| entry.path == ".codegraph/listener.sock"
                && entry.kind == RootRecipeSnapshotEntryKind::Special));

        // 产品自管聚合面变更：逐项 AllowlistedArtifact。
        for path in [
            ".codegraph/daemon.sock",
            ".codegraph/daemon.pid",
            ".codegraph/daemon.log",
            "codegraph.json",
        ] {
            let (class, allowed) = change_of(&receipt, path);
            assert_eq!(
                class,
                &RootRecipeChangeClass::AllowlistedArtifact,
                "{path} must be an allowlisted product-managed aggregate artifact"
            );
            assert!(allowed, "{path} must be allowed");
        }

        // 未知路径 special 条目：拒绝边界不变。
        let (class, allowed) = change_of(&receipt, "rogue.sock");
        assert_eq!(class, &RootRecipeChangeClass::UnknownPath);
        assert!(!allowed, "unknown-path special entry must stay rejected");
        assert_eq!(
            receipt.verdict,
            RootRecipeCommandVerdict::Rejected,
            "rogue special entry keeps the command rejected"
        );
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

    /// Task 1.5 carry → Task 3.4 → 块2 片6（REQ-AUD-01）：快照规模预算门。
    /// **口径改为「本次变更量」**：两遍扫描只记元数据，预算只对**进入变更
    /// 清单的路径**计（bytes 与 entries 都是）。因此：
    /// - 存量体量（哪怕远超上限）在窗口内零改动时**不失败**；
    /// - 窗口内写入超限内容才 fail-closed，且失败面是结构化载荷
    ///   （上限 / 累计字节 / 越线路径 / 大项分组 / 成员仓贡献）。
    #[test]
    fn receipt_snapshot_budget_gate_charges_only_this_windows_change_volume() {
        let fixture = ReceiptFixture::new();
        let (step, command) = command_spec(1);

        // 条目上限：预算 1 条 < 窗口内实际变更条目数 → after_command
        // fail-closed（结构化载荷）。
        let tight_entries =
            RootRecipeFilesystemAuditor::with_snapshot_budget(RootRecipeSnapshotBudget {
                max_entries: 1,
                max_bytes: u64::MAX,
            });
        let watch = tight_entries
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        // 存量体量不触发：before_command 在零改动下从不失败。
        std::fs::write(fixture.root.join(".aria/aggregate/a.txt"), "a\n").unwrap();
        std::fs::write(fixture.root.join(".aria/aggregate/b.txt"), "b\n").unwrap();
        let error = tight_entries
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap_err();
        let failure = budget_failure_of(&error);
        assert_eq!(failure.max_entries, 1);
        assert!(failure.cumulative_entries > failure.max_entries);
        assert!(failure.max_bytes == u64::MAX);

        // 字节上限：窗口内写入 128 字节、预算 64 字节 → fail-closed。
        let tight_bytes =
            RootRecipeFilesystemAuditor::with_snapshot_budget(RootRecipeSnapshotBudget {
                max_entries: usize::MAX,
                max_bytes: 64,
            });
        let watch = tight_bytes
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        std::fs::write(
            fixture.root.join(".aria/aggregate/oversize.txt"),
            vec![b'x'; 128],
        )
        .unwrap();
        let error = tight_bytes
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap_err();
        let failure = budget_failure_of(&error);
        assert_eq!(failure.max_bytes, 64);
        assert!(failure.cumulative_bytes > failure.max_bytes);
        assert_eq!(failure.over_line_path, ".aria/aggregate/oversize.txt");

        // 预算内：默认 auditor 正常产出 Allowed receipt，预算门不改变观测
        // 语义；默认预算非零可经 snapshot_budget() 复核（数值不变：
        // 20 000 条目 / 64 MiB）。
        let auditor = RootRecipeFilesystemAuditor::new();
        let budget = auditor.snapshot_budget();
        assert_eq!(budget.max_entries, 20_000);
        assert_eq!(budget.max_bytes, 64 * 1024 * 1024);
        let receipt = fixture.run_allowed_command(&auditor, 2, "budget-ok.txt", "ok\n");
        assert_eq!(receipt.verdict, RootRecipeCommandVerdict::Allowed);
    }

    // ---- Bug2(2026-10-16):审计快照 gitignore 感知 -------------------------
    // 报告:cadence/notes/2026-10-16_bug-filesystem-audit-不感知gitignore.md
    // —— node_modules/target 等被 .gitignore/.git/info/exclude 忽略的路径
    // 不计入快照预算、不进快照;未被忽略的超限路径仍 fail-closed 且报错
    // 带可操作提示;无 .gitignore 仓观测语义不变。

    /// 非 Git 聚合根 fixture,由用例自行布放 .gitignore/`.git/info/exclude`
    /// 与大文件(复刻 owner 现场的 root/成员仓布局)。
    struct GitignoreFixture {
        _temp: tempfile::TempDir,
        root: std::path::PathBuf,
    }

    impl GitignoreFixture {
        fn new(setup: impl FnOnce(&Path)) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let raw = temp.path().join("aggregate-root");
            std::fs::create_dir_all(&raw).unwrap();
            setup(&raw);
            let root = raw.canonicalize().unwrap();
            Self { _temp: temp, root }
        }

        fn byte_capped_auditor(max_bytes: u64) -> RootRecipeFilesystemAuditor {
            RootRecipeFilesystemAuditor::with_snapshot_budget(RootRecipeSnapshotBudget {
                max_entries: usize::MAX,
                max_bytes,
            })
        }
    }

    fn snapshot_paths(snapshot: &RootRecipeFilesystemSnapshot) -> Vec<&str> {
        snapshot
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect()
    }

    #[test]
    fn snapshot_excludes_gitignore_ignored_paths_from_budget_and_entries() {
        let fixture = GitignoreFixture::new(|root| {
            std::fs::write(
                root.join(".gitignore"),
                "node_modules/\ndist\n*.log\ntarget/\n",
            )
            .unwrap();
            std::fs::write(root.join("README.md"), "src\n").unwrap();
            std::fs::create_dir_all(root.join("node_modules/dep")).unwrap();
            std::fs::write(root.join("node_modules/dep/big.bin"), vec![0u8; 4096]).unwrap();
            std::fs::create_dir_all(root.join("dist")).unwrap();
            std::fs::write(root.join("dist/app.js"), vec![0u8; 4096]).unwrap();
            std::fs::write(root.join("debug.log"), vec![0u8; 4096]).unwrap();
            std::fs::create_dir_all(root.join("busi/target")).unwrap();
            std::fs::write(root.join("busi/target/app.jar"), vec![0u8; 4096]).unwrap();
        });
        // 字节预算 4096:被忽略面(≥16KiB)若计入必然超限——预算门只对
        // 未被忽略的观测面生效。
        let auditor = GitignoreFixture::byte_capped_auditor(4096);
        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        let paths = snapshot_paths(&watch.before);
        for banned in [
            "node_modules",
            "node_modules/dep",
            "node_modules/dep/big.bin",
            "dist",
            "dist/app.js",
            "debug.log",
            "busi/target",
            "busi/target/app.jar",
        ] {
            assert!(
                !paths.contains(&banned),
                "ignored path {banned} must stay out of the snapshot: {paths:?}"
            );
        }
        for observable in [".gitignore", "README.md", "busi"] {
            assert!(
                paths.contains(&observable),
                "unignored path {observable} must stay observable: {paths:?}"
            );
        }
    }

    #[test]
    fn snapshot_honors_nested_gitignore_and_member_info_exclude() {
        let fixture = GitignoreFixture::new(|root| {
            // 成员仓 frontend:嵌套 .gitignore(审计根自身无 .gitignore)。
            std::fs::create_dir_all(root.join("frontend/.git")).unwrap();
            std::fs::write(root.join("frontend/.git/HEAD"), "ref: refs/heads/main\n").unwrap();
            std::fs::write(root.join("frontend/.gitignore"), "node_modules/\n").unwrap();
            std::fs::create_dir_all(root.join("frontend/node_modules/element-plus")).unwrap();
            std::fs::write(
                root.join("frontend/node_modules/element-plus/big.js"),
                vec![0u8; 8192],
            )
            .unwrap();
            std::fs::create_dir_all(root.join("frontend/src")).unwrap();
            std::fs::write(root.join("frontend/src/main.js"), "1\n").unwrap();
            // 成员仓 busi:仅 .git/info/exclude 忽略 target/。
            std::fs::create_dir_all(root.join("busi/.git/info")).unwrap();
            std::fs::write(root.join("busi/.git/info/exclude"), "target/\n").unwrap();
            std::fs::create_dir_all(root.join("busi/target")).unwrap();
            std::fs::write(root.join("busi/target/app.jar"), vec![0u8; 8192]).unwrap();
            std::fs::write(root.join("busi/pom.xml"), "<p/>\n").unwrap();
        });
        let auditor = GitignoreFixture::byte_capped_auditor(4096);
        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        let paths = snapshot_paths(&watch.before);
        assert!(
            !paths.iter().any(|path| path.contains("node_modules")),
            "nested .gitignore must keep node_modules out: {paths:?}"
        );
        assert!(
            !paths.iter().any(|path| path.starts_with("busi/target")),
            "member .git/info/exclude must keep target/ out: {paths:?}"
        );
        // gitignore/exclude 自身与成员源码保持可观测(仅忽略其规则覆盖面)。
        for observable in [
            "frontend/.gitignore",
            "busi/pom.xml",
            "frontend/src/main.js",
        ] {
            assert!(
                paths.contains(&observable),
                "{observable} must stay observable: {paths:?}"
            );
        }
        // `.git` 子树不进快照观测面(REQ-AUD-02,2026-11-03 块2):成员仓的
        // `.git/HEAD` 与 `.git/info/exclude` 都不在条目里、不计预算。规则
        // 本身仍然生效(`.git/info/exclude` 照常被读取,故 `busi/target` 出局)。
        for hidden in ["frontend/.git/HEAD", "busi/.git/info/exclude"] {
            assert!(
                !paths.contains(&hidden),
                "{hidden} must stay out of the snapshot: {paths:?}"
            );
        }
    }

    /// 预算超限的失败面（REQ-AUD-04，块2 片6）：结构化载荷带上限、累计
    /// 字节、越线路径、大项分组与成员仓贡献；且 `.git` 永不入列。
    /// 口径改为「本次变更量」后，超限只可能由**窗口内**的变更触发。
    #[test]
    fn snapshot_budget_error_on_unignored_oversize_carries_actionable_hint() {
        let fixture = GitignoreFixture::new(|root| {
            std::fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
            std::fs::create_dir_all(root.join("node_modules")).unwrap();
            std::fs::write(root.join("node_modules/skipped.bin"), vec![0u8; 1024]).unwrap();
            std::fs::write(root.join("payload.bin"), vec![0u8; 4096]).unwrap();
        });
        let auditor = GitignoreFixture::byte_capped_auditor(2048);
        let (step, command) = command_spec(1);
        // 存量 4 KiB 未忽略文件**不触发**预算（窗口内零改动 → 零计费）。
        auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        // 窗口内写入超限内容 → 结构化预算失败。
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        std::fs::create_dir_all(fixture.root.join("pms/target")).unwrap();
        std::fs::write(fixture.root.join("pms/target/app.jar"), vec![0u8; 4096]).unwrap();
        let error = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap_err();
        let failure = budget_failure_of(&error);
        assert_eq!(failure.max_bytes, 2048);
        assert!(failure.cumulative_bytes > failure.max_bytes, "{failure:?}");
        assert_eq!(failure.over_line_path, "pms/target/app.jar");
        // 大项分组：键为 `<成员仓目录名>/<一级目录>`，且 `.git` 永不入列。
        assert!(
            failure
                .top_groups
                .iter()
                .any(|group| group.key == "pms/target" && group.bytes == 4096),
            "{failure:?}"
        );
        assert!(
            !failure
                .top_groups
                .iter()
                .any(|group| group.key.contains(".git")),
            ".git must never enter the group ranking: {failure:?}"
        );
        assert!(
            failure
                .member_summaries
                .iter()
                .any(|member| member.member == "pms" && member.bytes == 4096),
            "{failure:?}"
        );
    }

    #[test]
    fn gitignore_dir_only_basename_and_negation_semantics() {
        let fixture = GitignoreFixture::new(|root| {
            std::fs::create_dir_all(root.join("a")).unwrap();
            std::fs::write(root.join("a/skip.log"), "x\n").unwrap();
            std::fs::write(root.join("a/keep.log"), "x\n").unwrap();
            std::fs::write(root.join("target"), "file named target\n").unwrap();
            std::fs::create_dir_all(root.join("deep/target")).unwrap();
            std::fs::write(root.join("deep/target/x.txt"), "x\n").unwrap();
            std::fs::write(root.join("dist-root"), "anchored\n").unwrap();
            std::fs::create_dir_all(root.join("sub/dist-root")).unwrap();
            std::fs::write(root.join("sub/dist-root/y.txt"), "y\n").unwrap();
            std::fs::write(
                root.join(".gitignore"),
                "*.log\n!keep.log\ntarget/\n/dist-root\n",
            )
            .unwrap();
        });
        let auditor = GitignoreFixture::byte_capped_auditor(u64::MAX);
        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        let paths = snapshot_paths(&watch.before);
        assert!(
            !paths.contains(&"a/skip.log"),
            "*.log must ignore by basename: {paths:?}"
        );
        assert!(
            paths.contains(&"a/keep.log"),
            "!keep.log must re-include: {paths:?}"
        );
        assert!(
            paths.contains(&"target"),
            "dir-only pattern must not eat a file named target: {paths:?}"
        );
        assert!(
            !paths.iter().any(|path| path.starts_with("deep/target")),
            "basename dir pattern applies at any depth: {paths:?}"
        );
        assert!(
            !paths.contains(&"dist-root"),
            "leading-slash pattern anchors at the scope root: {paths:?}"
        );
        assert!(
            paths.contains(&"sub/dist-root") && paths.contains(&"sub/dist-root/y.txt"),
            "anchored pattern must not leak into subdirectories: {paths:?}"
        );
    }

    #[test]
    fn ignored_path_changes_stay_out_of_audit_diff() {
        let fixture = GitignoreFixture::new(|root| {
            std::fs::create_dir_all(root.join(".aria/aggregate")).unwrap();
            std::fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
            std::fs::create_dir_all(root.join("node_modules/dep")).unwrap();
            std::fs::write(root.join("node_modules/dep/a.js"), "1\n").unwrap();
        });
        let auditor = RootRecipeFilesystemAuditor::new();
        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        // 命令窗内:被忽略面新增 + allowlist 内合法 artifact。
        std::fs::write(fixture.root.join("node_modules/dep/b.js"), "2\n").unwrap();
        std::fs::write(fixture.root.join(".aria/aggregate/change.txt"), "ok\n").unwrap();
        let receipt = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap();
        assert_eq!(receipt.verdict, RootRecipeCommandVerdict::Allowed);
        assert!(
            !receipt
                .observed_changes
                .iter()
                .any(|change| change.path.contains("node_modules")),
            "changes inside ignored paths stay out of the diff: {:?}",
            receipt.observed_changes
        );
        assert_eq!(receipt.observed_changes.len(), 1);
        assert_eq!(receipt.observed_changes[0].path, ".aria/aggregate/change.txt");
    }

    /// 无 `.gitignore` 时观测面不缩水的**变更**仍全额计费：窗口内写入的
    /// 未忽略大文件照样撞线（REQ-AUD-01 口径：预算只对本次变更量计，但
    /// 「变更」本身不被忽略规则打折）。
    #[test]
    fn snapshot_without_gitignore_keeps_counting_everything() {
        let fixture = GitignoreFixture::new(|root| {
            std::fs::write(root.join("payload.bin"), vec![0u8; 4096]).unwrap();
        });
        let auditor = GitignoreFixture::byte_capped_auditor(1024);
        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        std::fs::write(fixture.root.join("payload.bin"), vec![0u8; 8192]).unwrap();
        let error = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap_err();
        let failure = budget_failure_of(&error);
        assert_eq!(failure.max_bytes, 1024);
        assert!(failure.cumulative_bytes > failure.max_bytes, "{failure:?}");
        assert_eq!(failure.over_line_path, "payload.bin");
    }

    /// /tmp 真实量级 fixture(复刻 2026-10-16 现场:node_modules >64MiB、
    /// Java target 数十 MiB、源码 <1MiB):默认 64MiB 预算下,被忽略路径
    /// 不计入——快照必须通过(owner 现场清理后 672K 可过的对偶面)。
    #[test]
    fn tmp_fixture_ignored_node_modules_and_target_fit_default_budget() {
        let temp = tempfile::tempdir_in("/tmp").unwrap();
        let root = temp.path().join("test-center-java");
        std::fs::create_dir_all(root.join("frontend/src")).unwrap();
        std::fs::write(root.join("frontend/.gitignore"), "node_modules/\ndist/\n").unwrap();
        std::fs::write(root.join("frontend/src/main.js"), "1\n").unwrap();
        std::fs::create_dir_all(root.join("frontend/node_modules/element-plus/es/components"))
            .unwrap();
        let oversize_map = std::fs::File::create(
            root.join("frontend/node_modules/element-plus/es/components/big.mjs.map"),
        )
        .unwrap();
        oversize_map.set_len(70 * 1024 * 1024).unwrap();
        drop(oversize_map);
        std::fs::create_dir_all(root.join("busi/.git/info")).unwrap();
        std::fs::write(root.join("busi/.git/info/exclude"), "target/\n").unwrap();
        std::fs::write(root.join("busi/pom.xml"), "<p/>\n").unwrap();
        std::fs::create_dir_all(root.join("busi/target")).unwrap();
        let fat_jar = std::fs::File::create(root.join("busi/target/app.jar")).unwrap();
        fat_jar.set_len(45 * 1024 * 1024).unwrap();
        drop(fat_jar);
        let root = root.canonicalize().unwrap();

        let auditor = RootRecipeFilesystemAuditor::new();
        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &root, step, 1, command)
            .unwrap();
        let paths = snapshot_paths(&watch.before);
        assert!(
            !paths
                .iter()
                .any(|path| path.starts_with("frontend/node_modules")
                    || path.starts_with("busi/target")),
            "ignored oversize trees must stay out of the default-budget snapshot: {paths:?}"
        );
        assert!(paths.contains(&"frontend/src/main.js"));
        assert!(paths.contains(&"busi/pom.xml"));
    }

    include!("root_recipe_receipt_snapshot_tests.inc.rs");
}
