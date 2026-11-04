//! 安装执行器（REQ-PCI-03/05 / tasks 11.2、11.6）。
//!
//! - **一个包一次安装**：API 只接受单个 `InstallTarget`，无批量入口
//!   （批量由块3 编排层循环调用）。
//! - npm 形态固定：`install -g --prefix <prefix> --omit=optional
//!   --no-audit --no-fund <pkg>[@version]`，环境带 `npm_config_registry`
//!   （默认阿里镜像，`NPM_REGISTRY` 可覆盖）；`<prefix>` 避开 sudo。
//! - Kimi 走官方脚本（`curl -fsSL … | bash`）。
//! - 成功后经 `PostInstallProber` 重新探测（`provider_health.refresh`）；
//!   **安装 ≠ 核验**：不写 capability、不触发核验探针。
//! - 失败经 `failure_classify` 分类落 job。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
};
use crate::cross_cutting::provider_health::ProviderHealthService;

use super::InstallTarget;
use super::failure_classify::{
    InstallFailureKind, InstallFailureSignals, InstallJobFailure, classify_install_failure,
};
use super::job_store::{InstallJobOutcome, InstallJobStore};
use super::source_table::{
    InstallSourceRow, manual_install_command, npm_prefix, npm_registry, row_for_wire,
};

/// 安装命令默认超时：npm 全局安装可能要几分钟（设计决策 9 未验证项 4）。
pub const DEFAULT_INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
/// 安装输出上限（stdout/stderr 各自）。
pub const INSTALL_OUTPUT_LIMIT: usize = 64 * 1024;

/// 宿主环境透传基线（计划覆盖层叠在其上；与 `provider_health` 的
/// probe 环境构造同口径）。
fn host_environment() -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    if let Some(path) = std::env::var_os("PATH") {
        environment.insert("PATH".to_string(), path.to_string_lossy().into_owned());
    }
    environment
}

/// 一次安装的完整命令计划（纯数据，可单测）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallCommandPlan {
    pub executable: String,
    pub argv: Vec<String>,
    /// 环境覆盖层（叠加在宿主环境之上）：npm 带 `npm_config_registry`。
    pub environment: BTreeMap<String, String>,
    pub manual_command: String,
}

/// npm 安装计划（REQ-PCI-03）：一个包一次安装，形态固定。
pub fn npm_install_plan(
    row: &InstallSourceRow,
    version: Option<&str>,
    registry: &str,
    prefix: &Path,
) -> InstallCommandPlan {
    let package = row.npm_package.expect("npm 行必有包名");
    let spec = match version {
        Some(version) if !version.trim().is_empty() => format!("{package}@{version}"),
        _ => format!("{package}@latest"),
    };
    let mut environment = BTreeMap::new();
    environment.insert("npm_config_registry".to_string(), registry.to_string());
    InstallCommandPlan {
        executable: "npm".to_string(),
        argv: vec![
            "install".to_string(),
            "-g".to_string(),
            "--prefix".to_string(),
            prefix.to_string_lossy().into_owned(),
            "--omit=optional".to_string(),
            "--no-audit".to_string(),
            "--no-fund".to_string(),
            spec,
        ],
        environment,
        manual_command: manual_install_command(row, registry),
    }
}

/// Kimi 官方脚本安装计划（非 npm；不接受版本锁定）。
pub fn kimi_install_plan(row: &InstallSourceRow, registry: &str) -> InstallCommandPlan {
    let script = row.install_script.expect("Kimi 行必有官方脚本地址");
    InstallCommandPlan {
        executable: "bash".to_string(),
        argv: vec!["-c".to_string(), format!("curl -fsSL {script} | bash")],
        environment: BTreeMap::new(),
        manual_command: manual_install_command(row, registry),
    }
}

/// 按目标派发安装计划；脚本安装源（Kimi）不走 npm 形态。
pub fn install_plan_for(
    target: &InstallTarget,
    version: Option<&str>,
    registry: &str,
    prefix: &Path,
) -> Option<InstallCommandPlan> {
    let row = row_for_wire(target.wire_label())?;
    Some(match row.install_script {
        Some(_) => kimi_install_plan(row, registry),
        None => npm_install_plan(row, version, registry, prefix),
    })
}

/// 跑安装命令的执行器（包装 `BoundedCommandRunner`）。
pub struct ProviderInstallRunner {
    command_runner: Arc<dyn BoundedCommandRunner>,
    working_dir: PathBuf,
    timeout: Duration,
    output_limit: usize,
}

