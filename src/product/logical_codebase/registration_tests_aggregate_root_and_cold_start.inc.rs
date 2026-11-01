    #[test]
    #[cfg(unix)]
    fn aggregate_root_preflight_rejects_git_root_and_symlink_escape() {
        let fixture = aggregate_root_fixture();
        fixture.init_git_at_root();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap_err()
                .code(),
            "aggregate_root_is_git"
        );

        fixture.remove_root_git();
        fixture.create_symlink_member_to_outside_root();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.symlink_member),
                )
                .unwrap_err()
                .code(),
            "member_symlink_escape"
        );
    }

    #[test]
    #[cfg(unix)]
    fn aggregate_root_preflight_rejects_members_outside_root_nested_worktrees_and_overlap() {
        let fixture = aggregate_root_fixture();
        let outside_member = fixture.root.parent().unwrap().join("outside-member");
        fs::create_dir(&outside_member).unwrap();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&outside_member),
                )
                .unwrap_err()
                .code(),
            "member_path_outside_root"
        );
        assert!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.root),
                )
                .is_err_and(|error| error.code() == "member_path_outside_root")
        );

        fixture.init_git_at_member();
        fixture.create_nested_worktree_at_member();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.nested_worktree_member),
                )
                .unwrap_err()
                .code(),
            "nested_worktree"
        );

        fixture.remove_nested_worktree();
        fixture.save_manifest_for("project_0002", fixture.root.parent().unwrap());
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap_err()
                .code(),
            "aggregate_root_overlap"
        );
    }

    #[test]
    #[cfg(unix)]
    fn aggregate_root_preflight_accepts_non_git_root_with_direct_git_members() {
        let fixture = aggregate_root_fixture();
        fixture.init_git_at_member();

        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap()
                .canonical_path,
            fs::canonicalize(&fixture.root).unwrap()
        );
    }

    /// owner 2026-10-29：根目录的 Cadence-skills 产物（`AGENTS.md` /
    /// `CLAUDE.md` / `.claude/rules/*` / `.mcp.json` / `.gitignore`）不再是
    /// 登记门禁——任何做过 skills 初始化的项目必然有这些文件，旧门禁让这类
    /// 项目永远无法添加 LC。登记只保留 root 结构/成员/Git/重叠检查。
    #[test]
    #[cfg(unix)]
    fn aggregate_root_preflight_accepts_initialized_root_with_user_rule_entries() {
        let fixture = aggregate_root_fixture();
        fixture.init_git_at_member();
        for (relative_path, contents) in [
            ("AGENTS.md", "# aggregate root rules\n"),
            ("CLAUDE.md", "# claude root instructions\n"),
            (".mcp.json", "{\"mcpServers\":{}}\n"),
            (".gitignore", "target/\n"),
            (".claude/rules/language.md", "# language rule\n"),
        ] {
            fixture.create_file(relative_path, contents);
        }

        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap()
                .canonical_path,
            fs::canonicalize(&fixture.root).unwrap()
        );
    }

    /// owner 2026-10-29：同一根目录不允许出现两个 LC——同项目不同 LC、跨项目
    /// 都拒绝（`new` 未指明 LC 时不跳过任何 manifest）。
    #[test]
    #[cfg(unix)]
    fn aggregate_root_preflight_rejects_second_lc_on_same_root() {
        let fixture = aggregate_root_fixture();
        fixture.init_git_at_member();

        // 同项目、不同 LC（per-LC 子树 manifest）、相同根 → 拒绝。
        fixture.save_lc_manifest_for("lc-a", "project_0001", &fixture.root);
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap_err()
                .code(),
            "aggregate_root_overlap"
        );

        // 指明了别的 LC（`for_lc`）同样拒绝——只跳过“自己”。
        assert_eq!(
            fixture
                .preflight_for_lc("lc-b")
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap_err()
                .code(),
            "aggregate_root_overlap"
        );

        // 跨项目、相同根（per-LC 子树 manifest）→ 拒绝。
        fixture.save_lc_manifest_for("lc-c", "project_0002", &fixture.root);
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0002",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap_err()
                .code(),
            "aggregate_root_overlap"
        );
    }


    /// owner 2026-10-29：聚合初始化复验（同一个 LC 重跑）不得把自己拦掉。
    #[test]
    #[cfg(unix)]
    fn aggregate_root_preflight_rerun_for_same_lc_passes() {
        let fixture = aggregate_root_fixture();
        fixture.init_git_at_member();

        // LC 自己的 manifest 落在同一根（登记后的复验形态）→ 放行。
        fixture.save_lc_manifest_for("lc-a", "project_0001", &fixture.root);
        assert_eq!(
            fixture
                .preflight_for_lc("lc-a")
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap()
                .canonical_path,
            fs::canonicalize(&fixture.root).unwrap()
        );

        // legacy 别名 LC 的复验同理：legacy manifest 以 legacy id 为自身键，
        // 跳过自身、拦住别人。
        let legacy_fixture = aggregate_root_fixture();
        legacy_fixture.init_git_at_member();
        legacy_fixture.save_manifest_for("project_0001", &legacy_fixture.root);
        let legacy_id = crate::product::logical_codebase::legacy_logical_codebase_id("project_0001");
        assert!(legacy_fixture
            .preflight_for_lc(&legacy_id)
            .validate(
                "project_0001",
                &legacy_fixture.root,
                std::slice::from_ref(&legacy_fixture.member),
            )
            .is_ok());
        assert_eq!(
            legacy_fixture
                .preflight_for_lc("lc-b")
                .validate(
                    "project_0001",
                    &legacy_fixture.root,
                    std::slice::from_ref(&legacy_fixture.member),
                )
                .unwrap_err()
                .code(),
            "aggregate_root_overlap"
        );
    }

    /// k3 审查建议 B7（2026-10-29）：没有 manifest 的 LC（冷启动 / 空成员 /
    /// 只跑过聚合初始化）也有 `record.json` 的 `aggregate_root`，必须参与唯一性
    /// 判定——旧枚举只认 manifest，这类 LC 不占根，两个 LC 可以登记同一个根。
    /// 墓碑状态同样占根（restore 语义要求归属保留到 purge），purge 之后才释放。
    #[test]
    fn aggregate_root_preflight_counts_manifest_less_and_tombstoned_lc_records() {
        let fixture = aggregate_root_fixture();
        let canonical_root = fs::canonicalize(&fixture.root).unwrap();
        let store = LogicalCodebaseStore::new(fixture.paths.clone());
        let record = store
            .create(
                "project_0001",
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "cold-start".to_string(),
                    aggregate_root: canonical_root,
                },
            )
            .unwrap();
        // 只有 record.json、没有 manifest.json：这是冷启动/空成员 LC 的形态。
        assert!(
            LogicalCodebaseStore::for_lc(fixture.paths.clone(), &record.id)
                .load_manifest("project_0001")
                .unwrap()
                .is_none()
        );

        // 别的 LC（含未指明自身身份的预检）不得再占用同一个根。
        assert_eq!(
            fixture
                .preflight()
                .validate("project_0001", &fixture.root, &[])
                .unwrap_err()
                .code(),
            "aggregate_root_overlap"
        );
        // 自己复验（for_lc）跳过自身，放行。
        assert!(
            fixture
                .preflight_for_lc(&record.id)
                .validate("project_0001", &fixture.root, &[])
                .is_ok()
        );

        // 软删后仍占根（墓碑 LC 的 record 还在）。
        store.delete_soft("project_0001", &record.id).unwrap();
        assert_eq!(
            fixture
                .preflight()
                .validate("project_0001", &fixture.root, &[])
                .unwrap_err()
                .code(),
            "aggregate_root_overlap"
        );

        // purge 之后归属才释放。
        store.purge("project_0001", &record.id).unwrap();
        assert!(
            fixture
                .preflight()
                .validate("project_0001", &fixture.root, &[])
                .is_ok()
        );
    }

    struct AggregateRootFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        root: PathBuf,
        member: PathBuf,
        symlink_member: PathBuf,
        nested_worktree_member: PathBuf,
    }

    impl AggregateRootFixture {
        fn preflight(&self) -> AggregateRootPreflight {
            AggregateRootPreflight::new(self.paths.clone())
        }


        fn preflight_for_lc(&self, lc_id: &str) -> AggregateRootPreflight {
            AggregateRootPreflight::for_lc(self.paths.clone(), lc_id)
        }
        fn create_file(&self, relative_path: &str, contents: &str) {
            let path = self.root.join(relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, contents).unwrap();
        }

        fn init_git_at_root(&self) {
            git(&self.root, &["init", "--quiet"]);
        }

        fn remove_root_git(&self) {
            fs::remove_dir_all(self.root.join(".git")).unwrap();
        }

        fn init_git_at_member(&self) {
            git(&self.member, &["init", "--quiet"]);
            git(
                &self.member,
                &["config", "user.email", "aria@example.invalid"],
            );
            git(&self.member, &["config", "user.name", "Aria Test"]);
            fs::write(self.member.join("README.md"), "# Member\n").unwrap();
            git(&self.member, &["add", "README.md"]);
            git(&self.member, &["commit", "--quiet", "-m", "initial"]);
        }

        fn create_symlink_member_to_outside_root(&self) {
            let outside_member = self.root.parent().unwrap().join("outside-symlink-target");
            fs::create_dir(&outside_member).unwrap();
            symlink(&outside_member, &self.symlink_member).unwrap();
        }

        fn create_nested_worktree_at_member(&self) {
            git(
                &self.member,
                &[
                    "worktree",
                    "add",
                    "--detach",
                    self.nested_worktree_member.to_str().unwrap(),
                ],
            );
        }

        fn remove_nested_worktree(&self) {
            git(
                &self.member,
                &[
                    "worktree",
                    "remove",
                    "--force",
                    self.nested_worktree_member.to_str().unwrap(),
                ],
            );
        }

        fn save_manifest_for(&self, project_id: &str, root: &Path) {
            let manifest = LogicalCodebaseManifest::new(
                project_id,
                fs::canonicalize(root).unwrap(),
                Vec::new(),
            );
            LogicalCodebaseStore::new(self.paths.clone())
                .save_manifest(project_id, &manifest)
                .unwrap();
        }

        fn save_lc_manifest_for(&self, lc_id: &str, project_id: &str, root: &Path) {
            let manifest = LogicalCodebaseManifest::new(
                project_id,
                fs::canonicalize(root).unwrap(),
                Vec::new(),
            );
            LogicalCodebaseStore::for_lc(self.paths.clone(), lc_id)
                .save_manifest(project_id, &manifest)
                .unwrap();
        }

    }
    fn aggregate_root_fixture() -> AggregateRootFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        let root = temp.path().join("aggregate-root");
        let member = root.join("member");
        let symlink_member = root.join("symlink-member");
        let nested_worktree_member = member.join("nested-worktree");
        fs::create_dir_all(&member).unwrap();
        for name in ["project", "project-other"] {
            ProjectStore::new(paths.clone())
                .create(CreateProjectInput {
                    name: name.to_string(),
                    description: None,
                })
                .unwrap();
        }

        AggregateRootFixture {
            _temp: temp,
            paths,
            root,
            member,
            symlink_member,
            nested_worktree_member,
        }
    }

    fn scan_fixture() -> ScanFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".into(),
                description: None,
            })
            .unwrap();

        let root = temp.path().join("aggregate-root");
        let clean_git = root.join("clean-git");
        let non_git = root.join("non-git");
        let nested_git = clean_git.join("nested-git");
        let dirty_git = root.join("dirty-git");
        let missing = root.join("missing");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&non_git).unwrap();
        fs::create_dir_all(&outside).unwrap();
        init_git_repository(&clean_git);
        fs::write(clean_git.join(".git/info/exclude"), "nested-git/\n").unwrap();
        init_git_repository(&nested_git);
        init_git_repository(&dirty_git);
        fs::write(dirty_git.join("README.md"), "dirty\n").unwrap();
        ScanFixture {
            _temp: temp,
            paths: paths.clone(),
            root: CanonicalAggregateRoot {
                canonical_path: fs::canonicalize(root).unwrap(),
            },
            clean_git,
            non_git,
            nested_git,
            dirty_git,
            missing,
            outside,
            coordinator: LogicalCodebaseRegistrationCoordinator::new(paths),
        }
    }

    fn linked_worktree_fixture() -> LinkedWorktreeFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
            })
            .unwrap();
        let root_path = temp.path().join("aggregate-root");
        let main = root_path.join("main");
        let linked = root_path.join("linked");
        init_git_repository(&main);
        git(
            &main,
            &["worktree", "add", "--detach", linked.to_str().unwrap()],
        );
        LinkedWorktreeFixture {
            _root: temp,
            root: CanonicalAggregateRoot {
                canonical_path: fs::canonicalize(&root_path).unwrap(),
            },
            linked,
            coordinator: LogicalCodebaseRegistrationCoordinator::new(paths),
        }
    }

    fn fifty_repository_fixture() -> FiftyRepositoryFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
            })
            .unwrap();
        let root_path = temp.path().join("aggregate-root");
        let repositories = (1..=50)
            .map(|index| {
                let path = root_path.join(format!("repo_{index:04}"));
                init_git_repository(&path);
                path
            })
            .collect::<Vec<_>>();
        FiftyRepositoryFixture {
            _root: temp,
            root: CanonicalAggregateRoot {
                canonical_path: fs::canonicalize(root_path).unwrap(),
            },
            repositories,
            coordinator: LogicalCodebaseRegistrationCoordinator::new(paths),
        }
    }

    fn batch_fixture_with_two_git_repositories() -> BatchFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
            })
            .unwrap();
        let root_path = temp.path().join("aggregate-root");
        let first = root_path.join("first");
        let second = root_path.join("second");
        init_git_repository(&first);
        init_git_repository(&second);
        BatchFixture {
            _root: temp,
            root: CanonicalAggregateRoot {
                canonical_path: fs::canonicalize(&root_path).unwrap(),
            },
            first,
            second,
            coordinator: LogicalCodebaseRegistrationCoordinator::new(paths),
        }
    }

    fn init_git_repository(path: &Path) {
        fs::create_dir_all(path).unwrap();
        git(path, &["init", "--quiet"]);
        git(path, &["config", "user.email", "aria@example.invalid"]);
        git(path, &["config", "user.name", "Aria Test"]);
        fs::write(path.join("README.md"), "# Repository\n").unwrap();
        git(path, &["add", "README.md"]);
        git(path, &["commit", "--quiet", "-m", "initial"]);
    }

    /// Bug45/T4 fixture 根:遵守批2 GC5(测试临时物在 /tmp/aria-bug45/<task>
    /// 私有子目录,不散落 owner HOME 或真实仓库)。
    fn bridge_fixture_dir() -> tempfile::TempDir {
        let base = Path::new("/tmp/aria-bug45/t4");
        fs::create_dir_all(base).expect("create /tmp/aria-bug45/t4 fixture base");
        tempfile::tempdir_in(base).expect("fixture dir under /tmp/aria-bug45/t4")
    }

    /// Bug45:registration 生产 git_probe/git_stdout(capture 链)是平台
    /// 受信启动面,不得执行 coder 可写的仓库执行桥——`core.fsmonitor` 探针
    /// 与 `.git/hooks`;allowlist 命令合同必须原样保留。
    #[test]
    fn registration_probe_and_capture_do_not_execute_repo_bridges() {
        use std::os::unix::fs::PermissionsExt;

        let temp = bridge_fixture_dir();
        let repo = temp.path().join("member");
        init_git_repository(&repo);

        let pre_commit_probe = repo.join(".git/aria-pre-commit-fired");
        fs::write(
            repo.join(".git/hooks/pre-commit"),
            format!("#!/bin/sh\n: > {}\nexit 0\n", pre_commit_probe.display()),
        )
        .expect("write pre-commit probe");
        fs::set_permissions(
            repo.join(".git/hooks/pre-commit"),
            fs::Permissions::from_mode(0o755),
        )
        .expect("chmod pre-commit probe");
        let fsmonitor_probe = repo.join(".git/aria-fsmonitor-fired");
        let fsmonitor_hook = repo.join(".git/aria-fsmonitor-hook.sh");
        fs::write(
            &fsmonitor_hook,
            format!("#!/bin/sh\n: > {}\nexit 1\n", fsmonitor_probe.display()),
        )
        .expect("write fsmonitor probe script");
        fs::set_permissions(&fsmonitor_hook, fs::Permissions::from_mode(0o755))
            .expect("chmod fsmonitor probe script");
        git(
            &repo,
            &[
                "config",
                "core.fsmonitor",
                fsmonitor_hook.to_str().expect("utf8 fsmonitor hook path"),
            ],
        );

        // capture 全链(status/rev-parse/for-each-ref/worktree list/config)零触发。
        RepositoryGitSnapshot::capture(&repo).expect("capture snapshot");
        assert!(
            !fsmonitor_probe.exists(),
            "Bug45: registration capture 内 git status 不得执行仓库 fsmonitor 探针"
        );

        // git_probe 的允许清单命令零触发。
        git_probe(&repo, &["status", "--porcelain"])
            .expect("probe allowed status")
            .expect("status output");
        assert!(
            !fsmonitor_probe.exists(),
            "Bug45: git_probe status 不得执行仓库 fsmonitor 探针"
        );

        // allowlist 合同保留:清单外命令仍拒绝(禁桥参数不放松命令面)。
        let rejected = git_probe(&repo, &["log", "--oneline"]).unwrap_err();
        assert!(
            matches!(rejected, ProductStoreError::InvalidRecord { .. }),
            "allowlist 合同必须保留,rejected={rejected:?}"
        );

        // 夹具有效性对照:coder 域(无禁桥)git 下探针会触发。
        git(&repo, &["status", "--porcelain"]);
        assert!(
            fsmonitor_probe.exists(),
            "对照失败:coder 域 status 应触发其自身 fsmonitor 探针,夹具失效"
        );
    }

    fn attach_fixture() -> AttachFixture {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path());
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".into(),
                description: None,
            })
            .unwrap();

        let git_root = root.path().join("api");
        fs::create_dir_all(&git_root).unwrap();
        git(&git_root, &["init", "--quiet"]);
        git(&git_root, &["config", "user.email", "aria@example.invalid"]);
        git(&git_root, &["config", "user.name", "Aria Test"]);
        fs::write(git_root.join("README.md"), "# API\n").unwrap();
        git(&git_root, &["add", "README.md"]);
        git(&git_root, &["commit", "--quiet", "-m", "initial"]);

        // v1.3：登记链路不再触发懒惰 identity 迁移 bootstrap manifest；
        // manifest 由首批登记（D7 原子语义）或既有迁移产物存在。
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest(
                "project_0001",
                &LogicalCodebaseManifest::new(
                    "project_0001",
                    root.path().to_path_buf(),
                    Vec::new(),
                ),
            )
            .unwrap();

        let head_before = git_output(&git_root, &["rev-parse", "HEAD"]);
        let branch_before = git_output(&git_root, &["branch", "--show-current"]);
        let status_before = git_output(&git_root, &["status", "--porcelain"]);

        AttachFixture {
            _root: root,
            paths: paths.clone(),
            coordinator: LogicalCodebaseRegistrationCoordinator::new(paths),
            git_root,
            head_before,
            branch_before,
            status_before,
        }
    }

    fn git(cwd: &Path, arguments: &[&str]) {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_output(cwd: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    // ---- C4 Task 4：首批登记冷启动（无 active index）与 command replay ----

    fn lc_first_registration_fixture() -> (
        tempfile::TempDir,
        ProductAppPaths,
        LogicalCodebaseRegistrationCoordinator,
        String,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
            })
            .unwrap();
        let root_path = temp.path().join("aggregate-root");
        let first = root_path.join("first");
        let second = root_path.join("second");
        init_git_repository(&first);
        init_git_repository(&second);
        let lc = LogicalCodebaseStore::new(paths.clone())
            .create(
                "project_0001",
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "cold-start".to_string(),
                    aggregate_root: fs::canonicalize(&root_path).unwrap(),
                },
            )
            .unwrap();
        let coordinator = LogicalCodebaseRegistrationCoordinator::for_lc(paths.clone(), &lc.id);
        (temp, paths, coordinator, lc.id)
    }

    fn git_state_inventory(repo: &Path) -> (String, String, Vec<(String, Vec<u8>)>) {
        let head = git_output(repo, &["rev-parse", "HEAD"]);
        let status = git_output(repo, &["status", "--porcelain"]);
        let mut inventory = Vec::new();
        let mut stack = vec![repo.join(".git")];
        while let Some(dir) = stack.pop() {
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if let (Ok(rel), Ok(bytes)) = (
                        path.strip_prefix(repo),
                        fs::read(&path),
                    ) {
                        inventory.push((rel.to_string_lossy().into_owned(), bytes));
                    }
                }
            }
        }
        inventory.sort();
        (head, status, inventory)
    }

    #[test]
    fn first_lc_registration_creates_manifest_and_checkouts_without_active_index_or_git_writes() {
        let (temp, paths, coordinator, lc_id) = lc_first_registration_fixture();
        let first_repo = temp.path().join("aggregate-root/first");
        let second_repo = temp.path().join("aggregate-root/second");
        let first_before = git_state_inventory(&first_repo);
        let second_before = git_state_inventory(&second_repo);

        // 无 active index、无 manifest 的冷启动：preflight → confirmed batch →
        // resume 完成首批登记。
        let input = ConfirmedRegistrationBatchInput::from_preflight(
            &coordinator
                .preflight(RegistrationPreflightInput {
                    project_id: "project_0001".to_string(),
                    aggregate_root: CanonicalAggregateRoot {
                        canonical_path: fs::canonicalize(temp.path().join("aggregate-root"))
                            .unwrap(),
                    },
                    paths: vec![first_repo.clone(), second_repo.clone()],
                })
                .unwrap(),
            false,
        );
        let batch = coordinator.submit_confirmed_batch(input).unwrap();
        let resumed = coordinator.resume_batch("project_0001", &batch.id).unwrap();
        assert_eq!(
            resumed.status,
            RegistrationBatchStatus::Completed,
            "items: {:?}",
            resumed.items
        );

        // manifest/members/checkouts 全部落在 logical-codebases/{lc_id}/ 子树。
        let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc_id);
        let manifest = lc_store.load_manifest("project_0001").unwrap().expect(
            "first registration must create the manifest inside the lc subtree",
        );
        assert_eq!(manifest.member_ids.len(), 2);
        assert_eq!(manifest.membership_revision, 3);
        let members = lc_store.list_members("project_0001").unwrap();
        assert_eq!(members.len(), 2);
        let mut digests = members
            .iter()
            .map(|member| member.source_identity.key_digest.clone())
            .collect::<Vec<_>>();
        digests.sort();
        digests.dedup();
        assert_eq!(digests.len(), 2, "no duplicate source identity");
        let checkouts = lc_store.list_checkouts("project_0001").unwrap();
        assert_eq!(checkouts.len(), 2);

        // 没有 active aggregate index 也不阻塞成员登记。
        assert!(
            crate::product::logical_codebase::aggregate_index::AggregateIndexStore::for_lc(
                paths.clone(),
                &lc_id
            )
            .active("project_0001")
            .unwrap()
            .is_none()
        );
        // 首建 policy bootstrap 从 manifest 事实可用。
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            paths.clone(),
            &lc_id,
        )
        .ensure_bootstrap(&manifest)
        .unwrap();

        // 成员仓零 Git 写副作用。
        assert_eq!(git_state_inventory(&first_repo), first_before);
        assert_eq!(git_state_inventory(&second_repo), second_before);
    }

    #[test]
    fn bootstrap_registration_replay_same_command_returns_same_batch_and_no_duplicate_members() {
        let (temp, paths, coordinator, lc_id) = lc_first_registration_fixture();
        let root = CanonicalAggregateRoot {
            canonical_path: fs::canonicalize(temp.path().join("aggregate-root")).unwrap(),
        };
        let input = ConfirmedRegistrationBatchInput::from_preflight(
            &coordinator
                .preflight(RegistrationPreflightInput {
                    project_id: "project_0001".to_string(),
                    aggregate_root: root,
                    paths: vec![
                        temp.path().join("aggregate-root/first"),
                        temp.path().join("aggregate-root/second"),
                    ],
                })
                .unwrap(),
            false,
        );

        // 同一 command/idempotency key 重放：返回同一 durable batch。
        let first = coordinator.submit_confirmed_batch(input.clone()).unwrap();
        let replayed = coordinator.submit_confirmed_batch(input).unwrap();
        assert_eq!(first.id, replayed.id);
        assert_eq!(first.idempotency_key, replayed.idempotency_key);
        assert_eq!(first.items.len(), replayed.items.len());
        let first_ids = first
            .items
            .iter()
            .map(|item| item.source_digest.clone())
            .collect::<Vec<_>>();
        let replayed_ids = replayed
            .items
            .iter()
            .map(|item| item.source_digest.clone())
            .collect::<Vec<_>>();
        assert_eq!(first_ids, replayed_ids);

        coordinator.resume_batch("project_0001", &first.id).unwrap();
        let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc_id);
        let members = lc_store.list_members("project_0001").unwrap();
        assert_eq!(members.len(), 2, "replay must not duplicate members");
        let manifest = lc_store.load_manifest("project_0001").unwrap().unwrap();

        // 过期 expected revision → fail-closed conflict，不产生新批次写入。
        let error = coordinator
            .resume_batch_checked("project_0001", &first.id, manifest.membership_revision + 1)
            .unwrap_err();
        assert!(
            matches!(
                &error,
                crate::product::json_store::ProductStoreError::Conflict { kind, id }
                    if *kind == "registration_batch_manifest_revision_mismatch" && id == &first.id
            ),
            "unexpected error: {error:?}"
        );

        // 正确 revision 的显式 resume 幂等返回已完成批次。
        let checked = coordinator
            .resume_batch_checked("project_0001", &first.id, manifest.membership_revision)
            .unwrap();
        assert_eq!(checked.status, RegistrationBatchStatus::Completed);
        assert_eq!(
            LogicalCodebaseStore::for_lc(paths.clone(), &lc_id)
                .list_members("project_0001")
                .unwrap()
                .len(),
            2
        );
    }
