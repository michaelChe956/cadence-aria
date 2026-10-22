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
        // Task 2(REQ-ENV-14,设计 A 条目 3):多仓(lc)形态不开评审窗——
        // 直入内层承载(内层自带 reviewer 缺失 fail-closed);同源复验点
        // 以材料断言替代窗口断言,无基线快照/对账/恢复,不产
        // plan_review_window_* 错误码或窗口注意力提示。单仓路径不变。
        if self.is_logical_codebase_issue() {
            return self
                .drive_review_session_with_window(provider, command_rx)
                .await;
        }
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
        let review_target = match self.plan_review_window_worktree() {
            Ok(resolved) => resolved,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                return;
            }
        };
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
        let first_session = provider
            .start_owned(
                input.clone(),
                self.plan_review_launch_ownership(),
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
                let repair_session = provider
                    .start_owned(
                        repair_input,
                        self.plan_review_launch_ownership(),
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
                let retry_session = provider
                    .start_owned(
                        retry_input,
                        self.plan_review_launch_ownership(),
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
        // Task 2(REQ-ENV-14,设计 A 条目 3):多仓(lc)形态不开评审窗——
        // gateway 版公共入口同构直入内层承载,材料断言替代窗口断言。
        if self.is_logical_codebase_issue() {
            return self
                .drive_review_session_via_gateway_with_window(command_rx)
                .await;
        }
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
        let review_target = match self.plan_review_window_worktree() {
            Ok(resolved) => resolved,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.finish_failed_run().await;
                return;
            }
        };
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
        let (first_session, first_trust_waiting) = start_review_session_via_gateway_owned(
            &gateway,
            &reviewer,
            input.clone(),
            project_id.clone(),
            self.plan_review_launch_ownership(),
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
                let (repair_session, _repair_trust_waiting) =
                    start_review_session_via_gateway_owned(
                        &gateway,
                        &reviewer,
                        repair_input,
                        project_id,
                        self.plan_review_launch_ownership(),
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
                let (retry_session, _retry_trust_waiting) = start_review_session_via_gateway_owned(
                    &gateway,
                    &reviewer,
                    retry_input,
                    project_id,
                    self.plan_review_launch_ownership(),
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
    /// B1-1:开窗时打开的 plan-review 主 scope(`plan-review-{window}` ×
    /// run × boot)——规划 reviewer 每次 launch 从此真实 scope 铸造受信
    /// 归属(此前开窗后即丢弃);窗口生命周期内持有,收口由既有窗口流
    /// 与 scope 级 quiesce 承担。
    pub(crate) launch_scope:
        std::sync::Arc<crate::cross_cutting::provider_resources::ProviderRunResourceScope>,
}

async fn start_review_session_via_gateway_owned(
    gateway: &Arc<LogicalCodebaseProviderGateway>,
    reviewer: &ProviderName,
    input: StreamingProviderInput,
    project_id: String,
    // B1-1:规划 reviewer 的受信资源归属(活动窗主 scope 铸造;direct/
    // gateway 两形态同一来源)。绑定进 validated input 由 adapter
    // `start_validated` 的 spawn 点消费。
    launch_ownership: Option<crate::cross_cutting::provider_resources::TrustedLaunchOwnership>,
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
    // B1-1:受信 launch 归属绑定进 validated input(crate 私有 setter),
    // adapter `start_validated` 的 spawn 点消费同一登记生命周期。
    let validated_input = match launch_ownership {
        Some(ownership) => validated_input.with_launch_ownership(ownership),
        None => validated_input,
    };
    match gateway.start_streaming(validated_input, cancel).await {
        Ok(session) => (Ok(session), None),
        Err(error) => {
            let (mapped, trust_waiting) =
                map_gateway_error_to_adapter_with_trust(gateway, reviewer, &error);
            (Err(mapped), trust_waiting)
        }
    }
}

/// 既有 5 参签名(测试/trust 分类面消费):无归属委托 owned 变体。
/// 生产规划 reviewer launch 一律经 `_owned` 传活动窗归属。
#[cfg(test)]
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
    start_review_session_via_gateway_owned(gateway, reviewer, input, project_id, None, cancel).await
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
// 约定(F13,波3):窗口 store 根目录名统一引用
// `reviewer_window_store::REVIEWER_WINDOW_STORE_DIR` 单一常量(与
// `coding_workspace_engine::code_review`、`coding_attempt_store/
// git_operation.rs` 同源;快照/资源 store 根仍为字面量锚定约定,
// 与 git_operation.rs 同款——本项只上收 reviewer-windows 一处)。
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
                    .join(crate::product::reviewer_window_store::REVIEWER_WINDOW_STORE_DIR),
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
    /// 面无 Coder 并行窗登记)。B1-1:同一 scope 亦是规划 reviewer launch
    /// 的受信归属来源(`begin_plan_review_window` 持有,原开窗后即丢弃)。
    fn plan_review_main_scope(
        &self,
        window_id: &str,
    ) -> Result<crate::cross_cutting::provider_resources::ProviderRunResourceScope, String> {
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
        ProviderRunResourceScope::open_for_recovery(
            &key,
            Some(window_id),
            &private_root,
            &store
                .app_paths()
                .root()
                .join(PLAN_PROVIDER_RESOURCE_STORE_DIR),
            None,
        )
        .map_err(|error| format!("plan_review_scope_open_failed: {error}"))
    }

    /// B1-1(波 2):规划 reviewer 本次 launch 的受信资源归属铸造——从
    /// 活动窗持有的 plan-review 主 scope(开窗实际归属:同 window/run/
    /// boot)生成唯一资源 ID 载体;首发/repair/静默重试与 gateway/direct
    /// 两形态共用。无活动窗(测试 seam)= `None`(真实 adapter 由窗口
    /// 守卫拒绝,不以无归属静默降级为生产语义)。
    fn plan_review_launch_ownership(
        &self,
    ) -> Option<crate::cross_cutting::provider_resources::TrustedLaunchOwnership> {
        self.active_review_window.as_ref().map(|active| {
            crate::cross_cutting::provider_resources::TrustedLaunchOwnership::mint(
                std::sync::Arc::clone(&active.launch_scope),
                &format!("plan review provider process: {}", active.provider),
            )
        })
    }

    /// 规划审查窗口目标解析——与 `review_launch_target_and_cwd` 同源起步:
    /// 目标带 `.git` 原样返回(单仓唯一出口,也是成员 checkout/direct 形态);
    /// 聚合根视图(非 git 的 provider_context_root,Story focus=None/Design
    /// involved 空/WorkItemPlan 多 target 非唯一三形态可达)时经实体四事实
    /// resolver 落到本次评审实际的唯一/focus 成员 checkout;**无唯一成员不再
    /// 回落任何单一仓库**(REQ-PLN-11 设计 A 条目 1:多仓已无「主仓/主成员」
    /// 概念)——返回专用错误码(聚合根规划评审窗口目标待 owner 裁定),不得
    /// 以 IdentityDrift 形态误报。
    fn plan_review_window_worktree(&self) -> Result<std::path::PathBuf, String> {
        let target = self
            .session
            .repository_path
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        if target.join(".git").exists() {
            return Ok(target);
        }
        // 聚合根视图形态:必须解析出唯一/focus 业务仓;聚合视图(label 级)
        // 与无唯一成员同属"无窗口对账域",一律专用码停等。
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
        let repository = match business {
            Ok(repository)
                if !crate::product::workspace_repository::is_aggregate_root_view_repository(
                    &repository,
                ) =>
            {
                repository
            }
            Ok(_) => {
                return Err(format!(
                    "{PLAN_REVIEW_WINDOW_TARGET_UNRESOLVED}: aggregate root view without a unique member repository; owner ruling pending"
                ));
            }
            Err(error) => {
                return Err(format!(
                    "{PLAN_REVIEW_WINDOW_TARGET_UNRESOLVED}: aggregate root view without a unique member repository (business resolution failed: {error}); owner ruling pending"
                ));
            }
        };
        if !repository.path.join(".git").exists() {
            return Err(format!(
                "{PLAN_REVIEW_WINDOW_TARGET_UNRESOLVED}: resolved member checkout {} is not a git worktree; owner ruling pending",
                repository.path.display()
            ));
        }
        Ok(repository.path)
    }

    /// 入口开窗(T17 冻结顺序):必须早于 `ensure_review_invocation_scope`/
    /// `build_review_input`/`load_plan_review_context` 的一切材料采集。
    /// 失败 fail-closed——不以无窗继续 review。
    pub(crate) async fn begin_plan_review_window(
        &mut self,
        reviewer: &ProviderName,
        worktree: std::path::PathBuf,
    ) -> Result<(), String> {
        // 窗口 ID 全局唯一:走 `role_run_window_id`(attempt 位=workspace
        // session id,role run 位=plan run id)。此前此处自持
        // `plan-<run>-w01` 格式化副本——窗口 store 目录跨会话共享,
        // 不同会话的同号 plan run 会算出同一 ID 并撞车(旧记录命中 →
        // `reviewer_window_conflict`,plan review fail-closed 走不下去)。
        let window_id = crate::product::reviewer_window_store::role_run_window_id(
            &self.session.session_id,
            self.active_run_id
                .as_deref()
                .unwrap_or(&self.session.session_id),
        );
        let worktree = worktree.canonicalize().unwrap_or(worktree);
        let provider = serde_json::to_string(reviewer)
            .map(|raw| raw.trim_matches('"').to_string())
            .unwrap_or_else(|_| format!("{reviewer:?}"));
        // B1-1:开窗打开的主 scope 以 Arc 持有——quiesce 目标借引用消费,
        // 窗口生命周期内保留给规划 reviewer launch 铸造受信归属(此前
        // 开窗消费后即丢弃)。
        let main_scope = std::sync::Arc::new(self.plan_review_main_scope(&window_id)?);
        let target_refs = [&*main_scope];
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
            launch_scope: main_scope,
        });
        Ok(())
    }

    /// 材料采集/provider 启动前同源复验(窗口 Running+指纹一致)。
    fn assert_plan_review_window(&self) -> Result<(), String> {
        // Task 2(REQ-ENV-14/PLN-11):同源复验点形态分派——多仓(lc)形态
        // 无评审窗,复验「材料集合可解析且非空」;单仓保持窗口 Running+
        // 材料指纹一致复验,函数体逐字不变。
        if self.is_logical_codebase_issue() {
            return self.assert_plan_review_material();
        }
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

    /// Task 2(REQ-ENV-14/PLN-11):多仓(lc)形态材料断言——无评审窗路径
    /// 下,provider 启动前以「参与仓库集合可解析且非空」替代窗口 Running
    /// 复验。lifecycle store 缺失、解析失败或集合为空一律 fail-closed
    /// (`plan_review_material_unresolved: <原因>`),绝不以空材料静默
    /// 继续,也不产任何 plan_review_window_* 错误码。
    fn assert_plan_review_material(&self) -> Result<(), String> {
        let Some(store) = self.lifecycle_store.as_ref() else {
            return Err("plan_review_material_unresolved: lifecycle_store_unavailable".to_string());
        };
        let resolution =
            crate::product::logical_codebase::PlanningContextSetResolver::new(store.app_paths())
                .resolve(
                    self.session.project_id.as_str(),
                    self.session.issue_id.as_str(),
                )
                .map_err(|cause| format!("plan_review_material_unresolved: {cause}"))?;
        if resolution.set.is_empty() {
            return Err(
                "plan_review_material_unresolved: resolved member set is empty".to_string(),
            );
        }
        Ok(())
    }

    /// reviewer 输入的 ReviewerVerify 窗口合同铸造(prompt 八点共用的
    /// 唯一窗口来源)。REQ-ENV-13(设计 A 条目 17)三分支:
    /// ① 有活动窗口 → 窗口 id(单仓路径,canonical 摘要逐字不变);
    /// ② 无窗口且多仓(lc)形态 → 每会话 UUID 身份(同一会话跨轮次/
    ///    repair 恒定,digest 域即该 UUID);
    /// ③ 无窗口且单仓 → None(由四家启动检查/网关复验拒绝,不得借
    ///    会话身份放行)。
    pub(crate) fn reviewer_verify_policy(&self) -> Option<ProviderToolPolicy> {
        if let Some(active) = self.active_review_window.as_ref() {
            return Some(ProviderToolPolicy::reviewer_verify(
                active.handle.window_id.clone(),
            ));
        }
        if self.is_logical_codebase_issue() {
            return Some(ProviderToolPolicy::reviewer_verify(
                self.session.review_identity_id.clone(),
            ));
        }
        None
    }

    /// 收口:干净恢复+释放(技术回收;规划面 verdict 无报告采信投影,
    /// 消费面采信由后续交付门承担——与 legacy group 面同口径披露)。
    ///
    /// F11(波3):一切「收口未完成」出口(store 失效/收口保锁/恢复
    /// 人工门/弃置失败)在 tracing::warn 之外同步经既有 EngineEvent
    /// 事件流发用户可见注意事件(带 window_id/原因/人工入口)。锁与
    /// generation 语义不变——投影不解锁、不迁移状态、不丢窗口引用;
    /// workspace plan 面以 session 为锚,不照搬 coding 专用
    /// `project_window_manual_attention`(那依赖 coding attempt 状态机)。
    async fn close_plan_review_window(&mut self, reason: &str) {
        let Some(active) = self.active_review_window.take() else {
            return;
        };
        let window = active.handle;
        let session_id = self.session.session_id.clone();
        let (Ok(window_store), Ok(snapshot_store)) = (
            self.plan_reviewer_window_store(),
            self.plan_reviewer_snapshot_store(),
        ) else {
            tracing::warn!("plan_review_window_close_store_unavailable");
            Self::emit_plan_review_window_attention(
                &self.event_tx,
                &session_id,
                &window.window_id,
                "close_store_unavailable",
                &format!("收口原因「{reason}」;持久 lifecycle store 不可用,窗口未收口"),
                "恢复 lifecycle store 后重试收口;窗口记录 reviewer-windows/windows/<本窗>.json(<aria> 下),锁与 generation 保持",
            )
            .await;
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
                Self::emit_plan_review_window_attention(
                    &self.event_tx,
                    &session_id,
                    &window.window_id,
                    "close_kept_lock",
                    &format!("begin_quiesce 失败: {error};窗口保持锁未收口"),
                    "核对窗口记录后人工重试收口或弃置;记录 reviewer-windows/windows/<本窗>.json(<aria> 下)",
                )
                .await;
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
                    Self::emit_plan_review_window_attention(
                        &self.event_tx,
                        &session_id,
                        &window.window_id,
                        "manual_gate",
                        "恢复完整但违规事实已落 journal:整轮作废,需人工裁决后才能关闭窗口",
                        "人工裁决 Invalidate(作废关闭)或 RequireNewReview(作废并开新一轮评审);违规 journal reviewer-windows/violations/<本窗>.json(<aria> 下);裁决前锁保持",
                    )
                    .await;
                    return;
                }
                if let Err(error) = crate::product::reviewer_window::abandon_window(
                    &window_store,
                    &window.window_id,
                    summary.record.generation,
                    reason,
                ) {
                    tracing::warn!(%error, window = %window.window_id, "plan_review_window_abandon_failed");
                    Self::emit_plan_review_window_attention(
                        &self.event_tx,
                        &session_id,
                        &window.window_id,
                        "abandon_failed",
                        &format!("恢复已完成但弃置收口失败: {error};窗口保持非终态"),
                        "重试收口或人工处置;窗口记录 reviewer-windows/windows/<本窗>.json(<aria> 下)",
                    )
                    .await;
                }
            }
            Err(error) => {
                tracing::warn!(%error, window = %window.window_id, "plan_review_window_close_kept_lock");
                Self::emit_plan_review_window_attention(
                    &self.event_tx,
                    &session_id,
                    &window.window_id,
                    "close_kept_lock",
                    &format!("恢复失败: {error};窗口保持锁未收口"),
                    "处置后重入收口续接;窗口记录 reviewer-windows/windows/<本窗>.json(<aria> 下)",
                )
                .await;
            }
        }
    }

    /// F11(波3):规划评审窗口的用户可见注意事件——复用既有
    /// EngineEvent::ExecutionEvent 事件流,status=Failed 表「需人工处理」。
    /// 发送 best-effort(`let _`,与既有事件发射一致),不影响窗口状态机。
    async fn emit_plan_review_window_attention(
        event_tx: &mpsc::Sender<EngineEvent>,
        session_id: &str,
        window_id: &str,
        attention: &str,
        reason: &str,
        manual_entry: &str,
    ) {
        let _ = event_tx
            .send(EngineEvent::ExecutionEvent {
                event: crate::cross_cutting::streaming_provider::ProviderExecutionEvent {
                    event_id: format!(
                        "plan_review_window_attention_{attention}_{session_id}_{window_id}"
                    ),
                    kind: crate::cross_cutting::streaming_provider::ProviderExecutionEventKind::Provider,
                    status: crate::cross_cutting::streaming_provider::ProviderExecutionEventStatus::Failed,
                    title: format!("规划评审窗口需人工处理({attention})"),
                    detail: Some(format!(
                        "window_id: {window_id}\n原因: {reason}\n人工入口: {manual_entry}"
                    )),
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

/// B1-1(波 2):规划 reviewer 开窗保留主 scope 并铸造 launch 归属。
#[cfg(test)]
mod b1_plan_launch_ownership_tests {
    use super::*;
    use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};

    fn b1_git_init(dir: &std::path::Path) {
        use std::process::Command;
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "b1@example.test"],
            vec!["config", "user.name", "B1 Plan"],
            vec!["commit", "-m", "init", "--quiet", "--allow-empty"],
        ] {
            let status = Command::new("git")
                .current_dir(dir)
                .args(&args)
                .status()
                .expect("git fixture command");
            assert!(status.success(), "git fixture {args:?} must succeed");
        }
    }

    #[tokio::test]
    async fn b1_plan_review_window_keeps_scope_and_mints_launch_ownership() {
        let root = tempfile::tempdir().expect("root");
        let paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
        let project = crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "B1-PLAN".to_string(),
                description: None,
            })
            .expect("project");
        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project.id.clone(),
                repo_id: None,
                logical_codebase_id: None,
                base_branch: None,
                title: "b1 plan".to_string(),
                description: None,
                change_id: None,
            })
            .expect("issue");
        let lifecycle = LifecycleStore::new(paths.clone());
        let record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: project.id.clone(),
                issue_id: issue.id.clone(),
                entity_id: issue.id.clone(),
                workspace_type: crate::product::models::WorkspaceType::Story,
                author_provider: crate::product::models::ProviderName::ClaudeCode,
                reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("workspace session");
        let checkpoint =
            std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                paths.issue_lifecycle_root(&project.id, &issue.id),
            ));
        let (tx, _rx) = mpsc::channel(64);
        let mut engine = WorkspaceEngine::new_persistent(
            checkpoint,
            LifecycleStore::new(paths.clone()),
            tx,
            crate::product::workspace_engine::types::WorkspaceSession::from_record(record),
        );
        // 直连形态:repository_path 即带 .git 的评审目标(跳过聚合根解析)。
        let worktree = root.path().join("plan-worktree");
        std::fs::create_dir_all(&worktree).expect("worktree");
        b1_git_init(&worktree);
        engine.session.repository_path = Some(worktree.clone());

        engine
            .begin_plan_review_window(&crate::product::models::ProviderName::ClaudeCode, worktree)
            .await
            .expect("begin plan review window");

        let active = engine
            .active_review_window
            .as_ref()
            .expect("活动窗持有主 scope");
        let expected_scope_id = format!("plan-review-{}", active.handle.window_id);
        assert_eq!(active.launch_scope.key().scope_id, expected_scope_id);
        // launch 归属从同一 scope 铸造(同 window/run/boot;资源 ID 唯一)。
        let ownership = engine
            .plan_review_launch_ownership()
            .expect("活动窗铸造 launch 归属");
        assert_eq!(ownership.scope_key(), active.launch_scope.key());
        let again = engine.plan_review_launch_ownership().expect("再次铸造");
        assert_ne!(ownership.resource_id(), again.resource_id());

        engine.close_plan_review_window("b1_plan_test_exit").await;
    }
}

