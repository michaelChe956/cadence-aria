// 经 provider_installation.rs 的 tests mod `include!` 引入（large_file_guard
// 1200 行红线拆分，纯移动自原文件 tests mod，内容逐字不变）。
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::Json;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Response};
    use chrono::{TimeDelta, Utc};
    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::{
        InstallJobExecution, OffPathQuery, TEST_INSTALL_EXECUTIONS, TestInstallExecutionBuilder,
        get_install_job, get_install_targets, off_path_query_with, post_provider_expose,
        post_provider_install, resolve_install_action, resolve_off_path,
    };
    use crate::cross_cutting::aria_state_paths::AriaStatePaths;
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };
    use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
    use crate::cross_cutting::provider_health::{ProviderHealthService, SystemProviderHealthClock};
    use crate::web::runtime::WebRuntime;
    use crate::web::state::WebAppState;

    // ---- 测试基建（与 providers.rs 测试同款脚本 runner） ----

    /// 命令执行副作用钩子（模拟 npm 安装真的落盘）：type alias 收敛签名
    /// 复杂度（clippy::type_complexity；语义不变，不用 #[allow] 压制）。
    type OnRunHook = Arc<dyn Fn(&BoundedCommandRequest) + Send + Sync>;

    struct ScriptedRunner {
        results: Mutex<VecDeque<Result<BoundedCommandResult, BoundedCommandError>>>,
        /// 脚本原件：脚本用尽时**回放一轮**（见 `run` 的说明）。
        script: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
        delay: Duration,
        /// 可选副作用：命令执行时的宿主侧效果（模拟 npm **真的装上了**）。
        on_run: Option<OnRunHook>,
    }

    impl ScriptedRunner {
        fn new(
            results: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
            delay: Duration,
        ) -> Arc<Self> {
            Arc::new(Self {
                results: Mutex::new(VecDeque::from(results.clone())),
                script: results,
                delay,
                on_run: None,
            })
        }

        /// 带副作用的 runner：每次命令执行**前**触发 `on_run`（模拟安装落盘）。
        fn with_side_effect(
            results: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
            delay: Duration,
            on_run: OnRunHook,
        ) -> Arc<Self> {
            Arc::new(Self {
                results: Mutex::new(VecDeque::from(results.clone())),
                script: results,
                delay,
                on_run: Some(on_run),
            })
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for ScriptedRunner {
        async fn run(
            &self,
            request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            if let Some(on_run) = self.on_run.as_ref() {
                on_run(&request);
            }
            let mut results = self.results.lock().expect("scripted results");
            if results.is_empty() {
                // 脚本用尽 = 发生了**额外一轮**探测（CI 慢机上更常见：一次刷新
                // 赶在快照过期后）。回放脚本原件（探测顺序与结果不变）而不是
                // panic ——「不该发生的额外调用」由用例自身的断言把关，不该由
                // 夹具的 panic 表达（那会把慢机误报成功能缺陷）。
                results.extend(self.script.iter().cloned());
            }
            results.pop_front().expect("scripted result")
        }
    }

    fn success(version: &str) -> Result<BoundedCommandResult, BoundedCommandError> {
        Ok(BoundedCommandResult {
            exit_code: Some(0),
            stdout: format!("cli {version}"),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        })
    }

    fn missing(program: &str) -> Result<BoundedCommandResult, BoundedCommandError> {
        Err(BoundedCommandError::CommandMissing {
            executable: program.to_string(),
            details: "not found".to_string(),
        })
    }

    fn install_success() -> Result<BoundedCommandResult, BoundedCommandError> {
        Ok(BoundedCommandResult {
            exit_code: Some(0),
            stdout: "added 1 package in 3s".to_string(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        })
    }

    /// 4 个版本探测结果 → 一个 health 服务（claude/codex/pi/kimi 顺序）。
    fn health_service(
        root: &Path,
        results: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
    ) -> Arc<ProviderHealthService> {
        Arc::new(ProviderHealthService::with_dependencies(
            AriaStatePaths::from_workspace_root(root),
            ScriptedRunner::new(results, Duration::ZERO),
            Arc::new(SystemProviderHealthClock),
            Duration::from_secs(1),
            4096,
        ))
    }

    fn state(
        root: &Path,
        health: Arc<ProviderHealthService>,
        install_runner: Arc<dyn BoundedCommandRunner>,
    ) -> WebAppState {
        let gate = Arc::new(ProviderAvailabilityGate::new(health.clone()));
        let mut state =
            WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()))
                .with_provider_health(health, gate, install_runner);
        state.test_provider_enabled = false;
        state
    }

    /// 用例级**宿主隔离环境**：临时 npm 前缀（自带 stub `package.json`）+
    /// 空 PATH + 临时 HOME。
    ///
    /// 为什么需要：装后版本**优先读 npm 包的 `package.json`**（§五 2.8①），
    /// 而安装落点由**宿主** PATH / npm 全局前缀 / HOME 决定 —— 宿主真装了
    /// codex 时，作业会落到宿主前缀、版本直接读宿主真实包（开发机装了什么
    /// 就读到什么），用例断言随开发机状态漂移。这里把落点钉到临时前缀，
    /// 宿主 PATH / 已装包 / HOME 一律不进断言。
    #[derive(Clone)]
    struct IsolatedInstallEnv {
        /// 隔离的 npm 全局前缀（安装落点；`lib/node_modules/<pkg>/package.json`）。
        prefix: PathBuf,
        /// 空 bin 目录（PATH 上没有任何命令 ⇒ 宿主的 visible 前缀不参与）。
        empty_bin: PathBuf,
        /// 临时 HOME（脚本类安装源落点推断用）。
        home: PathBuf,
        /// 装**后**的版本（假安装真的写进 `package.json`）。
        new_version: String,
    }

    impl IsolatedInstallEnv {
        /// 在 `root` 下建隔离环境：装前版本 `old_version`，假安装后写入
        /// `new_version`。
        fn new(root: &Path, old_version: &str, new_version: &str) -> Self {
            let prefix = root.join("npm-prefix");
            let package_dir = prefix
                .join("lib")
                .join("node_modules")
                .join("@openai")
                .join("codex");
            std::fs::create_dir_all(&package_dir).expect("package dir");
            std::fs::write(
                package_dir.join("package.json"),
                format!(r#"{{"name":"@openai/codex","version":"{old_version}"}}"#),
            )
            .expect("package.json");
            let empty_bin = root.join("empty-bin");
            std::fs::create_dir_all(&empty_bin).expect("empty bin");
            let home = root.join("home");
            std::fs::create_dir_all(&home).expect("home");
            Self {
                prefix,
                empty_bin,
                home,
                new_version: new_version.to_string(),
            }
        }

        /// 装**后**的 `package.json`（装前版本 → 装后版本）。
        fn package_manifest(&self) -> PathBuf {
            self.prefix
                .join("lib")
                .join("node_modules")
                .join("@openai")
                .join("codex")
                .join("package.json")
        }

        /// 假安装的副作用：npm 装成功 ⇒ 新版本真的落盘（模拟真实安装）。
        fn install_side_effect(&self) -> OnRunHook {
            let env = self.clone();
            Arc::new(move |request: &BoundedCommandRequest| {
                // 只有真正的安装命令才落盘（`--version` / `--help` 探测不落盘）。
                let is_install = request.executable == "npm"
                    && request.argv.first().is_some_and(|arg| arg == "install");
                if !is_install {
                    return;
                }
                std::fs::write(
                    env.package_manifest(),
                    format!(
                        r#"{{"name":"@openai/codex","version":"{}"}}"#,
                        env.new_version
                    ),
                )
                .expect("write installed package.json");
            })
        }

        /// 隔离执行器装配器：临时前缀 + 空 PATH + 临时 HOME + 不探测 npm
        /// 全局前缀（宿主 `npm prefix -g` 一律不参与）。
        fn builder(&self) -> TestInstallExecutionBuilder {
            let prefix = self.prefix.clone();
            let empty_bin = self.empty_bin.clone();
            let home = self.home.clone();
            Arc::new(move |store, state| {
                let mut execution = InstallJobExecution::production(
                    store,
                    state.command_runner.clone(),
                    state.workspace_root.clone(),
                    state.provider_health.clone(),
                );
                execution.path_value = Some(empty_bin.to_string_lossy().into_owned());
                execution.npm_prefix = prefix.clone();
                execution.npm_prefix_probe = None;
                execution.home_dir = Some(home.clone());
                execution
            })
        }
    }

    /// 隔离执行器的登记凭证：测试结束（含 panic 展开）自动摘除登记。
    struct IsolatedInstallEnvGuard {
        workspace_root: PathBuf,
    }

    impl Drop for IsolatedInstallEnvGuard {
        fn drop(&mut self) {
            if let Ok(mut registry) = TEST_INSTALL_EXECUTIONS.lock() {
                registry.retain(|(root, _)| *root != self.workspace_root);
            }
        }
    }

    /// 把隔离执行器挂到该 workspace_root 上（按根路径命中 ⇒ 并行用例不串味）。
    fn isolate_install_env(
        env: &IsolatedInstallEnv,
        workspace_root: &Path,
    ) -> IsolatedInstallEnvGuard {
        let mut registry = TEST_INSTALL_EXECUTIONS.lock().expect("registry lock");
        registry.retain(|(root, _)| *root != workspace_root);
        registry.push((workspace_root.to_path_buf(), env.builder()));
        IsolatedInstallEnvGuard {
            workspace_root: workspace_root.to_path_buf(),
        }
    }

    /// 预置 24h durable 版本缓存，避免 handler 测试触网。
    fn seed_version_cache(root: &Path, entries: Value) {
        let cache_path = root.join(".aria/state/provider-installation/version-cache.json");
        std::fs::create_dir_all(cache_path.parent().expect("parent")).expect("create cache dir");
        std::fs::write(
            &cache_path,
            serde_json::to_vec(&json!({ "entries": entries })).unwrap(),
        )
        .expect("seed cache");
    }

    async fn response_json(response: Response) -> Value {
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        serde_json::from_slice(&body).expect("json body")
    }

    async fn poll_job_terminal(state: &WebAppState, job_id: &str) -> Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let response = get_install_job(
                State(state.clone()),
                axum::extract::Path(job_id.to_string()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let job = response_json(response).await;
            if job["status"] != json!("running") {
                return job;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "install job did not reach terminal state in time"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    // ---- resolve_install_action 规则 ----

    #[test]
    fn install_target_action_rules_follow_spec() {
        // 未安装 → install
        assert_eq!(
            resolve_install_action(false, false, None, Some("2.0.0")),
            "install"
        );
        // 已安装但探针失败 → verify（不显示安装）
        assert_eq!(
            resolve_install_action(true, false, Some("1.0.0"), Some("2.0.0")),
            "verify"
        );
        // 版本落后 → update
        assert_eq!(
            resolve_install_action(true, true, Some("1.0.0"), Some("2.0.0")),
            "update"
        );
        // 预发布本地 < 正式最新 → update
        assert_eq!(
            resolve_install_action(true, true, Some("1.5.0-rc.1"), Some("1.5.0")),
            "update"
        );
        // 版本最新 → none
        assert_eq!(
            resolve_install_action(true, true, Some("2.0.0"), Some("2.0.0")),
            "none"
        );
        // registry 不可知（离线 / 查询失败）→ none（已装且探过）
        assert_eq!(
            resolve_install_action(true, true, Some("2.0.0"), None),
            "none"
        );
    }

    // ---- off_path 判定（REQ-PCI-08） ----

    #[test]
    fn off_path_marks_installed_target_missing_from_login_shell_path() {
        let root = tempdir().expect("root");
        // 已安装落点：workspace 隔离的 npm 前缀 bin（不在登录 shell PATH 上）。
        let prefix_bin = root.path().join("npm-global").join("bin");
        std::fs::create_dir_all(&prefix_bin).expect("prefix bin");
        std::fs::write(prefix_bin.join("claude"), "#!/bin/sh\n").expect("claude bin");
        // 登录 shell PATH 上的真实目录（**也放一份 claude** ⇒ 「在 PATH 上
        // 的那条分支」真的被测到，而不是落入「未安装 ⇒ false」）。
        let login_dir = root.path().join("login-bin");
        std::fs::create_dir_all(&login_dir).expect("login bin");
        std::fs::write(login_dir.join("claude"), "#!/bin/sh\n").expect("login claude bin");
        let query = OffPathQuery {
            install_search_dirs: vec![prefix_bin.clone()],
            login_path_dirs: vec![login_dir.clone()],
        };

        // 平台能找到、但所在目录不在登录 shell PATH 上 ⇒ true。
        assert!(resolve_off_path("claude", &query));
        // 二进制就落在登录 shell PATH 的目录里 ⇒ false。
        assert!(!resolve_off_path(
            "claude",
            &OffPathQuery {
                install_search_dirs: vec![login_dir.clone()],
                login_path_dirs: vec![login_dir.clone()],
            }
        ));
        // 未安装（搜索目录里找不到命令）⇒ false。
        assert!(!resolve_off_path("codex", &query));
        // 登录 shell PATH 快照不可用（启动 ≤2s 或探测失败）⇒ 不下结论（false）。
        assert!(!resolve_off_path(
            "claude",
            &OffPathQuery {
                install_search_dirs: vec![prefix_bin],
                login_path_dirs: Vec::new(),
            }
        ));
    }

    /// F8：off_path 的「平台能找到」口径必须含**服务进程 PATH**
    /// （`effective_path` 合并后的真实生效序）—— 否则「服务自己能跑、用户
    /// 终端 PATH 没有」会被误判成「未安装 ⇒ false」（REQ-PCI-08）。
    #[test]
    fn off_path_counts_service_process_path_as_platform_visible() {
        const CLI: &str = "aria-off-path-fixture-cli";
        let root = tempdir().expect("root");
        let service_bin = root.path().join("service-bin");
        std::fs::create_dir_all(&service_bin).expect("service bin");
        std::fs::write(service_bin.join(CLI), "#!/bin/sh\n").expect("service cli");
        let login_bin = root.path().join("login-bin");
        std::fs::create_dir_all(&login_bin).expect("login bin");
        let home = root.path().join("home");

        let service_path = service_bin.display().to_string();
        let login_path = login_bin.display().to_string();

        // ① 服务进程 PATH 有、登录 shell PATH 没有 ⇒ 平台能找到但不在终端
        //    PATH 上 ⇒ true。
        let query = off_path_query_with(&service_path, Some(login_path.as_str()), Some(home.as_path()));
        assert_eq!(
            query.install_search_dirs.first(),
            Some(&service_bin),
            "服务进程 PATH 必须排在查找序最前：{:?}",
            query.install_search_dirs
        );
        assert!(
            resolve_off_path(CLI, &query),
            "服务进程 PATH 有、登录 shell PATH 没有 ⇒ off_path 必须为 true"
        );

        // ② 登录 shell PATH 也有（同一个目录）⇒ false。
        let query = off_path_query_with(&service_path, Some(service_path.as_str()), Some(home.as_path()));
        assert!(
            !resolve_off_path(CLI, &query),
            "登录 shell PATH 也有 ⇒ off_path 必须为 false"
        );

        // ③ 两处都没有 ⇒ 未安装 ⇒ false。
        let empty_bin = root.path().join("empty-bin");
        let empty_path = empty_bin.display().to_string();
        let query = off_path_query_with(&empty_path, Some(login_path.as_str()), Some(home.as_path()));
        assert!(
            !resolve_off_path(CLI, &query),
            "两处都没有 ⇒ 未安装 ⇒ off_path 必须为 false"
        );
    }

    // ---- GET install-targets ----

    #[tokio::test]
    async fn install_targets_endpoint_lists_five_targets_with_actions() {
        let root = tempdir().expect("root");
        // health：claude ok、codex 缺失、pi ok（落后）、kimi ok。
        let health = health_service(
            root.path(),
            vec![
                success("1.0.0"),
                missing("codex"),
                success("0.85.0"),
                success("0.34.0"),
            ],
        );

        // handler 读取 latest_diagnostic——先刷新一次填入脚本化探测结果。
        health
            .refresh(tokio_util::sync::CancellationToken::new())
            .await
            .expect("refresh health");
        // codegraph 探测：命令缺失。
        let state = state(
            root.path(),
            health,
            ScriptedRunner::new(vec![missing("codegraph")], Duration::ZERO),
        );
        // 预置缓存：claude=1.0.0（最新）、codex=2.0.0（无关，缺装）、
        // pi=1.5.0（落后）、codegraph=1.6.2（无关，缺装）。
        // pi 的「落后」必须是**精确 semver 下真落后**（REQ-PCI-07）：
        // 本地 0.85.0（devbox pin）vs latest 1.5.0（块2 技术方案
        // §3.6.9 未验证项 1 实测的 registry latest）⇒ 0.85.0 < 1.5.0。
        // （旧的 0.9.0 在 semver 下**新于** 0.85.0 —— minor 9 < 85。）
        let checked_at = (Utc::now() - TimeDelta::hours(1)).to_rfc3339();
        seed_version_cache(
            root.path(),
            json!({
                "@anthropic-ai/claude-code": {
                    "package": "@anthropic-ai/claude-code",
                    "latest_version": "1.0.0",
                    "checked_at": checked_at
                },
                "@openai/codex": {
                    "package": "@openai/codex",
                    "latest_version": "2.0.0",
                    "checked_at": checked_at
                },
                "@earendil-works/pi-coding-agent": {
                    "package": "@earendil-works/pi-coding-agent",
                    "latest_version": "1.5.0",
                    "checked_at": checked_at
                },
                "@colbymchenry/codegraph": {
                    "package": "@colbymchenry/codegraph",
                    "latest_version": "1.6.2",
                    "checked_at": checked_at
                },
                "@moonshot-ai/kimi-code": {
                    "package": "@moonshot-ai/kimi-code",
                    "latest_version": "2.5.2",
                    "checked_at": checked_at
                }
            }),
        );

        let response = get_install_targets(State(state)).await;
        let payload = response_json(response.into_response()).await;

        let targets = payload["targets"].as_array().expect("targets array");
        assert_eq!(targets.len(), 5, "五类对象一次返回: {payload}");

        let by_target = |wire: &str| {
            targets
                .iter()
                .find(|dto| dto["target"] == json!(wire))
                .unwrap_or_else(|| panic!("missing {wire}: {payload}"))
                .clone()
        };

        let claude = by_target("claude_code");
        assert_eq!(claude["cli_available"], json!(true));
        assert_eq!(claude["local_version"], json!("1.0.0"));
        assert_eq!(claude["latest_version"], json!("1.0.0"));
        assert_eq!(claude["action"], json!("none"));
        assert_eq!(
            claude["install_source"],
            json!("npm @anthropic-ai/claude-code")
        );
        assert!(claude["latest_checked_at"].is_string());
        // 登录 shell PATH 快照不可用（测试线程未探测）⇒ off_path 不下结论（REQ-PCI-08）。
        assert_eq!(claude["off_path"], json!(false));

        let codex = by_target("codex");
        assert_eq!(codex["cli_available"], json!(false));
        assert_eq!(codex["action"], json!("install"));
        // 未安装对象 off_path 恒为 false（REQ-PCI-08）。
        assert_eq!(codex["off_path"], json!(false));

        let pi = by_target("pi");
        assert_eq!(pi["cli_available"], json!(true));
        assert_eq!(pi["local_version"], json!("0.85.0"));
        assert_eq!(pi["latest_version"], json!("1.5.0"));
        assert_eq!(pi["action"], json!("update"));

        // Kimi：版本来源 = npm 包 `@moonshot-ai/kimi-code`（REQ-PCI-07）⇒
        // 参与「有新版」判定；本地 0.34.0 < latest 2.5.2 ⇒ update。
        // **安装源仍是官方脚本**（REQ-PCI-01 不变）。
        let kimi = by_target("kimi_code");
        assert_eq!(kimi["cli_available"], json!(true));
        assert_eq!(kimi["local_version"], json!("0.34.0"));
        assert_eq!(kimi["latest_version"], json!("2.5.2"));
        assert!(kimi["latest_checked_at"].is_string());
        assert_eq!(kimi["action"], json!("update"));
        assert_eq!(
            kimi["install_source"],
            json!("official script https://code.kimi.com/kimi-code/install.sh")
        );

        let codegraph = by_target("codegraph");
        assert_eq!(codegraph["cli_available"], json!(false));
        assert_eq!(codegraph["action"], json!("install"));
        assert_eq!(
            codegraph["install_source"],
            json!("npm @colbymchenry/codegraph")
        );
    }

    /// Kimi **未安装** → 【安装】而不是【更新】：版本判定与安装判定相互独立
    /// —— 命令缺失时永远是 install（REQ-PCI-02 / REQ-PCI-07）。
    #[tokio::test]
    async fn install_targets_reports_install_for_missing_kimi() {
        let root = tempdir().expect("root");
        // health：claude ok、codex ok、pi ok、**kimi 缺失**。
        let health = health_service(
            root.path(),
            vec![
                success("1.0.0"),
                success("0.166.0"),
                success("1.3.0"),
                missing("kimi"),
            ],
        );
        health
            .refresh(tokio_util::sync::CancellationToken::new())
            .await
            .expect("refresh health");
        let state = state(
            root.path(),
            health,
            ScriptedRunner::new(vec![missing("codegraph")], Duration::ZERO),
        );
        // Kimi 的版本来源缓存（latest 2.5.2）——版本判定仍生效。
        let checked_at = (Utc::now() - TimeDelta::hours(1)).to_rfc3339();
        seed_version_cache(
            root.path(),
            json!({
                "@moonshot-ai/kimi-code": {
                    "package": "@moonshot-ai/kimi-code",
                    "latest_version": "2.5.2",
                    "checked_at": checked_at
                }
            }),
        );

        let response = get_install_targets(State(state)).await;
        let payload = response_json(response.into_response()).await;
        let targets = payload["targets"].as_array().expect("targets array");
        let kimi = targets
            .iter()
            .find(|dto| dto["target"] == json!("kimi_code"))
            .expect("kimi target");

        assert_eq!(kimi["cli_available"], json!(false));
        assert_eq!(kimi["local_version"], Value::Null);
        assert_eq!(kimi["latest_version"], json!("2.5.2"));
        assert_eq!(
            kimi["action"],
            json!("install"),
            "命令缺失时必须给【安装】，不能给【更新】"
        );
    }

    // ---- POST /install + GET /install/{job_id} ----

    /// 测试点 #8：POST 立即返回 {job_id}（不等待）；GET 轮询得终态。
    #[tokio::test]
    async fn install_target_post_returns_immediately_and_polls_to_completed() {
        let root = tempdir().expect("root");
        // 隔离宿主环境：装前 2.0.0 ⇒ 假安装写入 2.1.7（装前装后版本不同 ⇒
        // 不触发「命令仍是旧版」），断言只认隔离前缀里的 `package.json`。
        let env = IsolatedInstallEnv::new(root.path(), "2.0.0", "2.1.7");
        let health = health_service(
            root.path(),
            vec![
                success("1.0.0"),
                success("2.1.7"),
                success("0.85.0"),
                success("0.34.0"),
            ],
        );
        // 安装命令延迟 300ms，保证 POST 返回时作业仍是 running。
        let state = state(
            root.path(),
            health.clone(),
            ScriptedRunner::with_side_effect(
                vec![install_success()],
                Duration::from_millis(300),
                env.install_side_effect(),
            ),
        );
        let _isolation = isolate_install_env(&env, root.path());

        let response = post_provider_install(
            State(state.clone()),
            axum::extract::Path("codex".to_string()),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let started = response_json(response).await;
        let job_id = started["job_id"].as_str().expect("job_id").to_string();
        assert!(!job_id.is_empty());

        // 立即查询：仍是 running（安装尚未完成）。
        let running = response_json(
            get_install_job(State(state.clone()), axum::extract::Path(job_id.clone())).await,
        )
        .await;
        assert_eq!(running["status"], json!("running"));
        assert_eq!(running["target"], json!("codex"));

        // 轮询到终态：completed + 装完重探得到的版本。
        let done = poll_job_terminal(&state, &job_id).await;
        assert_eq!(done["status"], json!("completed"), "{done}");
        assert_eq!(done["outcome"]["installed_version"], json!("2.1.7"));
        // 重探确实发生（REQ-PCI-05 装完重新探测）。
        assert!(state.provider_health.latest_diagnostic().generation >= 1);
    }

    /// 测试点 #8：Kimi 缺 acknowledge_remote_script → 400（防绕过告知）。
    #[tokio::test]
    async fn install_target_post_rejects_kimi_without_acknowledgement() {
        let root = tempdir().expect("root");
        let health = health_service(
            root.path(),
            vec![
                success("1.0.0"),
                success("2.0.0"),
                success("0.85.0"),
                success("0.34.0"),
            ],
        );
        let state = state(
            root.path(),
            health,
            ScriptedRunner::new(vec![install_success()], Duration::ZERO),
        );

        for body in [None, Some(json!(false)), Some(json!(null))] {
            let request_body = body.as_ref().map(|ack| {
                Json(super::InstallRequestBody {
                    acknowledge_remote_script: ack.as_bool(),
                })
            });
            let response = post_provider_install(
                State(state.clone()),
                axum::extract::Path("kimi_code".to_string()),
                request_body,
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body:?}");
            let payload = response_json(response).await;
            assert_eq!(payload["code"], json!("install_acknowledgement_required"));
            assert!(
                payload["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("code.kimi.com")),
                "必须告知脚本来源: {payload}"
            );
        }

        // 未确认时不留任何作业痕迹。
        let jobs_dir = root.path().join(".aria/state/provider-installation/jobs");
        assert!(
            !jobs_dir.exists() || std::fs::read_dir(&jobs_dir).expect("jobs dir").count() == 0,
            "拒绝请求不得创建作业"
        );

        // 确认后放行。
        let response = post_provider_install(
            State(state.clone()),
            axum::extract::Path("kimi_code".to_string()),
            Some(Json(super::InstallRequestBody {
                acknowledge_remote_script: Some(true),
            })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let started = response_json(response).await;
        let job_id = started["job_id"].as_str().expect("job_id").to_string();
        let done = poll_job_terminal(&state, &job_id).await;
        assert_eq!(done["status"], json!("completed"), "{done}");
    }

    #[tokio::test]
    async fn install_target_post_unknown_target_is_rejected() {
        let root = tempdir().expect("root");
        let health = health_service(
            root.path(),
            vec![
                success("1.0.0"),
                success("2.0.0"),
                success("0.85.0"),
                success("0.34.0"),
            ],
        );
        let state = state(
            root.path(),
            health,
            ScriptedRunner::new(vec![], Duration::ZERO),
        );
        let response =
            post_provider_install(State(state), axum::extract::Path("fake".to_string()), None)
                .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], json!("install_target_unknown"));
    }

    // §五 2.11：未知对象（含 fake）→ 400，与 install 端点同口径。
    #[tokio::test]
    async fn expose_unknown_target_is_rejected() {
        for target in ["nope", "fake"] {
            let response = post_provider_expose(axum::extract::Path(target.to_string())).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{target}");
            let payload = response_json(response).await;
            assert_eq!(payload["code"], json!("install_target_unknown"), "{target}");
        }
    }

    #[tokio::test]
    async fn install_job_unknown_id_is_not_found() {
        let root = tempdir().expect("root");
        let health = health_service(
            root.path(),
            vec![
                success("1.0.0"),
                success("2.0.0"),
                success("0.85.0"),
                success("0.34.0"),
            ],
        );
        let state = state(
            root.path(),
            health,
            ScriptedRunner::new(vec![], Duration::ZERO),
        );
        let response =
            get_install_job(State(state), axum::extract::Path("no-such-job".to_string())).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], json!("install_job_not_found"));
    }

    /// 测试点 #9：安装成功 → Completed，未写任何 capability 记录、
    /// 未触发核验探针（spy 计数 0）；重探走 provider health。
    #[tokio::test]
    async fn install_success_does_not_write_capability_or_trigger_probes() {
        use crate::cross_cutting::provider_boundary::ProviderBoundaryError;
        use crate::product::logical_codebase::provider_capability_revalidate::{
            ProbeInvocation, ProbeRunner, ProviderCapabilityRevalidateService, VersionSource,
        };

        let root = tempdir().expect("root");
        // 隔离宿主环境（宿主真装了 codex 时，作业会落到宿主前缀 ⇒ 版本读宿主
        // 真实包、装前装后同版本 ⇒ 作业被判 failed）。
        let env = IsolatedInstallEnv::new(root.path(), "2.0.0", "2.1.7");
        let health = health_service(
            root.path(),
            vec![
                success("1.0.0"),
                success("2.1.7"),
                success("0.85.0"),
                success("0.34.0"),
            ],
        );
        let probe_calls = Arc::new(AtomicUsize::new(0));
        let calls_for_probe = probe_calls.clone();
        let probe_runner: ProbeRunner = Arc::new(move |_invocation: ProbeInvocation| {
            let calls = calls_for_probe.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(ProviderBoundaryError::ProbeFailed(
                    "capability probe must not run during install".to_string(),
                ))
            })
        });
        let version_source: VersionSource = Arc::new(|_program: &str| Ok("1.0.0".to_string()));
        let capability_service = Arc::new(
            ProviderCapabilityRevalidateService::new(
                crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria")),
            )
            .with_version_source(version_source)
            .with_probe_runner(probe_runner),
        );

        let gate = Arc::new(ProviderAvailabilityGate::new(health.clone()));
        let state = WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        )
        .with_provider_health(
            health.clone(),
            gate,
            ScriptedRunner::with_side_effect(
                vec![install_success()],
                Duration::ZERO,
                env.install_side_effect(),
            ),
        )
        .with_capability_revalidate_service(capability_service);
        let _isolation = isolate_install_env(&env, root.path());

        let response = post_provider_install(
            State(state.clone()),
            axum::extract::Path("codex".to_string()),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let started = response_json(response).await;
        let job_id = started["job_id"].as_str().expect("job_id").to_string();

        let done = poll_job_terminal(&state, &job_id).await;
        assert_eq!(done["status"], json!("completed"), "{done}");

        // 未触发任何能力核验探针（安装 ≠ 核验，REQ-PCI-05）。
        assert_eq!(
            probe_calls.load(Ordering::SeqCst),
            0,
            "安装成功不得自动触发核验探针"
        );

        // 未写任何 capability 记录（durable 工件扫描）。
        fn assert_no_capability_artifacts(directory: &Path) {
            let entries = std::fs::read_dir(directory).expect("read dir");
            for entry in entries {
                let entry = entry.expect("dir entry");
                let name = entry.file_name().to_string_lossy().into_owned();
                assert!(
                    !name.to_lowercase().contains("capability"),
                    "安装流程不得写 capability 记录: {}",
                    entry.path().display()
                );
                if entry.path().is_dir() {
                    assert_no_capability_artifacts(&entry.path());
                }
            }
        }
        let aria_root = root.path().join(".aria");
        if aria_root.exists() {
            assert_no_capability_artifacts(&aria_root);
        }

        // 装完确实重探了（provider health generation 前进）。
        assert!(state.provider_health.latest_diagnostic().generation >= 1);
    }
