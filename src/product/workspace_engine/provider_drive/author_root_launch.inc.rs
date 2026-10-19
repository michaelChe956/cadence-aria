// Task 2.1（lc-root-initialization，REQ-ENV-01/ENV-10，映射 openspec tasks 2.3 的
// Author/ChoiceFollowup 面）：LC 会话 root launch 解析 + gateway 启动接线。
// 物理拆分（1200 行守卫）：经 include! 挂载，模块域与 provider_drive.rs 相同。
use crate::product::logical_codebase::{
    PolicyTarget, ProviderRef, SessionLaunchRequest, SessionPolicyAction,
    ValidatedSessionLaunchPolicy,
};

impl WorkspaceEngine {
    /// 策略会话审计接线（Task 3.2，REQ-ENV-09/D7）：policy present 时为 input 绑定
    /// run-bound durable sink（LifecycleStore `tool-policy-run-audit/` 分区）并按
    /// provider run 分配 `role_run_seq`（分配随该 run 的 provider_start 首行落盘
    /// 持久化）。每次 provider run 重新分配（重试 run 独立审计文件）。持久 store
    /// 缺失（内存态 engine）时不接线——真实 adapter 对 policy 会话缺 sink 自身
    /// fail-closed，fake provider 测试路径不受影响。
    pub(crate) fn attach_tool_policy_audit(
        &self,
        mut input: StreamingProviderInput,
    ) -> StreamingProviderInput {
        // 批2 Task 11(T20 前中间态):Reviewer wire 面去 deny 后,执行审计按
        // 角色保留——Reviewer 的 provider run 同样分配 run-bound durable
        // sink(纪律=ReviewerVerify+窗口合同,真窗归 T20,不随去 deny 丢审计)。
        let is_reviewer_run =
            input.role == crate::protocol::contracts::AdapterRole::Reviewer;
        if (input.tool_policy.is_none() && !is_reviewer_run) || input.audit_sink.is_some() {
            return input;
        }
        let Some(store) = self.lifecycle_store.as_ref() else {
            tracing::warn!(
                "policy provider run without a persistent lifecycle store; durable tool-policy audit is not wired"
            );
            return input;
        };
        let workspace_session_id = input
            .workspace_session_id
            .clone()
            .unwrap_or_else(|| self.session.session_id.clone());
        let role_run_seq = match store.next_tool_policy_role_run_seq(&workspace_session_id) {
            Ok(seq) => seq,
            Err(error) => {
                tracing::error!(
                    error = %error,
                    "tool-policy role_run_seq allocation failed; leaving audit sink unset"
                );
                return input;
            }
        };
        input.audit_sink = Some(
            crate::cross_cutting::tool_policy_audit::RoleRunBoundAuditSink::new(
                std::sync::Arc::new(store.clone()),
                workspace_session_id,
                role_run_seq,
            )
            .into_sink(),
        );
        input
    }