impl ProviderInstallRunner {
    pub fn new(
        command_runner: Arc<dyn BoundedCommandRunner>,
        working_dir: impl Into<PathBuf>,
        timeout: Duration,
    ) -> Self {
        Self {
            command_runner,
            working_dir: working_dir.into(),
            timeout,
            output_limit: INSTALL_OUTPUT_LIMIT,
        }
    }

    /// 计划 → 有界命令请求（宿主 PATH 透传 + 计划覆盖层）。
    pub fn request_for(&self, plan: &InstallCommandPlan) -> BoundedCommandRequest {
        let mut environment = host_environment();
        for (key, value) in &plan.environment {
            environment.insert(key.clone(), value.clone());
        }
        BoundedCommandRequest {
            executable: plan.executable.clone(),
            argv: plan.argv.clone(),
            working_dir: self.working_dir.clone(),
            timeout: self.timeout,
            cancellation: CancellationToken::new(),
            environment,
            stdout_limit: self.output_limit,
            stderr_limit: self.output_limit,
        }
    }

    pub async fn execute(
        &self,
        plan: &InstallCommandPlan,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        let request = self.request_for(plan);
        self.command_runner.run(request).await
    }
}

/// 装完重探 seam（REQ-PCI-05）：成功安装后重新探测该对象的版本与可用性。
#[async_trait::async_trait]
pub trait PostInstallProber: Send + Sync {
    async fn refresh_after_install(&self, target: InstallTarget) -> Result<Option<String>, String>;
}

/// 生产实现：provider → `provider_health.refresh` 后回填版本；
/// codegraph 无 health 条目，用同一 runner 跑一次 `--version`。
pub struct HealthRefreshProber {
    health: Arc<ProviderHealthService>,
    command_runner: Arc<dyn BoundedCommandRunner>,
    working_dir: PathBuf,
    timeout: Duration,
}

impl HealthRefreshProber {
    pub fn new(
        health: Arc<ProviderHealthService>,
        command_runner: Arc<dyn BoundedCommandRunner>,
        working_dir: impl Into<PathBuf>,
        timeout: Duration,
    ) -> Self {
        Self {
            health,
            command_runner,
            working_dir: working_dir.into(),
            timeout,
        }
    }
}

#[async_trait::async_trait]
impl PostInstallProber for HealthRefreshProber {
    async fn refresh_after_install(&self, target: InstallTarget) -> Result<Option<String>, String> {
        self.health
            .refresh(CancellationToken::new())
            .await
            .map_err(|error| error.to_string())?;
        match target {
            InstallTarget::Provider(provider) => Ok(self
                .health
                .latest_diagnostic()
                .entry(&provider)
                .and_then(|entry| entry.version.clone())),
            InstallTarget::CodeGraph => {
                let environment = host_environment();
                let request = BoundedCommandRequest {
                    executable: "codegraph".to_string(),
                    argv: vec!["--version".to_string()],
                    working_dir: self.working_dir.clone(),
                    timeout: self.timeout,
                    cancellation: CancellationToken::new(),
                    environment,
                    stdout_limit: 4096,
                    stderr_limit: 4096,
                };
                match self.command_runner.run(request).await {
                    Ok(result) if result.exit_code == Some(0) && !result.timed_out => Ok(result
                        .stdout
                        .lines()
                        .map(str::trim)
                        .find(|line| !line.is_empty())
                        .map(str::to_string)),
                    _ => Ok(None),
                }
            }
        }
    }
}

const PATH_SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

/// 命令是否出现在给定 PATH 变量的某个目录里。
pub fn executable_on_path(command: &str, path_value: Option<&str>) -> bool {
    path_value
        .map(|value| {
            value
                .split(PATH_SEPARATOR)
                .filter(|dir| !dir.is_empty())
                .any(|dir| Path::new(dir).join(command).exists())
        })
        .unwrap_or(false)
}

/// npm 装完命令不在 PATH 上时的提示（REQ-PCI-03：`--prefix` 前缀的代价
/// 由提示兜底）。
pub fn npm_path_hint(command: &str, prefix: &Path, path_value: Option<&str>) -> Option<String> {
    if executable_on_path(command, path_value) {
        return None;
    }
    Some(format!(
        "已安装到 {}；若 `{command}` 不在 PATH 上，请把 {}/bin 加入 PATH 后重试",
        prefix.display(),
        prefix.display()
    ))
}

/// 作业执行时钟 seam。
pub trait InstallClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// 系统时钟。
pub struct SystemInstallClock;

