//! SC 教学重驱 transport（P01 纯搬迁切片）。
//!
//! 实现自 `run/single_candidate.rs` 整体迁出：函数签名、函数体、归一化与
//! 取消/close/finalize 时序保持原样；驱动中断与收口失败改由终局出口收口
//! （Durable/Prefix 落真实终局，Unknown 先返回既有 CAS 等待）。归一化 helper 与
//! 两个归一化事件发射器仍由 `single_candidate_compile_preflight` 原样提供，
//! 本模块只调用不复制其实现。

use super::single_candidate_compile_preflight::{
    emit_author_ears_normalized_event, emit_author_heading_normalized_event,
    prepare_author_delivery_for_compile,
};
use super::*;
use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceError, ScFailureEvidenceEvent, sc_failure_chain_id,
    sc_failure_split_locator,
};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::ProviderName;
use crate::product::work_item_split_engine::parse::WorkItemSplitProviderRunHandle;
use crate::web::workspace_ws_handler::run::PlanSplitRunContext;
use crate::web::workspace_ws_types::{
    ScEvidenceRef, ScFailureChainIdentity, ScFailureScope, ScRoundIdentity,
};

/// 一轮（原轮或教学轮）的绑定结果：链身份 + 轮身份 + 真实 split run 上下文。
///
/// `split` 是消费方真正吃进去的 run 输入（run-bound sink 绑定与 split run
/// 目录 key 同源）——它与 `round` 指向同一次物理 run，不是等值转发。
pub(in crate::web::workspace_ws_handler::run) struct ScBoundProviderRound {
    pub(in crate::web::workspace_ws_handler::run) chain: ScFailureChainIdentity,
    pub(in crate::web::workspace_ws_handler::run) round: ScRoundIdentity,
    pub(in crate::web::workspace_ws_handler::run) split: PlanSplitRunContext,
}

/// 驱动交付物：归一化 source（消费方 compile 用）+ 原始 raw 证据引用 + 轮身份。
///
/// raw 与 source 是两份不同正文：raw 恒为 provider 原始全文，source 为归一化
/// 后的 compile 源，二者摘要不得互相冒充。严格收口失败直接返回 Err，
/// 不以 Missing 证据伪装成成功交付。
pub(in crate::web::workspace_ws_handler::run) struct ScAuthorRoundDelivery {
    pub(in crate::web::workspace_ws_handler::run) source: String,
    pub(in crate::web::workspace_ws_handler::run) raw: ScEvidenceRef,
    pub(in crate::web::workspace_ws_handler::run) round: ScRoundIdentity,
}

/// 本文件的错误构造（store 的构造器私有；字段全 pub，可直接构造）。
fn sc_transport_error(
    code: &str,
    step: &str,
    round_id: Option<String>,
    detail: impl Into<String>,
    publication_incomplete: bool,
) -> ScFailureEvidenceError {
    ScFailureEvidenceError {
        code: code.to_string(),
        step: step.to_string(),
        round_id,
        detail: detail.into(),
        publication_incomplete,
        verified_prefix: None,
    }
}

/// SC 失败链的域作用域：会话四元组（plan_id 即会话实体——与 store 准入同源）。
pub(in crate::web::workspace_ws_handler::run) fn sc_failure_scope_for(
    engine: &WorkspaceEngine,
) -> ScFailureScope {
    let session = engine.session();
    ScFailureScope {
        session_id: session.session_id.clone(),
        project_id: session.project_id.clone(),
        issue_id: session.issue_id.clone(),
        plan_id: session.entity_id.clone(),
    }
}

/// 链身份：会话/业务键/原节点/原 role_run_seq 的长度前缀哈希。
fn sc_chain_identity(
    scope: &ScFailureScope,
    business_key: &str,
    original_node_id: &str,
    original_role_run_seq: u64,
) -> Result<ScFailureChainIdentity, ScFailureEvidenceError> {
    let chain_id = sc_failure_chain_id(
        &scope.session_id,
        business_key,
        original_node_id,
        original_role_run_seq,
    )
    .map_err(|mut failure| {
        failure.step = "begin_sc_failure_round".to_string();
        failure
    })?;
    Ok(ScFailureChainIdentity {
        scope: scope.clone(),
        chain_id,
        business_key: business_key.to_string(),
        original_node_id: original_node_id.to_string(),
        original_role_run_seq,
    })
}

