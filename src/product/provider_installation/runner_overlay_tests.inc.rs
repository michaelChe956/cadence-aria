use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::TimeZone;
use tempfile::tempdir;

use super::*;
use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::cross_cutting::provider_health::{ProviderHealthService, SystemProviderHealthClock};
use crate::product::models::ProviderName;

const REGISTRY: &str = "https://registry.npmmirror.com";

struct ScriptedRunner {
    results: Mutex<VecDeque<Result<BoundedCommandResult, BoundedCommandError>>>,
    requests: Mutex<Vec<BoundedCommandRequest>>,
    delay: Duration,
}

impl ScriptedRunner {
    fn new(
        results: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
        delay: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            results: Mutex::new(results.into()),
            requests: Mutex::new(Vec::new()),
            delay,
        })
    }

    fn requests(&self) -> Vec<BoundedCommandRequest> {
        self.requests.lock().expect("recorded requests").clone()
    }
}

#[async_trait::async_trait]
impl BoundedCommandRunner for ScriptedRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        self.requests
            .lock()
            .expect("recorded requests")
            .push(request);
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.results
            .lock()
            .expect("scripted results")
            .pop_front()
            .expect("scripted result")
    }
}

fn success(stdout: &str) -> Result<BoundedCommandResult, BoundedCommandError> {
    Ok(BoundedCommandResult {
        exit_code: Some(0),
        stdout: stdout.to_string(),
        stderr: String::new(),
        timed_out: false,
        cancelled: false,
        stdout_truncated: false,
        stderr_truncated: false,
        duration_ms: 1,
    })
}

fn failure(stderr: &str) -> Result<BoundedCommandResult, BoundedCommandError> {
    Ok(BoundedCommandResult {
        exit_code: Some(1),
        stdout: String::new(),
        stderr: stderr.to_string(),
        timed_out: false,
        cancelled: false,
        stdout_truncated: false,
        stderr_truncated: false,
        duration_ms: 1,
    })
}

struct SpyProber {
    calls: AtomicUsize,
    version: Option<String>,
    real_command_ok: bool,
    before_version: Option<String>,
}

impl SpyProber {
    fn new(version: Option<&str>) -> Arc<Self> {
        Self::with_real_command(version, true)
    }

    /// 真实命令**跑不起来**的装后探测（2.8③「装上但缺平台包」）。
    fn with_real_command(version: Option<&str>, real_command_ok: bool) -> Arc<Self> {
        Self::configured(version, real_command_ok, None)
    }

    /// 带更新前版本快照（Kimi 等脚本类：REQ-PCI-10 同源比较）。
    fn with_before_version(version: Option<&str>, before_version: Option<&str>) -> Arc<Self> {
        Self::configured(version, true, before_version)
    }

    fn configured(
        version: Option<&str>,
        real_command_ok: bool,
        before_version: Option<&str>,
    ) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            version: version.map(str::to_string),
            real_command_ok,
            before_version: before_version.map(str::to_string),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl PostInstallProber for SpyProber {
    async fn refresh_after_install(
        &self,
        _target: InstallTarget,
        _install_bin_dir: Option<&Path>,
    ) -> Result<PostInstallProbeOutcome, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(PostInstallProbeOutcome {
            installed_version: self.version.clone(),
            real_command_ok: self.real_command_ok,
        })
    }

    async fn version_before_update(&self, _target: InstallTarget) -> Option<String> {
        self.before_version.clone()
    }
}

struct FixedClock;

impl InstallClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 3, 9, 0, 0)
            .single()
            .expect("fixed timestamp")
    }
}

fn execution(
    root: &Path,
    install_runner: Arc<dyn BoundedCommandRunner>,
    prober: Arc<dyn PostInstallProber>,
) -> InstallJobExecution {
    InstallJobExecution {
        store: InstallJobStore::new(root.join("jobs")),
        runner: ProviderInstallRunner::new(
            install_runner,
            root.to_path_buf(),
            Duration::from_secs(5),
        ),
        prober,
        registry: REGISTRY.to_string(),
        npm_prefix: PathBuf::from("/home/dev/.npm-global"),
        // 默认隔离：不让宿主 PATH 上的**真实** CLI 参与前缀判定（否则用例
        // 结果随开发机装了什么而变，且会往 overlay 登记真实目录）。
        path_value: Some(root.join("empty-bin").to_string_lossy().into_owned()),
        npm_prefix_probe: None,
        home_dir: None,
        clock: Arc::new(FixedClock),
    }
}

