/// B2a（Task 5.1）/B2b3（Task 5.4）/B2c（Task 5.5）/B2d（Task 5.6）/B1
/// （终审 P1，author 族）fresh dispatch 的 tag 覆盖面——A/B 发布纪律
/// （设计 §4.5/§10.1）：fresh 启动能力按子包逐 tag 放开，未接管 tag
/// typed 拒零 spawn。已覆盖：
/// `legacy_outline_rebuild`（5.1，dispatch 时新解析 resolved planning
/// context 组装）；`legacy_draft`/`legacy_batch`/`legacy_plan_revision`
/// （5.4，kind 可直接组装，Draft/Batch 的业务代事实 spawn 前按 durable
/// active index typed 校验，PlanRevision 的 retained/redo 对照由臂内重算
/// 消费 engine artifact）；`sc_author`（5.5，payload phase/reservation
/// key/source ref spawn 前对 durable typed 对照）；`sc_manual_revision`
/// （5.6，payload turn 事实 spawn 前对 durable turn/ledger typed 对照）；
/// author 族（`author`/`author_choice_followup`/`author_feedback`/
/// `reviewer_revision`，终审 P1：业务四元组身份/artifact/feedback spawn 前
/// 对 durable typed 对照，臂内 author_root_launch/review drive 消费点以
/// FreshRunning 重入——resume=None 全新 native + full-fresh prompt 自足）。
/// 其余 tag typed 拒（fresh 链恒 `resume_provider_session_id=None`，旧
/// audit 不回捞）。
async fn lc_fresh_dispatch_kind(
    recovery: &crate::product::models::LcResumeRecoveryRecord,
    run_context: &ProviderRunContext,
) -> Result<ProviderRunKind, LcFreshDispatchRejection> {
    match lc_fresh_run_kind_for_recovery_context(&recovery.run_context)? {
        LcFreshRunKind::NeedsFreshPlanningContext => {
            let decision =
                crate::web::workspace_ws_handler::planning_resume_decision_with_fresh_index(
                    &run_context.app_paths,
                    &run_context.session_record.project_id,
                    &run_context.session_record.issue_id,
                )
                .await
                .map_err(|error| {
                    lc_context_unsupported(format!(
                        "legacy_outline_rebuild fresh dispatch failed to resolve planning context: {error}"
                    ))
                })?;
            let resolved = match decision {
                // 漂移仍在：携带 rebuilt（重新 build 的权威上下文）。
                Some(crate::product::logical_codebase::ResumeDecision::StaleContext {
                    rebuilt,
                    ..
                }) => rebuilt,
                // 快照已对齐（如等待期外力修复）：现解析结果即新 snapshot。
                Some(crate::product::logical_codebase::ResumeDecision::SameContext(resolved)) => {
                    resolved
                }
                None => {
                    return Err(lc_context_unsupported(
                        "legacy_outline_rebuild fresh dispatch requires logical planning routing"
                            .to_string(),
                    ));
                }
            };
            Ok(ProviderRunKind::WorkItemPlanOutlineRebuild {
                rebuilt: Box::new(resolved),
            })
        }
        // B2b3（Task 5.4，设计 §5.3 Draft/Batch/PlanRevision 行）：三 tag 的
        // full fresh 臂接管。Draft/Batch 业务代（generation/outline/batch）
        // 在 spawn 前按 durable active index typed 校验——漂移即
        // `lc_resume_recovery_stale` 拒收回等待，不用旧权限强行完成；
        // PlanRevision 的 retained/redo 重算对照在臂内（需 engine artifact）。
        kind @ LcFreshRunKind::Kind(
            ProviderRunKind::WorkItemPlanDraft { .. }
            | ProviderRunKind::WorkItemPlanBatch
            | ProviderRunKind::WorkItemPlanRevision { .. },
        ) => {
            verify_lc_fresh_business_generation(&recovery.run_context, run_context)?;
            match kind {
                LcFreshRunKind::Kind(kind) => Ok(kind),
                // 上面 match 收窄到 Kind；NeedsFreshPlanningContext 分支已消费。
                LcFreshRunKind::NeedsFreshPlanningContext => {
                    unreachable!("NeedsFreshPlanningContext arm is matched before the Kind arm")
                }
            }
        }
        // B2c（Task 5.5，设计 §5.3 sc_author 行）：SC author tag 的 full
        // fresh 臂接管——payload 的 phase/reservation key/source ref 与
        // durable typed 对照（漂移即 `lc_resume_recovery_stale` 拒收回
        // 等待，不用旧权限强行完成），对照通过即放行 kind；臂内
        // （provider_run SC 臂）以 LcFreshContext 重入：同 logical
        // reservation 不 reserve 第二次、resume=None 全新 native、verdict
        // 在场走 fresh 返修 prompt（上一版 source 全文自足）。
        kind @ LcFreshRunKind::Kind(ProviderRunKind::WorkItemPlanSingleCandidateAuthor) => {
            verify_lc_fresh_sc_author_payload(&recovery.run_context, run_context)?;
            match kind {
                LcFreshRunKind::Kind(kind) => Ok(kind),
                // 上面 match 收窄到 Kind；NeedsFreshPlanningContext 分支已消费。
                LcFreshRunKind::NeedsFreshPlanningContext => {
                    unreachable!("NeedsFreshPlanningContext arm is matched before the Kind arm")
                }
            }
        }
        // B2d（Task 5.6，设计 §5.3 sc_manual_revision 行）：SC manual tag 的
        // full fresh 臂接管——payload 冻结的 turn 事实（command/source_hash/
        // attempt/budget/状态非终态/reservation key/source ref）与 durable
        // typed 对照（漂移即 `lc_resume_recovery_stale` 拒收回等待，不用旧
        // turn 权限强行完成），对照通过即放行 kind；臂内（provider_run
        // manual 臂）同 turn 重入：同 turn prompt 重装（candidate hash 复
        // 验）、resume=None 全新 native、run_sc_manual_revision_turn 同
        // turn 完成回原 human gate（不新 turn/attempt/预算、不通用
        // StartGeneration rearm）。
        kind @ LcFreshRunKind::Kind(ProviderRunKind::HumanGateScManualRevision { .. }) => {
            verify_lc_fresh_sc_manual_revision_payload(&recovery.run_context, run_context)?;
            match kind {
                LcFreshRunKind::Kind(kind) => Ok(kind),
                // 上面 match 收窄到 Kind；NeedsFreshPlanningContext 分支已消费。
                LcFreshRunKind::NeedsFreshPlanningContext => {
                    unreachable!("NeedsFreshPlanningContext arm is matched before the Kind arm")
                }
            }
        }
        // B1（终审 P1，B1→B2 接缝，设计 §5.3 author 族行）：author 族四 tag
        //（author/author_choice_followup/author_feedback/reviewer_revision，
        // 生产唯一可达等待出生=review drive 的 ReviewerRevision mismatch
        // persist）的 full fresh dispatch 臂接管——payload 冻结的业务四元组
        //（entity/workspace 身份、author/reviewer provider、artifact 引用、
        // feedback）spawn 前与 durable typed 对照（等待期任一维度被推进即
        // 漂移，`lc_resume_recovery_stale` 拒收回等待，不用旧权限强行完
        // 成），对照通过即放行 kind；臂内（provider_run Author/ChoiceFollowup
        // 臂 + review drive revision 消费点）按 FreshRunning 重入：resume=None
        // 全新 native、full-fresh prompt 自足（B1a
        // `build_revision_input_with_resume(false)` 合同：artifact/feedback/
        // review 全量内嵌）。4.2 的 OperationResult/claim 链零改动。
        kind @ LcFreshRunKind::Kind(
            ProviderRunKind::Author { .. }
            | ProviderRunKind::AuthorChoiceFollowup { .. }
            | ProviderRunKind::Revision,
        ) => {
            verify_lc_fresh_author_family_payload(&recovery.run_context, run_context)?;
            match kind {
                LcFreshRunKind::Kind(kind) => Ok(kind),
                // 上面 match 收窄到 Kind；NeedsFreshPlanningContext 分支已消费。
                LcFreshRunKind::NeedsFreshPlanningContext => {
                    unreachable!("NeedsFreshPlanningContext arm is matched before the Kind arm")
                }
            }
        }
        // full fresh 臂接管——dispatch 直接放行既有 kind；臂内
        // （work_item_plan_legacy_author.inc.rs）以
        // build_outline_fresh_revision_invocation 构建 prompt 自足的 full
        // fresh 返修（旧 outline 全文/feedback/review/完整材料）并全新启动
        // （resume=None、全新节点、原 outline 确认门）；正常 resume 的增量
        // 返修/首发出生不受影响。legacy_outline_rebuild 仍走上方
        // NeedsFreshPlanningContext 臂（dispatch 时新解析 planning context）。
        kind @ LcFreshRunKind::Kind(
            ProviderRunKind::WorkItemPlanLegacyAuthor
            | ProviderRunKind::WorkItemPlanOutlineRevision { .. },
        ) => match kind {
            LcFreshRunKind::Kind(kind) => Ok(kind),
            // 上面 match 收窄到 Kind；NeedsFreshPlanningContext 分支已消费。
            LcFreshRunKind::NeedsFreshPlanningContext => {
                unreachable!("NeedsFreshPlanningContext arm is matched before the Kind arm")
            }
        },
        LcFreshRunKind::Kind(kind) => Err(lc_context_unsupported(format!(
            "run_context tag {} (resolved kind {kind:?}) fresh handler lands with the remaining B2 packages; explicit fresh stays waiting",
            lc_recovery_context_tag_label(&recovery.run_context)
        ))),
    }
}