/// 轮身份：round_id 由 (chain, node, ordinal) 稳定派生——同一轮重放命中同一
/// 槽位，不靠时间戳或运行序猜测。
fn sc_round_identity(
    chain: &ScFailureChainIdentity,
    node_id: &str,
    provider: &ProviderName,
    ordinal: u32,
    handle: &WorkItemSplitProviderRunHandle,
) -> ScRoundIdentity {
    ScRoundIdentity {
        chain_id: chain.chain_id.clone(),
        round_id: format!("{}-{}-{}", chain.chain_id, node_id, ordinal),
        node_id: node_id.to_string(),
        round_ordinal: ordinal,
        provider: provider.clone(),
        workspace_session_id: handle.workspace_session_id.clone(),
        role_run_seq: handle.role_run_seq,
        run_ref: handle.run_ref.clone(),
    }
}

/// 绑定一轮 SC 物理 run：真实 handle → chain 身份 → durable RoundBound。
///
/// 时序：RoundBound 在任何 provider 启动之前落盘（先 append 后读回）。
/// 重放：命中同一 (chain, node, ordinal) 的既有绑定时**复用既有 durable
/// handle**（不新分配 role_run_seq、不写新证据行、不启动 provider）；恢复出的
/// handle 由 gateway_start 构造器按真实 session/run 元数据校验后封装。
pub(in crate::web::workspace_ws_handler::run) fn begin_sc_failure_round(
    lifecycle: &LifecycleStore,
    scope: &ScFailureScope,
    business_key: &str,
    original: Option<&ScFailureChainIdentity>,
    node_id: &str,
    provider: &ProviderName,
    ordinal: u32,
) -> Result<ScBoundProviderRound, ScFailureEvidenceError> {
    const STEP: &str = "begin_sc_failure_round";
    // 教学轮沿用原轮的链身份（原 role_run_seq 即原轮的物理 run）；原轮的原
    // 节点即本轮节点。
    let original_node_id = original.map_or(node_id, |chain| chain.original_node_id.as_str());
    if let Some(chain) = original
        && (chain.business_key != business_key || chain.scope != *scope)
    {
        return Err(sc_transport_error(
            "sc_failure_chain_invalid",
            STEP,
            None,
            "教学轮作用域或业务键与原链不一致",
            false,
        ));
    }
    // 重放探测：store 严格 lookup 按身份找唯一 RoundBound（不限 latest、
    // 坏日志显式 Err、多命中 Err）。
    let existing = lifecycle
        .find_sc_failure_round(scope, business_key, original_node_id, node_id, ordinal)
        .map_err(|mut failure| {
            failure.step = STEP.to_string();
            failure
        })?;
    if let Some((chain, round)) = existing {
        if original.is_some_and(|expected| expected != &chain) || round.provider != *provider {
            return Err(sc_transport_error(
                "sc_failure_round_identity_mismatch",
                STEP,
                Some(round.round_id.clone()),
                "重放轮的链身份或 provider 与请求不一致",
                false,
            ));
        }
        let handle = WorkItemSplitProviderRunHandle {
            run_ref: round.run_ref.clone(),
            workspace_session_id: round.workspace_session_id.clone(),
            role_run_seq: round.role_run_seq,
        };
        // 恢复既有物理 run 的上下文（核真 session/project/issue/author provider/
        // run 元数据；不新分配 role_run_seq；封装字段仍 private）。
        let split =
            crate::web::workspace_ws_handler::run::gateway_start::plan_split_run_for_handle(
                lifecycle,
                &scope.project_id,
                &scope.issue_id,
                provider,
                handle,
            )
            .map_err(|message| {
                sc_transport_error(
                    "sc_failure_round_recovery_failed",
                    STEP,
                    Some(round.round_id.clone()),
                    message,
                    false,
                )
            })?;
        return Ok(ScBoundProviderRound { chain, round, split });
    }
    // 首绑：真实 handle（LC 与非 LC 一致）→ chain 身份 → RoundBound 先落盘。
    let split = crate::web::workspace_ws_handler::run::gateway_start::begin_plan_split_run(
        lifecycle,
        &scope.project_id,
        &scope.issue_id,
        provider,
        &scope.session_id,
    )
    .map_err(|message| {
        sc_transport_error(
            "sc_failure_split_begin_failed",
            STEP,
            None,
            message,
            true,
        )
    })?;
    let chain = match original {
        Some(chain) => chain.clone(),
        // 原轮：链在真实 handle 之后建（原 role_run_seq 即该 handle 的 seq）。
        None => sc_chain_identity(scope, business_key, node_id, split.handle.role_run_seq)?,
    };
    let round = sc_round_identity(&chain, node_id, provider, ordinal, &split.handle);
    lifecycle
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::RoundBound {
                round: round.clone(),
            },
        )
        .map_err(|mut failure| {
            failure.step = STEP.to_string();
            failure
        })?;
    // append 成功不代替严格读回；只有同一链、同一物理轮可读才允许 start。
    let persisted = lifecycle
        .find_sc_failure_round(scope, business_key, original_node_id, node_id, ordinal)
        .map_err(|mut failure| {
            failure.step = STEP.to_string();
            failure
        })?;
    if !persisted.is_some_and(|(saved_chain, saved_round)| saved_chain == chain && saved_round == round) {
        return Err(sc_transport_error(
            "sc_failure_round_readback_mismatch",
            STEP,
            Some(round.round_id.clone()),
            "RoundBound 写入后严格读回缺失或身份不一致",
            true,
        ));
    }
    Ok(ScBoundProviderRound { chain, round, split })
}

