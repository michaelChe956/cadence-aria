//! P08 replay 安全的 SC 轮入场编排（自 `run/sc_failure_transport.rs` 物理拆出，
//! 以守 1200 行硬线；纯搬移 + 分类接线，无行为分叉）。
//!
//! 三个入口都吃 `translate` 出来的 typed 分类 `ScRoundEntry`：只有 `Ready` 允许
//! 真实 `start_work_item_plan_author`；`Completed` 只复用严格读回的 durable raw；
//! `Waiting` 只走既有 `LcResumeRecoveryState::Waiting` CAS（不落 Failed/A4、不新
//! seq、不第二 start）。本模块不新增 wire 字段、不新增恢复状态机、不改教学窗口
//! 与 A/B 预算。

use super::sc_failure_transport::{
    ScAuthorRoundDelivery, ScBoundProviderRound, ScRoundEntry, begin_sc_failure_round,
    close_sc_split_round, drive_single_candidate_reredrive, replayed_sc_round_delivery,
};
use super::single_candidate_compile_preflight::{
    emit_author_ears_normalized_event, emit_author_heading_normalized_event,
    prepare_author_delivery_for_compile,
};
use super::*;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceError, ScFailureEvidenceEvent,
};
use crate::product::models::{
    LcFreshOperationState, LcRecoveryRunContext, LcResumeRecoveryState, ProviderName,
};
use crate::web::workspace_ws_handler::run::gateway_start::PlanAuthorLaunch;
use crate::web::workspace_ws_types::{ScFailureChainIdentity, ScFailureScope};

/// 显式 fresh operation 的物理身份锚（P08 Case 5/6）。
///
/// 事实只取 **durable** 记录：当前 operation item（`fresh_operations` 中与 root
/// `operation_id` 对齐者，退化到最后一个受理 item）的 `role_run_seq` /
/// `fresh_node_id`，以及原 recovery `run_context::ScAuthor.business.original_node_id`
/// 给出的旧链锚。不读 `PlanSplitRunContext` 私有快照、不改既有 authorize 策略。
// 一次决策返回一次，非热路径；boxing Fresh/Bound 会波及所有 match 点且不改变
// 行为，故保留平铺语义。
#[allow(clippy::large_enum_variant)]
pub(in crate::web::workspace_ws_handler::run) enum FreshRoundAnchor {
    /// 当前 operation 已绑定真实物理轮（同 operation crash 重入）：直接返回该轮
    /// 的真实分类（Ready/Completed/Waiting），**绝不**新造 node/seq。
    Bound(ScRoundEntry),
    /// 当前 operation 尚未进入可运行态（FreshReserved / 预约未完成 / 状态不一致）：
    /// 真实 CAS 等待，**不得**回落普通 spawn 路径、不得落 Failed。
    NotLaunched {
        reason_code: &'static str,
        detail: String,
    },
    /// 当前 operation 尚未绑定：新 AuthorRun node + 链内下一个 ordinal。
    /// `original` 为 `None` 表示旧链**尚无任何证据**（首次尝试即被 supersede、
    /// provider 从未物理启动）——这是链的首轮，按 ordinal 0 新建链身份。
    Fresh {
        original: Option<ScFailureChainIdentity>,
        ordinal: u32,
    },
}

fn anchor_error(code: &str, detail: impl Into<String>) -> ScFailureEvidenceError {
    ScFailureEvidenceError {
        code: code.to_string(),
        step: "fresh_operation_sc_round_anchor".to_string(),
        round_id: None,
        detail: detail.into(),
        publication_incomplete: false,
        verified_prefix: None,
    }
}