/// B2b3（Task 5.4，设计 §5.3）：Draft/Batch tag 的业务代 typed 校验——
/// payload 携带的 generation/outline/batch 事实与 durable active index
/// 不一致时以 `lc_resume_recovery_stale` 拒收（等待新漂移裁决）；index
/// 读取失败按 store unavailable 家族 typed 上浮。LegacyPlanRevision 不在
/// 此校验（retained/redo 由臂内重算对照）。
fn verify_lc_fresh_business_generation(
    context: &crate::product::models::LcRecoveryRunContext,
    run_context: &ProviderRunContext,
) -> Result<(), LcFreshDispatchRejection> {
    use crate::product::models::LcRecoveryRunContext;
    use crate::product::work_item_plan_store::WorkItemPlanStore;

    let (generation_round_id, active_outline_id, batch_id) = match context {
        LcRecoveryRunContext::LegacyDraft {
            generation_round_id,
            active_outline_id,
            ..
        } => (
            generation_round_id.as_str(),
            Some(active_outline_id.as_str()),
            None,
        ),
        LcRecoveryRunContext::LegacyBatch {
            generation_round_id,
            batch_id,
            ..
        } => (generation_round_id.as_str(), None, Some(batch_id.as_str())),
        _ => return Ok(()),
    };
    let stale = |message: String| LcFreshDispatchRejection {
        code: crate::product::lifecycle_store::LcFreshClaimOutcome::STALE_SUPERSEDED,
        message,
    };
    let index = WorkItemPlanStore::new(run_context.app_paths.clone())
        .load_active_index(
            &run_context.session_record.project_id,
            &run_context.session_record.issue_id,
            &run_context.session_record.entity_id,
        )
        .map_err(|error| LcFreshDispatchRejection {
            code: crate::web::workspace_ws_handler::socket::LC_RESUME_STORE_UNAVAILABLE_CODE,
            message: format!("load work item plan active index failed: {error}"),
        })?
        .ok_or_else(|| {
            stale(
                "fresh business generation verify failed: work item plan active index missing"
                    .to_string(),
            )
        })?;
    if index.current_generation_round_id != generation_round_id {
        return Err(stale(format!(
            "fresh business generation verify failed: payload round {generation_round_id} != durable round {}",
            index.current_generation_round_id
        )));
    }
    if let Some(expected_outline) = active_outline_id
        && index.active_outline_id.as_deref() != Some(expected_outline)
    {
        return Err(stale(format!(
            "fresh business generation verify failed: payload outline {expected_outline} != durable outline {:?}",
            index.active_outline_id
        )));
    }
    if let Some(expected_batch) = batch_id
        && !index.batches.iter().any(|batch| {
            batch.batch_id == expected_batch
                && batch.generation_round_id == index.current_generation_round_id
        })
    {
        return Err(stale(format!(
            "fresh business generation verify failed: payload batch {expected_batch} not found in durable generation {}",
            index.current_generation_round_id
        )));
    }
    Ok(())
}