/// 测试点 #4：npm argv 形态固定 + 单包（不出现第二个包名）。
#[test]
fn runner_npm_argv_carries_required_flags_and_single_package() {
    let row = row_for_wire("codex").expect("codex row");
    let plan = npm_install_plan(row, None, REGISTRY, Path::new("/home/dev/.npm-global"));
    assert_eq!(plan.executable, "npm");
    for flag in [
        "install",
        "-g",
        "--omit=optional",
        "--no-audit",
        "--no-fund",
    ] {
        assert!(
            plan.argv.contains(&flag.to_string()),
            "缺少 {flag}: {plan:?}"
        );
    }
    assert!(plan.argv.contains(&"--prefix".to_string()));
    let prefix_index = plan
        .argv
        .iter()
        .position(|arg| arg == "--prefix")
        .expect("prefix");
    assert_eq!(plan.argv[prefix_index + 1], "/home/dev/.npm-global");
    assert!(
        plan.argv
            .last()
            .is_some_and(|spec| spec == "@openai/codex@latest")
    );

    let joined = plan.argv.join(" ");
    for other in [
        "@anthropic-ai/claude-code",
        "@earendil-works/pi-coding-agent",
        "@colbymchenry/codegraph",
    ] {
        assert!(
            !joined.contains(other),
            "一个包一次安装：argv 不应含 {other}"
        );
    }

    // 版本锁定形态 <pkg>@<version>。
    let pinned = npm_install_plan(row, Some("1.2.3"), REGISTRY, Path::new("/p"));
    assert!(pinned.argv.contains(&"@openai/codex@1.2.3".to_string()));
}

/// Kimi 走官方脚本（不是 npm 形态）。
#[test]
fn runner_kimi_plan_runs_official_script() {
    let row = row_for_wire("kimi_code").expect("kimi row");
    let plan = kimi_install_plan(row, REGISTRY);
    assert_eq!(plan.executable, "bash");
    assert_eq!(
        plan.argv,
        vec![
            "-c".to_string(),
            "curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash".to_string(),
        ]
    );
    assert!(!plan.argv.join(" ").contains("npm install"));
}

/// 测试点 #5：`npm_config_registry` 默认阿里 + `NPM_REGISTRY` 覆盖。
#[test]
fn runner_npm_env_carries_registry_default_and_override() {
    struct NoopRunner;
    #[async_trait::async_trait]
    impl BoundedCommandRunner for NoopRunner {
        async fn run(
            &self,
            _request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            unreachable!("仅构造请求，不执行");
        }
    }

    let row = row_for_wire("codegraph").expect("codegraph row");
    let default_registry = super::super::source_table::resolve_npm_registry(None);
    assert_eq!(default_registry, "https://registry.npmmirror.com");
    let runner = ProviderInstallRunner::new(
        Arc::new(NoopRunner),
        PathBuf::from("/tmp"),
        Duration::from_secs(1),
    );

    let default_plan = npm_install_plan(row, None, &default_registry, Path::new("/p"));
    let default_request = runner.request_for(&default_plan);
    assert_eq!(
        default_request
            .environment
            .get("npm_config_registry")
            .map(String::as_str),
        Some("https://registry.npmmirror.com")
    );

    let overridden =
        super::super::source_table::resolve_npm_registry(Some("https://registry.npmjs.org"));
    let overridden_plan = npm_install_plan(row, None, &overridden, Path::new("/p"));
    let overridden_request = runner.request_for(&overridden_plan);
    assert_eq!(
        overridden_request
            .environment
            .get("npm_config_registry")
            .map(String::as_str),
        Some("https://registry.npmjs.org")
    );
    assert!(
        overridden_request.environment.contains_key("PATH"),
        "宿主 PATH 必须透传"
    );
}

/// 安装成功 → completed + 装完重探一次 + npm 前缀 PATH 提示。
#[tokio::test]
async fn runner_install_job_completes_with_refresh_and_path_hint() {
    let root = tempdir().expect("root");
    let prober = SpyProber::new(Some("1.0.5"));
    let install_runner = ScriptedRunner::new(vec![success("added 1 package")], Duration::ZERO);
    let execution = execution(root.path(), install_runner, prober.clone());
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
    assert_eq!(
        done.status,
        super::super::job_store::InstallJobStatus::Completed
    );
    let outcome = done.outcome.expect("outcome");
    assert_eq!(outcome.installed_version, Some("1.0.5".to_string()));
    // 前置条件「命令不在 PATH 上」是**宿主环境事实**（开发机可能已
    // 装好 codex）⇒ 断言按前提分流，与 `npm_path_hint` 同口径：
    // 不在 PATH ⇒ 必须给 path_hint；已在 PATH ⇒ 不给。提示文案含
    // `<prefix>/bin` 的那一支由下方 `npm_path_hint` 单测（受控
    // PATH）确定性覆盖。
    let command_on_path =
        first_path_dir_with_command("codex", execution.path_value.as_deref()).is_some();
    if command_on_path {
        assert!(
            outcome.path_hint.is_none(),
            "命令已在 PATH 上时不给 path_hint: {outcome:?}"
        );
    } else {
        assert!(
            outcome
                .path_hint
                .as_deref()
                .is_some_and(|hint| hint.contains("/bin")),
            "命令不在 PATH 上时必须给 path_hint: {outcome:?}"
        );
    }
    assert_eq!(prober.calls(), 1, "装完必须重新探测一次");
}