/// 教学轮开局：沿用原轮链身份（同一 scope/business_key/原节点），教学轮是新
/// 物理 provider run（新 handle + role_run_seq）。
///
/// `previous` = 主循环持有的当前轮（raw 引用 + 轮身份）：新轮 ordinal 由当前轮
/// 推进（当前轮 ordinal + 1；首轮教学 = 原轮 ordinal + 1），不写死轮次序号。
pub(in crate::web::workspace_ws_handler::run) fn begin_sc_teaching_round(
    lifecycle: &LifecycleStore,
    original: &ScBoundProviderRound,
    previous: Option<&(ScEvidenceRef, ScRoundIdentity)>,
    node_id: &str,
    provider: &ProviderName,
) -> Result<ScBoundProviderRound, ScFailureEvidenceError> {
    let current = previous.map_or(&original.round, |(_, round)| round);
    if current.chain_id != original.chain.chain_id
        || current.workspace_session_id != original.chain.scope.session_id
    {
        return Err(sc_transport_error(
            "sc_failure_round_identity_mismatch",
            "begin_sc_teaching_round",
            Some(current.round_id.clone()),
            "当前轮不属于原失败链或会话",
            false,
        ));
    }
    let ordinal = current.round_ordinal.checked_add(1).ok_or_else(|| {
        sc_transport_error(
            "sc_failure_round_ordinal_overflow",
            "begin_sc_teaching_round",
            Some(current.round_id.clone()),
            "当前轮逻辑序号无法推进",
            false,
        )
    })?;
    begin_sc_failure_round(
        lifecycle,
        &original.chain.scope,
        &original.chain.business_key,
        Some(&original.chain),
        node_id,
        provider,
        ordinal,
    )
}