/// B2c（Task 5.5，设计 §5.3 sc_author 行）：sc_author payload 与 durable 的
/// typed 对照——payload phase/reservation key/source ref 必须与当前 durable
/// 事实一致（等待期间 phase/ledger/source 被推进即漂移，`lc_resume_recovery_stale`
/// 拒收回等待，不用旧权限强行完成）；record 读取失败按 store unavailable
/// 家族 typed 上浮。同 logical reservation 重入的资格由此锚定：payload key
/// 即 durable ledger 既有键（fresh 不 reserve 第二次）。
fn verify_lc_fresh_sc_author_payload(
    context: &crate::product::models::LcRecoveryRunContext,
    run_context: &ProviderRunContext,
) -> Result<(), LcFreshDispatchRejection> {
    let crate::product::models::LcRecoveryRunContext::ScAuthor {
        single_candidate_phase,
        provider_start_reservation_key,
        source_revision_ref,
        ..
    } = context
    else {
        return Ok(());
    };
    let stale = |message: String| LcFreshDispatchRejection {
        code: crate::product::lifecycle_store::LcFreshClaimOutcome::STALE_SUPERSEDED,
        message,
    };
    let record =
        crate::product::lifecycle_store::LifecycleStore::new(run_context.app_paths.clone())
            .get_workspace_session(&run_context.session_id)
            .map_err(|error| LcFreshDispatchRejection {
                code: crate::web::workspace_ws_handler::socket::LC_RESUME_STORE_UNAVAILABLE_CODE,
                message: format!(
                    "load workspace session for sc author fresh verify failed: {error}"
                ),
            })?;
    if record.single_candidate_phase.as_ref() != Some(single_candidate_phase) {
        return Err(stale(format!(
            "sc author fresh verify failed: payload phase {single_candidate_phase:?} != durable phase {:?}",
            record.single_candidate_phase
        )));
    }
    if let Some(expected_key) = provider_start_reservation_key.as_deref()
        && !record
            .provider_start_ledger
            .iter()
            .any(|entry| entry.provider_start_idempotency_key == expected_key)
    {
        return Err(stale(format!(
            "sc author fresh verify failed: payload reservation key {expected_key} absent from durable provider start ledger"
        )));
    }
    if source_revision_ref != &record.work_item_plan_source_revision_ref {
        return Err(stale(format!(
            "sc author fresh verify failed: payload source ref {source_revision_ref:?} != durable source ref {:?}",
            record.work_item_plan_source_revision_ref
        )));
    }
    Ok(())
}