/// 安装失败 → failed + 分类（EACCES → 权限类），不触发重探。
#[tokio::test]
async fn runner_install_job_failure_is_classified_and_durable() {
    let root = tempdir().expect("root");
    let prober = SpyProber::new(Some("1.0.5"));
    let install_runner = ScriptedRunner::new(
        vec![failure("npm ERR! code EACCES\nnpm ERR! permission denied")],
        Duration::ZERO,
    );
    let execution = execution(root.path(), install_runner, prober.clone());
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
    assert_eq!(
        done.status,
        super::super::job_store::InstallJobStatus::Failed
    );
    let failure = done.failure.expect("failure");
    assert_eq!(
        failure.kind,
        super::super::failure_classify::InstallFailureKind::Permission
    );
    assert!(failure.manual_command.contains("--registry="));
    assert!(!failure.manual_command.starts_with("sudo"));
    assert_eq!(prober.calls(), 0, "失败安装不触发重探");
}

/// 2026-11-09 bug 修复：安装 / 更新必须落到 **PATH 上当前生效的 CLI
/// 所在前缀**（`<prefix>/bin/<cmd>`）。旧实现一律落 `~/.npm-global`
/// ⇒ 新版本装在 PATH 之外，用户手上的命令仍是旧版本（更新没效果）。
#[tokio::test]
async fn runner_installs_into_the_prefix_of_the_cli_on_path() {
    let root = tempdir().expect("root");
    let visible_prefix = root.path().join("installation");
    let bin = visible_prefix.join("bin");
    std::fs::create_dir_all(&bin).expect("bin dir");
    std::fs::write(bin.join("codex"), b"#!/bin/sh\n").expect("cli stub");
    let install_runner = ScriptedRunner::new(vec![success("added 1 package")], Duration::ZERO);
    let mut execution = execution(
        root.path(),
        install_runner.clone(),
        SpyProber::new(Some("1.0.5")),
    );
    // PATH 上生效的 codex 在 `visible_prefix/bin`。
    execution.path_value = Some(bin.to_string_lossy().into_owned());
    // 旧行为的固定前缀（可写）——新实现不得选它。
    execution.npm_prefix = root.path().join("fallback");
    std::fs::create_dir_all(&execution.npm_prefix).expect("fallback dir");

    let job = execution
        .store
        .create("codex", FixedClock.now())
        .expect("create");
    execution
        .run(&job.job_id, InstallTarget::Provider(ProviderName::Codex))
        .await;

    let requests = install_runner.requests();
    let install = requests.first().expect("install request");
    let prefix_index = install
        .argv
        .iter()
        .position(|arg| arg == "--prefix")
        .expect("--prefix");
    assert_eq!(
        install.argv[prefix_index + 1],
        visible_prefix.to_string_lossy().into_owned(),
        "安装必须落到 PATH 上当前生效的 CLI 所在前缀，否则更新不生效"
    );
}

/// 2026-11-09 bug 修复：CLI 未安装（PATH 上没有）⇒ 落到 npm 自己的
/// 全局前缀（可写 **且** 其 `bin` 在 PATH 上）——那是 npm 默认会装的地方。
#[tokio::test]
async fn runner_falls_back_to_npm_global_prefix_when_cli_absent() {
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
    let npm_global = root.path().join("npm-global");
    std::fs::create_dir_all(npm_global.join("bin")).expect("npm global dir");
    let install_runner = ScriptedRunner::new(vec![success("added 1 package")], Duration::ZERO);
    let mut execution = execution(
        root.path(),
        install_runner.clone(),
        SpyProber::new(Some("1.0.5")),
    );
    // PATH 上没有 codex（未安装），但 npm 全局前缀的 bin **在** PATH 上 ⇒
    // 判定链 ② 命中（装到 PATH 上才算真装上）。
    execution.path_value = Some(format!(
        "{}:{}",
        root.path().join("empty-bin").display(),
        npm_global.join("bin").display()
    ));
    execution.npm_prefix = root.path().join("fallback");
    std::fs::create_dir_all(&execution.npm_prefix).expect("fallback dir");
    execution.npm_prefix_probe = Some(Arc::new(StubProbe {
        prefix: Some(npm_global.clone()),
    }));

    let job = execution
        .store
        .create("codex", FixedClock.now())
        .expect("create");
    execution
        .run(&job.job_id, InstallTarget::Provider(ProviderName::Codex))
        .await;

    let requests = install_runner.requests();
    let install = requests.first().expect("install request");
    let prefix_index = install
        .argv
        .iter()
        .position(|arg| arg == "--prefix")
        .expect("--prefix");
    assert_eq!(
        install.argv[prefix_index + 1],
        npm_global.to_string_lossy().into_owned(),
        "CLI 未安装时必须落到 npm 自己的全局前缀"
    );
}

