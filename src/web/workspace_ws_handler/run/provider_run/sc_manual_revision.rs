//! S0（任务1，R2 条件前置触发）：SC manual revision 臂。原
//! `provider_run.rs` `:1020–1639` 逐字迁移：fresh 重入判定、native mismatch
//! precheck、structural claim pending、attempt prompt 装配、同 turn attempt
//! 循环（恰一次 claim 消费）、冻结交付 join、ledger 登记与失败回显的顺序
//! 原样保留。原 drive 级 `return` 的出口映射 `ArmExit::Stop`；正常穿过
//! （Accepted/break/terminal 拒收）映射 `Continue(engine)`。
//!
//! 三守卫不可混用（r16 现场死锁根修，缺陷#10家族——设计 §3.4）：
//! - engine 的 `MutexGuard` 按值属 drive/当前臂：drop 取消 drive 必须释放它，
//!   且先于兜底重取锁；
//! - idle `provider_drive_guard` 是外层 `Option<WorkspaceProviderDriveGuard>`
//!   （臂借 `&mut`，drop drive **不等于**释放 idle）；
//! - `entity_run_lease` 是独立外层 binding（不传给臂）。
//!
//! 不得用「provider_drive_guard 随 drive 一律释放」的歧义说法代替三者合同。

use super::task_body::{ArmDrive, ArmExit, boxed_child};
use super::*;
use std::pin::Pin;

