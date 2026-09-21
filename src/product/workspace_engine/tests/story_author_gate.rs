//! F-18（w2c 实测矩阵缺口）：story/design 会话（legacy 流）恒停 author_confirm
//! 门——approve 有 HTTP confirm 端点通路，terminate 零通路。本组用例钉
//! typed `abandon_human_gate` 的引擎关门语义：durable WaitingForHuman →
//! Terminated 终态 + Completed 阶段 + 「流程终止」节点 + 恰一条 close 事件。
use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
use crate::product::models::{ProviderName, WorkspaceSessionStatus, WorkspaceType};
use tempfile::TempDir;

/// story/design 会话在 author_confirm 门的引擎夹具：durable 记录
/// WaitingForHuman（author_confirm 的 status 映射），引擎内存态停在
/// AuthorConfirm 且持有活的 AuthorConfirm 时间线节点。
async fn story_gate_fixture(
    workspace_type: WorkspaceType,
) -> (
    TempDir,
    LifecycleStore,
    WorkspaceEngine,
    mpsc::Receiver<EngineEvent>,
) {
    let root = TempDir::new().expect("tempdir");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    // session id 进程内唯一：竞态 drift-hook 注册表以 session id 为全局键，
    // 并行测试族不得串扰（campaign harness 同款纪律）。
    static F18_SESSION_SEQUENCE: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(1);
    let session_id = format!(
        "workspace_session_f18_{}",
        F18_SESSION_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let mut record = lifecycle
        .create_workspace_session_with_id(
            CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "story_spec_0001".to_string(),
                workspace_type,
                author_provider: ProviderName::Fake,
                reviewer_provider: ProviderName::Fake,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                // story/design 会话不带 plan options：flow_kind 落默认 Legacy（生产同构）。
                work_item_plan_options: None,
            },
            session_id,
        )
        .expect("create story session");
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist story gate session");
    let (event_tx, event_rx) = mpsc::channel(16);
    let mut session = WorkspaceSession::from_record(record.clone());
    session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: "# Story Spec\n".to_string(),
        diff: None,
    });
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        session,
    );
    engine
        .enter_author_confirm(Some("Author 结果等待确认".to_string()))
        .await;
    (root, lifecycle, engine, event_rx)
}

fn collect_close_events(event_rx: &mut mpsc::Receiver<EngineEvent>) -> Vec<(String, String)> {
    let mut events = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        if let EngineEvent::HumanGateClosed { decision, stage } = event {
            events.push((decision, stage));
        }
    }
    events
}

#[tokio::test]
async fn story_author_gate_abandon_terminates_waiting_for_human_session() {
    for workspace_type in [WorkspaceType::Story, WorkspaceType::Design] {
        let (_root, lifecycle, mut engine, mut event_rx) = story_gate_fixture(workspace_type).await;
        assert_eq!(
            engine.session().stage,
            WorkspaceStage::AuthorConfirm,
            "夹具必须停在 author_confirm 门"
        );

        let outcome = engine
            .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
            .await
            .expect("story author gate abandon must close the gate");
        assert_eq!(outcome, HumanGateCloseOutcome::Abandoned);

        let durable = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("durable story session");
        assert_eq!(durable.status, WorkspaceSessionStatus::Terminated);
        assert_eq!(engine.session().stage, WorkspaceStage::Completed);
        assert_eq!(
            engine.session().session_status,
            WorkspaceSessionStatus::Terminated
        );
        let terminal_nodes: Vec<_> = engine
            .timeline_nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::Completed)
            .collect();
        assert_eq!(terminal_nodes.len(), 1, "必须落「流程终止」终态节点");
        assert_eq!(terminal_nodes[0].title, "流程终止");
        assert_eq!(terminal_nodes[0].status, TimelineNodeStatus::Completed);

        let close_events = collect_close_events(&mut event_rx);
        assert_eq!(
            close_events,
            vec![("terminate".to_string(), "completed".to_string())],
            "恰好一条 terminate close 事件（WS 权威关门通知）"
        );
    }
}

#[tokio::test]
async fn story_author_gate_repeat_abandon_is_idempotent_already_closed() {
    let (_root, lifecycle, mut engine, mut event_rx) =
        story_gate_fixture(WorkspaceType::Story).await;
    engine
        .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
        .await
        .expect("first abandon closes");
    let first_events = collect_close_events(&mut event_rx);
    assert_eq!(first_events.len(), 1);

    let second = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
        .await
        .expect("late abandon must be an idempotent no-op, not an error");
    assert_eq!(
        second,
        HumanGateCloseOutcome::AlreadyClosed {
            status: WorkspaceSessionStatus::Terminated
        }
    );
    let durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable story session");
    assert_eq!(durable.status, WorkspaceSessionStatus::Terminated);
    assert!(
        collect_close_events(&mut event_rx).is_empty(),
        "不重复发 close 事件"
    );
}