/// B2d（Task 5.6，设计 §5.3 sc_manual_revision 行）：sc_manual payload 与
/// durable 的 typed 对照——payload 冻结的 turn 事实（command_id/source_
/// hash/attempt_no/budget_reserved、turn 仍非终态、reservation key 在
/// durable ledger、source ref 与 record 一致）必须与当前 durable 事实一致
///（等待期间 turn 被推进/换代/终态化即漂移，`lc_resume_recovery_stale`
/// 拒收回等待，不用旧 turn 权限强行完成）；store 读取失败按 store
/// unavailable 家族 typed 上浮。同 turn 重入的资格由此锚定：payload 的
/// reservation key 即 durable ledger 既有键（fresh 不 reserve 第二次）。
fn verify_lc_fresh_sc_manual_revision_payload(
    context: &crate::product::models::LcRecoveryRunContext,
    run_context: &ProviderRunContext,
) -> Result<(), LcFreshDispatchRejection> {
    let crate::product::models::LcRecoveryRunContext::ScManualRevision {
        turn_id,
        command_id,
        source_hash,
        attempt_no,
        budget_reserved,
        human_gate_reservation_key,
        source_revision_ref,
        ..
    } = context
    else {
        return Ok(());
    };
    let stale = |message: String| LcFreshDispatchRejection {
        code: crate::product::lifecycle_store::LcFreshClaimOutcome::STALE_SUPERSEDED,
        message,
    };
    let store = crate::product::lifecycle_store::LifecycleStore::new(run_context.app_paths.clone());
    let record = store
        .get_workspace_session(&run_context.session_id)
        .map_err(|error| LcFreshDispatchRejection {
            code: crate::web::workspace_ws_handler::socket::LC_RESUME_STORE_UNAVAILABLE_CODE,
            message: format!("load workspace session for sc manual fresh verify failed: {error}"),
        })?;
    let turn = store
        .get_human_gate_turn(&run_context.session_id, turn_id)
        .map_err(|error| LcFreshDispatchRejection {
            code: crate::web::workspace_ws_handler::socket::LC_RESUME_STORE_UNAVAILABLE_CODE,
            message: format!("load human gate turn for sc manual fresh verify failed: {error}"),
        })?;
    if !matches!(
        turn.status,
        crate::product::models::HumanGateTurnStatus::Reserved
            | crate::product::models::HumanGateTurnStatus::Running
    ) {
        return Err(stale(format!(
            "sc manual fresh verify failed: turn {} is terminal ({:?})",
            turn.turn_id, turn.status
        )));
    }
    if turn.command_id != *command_id
        || turn.source_hash != *source_hash
        || turn.attempt_no != *attempt_no
        || turn.budget_reserved != *budget_reserved
    {
        return Err(stale(format!(
            "sc manual fresh verify failed: payload turn facts (command {command_id}, \
             source_hash {source_hash}, attempt {attempt_no}, budget {budget_reserved}) \
             != durable turn (command {}, source_hash {}, attempt {}, budget {})",
            turn.command_id, turn.source_hash, turn.attempt_no, turn.budget_reserved
        )));
    }
    if let Some(expected_key) = human_gate_reservation_key.as_deref()
        && !record
            .provider_start_ledger
            .iter()
            .any(|entry| entry.provider_start_idempotency_key == expected_key)
    {
        return Err(stale(format!(
            "sc manual fresh verify failed: payload reservation key {expected_key} absent from durable provider start ledger"
        )));
    }
    if source_revision_ref != &record.work_item_plan_source_revision_ref {
        return Err(stale(format!(
            "sc manual fresh verify failed: payload source ref {source_revision_ref:?} != durable source ref {:?}",
            record.work_item_plan_source_revision_ref
        )));
    }
    // SG6b：结构授权窗口 join（recovery×turn×grant×attempt×prompt）。
    verify_sc_manual_fresh_structural_grant_join(&record, context)
}