/// SC manual revision 臂（原 `:1020–1639`，机械迁移）。
pub(crate) fn drive_human_gate_sc_revision_arm<'e, 'd>(
    drive: ArmDrive<'e, 'd>,
    turn_id: String,
    prompt: String,
) -> Pin<Box<dyn Future<Output = ArmExit<'e>> + Send + 'd>>
where
    'e: 'd,
{
    Box::pin(async move {
        let ArmDrive {
            mut engine,
            provider: provider_for_run,
            command_rx,
            manager: manager_for_task,
            run_context: run_context_clone,
            outbound_tx: outbound_tx_for_task,
            run_label,
            run_token,
            provider_drive_guard,
            run_cancel,
            // S2：ArmDrive 新增 drive 尾部字段（provider_registry/rebuild_context
            // 等）本臂不消费——`..` 忽略，不改变臂内绑定面。
            ..
        } = drive;
        use crate::product::models::HumanGateTurnFailureClass;
        let mut command_rx = command_rx;
        // B2d（Task 5.6，设计 §5.3 sc_manual_revision 行）：
        // FreshRunning 的 sc_manual recovery → 同 turn fresh 重入
        //（重装同 turn prompt、resume=None 全新 native、
        // run_sc_manual_revision_turn 同 turn 完成回原 human gate，
        // 不新 turn/attempt/预算）；Ordinary 先行 native mismatch
        // precheck——`FrozenBindingUnavailable` 家族转 typed 等待
        //（persist 后原 turn/gate/预算保留，不 fail turn、不
        // rearm、零 provider start）。读取失败按 Message 面收口
        //（不确定等待/失败归属时不落 turn 失败）。
        let manual_fresh_recovery = match lc_fresh_recovery_record_if_running(
            &run_context_clone.app_paths,
            &engine.session().session_id,
        ) {
            Ok(Some(recovery)) => matches!(
                &recovery.run_context,
                crate::product::models::LcRecoveryRunContext::ScManualRevision {
                    turn_id: recovery_turn_id,
                    ..
                } if recovery_turn_id == &turn_id
            ),
            Ok(None) => false,
            Err(message) => {
                engine.mark_active_run_finished(run_label);
                drop(engine);
                drop(provider_drive_guard.take());
                boxed_child(|| manager_for_task.finish_run(run_token)).await;
                let _ = send_json_outbound(outbound_tx_for_task, &WsOutMessage::Error { message })
                    .await;
                return ArmExit::Stop;
            }
        };
        if !manual_fresh_recovery {
            match engine.precheck_sc_manual_revision_resume() {
                Ok(None) => {}
                Ok(Some(reason_code)) => {
                    match engine
                        .persist_sc_manual_revision_superseded_wait(&turn_id, reason_code)
                        .await
                    {
                        Ok(recovery) => {
                            // typed 等待收口：无 failed 节点（等待
                            // 不是失败）、turn/gate/预算原样保留，
                            // 等待消息上浮显式 fresh 出口。
                            engine.mark_active_run_finished(run_label);
                            drop(engine);
                            drop(provider_drive_guard.take());
                            boxed_child(|| manager_for_task.finish_run(run_token)).await;
                            let superseded_native = recovery
                                .superseded_native_session_id
                                .as_deref()
                                .unwrap_or("unknown");
                            let message = format!(
                                "sc manual revision native resume superseded ({reason_code}): \
                                             the old native session {superseded_native} can no longer be resumed \
                                             (recovery {}); no provider was started and the original \
                                             turn/gate/budget are preserved; 请通过显式恢复操作启动新原生会话继续同turn修订（explicit fresh）",
                                recovery.recovery_id
                            );
                            let _ = send_json_outbound(
                                outbound_tx_for_task,
                                &WsOutMessage::Error { message },
                            )
                            .await;
                            return ArmExit::Stop;
                        }
                        Err(message) => {
                            let _ = boxed_child(|| {
                                engine.fail_human_gate_turn(
                                    &turn_id,
                                    HumanGateTurnFailureClass::ProviderErr,
                                )
                            })
                            .await;
                            engine.mark_active_run_finished(run_label);
                            drop(engine);
                            drop(provider_drive_guard.take());
                            boxed_child(|| manager_for_task.finish_run(run_token)).await;
                            let _ = send_json_outbound(
                                outbound_tx_for_task,
                                &WsOutMessage::HumanGateTurnFailed {
                                    turn_id: turn_id.clone(),
                                    failure_class: "provider_err".to_string(),
                                    message,
                                },
                            )
                            .await;
                            return ArmExit::Stop;
                        }
                    }
                }
                Err(message) => {
                    let _ = boxed_child(|| {
                        engine
                            .fail_human_gate_turn(&turn_id, HumanGateTurnFailureClass::ProviderErr)
                    })
                    .await;
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    drop(provider_drive_guard.take());
                    boxed_child(|| manager_for_task.finish_run(run_token)).await;
                    let _ = send_json_outbound(
                        outbound_tx_for_task,
                        &WsOutMessage::HumanGateTurnFailed {
                            turn_id: turn_id.clone(),
                            failure_class: "provider_err".to_string(),
                            message,
                        },
                    )
                    .await;
                    return ArmExit::Stop;
                }
            }
        }
        // SG6a.2：durable 活动 grant（phase=Attempt2Claimed）＝
        // 本 turn 的 attempt2 已被唯一 claim（冻结机器指令
        // prompt 身份在案）。冻结全文只随 claim 时的 in-flight
        // run 内联交付，跨 spawn 不可重建——原反馈 prompt 重入
        // 会错消费唯一 attempt2（REQ-CG-03 机器指令驱动；P2
        // 修复 aa823108 恢复车道语义的 spawn 侧防线）。读取
        // 失败按 pending 处理（fail-closed 等待，零 start），
        // 终结收口归 SG6a.3。
        let structural_claim_pending = LifecycleStore::new(
                        run_context_clone.app_paths.clone(),
                    )
                    .get_workspace_session(&engine.session().session_id)
                    .map(|record| {
                        record
                            .structural_governance_state
                            .as_ref()
                            .and_then(|state| state.current_grant.as_ref())
                            .is_some_and(|grant| {
                                grant.turn_id.as_deref() == Some(turn_id.as_str())
                                    && grant.phase
                                        == Some(crate::product::models::workspace::StructuralGrantPhase::Attempt2Claimed)
                            })
                    })
                    .unwrap_or(true);
        if structural_claim_pending && !manual_fresh_recovery {
            engine.mark_active_run_finished(run_label);
            drop(engine);
            drop(provider_drive_guard.take());
            boxed_child(|| manager_for_task.finish_run(run_token)).await;
            let _ = send_json_outbound(
                            outbound_tx_for_task,
                            &WsOutMessage::ProtocolError {
                                code: SC_MANUAL_STRUCTURAL_CLAIM_PENDING_CODE.to_string(),
                                message: format!(
                                    "sc manual revision turn {turn_id} has a claimed structural attempt2 \
                                     (frozen machine-instruction prompt identity on record); the frozen \
                                     delivery is consumed by the in-flight run and cannot be reassembled \
                                     by a new spawn; no provider was started and the turn/gate/budget \
                                     are preserved; waiting for structural closure"
                                ),
                                context: None,
                            },
                        )
                        .await;
            return ArmExit::Stop;
        }
        let mut attempt_prompt = if manual_fresh_recovery || prompt.is_empty() {
            let turn = match LifecycleStore::new(run_context_clone.app_paths.clone())
                .get_human_gate_turn(&engine.session().session_id, &turn_id)
            {
                Ok(turn) => turn,
                Err(error) => {
                    let message = format!("load human gate turn failed: {error}");
                    let _ = boxed_child(|| {
                        engine
                            .fail_human_gate_turn(&turn_id, HumanGateTurnFailureClass::ProviderErr)
                    })
                    .await;
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let _ = send_json_outbound(
                        outbound_tx_for_task,
                        &WsOutMessage::HumanGateTurnFailed {
                            turn_id,
                            failure_class: "provider_err".to_string(),
                            message,
                        },
                    )
                    .await;
                    return ArmExit::Stop;
                }
            };
            // B2d：fresh 重入按 §5.3 同 turn 语义重装——重读同
            // turn feedback 并对 candidate hash 复验（漂移按业务
            // 校验规则 validation_reject 收口，与恢复面 source-hash
            // 失配同族；不用旧 turn 权限强行修订漂移后的候选）。
            let built = if manual_fresh_recovery {
                engine.build_sc_manual_revision_prompt_for_fresh_turn(&turn)
            } else {
                engine.build_sc_manual_revision_prompt_for_turn(&turn.feedback_text)
            };
            match built {
                Ok(prompt) => prompt,
                Err(message) => {
                    let (failure_class, class_label) =
                        if message.starts_with("HUMAN_GATE_REVISION_CANDIDATE_DRIFTED") {
                            (
                                HumanGateTurnFailureClass::ValidationReject,
                                "validation_reject",
                            )
                        } else {
                            (HumanGateTurnFailureClass::ProviderErr, "provider_err")
                        };
                    let _ =
                        boxed_child(|| engine.fail_human_gate_turn(&turn_id, failure_class)).await;
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let _ = send_json_outbound(
                        outbound_tx_for_task,
                        &WsOutMessage::HumanGateTurnFailed {
                            turn_id,
                            failure_class: class_label.to_string(),
                            message,
                        },
                    )
                    .await;
                    return ArmExit::Stop;
                }
            }
        } else {
            prompt
        };

        // Durable state must prove Running before the provider is started. This is
        // deliberately inside the spawned task, after the handler has cancelled any
        // previous run, so a crash cannot leave Reserved while a provider is active.
        if let Err(message) = engine.mark_human_gate_turn_running(&turn_id) {
            let _ = boxed_child(|| {
                engine.fail_human_gate_turn(&turn_id, HumanGateTurnFailureClass::ProviderErr)
            })
            .await;
            engine.mark_active_run_finished(run_label);
            drop(engine);
            let _ = send_json_outbound(
                outbound_tx_for_task,
                &WsOutMessage::HumanGateTurnFailed {
                    turn_id,
                    failure_class: "provider_err".to_string(),
                    message,
                },
            )
            .await;
            return ArmExit::Stop;
        }

        // F-49/A6：门修订轮必须与普通 SC 修订同构地落在 author 节点上。
        // 此前取 `active_timeline_node_id()`——门内活动节点就是 HumanConfirm
        // 门节点，修订 prompt/输出流/artifact_ref 与产物版本 source_node_id
        // 全部写进门节点 detail；门节点在对话流 rebuild 判 role=null，整节点
        // 零条目，用户侧「author 修订步不可见」。选节点规则（复用活动
        // AuthorRun / 新建）见 `begin_work_item_plan_human_gate_revision_run`。
        let node_id = boxed_child(|| engine.begin_work_item_plan_human_gate_revision_run()).await;
        let author_provider = engine.session().author_provider.clone();
        // B2b1(Task 5.2,REQ-PLN-08):修订 turn 与 fresh 轮的
        // envelope target 同源恒聚合根锚(E native anchor 消费面
        // 已收敛进 `resolve_plan_author_launch`,成员锚参数移除
        // ——r27 的锚漂移机制在 root 锚下结构性消失)。SG6a.2:
        // sameauthor/gateway key——launch 循环外解析一次,两次
        // 物理 attempt 共用同一 gateway 启动身份。
        let launch = resolve_plan_author_launch(&engine);
        let launch = match launch {
            Ok(launch) => launch,
            Err(error) => {
                let message = error.details.clone();
                let _ = engine
                    .fail_human_gate_turn(&turn_id, HumanGateTurnFailureClass::ProviderErr)
                    .await;
                engine.mark_active_run_finished(run_label);
                drop(engine);
                let _ = send_json_outbound(
                    outbound_tx_for_task,
                    &WsOutMessage::HumanGateTurnFailed {
                        turn_id,
                        failure_class: "provider_err".to_string(),
                        message,
                    },
                )
                .await;
                return ArmExit::Stop;
            }
        };
        // SG6a.2(REQ-CG-03):同 turn 物理 attempt 循环——含原
        // 调用最多 2,claim 与真实 start 分账。attempt1=原反馈
        // prompt;attempt1 结构拒收携案 claim 时以冻结机器指令
        // prompt 内联驱动 attempt2(冻结交付 join 拒→零 start)。
        let mut structural_claim_consumed = false;
        loop {
            engine
                .emit_provider_prompt_event(
                    &node_id,
                    attempt_prompt.clone(),
                    "发送给 SC human-gate revision provider 的完整修订提示词",
                    Some(author_provider.clone()),
                )
                .await;
            let worktree_path = engine
                .session()
                .repository_path
                .as_ref()
                .cloned()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            // B2d：fresh 重入全新 native（resume_provider_session_id=None
            // 结构保证——`build_work_item_plan_streaming_input_fresh`）；
            // Ordinary 保持既有 resume 语义（行为不变）。
            let provider_input = if manual_fresh_recovery {
                engine.build_work_item_plan_streaming_input_fresh(
                    crate::product::work_item_split_engine::types::provider_name_to_type(
                        &author_provider,
                    ),
                    attempt_prompt.clone(),
                    worktree_path.to_string_lossy().to_string(),
                    author_provider.clone(),
                    PlanAuthorOutputContract::Structured,
                )
            } else {
                engine.build_work_item_plan_streaming_input(
                    crate::product::work_item_split_engine::types::provider_name_to_type(
                        &author_provider,
                    ),
                    attempt_prompt.clone(),
                    worktree_path.to_string_lossy().to_string(),
                    author_provider.clone(),
                    PlanAuthorOutputContract::Structured,
                )
            };
            let provider_input = match provider_input {
                Ok(provider_input) => provider_input,
                Err(message) => {
                    // REQ-PIB-02（T2.3）：基线不可解析 → 门内轮次失败（可观测）。
                    let _ = boxed_child(|| {
                        engine
                            .fail_human_gate_turn(&turn_id, HumanGateTurnFailureClass::ProviderErr)
                    })
                    .await;
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let _ = send_json_outbound(
                        outbound_tx_for_task,
                        &WsOutMessage::HumanGateTurnFailed {
                            turn_id,
                            failure_class: "provider_err".to_string(),
                            message,
                        },
                    )
                    .await;
                    return ArmExit::Stop;
                }
            };
            let provider_input = engine.attach_tool_policy_audit(provider_input);
            // Task 1b 段②生产臂:门修订轮单次 provider run 同样绑定
            // split run 身份;Markdown 修订产物按原文存档收口。
            let plan_split_run = match begin_plan_split_run_if_logical(
                &launch,
                &crate::product::lifecycle_store::LifecycleStore::new(
                    run_context_clone.app_paths.clone(),
                ),
                &run_context_clone.session_record.project_id,
                &run_context_clone.session_record.issue_id,
                &author_provider,
                &engine.session().session_id,
            ) {
                Ok(run) => run,
                Err(message) => {
                    let _ = boxed_child(|| {
                        engine
                            .fail_human_gate_turn(&turn_id, HumanGateTurnFailureClass::ProviderErr)
                    })
                    .await;
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let _ = send_json_outbound(
                        outbound_tx_for_task,
                        &WsOutMessage::HumanGateTurnFailed {
                            turn_id,
                            failure_class: "provider_err".to_string(),
                            message,
                        },
                    )
                    .await;
                    return ArmExit::Stop;
                }
            };
            // T6 fix-loop r2：trust 分类已下沉
            // start_work_item_plan_author 的 Logical 分支共享口。
            let provider_session = boxed_child(|| {
                start_work_item_plan_author(
                    launch.clone(),
                    provider_for_run.clone(),
                    provider_input,
                    run_cancel.clone(),
                    plan_split_run.as_ref(),
                )
            })
            .await;
            // 治本（Task 14b）：重子调用装箱——provider 驱动的 future 状态不再
            // 内联在本臂帧里（本臂帧只留胖指针；poll 帧预留随之下沉）。
            let full_output_result = boxed_child(|| {
                engine.drive_work_item_plan_provider_session_to_output(
                    provider_session,
                    &mut command_rx,
                    node_id.clone(),
                    author_provider.clone(),
                )
            })
            .await;
            let full_output = match full_output_result {
                Ok(output) => output,
                Err(message) => {
                    // REQ-CG-10：取消 / 终止导致的驱动错误归 Cancelled（节点 Aborted +
                    // 「选择未生效」），真实 provider 失败仍 provider_err。
                    let outcome =
                        crate::product::workspace_engine::conversational_gate::cancel::revision_drive_error_outcome(
                            &message,
                            run_cancel.is_cancelled(),
                        );
                    fail_plan_split_run(
                        plan_split_run.as_ref(),
                        &format!("provider session drive failed: {message}"),
                    );
                    let _ = boxed_child(|| {
                        engine.fail_human_gate_turn(&turn_id, outcome.failure_class.clone())
                    })
                    .await;
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let _ = send_json_outbound(
                        outbound_tx_for_task,
                        &WsOutMessage::HumanGateTurnFailed {
                            turn_id,
                            failure_class: outcome.label.to_string(),
                            message: outcome.frame_message,
                        },
                    )
                    .await;
                    return ArmExit::Stop;
                }
            };
            close_plan_split_run_with_markdown(
                plan_split_run.as_ref(),
                &attempt_prompt,
                &full_output,
            );
            let sc_revision_result =
                boxed_child(|| engine.run_sc_manual_revision_turn(&turn_id, full_output.clone()))
                    .await;
            match sc_revision_result {
                Ok(crate::product::workspace_engine::ScManualRevisionResult::Accepted {
                    artifact_ref,
                }) => {
                    // SG6b（REQ-CG-03:74 / SG6a.3⑥移交「帧时序」）：
                    // human_gate_turn_completed 只在 turn durable
                    // Completed（synthetic 收口）后回呈；有 reviewer
                    // 分支 turn 仍 Running（verdict 面收口），帧由
                    // ReviewOnly 臂在 verdict 收口后发送。
                    let turn_closed = LifecycleStore::new(run_context_clone.app_paths.clone())
                        .get_human_gate_turn(&engine.session().session_id, &turn_id)
                        .map(|turn| {
                            turn.status == crate::product::models::HumanGateTurnStatus::Completed
                        })
                        .unwrap_or(false);
                    if turn_closed {
                        let _ = send_json_outbound(
                            outbound_tx_for_task,
                            &WsOutMessage::HumanGateTurnCompleted {
                                turn_id: turn_id.clone(),
                                artifact_ref,
                            },
                        )
                        .await;
                    }
                    break;
                }
                Ok(
                    crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected {
                        diagnostics,
                        claim,
                    },
                ) => {
                    let carry = match claim {
                        // 唯一 claim 恰消费一次：attempt2 的再拒收
                        // 恒不携案（Replayed/Exhausted 零新 claim）。
                        Some(carry) if !structural_claim_consumed => carry,
                        _ => {
                            // SG6a.2 in-band 回显 + REQ-WSC-14 终态可操作回执
                            // （帧构造与 durable 落盘在 engine 侧单点）。
                            for frame in boxed_child(|| {
                                engine.rejection_frames_for_terminal(
                                    &turn_id,
                                    SC_MANUAL_REVISION_VALIDATION_REJECTED_CODE,
                                    &diagnostics,
                                )
                            })
                            .await
                            {
                                let _ = send_json_outbound(outbound_tx_for_task, &frame).await;
                            }
                            break;
                        }
                    };
                    // 冻结交付精确 join（SG5.3）：input/fingerprint
                    // 拒绝在任何真实 start 之前稳定码回显——零
                    // start、turn 不终态、typed 等待。
                    let turn_now = match LifecycleStore::new(run_context_clone.app_paths.clone())
                        .get_human_gate_turn(&engine.session().session_id, &turn_id)
                    {
                        Ok(turn) => turn,
                        Err(error) => {
                            let message = format!("load human gate turn failed: {error}");
                            let _ = boxed_child(|| {
                                engine.fail_human_gate_turn(
                                    &turn_id,
                                    HumanGateTurnFailureClass::ProviderErr,
                                )
                            })
                            .await;
                            engine.mark_active_run_finished(run_label);
                            drop(engine);
                            let _ = send_json_outbound(
                                outbound_tx_for_task,
                                &WsOutMessage::HumanGateTurnFailed {
                                    turn_id: turn_id.clone(),
                                    failure_class: "provider_err".to_string(),
                                    message,
                                },
                            )
                            .await;
                            return ArmExit::Stop;
                        }
                    };
                    // 交付 join 的候选基线窗口（SG6a.2 P3）：布置
                    // 失败中间候选为 join 对账视图、精确 join——
                    // 基线还原合同由 `deliver_attempt2_frozen_prompt`
                    // 统一承载（attempt2 未成行不污染内存修订基
                    // 线，见其注释）。
                    let frozen_prompt = match deliver_attempt2_frozen_prompt(
                        &mut engine,
                        &turn_now,
                        full_output.as_str(),
                        &carry,
                    ) {
                        Ok(frozen_prompt) => frozen_prompt,
                        Err(message) => {
                            engine.mark_active_run_finished(run_label);
                            drop(engine);
                            drop(provider_drive_guard.take());
                            boxed_child(|| manager_for_task.finish_run(run_token)).await;
                            let _ = send_json_outbound(
                                outbound_tx_for_task,
                                &WsOutMessage::ProtocolError {
                                    code: SC_MANUAL_ATTEMPT2_DELIVERY_REJECTED_CODE.to_string(),
                                    message,
                                    context: None,
                                },
                            )
                            .await;
                            return ArmExit::Stop;
                        }
                    };
                    // 实际 launch observation 落 ledger（REQ-SG-05
                    // claim 与真实 start 分账；claim 已保证 key 缺
                    // 席，watchdog 同款 serde 投影＋内存同步）。
                    // 登记失败 fail-closed：零 start、typed 等待。
                    let provider_text = serde_json::to_value(&author_provider)
                        .ok()
                        .and_then(|value| value.as_str().map(ToString::to_string))
                        .unwrap_or_else(|| "unknown".to_string());
                    let started_at = chrono::Utc::now().to_rfc3339();
                    match LifecycleStore::new(run_context_clone.app_paths.clone())
                        .claim_provider_start_with_details(
                            &engine.session().session_id,
                            &carry.provider_start_key,
                            Some(provider_text.as_str()),
                            Some(started_at.clone()),
                        ) {
                        Ok(true) => {
                            engine.session.provider_start_ledger.push(
                                crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
                                    provider_start_idempotency_key: carry
                                        .provider_start_key
                                        .clone(),
                                    started: true,
                                    provider: Some(provider_text),
                                    started_at: Some(started_at),
                                    provider_attempt_key: None,
                                },
                            );
                        }
                        Ok(false) => {
                            // 崩溃重放：实际 launch 观察已在案
                            // ——幂等续跑同 attempt，不二次计。
                        }
                        Err(error) => {
                            engine.mark_active_run_finished(run_label);
                            drop(engine);
                            drop(provider_drive_guard.take());
                            boxed_child(|| manager_for_task.finish_run(run_token)).await;
                            let _ = send_json_outbound(
                                        outbound_tx_for_task,
                                        &WsOutMessage::ProtocolError {
                                            code: SC_MANUAL_ATTEMPT2_DELIVERY_REJECTED_CODE.to_string(),
                                            message: format!(
                                                "STRUCTURAL_ATTEMPT2_LAUNCH_REGISTRATION_FAILED: {error}; no provider was started and the turn/gate/budget are preserved"
                                            ),
                                            context: None,
                                        },
                                    ).await;
                            return ArmExit::Stop;
                        }
                    }
                    structural_claim_consumed = true;
                    attempt_prompt = frozen_prompt;
                    continue;
                }
                Err(message) => {
                    let _ = boxed_child(|| {
                        engine
                            .fail_human_gate_turn(&turn_id, HumanGateTurnFailureClass::ProviderErr)
                    })
                    .await;
                    let _ = send_json_outbound(
                        outbound_tx_for_task,
                        &WsOutMessage::HumanGateTurnFailed {
                            turn_id: turn_id.clone(),
                            failure_class: "provider_err".to_string(),
                            message,
                        },
                    )
                    .await;
                    break;
                }
            }
        } // SG6a.2：同 turn attempt 循环收口（恰一次 claim 消费）
        ArmExit::Continue(engine)
    })
}