/// 收口一轮：complete → 严格读回真实 raw → 逐字比较 → finalize launch 纪元。
///
/// raw 不可从内存自说自话：读回的是那条物理 run 的落盘产物，字节不同即拒
/// （不得把内存 source 落成假 durable revision）。任一失败一律 Err（不得用
/// Ok(Missing) 绕过「阻止下一次 start」）：真实已知的缺失事实随错误上浮，
/// 且失败不推进 launch 纪元（该轮不算成功收口）。
pub(in crate::web::workspace_ws_handler::run) fn close_sc_split_round(
    lifecycle: &LifecycleStore,
    bound: &ScBoundProviderRound,
    prompt: &str,
    raw: &str,
) -> Result<ScEvidenceRef, ScFailureEvidenceError> {
    const STEP: &str = "close_sc_split_round";
    let structured = serde_json::Value::String(raw.to_string());
    bound
        .split
        .complete(prompt, &structured)
        .map_err(|message| {
            sc_transport_error(
                "sc_failure_split_close_failed",
                STEP,
                Some(bound.round.round_id.clone()),
                message,
                true,
            )
        })?;
    let readback = lifecycle
        .read_sc_split_output(&bound.chain.scope, &bound.round)
        .map_err(|mut failure| {
            failure.step = STEP.to_string();
            failure
        })?;
    if readback != raw {
        return Err(sc_transport_error(
            "sc_failure_raw_mismatch",
            STEP,
            Some(bound.round.round_id.clone()),
            format!(
                "split run 落盘 raw 与 provider raw 不一致（{} vs {} 字节）",
                readback.len(),
                raw.len()
            ),
            true,
        ));
    }
    bound.split.finalize_launch_after_success();
    Ok(ScEvidenceRef::Available {
        // locator 由 store 的窄导出构造器给出（与 store 的 raw locator 同源，
        // transport 不复制前缀常量）。
        locator: sc_failure_split_locator(
            &bound.round.workspace_session_id,
            bound.round.role_run_seq,
        ),
        sha256: crate::web::workspace_ws_handler::run::single_candidate_structure_reredrive::sc_sha256(
            raw,
        ),
        bytes: raw.len() as u64,
    })
}