impl InstallClock for SystemInstallClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// durable 作业执行器：running（create 时已落盘）→ 执行安装 → 成功重探
/// → completed / 失败分类 → failed。
pub struct InstallJobExecution {
    pub store: InstallJobStore,
    pub runner: ProviderInstallRunner,
    pub prober: Arc<dyn PostInstallProber>,
    pub registry: String,
    pub npm_prefix: PathBuf,
    pub clock: Arc<dyn InstallClock>,
}

impl InstallJobExecution {
    /// 生产装配：镜像/前缀取环境解析值，时钟取系统时钟。
    pub fn production(
        store: InstallJobStore,
        command_runner: Arc<dyn BoundedCommandRunner>,
        working_dir: impl Into<PathBuf>,
        health: Arc<ProviderHealthService>,
    ) -> Self {
        let working_dir = working_dir.into();
        Self {
            store,
            runner: ProviderInstallRunner::new(
                command_runner.clone(),
                working_dir.clone(),
                DEFAULT_INSTALL_TIMEOUT,
            ),
            prober: Arc::new(HealthRefreshProber::new(
                health,
                command_runner,
                working_dir,
                Duration::from_secs(5),
            )),
            registry: npm_registry(),
            npm_prefix: npm_prefix(),
            clock: Arc::new(SystemInstallClock),
        }
    }

    /// 单个目标的安装作业主流程（无批量入口）：执行安装 → 成功重探 +
    /// 落 completed / 失败分类落 failed。**安装 ≠ 核验**：重探只走
    /// provider health，不写 capability、不触发核验探针。
    pub async fn run(&self, job_id: &str, target: InstallTarget) {
        let Some(row) = row_for_wire(target.wire_label()) else {
            let failure = InstallJobFailure {
                kind: InstallFailureKind::Other,
                message: format!("未知安装对象：{}", target.wire_label()),
                manual_command: String::new(),
            };
            let _ = self.store.fail(job_id, failure, self.clock.now());
            return;
        };
        let Some(plan) = install_plan_for(&target, None, &self.registry, &self.npm_prefix) else {
            // row_for_wire 已放行 ⇒ 这里必然可达，防御性兜底。
            return;
        };
        match self.runner.execute(&plan).await {
            Ok(result) if result.exit_code == Some(0) && !result.timed_out && !result.cancelled => {
                let detected = self
                    .prober
                    .refresh_after_install(target)
                    .await
                    .ok()
                    .flatten();
                let path_value = std::env::var("PATH").ok();
                let path_hint = match row.npm_package {
                    Some(_) => npm_path_hint(row.command, &self.npm_prefix, path_value.as_deref()),
                    None => None,
                };
                let outcome = InstallJobOutcome {
                    installed_version: detected,
                    path_hint,
                };
                let _ = self.store.complete(job_id, outcome, self.clock.now());
            }
            Ok(result) => {
                let signals = InstallFailureSignals {
                    stderr: result.stderr,
                    stdout: result.stdout,
                    timed_out: result.timed_out,
                    command_missing: None,
                };
                let failure = classify_install_failure(row, &signals, &self.registry);
                let _ = self.store.fail(job_id, failure, self.clock.now());
            }
            Err(BoundedCommandError::CommandMissing {
                executable,
                details,
            }) => {
                let signals = InstallFailureSignals {
                    stderr: details,
                    command_missing: Some(executable),
                    ..InstallFailureSignals::default()
                };
                let failure = classify_install_failure(row, &signals, &self.registry);
                let _ = self.store.fail(job_id, failure, self.clock.now());
            }
            Err(BoundedCommandError::Io { details }) => {
                let signals = InstallFailureSignals {
                    stderr: details,
                    ..InstallFailureSignals::default()
                };
                let failure = classify_install_failure(row, &signals, &self.registry);
                let _ = self.store.fail(job_id, failure, self.clock.now());
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
        delay: Duration,
    }

    impl ScriptedRunner {
        fn new(
            results: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
            delay: Duration,
        ) -> Arc<Self> {
            Arc::new(Self {
                results: Mutex::new(results.into()),
                delay,
            })
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for ScriptedRunner {
        async fn run(
            &self,
            _request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
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
        let command_on_path = executable_on_path("codex", std::env::var("PATH").ok().as_deref());
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

    /// PATH 上已有命令 → 不给 path_hint。
    #[test]
    fn runner_path_hint_only_when_command_missing_from_path() {
        // 受控 PATH ⇒ 提示文案（含 `<prefix>/bin`）在本机恒定可断言。
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
        assert!(npm_path_hint("my-cli-xyz", Path::new("/p"), bin.to_str()).is_none());
    }
}
