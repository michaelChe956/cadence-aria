/// 机器上**同时存在两份安装**：PATH 上生效的前缀 A 有一份、兜底前缀 B 里
/// 还有一份旧的 ⇒ 更新必须落在 **A**（用户实际在用的那份），不得写 B ——
/// 否则就是「更新了但用户跑的还是旧版」（2.10①）。
#[tokio::test]
async fn update_prefers_prefix_of_cli_visible_on_path_over_stale_fallback_copy() {
    let root = tempdir().expect("root");
    // A：PATH 上生效的前缀（`<A>/bin/codex` 存在 ⇒ visible）。
    let visible = root.path().join("visible-prefix");
    std::fs::create_dir_all(visible.join("bin")).expect("visible bin");
    std::fs::write(visible.join("bin").join("codex"), b"#!/bin/sh\n").expect("visible cli");
    // B：兜底前缀里还有一份旧的安装（`<B>/lib/node_modules/<pkg>`）。
    let fallback = root.path().join("fallback-prefix");
    std::fs::create_dir_all(
        fallback
            .join("lib")
            .join("node_modules")
            .join("@openai")
            .join("codex"),
    )
    .expect("fallback package dir");

    let mut execution = execution(
        root.path(),
        ScriptedRunner::new(Vec::new(), Duration::ZERO),
        SpyProber::new(Some("1.0.0")),
    );
    execution.path_value = Some(visible.join("bin").to_string_lossy().into_owned());
    execution.npm_prefix = fallback.clone();

    let row = row_for_wire("codex").expect("codex row");
    let resolved = execution
        .resolve_effective_prefix(row, execution.path_value.as_deref())
        .await;

    let expected = std::fs::canonicalize(&visible).expect("canonical visible prefix");
    assert_eq!(
        resolved, expected,
        "PATH 上生效的前缀优先：更新不得写进兜底前缀里的另一份旧安装"
    );
}

/// 「包在、版本读得到，但装后**真实命令跑不起来**」⇒ 不得判 completed
/// （假绿）；必须走 `--include=optional` 重试一次；重试后仍失败 ⇒ 作业
/// failed，且文案点名「装上但缺平台包」（2.8③）。
#[tokio::test]
async fn real_command_failure_is_not_completed_and_retries_with_include_optional() {
    let root = tempdir().expect("root");
    let install_runner = ScriptedRunner::new(
        vec![success("added 1 package"), success("added 1 package")],
        Duration::ZERO,
    );
    let mut execution = execution(
        root.path(),
        install_runner.clone(),
        SpyProber::with_real_command(Some("1.0.0"), false),
    );
    execution.path_value = Some(root.path().join("empty-bin").to_string_lossy().into_owned());
    execution.npm_prefix = root.path().join("installation");
    // 装上了：安装产物里有该命令 ⇒ 属「装上但缺平台包」而非「没装上」。
    std::fs::create_dir_all(execution.npm_prefix.join("bin")).expect("bin dir");
    std::fs::write(execution.npm_prefix.join("bin").join("codex"), b"#!/bin/sh\n").expect("cli");

    let job = execution
        .store
        .create("codex", FixedClock.now())
        .expect("create");
    execution
        .run(&job.job_id, InstallTarget::Provider(ProviderName::Codex))
        .await;

    let requests = install_runner.requests();
    assert_eq!(
        requests.len(),
        2,
        "真实命令跑不起来必须走 --include=optional 重试一次: {requests:?}"
    );
    assert!(
        requests[1].argv.contains(&"--include=optional".to_string()),
        "重试必须带 --include=optional: {:?}",
        requests[1].argv
    );

    let done = execution
        .store
        .get(&job.job_id)
        .expect("io")
        .expect("job exists");
    assert!(
        done.outcome.is_none(),
        "真实命令跑不起来不得判成功（假绿）: {:?}",
        done.outcome
    );
    let failure = done.failure.expect("重试后仍失败必须落 failed 载荷");
    assert!(
        failure.message.contains("缺平台包"),
        "必须点名「装上但缺平台包」: {}",
        failure.message
    );
    assert!(
        failure.manual_command.contains("--include=optional"),
        "必须给可复制修复命令: {}",
        failure.manual_command
    );
}