/// F2-B/3.6：教学重驱共用驱动——同 node 再驱一次 provider，返回归一化后的
/// markdown source。驱动中断/收口失败一律先落真实终局（Durable/Prefix），
/// `Started` 未交付计入缺项；启动观察不可确认（Unknown）时不落 Failed，
/// 先返回既有 CAS 等待（`NativeSupersededWaiting`）。
#[allow(clippy::too_many_arguments)]
#[allow(clippy::result_large_err)] // 上述枚举是运行收尾的唯一 typed 出口。
pub(in crate::web::workspace_ws_handler::run) async fn drive_single_candidate_reredrive(
    engine: &mut WorkspaceEngine,
    launch: &crate::web::workspace_ws_handler::run::gateway_start::PlanAuthorLaunch,
    provider_for_run: Arc<dyn StreamingProviderAdapter>,
    run_cancel: &CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    node_id: &str,
    author_provider: &crate::product::models::ProviderName,
    reredrive_prompt: &str,
    repository_path: &std::path::Path,
    // P08：教学轮的真实绑定（链身份 + 轮身份 + 该轮物理 run 上下文）。
    bound: &ScBoundProviderRound,
    // 严格收口需 store 读回真实 raw。
    lifecycle: &LifecycleStore,
    // Task 4（REQ-WSC-28C）：教学 attempt 引用（claim 侧 Task 6 接线）。
    sc_teaching_attempt: Option<&crate::product::work_item_plan_policy::ScTeachingAttemptRef>,
) -> Result<ScAuthorRoundDelivery, SingleCandidateProviderRunError> {
    let reredrive_input = engine
        .build_work_item_plan_streaming_input(
            crate::product::work_item_split_engine::types::provider_name_to_type(author_provider),
            reredrive_prompt.to_string(),
            repository_path.to_string_lossy().to_string(),
            author_provider.clone(),
            PlanAuthorOutputContract::Structured,
        )
        .map_err(SingleCandidateProviderRunError::Message)?;
    let reredrive_input = engine.attach_tool_policy_audit(reredrive_input);
    // T6 fix-loop r2：trust 分类已下沉 start_work_item_plan_author 的
    // Logical 分支共享口（教学重驱同臂）。
    let reredrive_session = start_work_item_plan_author(
        launch.clone(),
        Arc::clone(&provider_for_run),
        reredrive_input,
        run_cancel.clone(),
        Some(&bound.split),
    )
    .await;
    // Task 4（REQ-WSC-28C）：spawn 确认后、事件循环前同步回填（fail-closed）。
    let reredrive_session =
        engine.backfill_sc_teaching_launch(sc_teaching_attempt, reredrive_session);
    let reredrive_output = match engine
        .drive_work_item_plan_provider_session_to_output(
            reredrive_session,
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
            // 真实观察：attempt-key backfill 命中 ⇒ Started；其余 ⇒ Unknown
            // （Unknown 不 finalize、不 persist Failed，改走既有 CAS 等待）。
            let observations = super::sc_failure_evidence::teaching_launch_observations(
                lifecycle,
                &bound.chain.scope.session_id,
                &bound.round,
                sc_teaching_attempt,
            );
            return Err(super::sc_failure_evidence::terminal_exit_error(
                engine,
                lifecycle,
                &bound.chain,
                super::sc_failure_evidence::SC_TERMINAL_CLASS_TEACHING_INTERRUPTED,
                observations,
                format!("SC 教学重驱失败：{message}"),
            )
            .await);
        }
    };
    // P08 严格收口：complete → 读回真实 raw → 逐字比较 → 才 finalize 纪元。
    // 失败原样上浮：不得把归档失败包装成成功交付，也不推进 launch 纪元。
    let raw = match close_sc_split_round(lifecycle, bound, reredrive_prompt, &reredrive_output) {
        Ok(raw) => raw,
        Err(failure) => {
            let observations = super::sc_failure_evidence::teaching_launch_observations(
                lifecycle,
                &bound.chain.scope.session_id,
                &bound.round,
                sc_teaching_attempt,
            );
            return Err(super::sc_failure_evidence::terminal_exit_error(
                engine,
                lifecycle,
                &bound.chain,
                super::sc_failure_evidence::SC_TERMINAL_CLASS_TEACHING_INTERRUPTED,
                observations,
                format!("SC 教学重驱收口失败：{failure}"),
            )
            .await);
        }
    };
    let reredrive_delivery = prepare_author_delivery_for_compile(&reredrive_output);
    emit_author_heading_normalized_event(
        engine,
        node_id,
        &reredrive_output,
        &reredrive_delivery,
        author_provider,
    )
    .await;
    emit_author_ears_normalized_event(
        engine,
        node_id,
        &reredrive_output,
        &reredrive_delivery,
        author_provider,
    )
    .await;
    Ok(ScAuthorRoundDelivery {
        source: reredrive_delivery.source,
        raw,
        round: bound.round.clone(),
    })
}

