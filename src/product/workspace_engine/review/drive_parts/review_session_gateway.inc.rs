impl WorkspaceEngine {
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
        // 治本（Task 14b）：本函数的 future 状态（评审子链 + 窗口收口）整体
        // 装箱到堆——调用方（followups 宏体、engine 侧入口）帧里只留胖指针。
        boxed_child(|| async {
        // Task 2(REQ-ENV-14,设计 A 条目 3):多仓(lc)形态不开评审窗——
        // gateway 版公共入口同构直入内层承载,材料断言替代窗口断言。
        if self.is_logical_codebase_issue() {
            // 治本（Task 14b）：内层承载 future 的构造临时区落在装箱边界内
            // ——本帧只留胖指针（评审链的嵌套 poll 帧体积同步下降）。
            return boxed_child(|| self.drive_review_session_via_gateway_with_window(command_rx)).await;
        }
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
        // 治本（Task 14b）：内层承载 future 经装箱边界进入——本帧只留胖指针。
        boxed_child(|| self.drive_review_session_via_gateway_with_window(command_rx)).await;
        boxed_child(|| self.close_plan_review_window("plan_review_completed")).await;
        })
        .await;
    }

    /// gateway 版内层承载(测试 seam 同 direct 版)。
    ///
    /// 治本（Task 14b，栈根修）：本函数的**重子调用**（评审完成、provider
    /// 驱动、事件回呈等）一律经 `boxed_child` 装箱——子 future 的构造临时区
    /// 落在助手自己的栈帧内、随即装箱到堆，本函数帧里只剩胖指针（修前本
    /// 函数 poll 帧预留 768,440 B）。
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
            boxed_child(|| self.finish_failed_run()).await;
            return;
        };
        // 治本（Task 14b）：把「开窗 + 采集材料」两级链装箱一层——本函数的 poll
        // 帧不再内联该链的构造临时区（评审链嵌套 poll 深度降一层）。语义与原
        // `ensure_review_invocation_scope().await.and_then(|_| build_review_input())`
        // 逐字等价（开窗失败即原样上浮 message）。
        let review_input_result = boxed_child(|| async {
            self.ensure_review_invocation_scope().await?;
            self.build_review_input()
        })
        .await;
        let input = match review_input_result {
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
            let _ = boxed_child(|| self.event_tx.send(EngineEvent::Error { message })).await;
            boxed_child(|| self.finish_failed_run()).await;
            return;
        }
        // F-19：gateway review 路径同样登记 reviewer provider start（k3 P2——
        // drive_review_session 的镜像，logical repository 会话实际走此分支）。
        self.register_provider_start_in_ledger(
            ProviderConversationRole::Reviewer,
            reviewer.clone(),
        );
        // 闭包只捕获 owned/共享引用（不捕获 `&self`——engine 在测试构建下非
        // Sync，捕获 `&self` 会让闭包失去 Send）。
        let first_launch_ownership = self.plan_review_launch_ownership();
        let first_cancel = self.cancel.clone();
        let first_input = input.clone();
        let first_project_id = project_id.clone();
        let (first_session, first_trust_waiting) = boxed_child(|| {
            start_review_session_via_gateway_owned(
                &gateway,
                &reviewer,
                first_input,
                first_project_id,
                first_launch_ownership,
                first_cancel,
            )
        })
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
                // 指引卡简化补缺：trust 分型命中时，紧随 error 事件之后
                // 发出结构化指导（启动成功恒 None，其余 Failed 形态同样
                // 零发出）。
                boxed_child(|| self.emit_launch_trust_guidance(first_trust_waiting)).await;
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
                // 修复轮：首发已成功启动（trust 已复验通过），结构化等待
                // 恒 None——解包丢弃。
                // T20b:repair 入口同窗复验。
                if let Err(message) = self.assert_plan_review_window() {
                    let _ =
                        boxed_child(|| self.event_tx.send(EngineEvent::Error { message })).await;
                    boxed_child(|| self.finish_failed_run()).await;
                    return;
                }
                let repair_launch_ownership = self.plan_review_launch_ownership();
                let repair_cancel = self.cancel.clone();
                let (repair_session, _repair_trust_waiting) = boxed_child(|| {
                    start_review_session_via_gateway_owned(
                        &gateway,
                        &reviewer,
                        repair_input,
                        project_id,
                        repair_launch_ownership,
                        repair_cancel,
                    )
                })
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
                    let _ =
                        boxed_child(|| self.event_tx.send(EngineEvent::Error { message })).await;
                    boxed_child(|| self.finish_failed_run()).await;
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
                boxed_child(|| self.complete_review(first_completion, verdict)).await;
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
            let _ = boxed_child(|| self.event_tx.send(EngineEvent::Error { message })).await;
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
                    let _ =
                        boxed_child(|| self.event_tx.send(EngineEvent::Error { message })).await;
                }
                Err(message) => {
                    let _ = self
                        .event_tx
                        .send(EngineEvent::Error {
                            message: format!("persist lc superseded wait failed: {message}"),
                        })
                        .await;
                    boxed_child(|| self.finish_failed_run()).await;
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
                let _ = boxed_child(|| self.event_tx.send(EngineEvent::Error { message })).await;
                boxed_child(|| self.finish_failed_run()).await;
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
                    Some(author.clone()),
                )
            })
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
                    let _ =
                        boxed_child(|| self.event_tx.send(EngineEvent::Error { message })).await;
                    boxed_child(|| self.finish_failed_run()).await;
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
                    let _ =
                        boxed_child(|| self.event_tx.send(EngineEvent::Error { message })).await;
                    boxed_child(|| self.finish_failed_run()).await;
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

}
