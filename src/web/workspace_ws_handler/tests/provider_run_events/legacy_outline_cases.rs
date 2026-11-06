//! P3（任务4，brief task-4）：legacy outline 三族臂（Legacy / OutlineRevision /
//! OutlineRebuild）的体积/行为门禁。
//!
//! - `raw_legacy_outline_future_respects_budget`：隔离子进程内量未装箱新臂
//!   `drive_legacy_outline_arm` 与 outline 启动/输出最大 child
//!   （`start_work_item_plan_author` / `drive_work_item_plan_provider_session_
//!   to_output` / `complete_work_item_plan_outline_author_from_output`），
//!   硬断言 ≤131072 B（设计 §2.2；量未 box 类型，不测 `Pin<Box<_>>` 指针）。
//! - `legacy_outline_stop_and_rebuilt_context_contract`：三种 variant 经真实
//!   spawn 链（`spawn_provider_run_from_event`→小外壳→drive→臂）收口：
//!   Legacy 首发、Revision 增量返修（feedback 移动）、Rebuild 全新启动
//!   （Box 载荷 `as_deref()` 借用 + snapshot 成功启动才 commit、失败不
//!   commit）、include:271-278 输入失败 Error 帧、AutoRevision 循环真实
//!   retry 后 HumanConfirm 收口、typed fresh 等待（FreshRunning 记录→显式
//!   fresh 完整返修提示词 + resume=None）、AuthorConfirm 正常完成；Stop 后
//!   零后续 start（无 followups/二次 finish）。
//!
//! 三守卫不可混用（r16 现场死锁根修，缺陷#10家族——设计 §3.4）：engine
//! MutexGuard 按值属 drive/当前臂；idle `provider_drive_guard` 是外层 Option
//! 的 `&mut`；`entity_run_lease` 是独立外层 binding（不传给臂）。

use super::stack_budget::{arm_drive, print_raw_baseline, stack_budget_fixture};
use super::*;

/// P3 legacy outline 臂/child 的未装箱体积硬预算（设计 §2.2：≤128 KiB 硬上限）。
const RAW_LEGACY_OUTLINE_BUDGET: usize = 131072;

/// 量测并硬断言单对象预算；失败格式逐字对齐 brief Step 2。
fn assert_raw_budget(symbol: &'static str, bytes: usize) {
    print_raw_baseline(symbol, bytes);
    assert!(
        bytes <= RAW_LEGACY_OUTLINE_BUDGET,
        "raw future {symbol} {bytes} exceeds {RAW_LEGACY_OUTLINE_BUDGET}\
         （P3 legacy outline 预算；profile/target/toolchain 见上方打印）"
    );
}

/// Rebuild 载荷构造（全字段 public，最小权威值）：inventory rendered 携带
/// 可断言标记，snapshot 携带独立 aggregate_index_id——provider start 成功后
/// commit 落盘的就是这份 payload（载荷忠实转递的运行期证据）。
fn case_rebuild_context(
    cwd: &std::path::Path,
) -> crate::product::logical_codebase::ResolvedPlanningContext {
    use crate::product::logical_codebase::planning_context::{
        MemberCheckoutFingerprint, PlanningContextSnapshot,
    };
    use crate::product::logical_codebase::planning_context_set::RepositoryContextResolution;
    use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
    use uuid::Uuid;
    let member = LogicalRepositoryId(Uuid::from_u128(0x00f3));
    let checkout = RepositoryCheckoutId(Uuid::from_u128(0x00f4));
    crate::product::logical_codebase::ResolvedPlanningContext {
        snapshot: PlanningContextSnapshot {
            schema_version: 1,
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            membership_revision: 41,
            effective_member_ids: vec![member],
            member_fingerprints: vec![MemberCheckoutFingerprint {
                logical_repository_id: member,
                checkout_id: checkout,
                revision: "0123456789abcdef0123456789abcdef01234567".to_string(),
                dirty: false,
                available: true,
            }],
            aggregate_index_id: "aggregate_index_p3_rebuild".to_string(),
            index_revision: 7,
            policy_digest: "sha256:p3-rebuild-policy".to_string(),
            working_directory: cwd.to_path_buf(),
            access_fingerprint: String::new(),
            invalidation: None,
            captured_at: "2026-11-06T00:00:00Z".to_string(),
        },
        cwd: cwd.to_path_buf(),
        inventory_injection: crate::product::logical_codebase::planning_context_set::InventoryInjection {
            rendered: "p3-rebuild-inventory-marker".to_string(),
        },
        aggregate_index_id: "aggregate_index_p3_rebuild".to_string(),
        policy_digest: "sha256:p3-rebuild-policy".to_string(),
        best_effort_readonly_status:
            crate::product::logical_codebase::planning_context_resolver::BestEffortReadonlyStatus::BestEffortConfigured,
        context_resolution: RepositoryContextResolution {
            set: Vec::new(),
            invalid_member_ids: Vec::new(),
            membership_revision: 41,
            member_checkout_paths: Default::default(),
        },
        invalidation: None,
    }
}

