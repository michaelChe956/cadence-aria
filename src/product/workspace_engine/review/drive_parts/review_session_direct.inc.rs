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
        // 治本（Task 14b）：本函数的 future 状态（评审子链 + 窗口收口）整体
        // 装箱到堆——调用方（followups 宏体、engine 侧入口）帧里只留胖指针。
        boxed_child(|| async {
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
            boxed_child(|| self.finish_failed_run()).await;
            return;
        };
        // T20b:开窗早于材料采集(生产入口无条件;测试内层直调走
        // `drive_review_session_with_window`,无窗形态=内层测试 seam)。
        // 修复轮 1(F1):窗口目标经聚合根视图感知解析(成员 checkout/
        // direct 原样;聚合视图落唯一/focus 成员;无唯一成员=专用码)。
        let review_target = match self.plan_review_window_worktree() {
            Ok(resolved) => resolved,
            Err(message) => {
                let _ = boxed_child(|| self.event_tx.send(EngineEvent::Error { message })).await;
                boxed_child(|| self.finish_failed_run()).await;
                return;
            }
        };
        if let Err(message) = boxed_child(|| self.begin_plan_review_window(&reviewer, review_target)).await
        {
            let _ = boxed_child(|| self.event_tx.send(EngineEvent::Error { message })).await;
            boxed_child(|| self.finish_failed_run()).await;
            return;
        }
        boxed_child(|| self.drive_review_session_with_window(provider, command_rx)).await;
        // 统一收口(覆盖成功/失败/中止一切退出;内层已收口时幂等)。
        boxed_child(|| self.close_plan_review_window("plan_review_completed")).await;
        })
        .await;
    }

    /// 内层承载(批2 Task 20b):公共入口开窗后进入;测试面可直接调用
    /// (无活动窗=测试 seam,真实 adapter 由四家启动检查/网关复验拒绝)。
    /// 每次 provider 启动前同源复验窗口 Running。
    pub(crate) async fn drive_review_session_with_window(
        &mut self,
        provider: Arc<dyn StreamingProviderAdapter>,
        command_rx: mpsc::Receiver<ProviderCommand>,
    ) {
        // 治本（Task 14b）：本函数的 future 状态（评审子链 + 窗口收口）整体
        // 装箱到堆——调用方（followups 宏体、engine 侧入口）帧里只留胖指针。
        boxed_child(|| async {
        // C2 Task 5（REQ-CRO-05）：reviewer 缺失（空 effective）不驱动 review
        // run——错误事件＋失败收尾，绝不以 Codex 顶替。
        let Some(reviewer) = self.session.reviewer_provider.clone() else {
            let _ = self
                .event_tx
                .send(EngineEvent::Error {
                    message: "reviewer_configuration_missing: review run not started".to_string(),
                })
                .await;
            boxed_child(|| self.finish_failed_run()).await;
            return;
        };
        // 治本（Task 14b）：开窗+采集材料链装箱（构造落在助手栈帧内）。
        let input_result = boxed_child(|| async {
            self.ensure_review_invocation_scope().await?;
            self.build_review_input()
        })
        .await;
        let input = match input_result {
            Ok(input) => input,
            Err(message) => {
                let _ = boxed_child(|| self.event_tx.send(EngineEvent::Error { message })).await;
                boxed_child(|| self.finish_failed_run()).await;
                return;
            }
        };
        if let Some(node_id) = self.active_node_id.clone() {
            let _ =
                boxed_child(|| self.persist_prompt_snapshot(&node_id, input.prompt.clone())).await;
            boxed_child(|| {
                self.emit_execution_event(
                    provider_prompt_event(
                        &node_id,
                        input.prompt.clone(),
                        "发送给 Workspace provider 的完整提示词",
                    ),
                    Some(node_id),
                    Some(reviewer.clone()),
                )
            })
            .await;
        }
        let mut command_rx = command_rx;
        // P1-2：reviewer 是策略角色——与 author 主流同法绑定 run-bound durable
        // sink（持久 store 缺失时不接线，真实 adapter 对 policy+缺 sink fail-closed）。
        let input = self.attach_tool_policy_audit(input);
        // T20b(决策项⑦):provider 启动前同源复验——窗口 Running 且材料
        // 与 baseline 同指纹(fail-closed,不无窗启动)。
        if let Err(message) = self.assert_plan_review_window() {
            let _ = boxed_child(|| self.event_tx.send(EngineEvent::Error { message })).await;
            boxed_child(|| self.finish_failed_run()).await;
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
        // 治本（Task 14b）：重子调用装箱——provider 驱动的 future 状态不再内联
        // 在本函数帧里（本帧只留胖指针；poll 帧预留随之下沉）。
        let first_result = boxed_child(|| {
            self.drive_reviewer_provider_session_once(first_session, &mut command_rx, &reviewer)
        })
        .await;
        let first_completion = match first_result {
            ReviewProviderRunResult::Completed(completion) => completion,
            ReviewProviderRunResult::Aborted => return,
            ReviewProviderRunResult::Failed(failure) => {
                boxed_child(|| self.finish_review_provider_run_failure(failure)).await;
                return;
            }
        };

        match self.parse_review_completion_for_active_node(&first_completion) {
            Ok(verdict) => boxed_child(|| self.complete_review(first_completion, verdict)).await,
            // 分类枚举错误必须直接沿 fallback diagnostic → policy fatal
            // 路径收口，不能进入 JSON repair 或人工 gate。
            Err(first_error) if first_error.is_classification_fatal() => {
                let verdict = fallback_review_verdict(&first_completion, &first_error, false);
                boxed_child(|| self.complete_review(first_completion, verdict)).await;
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
                        boxed_child(|| self.complete_review(first_completion, verdict)).await;
                        return;
                    }
                };
                let repair_node_id = self.active_node_id.clone();
                boxed_child(|| {
                    self.emit_execution_event(
                        structured_output_repair_event(
                            ProviderExecutionEventStatus::Started,
                            first_error.code(),
                        ),
                        repair_node_id.clone(),
                        Some(reviewer.clone()),
                    )
                })
                .await;
                let repair_input = self.attach_tool_policy_audit(repair_input);
                // T20b:repair 入口同窗复验(恢复成功不洗白;窗口失效
                // 即停,不换窗续跑)。
                if let Err(message) = self.assert_plan_review_window() {
                    let _ =
                        boxed_child(|| self.event_tx.send(EngineEvent::Error { message })).await;
                    boxed_child(|| self.finish_failed_run()).await;
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
                        boxed_child(|| {
                            self.emit_execution_event(
                                structured_output_repair_event(
                                    ProviderExecutionEventStatus::Failed,
                                    first_error.code(),
                                ),
                                repair_node_id,
                                Some(reviewer.clone()),
                            )
                        })
                        .await;
                        return;
                    }
                    ReviewProviderRunResult::Failed(_) => {
                        boxed_child(|| {
                            self.emit_execution_event(
                                structured_output_repair_event(
                                    ProviderExecutionEventStatus::Failed,
                                    first_error.code(),
                                ),
                                repair_node_id,
                                Some(reviewer.clone()),
                            )
                        })
                        .await;
                        let verdict =
                            fallback_review_verdict(&first_completion, &first_error, true);
                        boxed_child(|| self.complete_review(first_completion, verdict)).await;
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
                        boxed_child(|| {
                            self.emit_execution_event(
                                structured_output_repair_event(
                                    ProviderExecutionEventStatus::Completed,
                                    first_error.code(),
                                ),
                                repair_node_id.clone(),
                                Some(reviewer.clone()),
                            )
                        })
                        .await;
                        boxed_child(|| self.complete_review(normalized, verdict)).await;
                    }
                    Ok(_) => {
                        let error = ReviewCompletionError::RepairPayloadChanged;
                        boxed_child(|| {
                            self.emit_execution_event(
                                structured_output_repair_event(
                                    ProviderExecutionEventStatus::Failed,
                                    error.code(),
                                ),
                                repair_node_id.clone(),
                                Some(reviewer.clone()),
                            )
                        })
                        .await;
                        let verdict = fallback_review_verdict(&first_completion, &error, true);
                        boxed_child(|| self.complete_review(first_completion, verdict)).await;
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
                        boxed_child(|| {
                            self.emit_execution_event(
                                structured_output_repair_event(
                                    ProviderExecutionEventStatus::Failed,
                                    second_error.code(),
                                ),
                                repair_node_id,
                                Some(reviewer.clone()),
                            )
                        })
                        .await;
                        let verdict = fallback_review_verdict(&normalized, &second_error, true);
                        boxed_child(|| self.complete_review(normalized, verdict)).await;
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
                    let _ =
                        boxed_child(|| self.event_tx.send(EngineEvent::Error { message })).await;
                    boxed_child(|| self.finish_failed_run()).await;
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
                boxed_child(|| self.complete_review(first_completion, verdict)).await;
            }
        }
        })
        .await;
    }

}
