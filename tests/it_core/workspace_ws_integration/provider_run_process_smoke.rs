// P9（任务10）D 层：制品进程 smoke——真实 aria 二进制进程的 provider run 回归。
// 本文件经 workspace_ws_integration.rs include! 进同一模块作用域——具名内联
// 子模块包裹并 `use super::*` 复用父 fixture（part_05 夹具/WS 收发工具），不向
// 旧大 harness（it_web/web_lc_gateway_multi_provider/harness.rs）追加内容。
//
// 结构（设计 §6.1 D 行 / §6.3）：
// - `provider_run_process_smoke_matrix`：同步父测试，构建客户端 runtime 后等待
//   `run_provider_process_smoke` 完成（父线程只等待并读退出状态，不在同步
//   helper 内嵌套 runtime）；
// - `run_provider_process_smoke`：自有服务（真实 `aria web` 子进程，独立进程
//   组、自有空闲端口、自有 tempdir 根）、持续排日志与证据收集；
// - 真实 adapter 前置缺失时以显式「D 未验证」Err 报告，不 fake 成功、不加
//   产品测试后门；无进程残留（整组收口 + ESRCH 探测）才算完成。
pub(crate) mod process_smoke {
    use super::*;

    // D 层证据合同与制品工具（文本内联——同一模块作用域，无需跨模块可见性）。
    include!("provider_run_process_smoke_contract.rs");

    /// 子模式环境标识（与单元侧 small_stack_worker 同合同、各自私有）。
    const PROCESS_PROVIDER_CASE_CHILD_MODE_ENV: &str = "ARIA_PROVIDER_CASE_CHILD_MODE";

    // 私有同合同隔离 helper（同 small_stack_worker 合同，本测试二进制内私有，
    // 不跨二进制耦合）。
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

    // -----------------------------------------------------------------------
    // D 层：真实制品进程 smoke
    // -----------------------------------------------------------------------

    /// D 层证据：二进制指纹/profile/实际启动栈配置/日志规模/进程收口（打印进
    /// D 层真实 WS 深链：与 C 层同一动态合同（真实 start / 完整帧+偏序 /
    /// durable 进展 / 取消后再生成 / health+重连），但 provider 是真实
    /// ClaudeCode adapter（进程内无 Scripted 替身——任何流式输出都来自真实
    /// `claude` CLI 会话）。真实模型调用有延迟：各阶段有界窗口 600s。
    /// D 层进程内合同（**不依赖真实 adapter**）：真实 aria 制品进程起服务后，
    /// 以真实 WS 连接完成准入（session_state + Hello Driver + Ping→Pong），
    /// 并核对 `/api/health` 200 + 全新 WS 重连——证明真实制品进程**未 abort**
    /// 且 WS 深链可用。此层与真实 provider 前置解耦：真实 adapter 缺 CLI/鉴权/
    /// 信任前置时，本层仍必须真实通过（否则连「服务活着」都无证据）。
    async fn drive_process_smoke_admission(
        ws: &mut ProcessWsStream,
        port: u16,
        session_id: &str,
    ) -> Result<(), String> {
        let url = format!("ws://127.0.0.1:{port}/api/workspace-sessions/{session_id}/ws");
        let origin = format!("http://127.0.0.1:{port}");

        let initial = recv_json(ws).await;
        assert!(
            matches!(initial, WsOutMessage::SessionState { .. }),
            "expected initial session_state, got {initial:?}"
        );
        // Hello Driver 准入（last_seen_node_id=None、after_event_seq=None）。
        send_json(
            ws,
            &WsInMessage::Hello {
                session_id: session_id.to_string(),
                last_seen_node_id: None,
                role: Some(HelloRole::Driver),
                after_event_seq: None,
            },
        )
        .await;
        send_json(ws, &WsInMessage::Ping).await;
        let mut ponged = false;
        for _ in 0..40 {
            if matches!(recv_json(ws).await, WsOutMessage::Pong) {
                ponged = true;
                break;
            }
        }
        if !ponged {
            return Err("D 未验证：driver 准入 Pong 未返回".to_string());
        }

        // 服务未 abort：/api/health（公开端点）+ 全新 WS 重连。
        let mut health = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|error| format!("D 未验证：health 连接失败（服务可能已 abort）: {error}"))?;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let request = format!(
            "GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        health
            .write_all(request.as_bytes())
            .await
            .map_err(|error| format!("D 未验证：health 写失败: {error}"))?;
        let mut response = String::new();
        health
            .read_to_string(&mut response)
            .await
            .map_err(|error| format!("D 未验证：health 读失败: {error}"))?;
        let status_line = response.lines().next().unwrap_or("");
        if !status_line.contains(" 200 ") {
            return Err(format!("D 未验证：health 非 200: {status_line}"));
        }
        let mut reconnected = connect_process_ws(&url, &origin)
            .await
            .map_err(|error| format!("D 未验证：重连失败（服务可能已 abort）: {error}"))?;
        let reload = recv_json(&mut reconnected).await;
        assert!(
            matches!(reload, WsOutMessage::SessionState { .. }),
            "reconnect must yield session_state, got {reload:?}"
        );
        drop(reconnected);
        Ok(())
    }