// ---------------------------------------------------------------------------
// P3 体积门禁（A 层，只构造不 poll；隔离子进程内执行——设计 §6.3）
// ---------------------------------------------------------------------------

/// raw-budget 子模式用例：legacy outline 臂与最大 child 硬断言全部在隔离
/// 子进程内执行（父不 poll 被测 future）；常规套件直通空壳。
#[tokio::test]
async fn provider_case_child_legacy_outline_raw_budget() {
    if std::env::var_os(super::small_stack_worker::PROVIDER_CASE_CHILD_MODE_ENV).is_none() {
        return;
    }
    run_legacy_outline_raw_budget_measurement().await;
}

/// P3 同步父用例（brief Step 1/2）：隔离子例逐对象量未装箱臂与 child。
#[test]
fn raw_legacy_outline_future_respects_budget() {
    super::small_stack_worker::run_isolated_provider_case(
        "provider_case_child_legacy_outline_raw_budget",
        std::time::Duration::from_secs(120),
    )
    .expect("legacy outline 臂与启动/输出 child 的未装箱体积必须全部≤131072（隔离子例证据）");
}

/// 逐对象构造未装箱 future 并硬断言（只构造不 poll，Drop 即释放夹具资源）。
async fn run_legacy_outline_raw_budget_measurement() {
    // —— 未装箱臂本体：三 variant 共用同一 future 类型（载荷是运行期值，
    //    不改类型布局）；以 Rebuild 载荷构造——`Option<Box<ResolvedPlanning
    //    Context>>` 按值入臂（P3 Box 边界，非 `Pin<Box<_>>` 指针假门禁）——
    {
        let (run_context, outbound_tx, run_cancel, command_rx, run_token) =
            stack_budget_fixture().await;
        let provider_for_run = run_context
            .provider_registry
            .get(&ProviderName::ClaudeCode)
            .expect("fixture provider");
        let engine_guard = run_context.engine.lock().await;
        let mut idle_guard = Some(
            run_context
                .workspace_runs
                .begin_provider_drive(&run_context.session_id),
        );
        let arm = super::run::drive_legacy_outline_arm(
            arm_drive(
                engine_guard,
                command_rx,
                provider_for_run,
                &run_context,
                &outbound_tx,
                &mut idle_guard,
                &run_cancel,
                run_token,
            ),
            None,
            false,
            Some(Box::new(case_rebuild_context(std::path::Path::new(
                "/tmp/p3-rebuild-cwd",
            )))),
        );
        assert_raw_budget("drive_legacy_outline_arm", std::mem::size_of_val(&arm));
        drop(arm);
    }

    // —— outline 启动/输出最大 child（真实 Legacy 夹具构造真实 launch/
    //    input/session，逐对象只构造不 poll）——
    let (fixture, _engine_rx) =
        ProviderRunFixture::new_with_engine_rx(WorkItemPlanFlowKind::Legacy);
    let provider_for_run: Arc<dyn StreamingProviderAdapter> = Arc::new(LegacyOutlineProvider {
        outputs: std::sync::Mutex::new(Vec::new()),
        starts: Arc::new(AtomicUsize::new(0)),
        inputs: mpsc::unbounded_channel().0,
        start_fails: false,
    });
    {
        // 启动 child：start_work_item_plan_author（唯一物理启动口）。
        let engine = fixture.engine.lock().await;
        let launch = crate::web::workspace_ws_handler::run::resolve_plan_author_launch(&engine)
            .expect("legacy plan launch");
        let input = engine
            .build_work_item_plan_streaming_input(
                crate::protocol::contracts::ProviderType::ClaudeCode,
                "p3 stack budget legacy prompt".to_string(),
                fixture.member_checkout_root().to_string_lossy().to_string(),
                ProviderName::ClaudeCode,
                crate::product::workspace_engine::PlanAuthorOutputContract::Structured,
            )
            .expect("legacy provider input");
        let start_child = crate::web::workspace_ws_handler::run::start_work_item_plan_author(
            launch,
            provider_for_run.clone(),
            input,
            CancellationToken::new(),
            None,
        );
        assert_raw_budget(
            "start_work_item_plan_author",
            std::mem::size_of_val(&start_child),
        );
        drop(start_child);
    }
    {
        // 输出驱动 child：drive_work_item_plan_provider_session_to_output。
        let mut engine = fixture.engine.lock().await;
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(1);
        drop((event_tx, _command_rx));
        let session = ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        };
        let (_tx, mut command_rx) = mpsc::channel(8);
        let drive_child = engine.drive_work_item_plan_provider_session_to_output(
            Ok(session),
            &mut command_rx,
            "stack_budget_legacy_node".to_string(),
            ProviderName::ClaudeCode,
        );
        assert_raw_budget(
            "drive_work_item_plan_provider_session_to_output",
            std::mem::size_of_val(&drive_child),
        );
        drop(drive_child);
    }
    {
        // 完成判定 child：complete_work_item_plan_outline_author_from_output。
        let mut engine = fixture.engine.lock().await;
        let complete_child =
            crate::web::workspace_ws_handler::run::complete_work_item_plan_outline_author_from_output(
                &mut engine,
                "",
            );
        assert_raw_budget(
            "complete_work_item_plan_outline_author_from_output",
            std::mem::size_of_val(&complete_child),
        );
        drop(complete_child);
    }
}

