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

/// S-B：codegraph 的**真实命令**验证（`status --json`）在 3.1 GB 级索引上是
/// 读写打开（会触发上游 WAL 自愈），耗时可能远超 5s ⇒ 必须**单独**放宽
/// 时限；否则健康安装被误判「真实命令失败」→ 无谓 `--include=optional`
/// 重试 → failed。其它 provider 的真实命令探测口径不得被改动。
#[tokio::test]
async fn codegraph_real_command_probe_has_wider_timeout_than_providers() {
    let root = tempdir().expect("root");
    let runner = Arc::new(AlwaysFailingRunner::default());
    let prober = health_prober(root.path(), runner.clone());
    let _ = prober
        .refresh_after_install(InstallTarget::CodeGraph, None)
        .await
        .expect("probe");
    let codegraph_verify = runner
        .requests()
        .into_iter()
        .find(|request| request.argv == vec!["status".to_string(), "--json".to_string()])
        .expect("codegraph 必须跑 `status --json` 真实命令");
    assert!(
        codegraph_verify.timeout >= Duration::from_secs(30),
        "codegraph 真实命令探针必须单独放宽到 30s 量级（3.1 GB 索引上 status 可能 >5s）: {:?}",
        codegraph_verify.timeout
    );

    let provider_runner = Arc::new(AlwaysFailingRunner::default());
    let provider_prober = health_prober(root.path(), provider_runner.clone());
    let _ = provider_prober
        .refresh_after_install(InstallTarget::Provider(ProviderName::Codex), None)
        .await
        .expect("probe");
    let provider_verify = provider_runner
        .requests()
        .into_iter()
        .find(|request| request.argv == vec!["--help".to_string()])
        .expect("provider 必须跑 `--help` 真实命令");
    assert_eq!(
        provider_verify.timeout,
        Duration::from_secs(1),
        "其它 provider 的真实命令探测超时口径不得被改动"
    );
}

/// 更新前后版本**必须同源同口径**（REQ-PCI-10 复审 R10）：装前
/// （`version_before_update`）与装后（provider health 快照）都必须走
/// `version_token_from_output`（首条非空行 → 版本 token）。若装前不解析
/// token，则「`kimi 2.6.1` vs `2.6.1`」**永不相等** ⇒ 脚本 exit 0 但版本没变
/// 时仍判 completed。
///
/// 本用例**不注入 `SpyProber.before_version`**：before / after 都走生产装配的
/// `HealthRefreshProber`（真实取数路径），断言的是生产口径而不是调用点；
/// 该 runner 按「安装脚本是否已跑过」切换 `--version` 输出。
struct InstallAwareVersionRunner {
    before_output: String,
    after_output: String,
    installed: AtomicUsize,
}

impl InstallAwareVersionRunner {
    fn new(before_output: &str, after_output: &str) -> Arc<Self> {
        Arc::new(Self {
            before_output: before_output.to_string(),
            after_output: after_output.to_string(),
            installed: AtomicUsize::new(0),
        })
    }
}

#[async_trait::async_trait]
impl BoundedCommandRunner for InstallAwareVersionRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        let installing = request.executable == "bash";
        let argv = request.argv.clone();
        // 安装脚本（`bash -c 'curl … | bash'`）：装完 ⇒ `--version` 切到新输出。
        if installing {
            self.installed.store(1, Ordering::SeqCst);
            return success("Installed to /tmp/elsewhere/kimi");
        }
        if argv == vec!["--version".to_string()] {
            let output = if self.installed.load(Ordering::SeqCst) == 1 {
                &self.after_output
            } else {
                &self.before_output
            };
            return success(output);
        }
        // 真实命令验证（`--help`）：跑得起来。
        success("usage: kimi")
    }
}

/// 生产口径：Kimi 脚本 exit 0 但版本没变 ⇒ 作业 failed 且报「命令仍是旧版」。
#[tokio::test]
async fn kimi_update_production_probe_reports_still_old_version() {
    let root = tempdir().expect("root");
    let home = root.path().join("home");
    let kimi_bin = home.join(".kimi-code").join("bin");
    std::fs::create_dir_all(&kimi_bin).expect("kimi bin dir");
    std::fs::write(kimi_bin.join("kimi"), b"#!/bin/sh\n").expect("kimi stub");

    let runner = InstallAwareVersionRunner::new("kimi 2.6.1\n", "kimi 2.6.1\n");
    let health = ProviderHealthService::with_dependencies(
        AriaStatePaths::from_workspace_root(root.path()),
        runner.clone(),
        Arc::new(SystemProviderHealthClock),
        Duration::from_secs(1),
        4096,
    );
    let mut execution = execution(
        root.path(),
        runner.clone(),
        Arc::new(HealthRefreshProber::new(
            Arc::new(health),
            runner.clone(),
            root.path().to_path_buf(),
            Duration::from_secs(1),
        )),
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
        "生产口径下前后版本相同时不得判成功（R10/REQ-PCI-10）: {:?}",
        done.outcome
    );
    let failure = done
        .failure
        .expect("同版本必须落 failed 载荷（生产取数口径）");
    assert!(
        failure.message.contains("命令仍是旧版"),
        "必须明确报「命令仍是旧版」: {}",
        failure.message
    );
    crate::cross_cutting::path_overlay::clear();
}

