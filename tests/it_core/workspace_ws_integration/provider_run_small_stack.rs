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

// 送审轮场景（ReviewOnly——真机爆栈所在的轮次）独立成文件：它的证据合同与
// 上面三个场景族不同（要观测 reviewer 的真实 start + durable ledger）。
#[path = "provider_run_small_stack/scenario_review.rs"]
pub(crate) mod ws_scenario_review;

pub(crate) mod ws_small_stack {
    /// 子模式环境标识（与单元侧 small_stack_worker 同合同、各自私有）。
    const WS_PROVIDER_CASE_CHILD_MODE_ENV: &str = "ARIA_PROVIDER_CASE_CHILD_MODE";

    /// 子进程场景证据落盘路径（父→子）。栈溢出是 SIGABRT ⇒ 子进程被 abort
    /// 时 stdout 与析构都可能丢；「送审轮是否真的走到」必须由父进程在子进程
    /// 退出后从该文件读取，不能只依赖子进程退出前的打印。
    pub(super) const WS_PROVIDER_CASE_EVIDENCE_ENV: &str = "ARIA_PROVIDER_CASE_EVIDENCE_PATH";

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
        // 证据落盘文件（跨 SIGABRT 存活）：子进程每条证据行即时 flush，父进程
        // 在子进程退出后读取——子进程被 abort 时 stdout 可能只到一半。
        let evidence_path = std::env::temp_dir().join(format!(
            "aria-provider-case-evidence-{}-{}.log",
            std::process::id(),
            case_suffix.replace(['/', ':', ' '], "_")
        ));
        let _ = std::fs::remove_file(&evidence_path);
        command.env(WS_PROVIDER_CASE_EVIDENCE_ENV, &evidence_path);
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
                        let evidence_excerpt = excerpt(&read_case_evidence(&evidence_path));
                        let durable_evidence =
                            read_durable_evidence(&read_case_evidence(&evidence_path));
                        return Err(format!(
                            "isolated child failure must be rejected: case={case} timeout after {timeout:?}\
                             （墙钟到期 kill 自有进程组；status={status}）stdout[..]={stdout_excerpt}, stderr[..]={stderr_excerpt}, evidence[..]={evidence_excerpt}, {durable_evidence}"
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
        let evidence = read_case_evidence(&evidence_path);
        let evidence_excerpt = excerpt(&evidence);
        // 子进程被 SIGABRT 终结时 stdout/析构都可能丢：durable 证据（ReviewerRun
        // 节点 / provider_start_ledger）只能由父进程从**遗留数据根**读回。
        let durable_evidence = read_durable_evidence(&evidence);

        if status.success() {
            if !evidence.is_empty() {
                println!("[ws-2mib] isolated case {case} 场景证据:\n{evidence}");
            }
            return Ok(());
        }
        // Unix：任何信号都算失败（SIGABRT=signal 6 是易 abort 场景的预期红形态）；
        // 正常场景永不要求「必须 abort 才通过」。
        #[cfg(unix)]
        if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
            // 栈溢出归因：Rust runtime 的栈溢出诊断串（"has overflowed its
            // stack"）是本次爆栈的可判定特征——SIGABRT 在此必须归因为
            // **stack overflow**（深 inbound 链 + 评审链 poll 帧之和超 2 MiB
            // worker 栈），不得笼统写成 signal 6。
            let stack_overflow = stderr.contains("has overflowed its stack")
                || stdout.contains("has overflowed its stack");
            let attribution = if stack_overflow {
                "stack overflow（tokio worker 栈溢出：深 inbound 链 + 评审链 poll 帧之和超 2 MiB）"
            } else {
                "signal（未观测到栈溢出特征）"
            };
            return Err(format!(
                "isolated child failure must be rejected: case={case} {attribution} signal {signal} (code {}), \
                 stdout[..]={stdout_excerpt}, stderr[..]={stderr_excerpt}, evidence[..]={evidence_excerpt}, {durable_evidence}",
                status.code().unwrap_or(-1)
            ));
        }
        Err(format!(
            "isolated child failure must be rejected: case={case} exit {} (nonzero), \
             stdout[..]={stdout_excerpt}, stderr[..]={stderr_excerpt}, evidence[..]={evidence_excerpt}, {durable_evidence}",
            status.code().unwrap_or(-1)
        ))
    }

