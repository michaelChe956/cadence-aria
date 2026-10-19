use super::*;
use crate::cross_cutting::provider_adapter::{
    PROVIDER_ERROR_STDERR_TAIL_BYTES, ProviderAdapterError,
};
use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use crate::product::logical_codebase::provider_gateway::{
    GatewaySessionDisposition, ResumeSessionLaunchRequest,
};
use crate::product::logical_codebase::{
    LogicalCodebaseProviderGateway, PolicyTarget, ProviderGatewayError, ProviderRef,
    SessionLaunchRequest, SessionPolicyAction, ValidatedSessionLaunchPolicy,
};
use crate::product::models::{LcRecoveryBusinessContext, LcRecoveryRunContext};
use crate::product::workspace_engine::provider_drive::{
    PROVIDER_CHOICE_WAIT_TIMEOUT, PROVIDER_IDLE_WATCHDOG_TIMEOUT, PendingChoiceRequests,
};
use crate::product::workspace_engine::types::ReviewProviderRunFailure;

impl WorkspaceEngine {
    /// 批2 Task 20b(决策项⑦):公共入口——reviewer 解析后、一切材料采集
    /// (ensure_review_invocation_scope/build_review_input/load_plan_review_
    /// context)之前**无条件开窗**;失败 fail-closed 不以无窗继续;轮次
    /// 结束(成功/失败/中止)统一技术收口(干净恢复+释放;违规留人工门)。
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
        // T20b:开窗早于材料采集(生产入口无条件;测试内层直调走
        // `drive_review_session_with_window`,无窗形态=内层测试 seam)。
        // 修复轮 1(F1):窗口目标经聚合根视图感知解析(成员 checkout/
        // direct 原样;聚合视图落唯一/focus 成员;无唯一成员=专用码)。
        let (review_target, primary_anchored) = match self.plan_review_window_worktree() {
            Ok(resolved) => resolved,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                return;
            }
        };
        if primary_anchored {
            Self::emit_primary_member_anchor_note(
                &self.event_tx,
                &self.session.session_id,
                &review_target,
            )
            .await;
        }
        if let Err(message) = self
            .begin_plan_review_window(&reviewer, review_target)
            .await
        {
            let _ = self.event_tx.send(EngineEvent::Error { message }).await;
            self.finish_failed_run().await;
            return;
        }
        self.drive_review_session_with_window(provider, command_rx)
            .await;
        // 统一收口(覆盖成功/失败/中止一切退出;内层已收口时幂等)。
        self.close_plan_review_window("plan_review_completed").await;
    }

    /// 内层承载(批2 Task 20b):公共入口开窗后进入;测试面可直接调用
    /// (无活动窗=测试 seam,真实 adapter 由四家启动检查/网关复验拒绝)。
    /// 每次 provider 启动前同源复验窗口 Running。
    pub(crate) async fn drive_review_session_with_window(
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
        // T20b(决策项⑦):provider 启动前同源复验——窗口 Running 且材料
        // 与 baseline 同指纹(fail-closed,不无窗启动)。
        if let Err(message) = self.assert_plan_review_window() {
            let _ = self.event_tx.send(EngineEvent::Error { message }).await;
            self.finish_failed_run().await;
            return;
        }
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
                // T20b:repair 入口同窗复验(恢复成功不洗白;窗口失效
                // 即停,不换窗续跑)。
                if let Err(message) = self.assert_plan_review_window() {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.finish_failed_run().await;
                    return;
                }
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
                // T20b:静默重试同窗复验(不换窗、不清窗)。
                if let Err(message) = self.assert_plan_review_window() {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.finish_failed_run().await;
                    return;
                }
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
    /// 批2 Task 20b(决策项⑦):gateway 版公共入口——同 direct 版,
    /// 材料采集前无条件开窗,轮次结束统一收口。
    pub(crate) async fn drive_review_session_via_gateway(
        &mut self,
        command_rx: mpsc::Receiver<ProviderCommand>,
    ) {
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
        let (review_target, primary_anchored) = match self.plan_review_window_worktree() {
            Ok(resolved) => resolved,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                return;
            }
        };
        if primary_anchored {
            Self::emit_primary_member_anchor_note(
                &self.event_tx,
                &self.session.session_id,
                &review_target,
            )
            .await;
        }
        if let Err(message) = self
            .begin_plan_review_window(&reviewer, review_target)
            .await
        {
            let _ = self.event_tx.send(EngineEvent::Error { message }).await;
            self.finish_failed_run().await;
            return;
        }
        self.drive_review_session_via_gateway_with_window(command_rx)
            .await;
        self.close_plan_review_window("plan_review_completed").await;
    }

    /// gateway 版内层承载(测试 seam 同 direct 版)。
    pub(crate) async fn drive_review_session_via_gateway_with_window(
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
        // T20b(决策项⑦):gateway 启动前同源复验(窗口 Running+指纹一致)。
        if let Err(message) = self.assert_plan_review_window() {
            let _ = self.event_tx.send(EngineEvent::Error { message }).await;
            self.finish_failed_run().await;
            return;
        }
        // F-19：gateway review 路径同样登记 reviewer provider start（k3 P2——
        // drive_review_session 的镜像，logical repository 会话实际走此分支）。
        self.register_provider_start_in_ledger(
            ProviderConversationRole::Reviewer,
            reviewer.clone(),
        );
        let (first_session, first_trust_waiting) = start_review_session_via_gateway(
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
                // 指引卡简化补缺：trust 分型命中时，紧随 error 事件之后
                // 发出结构化指导（启动成功恒 None，其余 Failed 形态同样
                // 零发出）。
                self.emit_launch_trust_guidance(first_trust_waiting).await;
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
                // 修复轮：首发已成功启动（trust 已复验通过），结构化等待
                // 恒 None——解包丢弃。
                // T20b:repair 入口同窗复验。
                if let Err(message) = self.assert_plan_review_window() {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.finish_failed_run().await;
                    return;
                }
                let (repair_session, _repair_trust_waiting) = start_review_session_via_gateway(
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
                // 静默重试：首发已成功启动（trust 已复验通过），结构化等待
                // 恒 None——解包丢弃。
                // T20b:静默重试同窗复验。
                if let Err(message) = self.assert_plan_review_window() {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.finish_failed_run().await;
                    return;
                }
                let (retry_session, _retry_trust_waiting) = start_review_session_via_gateway(
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
        let mut node_id = self.active_node_id.clone();
        // B1（终审 P1，B1→B2 接缝）：显式 fresh 消费判据以 durable 为准——
        // dispatch（claim 受理）推进的是 durable record 的 FreshRunning，
        // 长等待/重连的 engine 内存镜像可能仍停在 Waiting。author 族 tag +
        // FreshRunning 即本 run 的授权凭证（入站门/engine 守卫已拒全部普通
        // 旁路），本 driver 消费 fresh 而非继续等待。
        let fresh_recovery = self.lc_fresh_author_family_recovery();
        // A2b（Task 2.6，设计 §4.3/§4.5）：active LC 恢复等待期间，同一
        // revision driver 不得绕过显式 fresh 授权启动（A/B 未齐前旁路一律
        // 后端拒收）——幂等收口后上浮 typed 等待错误，零 provider 启动。
        // B1（终审 P1）：FreshRunning author 族 recovery 的本 run 即已授权
        // 的显式 fresh（上方判据），不落入等待臂。
        if self.lc_resume_recovery_active() && fresh_recovery.is_none() {
            let (reason_code, recovery_id) = self
                .session
                .lc_resume_recovery
                .as_ref()
                .map(|recovery| (recovery.reason_code.clone(), recovery.recovery_id.clone()))
                .unwrap_or_default();
            self.reconcile_lc_resume_waiting().await;
            let message = format!(
                "logical revision blocked by lc resume recovery ({reason_code}): recovery {recovery_id} is waiting for an explicit fresh and no provider was started; start a new generation explicitly"
            );
            let _ = self.event_tx.send(EngineEvent::Error { message }).await;
            return;
        }
        // B1（终审 P1）：fresh 重入准备——冻结 feedback 原样透传（重连重建
        // 丢失内存态 pending_revision_context 时不重算、不丢）；等待收口已
        // 终态化旧修订节点并回 PrepareContext，本 run 重建全新修订节点
        //（retry-of 失败节点）并回 Running 阶段（完成后经
        // complete_assistant_message 回 AuthorConfirm 确认门）。
        if let Some(recovery) = fresh_recovery.as_ref() {
            let frozen_feedback = match &recovery.run_context {
                crate::product::models::LcRecoveryRunContext::AuthorFeedback {
                    original_feedback,
                    ..
                }
                | crate::product::models::LcRecoveryRunContext::ReviewerRevision {
                    original_feedback,
                    ..
                } => original_feedback.clone(),
                _ => None,
            };
            if let Some(feedback) = frozen_feedback.clone() {
                self.pending_revision_context = Some(feedback);
            }
            self.transition_stage(WorkspaceStage::Running).await;
            let retry_attempt = self
                .timeline_nodes
                .iter()
                .filter(|node| {
                    node.retry.as_ref().is_some_and(|retry| {
                        retry.retry_of_node_id == recovery.failed_provider_node_id
                    })
                })
                .count() as u32
                + 1;
            let fresh_node_id = self
                .create_timeline_node_with_retry(
                    TimelineNodeDraft {
                        node_type: TimelineNodeType::Revision,
                        agent: Some(author.clone()),
                        stage: WorkspaceStage::Running,
                        round: None,
                        title: "显式 fresh 修订".to_string(),
                        summary: frozen_feedback.clone(),
                        status: TimelineNodeStatus::Active,
                    },
                    Some(TimelineNodeRetry {
                        retry_of_node_id: recovery.failed_provider_node_id.clone(),
                        retry_attempt,
                        retry_reason: "lc_resume_explicit_fresh".to_string(),
                        retry_error: TimelineNodeRetryError {
                            code: "lc_resume_recovery_fresh".to_string(),
                            message: "native 会话漂移，显式 fresh 重启修订 run".to_string(),
                        },
                    }),
                )
                .await;
            if let Some(feedback) = frozen_feedback {
                let _ = self
                    .update_node_detail(&fresh_node_id, |detail| {
                        detail.revision_feedback = Some(feedback);
                    })
                    .await;
            }
            node_id = Some(fresh_node_id);
        }
        // Task 2.3（REQ-PLN-03/PLN-07、REQ-ENV-04/ENV-10）：LC revision 的 root
        // launch + resume 决策先于 input 构建——delta/full prompt 分流依赖 resume
        // 决策（cwd 漂移 supersede 后不得再以 delta 续写旧 thread）。`None` = 非
        // 逻辑会话（未注入 gateway）→ 下方 Legacy 直连原样（单仓零变化）。
        let logical_launch = if fresh_recovery.is_some() {
            // B1（终审 P1）：显式 fresh 不进 resume_or_start 的 supersede 决策
            //（本 run 即授权的 StartNew），直接 validate 冻结全新 launch。
            self.resolve_revision_root_launch_fresh()
        } else {
            self.resolve_revision_root_launch()
        };

        // A2b（Task 2.6，设计 §4.3）：StartNew(supersede) 走**真实等待收口**
        // ——先持久 superseded waiting record（安全 record 先落盘，阻止旧
        // resume 与旁路 spawn），再清理被取代旧会话的 native 引用（显式
        // fresh 不再携带旧 resume id），再由 `reconcile_lc_resume_waiting`
        // 幂等终态化旧 provider 节点/清 active/收口 run 阶段；同一 driver
        // 不得继续 start_streaming（零 provider 启动），显式 fresh 面归 B1b
        // 的 `resume_recovery` 入参。写失败 typed 传播、不吞（§4.3），此时
        // 按原失败收口可见回 PrepareContext，仍不启动任何 provider。
        if let Some(Ok(decision)) = logical_launch.as_ref()
            && let Some(superseded) = decision.superseded.as_ref()
        {
            let context = LcRecoveryRunContext::ReviewerRevision {
                business: self.lc_recovery_business_context(),
                original_feedback: self.pending_revision_context.clone(),
            };
            match self
                .persist_lc_superseded_wait(context, &superseded.waiting.reason_code)
                .await
            {
                Ok(record) => {
                    self.clear_superseded_provider_session(
                        ProviderConversationRole::Author,
                        &author,
                        &superseded.superseded_session_id,
                    )
                    .await;
                    self.reconcile_lc_resume_waiting().await;
                    let message = format!(
                        "logical revision resume superseded ({}): the old native session {} was marked superseded (recovery {}) and no provider was started; start a new generation explicitly",
                        superseded.waiting.reason_code,
                        superseded.superseded_session_id,
                        record.recovery_id
                    );
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                }
                Err(message) => {
                    let _ = self
                        .event_tx
                        .send(EngineEvent::Error {
                            message: format!("persist lc superseded wait failed: {message}"),
                        })
                        .await;
                    self.finish_failed_run().await;
                }
            }
            return;
        }
        // B1（终审 P1）：显式 fresh 恒走 B1a 的 full-fresh 合同
        //（allow_resume=false → resume=None + full prompt 自足）。
        let allow_resume = fresh_recovery.is_none()
            && logical_launch
                .as_ref()
                .is_none_or(|decision| decision.as_ref().map(|d| d.resume_allowed) == Ok(true));
        let input = match self.build_revision_input_with_resume(allow_resume) {
            Ok(input) => input,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                // 复审 P2：驱动前失败必须对账 recovery——回 Waiting +
                // operation Failed（fail-safe），不得滞留 FreshRunning 令后续
                // claim 恒 Busy。
                if fresh_recovery.is_some() {
                    let reconcile_node = node_id.clone().or_else(|| {
                        fresh_recovery
                            .as_ref()
                            .and_then(|recovery| recovery.fresh_node_id.clone())
                    });
                    self.reconcile_lc_fresh_author_family_run(reconcile_node.as_deref())
                        .await;
                }
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
        // Task 2.3：LC 分支——cwd 重绑 root（envelope 冻结的 canonical
        // working_directory）；input.working_dir 保持业务材料锚（成员
        // checkout），envelope target 恒 E native anchor 聚合根锚；
        // 经 gateway start_streaming 启动（validate→spawn 复验 + audit）。解析/
        // 校验 fail-closed 与 input 构建失败同形收口，绝不静默回落 member cwd
        // 直连。artifact retry 与 Codex resume fallback 属直连专用路径，LC 下真实
        // 启动唯一经 gateway——两者不接线（失败可见收口，不静默换道）。
        if let Some(decision) = logical_launch {
            let decision = match decision {
                Ok(decision) => decision,
                Err(message) => {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.finish_failed_run().await;
                    // 复审 P2：驱动前（launch 决策 Err）同 input 构建失败——
                    // fresh 判据下对账回 Waiting，不滞留 FreshRunning。
                    if fresh_recovery.is_some() {
                        let reconcile_node = node_id.clone().or_else(|| {
                            fresh_recovery
                                .as_ref()
                                .and_then(|recovery| recovery.fresh_node_id.clone())
                        });
                        self.reconcile_lc_fresh_author_family_run(reconcile_node.as_deref())
                            .await;
                    }
                    return;
                }
            };
            let gateway = self
                .logical_provider_gateway
                .clone()
                .expect("logical revision launch requires a logical provider gateway");
            self.logical_launch_fingerprints.insert(
                self.session.author_provider.clone(),
                decision.launch.fingerprint().clone(),
            );
            let mut input = input;
            input.working_directory = Some(decision.launch.envelope().working_directory.clone());
            let input = self.attach_tool_policy_audit(input);
            // F-19：LC revision run 拉起前登记 provider start（诊断面，best-effort）。
            self.register_provider_start_in_ledger(
                ProviderConversationRole::Author,
                author.clone(),
            );
            // 批2 Task 11（硬约束2·resume 同口径）：revision 续接与 fresh
            // 首轮绑定同一 duty=Author 执行合同（bind_author_execution_
            // contract），resume 判定以未装饰投影比对（T10），绑定不引起
            // supersede；绑定失败与 launch 决策 Err 同形收口。
            let validated_input = match self.bind_author_execution_contract(input, decision.launch)
            {
                Ok(validated) => validated,
                Err(message) => {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.finish_failed_run().await;
                    if fresh_recovery.is_some() {
                        let reconcile_node = node_id.clone().or_else(|| {
                            fresh_recovery
                                .as_ref()
                                .and_then(|recovery| recovery.fresh_node_id.clone())
                        });
                        self.reconcile_lc_fresh_author_family_run(reconcile_node.as_deref())
                            .await;
                    }
                    return;
                }
            };
            // 指引卡简化补缺：分类桥结构化变体携带同源等待投影，
            // `trust_waiting` 随驱动收尾紧随 error 事件发出结构化指导
            // （`EngineEvent::TrustMissingGuidance`）；非 trust 分型为 None。
            let (session, revision_trust_waiting) = match gateway
                .start_streaming(validated_input, self.cancel.clone())
                .await
            {
                Ok(session) => (Ok(session), None),
                Err(error) => {
                    let (mapped, trust_waiting) =
                        map_gateway_error_to_adapter_with_trust(&gateway, &author, &error);
                    (Err(mapped), trust_waiting)
                }
            };
            self.drive_provider_session(ProviderSessionDriveInput {
                session,
                command_rx,
                node_id: node_id.clone(),
                agent: Some(author),
                role: ProviderConversationRole::Author,
                artifact_retry: None,
                revision_resume_fallback: None,
            })
            .await;
            self.emit_launch_trust_guidance(revision_trust_waiting)
                .await;
            // recovery（Completed → Resolved；其余回 Waiting，fail-safe）。
            if fresh_recovery.is_some() {
                let reconcile_node = node_id.clone().or_else(|| {
                    fresh_recovery
                        .as_ref()
                        .and_then(|recovery| recovery.fresh_node_id.clone())
                });
                self.reconcile_lc_fresh_author_family_run(reconcile_node.as_deref())
                    .await;
            }
            return;
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
            node_id: node_id.clone(),
            agent: Some(author),
            role: ProviderConversationRole::Author,
            artifact_retry: retry_context,
            revision_resume_fallback,
        })
        .await;
        // B1（终审 P1）：fresh 判据在直连臂同样对账（fresh 出生恒 LC，此处
        // 为守卫完备性——非 fresh 时 reconcile 幂等返回）。
        if fresh_recovery.is_some() {
            let reconcile_node = node_id.clone().or_else(|| {
                fresh_recovery
                    .as_ref()
                    .and_then(|recovery| recovery.fresh_node_id.clone())
            });
            self.reconcile_lc_fresh_author_family_run(reconcile_node.as_deref())
                .await;
        }
    }

    /// Task 2.3：LC revision 的 root launch + resume 决策（`drive_revision_session`
    /// 的 LC 分支消费）。launch 与 Task 2.1 author 首轮同源（cwd = authority
    /// manifest `provider_context_root`；target = E `native_launch_anchor`
    /// 聚合根锚，REQ-PLN-08：scope 回写不迁移；`PlanningReadOnly` + author
    /// provider ref），envelope/resume 面一致。
    ///
    /// resume 决策（REQ-ENV-04 cwd 维度）：input 携带旧 native session 且引擎
    /// 记有其发行 launch 的 cwd-inclusive 指纹时，经 `gateway.resume_or_start`
    /// 全维度比对——指纹一致（cwd 未漂移）→ `resume_allowed=true`，native
    /// session identity 原样续接；漂移 → supersede 审计 + StartNew
    ///（`resume_allowed=false`，丢 resume id 走 full prompt 新 thread）。指纹
    /// 记忆缺失（跨连接重建 engine）→ fail-closed 新会话：无法证明旧 thread 与
    /// 当前 launch 全维度一致，绝不静默续接。
    pub(crate) fn resolve_revision_root_launch(
        &self,
    ) -> Option<Result<RevisionLaunchDecision, String>> {
        self.resolve_revision_root_launch_with_freshness(false)
    }

    /// B1（终审 P1）：`force_fresh=true` —— 显式 fresh run（FreshRunning
    /// author 族 recovery spawn 的本 run）的 launch 决策：不进
    /// `resume_or_start` 的 supersede 决策（本 run 即授权的 StartNew，旧
    /// native 已 tombstone 不回捞），直接 `gateway.validate` 冻结全新
    /// launch；`resume_allowed=false` 供调用方按 B1a 合同强制 resume=None
    /// 的 full-fresh prompt。
    fn resolve_revision_root_launch_fresh(&self) -> Option<Result<RevisionLaunchDecision, String>> {
        self.resolve_revision_root_launch_with_freshness(true)
    }

    fn resolve_revision_root_launch_with_freshness(
        &self,
        force_fresh: bool,
    ) -> Option<Result<RevisionLaunchDecision, String>> {
        let gateway = self.logical_provider_gateway()?;
        let Some(lifecycle) = self.lifecycle_store.as_ref() else {
            return Some(Err(
                "logical revision launch requires a persistent lifecycle store".to_string(),
            ));
        };
        let record = match lifecycle.get_workspace_session(&self.session.session_id) {
            Ok(record) => record,
            // B1a（Task 4.1）：record 读取错误不再 `.ok()` 吞错——底层
            // 类别（product_store_io/json/identity）随消息保留传播。
            Err(error) => {
                return Some(Err(format!(
                    "logical revision launch cannot load workspace session record {}: {error}",
                    self.session.session_id
                )));
            }
        };
        let provider = self.session.author_provider.clone();
        let request = match self.build_root_launch_request(
            &record,
            &provider,
            SessionPolicyAction::PlanningReadOnly,
        )? {
            Ok(request) => request,
            Err(message) => return Some(Err(message)),
        };
        // B1a（Task 4.1，BP2）：candidateErr 即 `Some(Err(..))`（typed 类别
        // 保留），不落 `(None, _)` validate/fresh 臂；成功候选保留 native/seq
        // （seq 与 durable fullmap/audit 联合绑定已在候选内校验）。
        // B1（终审 P1）：fresh 直接 validate 全新 launch——不查旧 candidate
        //（tombstone 候选的 FrozenBindingUnavailable 不阻断授权 fresh）、
        // 不进 resume_or_start 的 supersede 决策。
        let decision = if force_fresh {
            match gateway.validate(request) {
                Ok(launch) => RevisionLaunchDecision {
                    launch,
                    resume_allowed: false,
                    superseded: None,
                },
                Err(error) => {
                    return Some(Err(format!(
                        "logical revision gateway validation failed: {error}"
                    )));
                }
            }
        } else {
            let resume_session_id = match self
                .provider_resume_session_candidate(ProviderConversationRole::Author, &provider)
            {
                Ok(candidate) => candidate
                    .map(|candidate| candidate.native_id)
                    .filter(|id| !id.trim().is_empty()),
                Err(error) => {
                    return Some(Err(format!(
                        "logical revision resume candidate unavailable: {error}"
                    )));
                }
            };
            match (
                resume_session_id,
                self.logical_launch_fingerprints.get(&provider),
            ) {
                (Some(previous_session_id), Some(previous_fingerprint)) => {
                    match gateway.resume_or_start(ResumeSessionLaunchRequest {
                        launch: request,
                        previous_fingerprint: previous_fingerprint.clone(),
                        previous_session_id,
                    }) {
                        Ok(GatewaySessionDisposition::Resume(launch)) => RevisionLaunchDecision {
                            launch,
                            resume_allowed: true,
                            superseded: None,
                        },
                        // Task 9c:StartNew 携带 superseded session id 与稳定等待
                        // 投影——同一 revision driver 不得据此继续 start_streaming。
                        Ok(GatewaySessionDisposition::StartNew {
                            validated,
                            superseded_session_id,
                            waiting,
                        }) => RevisionLaunchDecision {
                            launch: validated,
                            resume_allowed: false,
                            superseded: Some(SupersededRevisionWait {
                                superseded_session_id,
                                waiting,
                            }),
                        },
                        Err(error) => {
                            return Some(Err(format!(
                                "logical revision gateway validation failed: {error}"
                            )));
                        }
                    }
                }
                // 指纹记忆缺失（跨连接重建）：fail-closed 新会话。
                (Some(_), None) => match gateway.validate(request) {
                    Ok(launch) => RevisionLaunchDecision {
                        launch,
                        resume_allowed: false,
                        superseded: None,
                    },
                    Err(error) => {
                        return Some(Err(format!(
                            "logical revision gateway validation failed: {error}"
                        )));
                    }
                },
                (None, _) => match gateway.validate(request) {
                    Ok(launch) => RevisionLaunchDecision {
                        launch,
                        resume_allowed: true,
                        superseded: None,
                    },
                    Err(error) => {
                        return Some(Err(format!(
                            "logical revision gateway validation failed: {error}"
                        )));
                    }
                },
            }
        };
        Some(Ok(decision))
    }

    /// A2b（Task 2.6）：LC 等待 record 的业务上下文快照——engine 已知身份
    /// 全量携带；未持久化的身份（业务 repository 物理/logical id、source
    /// story/design ids、role_run_seq）按 A1a 模型纪律留 None 不倒算
    /// （covered-run fresh handler 由 B1b/B2 按 run kind 补全重建事实）。
    fn lc_recovery_business_context(&self) -> LcRecoveryBusinessContext {
        LcRecoveryBusinessContext {
            entity_id: self.session.entity_id.clone(),
            plan_id: None,
            workspace_type: self.session.workspace_type.clone(),
            business_repository_physical_id: None,
            business_repository_logical_id: None,
            business_checkout_id: None,
            business_repository_path: self
                .session
                .repository_path
                .clone()
                .map(|path| path.to_string_lossy().to_string()),
            source_story_ids: Vec::new(),
            source_design_ids: Vec::new(),
            plan_options_ref: None,
            artifact_refs: Vec::new(),
            review_refs: Vec::new(),
            author_provider: self.session.author_provider.clone(),
            effective_reviewer_provider: self.session.reviewer_provider.clone(),
            original_node_id: self.active_node_id.clone(),
            original_role_run_seq: None,
        }
    }

    /// B1（终审 P1）：durable 侧的 author 族显式 fresh run 判据——
    /// `FreshRunning` 且 run_context tag 属 author 族（Author/
    /// AuthorChoiceFollowup/AuthorFeedback/ReviewerRevision）。以 durable
    /// record 为准（dispatch 推进的是 durable 状态；engine 内存镜像可能滞留
    /// Waiting）；读取失败按无 fresh 处理（fail-safe 等待方向）并 warn。
    pub(crate) fn lc_fresh_author_family_recovery(
        &self,
    ) -> Option<crate::product::models::LcResumeRecoveryRecord> {
        let store = self.lifecycle_store.as_ref()?;
        let record = match store.get_workspace_session(&self.session.session_id) {
            Ok(record) => record,
            Err(error) => {
                tracing::warn!(
                    %error,
                    session_id = %self.session.session_id,
                    "load workspace session for author family fresh recovery failed; treating as not-fresh"
                );
                return None;
            }
        };
        record.lc_resume_recovery.filter(|recovery| {
            recovery.state == crate::product::models::LcResumeRecoveryState::FreshRunning
                && matches!(
                    recovery.run_context,
                    crate::product::models::LcRecoveryRunContext::Author { .. }
                        | crate::product::models::LcRecoveryRunContext::AuthorChoiceFollowup { .. }
                        | crate::product::models::LcRecoveryRunContext::AuthorFeedback { .. }
                        | crate::product::models::LcRecoveryRunContext::ReviewerRevision { .. }
                )
        })
    }

    /// B1（终审 P1，§4.3/§5.3 author 族行）：fresh run 驱动结束后的 recovery
    /// 对账——fresh 节点 Completed → Resolved（新 native/节点回填 +
    /// operation Completed）；其余（Failed/悬置）→ 回 Waiting（operation
    /// Failed，结果保留：同 ID 只 Replay、下一次显式点击用新 ID）。
    /// 整记录 CAS single-flight；败给并发写方让位、store 写失败 warn（等待
    /// 方向 fail-safe）；engine 内存镜像同步。
    pub(crate) async fn reconcile_lc_fresh_author_family_run(
        &mut self,
        fresh_node_id: Option<&str>,
    ) {
        use crate::product::models::{
            LcFreshOperationState, LcRecoveryRunContext, LcResumeRecoveryState,
        };
        let Some(store) = self.lifecycle_store.clone() else {
            return;
        };
        let Ok(expected) = store.get_workspace_session(&self.session.session_id) else {
            // 读取失败不误判终态（等待方向 fail-safe）。
            return;
        };
        let Some(mut recovery) = expected.lc_resume_recovery.clone().filter(|recovery| {
            recovery.state == LcResumeRecoveryState::FreshRunning
                && matches!(
                    recovery.run_context,
                    LcRecoveryRunContext::Author { .. }
                        | LcRecoveryRunContext::AuthorChoiceFollowup { .. }
                        | LcRecoveryRunContext::AuthorFeedback { .. }
                        | LcRecoveryRunContext::ReviewerRevision { .. }
                )
        }) else {
            return;
        };
        let author_provider = self.session.author_provider.clone();
        let conversation = self
            .session
            .provider_conversations
            .iter()
            .find(|conversation| {
                conversation.role == ProviderConversationRole::Author
                    && conversation.provider == author_provider
            })
            .cloned();
        let fresh_node_id = fresh_node_id
            .map(str::to_string)
            .or_else(|| recovery.fresh_node_id.clone())
            .or_else(|| {
                conversation
                    .as_ref()
                    .and_then(|conversation| conversation.last_node_id.clone())
            });
        let native_session_id = conversation
            .map(|conversation| conversation.provider_session_id)
            .filter(|id| !id.trim().is_empty());
        let node_completed = fresh_node_id
            .as_deref()
            .and_then(|node_id| {
                self.timeline_nodes
                    .iter()
                    .find(|node| node.node_id == node_id)
            })
            .is_some_and(|node| node.status == TimelineNodeStatus::Completed);
        let now = chrono::Utc::now().to_rfc3339();
        if node_completed {
            recovery.state = LcResumeRecoveryState::Resolved;
            if recovery.fresh_native_session_id.is_none() {
                recovery.fresh_native_session_id = native_session_id;
            }
            if recovery.fresh_node_id.is_none() {
                recovery.fresh_node_id = fresh_node_id.clone();
            }
            if let Some(operation_id) = recovery.operation_id.as_deref() {
                for operation in recovery.fresh_operations.iter_mut() {
                    if operation.operation_id == operation_id {
                        operation.state = LcFreshOperationState::Completed;
                        if operation.fresh_node_id.is_none() {
                            operation.fresh_node_id = fresh_node_id.clone();
                        }
                        operation.updated_at = now.clone();
                    }
                }
            }
        } else {
            recovery.state = LcResumeRecoveryState::Waiting;
            let operation_id = recovery.operation_id.take();
            if let Some(operation_id) = operation_id.as_deref() {
                for operation in recovery.fresh_operations.iter_mut() {
                    if operation.operation_id == operation_id {
                        operation.state = LcFreshOperationState::Failed;
                        operation.reason_code = Some(
                            "lc_resume_fresh_run_failed: fresh author run did not complete"
                                .to_string(),
                        );
                        operation.updated_at = now.clone();
                    }
                }
            }
        }
        recovery.updated_at = now.clone();
        match store.compare_and_save_lc_resume_state(&expected, recovery) {
            Ok(saved) => {
                self.session.lc_resume_recovery = saved.lc_resume_recovery.clone();
            }
            Err(crate::product::json_store::ProductStoreError::Conflict { .. }) => {
                tracing::debug!(
                    session_id = %self.session.session_id,
                    "lc fresh author run reconcile CAS lost to a concurrent writer; conceding"
                );
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    session_id = %self.session.session_id,
                    "reconcile lc fresh author run outcome failed (fail-safe: durable state unchanged)"
                );
            }
        }
    }
}

/// Task 2.3：LC revision launch 决策结果——validated launch（cwd=root、
/// target=E `native_launch_anchor` 聚合根锚）+ resume 是否放行（supersede/
/// fresh 时 false）。
/// Task 9c:`superseded` 携带 StartNew 的旧会话 id 与稳定等待投影(仅显式
/// `StartGeneration` fresh)。
#[derive(Debug)]
pub(crate) struct RevisionLaunchDecision {
    launch: ValidatedSessionLaunchPolicy,
    resume_allowed: bool,
    superseded: Option<SupersededRevisionWait>,
}

/// Task 9c:StartNew(supersede)的等待上下文——被取代旧会话 id + gateway
/// 稳定等待投影(reason_code + 唯一 allowed action StartGeneration)。
#[derive(Debug)]
struct SupersededRevisionWait {
    superseded_session_id: String,
    waiting: crate::product::logical_codebase::provider_gateway::LcSupersededWaiting,
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
///
/// Task 2.3（REQ-ENV-10）：cwd 与 target 分离——cwd 取 gateway 冻结的 canonical
/// authority root（manifest `provider_context_root`，双工厂 canonical 一致性由
/// Task 2.8 断言），不再与 target 同源；target 保持独立 `aggregate_root` 锚
/// （成员 worktree，即 review 对象），readable roots 随 cwd root 化（成员位于
/// root 子树内的聚合布局）。validated 后 input 显式回填独立 cwd——spawn 前
/// effective cwd 复验以 envelope 冻结值为准。
///
/// 指引卡简化补缺：返回值携带分类桥同源等待投影（`trust_waiting`，仅
/// trust 分型命中且启动失败时 Some）——reviewer 首发臂消费它紧随 error
/// 事件发出 `EngineEvent::TrustMissingGuidance`；启动成功恒 None。
/// 批2 Task 20b(决策项⑦):规划面活动审查窗——句柄+同源复验的期望
/// 事实(worktree/provider/run_id 在开窗时冻结;assert 不再回读会话
/// 字段,repair/静默重试与测试换装会话字段零漂移)。
pub(crate) struct ActivePlanReviewWindow {
    pub(crate) handle: crate::product::reviewer_window::ReviewerWindowHandle,
    pub(crate) worktree: std::path::PathBuf,
    pub(crate) provider: String,
    pub(crate) run_id: String,
}

async fn start_review_session_via_gateway(
    gateway: &Arc<LogicalCodebaseProviderGateway>,
    reviewer: &ProviderName,
    input: StreamingProviderInput,
    project_id: String,
    cancel: CancellationToken,
) -> (
    Result<ProviderSession, ProviderAdapterError>,
    Option<crate::product::logical_codebase::provider_trust::ProviderTrustWaiting>,
) {
    let root = gateway.authority_root().to_path_buf();
    let provider_ref = match ProviderRef::from_provider_name(reviewer, "cap_managed_snapshot") {
        Ok(provider_ref) => provider_ref,
        Err(error) => return (Err(map_gateway_error_to_adapter(&error)), None),
    };
    let request = SessionLaunchRequest {
        project_id,
        provider: provider_ref,
        action: SessionPolicyAction::ReviewReadOnly,
        target: PolicyTarget::aggregate_root(input.working_dir.clone()),
        // Task 2.3：cwd=canonical root（独立于 target）；target 是
        // aggregate_root 锚（worktree 取成员路径，即 review 对象）。
        working_directory: root.clone(),
        readable_roots: vec![root],
        writable_roots: Vec::new(),
        config_artifact_ref: "sha256:managed-config-artifact".to_string(),
    };
    let validated = match gateway.validate(request) {
        Ok(validated) => validated,
        Err(error) => return (Err(map_gateway_error_to_adapter(&error)), None),
    };
    let mut input = input;
    input.working_directory = Some(validated.envelope().working_directory.clone());
    // A1-b1(Bug45 批2 环境回退):reviewer 受信环境注入停止——子进程经底座
    // (process_manager)继承宿主环境,caller input.env_vars 原样透传(仅作
    // 显式覆盖层);窗口纪律仍由 assert_plan_review_window 与 start 漏斗
    // 窗口守卫把关,与启动环境解耦。
    let validated_input = ValidatedStreamingProviderInput::new(input, validated);
    match gateway.start_streaming(validated_input, cancel).await {
        Ok(session) => (Ok(session), None),
        Err(error) => {
            let (mapped, trust_waiting) =
                map_gateway_error_to_adapter_with_trust(gateway, reviewer, &error);
            (Err(mapped), trust_waiting)
        }
    }
}


/// T6 fix-loop r2：review/drive.rs 两个 gateway 启动口（author revision、
/// reviewer 首发）共用的压平+分类——共享分类桥消费 typed 变体判据
/// （压平前），Generation 场景；`provider` 是本次启动的真实角色
/// provider（revision 传 author、review 传 reviewer），不得把参数名
/// 机械照搬成错误角色。
/// 指引卡简化补缺：结构化变体一次产出平铺 details 与同源等待投影
/// （`trust_waiting` 供调用方紧随 error 事件发出
/// `EngineEvent::TrustMissingGuidance`）；判据/文案与平铺面完全一致。
fn map_gateway_error_to_adapter_with_trust(
    gateway: &crate::product::logical_codebase::LogicalCodebaseProviderGateway,
    provider: &ProviderName,
    error: &ProviderGatewayError,
) -> (
    ProviderAdapterError,
    Option<crate::product::logical_codebase::provider_trust::ProviderTrustWaiting>,
) {
    let mut mapped = map_gateway_error_to_adapter(error);
    let classification = gateway.classify_launch_trust_failure(
        error,
        provider,
        &mapped.details,
        crate::product::logical_codebase::provider_trust::ProviderTrustScenario::Generation,
    );
    mapped.details = classification.details;
    (mapped, classification.trust_waiting)
}

/// gateway 错误到 adapter 错误的桥接:使调用点后续对 `Err` 的处理与直接
/// `provider.start` 完全一致(`ReviewProviderRunFailure::Start`)。
/// 诊断直通（claude×轻 握手谜团 第 2 轮）:Adapter 变体不再丢弃 stderr
/// 字段——stderr 尾部（有界）并入 details（随 Start 失败 →
/// EngineEvent::Error → WS error 消息上浮），stderr 字段同源保留；其余
/// gateway 校验错误维持 Display 文案不变（零变化）。
fn map_gateway_error_to_adapter(error: &ProviderGatewayError) -> ProviderAdapterError {
    match error {
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

// ---- 批2 Task 20b(决策项⑦):规划面 reviewer 审查窗口 ----
// 约定:窗口/快照/资源 store 根与 coding_workspace_engine::code_review 的
// `REVIEWER_WINDOW_STORE_DIR`/`REVIEWER_SNAPSHOT_STORE_DIR`/
// `PROVIDER_RESOURCE_STORE_DIR` 同名同根(跨模块字面量锚定,不跨模块引用
// 私有常量——与 coding_attempt_store/git_operation.rs 同款约定)。
const PLAN_REVIEWER_WINDOW_STORE_DIR: &str = "reviewer-windows";
const PLAN_REVIEWER_SNAPSHOT_STORE_DIR: &str = "reviewer-snapshots";
const PLAN_PROVIDER_RESOURCE_STORE_DIR: &str = "provider-resources";
/// 修复轮 1(F1):聚合根规划评审窗口目标待 owner 裁定的专用错误码
/// (真·无唯一成员面不静默杀、不以 IdentityDrift 误报)。
pub(crate) const PLAN_REVIEW_WINDOW_TARGET_UNRESOLVED: &str =
    "plan_review_window_target_unresolved";

impl WorkspaceEngine {
    fn plan_reviewer_window_store(
        &self,
    ) -> Result<crate::product::reviewer_window_store::ReviewerWindowStore, String> {
        let store = self.lifecycle_store.as_ref().ok_or_else(|| {
            "plan_review_window_unavailable: no persistent lifecycle store".to_string()
        })?;
        Ok(
            crate::product::reviewer_window_store::ReviewerWindowStore::new(
                store
                    .app_paths()
                    .root()
                    .join(PLAN_REVIEWER_WINDOW_STORE_DIR),
            ),
        )
    }

    fn plan_reviewer_snapshot_store(
        &self,
    ) -> Result<crate::product::reviewer_snapshot::ReviewerSnapshotStore, String> {
        let store = self.lifecycle_store.as_ref().ok_or_else(|| {
            "plan_review_window_unavailable: no persistent lifecycle store".to_string()
        })?;
        Ok(
            crate::product::reviewer_snapshot::ReviewerSnapshotStore::new(
                store
                    .app_paths()
                    .root()
                    .join(PLAN_REVIEWER_SNAPSHOT_STORE_DIR),
            ),
        )
    }

    /// 规划面 quiesce 目标:主 scope=本次 review run(空登记 Quiescent 即
    /// 静止证明;与 coding 面同构,不含 aria.worktree 协调 scope——规划
    /// 面无 Coder 并行窗登记)。
    fn plan_review_quiesce_targets(
        &self,
        window_id: &str,
    ) -> Result<Vec<crate::cross_cutting::provider_resources::ProviderRunResourceScope>, String>
    {
        use crate::cross_cutting::provider_resources::{
            ProviderRunResourceScope, ResourceScopeKey,
        };
        let store = self.lifecycle_store.as_ref().ok_or_else(|| {
            "plan_review_window_unavailable: no persistent lifecycle store".to_string()
        })?;
        let private_root = store
            .app_paths()
            .root()
            .join("plan-review-scopes")
            .join(window_id);
        std::fs::create_dir_all(&private_root)
            .map_err(|error| format!("plan_review_scope_create_failed: {error}"))?;
        let key = ResourceScopeKey {
            scope_id: format!("plan-review-{window_id}"),
            run_id: self
                .active_run_id
                .clone()
                .unwrap_or_else(|| self.session.session_id.clone()),
            boot_identity: format!("aria-engine-{}", std::process::id()),
        };
        let scope = ProviderRunResourceScope::open_for_recovery(
            &key,
            Some(window_id),
            &private_root,
            &store
                .app_paths()
                .root()
                .join(PLAN_PROVIDER_RESOURCE_STORE_DIR),
            None,
        )
        .map_err(|error| format!("plan_review_scope_open_failed: {error}"))?;
        Ok(vec![scope])
    }

    /// 修复轮 1(F1,Main 裁定):规划审查窗口目标解析——与
    /// `review_launch_target_and_cwd` 同源起步:目标带 `.git` 原样返回
    /// (成员 checkout/direct);聚合根视图(非 git 的 provider_context_
    /// root,Story focus=None/Design involved 空/WorkItemPlan 多 target
    /// 非唯一三形态可达)时经实体四事实 resolver 落到本次评审实际的
    /// 唯一/focus 成员 checkout;真·无唯一成员面不静默杀——返回专用
    /// 错误码(聚合根规划评审窗口目标待 owner 裁定),不得以
    /// IdentityDrift 形态误报。
    fn plan_review_window_worktree(&self) -> Result<(std::path::PathBuf, bool), String> {
        let target = self
            .session
            .repository_path
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        if target.join(".git").exists() {
            return Ok((target, false));
        }
        // 聚合根视图形态:优先业务仓唯一/focus 成员;聚合视图/无唯一成员
        // 回落主成员锚(Main 裁定 b:材料不触成员 diff 的聚合评审以主成员
        // checkout 作窗口对账域;非主成员面不在对账域=R8 类已知边界,由
        // EngineEvent 附注披露+T24 评估)。真零成员(空 manifest)→专用码。
        let Some(store) = self.lifecycle_store.as_ref() else {
            return Err(format!(
                "{PLAN_REVIEW_WINDOW_TARGET_UNRESOLVED}: aggregate root view without a persistent lifecycle store; owner ruling pending"
            ));
        };
        let business =
            crate::product::workspace_repository::resolve_workspace_repository_context_for_entity(
                &store.app_paths(),
                store,
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
                self.session.workspace_type.clone(),
            )
            .and_then(|context| context.business_repository);
        let (repository, primary_anchored) = match business {
            Ok(repository)
                if !crate::product::workspace_repository::is_aggregate_root_view_repository(
                    &repository,
                ) =>
            {
                (repository, false)
            }
            _ => {
                // 聚合视图或无唯一成员:主成员锚(manifest member_ids 首成员)。
                let primary = crate::product::workspace_repository::resolve_primary_member_repository(
                    &store.app_paths(),
                    &self.session.project_id,
                    &self.session.issue_id,
                )
                .map_err(|error| {
                    format!(
                        "{PLAN_REVIEW_WINDOW_TARGET_UNRESOLVED}: aggregate form without any member to anchor (primary member resolution failed: {error}); owner ruling pending"
                    )
                })?;
                (primary, true)
            }
        };
        if !repository.path.join(".git").exists() {
            return Err(format!(
                "{PLAN_REVIEW_WINDOW_TARGET_UNRESOLVED}: resolved member checkout {} is not a git worktree; owner ruling pending",
                repository.path.display()
            ));
        }
        Ok((repository.path, primary_anchored))
    }

    /// 入口开窗(T17 冻结顺序):必须早于 `ensure_review_invocation_scope`/
    /// `build_review_input`/`load_plan_review_context` 的一切材料采集。
    /// 失败 fail-closed——不以无窗继续 review。
    pub(crate) async fn begin_plan_review_window(
        &mut self,
        reviewer: &ProviderName,
        worktree: std::path::PathBuf,
    ) -> Result<(), String> {
        let window_id = format!(
            "plan-{}-w01",
            self.active_run_id
                .clone()
                .unwrap_or_else(|| self.session.session_id.clone())
        );
        let worktree = worktree.canonicalize().unwrap_or(worktree);
        let provider = serde_json::to_string(reviewer)
            .map(|raw| raw.trim_matches('"').to_string())
            .unwrap_or_else(|_| format!("{reviewer:?}"));
        let targets = self.plan_review_quiesce_targets(&window_id)?;
        let target_refs: Vec<&crate::cross_cutting::provider_resources::ProviderRunResourceScope> =
            targets.iter().collect();
        let request = crate::product::reviewer_window::ReviewerWindowBeginRequest {
            window_id: &window_id,
            attempt_id: &self.session.session_id,
            run_id: self
                .active_run_id
                .as_deref()
                .unwrap_or(&self.session.session_id),
            provider: &provider,
            worktree: &worktree,
            quiesce_targets: &target_refs,
            platform_write_paths: Vec::new(),
            budget: crate::product::reviewer_snapshot::SnapshotBudget {
                max_entries: 200_000,
                max_total_bytes: 2 * 1024 * 1024 * 1024,
            },
            drain_timeout: std::time::Duration::from_secs(60),
        };
        let window = crate::product::reviewer_window::begin_review_window(
            &self.plan_reviewer_window_store()?,
            &self.plan_reviewer_snapshot_store()?,
            &request,
        )
        .await
        .map_err(|error| format!("plan_review_window_begin_failed: {error}"))?;
        let run_id = self
            .active_run_id
            .clone()
            .unwrap_or_else(|| self.session.session_id.clone());
        self.active_review_window = Some(ActivePlanReviewWindow {
            handle: window,
            worktree,
            provider,
            run_id,
        });
        Ok(())
    }

    /// 材料采集/provider 启动前同源复验(窗口 Running+指纹一致)。
    fn assert_plan_review_window(&self) -> Result<(), String> {
        let Some(active) = self.active_review_window.as_ref() else {
            return Ok(());
        };
        let expectation = crate::product::reviewer_window::WindowExpectation {
            window_id: &active.handle.window_id,
            attempt_id: &self.session.session_id,
            run_id: &active.run_id,
            provider: &active.provider,
            worktree: &active.worktree,
        };
        let proof = crate::product::reviewer_window::assert_review_window_current(
            &self.plan_reviewer_window_store()?,
            &self.plan_reviewer_snapshot_store()?,
            &expectation,
            &[crate::product::reviewer_window_store::ReviewerWindowState::Running],
        )
        .map_err(|error| format!("plan_review_window_not_current: {error}"))?;
        if proof.manifest_sha256 != active.handle.manifest_sha256 {
            return Err("plan_review_window_material_fingerprint_drift".to_string());
        }
        Ok(())
    }

    /// reviewer 输入的 ReviewerVerify 窗口合同铸造(prompt 八点共用的
    /// 唯一窗口来源;无活动窗=None,由四家启动检查/网关复验拒绝)。
    pub(crate) fn reviewer_verify_policy(&self) -> Option<ProviderToolPolicy> {
        self.active_review_window
            .as_ref()
            .map(|active| ProviderToolPolicy::reviewer_verify(active.handle.window_id.clone()))
    }

    /// 收口:干净恢复+释放(技术回收;规划面 verdict 无报告采信投影,
    /// 消费面采信由后续交付门承担——与 legacy group 面同口径披露)。
    async fn close_plan_review_window(&mut self, reason: &str) {
        let Some(active) = self.active_review_window.take() else {
            return;
        };
        let window = active.handle;
        let (Ok(window_store), Ok(snapshot_store)) = (
            self.plan_reviewer_window_store(),
            self.plan_reviewer_snapshot_store(),
        ) else {
            tracing::warn!("plan_review_window_close_store_unavailable");
            return;
        };
        let record = match crate::product::reviewer_window::begin_quiesce(
            &window_store,
            &window.window_id,
            window.generation,
            reason,
        ) {
            Ok(record) => record,
            Err(error) => {
                tracing::warn!(%error, window = %window.window_id, "plan_review_window_close_kept_lock");
                return;
            }
        };
        match crate::product::reviewer_window::restore_window(
            &window_store,
            &snapshot_store,
            &window.window_id,
            record.generation,
        )
        .await
        {
            Ok(summary) => {
                if summary.manual_gate {
                    tracing::warn!(
                        window = %window.window_id,
                        "plan_review_window_violation_manual_gate"
                    );
                    return;
                }
                if let Err(error) = crate::product::reviewer_window::abandon_window(
                    &window_store,
                    &window.window_id,
                    summary.record.generation,
                    reason,
                ) {
                    tracing::warn!(%error, window = %window.window_id, "plan_review_window_abandon_failed");
                }
            }
            Err(error) => {
                tracing::warn!(%error, window = %window.window_id, "plan_review_window_close_kept_lock");
            }
        }
    }
}

impl WorkspaceEngine {
    /// 修复轮 1(Main 裁定 b):聚合形态主成员锚的窗口附注——R8 类已知
    /// 边界如实披露(该窗内非主成员写入不在对账域;T24 重放/崩溃演练
    /// 评估项与 owner 报告项)。
    async fn emit_primary_member_anchor_note(
        event_tx: &mpsc::Sender<EngineEvent>,
        session_id: &str,
        anchor: &std::path::Path,
    ) {
        let _ = event_tx
            .send(EngineEvent::ExecutionEvent {
                event: crate::cross_cutting::streaming_provider::ProviderExecutionEvent {
                    event_id: format!("plan_review_window_primary_anchor_{session_id}"),
                    kind: crate::cross_cutting::streaming_provider::ProviderExecutionEventKind::Provider,
                    status: crate::cross_cutting::streaming_provider::ProviderExecutionEventStatus::Completed,
                    title: "审查窗口主成员锚定(primary-member anchored;非主成员面未对账——R8 类已知边界,详 T20b 修复轮1 报告)".to_string(),
                    detail: Some(format!("anchor: {}", anchor.display())),
                    command: None,
                    cwd: None,
                    output: None,
                    exit_code: None,
                },
                node_id: None,
                agent: None,
            })
            .await;
    }
}
#[cfg(test)]
#[path = "drive_tests.rs"]
mod tests;