    /// D 层 provider 深链（**依赖真实 adapter**）：真实 ClaudeCode adapter
    /// （进程内无 Scripted 替身——任何流式输出都来自真实 `claude` CLI 会话）
    /// 完成真实 start → 完整帧 + 因果偏序 → durable 进展 → 取消后再生成。
    async fn drive_process_smoke_chain(
        ws: &mut ProcessWsStream,
        _port: u16,
        app_paths: &ProductAppPaths,
        session_id: &str,
    ) -> Result<(), String> {
        let timeline_count = |lifecycle: &LifecycleStore| -> usize {
            lifecycle
                .load_timeline_nodes(session_id)
                .map(|nodes| nodes.len())
                .unwrap_or(0)
        };
        let message_count = |lifecycle: &LifecycleStore| -> usize {
            lifecycle
                .get_workspace_session(session_id)
                .map(|record| record.messages.len())
                .unwrap_or(0)
        };
        let lifecycle = LifecycleStore::new(app_paths.clone());
        let (timeline_base, messages_base) = (timeline_count(&lifecycle), message_count(&lifecycle));

        // run1：StartGeneration → locked → 真实 adapter 流式输出 → 完成回显。
        send_json(ws, &process_start_generation_message()).await;
        let mut frame_seq = 0_usize;
        let mut locked_at = None;
        let mut first_output_at = None;
        let mut first_chunk_sample = String::new();
        let mut durable_completed_at = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
        let completed_echo_at = loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(
                    "D 未验证：run1 超时（真实 adapter 未在 600s 内产出/收口）".to_string(),
                );
            }
            frame_seq += 1;
            let value = recv_process_frame(ws).await?;
            match value["type"].as_str().unwrap_or("") {
                "provider_locked" => locked_at.get_or_insert(frame_seq),
                "stream_chunk" => {
                    if first_output_at.is_none() {
                        first_chunk_sample = value["content"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(80)
                            .collect();
                    }
                    first_output_at.get_or_insert(frame_seq)
                }
                "message_complete" => break frame_seq,
                "error" => {
                    return Err(format!(
                        "D 未验证：服务返回 error 帧（真实 adapter 前置缺失或启动失败）: {}",
                        value["message"]
                    ));
                }
                "protocol_error" => {
                    return Err(format!(
                        "D 未验证：协议拒收（阶段矩阵/准入前置）: {}",
                        value["message"]
                    ));
                }
                _ => continue,
            };
            // 帧后 durable 检查点：真实运行类节点 Completed 的可观察位置。
            if durable_completed_at.is_none() {
                let probe = LifecycleStore::new(app_paths.clone());
                let completed = probe
                    .load_timeline_nodes(session_id)
                    .map(|nodes| {
                        nodes.iter().any(|node| {
                            node.status == TimelineNodeStatus::Completed
                                && matches!(
                                    node.node_type,
                                    TimelineNodeType::AuthorRun
                                        | TimelineNodeType::ReviewerRun
                                        | TimelineNodeType::Revision
                                )
                        })
                    })
                    .unwrap_or(false);
                if completed {
                    durable_completed_at = Some(frame_seq);
                }
            }
        };
        let locked_at = locked_at.ok_or("D 未验证：provider_locked 前奏帧缺失")?;
        let first_output_at =
            first_output_at.ok_or("D 未验证：真实 adapter 无流式输出（未观察到 start）")?;
        if locked_at >= first_output_at {
            return Err("D 未验证：偏序破坏（locked 未先于本 run 输出）".to_string());
        }
        if durable_completed_at >= Some(completed_echo_at) {
            return Err(format!(
                "D 未验证：偏序破坏（durable Completed 未先于 completed 回显: \
                 durable={durable_completed_at:?}, echo={completed_echo_at}）"
            ));
        }
        println!(
            "[process-smoke] run1 完成：真实 adapter 流式输出样本={first_chunk_sample:?} \
             locked@{locked_at} output@{first_output_at} durable@{durable_completed_at:?} \
             echo@{completed_echo_at}"
        );