// ---------------------------------------------------------------------------
// P3 行为门禁：Stop 合同 / 三 variant 载荷 / Box 重建上下文（brief Step 1）
// ---------------------------------------------------------------------------

/// 记录 input 的 legacy outline provider：`outputs` 逐条按 start 轮次弹出
/// （每轮一条完整输出）；`start_fails=true` 时 start 直接 Err（Rebuild
/// snapshot commit 合同的失败支）。
struct LegacyOutlineProvider {
    outputs: std::sync::Mutex<Vec<String>>,
    starts: Arc<AtomicUsize>,
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
    start_fails: bool,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for LegacyOutlineProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let _ = self.inputs.send(input);
        if self.start_fails {
            return Err(ProviderAdapterError::provider_unavailable(
                "p3 planned legacy provider start failure".to_string(),
            ));
        }
        let output = self
            .outputs
            .lock()
            .expect("legacy outputs")
            .pop()
            .unwrap_or_else(|| "p3 legacy provider no more scripted output".to_string());
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    output, None,
                )))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("workspace runs use start")
    }
}

/// 有效单条 outline 的结构化输出全文（sentinel 包裹；通过
/// `WorkItemPlanOutlineValidator` → AuthorConfirm）。
fn case_valid_outline_output() -> String {
    let payload = serde_json::json!({
        "outline": {
            "id": "outline_p3_case",
            "project_id": "project_0001",
            "issue_id": "issue_0001",
            "source_story_spec_ids": ["story_0001"],
            "source_design_spec_ids": ["design_0001"],
            "strategy_summary": "P3 legacy outline case strategy",
            "work_item_outlines": [{
                "outline_id": "outline_case_a",
                "logical_work_item_id": "wi_case_a",
                "title": "Case A",
                "kind": "backend",
                "goal": "Case A goal",
                "scope": ["src/a.rs"],
                "non_goals": [],
                "estimated_context_tokens": 12000,
                "session_fit": "fits_single_agent_session",
                "source_story_spec_ids": ["story_0001"],
                "source_design_spec_ids": ["design_0001"],
                "exclusive_write_scopes": ["src/a.rs"],
                "forbidden_write_scopes": [],
                "depends_on": [],
                "verification_intent": ["cargo test --locked --lib a"],
                "trusted_verification_commands": [],
                "handoff_notes": "handoff case A"
            }],
            "risks": [],
            "handoff_strategy": "serial handoff",
            "status": "draft"
        }
    });
    structured_output_sentinel("p3-outline-nonce", &payload)
}