/// path_hint 口径（2026-11-09 bug 修复）：PATH 上生效的命令必须就是
/// 本次安装前缀里的那个，否则用户跑的仍是旧版本 ⇒ 必须提示换前缀。
#[test]
fn runner_path_hint_reflects_whether_the_visible_binary_is_the_installed_one() {
    // 命令完全不在 PATH 上 ⇒ 提示加 PATH。
    let hint = npm_path_hint(
        "definitely-not-on-path-cli-xyz",
        Path::new("/home/dev/.npm-global"),
        Some("/nonexistent-dir-xyz"),
    )
    .expect("命令不在 PATH 上 ⇒ 必须给 path_hint");
    assert!(
        hint.contains("/home/dev/.npm-global/bin"),
        "path_hint 必须点名 <prefix>/bin: {hint}"
    );

    let root = tempdir().expect("root");
    let bin = root.path().join("bin");
    std::fs::create_dir(&bin).expect("bin dir");
    let cli = bin.join("my-cli-xyz");
    std::fs::write(&cli, b"#!/bin/sh\n").expect("cli stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755))
            .expect("executable bit");
    }
    // PATH 上生效的命令就是安装前缀里的那个 ⇒ 不给提示。
    assert!(
        npm_path_hint("my-cli-xyz", root.path(), bin.to_str()).is_none(),
        "PATH 上生效的就是本次安装的那个 ⇒ 不给 path_hint"
    );
    // PATH 上生效的是别处的旧版本 ⇒ 必须提示把新前缀放到 PATH 前面。
    let hint = npm_path_hint(
        "my-cli-xyz",
        Path::new("/home/dev/.npm-global"),
        bin.to_str(),
    )
    .expect("PATH 上生效的不是本次安装的那个 ⇒ 必须给 path_hint");
    assert!(
        hint.contains("/home/dev/.npm-global/bin"),
        "path_hint 必须点名 <prefix>/bin: {hint}"
    );
    assert!(
        hint.contains("PATH 前面"),
        "path_hint 必须说明把新前缀放到 PATH 前面: {hint}"
    );
}

/// `npm prefix -g` 的 bin 目录**不在 PATH 上** ⇒ 不得选它：装到 PATH 之外
/// ⇒ 装完探测仍失败 ⇒ 界面仍显示「未安装」（用户点一次没效果）。
#[tokio::test]
async fn resolve_effective_prefix_skips_npm_global_when_bin_not_on_path() {
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
    let npm_global = root.path().join("npm-global");
    std::fs::create_dir_all(&npm_global).expect("npm global dir");
    let fallback = root.path().join("fallback");
    std::fs::create_dir_all(&fallback).expect("fallback dir");

    let mut execution = execution(
        root.path(),
        ScriptedRunner::new(vec![success("added 1 package")], Duration::ZERO),
        SpyProber::new(Some("1.0.0")),
    );
    // PATH 上没有该命令（未安装），且 npm 全局前缀的 bin 也不在 PATH 上。
    execution.path_value = Some(root.path().join("empty-bin").to_string_lossy().into_owned());
    execution.npm_prefix = fallback.clone();
    execution.npm_prefix_probe = Some(Arc::new(StubProbe {
        prefix: Some(npm_global.clone()),
    }));

    let row = row_for_wire("codex").expect("codex row");
    let resolved = execution
        .resolve_effective_prefix(row, execution.path_value.as_deref())
        .await;

    assert_ne!(
        resolved, npm_global,
        "npm 全局前缀的 bin 不在 PATH 上 ⇒ 不得选它（装到 PATH 之外 = 装完探测仍失败）"
    );
    assert_eq!(resolved, fallback, "应落到兜底前缀");
}