/// 波3 F11:close_plan_review_window 的用户可见人工注意投影。
/// 红线:store 失效/恢复人工门时仅 tracing::warn 不可接受——必须经
/// 既有 EngineEvent 事件流发用户可见事件(带 window_id/原因/人工入口),
/// 且不改变锁与 generation 语义(不解锁、不丢窗口引用、状态机原样)。
#[cfg(test)]
mod f11_close_attention_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::reviewer_window_store::{
        REVIEWER_WINDOW_STORE_DIR, ReviewerWindowState, ReviewerWindowStore, ViolationJournalEntry,
        tree_key_of,
    };

    fn f11_git_init(dir: &std::path::Path) {
        use std::process::Command;
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "f11@example.test"],
            vec!["config", "user.name", "F11 Plan"],
            vec!["commit", "-m", "init", "--quiet", "--allow-empty"],
        ] {
            let status = Command::new("git")
                .current_dir(dir)
                .args(&args)
                .status()
                .expect("git fixture command");
            assert!(status.success(), "git fixture {args:?} must succeed");
        }
    }

    fn f11_engine(
        root: &std::path::Path,
    ) -> (
        WorkspaceEngine,
        mpsc::Receiver<EngineEvent>,
        ProductAppPaths,
        std::path::PathBuf,
    ) {
        let paths = ProductAppPaths::new(root.join(".aria"));
        let project = crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "F11-PLAN".to_string(),
                description: None,
            })
            .expect("project");
        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project.id.clone(),
                repo_id: None,
                logical_codebase_id: None,
                base_branch: None,
                title: "f11 plan".to_string(),
                description: None,
                change_id: None,
            })
            .expect("issue");
        let lifecycle = LifecycleStore::new(paths.clone());
        let record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: project.id.clone(),
                issue_id: issue.id.clone(),
                entity_id: issue.id.clone(),
                workspace_type: crate::product::models::WorkspaceType::Story,
                author_provider: crate::product::models::ProviderName::ClaudeCode,
                reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("workspace session");
        let checkpoint =
            std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                paths.issue_lifecycle_root(&project.id, &issue.id),
            ));
        let (tx, rx) = mpsc::channel(64);
        let mut engine = WorkspaceEngine::new_persistent(
            checkpoint,
            LifecycleStore::new(paths.clone()),
            tx,
            crate::product::workspace_engine::types::WorkspaceSession::from_record(record),
        );
        let worktree = root.join("f11-worktree");
        std::fs::create_dir_all(&worktree).expect("worktree");
        f11_git_init(&worktree);
        engine.session.repository_path = Some(worktree.clone());
        (engine, rx, paths, worktree)
    }

    async fn f11_next_attention(
        rx: &mut mpsc::Receiver<EngineEvent>,
        prefix: &str,
    ) -> Option<crate::cross_cutting::streaming_provider::ProviderExecutionEvent> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while let Ok(Some(event)) = tokio::time::timeout_at(deadline, rx.recv()).await {
            if let EngineEvent::ExecutionEvent { event, .. } = event
                && event.event_id.starts_with(prefix)
            {
                return Some(event);
            }
        }
        None
    }

    /// store 失效:仅 warn 不可接受——必须发用户可见注意事件,且窗口
    /// 记录(锁与 generation)原样保持。
    #[tokio::test]
    async fn f11_store_unavailable_close_emits_attention_and_keeps_window() {
        let root = tempfile::tempdir().expect("root");
        let (mut engine, mut rx, paths, worktree) = f11_engine(root.path());
        engine
            .begin_plan_review_window(&crate::product::models::ProviderName::ClaudeCode, worktree)
            .await
            .expect("begin plan review window");
        let active = engine.active_review_window.as_ref().expect("active window");
        let window_id = active.handle.window_id.clone();
        let generation = active.handle.generation;
        let window_store = ReviewerWindowStore::new(paths.root().join(REVIEWER_WINDOW_STORE_DIR));
        let before = window_store.load_window(&window_id).expect("record before");

        engine.lifecycle_store = None;
        engine.close_plan_review_window("f11_store_gone").await;

        let attention = f11_next_attention(
            &mut rx,
            "plan_review_window_attention_close_store_unavailable",
        )
        .await
        .expect("F11 红:store 失效收口必须发用户可见注意事件");
        let detail = attention.detail.as_deref().unwrap_or("");
        assert!(detail.contains(&window_id), "注意事件必须带 window_id");
        assert!(!detail.is_empty(), "注意事件必须带原因/人工入口");

        let after = window_store.load_window(&window_id).expect("record after");
        assert_eq!(after.state, before.state, "锁与状态保持");
        assert_eq!(after.generation, before.generation, "generation 保持");
        assert_eq!(after.generation, generation);
    }

    /// 恢复人工门:必须发用户可见注意事件(含人工裁决入口),锁保持
    /// (ManualGate 非终态、租约不释放)。
    #[tokio::test]
    async fn f11_manual_gate_close_emits_attention_and_keeps_lock() {
        let root = tempfile::tempdir().expect("root");
        let (mut engine, mut rx, paths, worktree) = f11_engine(root.path());
        engine
            .begin_plan_review_window(&crate::product::models::ProviderName::ClaudeCode, worktree)
            .await
            .expect("begin plan review window");
        let active = engine.active_review_window.as_ref().expect("active window");
        let window_id = active.handle.window_id.clone();
        let window_store = ReviewerWindowStore::new(paths.root().join(REVIEWER_WINDOW_STORE_DIR));
        let record = window_store.load_window(&window_id).expect("record");
        window_store
            .append_violation(
                &window_id,
                ViolationJournalEntry {
                    seq: 0,
                    entry_id: "f11-violation-entry-1".to_string(),
                    at: chrono::Utc::now().to_rfc3339(),
                    window_id: window_id.clone(),
                    attempt_id: record.attempt_id.clone(),
                    run_id: record.run_id.clone(),
                    report_id: None,
                    kind: "worktree_mutation".to_string(),
                    path: "reviewer-edit.txt".to_string(),
                    before_digest: None,
                    after_digest: None,
                    detail: "F11 夹具:预置违规 journal 触发人工门".to_string(),
                    restore_cursor: "started+0".to_string(),
                },
            )
            .expect("append violation");

        engine.close_plan_review_window("f11_violation").await;

        let attention = f11_next_attention(&mut rx, "plan_review_window_attention_manual_gate")
            .await
            .expect("F11 红:人工门收口必须发用户可见注意事件");
        let detail = attention.detail.as_deref().unwrap_or("");
        assert!(detail.contains(&window_id), "注意事件必须带 window_id");
        assert!(
            detail.contains("Invalidate") && detail.contains("RequireNewReview"),
            "注意事件必须披露人工裁决入口: {detail}"
        );

        let after = window_store.load_window(&window_id).expect("record after");
        assert_eq!(
            after.state,
            ReviewerWindowState::ManualGate,
            "锁保持:人工门非终态"
        );
        let tree_key = tree_key_of(&after.identity.canonical_worktree);
        let lease = window_store
            .load_lease(&tree_key)
            .expect("lease read")
            .expect("锁保持:租约未释放");
        assert_eq!(lease.window_id, window_id);
    }
}