/// 直驱 `spawn_provider_run_from_event` 的 run_context（真实 spawn 链：
/// 准入→注册→小外壳→task body→boxed drive→legacy 臂）。
fn case_event_run_context(
    fixture: &ProviderRunFixture,
    provider: Arc<dyn StreamingProviderAdapter>,
) -> ProviderRunContext {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let mut run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        fixture.engine.clone(),
        fixture.workspace_runs.clone(),
        fixture.record.id.clone(),
        fixture.app_paths.clone(),
        fixture.record.clone(),
    );
    run_context.manager = fixture.manager.clone();
    run_context
}

/// 等待 manager 注册被 finish_run 清空（Stop 收口的可观测面）。
async fn case_wait_run_registration_cleared(
    manager: &Arc<crate::web::workspace_session::WorkspaceSessionManager>,
) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while manager.active_run().await.is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("run 注册必须被 finish_run 清空（ArmExit::Stop 收口）");
}

/// 观察窗口内不得再出现 provider input（Stop 后零后续 start / 无 followups）。
async fn case_assert_no_more_provider_inputs(
    input_rx: &mut mpsc::UnboundedReceiver<StreamingProviderInput>,
    note: &str,
) {
    if let Ok(Some(input)) =
        tokio::time::timeout(std::time::Duration::from_millis(150), input_rx.recv()).await
    {
        let head: String = input.prompt.chars().take(160).collect();
        panic!("{note}——意外 input prompt[..]={head}");
    }
}

/// 读取下一条 type=="error" 的出站帧消息文本。
async fn case_next_error_message(outbound_rx: &mut mpsc::Receiver<OutboundControl>) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let outbound = outbound_rx.recv().await.expect("outbound control expected");
            let OutboundControl::Text(json) = outbound else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_str(&json).expect("outbound json");
            if value["type"] == "error" {
                return value["message"].as_str().unwrap_or_default().to_string();
            }
        }
    })
    .await
    .expect("error outbound expected")
}

/// 观察窗口内不得出现 error 帧（静默退场分支的负锚）。
async fn case_assert_no_error_frame(outbound_rx: &mut mpsc::Receiver<OutboundControl>, note: &str) {
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(150), outbound_rx.recv()).await,
            Ok(Some(OutboundControl::Text(json)))
                if serde_json::from_str::<serde_json::Value>(&json)
                    .map(|value| value["type"] == "error")
                    .unwrap_or(false)
        ),
        "{note}"
    );
}

/// durable 会话记录 + 原地改写（json 同源直写，t17 先例）。
fn case_rewrite_session_record(
    fixture: &ProviderRunFixture,
    mutate: impl FnOnce(&mut crate::product::models::WorkspaceSessionRecord),
) {
    let mut record = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session");
    mutate(&mut record);
    let path = fixture
        .app_paths
        .issue_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", record.id));
    crate::product::json_store::write_json(&path, &record).expect("rewrite durable record");
}

/// P3 行为合同总用例（brief Step 1）：六个子场景各自独立夹具。
#[tokio::test]
async fn legacy_outline_stop_and_rebuilt_context_contract() {
    legacy_first_generation_starts_and_author_confirm_stops().await;
    revision_variant_moves_feedback_into_incremental_prompt().await;
    rebuild_variant_fresh_start_commits_snapshot_only_after_success().await;
    rebuild_start_failure_does_not_commit_snapshot().await;
    provider_input_failure_stops_with_error_frame().await;
    auto_revision_loop_retries_then_human_confirm_stops().await;
    typed_fresh_recovery_uses_fresh_invocation_without_resume().await;
    assert_legacy_outline_arm_borrows_boxed_rebuild_context();
}

