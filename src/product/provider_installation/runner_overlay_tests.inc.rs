use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::TimeZone;
use tempfile::tempdir;

use super::*;
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
}

impl SpyProber {
    fn new(version: Option<&str>) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            version: version.map(str::to_string),
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
    ) -> Result<Option<String>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.version.clone())
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
        path_value: None,
        npm_prefix_probe: None,
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
        first_path_dir_with_command("codex", std::env::var("PATH").ok().as_deref()).is_some();
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
/// 全局前缀（可写时）——那是 npm 默认会装的地方。
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
    std::fs::create_dir_all(&npm_global).expect("npm global dir");
    let install_runner = ScriptedRunner::new(vec![success("added 1 package")], Duration::ZERO);
    let mut execution = execution(
        root.path(),
        install_runner.clone(),
        SpyProber::new(Some("1.0.5")),
    );
    // PATH 上没有 codex（未安装）。
    execution.path_value = Some(root.path().join("empty-bin").to_string_lossy().into_owned());
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
        resolved,
        npm_global,
        "npm 全局前缀的 bin 不在 PATH 上 ⇒ 不得选它（装到 PATH 之外 = 装完探测仍失败）"
    );
    assert_eq!(resolved, fallback, "应落到兜底前缀");
}