/// 显式 fresh operation → 物理轮身份决策（纯读、零启动、零写入）。
///
/// `None` = 当前不是显式 fresh operation（保持既有 active 复用/新建路径）。
/// 旧链锚与最大 ordinal 全部来自既有证据日志：先按
/// `(scope, key, 原节点, 原节点, 0)` 取首轮（唯一命中即链身份，
/// `original_role_run_seq` = 该轮 seq），再严格读全链 `RoundBound`——当前 item 的
/// `role_run_seq` 命中即为 `Bound`，否则 `Fresh { original, max+1 }`。
pub(in crate::web::workspace_ws_handler::run) fn fresh_operation_sc_round_anchor(
    engine: &WorkspaceEngine,
    lifecycle: &LifecycleStore,
    scope: &ScFailureScope,
    business_key: &str,
    provider: &ProviderName,
) -> Result<FreshRoundAnchor, ScFailureEvidenceError> {
    const STEP: &str = "fresh_operation_sc_round_anchor";
    // 事实只取 **durable** 记录：引擎内存镜像由 run 入口快照构建，可能仍停在
    // `FreshReserved`（dispatch 的 CAS 已提交 FreshRunning，但镜像未同步），
    // 据此判定会把已提交的可运行态读回成「未运行」并错误拒启。
    let session_id = engine.session().session_id.clone();
    let durable = lifecycle
        .get_workspace_session(&session_id)
        .map_err(|error| {
            anchor_error(
                "sc_failure_fresh_anchor_unavailable",
                format!("读取显式 fresh 恢复记录失败（零启动）：{error}"),
            )
        })?;
    let Some(recovery) = durable.lc_resume_recovery else {
        return Ok(FreshRoundAnchor::NotLaunched {
            reason_code: "sc_failure_fresh_operation_not_running",
            detail: "显式 fresh 入口缺少 recovery 记录：不启动 provider（等 gateway 预约/prepare）"
                .to_string(),
        });
    };
    // 显式 fresh 只有**真实 FreshRunning** 才可能携带可 spawn 的物理身份：
    // FreshReserved（claim 已受理、prepare 未完成）不是 spawn 授权，Waiting/
    // Resolved 保持既有路径。预约/受理步骤仍由 gateway 完成，本锚只读。
    if recovery.state != LcResumeRecoveryState::FreshRunning {
        return Ok(FreshRoundAnchor::NotLaunched {
            reason_code: "sc_failure_fresh_operation_not_running",
            detail: format!(
                "显式 fresh 恢复状态为 {:?}（非 FreshRunning）：不启动 provider，等 gateway 完成预约/prepare",
                recovery.state
            ),
        });
    }
    let LcRecoveryRunContext::ScAuthor { business, .. } = &recovery.run_context else {
        return Ok(FreshRoundAnchor::NotLaunched {
            reason_code: "sc_failure_fresh_anchor_unavailable",
            detail: "显式 fresh 恢复的 run_context 不是 SC author：不启动 provider".to_string(),
        });
    };
    // 旧 original-node 锚：run_context business 与 failed_provider_node_id 必须一致。
    let business_original = business
        .original_node_id
        .as_deref()
        .unwrap_or_default()
        .trim();
    let failed_node = recovery.failed_provider_node_id.trim();
    if !business_original.is_empty() && !failed_node.is_empty() && business_original != failed_node
    {
        return Err(anchor_error(
            "sc_failure_fresh_anchor_unavailable",
            format!(
                "恢复记录的原节点（{business_original}）与失败节点（{failed_node}）不一致（零启动）"
            ),
        ));
    }
    let old_original_node = if !business_original.is_empty() {
        business_original
    } else if !failed_node.is_empty() {
        failed_node
    } else {
        return Err(anchor_error(
            "sc_failure_fresh_anchor_unavailable",
            "显式 fresh 恢复缺少原节点锚：不把新 node 当 original（零启动）",
        ));
    };
    let found = lifecycle
        .find_sc_failure_round(scope, business_key, old_original_node, old_original_node, 0)
        .map_err(|mut failure| {
            failure.step = STEP.to_string();
            failure
        })?;
    // 旧链尚无任何 RoundBound：这是**链的首轮**，不是「无法归属」。以 ordinal 0
    // 新建链身份启动；绝不为从未物理启动的节点伪造 RoundBound，也不从「最后
    // 一个」猜身份。
    let (original, base_ordinal, rounds): (
        Option<ScFailureChainIdentity>,
        u32,
        Vec<ScRoundIdentity>,
    ) = match found {
        Some((original, first_round)) => {
            let read = lifecycle
                .read_sc_failure_evidence(&original)
                .map_err(|mut failure| {
                    failure.step = STEP.to_string();
                    failure
                })?;
            let rounds: Vec<ScRoundIdentity> = read
                .entries
                .iter()
                .filter_map(|entry| match &entry.event {
                    ScFailureEvidenceEvent::RoundBound { round } => Some(round.clone()),
                    _ => None,
                })
                .collect();
            (Some(original), first_round.round_ordinal, rounds)
        }
        None => (None, 0, Vec::new()),
    };
    // 当前 operation 必须**显式**且精确匹配：不从“最后一个”猜 latest。缺
    // `operation_id`、或该 id 不在 `fresh_operations` 内、或其状态不是 Running
    // （既有准入验证未通过）⇒ 可见错误、零启动，绝不挑最近 operation 顶替。
    let operation_id = recovery
        .operation_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            anchor_error(
                "sc_failure_fresh_anchor_unavailable",
                "显式 fresh 恢复缺少当前 operation_id：不对账、不挑 latest（零启动）",
            )
        })?;
    let item = recovery
        .fresh_operations
        .iter()
        .find(|item| item.operation_id == operation_id)
        .ok_or_else(|| {
            anchor_error(
                "sc_failure_fresh_anchor_unavailable",
                format!("当前 operation {operation_id} 不在 fresh_operations 中（零启动）"),
            )
        })?;
    if item.state != LcFreshOperationState::Running {
        return Err(anchor_error(
            "sc_failure_fresh_anchor_unavailable",
            format!(
                "当前 operation {operation_id} 的状态为 {:?}（非 Running）：不得据此启动 provider（零启动）",
                item.state
            ),
        ));
    }
    if let Some(seq) = item.role_run_seq {
        // 已绑定（同 operation crash 重入）：只分类既有轮，不再造 node/seq。
        let bound_round = rounds
            .iter()
            .find(|round| {
                round.role_run_seq == seq
                    && item
                        .fresh_node_id
                        .as_deref()
                        .is_none_or(|node| round.node_id == node)
            })
            .ok_or_else(|| {
                anchor_error(
                    "sc_failure_fresh_anchor_unavailable",
                    format!("当前 operation 绑定 seq {seq} 但旧链内无对应 RoundBound：零启动"),
                )
            })?;
        // 必须带**原链身份**（`original.as_ref()`）：传 None 会让 begin 以
        // (scope, key, bound_node, bound_node, ordinal) 查询，跨原节点必然落空
        // ⇒ 建新 chain/新 seq（= 对同一 operation 的第二物理启动）。
        let entry = begin_sc_failure_round(
            lifecycle,
            scope,
            business_key,
            original.as_ref(),
            &bound_round.node_id,
            provider,
            bound_round.round_ordinal,
        )
        .map_err(|mut failure| {
            failure.step = STEP.to_string();
            failure
        })?;
        return Ok(FreshRoundAnchor::Bound(entry));
    }
    // 未绑定：新物理轮序号 = 链内真实最大 ordinal + 1（checked）。
    let max_ordinal = rounds
        .iter()
        .map(|round| round.round_ordinal)
        .max()
        .unwrap_or(base_ordinal);
    let ordinal = max_ordinal.checked_add(1).ok_or_else(|| {
        anchor_error(
            "sc_failure_round_ordinal_overflow",
            "旧链轮序号无法推进（零启动）",
        )
    })?;
    Ok(FreshRoundAnchor::Fresh { original, ordinal })
}