/// Legacy 首发：真实 start、有效 outline → AuthorConfirm 收口（mark→drop
/// engine→idle take→finish_run），Stop 后零后续 start。
async fn legacy_first_generation_starts_and_author_confirm_stops() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::Legacy);
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LegacyOutlineProvider {
        outputs: std::sync::Mutex::new(vec![case_valid_outline_output()]),
        starts: starts.clone(),
        inputs: input_tx,
        start_fails: false,
    });
    let run_context = case_event_run_context(&fixture, provider);
    let manager = run_context.manager.clone();
    let (outbound_tx, mut outbound_rx) = mpsc::channel(64);

    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanLegacyAuthor,
        None,
        outbound_tx,
    )
    .await
    .expect("spawn legacy author run");
    case_wait_run_registration_cleared(&manager).await;

    // 首发恰一次 start（AuthorConfirm 首轮即收口，无 AutoRevision、无 followups）。
    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "Legacy 首发必须恰一次真实 start 后 AuthorConfirm 收口"
    );
    let input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("provider input expected")
        .expect("provider input channel open");
    assert!(!input.prompt.is_empty(), "Legacy 首发 prompt 必须真实构造");
    // Stop 后零后续 start（不落 followups、不二次 finish）。
    case_assert_no_more_provider_inputs(&mut input_rx, "AuthorConfirm Stop 后不得再燃 followups")
        .await;
    case_assert_no_error_frame(&mut outbound_rx, "AuthorConfirm 正常收口不得发 error 帧").await;
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let _ = fixture.engine.lock().await;
    })
    .await
    .expect("AuthorConfirm Stop 后 engine 锁必须可重取（guard 按值释放）");
    // durable 进展：timeline 节点落盘（prompt 事件 + 节点完成）。
    let nodes = fixture
        .lifecycle
        .load_timeline_nodes(&fixture.record.id)
        .expect("durable timeline nodes");
    assert!(
        !nodes.is_empty(),
        "Legacy 首发必须留下 durable timeline 进展"
    );
}

/// Revision variant：feedback 精确移动进增量返修 prompt（不重复完整上下文）。
async fn revision_variant_moves_feedback_into_incremental_prompt() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::Legacy);
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LegacyOutlineProvider {
        outputs: std::sync::Mutex::new(vec![case_valid_outline_output()]),
        starts: starts.clone(),
        inputs: input_tx,
        start_fails: false,
    });
    let run_context = case_event_run_context(&fixture, provider);
    let manager = run_context.manager.clone();
    let (outbound_tx, _outbound_rx) = mpsc::channel(64);

    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanOutlineRevision {
            feedback: Some("p3-revision-feedback-marker".to_string()),
        },
        None,
        outbound_tx,
    )
    .await
    .expect("spawn outline revision run");
    case_wait_run_registration_cleared(&manager).await;

    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "Revision variant 必须恰一次真实 start"
    );
    let input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("revision input expected")
        .expect("revision input channel open");
    assert!(
        input.prompt.contains("p3-revision-feedback-marker"),
        "Revision feedback 必须移动进增量返修 prompt：{}",
        input.prompt.chars().take(120).collect::<String>()
    );
    case_assert_no_more_provider_inputs(
        &mut input_rx,
        "Revision AuthorConfirm Stop 后不得再燃 followups",
    )
    .await;
}

/// Rebuild variant：Box 载荷经 `as_deref()` 借用注入 prompt（inventory 标记），
/// 全新启动（resume=None），provider start 成功且完成输出后 snapshot 才 commit。
async fn rebuild_variant_fresh_start_commits_snapshot_only_after_success() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::Legacy);
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LegacyOutlineProvider {
        outputs: std::sync::Mutex::new(vec![case_valid_outline_output()]),
        starts: starts.clone(),
        inputs: input_tx,
        start_fails: false,
    });
    let run_context = case_event_run_context(&fixture, provider);
    let manager = run_context.manager.clone();
    let (outbound_tx, _outbound_rx) = mpsc::channel(64);
    let rebuild = case_rebuild_context(&fixture.member_checkout_root());

    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanOutlineRebuild {
            rebuilt: Box::new(rebuild.clone()),
        },
        None,
        outbound_tx,
    )
    .await
    .expect("spawn outline rebuild run");
    case_wait_run_registration_cleared(&manager).await;

    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "Rebuild variant 必须恰一次真实 start（全新启动）"
    );
    let input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("rebuild input expected")
        .expect("rebuild input channel open");
    // as_deref() 借现 Box：rebuilt 的 inventory 渲染进 prompt（载荷忠实转递）。
    assert!(
        input.prompt.contains("p3-rebuild-inventory-marker"),
        "Rebuild 载荷必须经 as_deref() 借用注入 prompt：{}",
        input.prompt.chars().take(120).collect::<String>()
    );
    // 全新启动：不携带 resume 会话 id。
    assert_eq!(
        input.resume_provider_session_id, None,
        "Rebuild 必须全新启动（resume=None，不回捞旧 native）"
    );
    // provider start 成功且输出完成 → snapshot 已 commit（内容＝构造载荷）。
    let committed = crate::product::logical_codebase::PlanningContextSnapshotStore::new(
        fixture.app_paths.clone(),
    )
    .load("project_0001", "issue_0001")
    .expect("load planning snapshot")
    .expect("provider start 成功后 rebuilt snapshot 必须 commit");
    assert_eq!(
        committed.aggregate_index_id, "aggregate_index_p3_rebuild",
        "commit 的 snapshot 必须是 Rebuild 载荷本身"
    );
    assert_eq!(
        committed.effective_member_ids, rebuild.snapshot.effective_member_ids,
        "commit 的 snapshot 成员面必须与构造载荷一致"
    );
    case_assert_no_more_provider_inputs(
        &mut input_rx,
        "Rebuild AuthorConfirm Stop 后不得再燃 followups",
    )
    .await;
}

