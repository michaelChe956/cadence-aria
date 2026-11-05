// S0（任务1）Step 6：制品进程 smoke 隔离入口的父进程失败分类红测试。
// 本文件经 workspace_ws_integration.rs include! 进同一模块作用域——具名内联
// 子模块包裹并 `use super::*` 复用父 fixture，避免与其它 include! 文件的
// 同合同私有 helper 重名（设计 §6.3；隔离器实现属 Step 8，本文件先行红测）。
pub(crate) mod process_smoke {
    #[allow(unused_imports)] // 复用父 fixture 的通道：任务 9/10 真实制品 smoke 场景消费后移除
    use super::*;

    /// 子模式环境标识（与单元侧 small_stack_worker 同合同、各自私有）。
    const PROCESS_PROVIDER_CASE_CHILD_MODE_ENV: &str = "ARIA_PROVIDER_CASE_CHILD_MODE";

    // 私有同合同隔离 helper（同 small_stack_worker 合同，本测试二进制内私有，
    // 不跨二进制耦合；Step 8 实现）。
    fn run_isolated_provider_case(
        case_suffix: &str,
        timeout: std::time::Duration,
    ) -> Result<(), String> {
        use std::process::{Command, Stdio};

        // 明确 child 模式防递归：子模式进程内不得再启动隔离子例（设计 §6.3）。
        if std::env::var_os(PROCESS_PROVIDER_CASE_CHILD_MODE_ENV).is_some() {
            return Err(format!(
                "isolated child failure must be rejected: recursion guard hit, case={case_suffix} \
                 （子模式进程内禁止再启动隔离子例）"
            ));
        }

        let current_exe =
            std::env::current_exe().map_err(|error| format!("current_exe failed: {error}"))?;

        // 测试列表唯一匹配后生成 exact（使用实施后实际存在的全路径，不预写猜测命令）。
        let listing = Command::new(&current_exe)
            .arg("--list")
            .output()
            .map_err(|error| format!("list isolated provider cases failed: {error}"))?;
        let matched: Vec<String> = String::from_utf8_lossy(&listing.stdout)
            .lines()
            .map(|line| line.trim_end().trim_end_matches(": test").to_string())
            .filter(|name| name.ends_with(case_suffix))
            .collect();
        let [case] = matched.as_slice() else {
            return Err(format!(
                "isolated provider case suffix {case_suffix:?} must match exactly one test, matched {matched:?}"
            ));
        };
        let case = case.to_string();

        let mut command = Command::new(&current_exe);
        command
            .args(["--exact", &case, "--nocapture", "--test-threads=1"])
            .env(PROCESS_PROVIDER_CASE_CHILD_MODE_ENV, "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // 自有进程组（子进程即组长）：墙钟超时可整组 kill，不误伤父套件。
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("spawn isolated case {case} failed: {error}"))?;

        // 同步父持续排空两条输出管道（线程读到 EOF），避免管道写满造成伪超时。
        let stdout_thread = drain_pipe(child.stdout.take().expect("piped stdout"));
        let stderr_thread = drain_pipe(child.stderr.take().expect("piped stderr"));
        let deadline = std::time::Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        kill_process_group(&mut child);
                        let status = child.wait().unwrap_or_else(|error| {
                            panic!("wait isolated case {case} after kill failed: {error}")
                        });
                        let stdout = stdout_thread.join().unwrap_or_default();
                        let stderr = stderr_thread.join().unwrap_or_default();
                        let stdout_excerpt = excerpt(&stdout);
                        let stderr_excerpt = excerpt(&stderr);
                        return Err(format!(
                            "isolated child failure must be rejected: case={case} timeout after {timeout:?}\
                             （墙钟到期 kill 自有进程组；status={status}）stdout[..]={stdout_excerpt}, stderr[..]={stderr_excerpt}"
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(error) => return Err(format!("wait isolated case {case} failed: {error}")),
            }
        };
        let stdout = stdout_thread.join().unwrap_or_default();
        let stderr = stderr_thread.join().unwrap_or_default();
        let stdout_excerpt = excerpt(&stdout);
        let stderr_excerpt = excerpt(&stderr);

        if status.success() {
            return Ok(());
        }
        // Unix：任何信号都算失败（SIGABRT=signal 6 是易 abort 场景的预期红形态）；
        // 正常场景永不要求「必须 abort 才通过」。
        #[cfg(unix)]
        if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
            return Err(format!(
                "isolated child failure must be rejected: case={case} signal {signal} (code {}), \
                 stdout[..]={stdout_excerpt}, stderr[..]={stderr_excerpt}",
                status.code().unwrap_or(-1)
            ));
        }
        Err(format!(
            "isolated child failure must be rejected: case={case} exit {} (nonzero), \
             stdout[..]={stdout_excerpt}, stderr[..]={stderr_excerpt}",
            status.code().unwrap_or(-1)
        ))
    }

    /// 后台排空一条子进程管道直到 EOF（持续读，防管道写满伪超时）。
    fn drain_pipe(pipe: impl std::io::Read + Send + 'static) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let mut pipe = pipe;
            let mut buffer = String::new();
            let _ = pipe.read_to_string(&mut buffer);
            buffer
        })
    }

    /// 输出摘录（尾部，压平换行）——完整证据由父测试打印保存。
    fn excerpt(output: &str) -> String {
        let tail: String = output
            .chars()
            .rev()
            .take(1200)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        tail.replace(['\n', '\r'], " ⏎ ")
    }

    /// 墙钟超时 kill：unix 广播自有进程组；其余平台退回 kill 组长。
    fn kill_process_group(child: &mut std::process::Child) {
        #[cfg(unix)]
        {
            // 自有进程组（spawn 时 process_group(0)）：负 pid 广播整组，防孙进程残留。
            let pgid = child.id() as i32;
            unsafe { libc::kill(-pgid, libc::SIGKILL) };
        }
        #[cfg(not(unix))]
        {
            let _ = child.kill();
        }
    }

    /// 子模式入口：显式 2 MiB runtime + `rt.spawn` 真正入口，根只等待
    /// （设计 §6.2/§6.3——入口流程发生在 worker，不在 libtest/block_on 根）。
    fn run_process_child_scenario(scenario: &str) {
        if std::env::var_os(PROCESS_PROVIDER_CASE_CHILD_MODE_ENV).is_none() {
            // 常规套件直通：本用例是隔离入口，不是常规断言面。
            return;
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_stack_size(2 * 1024 * 1024)
            .enable_all()
            .build()
            .expect("process child explicit 2 MiB runtime");
        let scenario = scenario.to_string();
        rt.block_on(async {
            tokio::spawn(async move {
                match scenario.as_str() {
                    "success" => {}
                    "assertion_failure" => {
                        panic!("process planned assertion failure for isolation classification")
                    }
                    #[cfg(unix)]
                    "sigabrt" => std::process::abort(),
                    "timeout" => std::future::pending::<()>().await,
                    other => unreachable!("unknown process child scenario: {other}"),
                }
            })
            .await
            .expect("process child scenario task must not panic before its planned exit");
        });
    }

    #[test]
    fn process_provider_case_child_success() {
        run_process_child_scenario("success");
    }

    #[test]
    fn process_provider_case_child_assertion_failure() {
        run_process_child_scenario("assertion_failure");
    }

    #[cfg(unix)]
    #[test]
    fn process_provider_case_child_sigabrt() {
        run_process_child_scenario("sigabrt");
    }

    #[test]
    fn process_provider_case_child_timeout() {
        run_process_child_scenario("timeout");
    }

    #[test]
    fn process_isolated_case_rejects_assertion_signal_and_timeout() {
        // 普通成功子例得到 Ok。
        let ok = run_isolated_provider_case(
            "process_provider_case_child_success",
            std::time::Duration::from_secs(60),
        );
        assert!(ok.is_ok(), "normal process child case must pass: {ok:?}");

        // 断言失败子例：Err 附 case/exit/stderr。
        let err = run_isolated_provider_case(
            "process_provider_case_child_assertion_failure",
            std::time::Duration::from_secs(60),
        )
        .expect_err("process assertion-failure child must be rejected");
        assert!(
            err.contains("process_provider_case_child_assertion_failure"),
            "report must carry the case name: {err}"
        );
        assert!(err.contains("exit"), "report must carry exit status: {err}");
        assert!(
            err.contains("process planned assertion failure"),
            "report must carry stderr excerpt: {err}"
        );

        // Unix SIGABRT 子例：Err 附 signal 6。
        #[cfg(unix)]
        {
            let err = run_isolated_provider_case(
                "process_provider_case_child_sigabrt",
                std::time::Duration::from_secs(60),
            )
            .expect_err("process SIGABRT child must be rejected");
            assert!(
                err.contains("process_provider_case_child_sigabrt"),
                "report must carry the case name: {err}"
            );
            assert!(
                err.contains("signal 6"),
                "report must carry SIGABRT signal: {err}"
            );
        }

        // 超时子例：Err 附 case 与 timeout 归因。
        let err = run_isolated_provider_case(
            "process_provider_case_child_timeout",
            std::time::Duration::from_secs(5),
        )
        .expect_err("process timeout child must be rejected");
        assert!(
            err.contains("process_provider_case_child_timeout"),
            "report must carry the case name: {err}"
        );
        assert!(
            err.contains("timeout"),
            "report must attribute timeout: {err}"
        );
    }
}