/// 原轮入场的唯一入口：node 选择 + 真实绑定 + 启动分类。
///
/// - 显式 fresh operation：`Bound` ⇒ 直接返回既有轮分类（不新 node/seq）；
///   `Fresh` ⇒ **新建** AuthorRun node（绝不复用 active node，否则新 fresh 又会
///   落回旧 node/seq），并以锚定 `max+1` ordinal 绑定到旧链；
/// - 其余：保持既有「active AuthorRun 复用，否则新建」+ ordinal 0。
///
/// 任一 Err 都是链/存储级失败：持久化 Failed 后以 Message 上浮（既有收口同形、
/// 零启动）。`Waiting`/`Completed` 是 Ok 分类，不在此落 Failed。
#[allow(clippy::result_large_err)] // 运行收尾的唯一 typed 出口（同既有先例）。
pub(in crate::web::workspace_ws_handler::run) async fn begin_sc_author_round_entry(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    scope: &ScFailureScope,
    business_key: &str,
    provider: &ProviderName,
    is_fresh_context: bool,
) -> Result<ScRoundEntry, SingleCandidateProviderRunError> {
    let outcome = if is_fresh_context {
        match fresh_operation_sc_round_anchor(engine, lifecycle, scope, business_key, provider)
            .map_err(|failure| {
                // 定点转换：保留 stable code/step 与原 detail（不加全局 From、不加 variant）。
                SingleCandidateProviderRunError::Message(format!(
                    "SC 轮入场锚不可用（code={} step={}）：{}",
                    failure.code, failure.step, failure.detail
                ))
            })? {
            FreshRoundAnchor::Bound(entry) => Ok(entry),
            FreshRoundAnchor::Fresh { original, ordinal } => {
                let node_id = engine.begin_work_item_plan_author_run().await;
                begin_sc_failure_round(
                    lifecycle,
                    scope,
                    business_key,
                    original.as_ref(),
                    &node_id,
                    provider,
                    ordinal,
                )
            }
            FreshRoundAnchor::NotLaunched {
                reason_code,
                detail,
            } => {
                // 显式 fresh 但当前 operation 尚未进入可运行态（FreshReserved /
                // 预约未完成）：**不得**回落普通 spawn 路径，也不落 Failed——
                // 真实 CAS 等待，等 gateway 完成预约/prepare。
                return Err(wait_sc_author_launch(engine, reason_code, &detail).await);
            }
        }
    } else {
        let node_id = if engine.active_node_type()
            == Some(crate::web::workspace_ws_types::TimelineNodeType::AuthorRun)
        {
            engine.active_timeline_node_id().ok_or_else(|| {
                SingleCandidateProviderRunError::Message(
                    "single candidate author run node unavailable".to_string(),
                )
            })?
        } else {
            engine.begin_work_item_plan_author_run().await
        };
        begin_sc_failure_round(lifecycle, scope, business_key, None, &node_id, provider, 0)
    };
    outcome.map_err(|failure| {
        engine.persist_single_candidate_terminal_phase(
            crate::product::models::SingleCandidatePhase::Failed,
        );
        SingleCandidateProviderRunError::Message(format!("{failure}"))
    })
}