/// P08 红测：SC 物理轮与严格证据 store 的绑定面（store-binding 纯单测）。
///
/// 覆盖面：原轮/教学轮各一真 handle 且互不相同、durable RoundBound 可逐字
/// 读回、同轮重放复用同一 handle（不再分配 role_run_seq）、非 SC 路径零身份
/// 零日志。
///
/// 边界：本模块**不**证明「bound 早于真实 provider start」——真实 start 顺序
/// 由 controller 的既有 handler 集成臂承担（bound 写失败 ⇒ 真实 start = 0；
/// 成功 start 时 bound 行已被消费方可读）。本模块亦**不**声称覆盖 LC 运行时
/// （LC 覆盖由既有 `R single_candidate_provider_run` 套件承担）。
///
/// 断言口径：只按消费者可见身份断言（round_id / role_run_seq / run_ref /
/// chain_id / workspace_session_id），不 pin store 的私有逻辑槽键前缀；
/// `role_run_seq` 不断言 > 0（会话首个 handle 合法地 seed 0）。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::inputs::{
        CreateWorkspaceSessionInput, WorkItemPlanSessionOptions,
    };
    use crate::product::lifecycle_store::sc_failure_evidence::{
        ScFailureEvidenceEvent, ScFailureEvidenceRead, sc_failure_chain_id,
    };
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::product::models::{ProviderName, WorkspaceType};
    use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
    use crate::web::workspace_ws_types::{
        ScEvidenceRef, ScFailureChainIdentity, ScFailureScope, ScRoundIdentity,
    };

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const PLAN_ID: &str = "plan_0001";
    const NODE_ID: &str = "node_sc_0001";
    const BUSINESS_KEY: &str = "workspace_author:session:0";

    /// SC single-candidate WorkItemPlan session (store admission requires
    /// flow_kind == SingleCandidate and scope.plan_id == session.entity_id).
    pub(super) fn sc_round_fixture() -> (tempfile::TempDir, LifecycleStore, ScFailureScope) {
        let tmp = tempfile::tempdir().expect("temp root");
        let lifecycle = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.into(),
                issue_id: ISSUE_ID.into(),
                entity_id: PLAN_ID.into(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: Some(ProviderName::Fake),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(WorkItemPlanSessionOptions {
                    flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                    ..Default::default()
                }),
            })
            .expect("create SC session");
        let scope = ScFailureScope {
            session_id: record.id,
            project_id: PROJECT_ID.into(),
            issue_id: ISSUE_ID.into(),
            plan_id: PLAN_ID.into(),
        };
        (tmp, lifecycle, scope)
    }

    /// Durable physical provider starts (claim placeholders excluded).
    pub(super) fn actual_starts(lifecycle: &LifecycleStore, session_id: &str) -> usize {
        lifecycle
            .get_workspace_session(session_id)
            .expect("session readable")
            .provider_start_ledger
            .iter()
            .filter(|entry| entry.is_actual_launch_observation())
            .count()
    }

    /// Bound rounds read back from the strict evidence log (event-level view).
    fn bound_rounds(read: &ScFailureEvidenceRead) -> Vec<ScRoundIdentity> {
        read.entries
            .iter()
            .filter_map(|entry| match &entry.event {
                ScFailureEvidenceEvent::RoundBound { round } => Some(round.clone()),
                _ => None,
            })
            .collect()
    }

    fn begin(
        lifecycle: &LifecycleStore,
        scope: &ScFailureScope,
        original: Option<&ScFailureChainIdentity>,
        ordinal: u32,
    ) -> ScBoundProviderRound {
        begin_sc_failure_round(
            lifecycle,
            scope,
            BUSINESS_KEY,
            original,
            NODE_ID,
            &ProviderName::Fake,
            ordinal,
        )
        .expect("round binds a real handle")
    }

    #[test]
    fn original_and_teaching_rounds_bind_distinct_durable_handles() {
        let (_tmp, lifecycle, scope) = sc_round_fixture();
        let original = begin(&lifecycle, &scope, None, 0);
        assert_eq!(
            (original.round.round_ordinal, original.round.node_id.as_str()),
            (0, NODE_ID)
        );
        assert_eq!(original.round.workspace_session_id, scope.session_id);
        // `split` is the consumer's actual shared run input (it drives the
        // run-bound sink); it must name the same physical run as the DTO round.
        assert_eq!(
            (
                original.split.handle.run_ref.clone(),
                original.split.handle.role_run_seq
            ),
            (original.round.run_ref.clone(), original.round.role_run_seq)
        );
        assert_eq!(original.chain.business_key, BUSINESS_KEY);
        let teaching = begin(&lifecycle, &scope, Some(&original.chain), 1);
        assert_eq!(teaching.round.round_ordinal, 1);
        assert_eq!(teaching.chain.chain_id, original.chain.chain_id);
        assert_eq!(
            teaching.chain.original_role_run_seq,
            original.round.role_run_seq
        );
        assert_ne!(teaching.round.round_id, original.round.round_id);
        assert_ne!(teaching.round.run_ref, original.round.run_ref);
        assert_ne!(teaching.round.role_run_seq, original.round.role_run_seq);

        let raw_original = "{\"work_items\":[{\"id\":\"a\",\"raw\":\"original\"}]}";
        let raw_teaching = "{\"work_items\":[{\"id\":\"a\",\"raw\":\"teaching-revision\"}]}";
        let original_ref = close_sc_split_round(&lifecycle, &original, "original prompt", raw_original)
            .expect("original close reads its own raw back verbatim");
        let teaching_ref = close_sc_split_round(&lifecycle, &teaching, "teaching prompt", raw_teaching)
            .expect("teaching close reads its own raw back verbatim");
        assert!(matches!(original_ref, ScEvidenceRef::Available { .. }));
        assert!(matches!(teaching_ref, ScEvidenceRef::Available { .. }));

        // Rebuilt store: per-round verbatim raw + both RoundBound rows durable.
        let rebuilt = LifecycleStore::new(lifecycle.app_paths());
        assert_eq!(
            rebuilt
                .read_sc_split_output(&scope, &original.round)
                .expect("original raw"),
            raw_original
        );
        // Consumer-input check: locating the raw from the shared `split.handle`
        // alone must hit the same physical run content — not just field equality.
        let via_shared_handle = ScRoundIdentity {
            run_ref: original.split.handle.run_ref.clone(),
            role_run_seq: original.split.handle.role_run_seq,
            workspace_session_id: original.split.handle.workspace_session_id.clone(),
            ..original.round.clone()
        };
        assert_eq!(
            rebuilt
                .read_sc_split_output(&scope, &via_shared_handle)
                .expect("shared raw"),
            raw_original
        );
        assert_eq!(
            rebuilt
                .read_sc_split_output(&scope, &teaching.round)
                .expect("teaching raw"),
            raw_teaching
        );
        let read = rebuilt
            .read_sc_failure_evidence(&original.chain)
            .expect("chain evidence readable");
        let rounds = bound_rounds(&read);
        assert_eq!(rounds.len(), 2, "exactly one RoundBound per physical round");
        assert!(rounds.contains(&original.round) && rounds.contains(&teaching.round));
        // Consumer view: the bound round identity is what the run consumer reads.
        // `role_run_seq` is NOT asserted to be > 0 (the first handle of a session
        // legitimately seeds 0) — the identity facts are: two physical runs with
        // different handles, and each round's raw read back from its own run.json.
        for round in &rounds {
            assert!(!round.round_id.is_empty());
            assert_eq!(round.chain_id, original.chain.chain_id);
            assert_eq!(round.workspace_session_id, scope.session_id);
            assert!(!round.run_ref.is_empty());
        }
        assert_ne!(rounds[0].role_run_seq, rounds[1].role_run_seq);
        assert_ne!(rounds[0].run_ref, rounds[1].run_ref);
    }

    /// Store-binding unit: `begin` never starts a provider, so ledger==0 here
    /// only says this path spends nothing. Real-start ordering belongs to the P08
    /// handler integration (bound-write failure => no start; start => bound readable).
    #[test]
    fn store_binding_is_durable_and_starts_nothing_on_this_path() {
        let (_tmp, lifecycle, scope) = sc_round_fixture();
        let bound = begin(&lifecycle, &scope, None, 0);
        // Bound row is readable from disk without any provider involvement.
        let rebuilt = LifecycleStore::new(lifecycle.app_paths());
        let read = rebuilt
            .read_sc_failure_evidence(&bound.chain)
            .expect("bound row readable");
        assert_eq!(bound_rounds(&read), vec![bound.round.clone()]);
        assert!(
            read.entries
                .iter()
                .all(|entry| !matches!(entry.event, ScFailureEvidenceEvent::FailureObserved { .. })),
            "no failure row before any provider ran"
        );
        assert_eq!(actual_starts(&lifecycle, &scope.session_id), 0);
        assert_eq!(
            rebuilt
                .read_sc_split_output(&scope, &bound.round)
                .expect_err("no raw yet")
                .code,
            "sc_failure_raw_missing"
        );
        // Binding failure before any start spends nothing either.
        let failed = begin_sc_failure_round(
            &lifecycle,
            &scope,
            "",
            Some(&bound.chain),
            NODE_ID,
            &ProviderName::Fake,
            1,
        );
        assert_eq!(
            failed.err().expect("empty business key refused").code,
            "sc_failure_chain_invalid"
        );
        assert_eq!(actual_starts(&lifecycle, &scope.session_id), 0, "failed binding adds no start");
        assert_eq!(
            lifecycle
                .read_sc_failure_evidence(&bound.chain)
                .expect("evidence readable")
                .entries
                .len(),
            1,
            "failed binding adds no evidence row"
        );
    }

    #[test]
    fn replaying_the_same_round_reuses_the_handle_without_new_seq() {
        let (_tmp, lifecycle, scope) = sc_round_fixture();
        let first = begin(&lifecycle, &scope, None, 0);
        let rows_before = lifecycle
            .read_sc_failure_evidence(&first.chain)
            .expect("evidence readable")
            .entries
            .len();
        let starts_before = actual_starts(&lifecycle, &scope.session_id);
        // Replay: the transport must recover the SAME durable handle from the
        // strict binding — no second begin, no second role_run_seq.
        let replay = begin(&lifecycle, &scope, None, 0);
        assert_eq!(replay.chain.chain_id, first.chain.chain_id);
        assert_eq!(replay.round.round_id, first.round.round_id);
        assert_eq!(replay.round.run_ref, first.round.run_ref);
        assert_eq!(replay.round.role_run_seq, first.round.role_run_seq);
        assert_eq!(replay.split.handle.role_run_seq, first.split.handle.role_run_seq);
        assert_eq!(replay.split.handle.run_ref, first.split.handle.run_ref);
        assert_eq!(
            lifecycle
                .read_sc_failure_evidence(&first.chain)
                .expect("evidence readable")
                .entries
                .len(),
            rows_before,
            "replay adds no evidence row"
        );
        assert_eq!(
            actual_starts(&lifecycle, &scope.session_id),
            starts_before,
            "replay starts nothing"
        );
    }

    #[test]
    fn legacy_non_sc_caller_adds_no_split_identity_and_no_sc_log() {
        let (tmp, lifecycle, scope) = sc_round_fixture();
        let legacy = crate::web::workspace_ws_handler::run::begin_plan_split_run_if_logical(
            &crate::web::workspace_ws_handler::run::gateway_start::PlanAuthorLaunch::Legacy,
            &lifecycle,
            PROJECT_ID,
            ISSUE_ID,
            &ProviderName::Fake,
            &scope.session_id,
        )
        .expect("legacy direct launch stays infallible");
        assert!(legacy.is_none(), "non-SC gets no split identity");
        let chain = ScFailureChainIdentity {
            scope: scope.clone(),
            chain_id: sc_failure_chain_id(&scope.session_id, BUSINESS_KEY, NODE_ID, 0)
                .expect("chain id"),
            business_key: BUSINESS_KEY.into(),
            original_node_id: NODE_ID.into(),
            original_role_run_seq: 0,
        };
        assert!(
            lifecycle
                .read_sc_failure_evidence(&chain)
                .expect("absent log reads empty")
                .entries
                .is_empty(),
            "legacy path writes no SC evidence row"
        );
        assert_eq!(actual_starts(&lifecycle, &scope.session_id), 0);
        assert!(
            !contains_dir(tmp.path(), "sc-failure-evidence"),
            "no SC evidence directory"
        );
    }

    fn contains_dir(root: &std::path::Path, name: &str) -> bool {
        let Ok(entries) = std::fs::read_dir(root) else {
            return false;
        };
        entries.flatten().any(|entry| {
            let path = entry.path();
            path.is_dir() && (entry.file_name() == name || contains_dir(&path, name))
        })
    }
}

/// P08 replay-safe 分类红测（一）：`Ready` / `Completed`（durable raw 逐字复用）。
///
/// 复本文件守护 1200 行硬线；fixture/启动计数复用父模块 `tests` 的既有 helper
/// （`pub(super)`），不复制第二份。
#[cfg(test)]
#[path = "sc_failure_replay_completed_tests.rs"]
mod replay_completed_tests;

/// P08 replay-safe 分类红测（二）：审计启动事实 → `Waiting`（含教学轮 ordinal≥1）。
#[cfg(test)]
#[path = "sc_failure_replay_launch_tests.rs"]
mod replay_launch_tests;
