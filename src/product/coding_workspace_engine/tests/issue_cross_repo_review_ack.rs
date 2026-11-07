// issue 级跨仓最终评审的「已知风险留痕 + 时序收敛」测试（REQ-GFR-CR-05
// 「advisory 呈现」/ REQ-GFR-CR-06「已知风险，仍然确认」留痕，设计 B §五 C /
// §六 D；P2-1 补写 / P3-1 时限回归）。
//
// - request_changes 不阻断最终确认；确认后留痕恰好一条、字段最小化
//   （不记确认人/通道）。
// - 「确认早于评审落盘」时序：评审落盘后幂等补写留痕，advisory 提醒关闭。
// - provider start 楔死按 advisory 短时限收敛为 Failed 落盘。

use super::issue_cross_repo_review_fixture::*;
use super::*;
use crate::cross_cutting::streaming_provider::ProviderCompletion;
use crate::product::coding_models::{
    CodingRoleProviderConfigSnapshot, CodingStageGateStatus, IssueCrossRepoReviewStatus,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// Task 18：advisory 呈现 + 已知风险留痕（REQ-GFR-CR-05「advisory 红线」/
// REQ-GFR-CR-06「已知风险确认留痕」，设计 B §五 C / §六 D）。
// ---------------------------------------------------------------------------

/// request_changes 假 provider 输出：1 条 error + 1 条 warning finding。
const REQUEST_CHANGES_OUTPUT: &str = "{\"verdict\":\"request_changes\",\"summary\":\"跨仓契约不一致\",\"findings\":[{\"severity\":\"error\",\"message\":\"网关与前端对公告分页参数命名不一致\"},{\"severity\":\"warning\",\"message\":\"busi 删除接口缺少级联清理\"}]}";

/// 把已停在人工最终确认的 attempt 补齐 `head_commit`（完成门需要）。
fn with_head_commit(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> CodingExecutionAttempt {
    let worktree = attempt
        .worktree_path
        .clone()
        .expect("fixture attempt has worktree");
    let head = super::git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    store
        .update_attempt_head_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            Some(head),
        )
        .expect("seed head commit")
}

#[tokio::test]
async fn cross_repo_request_changes_still_allows_final_confirm() {
    let (_root, store, engine, attempts) =
        cross_repo_trigger_fixture_with_states(2, &[false, true]);
    let (resolver, _starts, _prompts) = fake_review_provider(REQUEST_CHANGES_OUTPUT);

    // 评审记录在人工确认之前落盘（T17 接线），结论为要求修改。
    engine
        .maybe_complete_issue_delivery(&attempts[0], &resolver)
        .await
        .expect("issue delivery completion");
    let review = wait_for_review_record(&store).await;
    assert_eq!(review.verdict, ReviewVerdict::RequestChanges);

    // REQ-GFR-CR-05：要求修改仍可最终确认，attempt 进入完成态。
    let parked = with_head_commit(&store, &attempts[1]);
    let confirmed = engine
        .handle_final_confirm(PROJECT_ID, ISSUE_ID, &parked.id)
        .await
        .expect("request_changes 不得阻断最终确认");
    assert_eq!(confirmed.status, CodingAttemptStatus::Completed);

    // issue 不被阻断。
    let issue = IssueStore::new(store.paths())
        .get(PROJECT_ID, ISSUE_ID)
        .expect("issue record");
    assert_ne!(issue.status, IssueStatus::Blocked);

    // 待处理门保持为空（advisory 不建门：choice / blocked / stage 全空）。
    for attempt_id in [&attempts[0].id, &confirmed.id] {
        assert!(
            store
                .list_open_choice_gates(PROJECT_ID, ISSUE_ID, attempt_id)
                .expect("choice gates")
                .is_empty()
        );
        assert!(
            store
                .list_open_blocked_gate_facts(PROJECT_ID, ISSUE_ID, attempt_id)
                .expect("blocked gates")
                .is_empty()
        );
        assert!(
            store
                .list_open_stage_gates(PROJECT_ID, ISSUE_ID, attempt_id)
                .expect("stage gates")
                .iter()
                .all(|gate| gate.status != CodingStageGateStatus::Open)
        );
    }

    // 确认在案：REQ-GFR-CR-06 留痕恰好一条（字段级断言见下一用例）。
    let acks = store
        .list_issue_cross_repo_acknowledgements(PROJECT_ID, ISSUE_ID)
        .expect("acknowledgements");
    assert_eq!(acks.len(), 1, "要求修改下确认必须留下已知风险留痕");
    assert_eq!(acks[0].cross_repo_review_id, review.id);
    assert_eq!(acks[0].attempt_id, confirmed.id);
}

#[tokio::test]
async fn cross_repo_acknowledgement_written_once_after_final_confirm() {
    let (_root, store, engine, attempts) =
        cross_repo_trigger_fixture_with_states(2, &[false, true]);
    let (resolver, _starts, _prompts) = fake_review_provider(REQUEST_CHANGES_OUTPUT);
    engine
        .maybe_complete_issue_delivery(&attempts[0], &resolver)
        .await
        .expect("issue delivery completion");
    let review = wait_for_review_record(&store).await;
    assert_eq!(review.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(review.findings.len(), 2);

    // 确认前无留痕。
    assert!(
        store
            .list_issue_cross_repo_acknowledgements(PROJECT_ID, ISSUE_ID)
            .expect("acknowledgements")
            .is_empty()
    );

    let parked = with_head_commit(&store, &attempts[1]);
    engine
        .handle_final_confirm(PROJECT_ID, ISSUE_ID, &parked.id)
        .await
        .expect("final confirm");

    // 确认后恰好一条留痕：时间 + 评审 id + 结论 + findings 摘要。
    let acks = store
        .list_issue_cross_repo_acknowledgements(PROJECT_ID, ISSUE_ID)
        .expect("acknowledgements");
    assert_eq!(acks.len(), 1, "确认后必须恰好留痕一条");
    let ack = &acks[0];
    assert_eq!(ack.cross_repo_review_id, review.id);
    assert_eq!(ack.attempt_id, parked.id);
    assert_eq!(ack.verdict, ReviewVerdict::RequestChanges);
    assert!(
        chrono::DateTime::parse_from_rfc3339(&ack.acknowledged_at).is_ok(),
        "acknowledged_at 必须是 RFC3339 时间"
    );
    assert!(ack.findings_summary.contains("error=1"));
    assert!(ack.findings_summary.contains("warning=1"));
    assert!(
        ack.findings_summary
            .contains("网关与前端对公告分页参数命名不一致")
    );
    assert!(ack.findings_summary.contains("busi 删除接口缺少级联清理"));

    // 原始 JSON 只含允许字段：不得出现确认人 / 通道（owner 裁定）。
    let ack_dir = store
        .paths()
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("coding-cross-repo-reviews")
        .join("acknowledgements");
    let mut ack_files: Vec<PathBuf> = fs::read_dir(&ack_dir)
        .expect("acknowledgements dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect();
    ack_files.sort();
    assert_eq!(ack_files.len(), 1);
    let raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&ack_files[0]).expect("read ack"))
            .expect("ack json");
    let allowed = [
        "id",
        "project_id",
        "issue_id",
        "plan_id",
        "attempt_id",
        "cross_repo_review_id",
        "verdict",
        "findings_summary",
        "acknowledged_at",
    ];
    for key in raw.as_object().expect("ack object").keys() {
        assert!(
            allowed.contains(&key.as_str()),
            "留痕不得含确认人/通道等字段：{key}"
        );
    }
    assert!(raw.get("acknowledged_by").is_none());
    assert!(raw.get("acknowledged_channel").is_none());

    // 幂等（REQ-GFR-CR-06）：同一 (cross_repo_review_id, attempt_id) 只写一条。
    let mut duplicate = ack.clone();
    duplicate.id = "cross_repo_review_ack_9999".to_string();
    let written = store
        .append_issue_cross_repo_acknowledgement(&duplicate)
        .expect("duplicate append");
    assert!(!written, "同键留痕必须幂等拒绝");
    // 重复确认被状态机拒绝，也不得新增留痕。
    assert!(
        engine
            .handle_final_confirm(PROJECT_ID, ISSUE_ID, &parked.id)
            .await
            .is_err()
    );
    assert_eq!(
        store
            .list_issue_cross_repo_acknowledgements(PROJECT_ID, ISSUE_ID)
            .expect("acknowledgements")
            .len(),
        1,
        "重复确认不得产生第二条留痕"
    );
}

// ---------------------------------------------------------------------------
// P2-1 / P3-1 修复回归：确认早于评审落盘的留痕补写、advisory 时限覆盖
// provider.start。
// ---------------------------------------------------------------------------

/// 受控假 provider（P2-1）：`start` 立即返回会话，但终态 `Completed` 事件被
/// gate 挡住——`release` 前评审既不落盘也不结束，用于构造「用户确认早于
/// 评审落盘」的时序。
struct GatedFakeProvider {
    output: String,
    starts: Arc<AtomicUsize>,
    release: tokio::sync::watch::Receiver<bool>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for GatedFakeProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        let output = self.output.clone();
        let mut release = self.release.clone();
        tokio::spawn(async move {
            while !*release.borrow_and_update() {
                if release.changed().await.is_err() {
                    return;
                }
            }
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::from_output(
                    output, None, None,
                )))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

/// gated provider 的触发边界：`(resolver, starts, release)`——`release` 放行
/// 前评审不落盘。
fn gated_review_provider(
    output: &str,
) -> (
    IssueCrossRepoReviewProviderResolver<'static>,
    Arc<AtomicUsize>,
    tokio::sync::watch::Sender<bool>,
) {
    let (release_tx, release_rx) = tokio::sync::watch::channel(false);
    let provider = GatedFakeProvider {
        output: output.to_string(),
        starts: Arc::new(AtomicUsize::new(0)),
        release: release_rx,
    };
    let starts = provider.starts.clone();
    let adapter: Arc<dyn StreamingProviderAdapter> = Arc::new(provider);
    let resolver =
        IssueCrossRepoReviewProviderResolver::new(move |_: &ProviderName| Some(adapter.clone()));
    (resolver, starts, release_tx)
}

/// start 楔死假 provider（P3-1）：`start` 计数后永不返回——模拟版本探测/
/// 握手/进程 spawn 楔死。修复前 `start` 在 advisory 时限之外，评审任务挂
/// 在 `start` 上永不落盘、在飞登记永不释放。
struct HangingStartProvider {
    starts: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for HangingStartProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        // start 永不返回（远超 advisory 时限）。
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        unreachable!("hanging start provider must never return a session")
    }
}

/// P2-1 主场景夹具：多仓 issue + group 形态「最终确认」链路——readiness
/// 材料（units/独立审查）由 `seed_complete_group_readiness` 补齐后，attempt
/// 可经 `prepare_group_final_confirm_from_readiness` 进入 FinalConfirm、由
/// `handle_final_confirm` 确认。readiness fixture 默认无 issue 记录、无
/// lifecycle work items（schema_v2 拓扑）——两者补齐后跨仓触发门（issue
/// 形态 + 全部 work item 开发完成）才放行。
fn cross_repo_group_readiness_fixture() -> super::group_final_readiness_support::ReadinessFixture {
    let fixture = super::group_final_readiness_support::readiness_fixture();
    // 多仓 issue 记录（P2-3 口径：形态按 issue 的 logical_codebase_id）。
    // readiness fixture 的 group attempt 已预建 `issues/issue_0001/` 目录
    // 树（无 issue.json）——`IssueStore::create` 的目录序号分配只能拿到
    // issue_0002；attempt 侧身份固定 issue_0001，把记录以 issue_0001 身份
    // 直接落盘。
    let mut issue = IssueStore::new(fixture.store.paths())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id: CROSS_REPO_LC_ID.to_string(),
            title: "issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("create multi-repo issue record");
    issue.id = ISSUE_ID.to_string();
    crate::product::json_store::write_json(
        &fixture
            .store
            .paths()
            .project_root(PROJECT_ID)
            .join("issues")
            .join(ISSUE_ID)
            .join("issue.json"),
        &issue,
    )
    .expect("persist multi-repo issue under issue_0001");
    // 触发门的 work item 拓扑（`issue_development_finished` 枚举 lifecycle
    // work items；schema_v2 fixture 不建 v1 记录 → 空集不触发评审）。
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    for index in 1..=3 {
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(format!("work_item_{index:04}")),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: "repository_0001".to_string(),
                title: format!("group work item {index}"),
                work_item_set_id: Some("work_item_plan_0001".to_string()),
                sequence_hint: Some(index * 10),
                plan_status: WorkItemPlanStatus::Confirmed,
                ..Default::default()
            })
            .expect("work item");
    }
    lifecycle
        .create_issue_work_item_plan(
            crate::product::lifecycle_store::CreateIssueWorkItemPlanInput {
                id: Some("work_item_plan_0001".to_string()),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                source_story_spec_ids: Vec::new(),
                source_design_spec_ids: Vec::new(),
                options: crate::product::models::IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: crate::product::models::IssueWorkItemPlanStatus::Confirmed,
                work_item_ids: (1..=3)
                    .map(|index| format!("work_item_{index:04}"))
                    .collect(),
                repository_profile_ref: None,
                verification_plan_ids: Vec::new(),
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
            },
        )
        .expect("group plan");
    // 跨仓评审 provider 在场（`.internal_reviewer`）。
    fixture
        .store
        .update_role_provider_config_snapshot(
            PROJECT_ID,
            ISSUE_ID,
            &fixture.attempt.id,
            CodingRoleProviderConfigSnapshot {
                coder: ProviderName::Codex,
                code_reviewer: None,
                internal_reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 0,
                permission_modes: Default::default(),
            },
        )
        .expect("seed internal reviewer role snapshot");
    fixture
}