/// `Ready` 分支的物理运行：真实 start → 驱动到输出 → 严格收口（complete → 读回
/// raw → 逐字比较 → finalize 纪元）。失败语义与本臂既有出口完全一致（驱动失败走
/// `fail_plan_split_run` + Failed 相位；收口失败保留严格证据错误且零交付）。
#[allow(clippy::too_many_arguments)]
#[allow(clippy::result_large_err)] // 同上：该枚举是运行收尾的唯一 typed 出口。
pub(in crate::web::workspace_ws_handler::run) async fn drive_ready_sc_round(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    launch: &PlanAuthorLaunch,
    provider_for_run: Arc<dyn StreamingProviderAdapter>,
    provider_input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
    run_cancel: &CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    node_id: &str,
    author_provider: &ProviderName,
    prompt: &str,
    bound: &ScBoundProviderRound,
) -> Result<ScAuthorRoundDelivery, SingleCandidateProviderRunError> {
    let provider_session =
        crate::web::workspace_ws_handler::run::gateway_start::start_work_item_plan_author(
            launch.clone(),
            provider_for_run,
            provider_input,
            run_cancel.clone(),
            Some(&bound.split),
        )
        .await;
    let full_output = match engine
        .drive_work_item_plan_provider_session_to_output(
            provider_session,
            command_rx,
            node_id.to_string(),
            author_provider.clone(),
        )
        .await
    {
        Ok(output) => output,
        Err(message) => {
            crate::web::workspace_ws_handler::run::fail_plan_split_run(
                Some(&bound.split),
                &format!("provider session drive failed: {message}"),
            );
            engine.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
            return Err(SingleCandidateProviderRunError::AlreadyFinished);
        }
    };
    let raw = match close_sc_split_round(lifecycle, bound, prompt, &full_output) {
        Ok(raw) => raw,
        Err(failure) => {
            engine.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
            return Err(SingleCandidateProviderRunError::Message(format!(
                "{failure}"
            )));
        }
    };
    let delivery = prepare_author_delivery_for_compile(&full_output);
    emit_author_heading_normalized_event(engine, node_id, &full_output, &delivery, author_provider)
        .await;
    emit_author_ears_normalized_event(engine, node_id, &full_output, &delivery, author_provider)
        .await;
    Ok(ScAuthorRoundDelivery {
        source: delivery.source,
        raw,
        round: bound.round.clone(),
    })
}

/// 无物理轮绑定时的等待收口（显式 fresh 尚未可运行）：同一既有 CAS，
/// 不伪造 round 身份、不落 Failed。
pub(in crate::web::workspace_ws_handler::run) async fn wait_sc_author_launch(
    engine: &mut WorkspaceEngine,
    reason_code: &'static str,
    detail: &str,
) -> SingleCandidateProviderRunError {
    let reason = format!("{reason_code}: {detail}");
    if let Some(existing) = engine.session().lc_resume_recovery.as_ref()
        && existing.state == crate::product::models::LcResumeRecoveryState::Waiting
        && existing.reason_code == reason
    {
        return SingleCandidateProviderRunError::NativeSupersededWaiting {
            reason_code: reason_code.to_string(),
            superseded_native_session_id: existing.superseded_native_session_id.clone(),
            recovery_id: existing.recovery_id.clone(),
        };
    }
    match engine
        .persist_single_candidate_author_superseded_wait(&reason)
        .await
    {
        Ok(recovery) => SingleCandidateProviderRunError::NativeSupersededWaiting {
            reason_code: reason_code.to_string(),
            superseded_native_session_id: recovery.superseded_native_session_id.clone(),
            recovery_id: recovery.recovery_id,
        },
        Err(error) => SingleCandidateProviderRunError::Message(format!(
            "{reason_code}: 等待收口持久化失败（{error}）；{detail}"
        )),
    }
}

