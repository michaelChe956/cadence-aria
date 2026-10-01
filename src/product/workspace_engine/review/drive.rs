use super::*;
use crate::cross_cutting::provider_adapter::{
    PROVIDER_ERROR_STDERR_TAIL_BYTES, ProviderAdapterError,
};
use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use crate::product::logical_codebase::{
    LogicalCodebaseProviderGateway, PolicyTarget, ProviderGatewayError, ProviderRef,
    SessionLaunchRequest, SessionPolicyAction,
};
use crate::product::workspace_engine::provider_drive::{
    PROVIDER_CHOICE_WAIT_TIMEOUT, PROVIDER_IDLE_WATCHDOG_TIMEOUT, PendingChoiceRequests,
};
use crate::product::workspace_engine::types::ReviewProviderRunFailure;

impl WorkspaceEngine {
    pub async fn drive_review_session(
        &mut self,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
    ) {
        // C2 Task 5（REQ-CRO-05）：reviewer 缺失（空 effective）不驱动 review
        // run——错误事件＋失败收尾，绝不以 Codex 顶替。
        let Some(reviewer) = self.session.reviewer_provider.clone() else {
            let _ = self
                .event_tx
                .send(EngineEvent::Error {
                    message: "reviewer_configuration_missing: review run not started".to_string(),
                })
                .await;
            self.finish_failed_run().await;
            return;
        };
        let input = match self
            .ensure_review_invocation_scope()
            .await
            .and_then(|_| self.build_review_input())
        {
            Ok(input) => input,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                return;
            }
        };
        if let Some(node_id) = self.active_node_id.clone() {
            let _ = self
                .persist_prompt_snapshot(&node_id, input.prompt.clone())
                .await;
            self.emit_execution_event(
                provider_prompt_event(
                    &node_id,
                    input.prompt.clone(),
                    "发送给 Workspace provider 的完整提示词",
                ),
                Some(node_id),
                Some(reviewer.clone()),
            )
            .await;
        }
        let mut command_rx = command_rx;
        // P1-2：reviewer 是策略角色——与 author 主流同法绑定 run-bound durable
        // sink（持久 store 缺失时不接线，真实 adapter 对 policy+缺 sink fail-closed）。
        let input = self.attach_tool_policy_audit(input);
        // F-19：reviewer run 拉起前登记 provider start（诊断面，best-effort）。
        self.register_provider_start_in_ledger(
            ProviderConversationRole::Reviewer,
            reviewer.clone(),
        );
        let first_session = provider.start(input.clone(), self.cancel.clone()).await;
        let first_completion = match self
            .drive_reviewer_provider_session_once(first_session, &mut command_rx, &reviewer)
            .await
        {
            ReviewProviderRunResult::Completed(completion) => completion,
            ReviewProviderRunResult::Aborted => return,
            ReviewProviderRunResult::Failed(failure) => {
                self.finish_review_provider_run_failure(failure).await;
                return;
            }
        };