    /// 读取子进程落盘的场景证据（跨 SIGABRT 存活；缺失即空串——不因此判失败，
    /// 证据缺失只影响归因信息量）。
    fn read_case_evidence(evidence_path: &std::path::Path) -> String {
        std::fs::read_to_string(evidence_path).unwrap_or_default()
    }

    /// 子进程遗留的 durable 证据（读回崩溃当时已落盘的事实）。
    ///
    /// 子进程被 SIGABRT 终结 ⇒ `TempDir` 不析构 ⇒ 数据根仍在盘上；父进程从
    /// 证据行的 `data_root=` 取出数据根，用生产 `LifecycleStore` 读出 durable
    /// 会话记录与时间线节点 ⇒ **送审轮是否真的走到**（ReviewerRun 节点已建 /
    /// reviewer 的 provider_start_ledger 条目）在崩溃后仍可判定。
    /// 读不到（子进程正常退出并清理了数据根）时返回空串，不影响判定。
    fn read_durable_evidence(evidence: &str) -> String {
        use super::{LifecycleStore, ProductAppPaths, TimelineNodeType};
        let Some(data_root) = evidence
            .split("data_root=")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .map(str::to_string)
        else {
            return String::new();
        };
        let app_paths = ProductAppPaths::new(std::path::Path::new(&data_root).join(".aria"));
        let lifecycle = LifecycleStore::new(app_paths);
        let session = lifecycle.get_workspace_session("workspace_session_0001");
        let nodes = lifecycle
            .load_timeline_nodes("workspace_session_0001")
            .unwrap_or_default();
        let reviewer_nodes: Vec<String> = nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::ReviewerRun)
            .map(|node| format!("{}:{:?}", node.node_id, node.status))
            .collect();
        let node_types: Vec<String> = nodes
            .iter()
            .map(|node| format!("{:?}", node.node_type))
            .collect();
        let ledger: Vec<String> = session
            .as_ref()
            .map(|record| {
                record
                    .provider_start_ledger
                    .iter()
                    .map(|entry| {
                        format!(
                            "{}(started={},provider={:?},at={:?})",
                            entry.provider_start_idempotency_key,
                            entry.started,
                            entry.provider,
                            entry.started_at
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let status = session
            .as_ref()
            .map(|record| format!("{:?}", record.status))
            .unwrap_or_else(|_| "<会话记录不可读>".to_string());
        let reviewer_provider = session
            .as_ref()
            .map(|record| format!("{:?}", record.reviewer_provider))
            .unwrap_or_else(|_| "<会话记录不可读>".to_string());
        // 遗留数据根清理（子进程被 abort ⇒ TempDir 未析构，父进程负责回收）。
        let _ = std::fs::remove_dir_all(std::path::Path::new(&data_root));
        format!(
            "durable 送审轮证据: status={status}, reviewer_provider={reviewer_provider}, \
             reviewer_run_nodes={reviewer_nodes:?}, provider_start_ledger={ledger:?}, \
             timeline_node_types={node_types:?}"
        )
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
        if evidence.expects_provider_locked
            && evidence.provider_locked_at >= evidence.first_output_at
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
        run_ws_child_scenario_with_stack(scenario, 2 * 1024 * 1024);
    }

    /// 子模式执行体（显式 worker 栈可参数化）。
    ///
    /// 门禁固定 2 MiB（设计 §6.1 C 行 / §6.2：显式值覆盖仓级
    /// `RUST_MIN_STACK=32 MiB`）；`8 MiB` **仅供对照面**使用——同一送审轮
    /// 场景在宽栈上跑通，证明「2 MiB 的红是栈深所致、且送审轮确实被走到」，
    /// **不是**门禁放宽（门禁永远是 2 MiB 那条）。
    fn run_ws_child_scenario_with_stack(scenario: &str, stack_bytes: usize) {
        if std::env::var_os(WS_PROVIDER_CASE_CHILD_MODE_ENV).is_none() {
            // 常规套件直通：本用例是隔离入口，不是常规断言面。
            return;
        }
        let root_thread = std::thread::current().id();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_stack_size(stack_bytes)
            .enable_all()
            .build()
            .expect("ws child explicit stack runtime");
        let scenario = scenario.to_string();
        let stack_mib = stack_bytes / (1024 * 1024);
        let outcome = rt.block_on(async {
            tokio::spawn(async move {
                // 记录线程 identity：入口必须运行在 rt.spawn worker（显式
                // 栈值），不在 block_on 根线程（设计 §6.2）。
                let entry_thread = std::thread::current().id();
                let entry_thread_name = std::thread::current()
                    .name()
                    .unwrap_or("<unnamed>")
                    .to_string();
                assert_ne!(
                    root_thread, entry_thread,
                    "入口必须运行在 rt.spawn worker，而非 block_on 根线程"
                );
                println!(
                    "[ws-{stack_mib}mib] thread identity: block_on_root={root_thread:?} \
                     entry_worker={entry_thread:?} name={entry_thread_name} stack={stack_mib}MiB"
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
                    "review_round" => super::ws_scenario_review::run_review_round_scenario().await,
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

    /// 送审轮子模式入口（常规套件直通；隔离执行体见
    /// `provider_run_2mib_real_ws_review_round_gate`）。
    #[test]
    fn provider_run_2mib_real_ws_child_review_round() {
        run_ws_child_scenario("review_round");
    }

    /// 送审轮**对照面**子模式入口（8 MiB worker，**非门禁**）：同一场景在宽栈
    /// 上必须跑通并观测到 reviewer 的真实 validated start——用来证明「2 MiB 的
    /// 红确实发生在送审轮、且送审轮真被走到」，排除「场景根本没走到评审轮就
    /// 别处崩了」的假阳性。门禁本身永远是 2 MiB 那条。
    #[test]
    fn provider_run_8mib_ws_review_round_control() {
        run_ws_child_scenario_with_stack("review_round", 8 * 1024 * 1024);
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

    /// C 层送审轮门禁（当前代码下**预期红**）：真实 WS 深链 + 显式 2 MiB
    /// worker 走到 ReviewOnly 送审轮 ⇒ 真机爆栈（SIGABRT / 栈溢出）被隔离器
    /// 捕获并归因「stack overflow」。
    ///
    /// 本用例断言的是**正确行为**（送审轮必须能在 2 MiB worker 上跑完并留下
    /// 四条证据），当前产品未修 ⇒ 红；治本落地后自动转绿。**不得**为了让本
    /// 用例变绿而放宽 worker 栈、跳过送审轮或把「只收到 locked」当通过。
    #[test]
    fn provider_run_2mib_real_ws_review_round_gate() {
        let result = run_isolated_provider_case(
            "provider_run_2mib_real_ws_child_review_round",
            std::time::Duration::from_secs(300),
        );
        assert!(
            result.is_ok(),
            "C 层送审轮门禁未通过（当前代码预期红：送审轮深链在 2 MiB worker 上\
             栈溢出，被隔离器判失败；治本落地后应转绿）: {result:?}"
        );
    }

    /// 送审轮**对照面**（8 MiB worker，**非门禁**）：同一送审轮场景在宽栈上
    /// 必须跑通——由此证明 2 MiB 门禁的红确实落在送审轮（reviewer 的真实
    /// validated start 被观测到、评审轮完成、durable 证据齐备），而不是「场景
    /// 没走到评审轮就崩了」的假阳性。
    ///
    /// 本用例**不**放宽门禁：2 MiB 那条（`provider_run_2mib_real_ws_review_round_gate`）
    /// 才是判据；本条只是对照说明。
    #[test]
    fn provider_run_8mib_ws_review_round_control_gate() {
        let result = run_isolated_provider_case(
            "provider_run_8mib_ws_review_round_control",
            std::time::Duration::from_secs(300),
        );
        assert!(
            result.is_ok(),
            "送审轮对照面（8 MiB worker）未通过——说明场景本身有问题（未真正走到\
             送审轮或评审轮未完成），不能作为 2 MiB 红门的归因依据: {result:?}"
        );
    }
}
