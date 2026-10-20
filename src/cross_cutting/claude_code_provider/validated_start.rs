//! 从 mod.rs 拆出以满足 large_file_guard 的 1200 行上限（纯移动，无行为变化）。
//! LC validated 启动与 prepare_lc_facts 执行体（B1 受信归属在 spawn 点接线）。
use super::*;

impl ClaudeCodeProvider {
    /// LC validated 启动(Task 4a):只接受 gateway 产出的 validated input,
    /// 消费冻结 envelope 派生的不可伪造 boundary plan,投影后以 canonical
    /// LC root 为进程 cwd 启动;统一落盘 `ProviderStartAudit.lc_projection`。
    /// 无通用 tool policy 的角色(Coding/Executor)同样执行 version 解析、
    /// 原生握手与统一 launch audit,不以 `tool_policy=None` 早退。
    pub(super) async fn start_validated_inner(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // B1-1:受信归属在拆包前取出(载体随 validated 输入的 crate 私有
        // 绑定进入本 spawn 面;`None` = 无归属请求)。
        let ownership = validated.launch_ownership().cloned();
        let (input, launch) = validated.into_parts();
        let envelope = launch.envelope().clone();

        // 共用 LC run-context guard(Task 7):projection role 与 input role
        // 一致性 + root recipe marker 相位语义 + 双向 tool-policy 守卫,
        // 整块早于版本/session child(与 direct `start` 的纯双向守卫同源)。
        crate::cross_cutting::streaming_provider::guard_lc_validated_launch(
            launch.is_root_recipe_phase(),
            &input.role,
            &input.role,
            input.tool_policy.as_ref(),
        )
        .map_err(|error| {
            ProviderAdapterError::parse_error(error.to_string(), String::new(), String::new())
        })?;

        // adapter 匹配:validated input 必须是 Claude Code(不匹配即拒,
        // 不回退其它 provider/dialect)。
        if input.provider_type != crate::protocol::contracts::ProviderType::ClaudeCode
            || envelope.provider_dialect
                != crate::product::logical_codebase::policy::ProviderDialect::ClaudeCodeCliV1
        {
            return Err(ProviderAdapterError::parse_error(
                "claude lc validated start: only Claude Code launches are accepted by this adapter",
                String::new(),
                String::new(),
            ));
        }

        // 统一 launch audit sink:LC 会话必须绑定 run-bound sink,缺失
        // fail-closed(1b 的 prepare 契约之前由本路径强制)。
        let sink = input.audit_sink.clone().ok_or_else(|| {
            ProviderAdapterError::parse_error(
                "claude lc validated start: audit sink is required for LC launches",
                String::new(),
                String::new(),
            )
        })?;

        // Task 3.3(B0b):LC 只读 facts 与公共 prepare 合同同源
        // (`prepare_lc_facts` 同一观测实现:supplier seam 优先,默认真实
        // `--version` 有界探测经实例内 60 秒 TTL/single-flight 缓存;不可得
        // fail-closed)。全 LC 路径必填,非仅策略角色;guard 已先行,观测
        // 发生在任何 session child 之前。
        let facts = self.observe_lc_facts(&launch, cancel.clone()).await?;
        let provider_version = facts.exact_version().to_string();

        // start_validated 消费 prepared projection(与 gateway 冻结 full
        // projection 同源):完整上下文路径(resume 判定)携带的冻结投影
        // 直接作为审计投影——child 前以当前 facts 复验 exact version(TTL
        // 内同一观测),漂移 typed 拒、零 child;裸 validate 路径(无冻结
        // 投影,legacy/direct fixture)按原语义以 facts 同版本 projector
        // 自投影。trust/MCP bundle digest 的 gateway 侧装配归 1b/1c;空串
        // 同样纳入 session digest,装配后任一漂移都会改变 projection_digest。
        let lc_projection = match launch.lc_projection() {
            Some(frozen) => {
                if frozen.exact_version() != facts.exact_version() {
                    return Err(ProviderAdapterError::parse_error(
                        format!(
                            "claude lc validated start: frozen projection exact version {} disagrees with observed facts version {} (drift before spawn; zero children, no session started)",
                            frozen.exact_version(),
                            facts.exact_version()
                        ),
                        String::new(),
                        String::new(),
                    ));
                }
                frozen.clone()
            }
            None => {
                let projection_input =
                    crate::product::logical_codebase::provider_projection::ProviderProjectionInput::new(
                        envelope.clone(),
                        crate::product::logical_codebase::provider_gateway::ProviderRef::claude_code(
                            launch.capability_snapshot_ref(),
                        ),
                        envelope.action,
                        input.role.clone(),
                        input.permission_mode.clone(),
                        input.tool_policy.clone(),
                        facts.approval_policy().to_string(),
                        facts.mcp_bundle_digest().to_string(),
                        envelope.config_artifact_ref.clone(),
                        String::new(),
                        Some(facts.boundary().clone()),
                    );
                facts
                    .projector()
                    .project(&projection_input)
                    .map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!("claude lc validated start: {error}"),
                            String::new(),
                            String::new(),
                        )
                    })?
            }
        };

        // Task 9a:LC 显式 resume 在 child 前比较全 LC audit(完整字面量含
        // 投影)。存档缺 lc_projection(旧/direct 形态)或任一投影摘要漂移
        // → superseded 终止审计写被取代旧 run、零 child 拒绝(不清 resume
        // id 静默 fresh);无存档记录时依赖 native 握手确认(9b 收口),
        // 本层不清 id。
        if let Some(resume_id) = input
            .resume_provider_session_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            let stored = sink.find_provider_start(resume_id).map_err(|error| {
                ProviderAdapterError::parse_error(
                    format!("claude lc validated start: resume lookup failed: {error}"),
                    String::new(),
                    String::new(),
                )
            })?;
            if let Some(stored) = stored.as_ref() {
                let current = lc_start_audit_literal(&envelope, &lc_projection);
                if matches!(
                    crate::cross_cutting::tool_policy_audit::resume_with_lc_start_record(
                        &stored.record,
                        &current
                    ),
                    crate::cross_cutting::tool_policy_audit::ResumeDecision::RejectSupersedeAndStartNew
                ) {
                    // P1-4 同源裁决:superseded 终止审计写入被取代旧 run。
                    crate::cross_cutting::tool_policy_audit::append_superseded_policy_drift(
                        sink.as_ref(),
                        stored,
                    )
                    .map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!(
                                "claude lc validated start: superseded audit append failed: {error}"
                            ),
                            String::new(),
                            String::new(),
                        )
                    })?;
                    return Err(ProviderAdapterError::parse_error(
                        "claude lc validated start: resume audit drifted or legacy record lacks lc projection; refusing to spawn a fresh child for an explicit LC resume",
                        String::new(),
                        String::new(),
                    ));
                }
            }
        }

        // LC argv 与投影同源;进程 cwd = envelope 冻结的 canonical LC root。
        let args = self.build_lc_validated_args(
            input.resume_provider_session_id.as_deref(),
            input.tool_policy.as_ref(),
        );
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let command = self.command.to_string_lossy().to_string();
        let process_cwd = envelope.working_directory.clone();
        // B1-1:归属在场时预留先于一切 spawn 副作用(durable;失败零
        // spawn,直接上抛)。版本 `--version` 探测不是业务 writer,不计入。
        let reservation = match ownership.as_ref() {
            Some(ownership) => Some(ownership.reserve()?),
            None => None,
        };
        // B1-1 F2/N1:reserve 后常驻守卫——launch future 被引擎在返回前
        // drop 时由 Drop 兜底收口注销;错误路径显式收口后的 Drop 为幂等
        // 空操作,尾任务接管后 disarm。
        let mut launch_guard = ownership
            .clone()
            .map(crate::cross_cutting::streaming_provider::LaunchReservationGuard::new);
        let process = match ProcessManager::spawn(
            &command,
            &arg_refs,
            &process_cwd,
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
        // B1-1:spawn 点回填真实 PID+starttime;身份不可证即整组停止,
        // 不留未登记子进程。
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

        // Task 9b:LC resume 同样执行真实 init 握手(native 应答 id 必须确认
        // 同 id);fresh 等 init 握手。direct `start` 的「resume 已知即不等
        // init」行为保持原样,不属于本路径。
        let resume_native_id = input
            .resume_provider_session_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(ToString::to_string);

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
                cwd: Some(process_cwd.display().to_string()),
                output: None,
                exit_code: None,
            }))
            .await;

        let workspace_session_id = input.workspace_session_id.clone().unwrap_or_default();
        let role_text =
            crate::cross_cutting::streaming_provider::adapter_role_text(&input.role).to_string();
        let tool_policy_digest = projection::lc_tool_policy_canonical_digest(
            input.tool_policy.as_ref(),
            launch.is_root_recipe_phase(),
        )
        .map_err(|error| {
            ProviderAdapterError::parse_error(
                format!("claude lc validated start: {error}"),
                String::new(),
                String::new(),
            )
        })?;

        // 与策略路径同构的有界「初始写入→Running→握手→provider_start 落盘」;
        // 差异仅在于 audit 追加统一 `lc_projection`(LC 会话必填)。
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

        let bound = stream::CLAUDE_POLICY_HANDSHAKE_TIMEOUT.saturating_mul(3);
        let outcome = tokio::time::timeout(bound, async {
            Self::write_initial_messages(&stdin, &input)
                .await
                .map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!(
                            "claude lc validated start: initial write failed: {}",
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
                    cwd: Some(process_cwd.display().to_string()),
                    output: None,
                    exit_code: None,
                }))
                .await;
            let (reader, native_id) = match resume_native_id.as_deref() {
                // Task 9b:原生应答 id 必须与请求 resume id 逐字相等才构成
                // 「已恢复」。缺 id(init 缺/空白,F2)或错 id(应答不同会话)
                // 属于已启动 child 后的 runtime 失败:沿既有 kill 链终止并
                // 回收子进程,错误显式记录「未恢复」(不伪称零 spawn);绝不
                // 回填请求 id、绝不清 id 转 fresh。
                Some(requested_native_id) => {
                    let (reader, native_id) =
                        stream::wait_for_claude_init(stdout, &cancel)
                            .await
                            .map_err(|error| {
                                ProviderAdapterError::parse_error(
                                    format!(
                                        "claude lc validated start: native resume not confirmed (session NOT resumed; the started child was killed and reaped, not a zero-spawn refusal): {}",
                                        error.details
                                    ),
                                    String::new(),
                                    String::new(),
                                )
                            })?;
                    if native_id != requested_native_id {
                        return Err(ProviderAdapterError::parse_error(
                            format!(
                                "claude lc validated start: native init session id {native_id} does not confirm the requested resume id {requested_native_id} (session NOT resumed; the started child was killed and reaped, not a zero-spawn refusal)"
                            ),
                            String::new(),
                            String::new(),
                        ));
                    }
                    (reader, native_id)
                }
                None => {
                    let (reader, session_id) = stream::wait_for_claude_init(stdout, &cancel)
                        .await
                        .map_err(|error| {
                            ProviderAdapterError::parse_error(
                                format!(
                                    "claude lc validated start: handshake failed: {}",
                                    error.details
                                ),
                                String::new(),
                                String::new(),
                            )
                        })?;
                    (reader, session_id)
                }
            };
            // F2 同验:空白原生会话 id 不得进入 durable 审计。
            if native_id.trim().is_empty() {
                return Err(ProviderAdapterError::parse_error(
                    "claude lc validated start: native session id is blank before audit write",
                    String::new(),
                    String::new(),
                ));
            }
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
                lc_projection: Some(crate::cross_cutting::tool_policy_audit::LcProjectionAudit {
                    action: projection::action_text(envelope.action).to_string(),
                    wire_dialect: projection::wire_dialect_text(lc_projection.wire_dialect())
                        .to_string(),
                    capability_projection_digest: lc_projection
                        .capability_projection_digest()
                        .to_string(),
                    projection_digest: lc_projection.projection_digest().to_string(),
                    boundary_evidence_ref: lc_projection.boundary_evidence_ref().to_string(),
                }),
            });
            sink.append_bound(audit_event).map_err(|error| {
                ProviderAdapterError::parse_error(
                    format!(
                        "claude lc validated start: provider_start audit append failed: {error}"
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
        // 成功:child 与续读 reader 移交会话收尾任务(与策略路径共用收尾)；
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
        Ok(ProviderSession {
            native_session_id: Some(native_id),
            events: event_rx,
            commands,
        })
    }

    /// Task 3.3(B0b):Claude 真实 `prepare_lc_facts`——消费 3.2 公共 prepare
    /// 合同,以只读方式观测本家 facts(exact version 经 60 秒 TTL/single-
    /// flight 的真实 CLI `--version` 观测或 supplier seam、envelope 派生
    /// boundary、与该版本同源的纯 projector),供 gateway 装配 full
    /// projection。guard 先行(早于版本 probe/child),adapter 匹配错位即
    /// typed 拒;全程零 spawn/零写,不从 registry/capability 空 version
    /// 取证。
    pub(super) async fn prepare_lc_facts_inner(
        &self,
        input: &StreamingProviderInput,
        launch: &crate::product::logical_codebase::provider_gateway::ValidatedSessionLaunchPolicy,
        cancel: CancellationToken,
    ) -> Result<
        crate::product::logical_codebase::provider_projection::ProviderLcPrepareFacts,
        ProviderAdapterError,
    > {
        // guard 早于 probe/child(与 start_validated 同一守卫:run-context
        // role 一致 + root recipe 相位语义 + 双向 tool-policy 守卫)。
        crate::cross_cutting::streaming_provider::guard_lc_validated_launch(
            launch.is_root_recipe_phase(),
            &input.role,
            &input.role,
            input.tool_policy.as_ref(),
        )
        .map_err(|error| {
            ProviderAdapterError::parse_error(error.to_string(), String::new(), String::new())
        })?;

        // adapter 匹配:只有 Claude Code/ClaudeCodeCliV1 的 launch 可观测
        // (不匹配即拒,不回退其它 provider/dialect)。
        if input.provider_type != crate::protocol::contracts::ProviderType::ClaudeCode
            || launch.envelope().provider_dialect
                != crate::product::logical_codebase::policy::ProviderDialect::ClaudeCodeCliV1
        {
            return Err(ProviderAdapterError::parse_error(
                "claude lc prepare facts: only Claude Code launches are accepted by this adapter",
                String::new(),
                String::new(),
            ));
        }

        self.observe_lc_facts(launch, cancel).await
    }
}