/// 版本读不到但真实命令能跑 ⇒ 属「装上但**不在 PATH** 上」，MUST NOT 与
/// 「装上但缺平台包」共用同一条含糊文案（2.8③）。
#[tokio::test]
async fn installed_but_off_path_failure_is_distinguished_from_missing_platform_package() {
    let root = tempdir().expect("root");
    let install_runner = ScriptedRunner::new(
        vec![success("added 1 package"), success("added 1 package")],
        Duration::ZERO,
    );
    // 真实命令可跑（`--help` / files 类），但探测不到版本 ⇒ PATH 口径取不到。
    let mut execution = execution(
        root.path(),
        install_runner,
        SpyProber::with_real_command(None, true),
    );
    execution.path_value = Some(root.path().join("empty-bin").to_string_lossy().into_owned());
    execution.npm_prefix = root.path().join("installation");
    std::fs::create_dir_all(execution.npm_prefix.join("bin")).expect("bin dir");
    std::fs::write(execution.npm_prefix.join("bin").join("codex"), b"#!/bin/sh\n").expect("cli");

    let job = execution
        .store
        .create("codex", FixedClock.now())
        .expect("create");
    execution
        .run(&job.job_id, InstallTarget::Provider(ProviderName::Codex))
        .await;

    let done = execution
        .store
        .get(&job.job_id)
        .expect("io")
        .expect("job exists");
    let failure = done.failure.expect("取不到版本必须落 failed 载荷");
    assert!(
        failure.message.contains("不在 PATH"),
        "必须点名「装上但不在 PATH 上」: {}",
        failure.message
    );
    assert!(
        !failure.message.contains("缺平台包"),
        "MUST NOT 与「缺平台包」共用含糊文案: {}",
        failure.message
    );
}

/// 已安装位置在 **raw `npm prefix -g`** 里、但那个 bin **不在 PATH** 上
/// （本轮核心现场）⇒ 更新必须装回它（2.10②）；`bin ∈ PATH` 过滤只适用于
/// **新装候选**，不得把已安装位置丢掉 ⇒ 否则改装到 fallback。
#[tokio::test]
async fn update_targets_raw_npm_global_install_when_its_bin_is_off_path() {
    struct StubProbe {
        prefix: Option<PathBuf>,
    }

    #[async_trait::async_trait]
    impl NpmGlobalPrefixProbe for StubProbe {
        async fn npm_global_prefix(&self) -> Option<PathBuf> {
            self.prefix.clone()
        }
    }

    let root = tempdir().expect("root");
    // 唯一已有的安装：正好在 `npm prefix -g`，但它的 bin 不在 PATH 上。
    let npm_global = root.path().join("npm-global");
    std::fs::create_dir_all(npm_global.join("bin")).expect("npm global bin");
    std::fs::create_dir_all(
        npm_global
            .join("lib")
            .join("node_modules")
            .join("@openai")
            .join("codex"),
    )
    .expect("package dir");
    // fallback 与它不同，且里面**没有**该包。
    let fallback = root.path().join("fallback");

    let mut execution = execution(
        root.path(),
        ScriptedRunner::new(vec![success("added 1 package")], Duration::ZERO),
        SpyProber::new(Some("1.0.0")),
    );
    execution.path_value = Some(root.path().join("empty-bin").to_string_lossy().into_owned());
    execution.npm_prefix = fallback.clone();
    execution.npm_prefix_probe = Some(Arc::new(StubProbe {
        prefix: Some(npm_global.clone()),
    }));

    let row = row_for_wire("codex").expect("codex row");
    let resolved = execution
        .resolve_effective_prefix(row, execution.path_value.as_deref())
        .await;

    assert_eq!(
        resolved, npm_global,
        "已安装扫描必须用**未过滤**的 raw npm 前缀：不得把更新改装到 fallback"
    );
}