/// 装成功后必须把「安装产物的 bin 目录」登记进 PATH overlay ⇒ 平台自身的
/// 命令可见 PATH 立刻包含新装的 CLI（决策 D1/D2）。
// overlay 是进程级共享状态 ⇒ 必须持全局锁把「登记 + 断言」整段串行化
// （锁要跨 await 持有：先例 workspace_engine/tests/part_03/task_3_4_oracles.rs）。
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn install_registers_install_bin_dir_into_path_overlay() {
    // overlay 是进程级共享状态 ⇒ 必须持全局锁并在结束时清空。
    let _lock = crate::cross_cutting::path_overlay::overlay_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    crate::cross_cutting::path_overlay::clear();

    let root = tempdir().expect("root");
    let prefix = root.path().join("installation");
    let bin = prefix.join("bin");
    std::fs::create_dir_all(&bin).expect("bin dir");
    std::fs::write(bin.join("codex"), b"#!/bin/sh\n").expect("cli stub");

    let install_runner = ScriptedRunner::new(vec![success("added 1 package")], Duration::ZERO);
    let mut execution = execution(
        root.path(),
        install_runner.clone(),
        SpyProber::new(Some("1.0.5")),
    );
    execution.path_value = Some(root.path().join("empty-bin").to_string_lossy().into_owned());
    execution.npm_prefix = prefix.clone();

    let job = execution
        .store
        .create("codex", FixedClock.now())
        .expect("create");
    execution
        .run(&job.job_id, InstallTarget::Provider(ProviderName::Codex))
        .await;

    let registered = crate::cross_cutting::path_overlay::snapshot();
    assert!(
        registered.contains(&bin),
        "装完必须把安装产物的 bin 目录登记进 overlay: {registered:?}"
    );
    let merged = crate::cross_cutting::path_overlay::effective_path(Some("/usr/bin"));
    assert_eq!(
        std::env::split_paths(&merged).next(),
        Some(bin.clone()),
        "登记的目录必须置前（平台自身探测立即可见）: {merged}"
    );
    crate::cross_cutting::path_overlay::clear();
}

/// npm 计划必须带 `--allow-scripts=<pkg>`（npm ≥12 不带它**不执行**包的
/// install 脚本 —— `@anthropic-ai/claude-code` 有 `postinstall: node
/// install.cjs`；npm 10/11 把它当未知配置忽略 ⇒ 兼容），且安装子进程的
/// PATH **前插**目标前缀的 bin（magpie `updateEnv` 思路）。
#[test]
fn npm_plan_carries_allow_scripts_and_prefix_bin_on_path() {
    struct NoopRunner;
    #[async_trait::async_trait]
    impl BoundedCommandRunner for NoopRunner {
        async fn run(
            &self,
            _request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            unreachable!("仅构造请求，不执行");
        }
    }

    let row = row_for_wire("claude_code").expect("claude row");
    let prefix = Path::new("/home/dev/.npm-global");
    let plan = npm_install_plan(row, None, REGISTRY, prefix);
    assert!(
        plan.argv
            .contains(&"--allow-scripts=@anthropic-ai/claude-code".to_string()),
        "npm ≥12 需要 --allow-scripts=<pkg> 才会执行包的 install 脚本: {plan:?}"
    );

    let runner = ProviderInstallRunner::new(
        Arc::new(NoopRunner),
        PathBuf::from("/tmp"),
        Duration::from_secs(1),
    );
    let request = runner.request_for(&plan);
    let path = request.environment.get("PATH").expect("PATH");
    let entries: Vec<PathBuf> = std::env::split_paths(path).collect();
    assert_eq!(
        entries.first(),
        Some(&prefix.join("bin")),
        "安装子进程的 PATH 必须前插目标前缀的 bin: {path}"
    );
}

/// 装完探测失败（装上但跑不起来）⇒ 自动以 `--include=optional` 重试一次；
/// 仍失败 ⇒ 错误文案含**可复制**的修复命令（npm 会「留下缺 native 包的
/// 安装」却仍 exit 0 ⇒ 首次装不上平台包时要带 optional 重装）。
#[tokio::test]
async fn install_retries_once_with_include_optional_when_probe_fails() {
    let root = tempdir().expect("root");
    let install_runner = ScriptedRunner::new(
        vec![success("added 1 package"), success("added 1 package")],
        Duration::ZERO,
    );
    let mut execution = execution(root.path(), install_runner.clone(), SpyProber::new(None));
    execution.path_value = Some(root.path().join("empty-bin").to_string_lossy().into_owned());
    execution.npm_prefix = root.path().join("installation");

    let job = execution
        .store
        .create("codex", FixedClock.now())
        .expect("create");
    execution
        .run(&job.job_id, InstallTarget::Provider(ProviderName::Codex))
        .await;

    let requests = install_runner.requests();
    assert_eq!(requests.len(), 2, "探测失败必须自动重试一次: {requests:?}");
    assert!(
        requests[1].argv.contains(&"--include=optional".to_string()),
        "重试必须带 --include=optional: {:?}",
        requests[1].argv
    );
    assert!(
        !requests[1].argv.contains(&"--omit=optional".to_string()),
        "重试不得与 --omit=optional 互斥: {:?}",
        requests[1].argv
    );

    let done = execution
        .store
        .get(&job.job_id)
        .expect("io")
        .expect("job exists");
    let failure = done.failure.expect("仍失败必须有失败载荷");
    assert!(
        failure.message.contains("--include=optional") && failure.message.contains("--prefix"),
        "错误文案必须含可复制的修复命令: {}",
        failure.message
    );
    assert!(
        failure.manual_command.contains("--include=optional"),
        "手动命令必须含可复制的修复命令: {}",
        failure.manual_command
    );
}