#[tokio::test]
async fn story_author_gate_abandon_does_not_clobber_confirmed_record() {
    let (_root, lifecycle, mut engine, _event_rx) = story_gate_fixture(WorkspaceType::Story).await;
    // HTTP confirm 先行（approve 端点写 Confirmed）后，迟到 abandon 必须
    // fail-closed：不把已确认会话改写为 Terminated。
    lifecycle
        .update_workspace_session_status(
            &engine.session().session_id,
            WorkspaceSessionStatus::Confirmed,
        )
        .expect("seed confirmed record");

    let error = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
        .await
        .expect_err("abandon must not clobber a confirmed session");
    assert!(
        error.contains("waiting_for_human"),
        "错误必须点名门开态前置（waiting_for_human）：{error}"
    );
    let durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable story session");
    assert_eq!(durable.status, WorkspaceSessionStatus::Confirmed);
}

#[tokio::test]
async fn work_item_plan_author_confirm_abandon_keeps_sc_close_boundary() {
    // 契约边界（REQ-CG-04/REQ-RET-03）：typed abandon 的 author_confirm 关门
    // 语义只服务 story/design 门；WorkItemPlan 会话（无论 SC 还是 legacy 流）
    // 不经此分支——仍走 SC close 的 human_confirm 前置守卫并明确报错。
    let (_root, _lifecycle, mut engine, _event_rx) = story_gate_fixture(WorkspaceType::Story).await;
    engine.session.workspace_type = WorkspaceType::WorkItemPlan;
    engine.session.stage = WorkspaceStage::AuthorConfirm;
    let error = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
        .await
        .expect_err("work item plan must not take the story author-gate branch");
    assert!(
        error.contains("single-candidate work-item plan"),
        "必须由 SC close 守卫报语义前置错误：{error}"
    );
}

#[tokio::test]
async fn story_author_gate_terminate_loses_race_to_http_confirm_without_overwrite() {
    // k3 审 P2：HTTP confirm（不持锁无条件写 Confirmed）落在引擎读与终态写之间时，
    // terminate 不得覆盖 Confirmed——CAS 失配后翻译为明确错误，durable 保持 Confirmed。
    let (_root, lifecycle, mut engine, mut event_rx) =
        story_gate_fixture(WorkspaceType::Story).await;
    let session_id = engine.session().session_id.clone();
    let raced_lifecycle = lifecycle.clone();
    let drifted_id = session_id.clone();
    crate::product::workspace_engine::conversational_gate::register_story_terminate_drift_hook(
        &session_id,
        Box::new(move || {
            raced_lifecycle
                .update_workspace_session_status(&drifted_id, WorkspaceSessionStatus::Confirmed)
                .expect("seed http-confirm race drift");
        }),
    );

    let outcome = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
        .await;
    match outcome {
        Err(error) => assert!(
            error.contains("race") && error.contains("Confirmed"),
            "竞态输方必须点名单飞竞态与当前终态：{error}"
        ),
        other => panic!("race-lost terminate must not report success: {other:?}"),
    }
    let durable = lifecycle
        .get_workspace_session(&session_id)
        .expect("durable story session");
    assert_eq!(
        durable.status,
        WorkspaceSessionStatus::Confirmed,
        "Confirmed 不得被迟到 terminate 覆盖"
    );
    assert!(
        collect_close_events(&mut event_rx).is_empty(),
        "竞态输方不得发 close 事件"
    );
}

#[tokio::test]
async fn story_author_gate_terminate_race_lost_to_other_terminate_translates_already_closed() {
    // 另一 worker 先到 terminate（读-CAS 窗口内落 Terminated）：迟到者 CAS 失配
    // 重读后翻译幂等 AlreadyClosed，不重复关门、不重复发事件。
    let (_root, lifecycle, mut engine, mut event_rx) =
        story_gate_fixture(WorkspaceType::Story).await;
    let session_id = engine.session().session_id.clone();
    let raced_lifecycle = lifecycle.clone();
    let drifted_id = session_id.clone();
    crate::product::workspace_engine::conversational_gate::register_story_terminate_drift_hook(
        &session_id,
        Box::new(move || {
            raced_lifecycle
                .update_workspace_session_status(&drifted_id, WorkspaceSessionStatus::Terminated)
                .expect("seed other-terminate race drift");
        }),
    );

    let outcome = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
        .await
        .expect("race-lost terminate must translate, not error");
    assert_eq!(
        outcome,
        HumanGateCloseOutcome::AlreadyClosed {
            status: WorkspaceSessionStatus::Terminated
        }
    );
    let durable = lifecycle
        .get_workspace_session(&session_id)
        .expect("durable story session");
    assert_eq!(durable.status, WorkspaceSessionStatus::Terminated);
    assert!(
        collect_close_events(&mut event_rx).is_empty(),
        "迟到者不得重复发 close 事件"
    );
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::AuthorConfirm,
        "迟到者不本地关门（先到者已关门，无 second close 语义）"
    );
}