/// SG6b（plan v1.1 Task SG6b / REQ-SG-05「显式 fresh 同 attempt
/// None-resume/子 prompt」）：sc_manual_revision 显式 fresh dispatch 的
/// 结构授权窗口 join（recovery×turn×grant×attempt×prompt 的 durable 侧
/// 判据，纯函数零 I/O）——活动 grant 绑定本 turn 且 phase=Attempt2Claimed
/// 时，fresh 重入驱动的就是 claim 绑定的冻结机器指令子 prompt：grant 必
/// 须携带完整 prompt 身份且恰消费下一 attempt（grant.attempt_no ==
/// payload attempt_no + 1）；漂移（错 recovery/身份缺失/attempt 越界/
/// 窗口已 closed）一律 `lc_resume_recovery_stale` 拒收回等待（零
/// start、预算不改写）。无结构窗口（无 grant）保持原 fresh 路径不变。
fn verify_sc_manual_fresh_structural_grant_join(
    record: &crate::product::models::WorkspaceSessionRecord,
    context: &crate::product::models::LcRecoveryRunContext,
) -> Result<(), LcFreshDispatchRejection> {
    let crate::product::models::LcRecoveryRunContext::ScManualRevision {
        turn_id,
        command_id,
        attempt_no,
        ..
    } = context
    else {
        return Ok(());
    };
    verify_sc_manual_fresh_grant_join_core(
        record
            .structural_governance_state
            .as_ref()
            .and_then(|state| state.current_grant.as_ref()),
        turn_id,
        command_id,
        *attempt_no,
    )
}