        match self.parse_review_completion_for_active_node(&first_completion) {
            Ok(verdict) => self.complete_review(first_completion, verdict).await,
            // 分类枚举错误必须直接沿 fallback diagnostic → policy fatal
            // 路径收口，不能进入 JSON repair 或人工 gate。
            Err(first_error) if first_error.is_classification_fatal() => {
                let verdict = fallback_review_verdict(&first_completion, &first_error, false);
                self.complete_review(first_completion, verdict).await;
            }
            // Kimi 仅复用既有的一次 JSON 等值 repair；Pi 仍不进入 repair。
            Err(first_error)
                if provider_allows_review_repair(&reviewer) && first_error.is_repairable() =>
            {
                let repair_input = match self.build_review_repair_input(
                    &input,
                    &first_completion,
                    &first_error,
                    first_completion.provider_session_id.clone(),
                ) {
                    Ok(input) => input,
                    Err(_) => {
                        let verdict =
                            fallback_review_verdict(&first_completion, &first_error, false);
                        self.complete_review(first_completion, verdict).await;
                        return;
                    }
                };
                let repair_node_id = self.active_node_id.clone();
                self.emit_execution_event(
                    structured_output_repair_event(
                        ProviderExecutionEventStatus::Started,
                        first_error.code(),
                    ),
                    repair_node_id.clone(),
                    Some(reviewer.clone()),
                )
                .await;
                let repair_input = self.attach_tool_policy_audit(repair_input);
                let repair_session = provider.start(repair_input, self.cancel.clone()).await;
                let repair_result = self
                    .drive_reviewer_provider_session_once(
                        repair_session,
                        &mut command_rx,
                        &reviewer,
                    )
                    .await;
                let repaired_completion = match repair_result {
                    ReviewProviderRunResult::Completed(completion) => completion,
                    ReviewProviderRunResult::Aborted => {
                        self.emit_execution_event(
                            structured_output_repair_event(
                                ProviderExecutionEventStatus::Failed,
                                first_error.code(),
                            ),
                            repair_node_id,
                            Some(reviewer.clone()),
                        )
                        .await;
                        return;
                    }
                    ReviewProviderRunResult::Failed(_) => {
                        self.emit_execution_event(
                            structured_output_repair_event(
                                ProviderExecutionEventStatus::Failed,
                                first_error.code(),
                            ),
                            repair_node_id,
                            Some(reviewer.clone()),
                        )
                        .await;
                        let verdict =
                            fallback_review_verdict(&first_completion, &first_error, true);
                        self.complete_review(first_completion, verdict).await;
                        return;
                    }
                };

                match self.parse_review_completion_for_active_node(&repaired_completion) {
                    Ok(mut verdict)
                        if repair_payload_is_compatible(&first_error, &repaired_completion) =>
                    {
                        verdict.structured_output_diagnostic =
                            Some(success_diagnostic(&first_error));
                        let normalized = ProviderCompletion {
                            full_output: format!(
                                "{}\n{}",
                                first_completion.full_output, repaired_completion.full_output
                            ),
                            readable_output: first_completion.readable_output,
                            structured_output: repaired_completion.structured_output,
                            provider_session_id: repaired_completion.provider_session_id,
                        };
                        self.emit_execution_event(
                            structured_output_repair_event(
                                ProviderExecutionEventStatus::Completed,
                                first_error.code(),
                            ),
                            repair_node_id.clone(),
                            Some(reviewer.clone()),
                        )
                        .await;
                        self.complete_review(normalized, verdict).await;
                    }
                    Ok(_) => {
                        let error = ReviewCompletionError::RepairPayloadChanged;
                        self.emit_execution_event(
                            structured_output_repair_event(
                                ProviderExecutionEventStatus::Failed,
                                error.code(),
                            ),
                            repair_node_id.clone(),
                            Some(reviewer.clone()),
                        )
                        .await;
                        let verdict = fallback_review_verdict(&first_completion, &error, true);
                        self.complete_review(first_completion, verdict).await;
                    }
                    Err(second_error) => {
                        let normalized = ProviderCompletion {
                            full_output: format!(
                                "{}\n{}",
                                first_completion.full_output, repaired_completion.full_output
                            ),
                            readable_output: first_completion.readable_output,
                            structured_output: repaired_completion.structured_output,
                            provider_session_id: repaired_completion.provider_session_id,
                        };
                        self.emit_execution_event(
                            structured_output_repair_event(
                                ProviderExecutionEventStatus::Failed,
                                second_error.code(),
                            ),
                            repair_node_id,
                            Some(reviewer.clone()),
                        )
                        .await;
                        let verdict = fallback_review_verdict(&normalized, &second_error, true);
                        self.complete_review(normalized, verdict).await;
                    }
                }
            }
            // C2（REQ-HGC-03 场景 2/F-52 R6）：invalid_json 等输出解析失败在
            // 同 invocation 内恰一次静默重试——同一 input 重新拉起 provider
            // （不铸 repair prompt、不发执行事件、不登记 ledger），续用同一
            // provider 会话；成功则正常消费（计入这一次 review），仍失败保
            // 原始 diagnostic 进人工，该轮不计为一次完整 review（routing 侧
            // 对 attempted&&!succeeded 的降级轮零计数）。
            Err(first_error) if first_error.is_output_parse_failure() => {
                let retry_input = StreamingProviderInput {
                    working_directory: None,
                    resume_provider_session_id: first_completion
                        .provider_session_id
                        .clone()
                        .filter(|session_id| !session_id.trim().is_empty()),
                    ..input.clone()
                };
                let retry_session = provider.start(retry_input, self.cancel.clone()).await;
                self.complete_silent_output_retry(
                    &first_completion,
                    &first_error,
                    retry_session,
                    &mut command_rx,
                    &reviewer,
                )
                .await;
            }
            Err(error) => {
                let verdict = fallback_review_verdict(&first_completion, &error, false);
                self.complete_review(first_completion, verdict).await;
            }
        }
    }

    /// 逻辑会话专属 review 驱动:镜像 `drive_review_session`,但三处 provider
    /// 启动点(first + repair)改为经 `logical_provider_gateway` 的
    /// `validate` + `start_streaming` 启动(每次用各自 input 重新组装 launch)。
    ///
    /// 仅在注入 `logical_provider_gateway` 后由 handler 调用(调用前先判
    /// `logical_provider_gateway().is_some()`);None 属编程错误——生产正常序列
    /// 该分支不可达,故 `expect` 直接暴露而非静默吞掉。
    pub(crate) async fn drive_review_session_via_gateway(
        &mut self,
        command_rx: mpsc::Receiver<ProviderCommand>,
    ) {
        let gateway = self
            .logical_provider_gateway
            .clone()
            .expect("drive_review_session_via_gateway requires a logical provider gateway");

        // C2 Task 5（REQ-CRO-05）：reviewer 缺失（空 effective）不驱动 review
        // run——错误事件＋失败收尾，绝不以 Codex 顶替。
        let Some(reviewer) = self.session.reviewer_provider.clone() else {
            let _ = self
                .event_tx
                .send(EngineEvent::Error {
                    message: "reviewer_configuration_missing: review run not started".to_string(),
                })
                .await;
            self.finish_failed_run().await;
            return;
        };
        let input = match self
            .ensure_review_invocation_scope()
            .await
            .and_then(|_| self.build_review_input())
        {
            Ok(input) => input,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                return;
            }
        };
        if let Some(node_id) = self.active_node_id.clone() {
            let _ = self
                .persist_prompt_snapshot(&node_id, input.prompt.clone())
                .await;
            self.emit_execution_event(
                provider_prompt_event(
                    &node_id,
                    input.prompt.clone(),
                    "发送给 Workspace provider 的完整提示词",
                ),
                Some(node_id),
                Some(reviewer.clone()),
            )
            .await;
        }
        // review 会话 repository 数据同 10a 来源:project_id 取
        // `logical_planning_launch()`(repository_path 缺失时回退 session.project_id)。
        let project_id = self
            .logical_planning_launch()
            .map(|(project_id, _)| project_id)
            .unwrap_or_else(|| self.session.project_id.clone());
        let mut command_rx = command_rx;
        // P1-2：gateway review 路径同样绑定 run-bound durable sink（input 进入
        // validated 包装前附着，sink 随 input 原样透传 gateway.start_streaming）。
        let input = self.attach_tool_policy_audit(input);
        // F-19：gateway review 路径同样登记 reviewer provider start（k3 P2——
        // drive_review_session 的镜像，logical repository 会话实际走此分支）。
        self.register_provider_start_in_ledger(
            ProviderConversationRole::Reviewer,
            reviewer.clone(),
        );
        let first_session = start_review_session_via_gateway(
            &gateway,
            &reviewer,
            input.clone(),
            project_id.clone(),
            self.cancel.clone(),
        )
        .await;
        let first_completion = match self
            .drive_reviewer_provider_session_once(first_session, &mut command_rx, &reviewer)
            .await
        {
            ReviewProviderRunResult::Completed(completion) => completion,
            ReviewProviderRunResult::Aborted => return,
            ReviewProviderRunResult::Failed(failure) => {
                self.finish_review_provider_run_failure(failure).await;
                return;
            }
        };

        match self.parse_review_completion_for_active_node(&first_completion) {
            Ok(verdict) => self.complete_review(first_completion, verdict).await,
            // 分类枚举错误必须直接沿 fallback diagnostic → policy fatal
            // 路径收口，不能进入 JSON repair 或人工 gate。
            Err(first_error) if first_error.is_classification_fatal() => {
                let verdict = fallback_review_verdict(&first_completion, &first_error, false);
                self.complete_review(first_completion, verdict).await;
            }
            Err(first_error) if first_error.is_repairable() && reviewer != ProviderName::Pi => {
                let repair_input = match self.build_review_repair_input(
                    &input,
                    &first_completion,
                    &first_error,
                    first_completion.provider_session_id.clone(),
                ) {
                    Ok(input) => input,
                    Err(_) => {
                        let verdict =
                            fallback_review_verdict(&first_completion, &first_error, false);
                        self.complete_review(first_completion, verdict).await;
                        return;
                    }
                };
                let repair_node_id = self.active_node_id.clone();
                self.emit_execution_event(
                    structured_output_repair_event(
                        ProviderExecutionEventStatus::Started,
                        first_error.code(),
                    ),
                    repair_node_id.clone(),
                    Some(reviewer.clone()),
                )
                .await;
                let repair_input = self.attach_tool_policy_audit(repair_input);
                let repair_session = start_review_session_via_gateway(
                    &gateway,
                    &reviewer,
                    repair_input,
                    project_id,
                    self.cancel.clone(),
                )
                .await;
                let repair_result = self
                    .drive_reviewer_provider_session_once(
                        repair_session,
                        &mut command_rx,
                        &reviewer,
                    )
                    .await;
                let repaired_completion = match repair_result {
                    ReviewProviderRunResult::Completed(completion) => completion,
                    ReviewProviderRunResult::Aborted => {
                        self.emit_execution_event(
                            structured_output_repair_event(
                                ProviderExecutionEventStatus::Failed,
                                first_error.code(),
                            ),
                            repair_node_id,
                            Some(reviewer.clone()),
                        )
                        .await;
                        return;
                    }
                    ReviewProviderRunResult::Failed(_) => {
                        self.emit_execution_event(
                            structured_output_repair_event(
                                ProviderExecutionEventStatus::Failed,
                                first_error.code(),
                            ),
                            repair_node_id,
                            Some(reviewer.clone()),
                        )
                        .await;
                        let verdict =
                            fallback_review_verdict(&first_completion, &first_error, true);
                        self.complete_review(first_completion, verdict).await;
                        return;
                    }
                };

                match self.parse_review_completion_for_active_node(&repaired_completion) {
                    Ok(mut verdict)
                        if repair_payload_is_compatible(&first_error, &repaired_completion) =>
                    {
                        verdict.structured_output_diagnostic =
                            Some(success_diagnostic(&first_error));
                        let normalized = ProviderCompletion {
                            full_output: format!(
                                "{}\n{}",
                                first_completion.full_output, repaired_completion.full_output
                            ),
                            readable_output: first_completion.readable_output,
                            structured_output: repaired_completion.structured_output,
                            provider_session_id: repaired_completion.provider_session_id,
                        };
                        self.emit_execution_event(
                            structured_output_repair_event(
                                ProviderExecutionEventStatus::Completed,
                                first_error.code(),
                            ),
                            repair_node_id.clone(),
                            Some(reviewer.clone()),
                        )
                        .await;
                        self.complete_review(normalized, verdict).await;
                    }
                    Ok(_) => {
                        let error = ReviewCompletionError::RepairPayloadChanged;
                        self.emit_execution_event(
                            structured_output_repair_event(
                                ProviderExecutionEventStatus::Failed,
                                error.code(),
                            ),
                            repair_node_id.clone(),
                            Some(reviewer.clone()),
                        )
                        .await;
                        let verdict = fallback_review_verdict(&first_completion, &error, true);
                        self.complete_review(first_completion, verdict).await;
                    }
                    Err(second_error) => {
                        let normalized = ProviderCompletion {
                            full_output: format!(
                                "{}\n{}",
                                first_completion.full_output, repaired_completion.full_output
                            ),
                            readable_output: first_completion.readable_output,
                            structured_output: repaired_completion.structured_output,
                            provider_session_id: repaired_completion.provider_session_id,
                        };
                        self.emit_execution_event(
                            structured_output_repair_event(
                                ProviderExecutionEventStatus::Failed,
                                second_error.code(),
                            ),
                            repair_node_id,
                            Some(reviewer.clone()),
                        )
                        .await;
                        let verdict = fallback_review_verdict(&normalized, &second_error, true);
                        self.complete_review(normalized, verdict).await;
                    }
                }
            }
            // C2（REQ-HGC-03 场景 2）：gateway 路径镜像同一「同 invocation 恰
            // 一次静默重试」契约（同一 input、静默、零记账）。
            Err(first_error) if first_error.is_output_parse_failure() => {
                let retry_input = StreamingProviderInput {
                    working_directory: None,
                    resume_provider_session_id: first_completion
                        .provider_session_id
                        .clone()
                        .filter(|session_id| !session_id.trim().is_empty()),
                    ..input.clone()
                };
                let retry_session = start_review_session_via_gateway(
                    &gateway,
                    &reviewer,
                    retry_input,
                    project_id,
                    self.cancel.clone(),
                )
                .await;
                self.complete_silent_output_retry(
                    &first_completion,
                    &first_error,
                    retry_session,
                    &mut command_rx,
                    &reviewer,
                )
                .await;
            }
            Err(error) => {
                let verdict = fallback_review_verdict(&first_completion, &error, false);
                self.complete_review(first_completion, verdict).await;
            }
        }
    }

    pub async fn drive_revision_session(
        &mut self,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
    ) {
        let author = self.session.author_provider.clone();
        let node_id = self.active_node_id.clone();
        let input = match self.build_revision_input() {
            Ok(input) => input,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                return;
            }
        };
        if let Some(node_id) = node_id.clone() {
            let _ = self
                .persist_prompt_snapshot(&node_id, input.prompt.clone())
                .await;
            self.emit_execution_event(
                provider_prompt_event(
                    &node_id,
                    input.prompt.clone(),
                    "发送给 Workspace provider 的完整提示词",
                ),
                Some(node_id),
                Some(author.clone()),
            )
            .await;
        }
        // 第一阶段不实证 Kimi resume 稳定性，排除 artifact retry（同 Pi）
        let retry_context =
            crate::product::workspace_engine::provider_drive::provider_allows_artifact_retry(
                &self.session.author_provider,
            )
            .then(|| ArtifactRetryContext {
                provider: provider.clone(),
                input: input.clone(),
                attempted: false,
            });
        let revision_resume_fallback = if input.resume_provider_session_id.is_some()
            && self.session.author_provider == ProviderName::Codex
        {
            Some(RevisionResumeFallbackContext {
                provider: provider.clone(),
                attempted: false,
            })
        } else {
            None
        };
        let input = self.attach_tool_policy_audit(input);
        // F-19：revision run 拉起前登记 provider start（诊断面，best-effort）。
        self.register_provider_start_in_ledger(ProviderConversationRole::Author, author.clone());
        let session = provider.start(input, self.cancel.clone()).await;
        self.drive_provider_session(ProviderSessionDriveInput {
            session,
            command_rx,
            node_id,
            agent: Some(author),
            role: ProviderConversationRole::Author,
            artifact_retry: retry_context,
            revision_resume_fallback,
        })
        .await;
    }
}