    /// 同 `attach_tool_policy_audit`，但作用于 gateway validated input（内部 input
    /// 重建后原样保留 launch policy）。
    fn attach_tool_policy_audit_to_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
    ) -> crate::cross_cutting::session_launch::ValidatedStreamingProviderInput {
        let (input, launch) = validated.into_parts();
        let input = self.attach_tool_policy_audit(input);
        crate::cross_cutting::session_launch::ValidatedStreamingProviderInput::new(input, launch)
    }

    /// Task 11:逻辑代码库 planning 栈入口。与 `handle_author_message_with_prompt_mode`
    /// 对称,但 provider 会话改由 `LogicalCodebaseProviderGateway::start_streaming` 启动,
    /// 使真实启动唯一由 gateway 产出并留 audit。
    ///
    /// Task 2.1 起由 LC 会话的 Author 首轮/ChoiceFollowup 消费（经
    /// `handle_author_message_with_prompt_mode` 的 logical 分支）：调用方在确认
    /// issue 属于逻辑代码库后,构造 `SessionLaunchRequest`、经 `gateway.validate`
    /// 产出 validated policy,再组装 `ValidatedStreamingProviderInput` 传入;本方法
    /// 仅消费 validated input 启动并驱动。传统单仓/非逻辑 issue 仍走
    /// `handle_user_message`/`handle_author_message_with_prompt_mode` 的直接
    /// `provider.start` 路径。
    pub(crate) async fn drive_author_provider_session_via_gateway(
        &mut self,
        validated_input: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        command_rx: mpsc::Receiver<ProviderCommand>,
        generation_node_id: String,
    ) {
        let gateway = self
            .logical_provider_gateway
            .clone()
            .expect("logical provider gateway must be injected before driving via gateway");
        let validated_input = self.attach_tool_policy_audit_to_validated(validated_input);
        let author_provider = self.session.author_provider.clone();
        // 指引卡简化补缺：分类桥结构化变体一次产出平铺 details 与同源
        // 等待投影——`trust_waiting` 随驱动收尾紧随 error 事件发出结构化
        // 指导帧（`EngineEvent::TrustMissingGuidance`）；非 trust 分型为
        // None，零发出。
        let mut launch_trust_waiting = None;
        let session = gateway
            .start_streaming(validated_input, self.cancel.clone())
            .await
            .map_err(|error| {
                // 诊断直通（claude×轻 握手谜团 第 2 轮）：不丢弃 adapter stderr——
                // 尾部（有界）并入 details，stderr 字段同源保留（其余 gateway 校验
                // 错误维持 Display 文案）。同 review/drive.rs 的映射约定。
                // T6 fix-loop r2：压平前经共享分类桥——trust 维度拒绝提升
                // provider_trust_missing 分类+同源指导+生成场景 retry_action
                // （owner 现场 Story 首生成裸串的根因位）；其余变体原样，
                // 不给漂移根 trust 指导。
                let mut mapped = crate::cross_cutting::provider_adapter::ProviderAdapterError {
                    code: crate::protocol::provider_errors::ProviderErrorCode::ProviderUnavailable,
                    details: error.to_string(),
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: None,
                    timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
                    duration_ms: 0,
                };
                if let crate::product::logical_codebase::ProviderGatewayError::Adapter(inner) =
                    &error
                {
                    crate::cross_cutting::provider_adapter::ProviderAdapterError::append_bounded_stderr_tail(
                        &mut mapped.details,
                        &inner.stderr,
                        crate::cross_cutting::provider_adapter::PROVIDER_ERROR_STDERR_TAIL_BYTES,
                    );
                    mapped.stderr = inner.stderr.clone();
                }
                let classification = gateway.classify_launch_trust_failure(
                    &error,
                    &author_provider,
                    &mapped.details,
                    crate::product::logical_codebase::provider_trust::ProviderTrustScenario::Generation,
                );
                launch_trust_waiting = classification.trust_waiting;
                mapped.details = classification.details;
                mapped
            });
        self.drive_provider_session(ProviderSessionDriveInput {
            session,
            command_rx,
            node_id: Some(generation_node_id),
            agent: Some(self.session.author_provider.clone()),
            role: ProviderConversationRole::Author,
            artifact_retry: None,
            revision_resume_fallback: None,
        })
        .await;
        // 紧随 error 事件之后的结构化指导（error 帧先清旧指导、此帧再建；
        // 驱动 Err 收口内部已发 EngineEvent::Error，Ok 路径 waiting 恒
        // None）。
        self.emit_launch_trust_guidance(launch_trust_waiting).await;
    }

    /// 批2 Task 11（Author 独立入口·硬约束1/3）：LC author launch 的受信
    /// 装配——以 engine durable 事实构造 duty=Author、carrier=Orchestrator
    /// 的 `TrustedRunIdentity`，按权威装配矩阵冻结 `HostCoding +
    /// AuthorToolPolicy` 执行合同，并按 duty 强制 wire 岗位投影
    /// （`tool_policy_for_duty`：Author → DenyFileWriteBuiltins，Claude
    /// `Edit/Write/NotebookEdit`、Pi `edit/write` 的 argv 过滤由此携带；
    /// caller 传入值被覆盖，不得凭 input 铸造/剥离岗位投影），最后把合同
    /// 绑定进 validated launch。
    ///
    /// 口径合同（硬约束2）：fresh（首轮/ChoiceFollowup）与 resume/revision
    /// 续接同用本装配——resume 判定以未装饰投影比对（T10），合同摘要走
    /// resume 稳定域（不含 provider_run_id），绑定不引起 supersede。
    /// 伪造 role 缺合同：Author launch 唯一经本受信装配，role 字符串/
    /// serde 输入无从铸合同。audit 面经 Orchestrator 承载仍记录
    /// duty=Author（合同事实面）。
    pub(crate) fn bind_author_execution_contract(
        &self,
        mut input: StreamingProviderInput,
        launch: ValidatedSessionLaunchPolicy,
    ) -> Result<
        crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        String,
    > {
        use crate::cross_cutting::provider_execution::{
            AuthorToolPolicy, ProviderExecutionProfile, TrustedProviderRole, TrustedRunIdentity,
            freeze_provider_execution,
        };
        if input.role != crate::protocol::contracts::AdapterRole::Orchestrator {
            return Err(format!(
                "author execution contract requires the Orchestrator carrier; got {:?}",
                input.role
            ));
        }
        // engine durable run 事实（crate 内受信 builder 面）：fresh 每轮新
        // run id（合同 resume 摘要不含 run id，续接指纹稳定）。
        let run_record = crate::protocol::contracts::ProviderRunRecord {
            provider_run_id: format!(
                "author_{}_{}",
                self.session.session_id,
                uuid::Uuid::new_v4().simple()
            ),
            node_id: "author_root_launch".to_string(),
            provider_type: input.provider_type.clone(),
            runtime_role: crate::protocol::contracts::RuntimeRole::Orchestrator,
            adapter_role: crate::protocol::contracts::AdapterRole::Orchestrator,
            provider_capability_ref: "cap_managed_snapshot".to_string(),
            adapter_compatibility_ref: "compat_1".to_string(),
            context_package_ref: "ctx_author_root".to_string(),
            adapter_input_ref: "ain_author_root".to_string(),
            adapter_output_ref: "aout_author_root".to_string(),
            raw_artifact_refs: Vec::new(),
            exit_code: None,
            error_code: None,
            error_details: None,
            stdout_ref: None,
            stderr_ref: None,
            structured_output_ref: None,
            files_modified: Vec::new(),
            status: crate::protocol::contracts::ProviderRunStatus::Running,
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
            duration_ms: None,
            timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
            retry_count: 0,
            approval_policy: crate::protocol::contracts::ApprovalPolicy::OnRequest,
            sandbox_mode: crate::protocol::contracts::SandboxMode::FullAccess,
            constraint_check_ref: None,
            traceability_binding_refs: Vec::new(),
        };
        let identity = TrustedRunIdentity::from_durable_run(
            &run_record,
            &self.session.session_id,
            TrustedProviderRole::Author,
        );
        let contract = freeze_provider_execution(
            &identity,
            ProviderExecutionProfile::HostCoding,
            Some(AuthorToolPolicy::author_v1()),
            format!("role_run_author_{}", self.session.session_id),
            None,
        )
        .map_err(|error| format!("author execution contract freeze failed: {}", error.details))?;
        // A1-b1(Bug45 批2 环境回退):author root launch 停止注入环境——
        // caller 的 env_vars 原样透传(仅作显式覆盖层),子进程经底座
        // (process_manager)继承宿主;冻结合同仍绑定 launch。
        // 受信装配独占 deny 投影：按 duty 强制覆盖（与
        // prepare_trusted_streaming_launch 同一口径）。
        input.tool_policy =
            crate::cross_cutting::streaming_provider::tool_policy_for_duty(&identity.duty());
        Ok(
            crate::cross_cutting::session_launch::ValidatedStreamingProviderInput::new(
                input, launch,
            )
            .with_frozen_execution(contract),
        )
    }

    /// Task 2.1（REQ-ENV-01/ENV-10）：LC 会话 Author/ChoiceFollowup 的 root launch
    /// 解析。cwd = 唯一 authority resolver 冻结的 manifest `provider_context_root`
    ///（canonical root）；target = E `native_launch_anchor` 聚合根锚（REQ-PLN-08：
    /// scope 回写不迁移，与 gateway_start.rs 同口径）。经 `gateway.validate`
    /// 冻结 envelope/resume fingerprint。
    ///
    /// 返回值：`None` = 非逻辑会话（未注入 gateway），调用方保持 Legacy 直连
    /// `provider.start` 路径（单仓零变化）；`Some(Err(..))` = 逻辑会话但解析/校验
    /// fail-closed（绝不静默回落 member cwd 直连）；`Some(Ok(..))` = 可启动的
    /// validated launch。
    pub(crate) fn resolve_author_root_launch(
        &self,
        session_record: &crate::product::models::WorkspaceSessionRecord,
    ) -> Option<Result<ValidatedSessionLaunchPolicy, String>> {
        let gateway = self.logical_provider_gateway()?;
        let request = match self.build_root_launch_request(
            session_record,
            &self.session.author_provider.clone(),
            SessionPolicyAction::PlanningReadOnly,
        )? {
            Ok(request) => request,
            Err(message) => return Some(Err(message)),
        };
        Some(
            gateway
                .validate(request)
                .map_err(|error| format!("logical author gateway validation failed: {error}")),
        )
    }

    /// Task 2.3：`resolve_author_root_launch` 的参数化内核——provider ref 与
    /// action 随角色注入（author/revision 共用 `PlanningReadOnly`；错误文案保持
    /// 2.1 的「author」口径，revision 即 author 角色修订面）。解析链（lifecycle
    /// store、E 分离 resolver 双输出（native anchor 聚合根 target +
    /// business 材料面）、authority manifest root、C-2 集中 provider ref
    /// 映射）与 2.1 完全同源；返回未校验的 launch request，供
    /// `validate`（author）或 `resume_or_start`（revision resume 决策）消费。
    pub(crate) fn build_root_launch_request(
        &self,
        session_record: &crate::product::models::WorkspaceSessionRecord,
        provider: &crate::product::models::ProviderName,
        action: SessionPolicyAction,
    ) -> Option<Result<SessionLaunchRequest, String>> {
        self.logical_provider_gateway()?;
        let Some(lifecycle) = self.lifecycle_store.as_ref() else {
            return Some(Err(
                "logical author launch requires a persistent lifecycle store".to_string(),
            ));
        };
        let app_paths = lifecycle.app_paths();
        // B1a（Task 4.1）/E（REQ-PLN-08）：同 resolver 一次派生两用途——
        // `native_launch_anchor` 给 native launch target（恒聚合根视图，
        // scope 回写 involved/focus/change_order 不迁移锚；business 成员
        // 不再进入 target——2026-10-12 design 修订指纹漂移根因修复）；
        // `business_repository` 保留原用途（gateway 注入谓词 fail-closed
        // 与业务材料面：SC compile/language.md 走原字段）。
        let context =
            match crate::product::workspace_repository::resolve_workspace_repository_context(
                &app_paths,
                lifecycle,
                session_record,
            ) {
                Ok(context) => context,
                Err(error) => {
                    return Some(Err(format!(
                        "resolve logical author target repository failed: {error}"
                    )));
                }
            };
        let repository = match context.business_repository {
            Ok(repository) => repository,
            Err(error) => {
                return Some(Err(format!(
                    "resolve logical author target repository failed: {error}"
                )));
            }
        };
        let native_launch_anchor = context.native_launch_anchor;
        // gateway 注入谓词即 `repository.logical_repository_id.is_some()`（manager
        // 工厂）；此处不一致属持久状态漂移——fail-closed，绝不回落 legacy 直连。
        if repository.logical_repository_id.is_none() {
            return Some(Err(
                "logical session repository has no logical identity; refusing legacy direct fallback"
                    .to_string(),
            ));
        }
        let root = match crate::product::logical_codebase::RepositoryAuthorityResolver::new(
            app_paths.clone(),
        )
        .resolve_for_issue(&session_record.project_id, &session_record.issue_id)
        {
            Ok(Some(resolution)) => match resolution.manifest {
                Some(manifest) => manifest.provider_context_root,
                None => {
                    return Some(Err(
                        "logical author launch has no authority manifest; register members first"
                            .to_string(),
                    ));
                }
            },
            Ok(None) => {
                return Some(Err(
                    "logical session has no logical codebase attribution for root launch"
                        .to_string(),
                ));
            }
            Err(error) => {
                return Some(Err(format!(
                    "resolve logical author authority root failed: {error}"
                )));
            }
        };
        let provider_ref = match ProviderRef::from_provider_name(provider, "cap_managed_snapshot") {
            Ok(provider_ref) => provider_ref,
            // C-2：session 配置的 provider 无 gateway 真实 dialect（Pi/KimiCode
            // 等）时显式失败，不静默回退 Claude，也不降级直连。
            Err(error) => {
                return Some(Err(format!(
                    "logical author launch provider ref failed: {error}"
                )));
            }
        };
        // E（REQ-PLN-08）author launch 消费缺口修复（2026-10-12 design
        // 修订指纹漂移现场）：native launch target 一律取
        // `native_launch_anchor`（PlanningReadOnly 的 story/design/plan 恒
        // 聚合根视图，与 gateway_start.rs 同口径）——scope 回写后 business
        // 恢复成员解析也不迁移锚，resume 指纹四漂移域（target logical
        // id/checkout/worktree/git identity）恒定。成员 checkout 不再进入
        // launch target（原 Some(checkout) 臂即漂移根因：生成轮聚合根 vs
        // 修订轮 busi 首成员 target）。
        let target = match native_launch_anchor {
            Ok(anchor) => PolicyTarget::aggregate_root(anchor.path.clone()),
            Err(error) => {
                return Some(Err(format!(
                    "resolve logical author native launch anchor failed: {error}"
                )));
            }
        };
        Some(Ok(SessionLaunchRequest {
            project_id: session_record.project_id.clone(),
            provider: provider_ref,
            action,
            target,
            // root cwd 与聚合根锚 target 显式分离（Task 2.5 字段合同；
            // `planning()` 构造器的 cwd==target 默认不适用于 LC root 形态）。
            working_directory: root.clone(),
            readable_roots: vec![root],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        }))
    }

    /// Task 2.1：web run 臂的 Author 首轮入口——LC 会话先解析 root launch（cwd=
    /// canonical root、target=E `native_launch_anchor` 聚合根锚、
    /// PlanningReadOnly 策略）再驱动；非逻辑会话（None）保持直接
    /// `provider.start` 路径（单仓零变化）。
    pub(crate) async fn handle_user_message_from_run(
        &mut self,
        content: String,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
        session_record: &crate::product::models::WorkspaceSessionRecord,
    ) {
        // B1（终审 P1）：显式 fresh（FreshRunning author 族 recovery）消费点
        //——dispatch 已授权本 run：FullConversation prompt 自足、旧 native 已
        // tombstone 不回捞；驱动结束后对账 recovery（Completed → Resolved /
        // 其余回 Waiting）。普通 run（无 fresh 判据）零变化。
        let fresh_recovery = self.lc_fresh_author_family_recovery();
        let gateway_launch = self.resolve_author_root_launch(session_record);
        self.handle_author_message_with_prompt_mode(
            content,
            provider,
            command_rx,
            AuthorPromptMode::FullConversation,
            gateway_launch,
        )
        .await;
        if fresh_recovery.is_some() {
            self.reconcile_lc_fresh_author_family_run(None).await;
        }
    }

    /// Task 2.1：ChoiceFollowup 复用同一 root launch/envelope/resume——choice
    /// 内容不构造新 target，也不把 follow-up 降为无策略 Executor（DeltaOnly
    /// 字节与 Legacy 路径一致）。
    pub(crate) async fn handle_author_choice_followup_from_run(
        &mut self,
        content: String,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
        session_record: &crate::product::models::WorkspaceSessionRecord,
    ) {
        // B1（终审 P1）：显式 fresh 消费点（同 handle_user_message_from_run）。
        let fresh_recovery = self.lc_fresh_author_family_recovery();
        let gateway_launch = self.resolve_author_root_launch(session_record);
        self.handle_author_message_with_prompt_mode(
            content,
            provider,
            command_rx,
            AuthorPromptMode::DeltaOnly,
            gateway_launch,
        )
        .await;
        if fresh_recovery.is_some() {
            self.reconcile_lc_fresh_author_family_run(None).await;
        }
    }
}
