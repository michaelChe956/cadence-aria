//! 从 mod.rs 拆出以满足 large_file_guard 的 1200 行上限（纯移动，无行为变化）。
//! direct/legacy 直调面与 B1 受信归属共用的 raw start 执行体。
use super::*;

impl ClaudeCodeProvider {
    /// raw start 执行体(direct/legacy 直调面与 B1 受信归属共用)。
    ///
    /// `ownership = None`:无归属请求(legacy/单仓 direct/测试面),零登记、
    /// 行为与既有裸 start 一致。
    /// `ownership = Some`:B1-1 受信归属生命周期——预留(durable,失败零
    /// spawn)→ spawn → 回填真实 PID+`/proc` starttime(PID 复用防护)→
    /// 会话真实收口(子进程退出/取消收口完成)后注销;**不在协议
    /// `Completed` 点注销**。
    pub(super) async fn start_inner(
        &self,
        mut input: StreamingProviderInput,
        cancel: CancellationToken,
        ownership: Option<TrustedLaunchOwnership>,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // 双向 spawn 前守卫（Task 3.1）：非法角色×策略组合在创建子进程之前拒绝。
        validate_tool_policy_for_role(&input.role, input.tool_policy.as_ref()).map_err(
            |error| {
                ProviderAdapterError::parse_error(error.to_string(), String::new(), String::new())
            },
        )?;
        // 策略上下文（Task 3.2/3.3，spawn 前）：版本解析（supplier seam 优先，默认
        // 真实 `--version` 探测+进程内缓存，不可得 fail-closed）与 resume 冻结三元组
        // 比对（记录缺失或 digest/version/dialect 任一不一致 → 追加 superseded 终止
        // 审计并新建会话）。
        // GC9：resume 记录缺失与 drift 同路径处置（清除 resume id、全新会话）；
        // 「标记 superseded」仅带内 ToolPolicyWarning（🔴 无旧文件可写，不伪造
        // durable 文件），在事件通道建立后送出。
        let mut superseded_record_missing = false;
        let mut policy_context: Option<(
            std::sync::Arc<dyn ToolPolicyAuditSink>,
            String,
            String,
            String,
        )> = None;
        // 批2 Task 20（硬约束6，codex T13 同构）：run-bound audit sink 在位
        // 即统一 launch 审计合同——岗位纪律会话（Author deny 投影）必须携带
        // sink（缺失 fail-closed 保持）；ReviewerVerify 窗口合同/其余合同
        // run（engine attach 的 sink）同样走握手+provider_start 首行，
        // digest 为窗口域 canonical；无 sink 的普通 Coder 会话零变化。
        // BootstrapExecutor marker 不进本合同（BOOT-04，root recipe receipt
        // 承担）。
        let discipline_policy = input.tool_policy.as_ref().is_some_and(|policy| {
            matches!(
                policy.intent,
                crate::cross_cutting::streaming_provider::ToolPolicyIntent::DenyFileWriteBuiltins
            )
        });
        let audit_contract_sink = if discipline_policy {
            Some(input.audit_sink.clone().ok_or_else(|| {
                ProviderAdapterError::parse_error(
                    "claude policy session: audit sink is required for policy sessions",
                    String::new(),
                    String::new(),
                )
            })?)
        } else {
            input.audit_sink.clone()
        };
        if let Some(sink) = audit_contract_sink {
            let context_label = if discipline_policy {
                "claude policy session"
            } else {
                "claude contract session"
            };
            let provider_version = match self.version_supplier.clone() {
                Some(supplier) => supplier().map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("{context_label}: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?,
                None => crate::cross_cutting::streaming_provider::cached_cli_version(
                    &self.command,
                    probe_claude_version(&self.command, CLAUDE_VERSION_PROBE_TIMEOUT),
                )
                .await
                .map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("{context_label}: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?,
            };
            // 决策项①(T20b P0):不 panic——sink 在位而 wire 策略缺席/
            // marker 形态都是可判定的合同形状:deny/ReviewerVerify →
            // canonical digest(窗口域);None → 无策略指纹(空串,与
            // codex T13「无策略合同 run」口径一致);marker 不进本合同
            // (BOOT-04,root recipe receipt 承担,无三元组)。
            let tool_policy_digest = match input.tool_policy.as_ref() {
                Some(
                    policy @ crate::cross_cutting::streaming_provider::ProviderToolPolicy {
                        intent:
                            crate::cross_cutting::streaming_provider::ToolPolicyIntent::DenyFileWriteBuiltins,
                    },
                )
                | Some(
                    policy @ crate::cross_cutting::streaming_provider::ProviderToolPolicy {
                        intent:
                            crate::cross_cutting::streaming_provider::ToolPolicyIntent::ReviewerVerify { .. },
                    },
                ) => canonical_tool_policy(TOOL_POLICY_PROVIDER_NAME, policy)
                    .map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!("{context_label}: {error}"),
                            String::new(),
                            String::new(),
                        )
                    })?
                    .digest,
                Some(
                    crate::cross_cutting::streaming_provider::ProviderToolPolicy {
                        intent:
                            crate::cross_cutting::streaming_provider::ToolPolicyIntent::BootstrapExecutorMarker(_),
                    },
                ) => {
                    // marker 不是合同会话:sink 若在场仅作透传,无策略指纹。
                    String::new()
                }
                None => String::new(),
            };
            // 批2 Task 20（RevB2T13 ⚠4 同构）：resume 三元组门只服务岗位
            // 纪律会话——非策略合同 run（ReviewerVerify）的连续性由 native
            // 会话承担，不按策略记录门控（不清 id、不误报告警）。
            let resume_id = if discipline_policy {
                input
                    .resume_provider_session_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(ToString::to_string)
            } else {
                None
            };
            if let Some(resume_id) = resume_id.as_ref() {
                let stored = sink.find_provider_start(resume_id).map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("claude policy session: resume lookup failed: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?;
                let current = ProviderStartAudit {
                    workspace_session_id: input.workspace_session_id.clone().unwrap_or_default(),
                    provider_session_id: resume_id.clone(),
                    tool_policy_canonical_digest: tool_policy_digest.clone(),
                    provider_version: provider_version.clone(),
                    adapter_dialect: CLAUDE_POLICY_DIALECT.to_string(),
                    ..ProviderStartAudit::default()
                };
                if let Some(stored) = stored.as_ref()
                    && matches!(
                        crate::cross_cutting::tool_policy_audit::resume_with_audit_record(
                            Some(stored.record.clone()),
                            &current
                        ),
                        crate::cross_cutting::tool_policy_audit::ResumeDecision::RejectSupersedeAndStartNew
                    )
                {
                    // P1-4 裁决：superseded 终止审计写入被取代旧 run 的文件（其
                    // provider_start 已是首行；被终止的是旧会话），新 run 照常从
                    // provider_start 开始。
                    crate::cross_cutting::tool_policy_audit::append_superseded_policy_drift(
                        sink.as_ref(),
                        stored,
                    )
                    .map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!(
                                "claude policy session: superseded audit append failed: {error}"
                            ),
                            String::new(),
                            String::new(),
                        )
                    })?;
                    input.resume_provider_session_id = None;
                } else if stored.is_none() {
                    // GC9：记录缺失与 drift 同路径处置——清除 resume id、以全新会话
                    // （fresh init 握手 + 新 run 的 provider_start）启动；「标记 superseded」
                    // 仅带内 ToolPolicyWarning：无旧文件可写，🔴 不得伪造无
                    // provider_start 首行的 durable 文件（首行不变量优先）。
                    input.resume_provider_session_id = None;
                    superseded_record_missing = true;
                }
            }
            // P1-8：durable 审计 role 保留真实 AdapterRole 序列化值（usage 展示层
            // 归一仅在 read_claude_stream 的 usage 上报使用）。
            let role_text =
                crate::cross_cutting::streaming_provider::adapter_role_text(&input.role)
                    .to_string();
            policy_context = Some((sink, provider_version, tool_policy_digest, role_text));
        }
        let args = self.build_args(
            input.resume_provider_session_id.as_deref(),
            input.tool_policy.as_ref(),
        );
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let command = self.command.to_string_lossy().to_string();
        // B1-1:归属在场时预留先于一切 spawn 副作用(durable;失败即
        // 调用方不得 spawn——本函数直接上抛,零子进程)。
        let reservation = match ownership.as_ref() {
            Some(ownership) => Some(ownership.reserve()?),
            None => None,
        };
        // B1-1 F2/N1:reserve 后常驻守卫——launch future 被引擎在返回前
        // drop(provider_start 取消/idle 停等臂)时,由 Drop 兜底收口注销;
        // 错误路径显式收口后的 Drop 为幂等空操作,尾任务接管后 disarm。
        let mut launch_guard = ownership
            .clone()
            .map(crate::cross_cutting::streaming_provider::LaunchReservationGuard::new);
        let process = match ProcessManager::spawn(
            &command,
            &arg_refs,
            &input.working_dir,
            &input.env_vars,
            cancel.clone(),
        )
        .await
        {
            Ok(process) => process,
            Err(error) => {
                // spawn 失败=零子进程:预留未确认,直接注销(幂等)。
                if let Some(ownership) = ownership.as_ref() {
                    ownership.release_after_session_tail().await;
                }
                return Err(error);
            }
        };
        // B1-1:spawn 点回填真实 PID+starttime(PID 复用防护);身份不可证
        // 即整组停止,不留未登记子进程。
        if let (Some(ownership), Some(reservation)) = (ownership.as_ref(), reservation.as_ref())
            && let Err(error) = backfill_launch_identity(ownership, reservation, &process.child)
        {
            let mut child = process.child;
            let _ = child.start_kill();
            let _ = child.wait().await;
            ownership.release_after_session_tail().await;
            return Err(error);
        }

        let stdin = Arc::new(Mutex::new(process.stdin));
        let stdout = process.stdout;
        let stderr = process.stderr;
        let mut child = process.child;
        let (event_tx, event_rx) = mpsc::channel(32);
        let bridge = ApprovalBridge::new(input.permission_mode.clone(), event_tx.clone());
        let commands = bridge.command_sender();
        let structured_output_contract = input.structured_output_contract.clone();

        let resume_native_id = if input.tool_policy.is_some() {
            input
                .resume_provider_session_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string)
        } else {
            None
        };

        let _ = event_tx
            .send(ProviderEvent::StatusChanged(ProviderStatus::Starting))
            .await;
        let _ = event_tx
            .send(ProviderEvent::Execution(ProviderExecutionEvent {
                event_id: "provider".to_string(),
                kind: ProviderExecutionEventKind::Provider,
                status: ProviderExecutionEventStatus::Started,
                title: "Claude Code provider started".to_string(),
                detail: None,
                command: None,
                cwd: Some(input.working_dir.display().to_string()),
                output: None,
                exit_code: None,
            }))
            .await;

        // GC9：resume 记录缺失的带内 superseded 标记（先于新会话 provider_start，
        // 与 drift 路径的 durable 顺序镜像）。
        if superseded_record_missing {
            let _ = event_tx
                .send(crate::cross_cutting::streaming_provider::superseded_policy_record_missing_warning())
                .await;
        }

        // workspace 会话 id（D6 冻结字段）在 input 移入后台任务前捕获，供
        // provider_start 审计落盘使用。
        let workspace_session_id = input.workspace_session_id.clone().unwrap_or_default();

        // 策略会话（P1-7 round 2 裁决）：「子进程所有权+初始写入+握手+provider_start
        // 写入」留在 start() 内有界完成（握手出后台任务的重构即要求本身）；成功后
        // 才把 child 移交会话收尾任务；任一失败由 start() 直接持有 child 同步
        // kill()+wait()（exit status 回收）后返回错误（对齐 codex/pi 先例）；
        // 有界 await 仅作 stderr 任务善后。事件通道随会话未返回而消亡，失败路径
        // 不再投递死信事件。
        if let Some((sink, provider_version, tool_policy_digest, role_text)) = policy_context {
            let stderr_output = Arc::new(Mutex::new(String::new()));
            let stderr_output_for_task = Arc::clone(&stderr_output);
            let stderr_task = tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut output = stderr_output_for_task.lock().await;
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&line);
                }
            });

            // 有界完成「初始写入→Running→握手→provider_start」：初始 stdin 写不可
            // 取消（不 select cancel，如 task.rs write_json_line 的 write_all），
            // 以外层超时放弃 future 并走 kill 链。
            let bound = stream::CLAUDE_POLICY_HANDSHAKE_TIMEOUT.saturating_mul(3);
            let outcome = tokio::time::timeout(bound, async {
                Self::write_initial_messages(&stdin, &input)
                    .await
                    .map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!(
                                "claude policy session: initial write failed: {}",
                                error.details
                            ),
                            String::new(),
                            String::new(),
                        )
                    })?;
                let _ = event_tx
                    .send(ProviderEvent::StatusChanged(ProviderStatus::Running))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Execution(ProviderExecutionEvent {
                        event_id: "turn".to_string(),
                        kind: ProviderExecutionEventKind::Turn,
                        status: ProviderExecutionEventStatus::Started,
                        title: "Turn started".to_string(),
                        detail: None,
                        command: None,
                        cwd: Some(input.working_dir.display().to_string()),
                        output: None,
                        exit_code: None,
                    }))
                    .await;
                let (reader, native_id) = match resume_native_id.clone() {
                    // resume 已知：native id 即 resume id，不等 init。
                    Some(id) => (tokio::io::BufReader::new(stdout), id),
                    None => {
                        let (reader, session_id) = stream::wait_for_claude_init(stdout, &cancel)
                            .await
                            .map_err(|error| {
                                ProviderAdapterError::parse_error(
                                    format!(
                                        "claude policy session: handshake failed: {}",
                                        error.details
                                    ),
                                    String::new(),
                                    String::new(),
                                )
                            })?;
                        (reader, session_id)
                    }
                };
                let audit_event = DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
                    provider: TOOL_POLICY_PROVIDER_NAME.to_string(),
                    role: role_text.clone(),
                    workspace_session_id: workspace_session_id.clone(),
                    provider_session_id: native_id.clone(),
                    tool_policy_canonical_digest: tool_policy_digest.clone(),
                    argv: args.clone(),
                    sandbox: None,
                    approval_policy: None,
                    provider_version: provider_version.clone(),
                    adapter_dialect: CLAUDE_POLICY_DIALECT.to_string(),
                    // direct 策略会话:无 LC 投影(仅 LC validated 启动落盘)。
                    lc_projection: None,
                });
                // F2（最终审）：审计写入前再验 provider_session_id 非空——空白
                // 原生会话 id 不得进入 durable 审计（fresh/resume 两路同验）。
                if native_id.trim().is_empty() {
                    return Err(ProviderAdapterError::parse_error(
                        "claude policy session: native session id is blank before audit write",
                        String::new(),
                        String::new(),
                    ));
                }
                sink.append_bound(audit_event).map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!(
                            "claude policy session: provider_start audit append failed: {error}"
                        ),
                        String::new(),
                        String::new(),
                    )
                })?;
                Ok::<_, ProviderAdapterError>((reader, native_id))
            })
            .await;

            let (reader, native_id) = match outcome {
                Ok(Ok(prepared)) => prepared,
                Ok(Err(mut error)) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    let _ = tokio::time::timeout(bound, stderr_task).await;
                    // D③（诊断强化）：失败附 stderr——claude 子进程死因可见，
                    // 有界快照（≤2000B）同时并入 details 与 stderr 字段。
                    let stderr_snapshot = Self::bounded_stderr_snapshot(&stderr_output).await;
                    if !stderr_snapshot.is_empty() {
                        if error.stderr.is_empty() {
                            error.stderr = stderr_snapshot.clone();
                        }
                        error.details.push_str(&format!(
                            "\nclaude stderr (last {} bytes): {stderr_snapshot}",
                            stderr_snapshot.len()
                        ));
                    }
                    // B1-1:错误链已 kill+wait 子进程;按登记身份收口后注销。
                    if let Some(ownership) = ownership.as_ref() {
                        ownership.release_after_session_tail().await;
                    }
                    return Err(error);
                }
                Err(_elapsed) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    let _ = tokio::time::timeout(bound, stderr_task).await;
                    // D③：同上——超时路径也携带 stderr 快照。
                    let stderr_snapshot = Self::bounded_stderr_snapshot(&stderr_output).await;
                    let details = if stderr_snapshot.is_empty() {
                        "provider command timed out".to_string()
                    } else {
                        format!(
                            "provider command timed out\nclaude stderr (last {} bytes): {stderr_snapshot}",
                            stderr_snapshot.len()
                        )
                    };
                    // B1-1:超时路径同样先收口(kill+wait 已在上方完成)再注销。
                    if let Some(ownership) = ownership.as_ref() {
                        ownership.release_after_session_tail().await;
                    }
                    return Err(ProviderAdapterError::timeout_with_details(
                        details,
                        String::new(),
                        stderr_snapshot,
                        stream::CLAUDE_POLICY_HANDSHAKE_TIMEOUT.as_millis() as u64,
                    ));
                }
            };
            // 成功：child 与续读 reader 移交会话收尾任务（流读取+终态处理）；
            // B1-1 归属随任务走，真实收口后注销。
            let usage_role = UsageReportData::role_text(&input.role);
            tokio::spawn(async move {
                run_claude_session_tail(
                    reader,
                    stdin,
                    bridge,
                    event_tx,
                    cancel,
                    structured_output_contract,
                    usage_role,
                    child,
                    stderr_output,
                    stderr_task,
                    ownership,
                )
                .await;
            });
            // 尾任务已接管注销:守卫退出舞台(此后 Drop 不触碰登记)。
            if let Some(guard) = launch_guard.as_mut() {
                guard.disarm();
            }
            return Ok(ProviderSession {
                native_session_id: Some(native_id),
                events: event_rx,
                commands,
            });
        }

        // 非策略/Coder 路径：既有行为零变化——初始写入与流读取由会话任务完成，
        // 失败终止与终态事件沿用任务内 kill 链（child 归任务持有）。
        tokio::spawn(async move {
            let stderr_output = Arc::new(Mutex::new(String::new()));
            let stderr_output_for_task = Arc::clone(&stderr_output);
            let stderr_task = tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut output = stderr_output_for_task.lock().await;
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&line);
                }
            });

            if let Err(error) = Self::write_initial_messages(&stdin, &input).await {
                let _ = child.start_kill();
                let status = child.wait().await;
                let _ = stderr_task.await;
                let stderr = tool::combine_stderr(stderr_output.lock().await.clone(), error.stderr);
                let _ = event_tx
                    .send(ProviderEvent::StatusChanged(ProviderStatus::Failed))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Execution(ProviderExecutionEvent {
                        event_id: "provider".to_string(),
                        kind: ProviderExecutionEventKind::Provider,
                        status: ProviderExecutionEventStatus::Failed,
                        title: "Claude Code provider failed".to_string(),
                        detail: Some(error.details),
                        command: None,
                        cwd: None,
                        output: if stderr.trim().is_empty() {
                            None
                        } else {
                            Some(stderr.clone())
                        },
                        exit_code: None,
                    }))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Failed {
                        message: tool::format_exit_failure(status, stderr),
                    })
                    .await;
                // B1-1:子进程已 kill+wait;按登记身份收口后注销。
                if let Some(ownership) = ownership.as_ref() {
                    ownership.release_after_session_tail().await;
                }
                return;
            }
            let _ = event_tx
                .send(ProviderEvent::StatusChanged(ProviderStatus::Running))
                .await;
            let _ = event_tx
                .send(ProviderEvent::Execution(ProviderExecutionEvent {
                    event_id: "turn".to_string(),
                    kind: ProviderExecutionEventKind::Turn,
                    status: ProviderExecutionEventStatus::Started,
                    title: "Turn started".to_string(),
                    detail: None,
                    command: None,
                    cwd: Some(input.working_dir.display().to_string()),
                    output: None,
                    exit_code: None,
                }))
                .await;

            // 非策略路径：无策略握手，直接续读流；终态处理交共用收尾函数。
            run_claude_session_tail(
                tokio::io::BufReader::new(stdout),
                stdin,
                bridge,
                event_tx,
                cancel,
                structured_output_contract,
                UsageReportData::role_text(&input.role),
                child,
                stderr_output,
                stderr_task,
                ownership,
            )
            .await;
        });
        // 尾任务已接管注销:守卫退出舞台(此后 Drop 不触碰登记)。
        if let Some(guard) = launch_guard.as_mut() {
            guard.disarm();
        }

        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands,
        })
    }
}