/// Rebuild 失败支：provider start 失败 → snapshot 不 commit（重连仍判
/// StaleContext，避免 TOCTOU）；驱动失败分支原序收口（mark 后 Stop）。
async fn rebuild_start_failure_does_not_commit_snapshot() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::Legacy);
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, _input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LegacyOutlineProvider {
        outputs: std::sync::Mutex::new(Vec::new()),
        starts: starts.clone(),
        inputs: input_tx,
        start_fails: true,
    });
    let run_context = case_event_run_context(&fixture, provider);
    let manager = run_context.manager.clone();
    let (outbound_tx, mut outbound_rx) = mpsc::channel(64);

    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanOutlineRebuild {
            rebuilt: Box::new(case_rebuild_context(&fixture.member_checkout_root())),
        },
        None,
        outbound_tx,
    )
    .await
    .expect("spawn failing rebuild run");
    // 原分支收口形态：mark_active_run_finished 后 Stop（该分支不 finish_run，
    // 注册留待接替/清理——原 include:319-336 行为，不补通用 finish）。
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while starts.load(Ordering::SeqCst) < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("failing rebuild must attempt provider start");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(starts.load(Ordering::SeqCst), 1, "失败支恰一次 start 尝试");
    let snapshot = crate::product::logical_codebase::PlanningContextSnapshotStore::new(
        fixture.app_paths.clone(),
    )
    .load("project_0001", "issue_0001")
    .expect("load planning snapshot store");
    assert!(
        snapshot.is_none(),
        "provider start 失败不得 commit rebuilt snapshot（避免再次 TOCTOU）"
    );
    // 该分支不发 error 帧（原 include 驱动失败分支无 Error 出站）。
    case_assert_no_error_frame(&mut outbound_rx, "驱动失败分支不发 error 帧（原序）").await;
    // 清理：abort 清空注册（原分支不 finish_run）。
    let _ = manager.abort_active_run().await;
}

/// include:271-278 输入失败：基线 fail-closed（REQ-PIB-02）→ mark→drop
/// engine→Error 帧→Stop；拒收场景零 start。
async fn provider_input_failure_stops_with_error_frame() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::Legacy);
    // 基线不可解析：issue.base_branch 指向不存在分支 → 输入构造 fail-closed。
    let issue_path = fixture
        .app_paths
        .project_root("project_0001")
        .join("issues")
        .join("issue_0001")
        .join("issue.json");
    let mut issue = crate::product::issue_store::IssueStore::new(fixture.app_paths.clone())
        .get("project_0001", "issue_0001")
        .expect("load fixture issue");
    issue.base_branch = Some("p3-missing-baseline-branch".to_string());
    crate::product::json_store::write_json(&issue_path, &issue).expect("rewrite issue baseline");

    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LegacyOutlineProvider {
        outputs: std::sync::Mutex::new(Vec::new()),
        starts: starts.clone(),
        inputs: input_tx,
        start_fails: false,
    });
    let run_context = case_event_run_context(&fixture, provider);
    let (outbound_tx, mut outbound_rx) = mpsc::channel(64);

    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanLegacyAuthor,
        None,
        outbound_tx,
    )
    .await
    .expect("spawn baseline-failing legacy run");
    let message = case_next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("p3-missing-baseline-branch"),
        "输入失败 Error 帧必须携带基线诊断：{message}"
    );
    // 拒收场景零 start：Error 帧（收口信号）之后不得再出现任何 start/input。
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(
        starts.load(Ordering::SeqCst),
        0,
        "输入构造失败必须零 provider start"
    );
    case_assert_no_more_provider_inputs(&mut input_rx, "输入失败 Stop 后零后续 start").await;
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let _ = fixture.engine.lock().await;
    })
    .await
    .expect("输入失败 Stop 后 engine 锁必须可重取");
    // 清理：原分支不 finish_run，注册由 abort 清空。
    let _ = fixture.manager.abort_active_run().await;
}

