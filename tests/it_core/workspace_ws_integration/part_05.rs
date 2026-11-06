impl HangingStreamingProvider {
    /// 裸 `start`（单仓 direct 历史路径）与 `start_validated`（LC
    /// gateway 路径）共用的会话构造。
    fn gateway_session(&self, cancel: CancellationToken) -> ProviderSession {
                let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: "# Draft".to_string(),
                })
                .await;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    command = command_rx.recv() => {
                        match command {
                            Some(ProviderCommand::Abort) | None => return,
                            Some(ProviderCommand::PermissionResponse { .. })
                            | Some(ProviderCommand::ChoiceResponse { .. })
                            | Some(ProviderCommand::ToolResult(_)) => {}
                        }
                    }
                }
            }
        });
        ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        }
    
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for HangingStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Ok(self.gateway_session(cancel))
    }

    /// LC 会话经 gateway validated 启动（it_web/issue_delivery_summary.rs
    /// FullChainStreamingProvider 先例：替身行为与输入无关，从 validated
    /// 事实复刻同一行为，按 trait 契约不回调裸 start）。
    async fn start_validated(
        &self,
        _validated: cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Ok(self.gateway_session(cancel))
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<
        mpsc::Receiver<cadence_aria::cross_cutting::streaming_provider::StreamChunk>,
        ProviderAdapterError,
    > {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

/// 断连不取消修复的测试 provider：先发首个 chunk（驱动测试可确认 run 已启动），
/// 随后挂起直到外部 `Notify` 触发才发出 Completed——用于验证「断连期间 run
/// 不被取消、触发完成后仍能驱动至完成并落盘」。
struct SignalledCompletionStreamingProvider {
    complete: Arc<Notify>,
}

impl SignalledCompletionStreamingProvider {
    /// 裸 `start`（单仓 direct 历史路径）与 `start_validated`（LC
    /// gateway 路径）共用的会话构造。
    fn gateway_session(&self, cancel: CancellationToken) -> ProviderSession {
                let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(8);
        let complete = self.complete.clone();
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: "# Draft".to_string(),
                })
                .await;
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = complete.notified() => {}
            }
            // 用合法 story artifact 作为完成输出：避免触发引擎的「artifact 缺失
            // 自动续写」二回合（那会掩盖本测试要观察的断连存活语义）。
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        VALID_STORY_SPEC.to_string(),
                        None,
                    ),
                ))
                .await;
        });
        ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        }
    
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for SignalledCompletionStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Ok(self.gateway_session(cancel))
    }

    /// LC 会话经 gateway validated 启动（it_web/issue_delivery_summary.rs
    /// FullChainStreamingProvider 先例：替身行为与输入无关，从 validated
    /// 事实复刻同一行为，按 trait 契约不回调裸 start）。
    async fn start_validated(
        &self,
        _validated: cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Ok(self.gateway_session(cancel))
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<
        mpsc::Receiver<cadence_aria::cross_cutting::streaming_provider::StreamChunk>,
        ProviderAdapterError,
    > {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

struct PiHangingStreamingProvider {
    started: Arc<std::sync::atomic::AtomicBool>,
    abort_observed: Arc<Notify>,
    seen_inputs: Arc<Mutex<Vec<(cadence_aria::protocol::contracts::ProviderType, cadence_aria::cross_cutting::streaming_provider::ProviderPermissionMode)>>>,
}

impl PiHangingStreamingProvider {
    /// 裸 `start`（单仓 direct 历史路径）与 `start_validated`（LC
    /// gateway 路径）共用的会话构造：先吐首段输出，随后挂起直到取消。
    fn hanging_session(&self, cancel: CancellationToken) -> ProviderSession {
        self.started
            .store(true, std::sync::atomic::Ordering::SeqCst);

        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(8);
        let abort_observed = self.abort_observed.clone();
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: "Pi partial output".to_string(),
                })
                .await;
            cancel.cancelled().await;
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: PI_POST_ABORT_OUTPUT.to_string(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        PI_POST_ABORT_OUTPUT.to_string(),
                        Some("pi-post-abort-session".to_string()),
                    ),
                ))
                .await;
            abort_observed.notify_one();
        });
        ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for PiHangingStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.seen_inputs
            .lock()
            .expect("Pi input recorder")
            .push((input.provider_type, input.permission_mode));
        Ok(self.hanging_session(cancel))
    }

    /// LC 会话经 gateway validated 启动：会话行为（首段输出 → 挂起 → 取消
    /// 后收尾）逐字复刻；但 validated 启动面只外借 role/working_dir/prompt，
    /// **不外借** provider_type / permission_mode（`into_parts` 是
    /// pub(crate)，crate 外取不到），adapter 侧无法记录这两个观察事实；
    /// 用例 `workspace_ws_abort_with_pi_reaches_cancelled_state_and_stops_output`
    /// 已改在 durable 层断言同一语义（session.author_provider ×
    /// permission_modes.author × provider_start_ledger actual launch）。
    /// 按 trait 契约不回调裸 start。
    async fn start_validated(
        &self,
        _validated: cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Ok(self.hanging_session(cancel))
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

struct ChoiceThenCompletingStreamingProvider;

impl ChoiceThenCompletingStreamingProvider {
    /// 裸 `start`（单仓 direct 历史路径）与 `start_validated`（LC
    /// gateway 路径）共用的会话构造。
    fn gateway_session(&self, cancel: CancellationToken) -> ProviderSession {
                let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id: "choice_completing_001".to_string(),
                    prompt: "继续方式？".to_string(),
                    options: vec![ChoiceOptionData {
                        id: "opt_0".to_string(),
                        label: "继续 author".to_string(),
                        description: None,
                    }],
                    allow_multiple: false,
                    allow_free_text: false,
                    questions: vec![],
                    source: ChoiceRequestSource::ProviderChoice,
                }))
                .await;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    command = command_rx.recv() => {
                        match command {
                            Some(ProviderCommand::ChoiceResponse { .. }) => {
                                let _ = event_tx
                                    .send(ProviderEvent::Completed(
                                        cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(
                                            VALID_STORY_SPEC.to_string(),
                                            Some(
                                            "choice-completing-session".to_string(),
                                        ),
                                        ),
                                    ))
                                    .await;
                                return;
                            }
                            Some(ProviderCommand::Abort) | None => return,
                            Some(ProviderCommand::PermissionResponse { .. })
                            | Some(ProviderCommand::ToolResult(_)) => {}
                        }
                    }
                }
            }
        });
        ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        }
    
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ChoiceThenCompletingStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Ok(self.gateway_session(cancel))
    }

    /// LC 会话经 gateway validated 启动（it_web/issue_delivery_summary.rs
    /// FullChainStreamingProvider 先例：替身行为与输入无关，从 validated
    /// 事实复刻同一行为，按 trait 契约不回调裸 start）。
    async fn start_validated(
        &self,
        _validated: cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Ok(self.gateway_session(cancel))
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

#[derive(Default)]
struct SequencedChoiceCompletingProvider {
    next_choice: Mutex<u32>,
}

impl SequencedChoiceCompletingProvider {
    /// 裸 `start`（单仓 direct 历史路径）与 `start_validated`（LC
    /// gateway 路径）共用的会话构造。
    fn gateway_session(&self, cancel: CancellationToken) -> ProviderSession {
                let id = {
            let mut next = self.next_choice.lock().unwrap();
            *next += 1;
            format!("choice_sequence_{next:03}")
        };
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id,
                    prompt: "继续方式？".to_string(),
                    options: vec![ChoiceOptionData {
                        id: "opt_0".to_string(),
                        label: "继续 author".to_string(),
                        description: None,
                    }],
                    allow_multiple: false,
                    allow_free_text: false,
                    questions: vec![],
                    source: ChoiceRequestSource::ProviderChoice,
                }))
                .await;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    command = command_rx.recv() => {
                        match command {
                            Some(ProviderCommand::ChoiceResponse { .. }) => {
                                let _ = event_tx
                                    .send(ProviderEvent::Completed(
                                        cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(
                                            VALID_STORY_SPEC.to_string(),
                                            Some(
                                            "choice-sequence-session".to_string(),
                                        ),
                                        ),
                                    ))
                                    .await;
                                return;
                            }
                            Some(ProviderCommand::Abort) | None => return,
                            Some(ProviderCommand::PermissionResponse { .. })
                            | Some(ProviderCommand::ToolResult(_)) => {}
                        }
                    }
                }
            }
        });
        ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        }
    
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for SequencedChoiceCompletingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Ok(self.gateway_session(cancel))
    }

    /// LC 会话经 gateway validated 启动（it_web/issue_delivery_summary.rs
    /// FullChainStreamingProvider 先例：替身行为与输入无关，从 validated
    /// 事实复刻同一行为，按 trait 契约不回调裸 start）。
    async fn start_validated(
        &self,
        _validated: cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Ok(self.gateway_session(cancel))
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

impl ScriptedStreamingProvider {
    /// 裸 `start`（单仓 direct 历史路径）与 `start_validated`（LC
    /// gateway 路径）共用的会话构造：消费脚本化输出，并按结构化输出契约
    /// 组装 completion（无契约时原样透传）。
    fn scripted_session(
        &self,
        prompt: String,
        contract: Option<cadence_aria::cross_cutting::structured_output::StructuredOutputContract>,
    ) -> ProviderSession {
        self.prompts.lock().unwrap().push(prompt);
        let output = self
            .outputs
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted provider output");
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed(scripted_provider_completion(
                    output,
                    contract.as_ref(),
                )))
                .await;
        });
        ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScriptedStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Ok(self.scripted_session(input.prompt, input.structured_output_contract))
    }

    /// LC 会话经 gateway validated 启动：观察点从 adapter 侧的裸 input
    /// 迁到 validated 启动事实的只读投影（`launch_view().prompt`）——投影
    /// 借出的正是本次 launch 的实际 prompt，语义不减；结构化输出契约不进
    /// 投影，reviewer 侧的契约事实由生产写在 prompt 的输出模板里，替身据此
    /// 复原同一 nonce（author 侧本就无契约，保持 None）。按 trait 契约不回调
    /// 裸 start。
    async fn start_validated(
        &self,
        validated: cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let view = validated.launch_view();
        let contract = reviewer_contract_from_prompt(view.role, view.prompt);
        Ok(self.scripted_session(view.prompt.to_string(), contract))
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<
        mpsc::Receiver<cadence_aria::cross_cutting::streaming_provider::StreamChunk>,
        ProviderAdapterError,
    > {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

fn scripted_provider_completion(
    output: String,
    contract: Option<
        &cadence_aria::cross_cutting::structured_output::StructuredOutputContract,
    >,
) -> cadence_aria::cross_cutting::streaming_provider::ProviderCompletion {
    let Some(contract) = contract else {
        return cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(
            output, None,
        );
    };
    let Some((comments, json)) = review_output_parts(&output) else {
        // 无 JSON fence 的产物（如「reviewer 未运行」提示）不包装 sentinel——
        // 与生产 Fake adapter 同口径：原样透传，结构化解析由契约 fail-closed。
        return cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::from_output(
            output,
            Some(contract),
            None,
        );
    };
    let mut json: serde_json::Value = serde_json::from_str(json).expect("review fixture JSON object");
    json.as_object_mut()
        .expect("review fixture JSON object")
        .insert("nonce".to_string(), serde_json::json!(contract.nonce));
    let full_output = format!(
        "{comments}\n<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">{json}</ARIA_STRUCTURED_OUTPUT>",
        contract.nonce
    );
    cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::from_output(
        full_output,
        Some(contract),
        None,
    )
}

/// reviewer 侧的结构化输出契约事实（nonce）——生产把同一 nonce 写进 prompt
/// 的「实际输出模板」块，替身据此复原同一契约（validated 启动面只外借
/// role/working_dir/prompt，不外借契约本身）。author 侧本就无契约，返回 None。
fn reviewer_contract_from_prompt(
    role: &cadence_aria::protocol::contracts::AdapterRole,
    prompt: &str,
) -> Option<cadence_aria::cross_cutting::structured_output::StructuredOutputContract> {
    if *role != cadence_aria::protocol::contracts::AdapterRole::Reviewer {
        return None;
    }
    let nonce = nonce_from_prompt(prompt)?;
    Some(
        cadence_aria::cross_cutting::structured_output::StructuredOutputContract {
            nonce,
            schema_name: "workspace_review".to_string(),
        },
    )
}

/// 从 prompt 的结构化输出 sentinel 模板取真实 nonce（示例块用
/// `EXAMPLE_NONCE` 占位，取最后一个非占位值）。
fn nonce_from_prompt(prompt: &str) -> Option<String> {
    const PREFIX: &str = "<ARIA_STRUCTURED_OUTPUT nonce=\"";
    let mut cursor = 0usize;
    let mut last: Option<String> = None;
    while let Some(offset) = prompt[cursor..].find(PREFIX) {
        let start = cursor + offset + PREFIX.len();
        let rest = &prompt[start..];
        let end = rest.find('"')?;
        let nonce = &rest[..end];
        if nonce != "EXAMPLE_NONCE" {
            last = Some(nonce.to_string());
        }
        cursor = start + end;
    }
    last
}

fn review_output_parts(output: &str) -> Option<(&str, &str)> {
    let trimmed = output.trim();
    if trimmed.starts_with('{') {
        return Some(("", trimmed));
    }
    let end = output.rfind("```")?;
    let before_end = &output[..end];
    let start = before_end.rfind("```")?;
    let json = before_end[start + 3..]
        .trim()
        .strip_prefix("json")
        .unwrap_or(&before_end[start + 3..])
        .trim();
    Some((output[..start].trim_end(), json))
}

async fn create_workspace_session_fixture(root: &TempDir) -> PathBuf {
    create_workspace_session_fixture_with_author(root, "fake").await
}

async fn create_workspace_session_fixture_with_author(
    root: &TempDir,
    author_provider: &str,
) -> PathBuf {
    create_workspace_session_fixture_with_providers(root, author_provider, "fake", 1).await
}

fn set_workspace_author_permission_mode_to_supervised(root: &TempDir) {
    let session = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .update_workspace_session_permission_modes(
            "workspace_session_0001",
            cadence_aria::product::models::WorkspaceRolePermissionModes {
                author: cadence_aria::cross_cutting::streaming_provider::ProviderPermissionMode::Supervised,
                reviewer: cadence_aria::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            },
        )
        .expect("persist supervised author permission mode");
    assert_eq!(
        session.permission_modes.author,
        cadence_aria::cross_cutting::streaming_provider::ProviderPermissionMode::Supervised
    );
}

async fn create_workspace_session_fixture_with_providers(
    root: &TempDir,
    author_provider: &str,
    reviewer_provider: &str,
    review_rounds: u32,
) -> PathBuf {
    let repo = git_repo(root.path());
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    let (status, accepted) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"]
        .as_str()
        .expect("repository initialization operation id");
    let operation_uri = format!(
        "/api/projects/project_0001/repository-initializations/{operation_id}"
    );
    let mut last_snapshot = accepted;
    let mut completed = None;
    for _ in 0..100 {
        let (status, snapshot) = request_json(
            app.clone(),
            Method::GET,
            &operation_uri,
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{snapshot}");
        tokio::task::yield_now().await;
        match snapshot["status"].as_str() {
            Some("completed") => {
                completed = Some(snapshot);
                break;
            }
            Some("failed") => panic!(
                "repository initialization failed before fixture issue creation: {snapshot}"
            ),
            _ => {
                last_snapshot = snapshot;
            }
        }
    }
    let completed = completed.unwrap_or_else(|| {
        panic!(
            "repository initialization did not complete before fixture issue creation: {last_snapshot}"
        )
    });
    assert_eq!(completed["status"], "completed", "{completed}");
    // 块3（remove-single-repository-path）：单仓 issue 创建入口已删除（缺
    // logical_codebase_id 会被 400 issue_not_bound_to_logical_codebase 拒绝）。
    // 按生产 v1.2→v1.3 身份迁移把 repos.json 的 repository_0001 升成 LC 权威
    // （member/checkout/identity-registry/manifest + bootstrap policy 均由
    // IdentityMigrationExecutor 落盘），issue 随即以确定性 legacy 别名 LC 归属；
    // LC record 由 require_logical_codebase 首读时的 migrate_legacy 自动生成。
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    IdentityMigrationExecutor::new(app_paths.clone())
        .ensure_identity_schema("project_0001")
        .expect("migrate fixture repository into logical codebase authority");
    // generate 入口经 PlanningContextResolver 读 LC 的 active 聚合索引（缺失即
    // aggregate_index_active_missing）。经**生产聚合索引操作 + 假 CodeGraph CLI**
    //（aggregate_index_fixture）建 active 聚合索引：夹具不真跑 `codegraph sync .`
    //（本机重活：抢全局 `/tmp/.codegraph` 锁 + 吃 30s 增量预算 ⇒ 422
    // codegraph_timeout，整批用例红在夹具前置上）；假 CLI 只替换 CLI 应答，真实
    // git 成员快照 / 成员覆盖校验 / durable 世代落盘 / active 投影全走生产链路。
    // rebuild 端点本身的行为覆盖由 it_web 的 web_aggregate_index_api 承担。
    crate::aggregate_index_fixture::seed_active_aggregate_index(
        &app_paths,
        "project_0001",
        &legacy_logical_codebase_id("project_0001"),
    );
    // LC 会话经生产 gateway 启动：bootstrap capability 行恒 Unknown（仅真实
    // probe 可写 Confirmed），正向夹具须显式物化「已探测」事实，否则 launch
    // 分格 fail-closed 为 provider_capability_launch_not_confirmed。
    seed_confirmed_gateway_capabilities(&app_paths);
    // 单成员 involved 钉定（r23）：story 草稿确认门（REQ-PLN-04）要求 involved
    // 已确定；夹具不走 AI 自决回写，按 generate 请求字段显式钉定首成员。
    let involved_repository_ids: Vec<String> = LogicalCodebaseStore::new(app_paths.clone())
        .list_members("project_0001")
        .expect("list fixture logical members")
        .into_iter()
        .map(|member| member.logical_repository_id.0.to_string())
        .collect();
    let (status, issue) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({
            "title":"登录会话过期",
            "description":"描述",
            "repository_id":"repository_0001",
            "logical_codebase_id": legacy_logical_codebase_id("project_0001"),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{issue}");
    let (status, story_response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title":"登录会话过期提示",
            "involved_repository_ids": involved_repository_ids,
            "author_provider":author_provider,
            "reviewer_provider":reviewer_provider,
            "review_rounds":review_rounds,
            "superpowers_enabled":true,
            "openspec_enabled":true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        story_response["workspace_session"]["workspace_session_id"],
        "workspace_session_0001"
    );
    repo
}

/// 三 canonical action 行全 `Confirmed` 的 capability 记录（trust 亦
/// Confirmed，执行语义 host-open，snapshot ref 与 gateway
/// `provider("cap_managed_snapshot")` 对齐）。与 tests/it_web/
/// provider_capability_fixture.rs 同构——该 helper 是 it_web target 私有，
/// it_core（独立 target）取不到，按其文档口径在此复刻；负向场景
/// （断言「能力未确认」被拒）不得调用本 helper。
fn confirmed_capability_record(provider_type: ProviderRefType) -> ProviderCapabilityRecord {
    let (adapter_dialect, wire_dialect) = match provider_type {
        ProviderRefType::ClaudeCode => (
            ProviderDialect::ClaudeCodeCliV1,
            ProviderWireDialect::ClaudeCodeStreamJson,
        ),
        ProviderRefType::Codex => (
            ProviderDialect::CodexCliV1,
            ProviderWireDialect::CodexAppServerRpc,
        ),
        ProviderRefType::Pi => (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc),
        ProviderRefType::KimiCode => (
            ProviderDialect::KimiAcpV1,
            ProviderWireDialect::KimiAcp,
        ),
    };
    let row = |action: SessionPolicyAction| ProviderActionCapability {
        action,
        launch: ProviderCapabilityEvidence::Confirmed,
        resume: ProviderCapabilityEvidence::Confirmed,
        write_boundary: ProviderCapabilityEvidence::Confirmed,
        projection_digest: format!("projection-digest-{action:?}"),
        evidence_ref: format!("probe://{action:?}"),
    };
    ProviderCapabilityRecord {
        provider_type,
        schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
        version: "1.4.0".to_string(),
        adapter_dialect,
        wire_dialect,
        capability_snapshot_ref: "cap_managed_snapshot".to_string(),
        evidence: CapabilityEvidence::ProductionVerified,
        resume_evidence: ResumeEvidenceState::Unsupported,
        supported_actions: Vec::new(),
        action_matrix: ProviderActionMatrix::from_rows(vec![
            row(SessionPolicyAction::PlanningReadOnly),
            row(SessionPolicyAction::CodingTargetWrite),
            row(SessionPolicyAction::ReviewReadOnly),
        ])
        .expect("three canonical capability rows"),
        trust: ProviderCapabilityEvidence::Confirmed,
        probed_at: None,
        probe_artifact_ref: None,
        execution_semantics: PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN.to_string(),
    }
}

/// 先物化 bootstrap 行（保持生产「未探测」语义），再把 gateway 四家真实
/// provider 覆盖写为 `Confirmed`（project 级 legacy 子树——与 legacy 别名 LC
/// 的 `for_lc` 读取路径同根）。
fn seed_confirmed_gateway_capabilities(app_paths: &ProductAppPaths) {
    let store = ProviderCapabilityStore::new(app_paths.clone());
    store
        .ensure_bootstrap()
        .expect("bootstrap capability records");
    for provider_type in [
        ProviderRefType::ClaudeCode,
        ProviderRefType::Codex,
        ProviderRefType::Pi,
        ProviderRefType::KimiCode,
    ] {
        store
            .upsert(&confirmed_capability_record(provider_type))
            .expect("seed confirmed capability record");
    }
}

/// 与 tests/it_web/issue_delivery_summary.rs 同构的「期望创建成功」注入：
/// gateway factory 直挂 always-available gate（真实 health service 未探测
/// 恒 degraded，会令 gateway launch 分格 provider_gateway_unavailable）；
/// 该先例为 it_web target 私有，it_core 取不到，按其口径在此复刻。
fn always_available_gate() -> Arc<ProviderAvailabilityGate> {
    struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);

    impl ProviderHealthSource for AlwaysHealthy {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.0.clone()
        }

        fn degraded(&self) -> bool {
            false
        }
    }

    let checked_at = chrono::Utc::now();
    let snapshot = Arc::new(ProviderHealthSnapshot {
        schema_version: 1,
        generation: 1,
        checked_at,
        providers: [
            ProviderName::ClaudeCode,
            ProviderName::Codex,
            ProviderName::Pi,
            ProviderName::KimiCode,
        ]
        .into_iter()
        .map(|provider| ProviderHealthEntry {
            provider,
            command: "stub".to_string(),
            available: true,
            version: Some("1.0".to_string()),
            reason_code: None,
            reason: None,
            checked_at,
        })
        .collect(),
    });
    Arc::new(ProviderAvailabilityGate::new(Arc::new(AlwaysHealthy(
        snapshot,
    ))))
}

/// 同步 adapter stub：gateway 工厂构造签名需要，workspace WS 只走 streaming
/// 路径，sync adapter 不会被调起。
struct StubSyncAdapter;

impl ProviderAdapter for StubSyncAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

/// 重建绑定 always-available gate 与注入 registry 的 gateway factory
/// （manager 每次 create 经 state.gateway_factory() 现取，重建后即生效）。
fn with_available_provider_health(state: WebAppState, root: &std::path::Path) -> WebAppState {
    let registry = state.provider_registry.clone();
    state.with_gateway_factory(Arc::new(LogicalCodebaseGatewayFactory::new(
        ProductAppPaths::new(root.join(".aria")),
        registry,
        Arc::new(StubSyncAdapter),
        always_available_gate(),
    )))
}

fn replace_workspace_session_messages(root: &std::path::Path, messages: Value) {
    let session_path = root.join(
        ".aria/projects/project_0001/issues/issue_0001/workspace-sessions/workspace_session_0001.json",
    );
    let mut session: Value =
        serde_json::from_str(&fs::read_to_string(&session_path).expect("workspace session json"))
            .expect("workspace session value");
    session["messages"] = messages;
    fs::write(
        &session_path,
        serde_json::to_string_pretty(&session).expect("workspace session json"),
    )
    .expect("write workspace session");
}

// part_05 拆分（large_file_guard 1200 行上限）：HTTP/WS 消息收发工具群经 include! 挂载，与本文档同一模块作用域，夹具调用方与行为零变化（纯移动）。
include!("part_05b.rs");

struct ChoiceThenHangingStreamingProvider;

impl ChoiceThenHangingStreamingProvider {
    /// 裸 `start`（单仓 direct 历史路径）与 `start_validated`（LC gateway
    /// 路径）共用的会话构造：先发 choice 后挂起。
    fn gateway_session(&self, cancel: CancellationToken) -> ProviderSession {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id: "choice_hanging_001".to_string(),
                    prompt: "继续方式？".to_string(),
                    options: vec![ChoiceOptionData {
                        id: "opt_0".to_string(),
                        label: "继续 author".to_string(),
                        description: None,
                    }],
                    allow_multiple: false,
                    allow_free_text: false,
                    questions: vec![],
                    source: ChoiceRequestSource::ProviderChoice,
                }))
                .await;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    command = command_rx.recv() => {
                        match command {
                            Some(ProviderCommand::Abort) | None => return,
                            Some(ProviderCommand::ChoiceResponse { .. }) => {
                                let _ = event_tx
                                    .send(ProviderEvent::StatusChanged(ProviderStatus::Running))
                                    .await;
                            }
                            Some(ProviderCommand::PermissionResponse { .. })
                            | Some(ProviderCommand::ToolResult(_)) => {}
                        }
                    }
                }
            }
        });
        ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ChoiceThenHangingStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Ok(self.gateway_session(cancel))
    }

    /// LC 会话经 gateway validated 启动（it_web/issue_delivery_summary.rs
    /// FullChainStreamingProvider 先例：替身行为与输入无关，从 validated
    /// 事实复刻同一行为，按 trait 契约不回调裸 start）。
    async fn start_validated(
        &self,
        _validated: cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Ok(self.gateway_session(cancel))
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}