/// 规划面窗口 ID 全局唯一(跨会话撞号回归)。
///
/// 窗口 store 目录跨会话共享(全局 `.aria` 目录):规划面此前自持
/// `plan-<run>-w01` 格式化副本,不同会话的同号 plan run 会算出同一
/// 窗口 ID → 后一个会话开窗命中旧记录被拒
/// (`reviewer_window_conflict[reviewer_window]`),plan review
/// fail-closed 走不下去。窗口 ID 必须带会话前缀
/// (`role_run_window_id(session_id, run_id)`)。
#[cfg(test)]
mod plan_window_id_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};

    fn git_init(dir: &std::path::Path) {
        use std::process::Command;
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "plan-window@example.test"],
            vec!["config", "user.name", "Plan Window"],
            vec!["commit", "-m", "init", "--quiet", "--allow-empty"],
        ] {
            let status = Command::new("git")
                .current_dir(dir)
                .args(&args)
                .status()
                .expect("git fixture command");
            assert!(status.success(), "git fixture {args:?} must succeed");
        }
    }

    /// 同项目同 issue 下的两个会话 + 各自的评审工作树(直连形态:
    /// repository_path 即评审目标)。
    fn two_session_engines(
        root: &std::path::Path,
    ) -> (WorkspaceEngine, WorkspaceEngine, ProductAppPaths) {
        let paths = ProductAppPaths::new(root.join(".aria"));
        let project = crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "PLAN-WINDOW-ID".to_string(),
                description: None,
            })
            .expect("project");
        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project.id.clone(),
                repo_id: None,
                logical_codebase_id: None,
                base_branch: None,
                title: "plan window id".to_string(),
                description: None,
                change_id: None,
            })
            .expect("issue");
        let lifecycle = LifecycleStore::new(paths.clone());
        let mut engines = Vec::new();
        for index in 0..2 {
            let record = lifecycle
                .create_workspace_session(CreateWorkspaceSessionInput {
                    project_id: project.id.clone(),
                    issue_id: issue.id.clone(),
                    entity_id: issue.id.clone(),
                    workspace_type: crate::product::models::WorkspaceType::Story,
                    author_provider: crate::product::models::ProviderName::ClaudeCode,
                    reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                    review_rounds: 1,
                    superpowers_enabled: false,
                    openspec_enabled: false,
                    work_item_plan_options: None,
                })
                .expect("workspace session");
            let checkpoint =
                std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                    paths.issue_lifecycle_root(&project.id, &issue.id),
                ));
            let (tx, _rx) = mpsc::channel(64);
            let mut engine = WorkspaceEngine::new_persistent(
                checkpoint,
                LifecycleStore::new(paths.clone()),
                tx,
                crate::product::workspace_engine::types::WorkspaceSession::from_record(record),
            );
            let worktree = root.join(format!("plan-window-worktree-{index}"));
            std::fs::create_dir_all(&worktree).expect("worktree");
            git_init(&worktree);
            engine.session.repository_path = Some(worktree.clone());
            engines.push(engine);
        }
        let mut engines = engines.into_iter();
        (
            engines.next().expect("first engine"),
            engines.next().expect("second engine"),
            paths,
        )
    }

    /// 回归:两个不同会话的同号 plan run → 窗口 ID 不撞,两窗都能开。
    #[tokio::test]
    async fn plan_window_ids_are_unique_across_sessions() {
        let root = tempfile::tempdir().expect("root");
        let (mut first, mut second, paths) = two_session_engines(root.path());
        // 两个会话跑同一个 run id(生产上跨会话可撞同一 run 号)。
        first.mark_active_run_started("run-2");
        second.mark_active_run_started("run-2");

        let worktree_a = first.session.repository_path.clone().expect("worktree a");
        let worktree_b = second.session.repository_path.clone().expect("worktree b");
        first
            .begin_plan_review_window(
                &crate::product::models::ProviderName::ClaudeCode,
                worktree_a,
            )
            .await
            .expect("first plan review window");
        second
            .begin_plan_review_window(
                &crate::product::models::ProviderName::ClaudeCode,
                worktree_b,
            )
            .await
            .expect("second plan review window must not collide");

        let window_a = first
            .active_review_window
            .as_ref()
            .expect("window a")
            .handle
            .window_id
            .clone();
        let window_b = second
            .active_review_window
            .as_ref()
            .expect("window b")
            .handle
            .window_id
            .clone();
        assert_ne!(
            window_a, window_b,
            "不同会话的同号 plan run 不得撞同一窗口 id"
        );
        assert_eq!(
            window_a,
            crate::product::reviewer_window_store::role_run_window_id(
                &first.session.session_id,
                "run-2"
            ),
            "窗口 id 必须带会话前缀(role_run_window_id)"
        );
        assert_eq!(
            window_b,
            crate::product::reviewer_window_store::role_run_window_id(
                &second.session.session_id,
                "run-2"
            )
        );

        // 两个窗口记录都在 store 里(后者没有顶掉前者)。
        let store = crate::product::reviewer_window_store::ReviewerWindowStore::new(
            paths
                .root()
                .join(crate::product::reviewer_window_store::REVIEWER_WINDOW_STORE_DIR),
        );
        assert_eq!(
            store.load_window(&window_a).expect("record a").state,
            crate::product::reviewer_window_store::ReviewerWindowState::Running
        );
        assert_eq!(
            store.load_window(&window_b).expect("record b").state,
            crate::product::reviewer_window_store::ReviewerWindowState::Running
        );
    }
}

