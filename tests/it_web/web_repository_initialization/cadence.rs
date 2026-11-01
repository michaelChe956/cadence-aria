struct NoGitRunner;

struct LocalOriginRunner {
    origin: PathBuf,
    calls: Mutex<Vec<Vec<String>>>,
}

#[async_trait::async_trait]
impl BoundedCommandRunner for LocalOriginRunner {
    async fn run(
        &self,
        mut request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        // Bug45/T4 修复轮2:生产 git_request 在 argv 前置平台受信禁桥参数
        // (core.hooksPath=/dev/null + core.fsmonitor=false);mock 断言前缀
        // 在位后按剥桥的逻辑子命令改写镜像 URL 为本地 origin、按逻辑
        // argv 记录 calls(末尾断言消费逻辑段)。
        let bridge_len = cadence_aria::cross_cutting::git_command::TRUSTED_GIT_BRIDGE_ARGS.len();
        assert!(
            request.argv.len() > bridge_len
                && request.argv[..bridge_len]
                    .iter()
                    .zip(
                        cadence_aria::cross_cutting::git_command::TRUSTED_GIT_BRIDGE_ARGS.iter()
                    )
                    .all(|(actual, expected)| actual == expected),
            "git argv missing trusted bridge prefix: {:?}",
            request.argv
        );
        let logical = &request.argv[bridge_len..];
        self.calls
            .lock()
            .expect("calls")
            .push(logical.to_vec());
        if logical.first().map(String::as_str) == Some("clone") {
            request.argv[bridge_len + 1] = self.origin.to_string_lossy().into_owned();
        } else if logical.len() == 4
            && logical[0] == "remote"
            && logical[1] == "set-url"
            && logical[2] == "origin"
        {
            request.argv[bridge_len + 3] = self.origin.to_string_lossy().into_owned();
        }
        TokioBoundedCommandRunner.run(request).await
    }
}

#[tokio::test]
async fn repository_initialization_cadence_online_clone_update_and_no_upstream_are_local() {
    let source_repo = git_repo();
    copy_fixture_tree(
        Path::new(CADENCE_FIXTURE),
        &source_repo.path().join("cadence-init/skills"),
    );
    run_git(source_repo.path(), &["add", "cadence-init/skills"]);
    run_git(source_repo.path(), &["commit", "--quiet", "-m", "skills"]);
    let origin_parent = tempdir().expect("origin parent");
    let origin = origin_parent.path().join("cadence-skills.git");
    let status = Command::new("git")
        .args(["init", "--bare", "--quiet"])
        .arg(&origin)
        .status()
        .expect("bare origin");
    assert!(status.success());
    let origin_text = origin.to_string_lossy().into_owned();
    run_git(
        source_repo.path(),
        &["remote", "add", "origin", &origin_text],
    );
    run_git(
        source_repo.path(),
        &["push", "--quiet", "-u", "origin", "master"],
    );

    let root = tempdir().expect("root");
    let home = root.path().join("home");
    let runner = Arc::new(LocalOriginRunner {
        origin,
        calls: Mutex::new(Vec::new()),
    });
    let environment = std::env::var("PATH")
        .ok()
        .map(|path| BTreeMap::from([("PATH".to_string(), path)]))
        .unwrap_or_default();
    let manager = CadenceSkillsManager::with_dependencies(&home, runner.clone(), environment);
    let cloned = manager
        .prepare(CancellationToken::new())
        .await
        .expect("online clone");
    assert_eq!(cloned.source_mode, CadenceSkillsSourceMode::OnlineClone);
    assert!(cloned.git_updated);

    fs::write(
        source_repo
            .path()
            .join("cadence-init/skills/alpha/SKILL.md"),
        "updated alpha\n",
    )
    .expect("updated fixture");
    run_git(
        source_repo.path(),
        &["add", "cadence-init/skills/alpha/SKILL.md"],
    );
    run_git(
        source_repo.path(),
        &["commit", "--quiet", "-m", "update alpha"],
    );
    run_git(source_repo.path(), &["push", "--quiet"]);
    let updated = manager
        .prepare(CancellationToken::new())
        .await
        .expect("online update");
    assert_eq!(updated.source_mode, CadenceSkillsSourceMode::OnlineUpdate);
    assert!(updated.git_updated);
    assert_eq!(
        fs::read_to_string(home.join(".agents/Cadence-skills/cadence-init/skills/alpha/SKILL.md"))
            .expect("updated alpha"),
        "updated alpha\n"
    );

    run_git(
        &home.join(".agents/Cadence-skills"),
        &["branch", "--unset-upstream"],
    );
    let no_upstream = manager
        .prepare(CancellationToken::new())
        .await
        .expect("no-upstream update");
    assert_eq!(
        no_upstream.source_mode,
        CadenceSkillsSourceMode::OnlineUpdate
    );
    assert!(!no_upstream.git_updated);
    assert_eq!(no_upstream.warnings, vec!["cadence_skills_no_upstream"]);
    let calls = runner.calls.lock().expect("calls");
    assert!(
        calls
            .iter()
            .any(|argv| argv.starts_with(&["fetch".to_string(), "--all".to_string()]))
    );
    assert!(
        calls
            .iter()
            .any(|argv| argv.starts_with(&["pull".to_string(), "--ff-only".to_string()]))
    );
}