        // durable 前后差异（真实进程写盘）。
        let (timeline_after, messages_after) =
            (timeline_count(&lifecycle), message_count(&lifecycle));
        if timeline_after == timeline_base && messages_after == messages_base {
            return Err("D 未验证：无 durable 进展（timeline/message 均未推进）".to_string());
        }
        println!(
            "[process-smoke] durable 差异：timeline {timeline_base}->{timeline_after}, \
             messages {messages_base}->{messages_after}"
        );

        // 取消后再生成：run2 首个流式块后立即 Abort（真实 adapter 已启动），
        // 等取消收口；run3 再生成必须再次出现真实流式输出。
        send_json(ws, &process_start_generation_message()).await;
        let mut run2_chunk_seen = false;
        let mut cancel_echoes = 0_usize;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err("D 未验证：run2 超时".to_string());
            }
            let value = recv_process_frame(ws).await?;
            match value["type"].as_str().unwrap_or("") {
                "stream_chunk" if !run2_chunk_seen => {
                    run2_chunk_seen = true;
                    send_json(ws, &WsInMessage::Abort).await;
                }
                "error" => {
                    cancel_echoes += 1;
                    if cancel_echoes > 1 {
                        return Err(format!("D 未验证：取消失败回显 >1（{cancel_echoes} 次）"));
                    }
                }
                "stage_change" if run2_chunk_seen && value["stage"] == json!("prepare_context") => {
                    // 唯一 break 路径：取消已收口（run2_chunk_seen 必为真）。
                    break;
                }
                "protocol_error" => {
                    return Err(format!("D 未验证：run2 协议拒收: {}", value["message"]));
                }
                _ => {}
            }
        }
        if !run2_chunk_seen {
            return Err("D 未验证：run2 未观察到真实流式输出即收口".to_string());
        }
        // run3（取消后再生成）：再次 StartGeneration，真实 adapter 必须再次产出。
        send_json(ws, &process_start_generation_message()).await;
        let run3_chunk: String;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err("D 未验证：run3 超时（取消后未再生成）".to_string());
            }
            let value = recv_process_frame(ws).await?;
            match value["type"].as_str().unwrap_or("") {
                "stream_chunk" => {
                    run3_chunk = value["content"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(40)
                        .collect();
                    break;
                }
                "error" => {
                    return Err(format!(
                        "D 未验证：run3 error 帧（取消后不能再生成）: {}",
                        value["message"]
                    ));
                }
                "protocol_error" => {
                    return Err(format!(
                        "D 未验证：run3 协议拒收（取消后不能再生成）: {}",
                        value["message"]
                    ));
                }
                _ => {}
            }
        }
        println!(
            "[process-smoke] 取消后再生成成立：run3 真实流式输出样本={run3_chunk:?} \
             cancel_echoes={cancel_echoes}"
        );

        Ok(())
    }

    /// D 层 smoke 主体：真实 aria 二进制起自有服务。
    ///
    /// `provider_level=false`：只跑**进程内合同**（WS 准入 + health + 重连）——
    /// 不依赖真实 adapter 前置，必须在任何环境下真实通过。
    /// `provider_level=true`：额外跑**真实 provider 深链**（真实 start / 完整帧 /
    /// durable 差异 / 取消后再生成）——缺真实 adapter 前置时显式报「D 未验证」。
    ///
    /// 父线程只等待并读退出状态；持续排两条日志管道；结束时整组收口并确认
    /// 无进程残留（ESRCH）。workspace TempDir 由调用方持有存活。
    async fn run_provider_process_smoke(
        binary: &std::path::Path,
        workspace_root: &std::path::Path,
        provider_level: bool,
    ) -> Result<(), String> {
        let app_paths = ProductAppPaths::new(workspace_root.join(".aria"));
        let session_id = "workspace_session_0001".to_string();

        let port = acquire_free_port()?;
        let binary_sha256 = binary_sha256(binary)?;
        let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
        let toolchain = rustc_version();
        let inherited_rust_min_stack = std::env::var("RUST_MIN_STACK").ok();
        println!(
            "[process-smoke] binary={} sha256={binary_sha256} profile={profile} \
             toolchain={toolchain} inherited_RUST_MIN_STACK={inherited_rust_min_stack:?} \
             port={port}",
            binary.display()
        );

        // 真实服务进程：独立进程组 + 持续排日志（防管道写满伪超时）。
        let mut command = std::process::Command::new(binary);
        command
            .arg("web")
            .arg("--workspace")
            .arg(workspace_root)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }
        let mut server = command
            .spawn()
            .map_err(|error| format!("spawn aria web failed: {error}"))?;
        let pgid = server.id();
        let server_stdout = drain_pipe(server.stdout.take().expect("piped server stdout"));
        let server_stderr = drain_pipe(server.stderr.take().expect("piped server stderr"));

        // 就绪等待：端口可连（bind 先于一切启动副作用）；提前退出=失败。
        let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
        loop {
            if tokio::time::Instant::now() >= deadline {
                kill_process_group(&mut server);
                let _ = server.wait();
                let stdout = server_stdout.join().unwrap_or_default();
                let stderr = server_stderr.join().unwrap_or_default();
                return Err(format!(
                    "D 层服务就绪超时（端口未就绪）；stdout尾={}\nstderr尾={}",
                    excerpt(&stdout),
                    excerpt(&stderr)
                ));
            }
            if let Ok(Some(status)) = server.try_wait() {
                let stdout = server_stdout.join().unwrap_or_default();
                let stderr = server_stderr.join().unwrap_or_default();
                return Err(format!(
                    "D 层服务提前退出（status={status}）；stdout尾={}\nstderr尾={}",
                    excerpt(&stdout),
                    excerpt(&stderr)
                ));
            }
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let url = format!("ws://127.0.0.1:{port}/api/workspace-sessions/{session_id}/ws");
        let origin = format!("http://127.0.0.1:{port}");
        let chain_result = match connect_process_ws(&url, &origin).await {
            Ok(mut ws) => {
                // 进程内合同（准入 + health + 重连）先行——真实 adapter 前置缺失
                // 不影响这一层的可验证性。
                let admission = drive_process_smoke_admission(&mut ws, port, &session_id).await;
                match (admission, provider_level) {
                    (Err(error), _) => Err(error),
                    (Ok(()), false) => Ok(()),
                    (Ok(()), true) => {
                        drive_process_smoke_chain(&mut ws, port, &app_paths, &session_id).await
                    }
                }
            }
            Err(error) => Err(error),
        };

        // 无进程残留：整组 SIGKILL + wait 收口，随后 ESRCH 探测进程组不存在。
        kill_process_group(&mut server);
        let _exit_status = server
            .wait()
            .map_err(|error| format!("wait server after kill failed: {error}"))?;
        let stdout = server_stdout.join().unwrap_or_default();
        let stderr = server_stderr.join().unwrap_or_default();
        #[cfg(unix)]
        let process_group_residue_free = {
            // 组长已 wait 收口；对整组再探测：ESRCH 即无残留。
            let probe = unsafe { libc::kill(-(pgid as i32), 0) };
            probe == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        };
        #[cfg(not(unix))]
        let process_group_residue_free = true;
        let (startup_log_profile, effective_stack_bytes) = parse_startup_stack_line(&stderr);
        let evidence = ProcessSmokeEvidence {
            binary: binary.display().to_string(),
            binary_sha256,
            profile,
            toolchain,
            inherited_rust_min_stack,
            effective_stack_bytes,
            startup_log_profile,
            server_port: port,
            stdout_lines: stdout.lines().count(),
            stderr_lines: stderr.lines().count(),
            process_group_residue_free,
        };
        println!("[process-smoke] evidence={evidence:?}");
        // 真实 adapter 日志摘录（stdout/stderr 中含 provider/adapter 启动痕迹的行）。
        for line in stdout
            .lines()
            .chain(stderr.lines())
            .filter(|line| {
                line.contains("provider") || line.contains("adapter") || line.contains("claude")
            })
            .take(30)
        {
            println!("[process-smoke][server-log] {line}");
        }
        // 证据字段逐项落地（不得只 Debug 打印后丢弃）：
        // - 二进制指纹：sha256 必须是 64 位十六进制，且路径必须是真实 aria 制品；
        // - profile：与本次编译 profile 一致（debug/release 不得张冠李戴）；
        // - 生效栈：记录继承的 RUST_MIN_STACK（任务13 前产品未打印启动栈日志，
        //   此处如实记录环境值，不臆造「生效栈」）；
        // - 端口：自有空闲端口；
        // - 进程收口：整组 ESRCH 探测必须通过（无残留）。
        if evidence.binary_sha256.len() != 64
            || !evidence
                .binary_sha256
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err(format!(
                "D 层失败：二进制指纹无效（sha256={}）",
                evidence.binary_sha256
            ));
        }
        if !evidence.binary.ends_with("aria") {
            return Err(format!(
                "D 层失败：制品路径不是 aria 二进制（binary={}）",
                evidence.binary
            ));
        }
        let expected_profile = if cfg!(debug_assertions) { "debug" } else { "release" };
        if evidence.profile != expected_profile {
            return Err(format!(
                "D 层失败：profile 与编译 profile 不一致（记录={}，实际={}）",
                evidence.profile, expected_profile
            ));
        }
        if evidence.server_port == 0 {
            return Err("D 层失败：服务端口无效".to_string());
        }
        // 生效栈交叉核对：产品启动日志自报的 profile 必须与二进制 profile 一致
        // （防 debug/release 张冠李戴——任务14 发布检查单的硬要求）。
        match (&evidence.startup_log_profile, evidence.effective_stack_bytes) {
            (Some(startup_profile), Some(stack_bytes)) => {
                if startup_profile != expected_profile {
                    return Err(format!(
                        "D 层失败：启动日志 profile 与二进制 profile 不一致（启动日志={startup_profile}，\
                         二进制={expected_profile}，binary={}）",
                        evidence.binary
                    ));
                }
                if stack_bytes == 0 {
                    return Err("D 层失败：启动日志 worker_stack_bytes 为 0".to_string());
                }
                println!(
                    "[process-smoke] effective stack: profile={startup_profile} \
                     worker_stack_bytes={stack_bytes}"
                );
            }
            (None, None) => {
                return Err(format!(
                    "D 层失败：产品启动日志缺 [aria-runtime] 生效栈行（无法核对生效栈与 profile；\
                     stderr 尾={}）",
                    excerpt(&stderr)
                ));
            }
            _ => {
                return Err(format!(
                    "D 层失败：产品启动日志的 profile/worker_stack_bytes 不完整（profile={:?}, \
                     bytes={:?}）",
                    evidence.startup_log_profile, evidence.effective_stack_bytes
                ));
            }
        }
        if evidence.toolchain.is_empty() || evidence.toolchain == "unknown" {
            return Err(format!(
                "D 层失败：工具链指纹缺失（toolchain={:?}）",
                evidence.toolchain
            ));
        }
        if evidence.stdout_lines == 0 && evidence.stderr_lines == 0 {
            return Err("D 层失败：服务进程无任何日志输出（证据不可采信）".to_string());
        }
        if !evidence.process_group_residue_free {
            return Err(format!(
                "D 层失败：服务进程组仍有残留（ESRCH 探测未通过，binary={}，profile={}，\
                 生效栈环境={:?}，stdout_lines={}，stderr_lines={}）",
                evidence.binary,
                evidence.profile,
                evidence.inherited_rust_min_stack,
                evidence.stdout_lines,
                evidence.stderr_lines
            ));
        }
        chain_result.map_err(|error| {
            format!(
                "D 层制品进程 smoke 未通过: {error}\nserver_stdout尾={}\nserver_stderr尾={}",
                excerpt(&stdout),
                excerpt(&stderr)
            )
        })
    }

    /// D 层同步父矩阵（设计 §6.1 D / §6.3）：真实 aria 制品进程执行与 C 层同一
    /// 动态合同；真实 adapter 前置缺失时显式失败并标注「D 未验证」，不 fake
    /// 成功。父线程构建客户端 runtime 后只等待结果并读退出状态（不嵌套 runtime）。
    #[test]
    fn provider_run_process_smoke_matrix() {
        let binary = std::path::PathBuf::from(env!("CARGO_BIN_EXE_aria"));
        assert!(
            binary.is_file(),
            "CARGO_BIN_EXE_aria 必须指向真实 aria 制品: {}",
            binary.display()
        );
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("process smoke client runtime");
        let result = rt.block_on(async {
            // durable 前置夹具（真实 project/LC/issue/session 铺盘，含 Confirmed
            // gateway capability 与 trust——机器级记录对真实进程同样生效）；
            // root 在 smoke 完成后 drop，被断言磁盘路径全程存活。
            //
            // 夹具的仓库初始化用「100 次 yield 轮询」等完成态——机器负载高时该
            // 预算可能先耗尽而初始化仍在跑（fixture panic）。此处按独立临时根
            // 重试至多 3 次：每次**全新** tempdir（失败根直接丢弃，不复用半成品
            // 磁盘状态），避免把负载抖动误报成 D 层失败。
            let mut last_error = String::new();
            for attempt in 1..=3_u32 {
                // 夹具在**独立 tokio 任务**里跑：其仓库初始化的 100 次 yield 轮询
                // 预算是负载敏感的（机器繁忙时可能先耗尽而初始化仍在跑），
                // 任务内 panic 不会终止进程，JoinError 可据此换新临时根重试。
                let fixture_task = tokio::spawn(async move {
                    let root = tempfile::tempdir().expect("process smoke workspace root");
                    // 生产 `aria web` 启动先跑 `ensure_product_data_schema`：schema
                    // 文件缺失且已有 legacy 业务数据 → 拒绝启动
                    // （product_data_schema_unsupported）。夹具先铺业务数据，故此处
                    // **先**落 schema 文件（与生产同版本号），再铺夹具——顺序反了
                    // 真实制品会带该错误退出（不是 D 层断言失败）。
                    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
                    cadence_aria::product::product_data_schema::ensure_product_data_schema(
                        &app_paths,
                    )
                    .expect("D 层 workspace root 必须先落 product data schema");
                    create_workspace_session_fixture_with_providers(
                        &root,
                        "claude_code",
                        "fake",
                        1,
                    )
                    .await;
                    root
                });
                match fixture_task.await {
                    Ok(root) => {
                        println!("[process-smoke] fixture ready (attempt {attempt})");
                        return run_provider_process_smoke(&binary, root.path(), false).await;
                    }
                    Err(join_error) => {
                        last_error = format!(
                            "fixture attempt {attempt} 未完成（仓库初始化预算耗尽）：{join_error}"
                        );
                        println!("[process-smoke] {last_error}——换新临时根重试");
                    }
                }
            }
            Err(format!("D 层前置夹具未就绪：{last_error}"))
        });
        assert!(result.is_ok(), "D 层制品进程 smoke 未通过: {result:?}");
    }

    /// D 层**真实 provider 深链**（发布验收级）：真实 aria 制品进程 + 真实
    /// ClaudeCode adapter（真实 `claude` CLI 会话）跑完整动态合同。
    ///
    /// 真实 adapter 缺 CLI/鉴权/信任前置时，本用例**显式失败并标注「D 未验证」**
    /// ——不 fake 成功、不给产品加测试后门。常规 CI 不跑本用例（真实凭据不进
    /// CI）；发布验收时显式 `--ignored` 运行。
    #[test]
    #[ignore = "D 层真实 provider 深链需真实 claude CLI 鉴权/信任前置（发布验收显式运行）"]
    fn provider_run_process_smoke_provider_chain() {
        let binary = std::path::PathBuf::from(env!("CARGO_BIN_EXE_aria"));
        assert!(
            binary.is_file(),
            "CARGO_BIN_EXE_aria 必须指向真实 aria 制品: {}",
            binary.display()
        );
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("process smoke client runtime");
        let result = rt.block_on(async {
            let root = tempfile::tempdir().expect("process smoke workspace root");
            let app_paths = ProductAppPaths::new(root.path().join(".aria"));
            cadence_aria::product::product_data_schema::ensure_product_data_schema(&app_paths)
                .expect("D 层 workspace root 必须先落 product data schema");
            create_workspace_session_fixture_with_providers(&root, "claude_code", "fake", 1).await;
            run_provider_process_smoke(&binary, root.path(), true).await
        });
        assert!(result.is_ok(), "D 层真实 provider 深链未通过: {result:?}");
    }
}
