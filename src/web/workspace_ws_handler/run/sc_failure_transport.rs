//! SC 教学重驱 transport（P01 纯搬迁切片）。
//!
//! 实现自 `run/single_candidate.rs` 整体迁出：函数签名、函数体、归一化与
//! 取消/AlreadyFinished/close/finalize 时序全部保持原样。归一化 helper 与
//! 两个归一化事件发射器仍由 `single_candidate_compile_preflight` 原样提供，
//! 本模块只调用不复制其实现。

use super::single_candidate_compile_preflight::{
    emit_author_ears_normalized_event, emit_author_heading_normalized_event,
    prepare_author_delivery_for_compile,
};
use super::*;

/// F2-B/3.6：教学重驱共用驱动——同 node 再驱一次 provider，返回归一化后的
/// markdown source。驱动中断（会话失败/取消）时已按既有终态形态持久化 Failed
/// 并返回 AlreadyFinished，与首轮驱动失败处理一致。
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
    // Task 1b 段②生产臂:重驱是独立 provider run,调用方每轮 begin 新
    // handle 后传入。
    plan_split_run: Option<&crate::web::workspace_ws_handler::run::PlanSplitRunContext>,
    // Task 4（REQ-WSC-28C）：教学 attempt 引用（claim 侧 Task 6 接线）。
    sc_teaching_attempt: Option<&crate::product::work_item_plan_policy::ScTeachingAttemptRef>,
) -> Result<String, SingleCandidateProviderRunError> {
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
        plan_split_run,
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
                plan_split_run,
                &format!("provider session drive failed: {message}"),
            );
            engine.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
            return Err(SingleCandidateProviderRunError::AlreadyFinished);
        }
    };
    crate::web::workspace_ws_handler::run::close_plan_split_run_with_markdown(
        plan_split_run,
        reredrive_prompt,
        &reredrive_output,
    );
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
    Ok(reredrive_delivery.source)
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
    fn sc_round_fixture() -> (tempfile::TempDir, LifecycleStore, ScFailureScope) {
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
    fn actual_starts(lifecycle: &LifecycleStore, session_id: &str) -> usize {
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
            failed.expect_err("empty business key refused").code,
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