/// 桩 runner：所有命令都失败（用于「版本只能来自 package.json」的场景）。
#[derive(Default)]
struct AlwaysFailingRunner {
    requests: Mutex<Vec<BoundedCommandRequest>>,
}

impl AlwaysFailingRunner {
    fn requests(&self) -> Vec<BoundedCommandRequest> {
        self.requests.lock().expect("recorded requests").clone()
    }
}

#[async_trait::async_trait]
impl BoundedCommandRunner for AlwaysFailingRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        self.requests
            .lock()
            .expect("recorded requests")
            .push(request);
        failure("not found")
    }
}

/// 装完探测用的 prober（health 服务另配桩 runner ⇒ 被断言的请求只来自
/// prober 自身）。
fn health_prober(root: &Path, runner: Arc<dyn BoundedCommandRunner>) -> HealthRefreshProber {
    let health = ProviderHealthService::with_dependencies(
        AriaStatePaths::from_workspace_root(root),
        Arc::new(AlwaysFailingRunner::default()),
        Arc::new(SystemProviderHealthClock),
        Duration::from_secs(1),
        4096,
    );
    HealthRefreshProber::new(
        Arc::new(health),
        runner,
        root.to_path_buf(),
        Duration::from_secs(1),
    )
}

/// 版本读取**优先**读 npm 包的 `package.json`（不执行进程、更快更稳）；
/// 读不到才跑 `--version`。
#[tokio::test]
async fn installed_version_prefers_npm_package_json() {
    let root = tempdir().expect("root");
    let prefix = root.path().join("installation");
    let package_dir = prefix
        .join("lib")
        .join("node_modules")
        .join("@openai")
        .join("codex");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    std::fs::write(
        package_dir.join("package.json"),
        br#"{"name":"@openai/codex","version":"9.9.9"}"#,
    )
    .expect("package.json");

    let runner = Arc::new(AlwaysFailingRunner::default());
    let prober = health_prober(root.path(), runner.clone());
    let bin_dir = prefix.join("bin");

    let outcome = prober
        .refresh_after_install(
            InstallTarget::Provider(ProviderName::Codex),
            Some(bin_dir.as_path()),
        )
        .await
        .expect("probe");
    assert_eq!(
        outcome.installed_version.as_deref(),
        Some("9.9.9"),
        "版本必须优先读 npm 包的 package.json（读不到才跑 --version）"
    );
}

/// 装后验证除 `--version` 外**必须跑一个真实命令**（实测 `codegraph
/// --version` 在平台 bundle 缺失时仍返回 1.6.2 / exit 0 ⇒ 不可作为唯一
/// 依据）。
#[tokio::test]
async fn post_install_probe_runs_a_real_command() {
    let root = tempdir().expect("root");
    let runner = Arc::new(AlwaysFailingRunner::default());
    let prober = health_prober(root.path(), runner.clone());

    let outcome = prober
        .refresh_after_install(InstallTarget::CodeGraph, None)
        .await
        .expect("probe");
    assert!(
        !outcome.real_command_ok,
        "桩 runner 全部失败 ⇒ 真实命令必然不通过"
    );
    let requests = runner.requests();
    assert!(
        requests
            .iter()
            .any(|request| request.argv != vec!["--version".to_string()]),
        "装后必须跑一个真实命令（不能只 --version）: {requests:?}"
    );
}

/// 桩 runner：裸命令失败、绝对路径命令成功（PATH 上找不到时的兜底探测）。
#[derive(Default)]
struct PathAwareRunner {
    requests: Mutex<Vec<BoundedCommandRequest>>,
}

impl PathAwareRunner {
    fn requests(&self) -> Vec<BoundedCommandRequest> {
        self.requests.lock().expect("recorded requests").clone()
    }
}

#[async_trait::async_trait]
impl BoundedCommandRunner for PathAwareRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        let executable = request.executable.clone();
        self.requests
            .lock()
            .expect("recorded requests")
            .push(request);
        if executable.contains('/') {
            success("1.2.3")
        } else {
            failure("not found")
        }
    }
}