#[async_trait::async_trait]
impl BoundedCommandRunner for NoGitRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        panic!(
            "offline Cadence branch must not execute {}",
            request.executable
        )
    }
}

#[tokio::test]
async fn repository_initialization_cadence_offline_syncs_three_link_layers_safely() {
    let root = tempdir().expect("root");
    let home = root.path().join("home");
    let source = home.join(".agents/Cadence-skills/cadence-init/skills");
    copy_fixture_tree(Path::new(CADENCE_FIXTURE), &source);
    let manager =
        CadenceSkillsManager::with_dependencies(&home, Arc::new(NoGitRunner), BTreeMap::new());
    let result = manager
        .prepare(CancellationToken::new())
        .await
        .expect("offline prepare");
    assert_eq!(result.source_mode, CadenceSkillsSourceMode::Offline);
    assert!(!result.git_updated);
    for skill in ["alpha", "beta"] {
        let source_skill = source.join(skill);
        let shared = home.join(".agents/skills").join(skill);
        let codex = home.join(".codex/skills/skills").join(skill);
        let claude = home.join(".claude/skills").join(skill);
        assert_eq!(fs::read_link(&shared).expect("shared link"), source_skill);
        assert_eq!(fs::read_link(&codex).expect("codex link"), shared);
        assert_eq!(fs::read_link(&claude).expect("claude link"), shared);
    }
    let second = manager
        .prepare(CancellationToken::new())
        .await
        .expect("idempotent prepare");
    assert!(second.warnings.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn repository_initialization_cadence_link_conflicts_preserve_user_content() {
    use std::os::unix::fs::symlink;

    let root = tempdir().expect("root");
    let home = root.path().join("home");
    let source = home.join(".agents/Cadence-skills/cadence-init/skills");
    copy_fixture_tree(Path::new(CADENCE_FIXTURE), &source);
    let shared_root = home.join(".agents/skills");
    let codex_root = home.join(".codex/skills/skills");
    fs::create_dir_all(&shared_root).expect("shared root");
    fs::create_dir_all(&codex_root).expect("codex root");
    let old_managed = home.join(".agents/Cadence-skills/old/skills/alpha");
    symlink(&old_managed, shared_root.join("alpha")).expect("old managed link");
    fs::write(shared_root.join("beta"), "user file\n").expect("user content");
    let unrelated = root.path().join("unrelated-alpha");
    symlink(&unrelated, codex_root.join("alpha")).expect("unrelated link");

    let manager =
        CadenceSkillsManager::with_dependencies(&home, Arc::new(NoGitRunner), BTreeMap::new());
    let result = manager
        .prepare(CancellationToken::new())
        .await
        .expect("conflict prepare");

    assert_eq!(
        fs::read_link(shared_root.join("alpha")).expect("replaced managed link"),
        source.join("alpha")
    );
    assert_eq!(
        fs::read_link(codex_root.join("alpha")).expect("preserved unrelated link"),
        unrelated
    );
    assert_eq!(
        fs::read_to_string(shared_root.join("beta")).expect("preserved user file"),
        "user file\n"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("unrelated_symlink"))
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("user_content"))
    );
}