/// codegraph 的「真实命令」验证必须**不要求已建索引**、且确实走平台包
/// （实测 1.6.2：未初始化目录里 `files --json` exit 1「CodeGraph not
/// initialized」；`status --json` exit 0；平台包缺失时两者都 exit 1）。
/// `--no-metadata` 是残留旗标（owner 已裁决不用）。
#[test]
fn codegraph_real_command_does_not_require_an_index() {
    let argv = post_install_verification_argv("codegraph");
    assert_eq!(
        argv,
        vec!["status".to_string(), "--json".to_string()],
        "codegraph 装后验证命令必须不依赖已建索引"
    );
    assert!(
        !argv.iter().any(|argument| argument == "files"),
        "`files` 在未初始化目录 exit 1 ⇒ 新装的健康 codegraph 会被判失败: {argv:?}"
    );
    assert!(
        !argv.iter().any(|argument| argument == "--no-metadata"),
        "残留的 `--no-metadata` 旗标必须去掉: {argv:?}"
    );
}

/// Kimi 的「更新前版本」必须从**同一来源**取得（REQ-PCI-10 F10）：脚本
/// exit 0 但版本没变 ⇒ 作业必须 failed 并明确报「命令仍是旧版」，不得静默
/// 判成功（旧实现里 `before_version` 只在 npm 分支读取，Kimi 恒为 None）。
#[tokio::test]
async fn kimi_update_reports_still_old_version_when_script_keeps_same_version() {
    let root = tempdir().expect("root");
    let home = root.path().join("home");
    let kimi_bin = home.join(".kimi-code").join("bin");
    std::fs::create_dir_all(&kimi_bin).expect("kimi bin dir");
    std::fs::write(kimi_bin.join("kimi"), b"#!/bin/sh\n").expect("kimi stub");

    let install_runner = ScriptedRunner::new(
        vec![success("Installed to /tmp/elsewhere/kimi")],
        Duration::ZERO,
    );
    let mut execution = execution(
        root.path(),
        install_runner,
        SpyProber::with_before_version(Some("1.2.3"), Some("1.2.3")),
    );
    execution.home_dir = Some(home);

    let job = execution
        .store
        .create("kimi_code", FixedClock.now())
        .expect("create");
    execution
        .run(&job.job_id, InstallTarget::Provider(ProviderName::KimiCode))
        .await;

    let done = execution
        .store
        .get(&job.job_id)
        .expect("io")
        .expect("job exists");
    assert!(
        done.outcome.is_none(),
        "Kimi 同版本不得判成功（REQ-PCI-10）: {:?}",
        done.outcome
    );
    let failure = done
        .failure
        .expect("Kimi 同版本必须明确报「命令仍是旧版」");
    assert!(
        failure.message.contains("命令仍是旧版"),
        "错误文案必须明确报「命令仍是旧版」: {}",
        failure.message
    );
    assert!(
        failure.message.contains("1.2.3"),
        "错误文案必须点名当前版本: {}",
        failure.message
    );
    crate::cross_cutting::path_overlay::clear();
}

/// Kimi 真正升级成功（版本变了）⇒ completed（不得被同版本判定误伤）。
#[tokio::test]
async fn kimi_update_completes_when_script_upgrades_version() {
    let root = tempdir().expect("root");
    let home = root.path().join("home");
    let kimi_bin = home.join(".kimi-code").join("bin");
    std::fs::create_dir_all(&kimi_bin).expect("kimi bin dir");
    std::fs::write(kimi_bin.join("kimi"), b"#!/bin/sh\n").expect("kimi stub");

    let install_runner = ScriptedRunner::new(
        vec![success("Installed to /tmp/elsewhere/kimi")],
        Duration::ZERO,
    );
    let mut execution = execution(
        root.path(),
        install_runner,
        SpyProber::with_before_version(Some("1.3.0"), Some("1.2.3")),
    );
    execution.home_dir = Some(home);

    let job = execution
        .store
        .create("kimi_code", FixedClock.now())
        .expect("create");
    execution
        .run(&job.job_id, InstallTarget::Provider(ProviderName::KimiCode))
        .await;

    let done = execution
        .store
        .get(&job.job_id)
        .expect("io")
        .expect("job exists");
    let outcome = done.outcome.expect("Kimi 升级成功必须 completed");
    assert_eq!(outcome.installed_version.as_deref(), Some("1.3.0"));
    assert!(done.failure.is_none(), "升级成功不得落失败载荷: {:?}", done.failure);
    crate::cross_cutting::path_overlay::clear();
}
