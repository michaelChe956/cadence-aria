    // Task 3（aggregate-policy-root-publication，REQ-BOOT-06）：首次登记的
    // 用户文件冲突检查不被 recipe 重跑证明放宽。本文件经
    // registration_tests.inc.rs 在 mod tests 内 include。

    /// 用真实 auditor 走完四命令审计并 finalize 最终 receipt（全部
    /// Allowed），返回 AGENTS/CLAUDE 各自旧 snapshot 原字节 digest 组成的
    /// 窄 map——与 coordinator 侧 proven map 同构的"归属证明"。
    fn freeze_replay_receipt_and_map(
        paths: &ProductAppPaths,
        canonical_root: &Path,
    ) -> std::collections::BTreeMap<String, String> {
        let store = crate::product::logical_codebase::RootRecipeReceiptStore::new(paths.clone());
        let auditor = crate::product::logical_codebase::RootRecipeFilesystemAuditor::new();
        let operation_id = "aggregate_initialization_registration_ownership_0001";
        let mut last_after = None;
        for (index, (step, command_index, command)) in
            crate::product::logical_codebase::root_recipe_command_index()
                .into_iter()
                .enumerate()
        {
            let watch = auditor
                .before_command(operation_id, canonical_root, step, command_index, command)
                .unwrap();
            let receipt = auditor
                .after_command(watch, format!("2026-10-02T00:10:{index:02}Z"))
                .unwrap();
            assert_eq!(
                receipt.verdict,
                crate::product::logical_codebase::RootRecipeCommandVerdict::Allowed
            );
            store
                .append_command("project_0001", receipt.clone())
                .unwrap();
            last_after = Some(receipt.after_snapshot);
        }
        let last_after = last_after.expect("four commands always run");
        let rule_digest =
            crate::product::logical_codebase::root_recipe_receipt::root_rule_digest(canonical_root)
                .unwrap()
                .expect("AGENTS.md digest");
        let receipt = store
            .finalize(
                "project_0001",
                operation_id,
                "sha256:bootstrap-placeholder-policy",
                &rule_digest,
                "2026-10-02T00:20:00Z".to_string(),
            )
            .unwrap();
        // 证据自洽：最终 rule digest 与末条 snapshot 的 AGENTS 摘要一致。
        let agents_digest = last_after
            .entries
            .iter()
            .find(|entry| entry.path == "AGENTS.md")
            .and_then(|entry| entry.content_digest.clone())
            .expect("AGENTS.md snapshot entry");
        assert_eq!(receipt.rule_digest, agents_digest);
        let mut proven = std::collections::BTreeMap::new();
        proven.insert("AGENTS.md".to_string(), agents_digest);
        if let Some(claude_digest) = last_after
            .entries
            .iter()
            .find(|entry| entry.path == "CLAUDE.md")
            .and_then(|entry| entry.content_digest.clone())
        {
            proven.insert("CLAUDE.md".to_string(), claude_digest);
        }
        proven
    }

    /// 带真实旧 receipt 也直接调用公开 validate：AGENTS/CLAUDE/.aria 仍
    /// ownership conflict；`validate_recipe_replay` 的 map 含未知名字不能
    /// 绕过检查，`.aria` 永不豁免——只有证明一致的窄 map 在 recipe 入口
    /// 豁免这两个文件的存在冲突。
    #[test]
    fn recipe_replay_does_not_relax_first_registration_ownership() {
        let fixture = aggregate_root_fixture();
        fixture.init_git_at_member();
        let agents = "# aggregate root rules\n";
        let claude = "# claude root instructions\n";
        fs::write(fixture.root.join("AGENTS.md"), agents).unwrap();
        fs::write(fixture.root.join("CLAUDE.md"), claude).unwrap();

        // 真实旧证据在场（四 Allowed + 最终 receipt）：公开 validate 不消费
        // 它，首次登记的用户文件冲突检查保持原样。
        let canonical_root = fs::canonicalize(&fixture.root).unwrap();
        let proven = freeze_replay_receipt_and_map(&fixture.paths, &canonical_root);
        assert_eq!(proven.len(), 2, "AGENTS 与 CLAUDE 都有 snapshot 摘要");

        let member = std::slice::from_ref(&fixture.member);
        let assert_conflict = |label: &str| {
            let error = fixture
                .preflight()
                .validate("project_0001", &fixture.root, member)
                .expect_err(label);
            assert_eq!(
                error.code(),
                "aggregate_root_ownership_conflict",
                "{label}: unexpected error {error:?}"
            );
        };
        // AGENTS/CLAUDE 同时在场 → 冲突（首个被拒的是 CLAUDE.md）。
        assert_conflict("both entries present");
        // 仅 AGENTS 在场 → 冲突。
        fs::remove_file(fixture.root.join("CLAUDE.md")).unwrap();
        assert_conflict("AGENTS only");
        // 仅 `.aria` 在场 → 冲突（永不豁免）。
        fs::remove_file(fixture.root.join("AGENTS.md")).unwrap();
        fs::create_dir_all(fixture.root.join(".aria")).unwrap();
        assert_conflict(".aria only");
        fs::remove_dir_all(fixture.root.join(".aria")).unwrap();

        // map 含未知名字不能绕过检查：白名单外的一律拒绝，不静默忽略。
        fs::write(fixture.root.join("AGENTS.md"), agents).unwrap();
        fs::write(fixture.root.join("CLAUDE.md"), claude).unwrap();
        let mut unknown = std::collections::BTreeMap::new();
        unknown.insert(
            "NOTES.md".to_string(),
            "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        );
        let error = fixture
            .preflight()
            .validate_recipe_replay("project_0001", &fixture.root, member, &unknown)
            .expect_err("unknown map names must not bypass ownership checks");
        assert_eq!(error.code(), "aggregate_root_ownership_conflict");

        // 对照分野：证明一致的窄 map 在 recipe 入口豁免这两个文件的存在
        // 冲突（成员边界照常复验）；`.aria` 在场时 recipe 入口同样拒绝。
        let replayed = fixture
            .preflight()
            .validate_recipe_replay("project_0001", &fixture.root, member, &proven)
            .expect("proven entries must be exempt in the recipe replay entry");
        assert_eq!(replayed.canonical_path, canonical_root);
        fs::create_dir_all(fixture.root.join(".aria")).unwrap();
        let error = fixture
            .preflight()
            .validate_recipe_replay("project_0001", &fixture.root, member, &proven)
            .expect_err(".aria is never exempt");
        assert_eq!(error.code(), "aggregate_root_ownership_conflict");
    }
