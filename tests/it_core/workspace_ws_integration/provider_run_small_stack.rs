// P9（任务10）C 层：真实 WS 深链在显式 2 MiB worker 上的回归。
// 本文件经 workspace_ws_integration.rs include! 进同一模块作用域——具名内联
// 子模块包裹并 `use super::*` 复用父 fixture（part_02/part_05 真实
// TcpListener→axum→WS→StartGeneration 夹具），不复制父 harness。
//
// 结构（设计 §6.1 C 行 / §6.2 / §6.3）：
// - `provider_run_2mib_real_ws_matrix`：同步父测试，经隔离子进程逐场景执行；
// - `provider_run_2mib_real_ws_child_*`：子模式场景（常规套件直通），入口
//   （accept/connection/StartGeneration 深链）全部在显式 2 MiB runtime 的
//   `rt.spawn` worker 上执行，block_on 根只等待，并记录/断言线程 identity；
// - `ws_smoke_rejects_close_timeout_and_missing_durable_progress`：动态证据
//   合同——客户端 close / timeout / 仅locked / 无provider-start / 无durable
//   五种证据形态必须全部 Err（exit 0 不算动态通过）；
// - 真实场景执行体在 `provider_run_small_stack/scenario_cases.rs`（#[path]
//   子模块，同 basic_sc_arm_cases/sc_author_stop_cases.rs 先例；父文件行数
//   门禁 ≤1000 的拆分）。
// 真实场景执行体（#[path] 顶层声明：路径相对 workspace_ws_integration.rs 所在
// 目录解析；场景文件 use super::* 直达父 harness fixture）。
#[path = "provider_run_small_stack/scenario_support.rs"]
pub(crate) mod ws_scenario_support;

#[path = "provider_run_small_stack/scenario_cases.rs"]
pub(crate) mod ws_scenario_cases;

pub(crate) mod ws_small_stack {
    /// 子模式环境标识（与单元侧 small_stack_worker 同合同、各自私有）。
    const WS_PROVIDER_CASE_CHILD_MODE_ENV: &str = "ARIA_PROVIDER_CASE_CHILD_MODE";

    // 真实场景执行体（Story 空 Author / 最重 plan 接力 / SC manual 门）。
    use super::ws_scenario_cases as scenario_cases;