fn verify_sc_manual_fresh_grant_join_core(
    grant: Option<&crate::product::models::workspace::StructuralRevisionGrant>,
    turn_id: &str,
    command_id: &str,
    attempt_no: u32,
) -> Result<(), LcFreshDispatchRejection> {
    use crate::product::models::workspace::StructuralGrantPhase;
    let stale = |message: String| LcFreshDispatchRejection {
        code: crate::product::lifecycle_store::LcFreshClaimOutcome::STALE_SUPERSEDED,
        message,
    };
    let Some(grant) = grant else {
        // 无结构授权窗口：原 fresh 路径不变。
        return Ok(());
    };
    match grant.turn_id.as_deref() {
        Some(grant_turn_id) if grant_turn_id == turn_id => {}
        Some(grant_turn_id) => {
            return Err(stale(format!(
                "sc manual fresh structural join failed: grant is bound to turn {grant_turn_id}, not {turn_id}"
            )));
        }
        None => {
            return Err(stale(
                "sc manual fresh structural join failed: grant has no bound turn".to_string(),
            ));
        }
    }
    if let Some(grant_command_id) = grant.command_id.as_deref()
        && grant_command_id != command_id
    {
        return Err(stale(format!(
            "sc manual fresh structural join failed: grant is bound to command {grant_command_id}, not {command_id}"
        )));
    }
    match grant.phase {
        Some(StructuralGrantPhase::Attempt2Claimed) => {
            let (prompt_ref, prompt_digest) = (
                grant.prompt_ref.as_deref().unwrap_or_default(),
                grant.prompt_digest.as_deref().unwrap_or_default(),
            );
            if prompt_ref.trim().is_empty() || prompt_digest.trim().is_empty() {
                return Err(stale(format!(
                    "sc manual fresh structural join failed: claimed grant carries an incomplete frozen prompt identity (ref {prompt_ref:?}, digest present: {})",
                    !prompt_digest.trim().is_empty()
                )));
            }
            // 恰消费下一 attempt：子 prompt 绑定 payload attempt + 1。
            if grant.attempt_no != attempt_no.saturating_add(1) {
                return Err(stale(format!(
                    "sc manual fresh structural join failed: claimed grant attempt {} does not continue payload attempt {attempt_no}",
                    grant.attempt_no
                )));
            }
            Ok(())
        }
        Some(
            StructuralGrantPhase::Reviewing
            | StructuralGrantPhase::Resolved
            | StructuralGrantPhase::SystemFault
            | StructuralGrantPhase::BusinessGate,
        ) => Err(stale(format!(
            "sc manual fresh structural join failed: structural window is closed for delivery (phase {:?}); explicit fresh stays waiting",
            grant.phase
        ))),
        // 未 claim 窗口（AuthorRunning/Assessing/InstructionFrozen/
        // BlockedByLcRecovery 且无 claim 身份）：fresh 重入＝原反馈 prompt
        // 的同 turn 重装，放行；携带 claim 身份的非 Attempt2Claimed 相位
        // 是不一致 durable，拒收。
        _ => {
            if grant
                .prompt_ref
                .as_deref()
                .is_some_and(|v| !v.trim().is_empty())
            {
                return Err(stale(format!(
                    "sc manual fresh structural join failed: grant phase {:?} carries a claimed prompt identity; refusing inconsistent structural state",
                    grant.phase
                )));
            }
            Ok(())
        }
    }
}