include!("drive_parts/reviewer_provider_session.inc.rs");

fn reviewer_failure_diagnostic(
    code: Option<crate::protocol::provider_errors::ProviderErrorCode>,
    message: &str,
) -> String {
    if code == Some(crate::protocol::provider_errors::ProviderErrorCode::ProviderUnavailable)
        && message.contains("503")
        && message.contains("No available accounts")
    {
        "provider_gateway_503_no_accounts: 推理网关账号池不可用；检查服务后手动重驱".to_string()
    } else {
        "provider_reviewer_failed: 评审运行失败；核对 provider 后手动重驱".to_string()
    }
}

fn provider_allows_review_repair(provider: &ProviderName) -> bool {
    !matches!(provider, ProviderName::Pi)
}

/// 逻辑 review 会话经 gateway 启动:组装 `ReviewReadOnly` launch → `validate` →
/// `start_streaming`。launch 的 provider ref 由调用方传入的 reviewer
/// (`session.reviewer_provider`)经集中映射 `ProviderRef::from_provider_name` 派生
/// (C-2:不再硬编码 ClaudeCode;不支持的 provider 显式失败)。gateway 错误映射为
/// `ProviderAdapterError`(与 `drive_reviewer_provider_session_once` 的 `Start`
/// 失败路径对齐)。
async fn start_review_session_via_gateway(
    gateway: &Arc<LogicalCodebaseProviderGateway>,
    reviewer: &ProviderName,
    input: StreamingProviderInput,
    project_id: String,
    cancel: CancellationToken,
) -> Result<ProviderSession, ProviderAdapterError> {
    let request = SessionLaunchRequest {
        project_id,
        provider: ProviderRef::from_provider_name(reviewer, "cap_managed_snapshot")
            .map_err(map_gateway_error_to_adapter)?,
        action: SessionPolicyAction::ReviewReadOnly,
        target: PolicyTarget::aggregate_root(input.working_dir.clone()),
        // Task 2.5：独立 cwd 字段；现状映射 target（aggregate root）worktree，
        // cwd==target 复验等式不变。
        working_directory: input.working_dir.clone(),
        readable_roots: vec![input.working_dir.clone()],
        writable_roots: Vec::new(),
        config_artifact_ref: "sha256:managed-config-artifact".to_string(),
    };
    let validated = gateway
        .validate(request)
        .map_err(map_gateway_error_to_adapter)?;
    let validated_input = ValidatedStreamingProviderInput::new(input, validated);
    gateway
        .start_streaming(validated_input, cancel)
        .await
        .map_err(map_gateway_error_to_adapter)
}

/// gateway 错误到 adapter 错误的桥接:使调用点后续对 `Err` 的处理与直接
/// `provider.start` 完全一致(`ReviewProviderRunFailure::Start`)。
/// 诊断直通（claude×轻 握手谜团第 2 轮）:Adapter 变体不再丢弃 stderr 字段
/// ——stderr 尾部（有界）并入 details（随 Start 失败 → EngineEvent::Error →
/// WS error 消息上浮），stderr 字段同源保留；其余 gateway 校验错误维持 Display
/// 文案不变（零变化）。
fn map_gateway_error_to_adapter(error: ProviderGatewayError) -> ProviderAdapterError {
    match &error {
        ProviderGatewayError::Adapter(inner) => {
            let mut mapped = ProviderAdapterError::provider_unavailable(error.to_string());
            ProviderAdapterError::append_bounded_stderr_tail(
                &mut mapped.details,
                &inner.stderr,
                PROVIDER_ERROR_STDERR_TAIL_BYTES,
            );
            mapped.stderr = inner.stderr.clone();
            mapped
        }
        _ => ProviderAdapterError::provider_unavailable(error.to_string()),
    }
}
#[cfg(test)]
#[path = "drive_tests.rs"]
mod tests;