/// `Waiting` 分支的收口：真实 CAS 既有 `LcResumeRecoveryState::Waiting`（保 SC
/// phase/reservation/ledger，不落 Failed/A4，不新 seq），成功后返回既有 typed
/// 等待。同一等待已落盘时复用真实 record（不刷新 recovery_id、不重复 CAS）。
pub(in crate::web::workspace_ws_handler::run) async fn wait_sc_round_launch(
    engine: &mut WorkspaceEngine,
    round: &ScRoundIdentity,
    reason_code: &'static str,
    detail: &str,
) -> SingleCandidateProviderRunError {
    let reason = format!("{reason_code}: {detail}");
    if let Some(existing) = engine.session().lc_resume_recovery.as_ref()
        && existing.state == crate::product::models::LcResumeRecoveryState::Waiting
        && existing.reason_code == reason
    {
        return SingleCandidateProviderRunError::NativeSupersededWaiting {
            reason_code: reason_code.to_string(),
            superseded_native_session_id: existing.superseded_native_session_id.clone(),
            recovery_id: existing.recovery_id.clone(),
        };
    }
    match engine
        .persist_single_candidate_author_superseded_wait(&reason)
        .await
    {
        Ok(recovery) => SingleCandidateProviderRunError::NativeSupersededWaiting {
            reason_code: reason_code.to_string(),
            superseded_native_session_id: recovery.superseded_native_session_id.clone(),
            recovery_id: recovery.recovery_id,
        },
        // CAS 失败：真实持久化错误上浮（不造「已 Waiting」、不落 Failed、不启动）。
        Err(error) => SingleCandidateProviderRunError::Message(format!(
            "{reason_code}: 等待收口持久化失败（{error}）；round={}；{detail}",
            round.round_id
        )),
    }
}

/// 教学轮 entry 收口：返回 `(交付物, 本轮是否发生真实启动)`——`Ready` 才真实
/// 驱动（调用方已按 `is_ready` 发过提示词事件），`Completed` 复用 durable raw
/// （不再 claim、不再发提示词、不记 `Sent`），`Waiting` 走真实 CAS 等待。
#[allow(clippy::too_many_arguments)]
#[allow(clippy::result_large_err)] // 同上：该枚举是运行收尾的唯一 typed 出口。
pub(in crate::web::workspace_ws_handler::run) async fn drive_sc_teaching_entry(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    launch: &PlanAuthorLaunch,
    provider_for_run: Arc<dyn StreamingProviderAdapter>,
    run_cancel: &CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    node_id: &str,
    author_provider: &ProviderName,
    reredrive_prompt: &str,
    repository_path: &std::path::Path,
    entry: ScRoundEntry,
    sc_teaching_attempt: Option<&crate::product::work_item_plan_policy::ScTeachingAttemptRef>,
    // P25 F1：真实发送边界审计（正文/locator 与已 Prepared 证据同源）。
    // 只有 Ready 会真正 start，因此也只有 Ready 会把它带到 transport 的记录点；
    // Completed（重放）与 Waiting（等待）永不写 Sent。
    sent_boundary: super::sc_failure_transport::ScTeachingSentBoundary<'_>,
) -> Result<ScAuthorRoundDelivery, SingleCandidateProviderRunError> {
    match entry {
        ScRoundEntry::Ready(bound) => {
            drive_single_candidate_reredrive(
                engine,
                launch,
                provider_for_run,
                run_cancel,
                command_rx,
                node_id,
                author_provider,
                reredrive_prompt,
                repository_path,
                &bound,
                lifecycle,
                sc_teaching_attempt,
                Some(sent_boundary),
            )
            .await
        }
        ScRoundEntry::Completed {
            bound,
            raw_text,
            raw,
        } => Ok(replayed_sc_round_delivery(raw_text, raw, bound.round)),
        ScRoundEntry::Waiting {
            round,
            reason_code,
            detail,
        } => Err(wait_sc_round_launch(engine, &round, reason_code, &detail).await),
    }
}
