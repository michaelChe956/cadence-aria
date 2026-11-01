// 经 automation_enrollment.rs 同模块 `include!` 引入（large_file_guard 1200 行红线
// 拆分，纯移动自原文件 901-1208 行，内容逐字不变）。
/// 批 2 T3（块 3 §4）：continue-chain——advance/coding_start 停点的第三种
/// 显式继续。advance 沿 R1 冻结 policy 复用引擎 retry-initialization；
/// coding_start 仅重放原 Claimed/RunnerRegistered claim（越 barrier 不清
/// claim，走既有 RecoverCoding/RestartCoding 恢复并核 auto）。成功 resolve
/// 原事实＋wake（auto 保留）；NeedsHuman/失败保留原事实。
pub async fn post_automation_enrollment_continue_chain(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<crate::web::types::AutomationContinueChainRequest>,
) -> ApiResult<Json<crate::web::types::AutomationContinueChainResponse>> {
    use crate::product::models::automation::{AutomationAttentionKind, AutomationAttentionStage};
    use crate::web::types::AutomationContinueChainResponse;

    validate_request_ids(&project_id, &issue_id)?;
    for id in [&request.command_id, &request.expected_stop_key] {
        validate_relative_id(id).map_err(|error| {
            ApiError::validation(
                "automation_continue_chain_invalid_id",
                format!("invalid continue-chain id: {error}"),
            )
        })?;
    }
    ensure_issue_exists(&state, &project_id, &issue_id)?;
    let paths = product_app_paths(&state);
    let store = IssueAutomationStore::new(paths.clone());
    let enrollment = store
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .ok_or_else(enrollment_not_found)?;
    if enrollment.terminated_at.is_some() {
        return Err(enrollment_conflict(
            Some(enrollment.policy_revision),
            "terminated enrollment cannot continue",
        ));
    }
    if !enrollment.enabled {
        return Err(enrollment_conflict(
            Some(enrollment.policy_revision),
            "automation enrollment is disabled",
        ));
    }
    let binding = enrollment
        .binding_history
        .as_ref()
        .map(|history| history.current.clone())
        .ok_or_else(|| {
            invalid_scope("enrollment has no versioned binding; cannot continue the chain")
        })?;
    if binding != request.expected_binding {
        return Err(enrollment_conflict(
            Some(enrollment.policy_revision),
            "continue-chain binding expired",
        ));
    }
    let Some(fact) = enrollment
        .attention_facts
        .iter()
        .find(|fact| fact.key == request.expected_stop_key)
        .cloned()
    else {
        return Err(ApiError::runtime(
            "automation_continue_chain_stop_key_not_found",
            "expected_stop_key does not match an open attention fact",
            json!({ "stop_key": request.expected_stop_key }),
        ));
    };
    if fact.kind != AutomationAttentionKind::AutomationFailed
        || !matches!(
            fact.stage,
            AutomationAttentionStage::Advance | AutomationAttentionStage::CodingStart
        )
        || fact.plan_id.as_deref() != Some(binding.plan_id.as_str())
    {
        return Err(ApiError::runtime(
            "automation_continue_chain_stop_key_not_replayable",
            "stop key is not an advance/coding_start drive failure for the bound plan",
            json!({ "stop_key": request.expected_stop_key, "stage": fact.stage }),
        ));
    }

    let resolve_and_wake = |store: IssueAutomationStore| {
        let outcome = store.resolve_automation_attention_fact(
            &project_id,
            &issue_id,
            &binding,
            &request.expected_stop_key,
        );
        let _ = state.autopilot_wake.send(true);
        outcome
    };

    match fact.stage {
        AutomationAttentionStage::Advance => {
            let advance_store = crate::product::advance_store::AdvanceStore::new(paths.clone());
            let Some(record) = advance_store
                .get_advance_for_plan(&project_id, &issue_id, &binding.plan_id)
                .map_err(product_store_api_error)?
            else {
                return Err(ApiError::runtime(
                    "automation_continue_chain_stop_key_not_replayable",
                    "no advance record for the bound plan",
                    json!({}),
                ));
            };
            if record.status != crate::product::advance_store::AdvanceStatus::Failed {
                // 停点事实已过期（advance 已非 Failed）：幂等解除并唤醒。
                resolve_and_wake(store).map_err(enrollment_api_error)?;
                return Ok(Json(AutomationContinueChainResponse {
                    command_id: request.command_id,
                    state: crate::product::models::automation::OperationState::Replayed,
                    stop_key: request.expected_stop_key,
                    stage: "advance".to_string(),
                    resolved: true,
                    detail: None,
                }));
            }
            let journal = advance_store
                .get_advance_initialization(&record)
                .map_err(product_store_api_error)?
                .ok_or_else(|| {
                    ApiError::runtime(
                        "automation_continue_chain_stop_key_not_replayable",
                        "failed advance has no initialization journal",
                        json!({}),
                    )
                })?;
            let retry_request = crate::product::models::automation::RetryInitializationRequest {
                command_id: request.command_id.clone(),
                expected_binding: binding.clone(),
                expected_attempt_id: journal.attempt_id.clone(),
                expected_checkpoint: journal.phase,
                confirm_unknown_side_effect: request.confirm_unknown_side_effect,
            };
            // 与 retry-initialization REST 同一引擎链（借用 R1 冻结 policy：
            // record.start_run_policy 首次冻结，不猜 Manual、不覆盖）。
            let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(paths.clone());
            let session_record = lifecycle
                .list_workspace_sessions(&project_id, &issue_id)
                .map_err(product_store_api_error)?
                .into_iter()
                .find(|record| {
                    record.workspace_type == crate::product::models::WorkspaceType::WorkItemPlan
                        && record.entity_id == binding.plan_id
                })
                .ok_or_else(|| {
                    ApiError::runtime(
                        "automation_continue_chain_session_not_found",
                        "no workspace session bound to the plan",
                        json!({}),
                    )
                })?;
            let admission = crate::product::workspace_engine::AdvancePlanAdmission::acquire(
                &paths,
                &project_id,
                &issue_id,
                &binding.plan_id,
            )
            .map_err(|error| {
                ApiError::runtime(
                    "automation_continue_chain_admission_failed",
                    format!("continue-chain admission rejected: {error:?}"),
                    json!({}),
                )
            })?;
            let retry = match state.workspace_sessions.peek(&session_record.id).await {
                Some(manager) => {
                    let engine_arc = manager.engine();
                    let mut engine = engine_arc.lock().await;
                    engine
                        .retry_initialization_with_lease(&retry_request, &admission)
                        .await
                }
                None => {
                    let full_record = lifecycle
                        .get_workspace_session(&session_record.id)
                        .map_err(product_store_api_error)?;
                    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
                    let mut engine =
                        crate::product::workspace_engine::WorkspaceEngine::new_persistent(
                            std::sync::Arc::new(
                                crate::product::checkpoint_store::CheckpointStore::new(
                                    state.workspace_root.join("advance-engine-checkpoints"),
                                ),
                            ),
                            lifecycle,
                            event_tx,
                            crate::product::workspace_engine::WorkspaceSession::from_record(
                                full_record,
                            ),
                        );
                    engine
                        .retry_initialization_with_lease(&retry_request, &admission)
                        .await
                }
            }
            .map_err(|error| {
                ApiError::runtime("automation_continue_chain_rejected", error, json!({}))
            })?;
            let succeeded = matches!(
                retry.state,
                crate::product::models::automation::OperationState::Accepted
                    | crate::product::models::automation::OperationState::Replayed
            ) && retry.outcome.as_ref().is_some_and(|outcome| {
                matches!(
                    outcome,
                    crate::product::advance_store::AdvanceOutcome::Completed { .. }
                        | crate::product::advance_store::AdvanceOutcome::Replayed { .. }
                )
            });
            if succeeded {
                resolve_and_wake(store).map_err(enrollment_api_error)?;
            }
            Ok(Json(AutomationContinueChainResponse {
                command_id: retry.command_id,
                state: retry.state,
                stop_key: request.expected_stop_key,
                stage: "advance".to_string(),
                resolved: succeeded,
                detail: if succeeded {
                    None
                } else {
                    Some("advance retry stopped at needs_human; fact kept".to_string())
                },
            }))
        }
        AutomationAttentionStage::CodingStart => {
            let Some(attempt_id) = fact.attempt_id.clone() else {
                return Err(ApiError::runtime(
                    "automation_continue_chain_stop_key_not_replayable",
                    "coding_start fact carries no attempt id",
                    json!({}),
                ));
            };
            let coding_store =
                crate::product::coding_attempt_store::CodingAttemptStore::new(paths.clone());
            let attempt = coding_store
                .get_attempt(&project_id, &issue_id, &attempt_id)
                .map_err(product_store_api_error)?;
            let Some(claim) = attempt.start_claim.clone() else {
                return Err(ApiError::runtime(
                    "automation_continue_chain_stop_key_not_replayable",
                    "attempt has no start claim; use the existing recovery paths",
                    json!({}),
                ));
            };
            if !matches!(
                claim.phase,
                crate::product::coding_models::CodingStartPhase::Claimed
                    | crate::product::coding_models::CodingStartPhase::RunnerRegistered
            ) {
                // 越屏障（ProviderMayHaveStarted/NeedsHuman）不清 claim——
                // 走既有 RecoverCoding/RestartCoding 恢复（服务端核 auto）。
                return Err(ApiError::runtime(
                    "automation_continue_chain_stop_key_not_replayable",
                    "start claim passed the replay barrier; use the existing recovery paths",
                    json!({ "claim_phase": format!("{:?}", claim.phase) }),
                ));
            }
            let outcome = crate::web::coding_start::start_coding_once(
                &state,
                &project_id,
                &issue_id,
                crate::web::coding_start::StartCodingCommand {
                    attempt_id: attempt.id.clone(),
                    command_id: claim.command_id.clone(),
                    origin: claim.origin.clone(),
                },
            )
            .await;
            match outcome {
                Ok(
                    crate::web::coding_start::StartCodingOutcome::Started { .. }
                    | crate::web::coding_start::StartCodingOutcome::AlreadyStarted { .. },
                ) => {
                    resolve_and_wake(store).map_err(enrollment_api_error)?;
                    Ok(Json(AutomationContinueChainResponse {
                        command_id: request.command_id,
                        state: crate::product::models::automation::OperationState::Accepted,
                        stop_key: request.expected_stop_key,
                        stage: "coding_start".to_string(),
                        resolved: true,
                        detail: None,
                    }))
                }
                Ok(crate::web::coding_start::StartCodingOutcome::NeedsHuman {
                    attempt_id,
                    reason,
                }) => Ok(Json(AutomationContinueChainResponse {
                    command_id: request.command_id,
                    state: crate::product::models::automation::OperationState::NeedsHuman,
                    stop_key: request.expected_stop_key,
                    stage: "coding_start".to_string(),
                    resolved: false,
                    detail: Some(format!("{attempt_id}: {reason}")),
                })),
                Err(error) => Err(ApiError::runtime(
                    "automation_continue_chain_start_failed",
                    format!("coding start replay failed: {}", error.message()),
                    json!({ "code": error.code() }),
                )),
            }
        }
        AutomationAttentionStage::Plan | AutomationAttentionStage::Termination => {
            // 上方 stage 过滤已排除； unreachable 分支保持穷尽。
            unreachable!("stage filtered above")
        }
    }
}
