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
        if input.tool_policy.is_none() || input.audit_sink.is_some() {
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
        let session = gateway
            .start_streaming(validated_input, self.cancel.clone())
            .await
            .map_err(|error| {
                // 诊断直通（claude×轻 握手谜团第 2 轮）：不丢弃 adapter stderr——
                // 尾部（有界）并入 details，stderr 字段同源保留（其余 gateway 校验
                // 错误维持 Display 文案）。同 review/drive.rs 的映射约定。
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
    }

    /// Task 2.1（REQ-ENV-01/ENV-10）：LC 会话 Author/ChoiceFollowup 的 root launch
    /// 解析。cwd = 唯一 authority resolver 冻结的 manifest `provider_context_root`
    ///（canonical root）；target = 与生产建链同源的 session 路由唯一成员 checkout
    ///（`workspace_repository_for_session`：Story focus/Design involved/Plan
    /// selection），resolved 逻辑身份成对时显式 `PolicyTarget::checkout`。经
    /// `gateway.validate` 冻结 envelope/resume fingerprint。
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
        let Some(lifecycle) = self.lifecycle_store.as_ref() else {
            return Some(Err(
                "logical author launch requires a persistent lifecycle store".to_string(),
            ));
        };
        let app_paths = lifecycle.app_paths();
        let repository =
            match crate::product::workspace_repository::workspace_repository_for_session(
                &app_paths,
                lifecycle,
                session_record,
            ) {
                Ok(repository) => repository,
                Err(error) => {
                    return Some(Err(format!(
                        "resolve logical author target repository failed: {error}"
                    )));
                }
            };
        // gateway 注入谓词即 `repository.logical_repository_id.is_some()`（manager
        // 工厂）；此处不一致属持久状态漂移——fail-closed，绝不回落 legacy 直连。
        let Some(logical_repository_id) = repository.logical_repository_id.clone() else {
            return Some(Err(
                "logical session repository has no logical identity; refusing legacy direct fallback"
                    .to_string(),
            ));
        };
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
        let provider_ref = match ProviderRef::from_provider_name(
            &self.session.author_provider,
            "cap_managed_snapshot",
        ) {
            Ok(provider_ref) => provider_ref,
            // C-2：session 配置的 provider 无 gateway 真实 dialect（Pi/KimiCode
            // 等）时显式失败，不静默回退 Claude，也不降级直连。
            Err(error) => {
                return Some(Err(format!(
                    "logical author launch provider ref failed: {error}"
                )));
            }
        };
        let target = match repository.primary_checkout_id.clone() {
            Some(checkout_id) => PolicyTarget::checkout(
                logical_repository_id.0.to_string(),
                checkout_id.0.to_string(),
                repository.path.clone(),
            ),
            // 身份不可得时镜像 gateway_start.rs 的 deferred 口径退 aggregate_root
            // 锚（cwd/target 分离语义不受影响：cwd 恒为 root）。
            None => PolicyTarget::aggregate_root(repository.path.clone()),
        };
        let request = SessionLaunchRequest {
            project_id: session_record.project_id.clone(),
            provider: provider_ref,
            action: SessionPolicyAction::PlanningReadOnly,
            target,
            // root cwd 与成员 target 显式分离（Task 2.5 字段合同；`planning()`
            // 构造器的 cwd==target 默认不适用于 LC root 形态）。
            working_directory: root.clone(),
            readable_roots: vec![root],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        };
        Some(
            gateway
                .validate(request)
                .map_err(|error| format!("logical author gateway validation failed: {error}")),
        )
    }

    /// Task 2.1：web run 臂的 Author 首轮入口——LC 会话先解析 root launch（cwd=
    /// canonical root、target=成员 checkout、PlanningReadOnly 策略）再驱动；
    /// 非逻辑会话（None）保持直接 `provider.start` 路径（单仓零变化）。
    pub(crate) async fn handle_user_message_from_run(
        &mut self,
        content: String,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
        session_record: &crate::product::models::WorkspaceSessionRecord,
    ) {
        let gateway_launch = self.resolve_author_root_launch(session_record);
        self.handle_author_message_with_prompt_mode(
            content,
            provider,
            command_rx,
            AuthorPromptMode::FullConversation,
            gateway_launch,
        )
        .await;
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
        let gateway_launch = self.resolve_author_root_launch(session_record);
        self.handle_author_message_with_prompt_mode(
            content,
            provider,
            command_rx,
            AuthorPromptMode::DeltaOnly,
            gateway_launch,
        )
        .await;
    }
}