/// PATH 找不到 CLI、`<bin>/<cmd> --version` 可执行 ⇒ `installed_version` 被
/// 记录（绝对路径探测**必须经 BoundedCommandRunner**，否则 PATH 覆盖层
/// 不生效）。
#[tokio::test]
async fn post_install_version_falls_back_to_absolute_path_probe() {
    let root = tempdir().expect("root");
    let bin = root.path().join("bin");
    std::fs::create_dir_all(&bin).expect("bin dir");

    let runner = Arc::new(PathAwareRunner::default());
    let prober = health_prober(root.path(), runner.clone());

    let outcome = prober
        .refresh_after_install(InstallTarget::CodeGraph, Some(bin.as_path()))
        .await
        .expect("probe");
    assert_eq!(
        outcome.installed_version.as_deref(),
        Some("1.2.3"),
        "PATH 找不到时绝对路径探测必须回填版本"
    );
    assert!(
        runner
            .requests()
            .iter()
            .any(|request| { request.executable == bin.join("codegraph").to_string_lossy() }),
        "绝对路径探测必须经 BoundedCommandRunner: {:?}",
        runner.requests()
    );
}

/// Kimi 行（官方脚本）安装结果必须带 `path_hint`，文案点名
/// `~/.kimi-code/bin`（官方脚本只改 shell rc ⇒ 运行中的平台进程 PATH 不变
/// ⇒ 探测必然 127 ⇒ 必须提示）。
// 同上：overlay 登记是进程级共享状态 ⇒ 持全局锁跨 await 串行化。
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn kimi_install_result_carries_path_hint() {
    // 该用例会触发 overlay 登记 ⇒ 必须持全局锁并在结束时清空，避免污染
    // 并行的 overlay 断言用例。
    let _lock = crate::cross_cutting::path_overlay::overlay_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    crate::cross_cutting::path_overlay::clear();

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
        install_runner.clone(),
        SpyProber::new(Some("1.2.3")),
    );
    execution.home_dir = Some(home.clone());

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
    let outcome = done.outcome.expect("outcome");
    let hint = outcome
        .path_hint
        .expect("Kimi 行必须给 path_hint（脚本只改 shell rc）");
    assert!(
        hint.contains(".kimi-code/bin"),
        "path_hint 必须点名 ~/.kimi-code/bin: {hint}"
    );
    crate::cross_cutting::path_overlay::clear();
}

/// 已安装但**不在 PATH 上** ⇒ 从已安装包的真实路径反推 npm 前缀（2.10②）：
/// 更新必须装回被更新对象所在的前缀，不得改装到别处（否则「更新装到 A、
/// 用户跑的还是 B」——2026-09-09 老 bug）。
#[tokio::test]
async fn update_targets_existing_install_that_is_not_on_path() {
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
    // npm 自己的全局前缀（bin 在 PATH 上，但**没有**装这个包）。
    let npm_global = root.path().join("npm-global");
    std::fs::create_dir_all(npm_global.join("bin")).expect("npm global bin");
    // 已有安装在兜底前缀里（其 bin **不在** PATH 上 ⇒ 属「已装但不在 PATH
    // 上」）。
    let existing = root.path().join("fallback");
    std::fs::create_dir_all(
        existing
            .join("lib")
            .join("node_modules")
            .join("@openai")
            .join("codex"),
    )
    .expect("package dir");
    let fallback = existing.clone();

    let mut execution = execution(
        root.path(),
        ScriptedRunner::new(vec![success("added 1 package")], Duration::ZERO),
        SpyProber::new(Some("1.0.0")),
    );
    execution.path_value = Some(format!(
        "{}:{}",
        root.path().join("empty-bin").display(),
        npm_global.join("bin").display()
    ));
    execution.npm_prefix = fallback.clone();
    execution.npm_prefix_probe = Some(Arc::new(StubProbe {
        prefix: Some(npm_global.clone()),
    }));

    let row = row_for_wire("codex").expect("codex row");
    let resolved = execution
        .resolve_effective_prefix(row, execution.path_value.as_deref())
        .await;

    assert_eq!(
        resolved, existing,
        "已安装但不在 PATH 上 ⇒ 必须反推回它所在的前缀，不得改装到别处"
    );
}

/// 更新后**版本未变** ⇒ 明确报「命令仍是旧版」，不得静默成功（2.10③；
/// magpie `still X after cmd; Y is out`）。
#[tokio::test]
async fn update_reports_still_old_version_when_version_unchanged() {
    let root = tempdir().expect("root");
    let prefix = root.path().join("installation");
    // 已安装 ⇒ 本次是「更新」而不是「安装」。
    let package_dir = prefix
        .join("lib")
        .join("node_modules")
        .join("@openai")
        .join("codex");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    std::fs::write(
        package_dir.join("package.json"),
        br#"{"name":"@openai/codex","version":"1.0.0"}"#,
    )
    .expect("package.json");
    std::fs::create_dir_all(prefix.join("bin")).expect("bin dir");

    let install_runner = ScriptedRunner::new(vec![success("added 1 package")], Duration::ZERO);
    let mut execution = execution(root.path(), install_runner, SpyProber::new(Some("1.0.0")));
    execution.path_value = Some(root.path().join("empty-bin").to_string_lossy().into_owned());
    execution.npm_prefix = prefix.clone();

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
    let failure = done
        .failure
        .expect("版本未变必须明确报出来（不得静默成功）");
    assert!(
        failure.message.contains("命令仍是旧版"),
        "错误文案必须明确报「命令仍是旧版」: {}",
        failure.message
    );
    assert!(
        failure.message.contains("1.0.0"),
        "错误文案必须点名当前版本: {}",
        failure.message
    );
}