/// REQ-ENV-13（Review Focus 1）：单仓「无窗口」路径 MUST 保持
/// `reviewer_verify_policy() == None`——多仓（lc）会话身份 UUID 策略
/// 不得外溢到单仓形态，否则本该被四家启动检查/网关复验拒绝的无窗
/// 单仓调用会借 UUID 合同通过。
#[cfg(test)]
mod single_repo_review_identity_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};

    #[test]
    fn single_repo_without_window_yields_no_reviewer_verify_policy() {
        let root = tempfile::tempdir().expect("root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());
        let record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "issue_0001".to_string(),
                workspace_type: crate::product::models::WorkspaceType::Story,
                author_provider: crate::product::models::ProviderName::ClaudeCode,
                reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("workspace session");
        let checkpoint =
            std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                paths.issue_lifecycle_root(&record.project_id, &record.issue_id),
            ));
        let (tx, _rx) = mpsc::channel(64);
        let engine = WorkspaceEngine::new_persistent(
            checkpoint,
            LifecycleStore::new(paths.clone()),
            tx,
            crate::product::workspace_engine::types::WorkspaceSession::from_record(record),
        );

        // 单仓形态：未注入 logical gateway；未开评审窗。
        assert!(
            !engine.is_logical_codebase_issue(),
            "前置：无 gateway 注入即单仓形态"
        );
        assert!(engine.active_review_window.is_none(), "前置：未开评审窗");
        assert!(
            engine.reviewer_verify_policy().is_none(),
            "单仓无窗口路径必须保持 None，不得借用会话 UUID 身份铸合同"
        );
    }
}