/// 生产口径：脚本确实升级了版本 ⇒ completed，且版本取解析后的 token。
#[tokio::test]
async fn kimi_update_production_probe_completes_when_version_differs() {
    let root = tempdir().expect("root");
    let home = root.path().join("home");
    let kimi_bin = home.join(".kimi-code").join("bin");
    std::fs::create_dir_all(&kimi_bin).expect("kimi bin dir");
    std::fs::write(kimi_bin.join("kimi"), b"#!/bin/sh\n").expect("kimi stub");

    let runner = InstallAwareVersionRunner::new("kimi 2.6.1\n", "kimi 2.7.0\n");
    let health = ProviderHealthService::with_dependencies(
        AriaStatePaths::from_workspace_root(root.path()),
        runner.clone(),
        Arc::new(SystemProviderHealthClock),
        Duration::from_secs(1),
        4096,
    );
    let mut execution = execution(
        root.path(),
        runner.clone(),
        Arc::new(HealthRefreshProber::new(
            Arc::new(health),
            runner.clone(),
            root.path().to_path_buf(),
            Duration::from_secs(1),
        )),
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
    let outcome = done.outcome.expect("版本确实升级 ⇒ completed");
    assert_eq!(
        outcome.installed_version.as_deref(),
        Some("2.7.0"),
        "装后版本必须是解析后的 token（与 health 同口径）"
    );
    assert!(
        done.failure.is_none(),
        "升级成功不得落失败载荷: {:?}",
        done.failure
    );
    crate::cross_cutting::path_overlay::clear();
}

// ---- 可诊断性（P0）：决策链 / 更新 / 失败分类的结构化事实 ----
// 日志只是这些事实的消费者：用例断言事实，而不是日志文本。

/// 四个候选（visible / 已安装位置反推 / npm 全局 / 兜底）都必须给出
/// `chosen` 与 `reason`，且全局只有一个被选中。
#[test]
fn prefix_decision_reports_every_candidate_with_reason() {
    let decision = decide_effective_prefix(&PrefixDecisionInput {
        visible: Some(PathBuf::from("/Users/dev/.npm-global")),
        visible_writable: true,
        installed: Some(PathBuf::from("/Users/dev/.npm-global")),
        npm_global: Some(PathBuf::from("/opt/homebrew")),
        npm_global_writable: true,
        npm_global_bin_on_path: false,
        fallback: PathBuf::from("/Users/dev/fallback-prefix"),
    });

    assert_eq!(
        decision.chosen_prefix,
        PathBuf::from("/Users/dev/.npm-global"),
        "PATH 上生效且可写的前缀必须优先（更新装回用户手上那份）"
    );
    assert_eq!(decision.candidates.len(), 4, "四个候选都要出现");
    let candidate = |source: PrefixCandidateSource| {
        decision
            .candidates
            .iter()
            .find(|candidate| candidate.source == source)
            .expect("candidate present")
    };
    assert_eq!(
        candidate(PrefixCandidateSource::Visible).reason,
        PrefixCandidateReason::Hit
    );
    assert!(candidate(PrefixCandidateSource::Visible).chosen);
    assert_eq!(
        candidate(PrefixCandidateSource::InstalledLocation).reason,
        PrefixCandidateReason::Superseded,
        "前序候选已命中 ⇒ 已安装反推候选不得再被选中"
    );
    assert_eq!(
        candidate(PrefixCandidateSource::NpmGlobal).reason,
        PrefixCandidateReason::BinNotOnPath,
        "npm 全局前缀的 bin 不在 PATH 上 ⇒ 排除原因必须点名"
    );
    assert_eq!(
        candidate(PrefixCandidateSource::Fallback).reason,
        PrefixCandidateReason::Superseded
    );
    assert_eq!(
        decision
            .candidates
            .iter()
            .filter(|candidate| candidate.chosen)
            .count(),
        1,
        "只能有一个候选被选中"
    );
}

/// visible 不可写 ⇒ 落到「已安装位置反推」；候选原因如实标注不可写。
#[test]
fn prefix_decision_reports_not_writable_visible_prefix() {
    let decision = decide_effective_prefix(&PrefixDecisionInput {
        visible: Some(PathBuf::from("/usr/local")),
        visible_writable: false,
        installed: Some(PathBuf::from("/Users/dev/.npm-global")),
        npm_global: None,
        npm_global_writable: false,
        npm_global_bin_on_path: false,
        fallback: PathBuf::from("/Users/dev/fallback-prefix"),
    });

    assert_eq!(
        decision.chosen_prefix,
        PathBuf::from("/Users/dev/.npm-global"),
        "visible 不可写 ⇒ 用已安装位置反推的前缀"
    );
    let visible = decision
        .candidates
        .iter()
        .find(|candidate| candidate.source == PrefixCandidateSource::Visible)
        .expect("visible candidate");
    assert_eq!(visible.reason, PrefixCandidateReason::NotWritable);
    assert!(!visible.chosen);
    let npm_global = decision
        .candidates
        .iter()
        .find(|candidate| candidate.source == PrefixCandidateSource::NpmGlobal)
        .expect("npm global candidate");
    assert_eq!(npm_global.reason, PrefixCandidateReason::NotFound);
    assert_eq!(npm_global.prefix, None);
}

/// 全新机器（无 visible / 无已安装 / npm 全局不可写）⇒ 兜底前缀，且
/// 每个候选都给出「不存在 / 不可写」原因。
#[test]
fn prefix_decision_falls_back_and_explains_every_exclusion() {
    let decision = decide_effective_prefix(&PrefixDecisionInput {
        visible: None,
        visible_writable: false,
        installed: None,
        npm_global: Some(PathBuf::from("/usr/local")),
        npm_global_writable: false,
        npm_global_bin_on_path: true,
        fallback: PathBuf::from("/Users/dev/.npm-global"),
    });

    assert_eq!(
        decision.chosen_prefix,
        PathBuf::from("/Users/dev/.npm-global"),
        "没有可用前缀 ⇒ 兜底"
    );
    assert!(decision.candidates.iter().all(|candidate| {
        candidate.chosen == (candidate.source == PrefixCandidateSource::Fallback)
    }));
    let reasons: Vec<&str> = decision
        .candidates
        .iter()
        .map(|candidate| candidate.reason.as_str())
        .collect();
    assert_eq!(
        reasons,
        vec!["not_found", "not_found", "not_writable", "hit"],
        "每个候选的排除原因都要能 grep: {reasons:?}"
    );
    let summary = decision.summary();
    for expected in [
        "visible=-",
        "installed_location=-",
        "npm_global=/usr/local",
        "reason=not_writable",
        "fallback=/Users/dev/.npm-global(chosen=true,reason=hit)",
    ] {
        assert!(
            summary.contains(expected),
            "决策链摘要必须可 grep（缺少 {expected}）：{summary}"
        );
    }
}

/// 更新前后版本比较（「仍是旧版」判定）的纯事实。
#[test]
fn update_version_facts_flags_still_old_version() {
    let same = update_version_facts(Some("1.2.3"), Some("1.2.3"));
    assert!(same.still_old, "前后版本相同 ⇒ 仍是旧版");
    assert_eq!(same.before.as_deref(), Some("1.2.3"));
    assert_eq!(same.after.as_deref(), Some("1.2.3"));

    let changed = update_version_facts(Some("1.2.3"), Some("1.2.4"));
    assert!(!changed.still_old, "版本真的变了 ⇒ 不是旧版");

    let unknown = update_version_facts(Some("1.2.3"), None);
    assert!(
        !unknown.still_old,
        "读不到装后版本不得判「仍是旧版」（不得静默成功，也不得假红灯）"
    );
    assert!(!update_version_facts(None, Some("1.2.3")).still_old);
}

/// 装后失败三语义的稳定分类 code（日志字段 `failure_class`）。
#[test]
fn post_install_failure_class_names_the_three_semantics() {
    assert_eq!(
        post_install_failure_class(false, false).as_str(),
        "not_installed"
    );
    assert_eq!(
        post_install_failure_class(true, false).as_str(),
        "missing_platform_package"
    );
    assert_eq!(
        post_install_failure_class(true, true).as_str(),
        "not_on_path"
    );
}