/// AutoRevision 循环：不可解析输出 → 首轮 AutoRevision（真实 retry start、
/// 每轮新 split handle）→ 次轮 HumanConfirm（engine 内部 retry 计数 ≥2 先于
/// revision_iterations>5 硬兜底触发——原 include 行为面，不缩窄）。收口后
/// Stop（finish 次序保持），零后续 start。
async fn auto_revision_loop_retries_then_human_confirm_stops() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::Legacy);
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    // 两条不可解析输出（无 sentinel）：round1 → AutoRevision；round2 →
    // HumanConfirm（outline_output_parse_failed，engine 计数≥2）。
    let provider = Arc::new(LegacyOutlineProvider {
        outputs: std::sync::Mutex::new(vec![
            "p3 unparseable legacy output two".to_string(),
            "p3 unparseable legacy output one".to_string(),
        ]),
        starts: starts.clone(),
        inputs: input_tx,
        start_fails: false,
    });
    let run_context = case_event_run_context(&fixture, provider);
    let manager = run_context.manager.clone();
    let (outbound_tx, mut outbound_rx) = mpsc::channel(64);

    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanLegacyAuthor,
        None,
        outbound_tx,
    )
    .await
    .expect("spawn auto-revision legacy run");
    case_wait_run_registration_cleared(&manager).await;

    // 恰两次 start：首发 + AutoRevision 每轮新 split handle 的 retry；次轮
    // HumanConfirm 收口（不再第三轮——engine 内部 work_item_plan_author_
    // retry_count≥2 的既有上限先于 revision_iterations>5 硬兜底）。
    assert_eq!(
        starts.load(Ordering::SeqCst),
        2,
        "AutoRevision 循环必须恰「首发 + 1 轮 retry」两次真实 start 后 HumanConfirm 收口"
    );
    let retry_input = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let mut last = None;
        while let Ok(input) = input_rx.try_recv() {
            last = Some(input);
        }
        last
    })
    .await
    .expect("collect inputs");
    let retry_prompt = retry_input.expect("retry input present").prompt;
    assert!(
        retry_prompt.contains("增量返修")
            && retry_prompt.contains("Provider did not return a valid WorkItemPlan Outline"),
        "retry 轮必须是携带上轮 parse findings 的增量返修提示词：{}",
        retry_prompt.chars().take(160).collect::<String>()
    );
    // HumanConfirm 收口分支：mark→drop engine→idle take→finish_run（注册已
    // 清空）→静默 Stop（无 Error 帧）。
    case_assert_no_error_frame(&mut outbound_rx, "HumanConfirm 收口不得发 error 帧").await;
    case_assert_no_more_provider_inputs(
        &mut input_rx,
        "HumanConfirm Stop 后不得再燃 followups（不第三轮 start）",
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let _ = fixture.engine.lock().await;
    })
    .await
    .expect("AutoRevision 收口后 engine 锁必须可重取");
}