/// 轮询等待留痕落盘（P2-1：补写发生在评审 `save` 之后的同一后台任务内，
/// 观测方可能先看到评审记录、后看到留痕）。
async fn wait_for_acknowledgements(
    store: &CodingAttemptStore,
) -> Vec<crate::product::coding_models::IssueCrossRepoRiskAcknowledgement> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let acks = store
            .list_issue_cross_repo_acknowledgements(PROJECT_ID, ISSUE_ID)
            .expect("acknowledgements");
        if !acks.is_empty() {
            return acks;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "留痕必须在评审落盘后被补写（5s 内）"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

/// P2-1（REQ-GFR-CR-06）：「用户确认早于评审落盘」时序——异步化（P1-2）
/// 后触发不再 await 终态，用户点击确认时评审记录可能尚未落盘：确认侧
/// 留痕（`record_cross_repo_risk_acknowledgement_if_needed`）读不到记录直接
/// 返回，而重复确认被状态机拒绝 → 留痕永久缺失、advisory 提醒永久悬空。
/// 修复：评审落盘后对已确认（`WorkItemGroup` attempt 的 `Completed` 唯一
/// 路径是最终确认）的 attempt 补写留痕，幂等键 `(review_id, attempt_id)`
/// 与确认侧共用。修复前本用例在「留痕必须被补写」处失败（留痕 0 条）。
#[tokio::test]
async fn cross_repo_acknowledgement_backfilled_when_confirm_precedes_review_landing() {
    let fixture = cross_repo_group_readiness_fixture();
    let running = super::group_final_readiness_support::seed_complete_group_readiness(&fixture);
    let (resolver, starts, release) = gated_review_provider(REQUEST_CHANGES_OUTPUT);

    // group 收口钩子：进 FinalConfirm 并触发跨仓评审（gated provider 已
    // 启动，结论被挡住，评审尚未落盘）。
    let prepared = fixture
        .engine
        .prepare_group_final_confirm_from_readiness(&running, &resolver)
        .await
        .expect("prepare group final confirm");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while starts.load(Ordering::SeqCst) == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "后台评审必须启动 provider"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(
        fixture
            .store
            .load_issue_cross_repo_review(PROJECT_ID, ISSUE_ID)
            .expect("load review")
            .is_none(),
        "gate 放行前评审不得落盘（构造「确认早于落盘」时序）"
    );

    // 用户在评审落盘之前完成最终确认（P2-1 时序）。
    let confirmed = fixture
        .engine
        .handle_final_confirm(PROJECT_ID, ISSUE_ID, &prepared.id)
        .await
        .expect("final confirm before review lands");
    assert_eq!(confirmed.status, CodingAttemptStatus::Completed);

    // 确认时刻评审记录不在场 → 确认侧无留痕可写（P2-1 缺陷起点）。
    assert!(
        fixture
            .store
            .list_issue_cross_repo_acknowledgements(PROJECT_ID, ISSUE_ID)
            .expect("acknowledgements")
            .is_empty(),
        "评审未落盘时确认侧不可能留痕"
    );

    // 评审以 request_changes 落盘。
    release.send(true).expect("release provider gate");
    let review = wait_for_review_record(&fixture.store).await;
    assert_eq!(review.verdict, ReviewVerdict::RequestChanges);

    // 落盘后补写留痕：最终存在且恰好 1 条，键 = (review.id, 被确认 attempt)。
    let acks = wait_for_acknowledgements(&fixture.store).await;
    assert_eq!(
        acks.len(),
        1,
        "确认早于评审落盘时，落盘后必须补写恰好一条留痕（P2-1）"
    );
    assert_eq!(acks[0].cross_repo_review_id, review.id);
    assert_eq!(acks[0].attempt_id, confirmed.id);
    assert_eq!(acks[0].verdict, ReviewVerdict::RequestChanges);

    // advisory 提醒事实关闭：注意力投影不再悬挂 cross_repo_review 待办。
    let items = crate::web::coding_attention_info::issue_coding_attention_items(
        &fixture.store.paths(),
        PROJECT_ID,
        ISSUE_ID,
    )
    .expect("attention items");
    assert!(
        items.iter().all(|item| item.kind != "cross_repo_review"),
        "留痕补写后跨仓评审提醒必须消失：{items:?}"
    );
}

/// P3-1：advisory 短时限必须覆盖 provider `start`（端到端 ≤ 时限）——
/// start 楔死（版本探测/握手/进程 spawn 无返回）也要按时限收敛为 `Failed`
/// 落盘、在飞登记随任务退出释放。修复前 start 在事件循环超时之外，
/// 评审任务挂在 start 上永不落盘（本用例 15s 轮询超时失败）。
#[tokio::test]
async fn cross_repo_review_start_hang_converges_to_failed_within_deadline() {
    let (_root, store, engine, attempts) = cross_repo_trigger_fixture(2);
    let provider = HangingStartProvider {
        starts: Arc::new(AtomicUsize::new(0)),
    };
    let starts = provider.starts.clone();
    let adapter: Arc<dyn StreamingProviderAdapter> = Arc::new(provider);
    let resolver =
        IssueCrossRepoReviewProviderResolver::new(move |_: &ProviderName| Some(adapter.clone()));

    engine
        .maybe_complete_issue_delivery(&attempts[1], &resolver)
        .await
        .expect("issue delivery completion");

    // start 楔死也必须按 advisory 短时限收敛为 Failed 落盘（测试态时限 1s，
    // 轮询宽限 15s）。
    let review = wait_for_review_record(&store).await;
    assert!(
        starts.load(Ordering::SeqCst) >= 1,
        "provider start 必须被调用"
    );
    assert_eq!(review.status, IssueCrossRepoReviewStatus::Failed);
    assert!(
        review
            .failure_reason
            .as_deref()
            .unwrap_or_default()
            .contains("启动超时"),
        "start 楔死必须收敛为「启动超时」Failed：{:?}",
        review.failure_reason
    );
}