// ---- 可诊断性（P0）：装后探测来源的结构化事实 ----

/// 桩 runner：只有 PATH 口径（裸命令）的 `--version` 成功，其余全失败。
#[derive(Default)]
struct VersionOnlyOnPathRunner {
    requests: Mutex<Vec<BoundedCommandRequest>>,
}

#[async_trait::async_trait]
impl BoundedCommandRunner for VersionOnlyOnPathRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        let bare = !request.executable.contains('/');
        let is_version = request.argv == vec!["--version".to_string()];
        self.requests
            .lock()
            .expect("recorded requests")
            .push(request);
        if bare && is_version {
            success("1.2.3")
        } else {
            failure("not found")
        }
    }
}

/// 版本来源必须是结构化事实：`package.json` 命中 ⇒ 不得再跑 `--version`。
#[tokio::test]
async fn post_install_probe_reports_package_json_version_source() {
    let root = tempdir().expect("root");
    let prefix = root.path().join("installation");
    let package_dir = prefix
        .join("lib")
        .join("node_modules")
        .join("@openai")
        .join("codex");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    std::fs::write(
        package_dir.join("package.json"),
        br#"{"name":"@openai/codex","version":"9.9.9"}"#,
    )
    .expect("package.json");

    let runner = Arc::new(AlwaysFailingRunner::default());
    let prober = health_prober(root.path(), runner.clone());
    let bin_dir = prefix.join("bin");

    let (result, facts) = prober
        .probe_with_facts(
            InstallTarget::Provider(ProviderName::Codex),
            Some(bin_dir.as_path()),
        )
        .await;

    let outcome = result.expect("probe");
    assert_eq!(outcome.installed_version.as_deref(), Some("9.9.9"));
    assert_eq!(
        facts.version_source,
        VersionProbeSource::PackageJson,
        "package.json 可读 ⇒ 版本来源必须是 from_package_json"
    );
    assert_eq!(
        facts.real_command_argv,
        vec!["--help".to_string()],
        "真实命令的 argv 必须如实上报（provider 是 --help）"
    );
    assert!(!facts.real_command_ok, "桩 runner 全失败 ⇒ 真实命令不通过");
    assert!(
        !facts.path_probe_ok,
        "PATH 口径真实命令失败（装到了 PATH 之外）必须如实上报"
    );
    assert!(
        !runner.requests().iter().any(|request| {
            request.argv == vec!["--version".to_string()]
        }),
        "package.json 命中后不得再执行 --version 探测"
    );
}

/// 无 `package.json`、PATH 口径 `--version` 成功 ⇒ 来源 `from_version_flag`。
#[tokio::test]
async fn post_install_probe_reports_version_flag_source() {
    let root = tempdir().expect("root");
    let runner = Arc::new(VersionOnlyOnPathRunner::default());
    let prober = health_prober(root.path(), runner.clone());

    let (result, facts) = prober
        .probe_with_facts(InstallTarget::Provider(ProviderName::Codex), None)
        .await;

    let outcome = result.expect("probe");
    assert_eq!(outcome.installed_version.as_deref(), Some("1.2.3"));
    assert_eq!(facts.version_source, VersionProbeSource::VersionFlag);
    assert!(
        facts.path_version_ok,
        "PATH 口径 --version 成功必须如实上报"
    );
}

/// PATH 口径 `--version` 失败、绝对路径兜底成功 ⇒ 来源 `absolute_path_probe`。
#[tokio::test]
async fn post_install_probe_reports_absolute_path_probe_source() {
    let root = tempdir().expect("root");
    let bin = root.path().join("bin");
    std::fs::create_dir_all(&bin).expect("bin dir");
    let runner = Arc::new(PathAwareRunner::default());
    let prober = health_prober(root.path(), runner.clone());

    let (result, facts) = prober
        .probe_with_facts(InstallTarget::CodeGraph, Some(bin.as_path()))
        .await;

    assert_eq!(
        result.expect("probe").installed_version.as_deref(),
        Some("1.2.3")
    );
    assert_eq!(facts.version_source, VersionProbeSource::AbsolutePathProbe);
    assert_eq!(
        facts.real_command_argv,
        vec!["status".to_string(), "--json".to_string()],
        "codegraph 的真实命令必须是 status --json"
    );
    assert!(
        !facts.path_version_ok,
        "PATH 口径 --version 失败（裸命令失败）必须如实上报"
    );
}