/// typed fresh 等待：FreshRunning + LegacyOutlineAuthor 记录 → 显式 fresh
/// 完整返修提示词（prompt 事件标签）+ 全新启动（resume=None）。
async fn typed_fresh_recovery_uses_fresh_invocation_without_resume() {
    let (fixture, engine_rx) = ProviderRunFixture::new_with_engine_rx(WorkItemPlanFlowKind::Legacy);
    // FreshRunning 恢复记录（legacy_outline_author tag）→ 臂内 typed fresh 支。
    case_rewrite_session_record(&fixture, |record| {
        record.lc_resume_recovery = Some(crate::product::models::LcResumeRecoveryRecord {
            schema_version: 1,
            recovery_id: "recovery_p3_fresh".to_string(),
            failed_provider_node_id: "timeline_node_p3".to_string(),
            reason_code: "stale_context".to_string(),
            role: crate::product::models::ProviderConversationRole::Author,
            provider: ProviderName::ClaudeCode,
            superseded_native_session_id: None,
            previous_fingerprint_digest: Some("sha256:p3-old".to_string()),
            previous_session_status: crate::product::models::WorkspaceSessionStatus::Running,
            previous_stage: "author_confirm".to_string(),
            run_context: crate::product::models::LcRecoveryRunContext::LegacyOutlineAuthor {
                business: crate::product::models::LcRecoveryBusinessContext {
                    entity_id: fixture.record.entity_id.clone(),
                    plan_id: None,
                    workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
                    business_repository_physical_id: None,
                    business_repository_logical_id: None,
                    business_checkout_id: None,
                    business_repository_path: None,
                    source_story_ids: Vec::new(),
                    source_design_ids: Vec::new(),
                    plan_options_ref: None,
                    artifact_refs: Vec::new(),
                    review_refs: Vec::new(),
                    author_provider: ProviderName::ClaudeCode,
                    effective_reviewer_provider: Some(ProviderName::Codex),
                    original_node_id: Some("timeline_node_p3".to_string()),
                    original_role_run_seq: None,
                },
                flow: WorkItemPlanFlowKind::Legacy,
                original_provider_node_id: Some("timeline_node_p3".to_string()),
                outline_ref: None,
            },
            state: crate::product::models::LcResumeRecoveryState::FreshRunning,
            operation_id: Some("op_p3_fresh".to_string()),
            fresh_node_id: None,
            fresh_native_session_id: None,
            fresh_role_run_seq: None,
            fresh_operations: Vec::new(),
            detected_at: "2026-11-06T00:00:00Z".to_string(),
            updated_at: "2026-11-06T00:00:00Z".to_string(),
        });
    });
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, _input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LegacyOutlineProvider {
        outputs: std::sync::Mutex::new(vec![case_valid_outline_output()]),
        starts: starts.clone(),
        inputs: input_tx,
        start_fails: false,
    });
    let run_context = case_event_run_context(&fixture, provider);
    let manager = run_context.manager.clone();
    let (outbound_tx, mut outbound_rx) = mpsc::channel(64);
    let forward = spawn_engine_event_forward_task(engine_rx, outbound_tx.clone(), None);

    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanLegacyAuthor,
        None,
        outbound_tx,
    )
    .await
    .expect("spawn typed-fresh legacy run");
    case_wait_run_registration_cleared(&manager).await;

    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "typed fresh 支必须恰一次真实 start"
    );
    // 出站帧含「显式 fresh 完整返修提示词」标签（fresh 分支选择的可观测面）。
    let saw_fresh_label = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let outbound = outbound_rx.recv().await.expect("outbound expected");
            let OutboundControl::Text(json) = outbound else {
                continue;
            };
            if json.contains("显式 fresh 完整返修提示词") {
                break true;
            }
        }
    })
    .await
    .expect("typed fresh 必须发显式 fresh 完整返修提示词事件");
    assert!(saw_fresh_label);
    forward.abort();
    let _ = forward.await;
}

/// 结构门禁（P3 Box 边界）：`drive_legacy_outline_arm` 以 `as_deref()` 借现
/// Box，不得出现裸 `(**rebuilt).clone()` 副本（运行期读源断言，large_file_
/// guard 同型）。迁移前本断言红（文件/符号缺失）——这是 P3 的新增性质，
/// 其余子场景为既有行为 characterization（已绿不冒称红）。
fn assert_legacy_outline_arm_borrows_boxed_rebuild_context() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/web/workspace_ws_handler/run/provider_run/legacy_outline.rs");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 legacy_outline.rs 失败（P3 未完成迁移）：{error}"));
    assert!(
        source.contains("rebuild_context.as_deref()"),
        "legacy 臂必须以 as_deref() 借用 Box 载荷（设计 §3.3 rebuild_context 行）"
    );
    // 只检查代码行（注释里的历史说明不构成 clone 副本）。
    let code = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("(**rebuilt).clone()"),
        "legacy 臂代码不得保留裸 ResolvedPlanningContext clone 副本"
    );
}