    // 私有同合同隔离 helper（同 small_stack_worker 合同，本测试二进制内私有，
    // 不跨二进制耦合）。
    fn run_isolated_provider_case(
        case_suffix: &str,
        timeout: std::time::Duration,
    ) -> Result<(), String> {
        use std::process::{Command, Stdio};

        // 明确 child 模式防递归：子模式进程内不得再启动隔离子例（设计 §6.3）。
        if std::env::var_os(WS_PROVIDER_CASE_CHILD_MODE_ENV).is_some() {
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
            .env(WS_PROVIDER_CASE_CHILD_MODE_ENV, "1")
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

    // -----------------------------------------------------------------------
    // C 层动态证据合同（设计 §6.1 C 判据；exit 0 不算动态通过）
    // -----------------------------------------------------------------------

    /// C 层动态证据：四条件（服务存活 / 真实 provider start / 完整帧+因果偏序 /
    /// durable 进展）与五种失败形态的判定面。帧序号统一采用「帧接收 + 帧后
    /// durable 检查点」的同一事件序，只断合同偏序，不断言跨 sender 全局总序。
    #[derive(Debug, Clone)]
    pub(super) struct WsSmokeEvidence {
        pub(super) scenario: String,
        /// 失败形态①：客户端观察到连接被服务端关闭。
        pub(super) client_closed: bool,
        /// 失败形态②：客户端读超时。
        pub(super) client_timeout: bool,
        /// ProviderLocked（前奏帧）在事件序中的位置——locked/登记 run 不算启动。
        pub(super) provider_locked_at: Option<usize>,
        /// 可信 provider-start 探针：Scripted 替身 start_validated 真实调用次数。
        pub(super) provider_starts: usize,
        /// 首个输出帧被观察到时**已发生**的真实 validated start 次数——因果偏序的
        /// 跨 sender 锚点（provider start 发生在输出帧之前，而非事后比较两个
        /// 独立计数器的终值）。
        pub(super) provider_starts_before_first_output: usize,
        /// 本路径是否应当出现 ProviderLocked 前奏帧——StartGeneration 族路径为
        /// true（无前奏即判失败）；人工门反馈族路径（无 StartGeneration 节点）
        /// 为 false（该路径生产上就没有 locked 帧，不得凭空要求）。
        pub(super) expects_provider_locked: bool,
        /// 本 run 首个输出帧（stream_chunk）位置——只收到初始 session_state 判失败。
        pub(super) first_output_at: Option<usize>,
        /// durable Completed 在事件序中首次可观察的位置（帧后检查点）。
        pub(super) durable_completed_at: Option<usize>,
        /// completed 回显帧（message_complete 等）位置。
        pub(super) completed_echo_at: Option<usize>,
        /// 取消 run 的失败回显次数（Error/Failed 回显，须 ≤1）。
        pub(super) cancel_failure_echoes: usize,
        /// durable 状态前后差异（timeline/message 至少一项推进）。
        pub(super) durable_progress: bool,
    }

    impl WsSmokeEvidence {
        fn good(scenario: &str) -> Self {
            Self {
                scenario: scenario.to_string(),
                client_closed: false,
                client_timeout: false,
                provider_locked_at: Some(1),
                provider_starts: 1,
                provider_starts_before_first_output: 1,
                expects_provider_locked: true,
                first_output_at: Some(3),
                durable_completed_at: Some(4),
                completed_echo_at: Some(5),
                cancel_failure_echoes: 0,
                durable_progress: true,
            }
        }
    }

    /// 判定 C 层证据：四条件全部成立才 Ok；五种失败形态（close / timeout /
    /// 仅locked / 无provider-start / 无durable）与偏序破坏全部 Err。
    pub(super) fn verify_ws_smoke_evidence(evidence: &WsSmokeEvidence) -> Result<(), String> {
        let scenario = &evidence.scenario;
        if evidence.client_closed {
            return Err(format!(
                "ws smoke rejected: client connection closed by server (scenario={scenario})\
                 （客户端 close 不算通过）"
            ));
        }
        if evidence.client_timeout {
            return Err(format!(
                "ws smoke rejected: client timeout (scenario={scenario})（超时不算通过）"
            ));
        }
        if evidence.provider_starts == 0 {
            return Err(format!(
                "trusted provider start missing (scenario={scenario})\
                 （locked/登记 run 不算真实启动）"
            ));
        }
        // 因果偏序锚点：真实 provider start 必须发生在**本 run 首个输出帧之前**
        // （跨 sender 偏序，用同一事件序上的探针快照判定，不比较事后终值）。
        if evidence.provider_starts_before_first_output == 0 {
            return Err(format!(
                "causal order violated: trusted provider start must precede run output \
                 (scenario={scenario}, starts_before_output=0)"
            ));
        }
        if evidence.expects_provider_locked && evidence.provider_locked_at.is_none() {
            return Err(format!(
                "provider_locked frame missing (scenario={scenario})"
            ));
        }
        if evidence.first_output_at.is_none() {
            return Err(format!(
                "run output frame missing (scenario={scenario})\
                 （只收到初始 session_state 判失败）"
            ));
        }
        if evidence.expects_provider_locked && evidence.provider_locked_at >= evidence.first_output_at
        {
            return Err(format!(
                "causal order violated: provider_locked must precede run output \
                 (scenario={scenario}, locked={:?}, output={:?})",
                evidence.provider_locked_at, evidence.first_output_at
            ));
        }
        if !evidence.durable_progress {
            return Err(format!(
                "durable progress missing (scenario={scenario})（无 durable 进展判失败）"
            ));
        }
        if let Some(echo) = evidence.completed_echo_at {
            match evidence.durable_completed_at {
                Some(durable) if durable < echo => {}
                other => {
                    return Err(format!(
                        "causal order violated: durable Completed must precede its completed \
                         echo (scenario={scenario}, durable={other:?}, echo={echo})"
                    ));
                }
            }
        }
        if evidence.cancel_failure_echoes > 1 {
            return Err(format!(
                "cancel failure echoed {} times (>1) (scenario={scenario})",
                evidence.cancel_failure_echoes
            ));
        }
        Ok(())
    }

    /// Step 2 动态证据拒绝测试：五种失败形态必须全部 Err；真实成功序列（偏序
    /// 核对：locked 先于本 run 输出、durable Completed 先于其 completed 回显、
    /// 取消失败回显 ≤1）必须 Ok。纯夹具侧证据，不启动服务。
    #[test]
    fn ws_smoke_rejects_close_timeout_and_missing_durable_progress() {
        // 失败形态①：客户端 close。
        let mut close = WsSmokeEvidence::good("close_form");
        close.client_closed = true;
        let err = verify_ws_smoke_evidence(&close).expect_err("close form must be rejected");
        assert!(
            err.contains("client connection closed"),
            "close form rejection must be explicit: {err}"
        );

        // 失败形态②：客户端 timeout。
        let mut timeout_form = WsSmokeEvidence::good("timeout_form");
        timeout_form.client_timeout = true;
        let err =
            verify_ws_smoke_evidence(&timeout_form).expect_err("timeout form must be rejected");
        assert!(
            err.contains("client timeout"),
            "timeout form rejection must be explicit: {err}"
        );

        // 失败形态③：仅 locked（前奏齐全但没有真实 provider start）。
        let mut locked_only = WsSmokeEvidence::good("locked_only_form");
        locked_only.provider_starts = 0;
        locked_only.first_output_at = None;
        let err =
            verify_ws_smoke_evidence(&locked_only).expect_err("locked-only form must be rejected");
        assert!(
            err.contains("trusted provider start missing"),
            "locked-only form must be rejected as missing real start: {err}"
        );

        // 失败形态④：无 provider-start（连 locked 前奏都未观察到，仍以 start
        // 缺失为第一归因）。
        let mut no_start = WsSmokeEvidence::good("no_provider_start_form");
        no_start.provider_starts = 0;
        no_start.provider_locked_at = None;
        let err =
            verify_ws_smoke_evidence(&no_start).expect_err("no-provider-start must be rejected");
        assert!(
            err.contains("trusted provider start missing"),
            "no-provider-start form must be rejected: {err}"
        );

        // 失败形态⑤：无 durable 进展。
        let mut no_durable = WsSmokeEvidence::good("no_durable_form");
        no_durable.durable_progress = false;
        let err =
            verify_ws_smoke_evidence(&no_durable).expect_err("no-durable form must be rejected");
        assert!(
            err.contains("durable progress missing"),
            "no-durable form must be rejected: {err}"
        );

        // 偏序破坏：durable Completed 晚于 completed 回显（写后回显合同被破坏）。
        let mut late_durable = WsSmokeEvidence::good("late_durable_form");
        late_durable.durable_completed_at = Some(6);
        let err = verify_ws_smoke_evidence(&late_durable)
            .expect_err("late durable completion must be rejected");
        assert!(
            err.contains("durable Completed must precede"),
            "late-durable form must be rejected: {err}"
        );

        // 取消失败回显 >1。
        let mut repeated_cancel = WsSmokeEvidence::good("repeated_cancel_form");
        repeated_cancel.cancel_failure_echoes = 2;
        let err = verify_ws_smoke_evidence(&repeated_cancel)
            .expect_err("repeated cancel echo must be rejected");
        assert!(
            err.contains("cancel failure echoed 2 times"),
            "repeated-cancel form must be rejected: {err}"
        );

        // 真实成功序列必须 Ok（四条件 + 偏序全部成立）。
        let good = WsSmokeEvidence::good("success_form");
        verify_ws_smoke_evidence(&good).expect("well-formed evidence must pass");
    }

    // -----------------------------------------------------------------------
    // 子模式入口：显式 2 MiB runtime + `rt.spawn` 真正入口，根只等待
    // （设计 §6.2/§6.3——入口流程发生在 worker，不在 libtest/block_on 根；
    // 显式 2 MiB 覆盖 RUST_MIN_STACK 环境值，记录线程 identity）。
    // -----------------------------------------------------------------------
    fn run_ws_child_scenario(scenario: &str) {
        if std::env::var_os(WS_PROVIDER_CASE_CHILD_MODE_ENV).is_none() {
            // 常规套件直通：本用例是隔离入口，不是常规断言面。
            return;
        }
        let root_thread = std::thread::current().id();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_stack_size(2 * 1024 * 1024)
            .enable_all()
            .build()
            .expect("ws child explicit 2 MiB runtime");
        let scenario = scenario.to_string();
        let outcome = rt.block_on(async {
            tokio::spawn(async move {
                // 记录线程 identity：入口必须运行在 rt.spawn worker（显式
                // 2 MiB），不在 block_on 根线程（设计 §6.2）。
                let entry_thread = std::thread::current().id();
                let entry_thread_name =
                    std::thread::current().name().unwrap_or("<unnamed>").to_string();
                assert_ne!(
                    root_thread, entry_thread,
                    "入口必须运行在 rt.spawn worker，而非 block_on 根线程"
                );
                println!(
                    "[ws-2mib] thread identity: block_on_root={root_thread:?} \
                     entry_worker={entry_thread:?} name={entry_thread_name} stack=2MiB"
                );
                match scenario.as_str() {
                    "success" => Ok(()),
                    "assertion_failure" => {
                        panic!("ws planned assertion failure for isolation classification")
                    }
                    #[cfg(unix)]
                    "sigabrt" => std::process::abort(),
                    "timeout" => {
                        let _ = std::future::pending::<()>().await;
                        unreachable!("timeout scenario must not return")
                    }
                    "story_author" => scenario_cases::run_story_author_scenario().await,
                    "plan_relay" => scenario_cases::run_plan_relay_scenario().await,
                    "sc_manual_gate" => scenario_cases::run_sc_manual_gate_scenario().await,
                    other => unreachable!("unknown ws child scenario: {other}"),
                }
            })
            .await
            .expect("ws child scenario task must not panic before its planned exit")
        });
        if let Err(error) = outcome {
            panic!("ws child scenario failed: {error}");
        }
    }

    #[test]
    fn ws_provider_case_child_success() {
        run_ws_child_scenario("success");
    }

    #[test]
    fn ws_provider_case_child_assertion_failure() {
        run_ws_child_scenario("assertion_failure");
    }

    #[cfg(unix)]
    #[test]
    fn ws_provider_case_child_sigabrt() {
        run_ws_child_scenario("sigabrt");
    }

    #[test]
    fn ws_provider_case_child_timeout() {
        run_ws_child_scenario("timeout");
    }

    #[test]
    fn provider_run_2mib_real_ws_child_story_author() {
        run_ws_child_scenario("story_author");
    }

    #[test]
    fn provider_run_2mib_real_ws_child_plan_relay() {
        run_ws_child_scenario("plan_relay");
    }

    #[test]
    fn provider_run_2mib_real_ws_child_sc_manual_gate() {
        run_ws_child_scenario("sc_manual_gate");
    }

    #[test]
    fn ws_isolated_case_rejects_assertion_signal_and_timeout() {
        // 普通成功子例得到 Ok。
        let ok = run_isolated_provider_case(
            "ws_provider_case_child_success",
            std::time::Duration::from_secs(60),
        );
        assert!(ok.is_ok(), "normal ws child case must pass: {ok:?}");

        // 断言失败子例：Err 附 case/exit/stderr。
        let err = run_isolated_provider_case(
            "ws_provider_case_child_assertion_failure",
            std::time::Duration::from_secs(60),
        )
        .expect_err("ws assertion-failure child must be rejected");
        assert!(
            err.contains("ws_provider_case_child_assertion_failure"),
            "report must carry the case name: {err}"
        );
        assert!(err.contains("exit"), "report must carry exit status: {err}");
        assert!(
            err.contains("ws planned assertion failure"),
            "report must carry stderr excerpt: {err}"
        );

        // Unix SIGABRT 子例：Err 附 signal 6。
        #[cfg(unix)]
        {
            let err = run_isolated_provider_case(
                "ws_provider_case_child_sigabrt",
                std::time::Duration::from_secs(60),
            )
            .expect_err("ws SIGABRT child must be rejected");
            assert!(
                err.contains("ws_provider_case_child_sigabrt"),
                "report must carry the case name: {err}"
            );
            assert!(
                err.contains("signal 6"),
                "report must carry SIGABRT signal: {err}"
            );
        }

        // 超时子例：Err 附 case 与 timeout 归因。
        let err = run_isolated_provider_case(
            "ws_provider_case_child_timeout",
            std::time::Duration::from_secs(5),
        )
        .expect_err("ws timeout child must be rejected");
        assert!(
            err.contains("ws_provider_case_child_timeout"),
            "report must carry the case name: {err}"
        );
        assert!(
            err.contains("timeout"),
            "report must attribute timeout: {err}"
        );
    }

    /// C 层同步父矩阵（设计 §6.1 C / §6.3）：三个真实 WS 深链场景经隔离子
    /// 进程执行——任何 signal/异常退出/墙钟超时都失败；exit 0 不算动态通过，
    /// 四条件证据由子场景内部断言（verify_ws_smoke_evidence）并打印进子
    /// stdout/stderr 供父报告摘录。
    #[test]
    fn provider_run_2mib_real_ws_matrix() {
        let scenarios: &[(&str, u64)] = &[
            ("provider_run_2mib_real_ws_child_story_author", 240),
            ("provider_run_2mib_real_ws_child_plan_relay", 300),
            ("provider_run_2mib_real_ws_child_sc_manual_gate", 240),
        ];
        for (case_suffix, timeout_secs) in scenarios {
            let result = run_isolated_provider_case(
                case_suffix,
                std::time::Duration::from_secs(*timeout_secs),
            );
            assert!(
                result.is_ok(),
                "C 层真实 WS 场景 {case_suffix} 未通过（四条件证据见子输出）: {result:?}"
            );
        }
    }
}