#[tokio::test]
async fn story_terminate_store_cas_rejects_drifted_expected_record() {
    // 原子积木钉测（amendment.rs:733 同款）：expected 快照漂移后 CAS 必须
    // IdentityMismatch 且不落任何写入。
    let (_root, lifecycle, _engine, _event_rx) = story_gate_fixture(WorkspaceType::Story).await;
    let session_id = _engine.session().session_id.clone();
    let expected = lifecycle
        .get_workspace_session(&session_id)
        .expect("read expected snapshot");
    lifecycle
        .update_workspace_session_status(&session_id, WorkspaceSessionStatus::Confirmed)
        .expect("drift durable record");

    let raced = lifecycle
        .compare_and_update_workspace_session_status(&expected, WorkspaceSessionStatus::Terminated);
    assert!(
        matches!(
            raced,
            Err(crate::product::json_store::ProductStoreError::IdentityMismatch { .. })
        ),
        "drifted expected must be rejected by CAS: {raced:?}"
    );
    let durable = lifecycle
        .get_workspace_session(&session_id)
        .expect("durable story session");
    assert_eq!(
        durable.status,
        WorkspaceSessionStatus::Confirmed,
        "CAS 失配路径零写入"
    );
}

// F-31 fix round（k3 P1/P2；v36 纠正轮参数化）：HTTP confirm 端点的引擎裁决矩阵。引擎是
// stage 的唯一权威——门态按用户意愿分流（with_review=false 定稿 / =true 且未启用则如实拒收），
// 在途 stage（评审/生成/修订在途）拒收（在途守卫优先于送审请求），已 Confirmed 终态幂等返回，
// 非 story/design 不接管。CrossReview/Running 等评审在途 stage 在 it_core 面不可全覆盖：
// provider run 任务在整段 drive 里持引擎锁，HTTP 端点只能观测到 durable running
//（见 tests/it_core/workspace_ws_integration/part_02.rs 的 CrossReview 测试），故该矩阵在此钉测。
#[tokio::test]
async fn http_confirm_disposition_matrix_follows_stage_authority() {
    // 门态：AuthorConfirm / HumanConfirm / PrepareContext + 用户未要求送审（缺省
    // with_review=false）→ 放行定稿（纠正轮：是否评审由用户选择，review 启用不再强制接管）。
    for stage in [
        WorkspaceStage::AuthorConfirm,
        WorkspaceStage::HumanConfirm,
        WorkspaceStage::PrepareContext,
    ] {
        let (_root, _lifecycle, mut engine, _event_rx) =
            story_gate_fixture(WorkspaceType::Story).await;
        engine.session.stage = stage.clone();
        assert_eq!(
            engine.http_confirm_disposition(false).await,
            HttpConfirmDisposition::Finalize,
            "门态 {stage:?} + 未要求送审必须放行定稿"
        );
    }

    // 纠正轮（引擎面红③）：显式 with_review=true + review 未启用（夹具 review_rounds=0）
    // → 如实拒收（端点 4xx），不得静默按定稿；零写入。
    for stage in [
        WorkspaceStage::AuthorConfirm,
        WorkspaceStage::HumanConfirm,
        WorkspaceStage::PrepareContext,
    ] {
        let (_root, lifecycle, mut engine, _event_rx) =
            story_gate_fixture(WorkspaceType::Story).await;
        engine.session.stage = stage.clone();
        assert_eq!(
            engine.http_confirm_disposition(true).await,
            HttpConfirmDisposition::ReviewUnavailable,
            "with_review=true + review 未启用，门态 {stage:?} 必须如实拒收"
        );
        assert_eq!(engine.session().stage, stage, "拒收不得改写 stage");
        assert_eq!(
            engine.session().session_status,
            WorkspaceSessionStatus::WaitingForHuman,
            "拒收不得改写引擎状态"
        );
        let durable = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("durable story session");
        assert_eq!(
            durable.status,
            WorkspaceSessionStatus::WaitingForHuman,
            "拒收不得改写 durable 状态"
        );
    }

    // 在途 stage：与送审意愿无关一律拒收且零写入（评审不被绕过/不被改写；在途守卫优先）。
    for stage in [
        WorkspaceStage::CrossReview,
        WorkspaceStage::Running,
        WorkspaceStage::Revision,
        WorkspaceStage::ReviewDecision,
    ] {
        let (_root, lifecycle, mut engine, _event_rx) =
            story_gate_fixture(WorkspaceType::Story).await;
        engine.session.stage = stage.clone();
        assert_eq!(
            engine.http_confirm_disposition(false).await,
            HttpConfirmDisposition::Rejected {
                stage: stage.as_str()
            },
            "在途 stage {stage:?} 必须拒收"
        );
        assert_eq!(
            engine.http_confirm_disposition(true).await,
            HttpConfirmDisposition::Rejected {
                stage: stage.as_str()
            },
            "在途 stage {stage:?} 即便 with_review=true 也必须拒收"
        );
        assert_eq!(engine.session().stage, stage, "拒收不得改写 stage");
        assert_eq!(
            engine.session().session_status,
            WorkspaceSessionStatus::WaitingForHuman,
            "拒收不得改写引擎状态"
        );
        let durable = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("durable story session");
        assert_eq!(
            durable.status,
            WorkspaceSessionStatus::WaitingForHuman,
            "拒收不得改写 durable 状态"
        );
    }

    // 终态：已 Confirmed 幂等返回；未 Confirmed 的终态（terminated/failed/blocked）拒收
    let (_root, _lifecycle, mut engine, _event_rx) = story_gate_fixture(WorkspaceType::Story).await;
    engine.session.stage = WorkspaceStage::Completed;
    engine.session.session_status = WorkspaceSessionStatus::Confirmed;
    assert_eq!(
        engine.http_confirm_disposition(false).await,
        HttpConfirmDisposition::AlreadyConfirmed
    );
    engine.session.session_status = WorkspaceSessionStatus::Terminated;
    assert_eq!(
        engine.http_confirm_disposition(false).await,
        HttpConfirmDisposition::Rejected { stage: "completed" },
        "未 Confirmed 的终态不得被确认改写"
    );

    // 非 story/design：不接管（端点维持既有 store-only 通路）
    let (_root, _lifecycle, mut engine, _event_rx) = story_gate_fixture(WorkspaceType::Story).await;
    engine.session.workspace_type = WorkspaceType::WorkItemPlan;
    assert_eq!(
        engine.http_confirm_disposition(false).await,
        HttpConfirmDisposition::NotHandled
    );

    // review 启用且本轮产物未评审：送审与否由用户选择——
    // ① with_review=false（未要求送审）→ Finalize 且零写入（纠正轮引擎面红①：review
    //    启用不得强制接管）；
    // ② with_review=true → 接管本轮（F-31 正向锚）——Fake reviewer 快速路径
    //    （ReviewerRun=Skipped + mark_latest_artifact_reviewed）落 HumanConfirm。
    let (_root, lifecycle, mut engine, _event_rx) = story_gate_fixture(WorkspaceType::Story).await;
    engine.session.reviewer_enabled_at_start = Some(true);
    engine.session.stage = WorkspaceStage::AuthorConfirm;
    engine
        .artifact_versions
        .push(crate::web::workspace_ws_types::ArtifactVersion {
            version: 1,
            payload: crate::web::workspace_ws_types::ArtifactPayload::Markdown {
                markdown: "# Story Spec\n".to_string(),
                diff: None,
            },
            generated_by: ProviderName::Fake,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: true,
            created_at: "2026-09-21T00:00:00Z".to_string(),
            source_node_id: "timeline_node_001".to_string(),
        });
    assert_eq!(
        engine.http_confirm_disposition(false).await,
        HttpConfirmDisposition::Finalize,
        "review 启用但用户未要求送审必须放行定稿"
    );
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::AuthorConfirm,
        "定稿裁决不得改写 stage"
    );
    assert!(
        engine
            .timeline_nodes
            .iter()
            .all(|node| node.node_type != TimelineNodeType::ReviewerRun),
        "定稿裁决不得创建评审节点"
    );
    assert_eq!(
        engine.http_confirm_disposition(true).await,
        HttpConfirmDisposition::ReviewStarted,
        "review 启用且本轮未评审 + 用户显式送审必须接管本轮"
    );
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::HumanConfirm,
        "Fake reviewer 快速路径落人工确认门"
    );
    assert!(
        engine.timeline_nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::ReviewerRun
                && node.status == TimelineNodeStatus::Skipped
        }),
        "Fake reviewer 快速路径必须留 Skipped 评审节点"
    );
    let durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable story session");
    assert_eq!(durable.status, WorkspaceSessionStatus::WaitingForHuman);
}
