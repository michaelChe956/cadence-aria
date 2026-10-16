//! 批 2 P3（REQ-WIGA-02/12）定向测试：自动 compile 臂的确认 trust 停点与
//! compile 恢复门对 N0 attention 事实的写/清。
//!
//! - trust 非 Ready：自动臂零 compile 副作用 + 落人工确认门 + 对精确绑定
//!   enabled enrollment 的会话 upsert plan_question(stage=plan)——未开页面
//!   也可直接读 N0 store 断言通知事实；trust Ready 时 resolve 后自动
//!   Confirmed；同 key 重复 upsert 不增项、occurred_at 首次冻结。
//! - compile 恢复门打开：focus 取真实 gate（durable compile transaction 的
//!   compile_id，不伪造）upsert；恢复动作成功关门即 resolve；Continue 复用
//!   原 transaction，不重跑已提交 compile。
//! - 手动 plan 零写入（无 enrollment／错绑／手动确认入口三面）；事实写失败
//!   经 EngineEvent::Error 上抛；finalizer 原值回归（auto WI 确认快照 false、
//!   manual true 仍拦）。

use super::*;

use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, EnrollmentTarget, LogicalCodebaseManifest,
    LogicalCodebaseProviderGateway, LogicalCodebaseStore, LogicalRepositoryId,
    ProviderTrustPrecondition, ProviderTrustPreparationResult, ProviderTrustWaiting,
    legacy_logical_codebase_id,
};
use crate::product::models::automation::{
    AutomationAttentionFact, AutomationAttentionKind, AutomationAttentionStage, EnrollmentOptions,
    EnrollmentSource, EnrollmentWriteCommand, PlanApprovalPolicy, SourceRevisionRef,
};
use crate::product::models::lifecycle::IssueWorkItemPlanOptions;
use crate::product::models::provider::ProviderName;
use crate::product::workspace_engine::conversational_gate::ConfirmationTrustContext;
use crate::product::work_item_plan_policy::RunPolicy;
use std::collections::VecDeque;
use std::path::PathBuf;
use parking_lot::Mutex;
use std::sync::Arc;

/// 可编排 Waiting/Ready 的 trust precondition fake（无锁 fake，默认 Ready；
/// `ensure_ready_cancellable` 走 trait 默认实现直调本方法）。
#[derive(Default)]
struct B2P3ScriptedTrustGate {
    scripted: Mutex<VecDeque<ProviderTrustPreparationResult>>,
}

impl B2P3ScriptedTrustGate {
    fn script_waiting(&self, reason_code: &str) {
        self.scripted.lock().push_back(
            ProviderTrustPreparationResult::Waiting {
                waiting: ProviderTrustWaiting {
                    provider: ProviderName::Codex,
                    canonical_root: PathBuf::from("/b2-p3-fixture-root"),
                    trust_key: "b2-p3-fixture-trust-key".to_string(),
                    reason_code: reason_code.to_string(),
                    message: "b2-p3 fixture 等待".to_string(),
                    retry_action: "修复后重新确认".to_string(),
                    manual_guidance: "b2-p3 fixture 指导".to_string(),
                    command_markdown: None,
                    recorded_at: "2026-10-16T00:00:00Z".to_string(),
                },
            },
        );
    }
}

impl ProviderTrustPrecondition for B2P3ScriptedTrustGate {
    fn ensure_ready(
        &self,
        _project_id: &str,
        _operation_id: &str,
        _lc_id: &str,
        _canonical_root: &std::path::Path,
        _providers: &[ProviderName],
        _scenario: crate::product::logical_codebase::provider_trust::ProviderTrustScenario,
    ) -> ProviderTrustPreparationResult {
        self.scripted
            .lock()
            .pop_front()
            .unwrap_or(ProviderTrustPreparationResult::Ready {
                registrations: Vec::new(),
            })
    }
}

/// presence-only gateway（复用 part_32 的测试桩）：确认链只把 gateway 用作
/// 「LC 会话」谓词，本测试不触达其 capability/launch 面。
fn b2_p3_presence_gateway(
    app_paths: &ProductAppPaths,
    authority_root: PathBuf,
) -> Arc<LogicalCodebaseProviderGateway> {
    Arc::new(LogicalCodebaseProviderGateway::new(
        AggregatePolicyArtifactStore::new(app_paths.clone()),
        Arc::new(ReviewStaticCapabilitySource::default()),
        Arc::new(ReviewPassThroughTargetResolver),
        Arc::new(crate::cross_cutting::provider_registry::ProviderRegistry::new()),
        Arc::new(ReviewStubSyncAdapter),
        review_always_available_gate(),
        authority_root,
    ))
}

struct B2P3TrustFixture {
    _root: tempfile::TempDir,
    lifecycle: LifecycleStore,
    app_paths: ProductAppPaths,
    trust: Arc<B2P3ScriptedTrustGate>,
    engine: WorkspaceEngine,
    event_rx: mpsc::Receiver<EngineEvent>,
}

/// LC 会话 + 可编排 trust precondition + Codex author（requires_workspace_trust）
/// + 目标 run policy 的 compile 就绪引擎。legacy scope manifest 与冻结根同源，
/// Context 漂移重验全部通过，trust 判定完全由 scripted precondition 决定。
fn b2_p3_trust_engine(policy: RunPolicy) -> B2P3TrustFixture {
    let (root, lifecycle, _plan_id, base) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let app_paths = lifecycle.app_paths();
    let worktree = root.path().join("b2-p3-aggregate-root");
    std::fs::create_dir_all(&worktree).expect("create aggregate root");
    let registry_lc_id = legacy_logical_codebase_id("project_0001");
    LogicalCodebaseStore::for_lc(app_paths.clone(), registry_lc_id)
        .save_manifest(
            "project_0001",
            &LogicalCodebaseManifest::new("project_0001", worktree.clone(), Vec::new()),
        )
        .expect("save legacy-scope LC manifest");

    // 会话重写：author=Codex（需要 workspace trust 的 provider）、reviewer 置
    // 空、目标 run policy 落 durable，再以持有事件接收端的引擎重建。
    let mut record = lifecycle
        .get_workspace_session(&base.session().session_id)
        .expect("load fixture session");
    record.author_provider = ProviderName::Codex;
    record.reviewer_provider = None;
    record.run_policy = policy;
    crate::product::json_store::write_json(
        &app_paths
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist rewired session");

    let (event_tx, event_rx) = mpsc::channel(64);
    let mut session = WorkspaceSession::from_record(record);
    session.artifact = base.session.artifact.clone();
    let engine = WorkspaceEngine::new_persistent(
        base.checkpoint_store.clone(),
        lifecycle.clone(),
        event_tx,
        session,
    );
    let trust = Arc::new(B2P3ScriptedTrustGate::default());
    let engine = engine
        .with_logical_provider_gateway(b2_p3_presence_gateway(&app_paths, worktree.clone()))
        .with_confirmation_trust_context(ConfirmationTrustContext {
            precondition: trust.clone(),
            issue_lc_id: None,
            frozen_root: worktree,
        });
    B2P3TrustFixture {
        _root: root,
        lifecycle,
        app_paths,
        trust,
        engine,
        event_rx,
    }
}

struct B2P3PlainFixture {
    _root: tempfile::TempDir,
    lifecycle: LifecycleStore,
    app_paths: ProductAppPaths,
    engine: WorkspaceEngine,
}

/// 无 gateway 的就绪引擎（非 LC 会话：确认 trust 免登记零调用直接放行，
/// legacy compile 仓库路由不受 LC 会话谓词影响——用于 compile/自动确认
/// 路径的定向测试）。
fn b2_p3_plain_engine(policy: RunPolicy) -> B2P3PlainFixture {
    let (root, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let app_paths = lifecycle.app_paths();
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load fixture session");
    record.run_policy = policy;
    crate::product::json_store::write_json(
        &app_paths
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist run policy");
    engine.session.run_policy = policy;
    B2P3PlainFixture {
        _root: root,
        lifecycle,
        app_paths,
        engine,
    }
}

/// enable＋bind：构造精确绑定本 (plan, session) 的 enabled enrollment，
/// 返回 N0 store（测试断言直读）。
fn b2_p3_enable_and_bind(
    app_paths: &ProductAppPaths,
    plan_id: &str,
    session_id: &str,
) -> IssueAutomationStore {
    let store = IssueAutomationStore::new(app_paths.clone());
    let command = EnrollmentWriteCommand::Enable {
        selection_key: "b2-p3-selection".to_string(),
        source: EnrollmentSource {
            stories: vec![SourceRevisionRef {
                id: "story_spec_0001".to_string(),
                version: 1,
            }],
            designs: vec![],
        },
        options: EnrollmentOptions {
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            plan_approval: PlanApprovalPolicy::Auto,
            plan_options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
        },
        target: EnrollmentTarget::LogicalCodebase {
            logical_codebase_id: "lc_b2_p3_fixture".to_string(),
            logical_repository_id: LogicalRepositoryId(
                uuid::Uuid::parse_str("00000001-0000-0000-0000-000000000000").unwrap(),
            ),
        },
    };
    let created = store
        .compare_and_set("project_0001", "issue_0001", None, command)
        .expect("enable enrollment");
    store
        .bind_plan(
            "project_0001",
            "issue_0001",
            created.policy_revision,
            plan_id,
            session_id,
        )
        .expect("bind enrollment to session");
    store
}

fn b2_p3_facts(store: &IssueAutomationStore) -> Vec<crate::product::models::automation::AutomationAttentionFact> {
    store
        .get("project_0001", "issue_0001")
        .expect("read enrollment")
        .expect("enrollment present")
        .attention_facts
}

fn b2_p3_collect_errors(event_rx: &mut mpsc::Receiver<EngineEvent>) -> Vec<String> {
    let mut errors = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        if let EngineEvent::Error { message } = event {
            errors.push(message);
        }
    }
    errors
}

/// 红测①：trust 非 Ready 不 compile，落 typed 停点并持久 plan_question
/// （未开页面仍可通知——直接读 N0 store 断言）。
#[tokio::test]
async fn b2_p3_trust_waiting_persists_plan_question_and_skips_compile() {
    let mut fixture = b2_p3_trust_engine(RunPolicy::AutoIfValid);
    let plan_id = fixture.engine.session().entity_id.clone();
    let session_id = fixture.engine.session().session_id.clone();
    let store = b2_p3_enable_and_bind(&fixture.app_paths, &plan_id, &session_id);
    fixture.trust.script_waiting("b2_p3_fixture_home_failure");

    fixture.engine.enter_policy_valid_work_item_plan_compile().await;

    // 零 compile 副作用：不建 compile/恢复节点；会话落人工确认门兜底。
    let nodes = fixture
        .lifecycle
        .load_timeline_nodes_for_issue_session("project_0001", "issue_0001", &session_id)
        .expect("timeline nodes");
    assert!(
        nodes.iter().all(|node| {
            node.node_type != TimelineNodeType::WorkItemPlanCompile
                && node.node_type != TimelineNodeType::WorkItemPlanCompileRecovery
        }),
        "trust 非 Ready 不得有任何 compile 节点：{nodes:?}"
    );
    assert_eq!(fixture.engine.session().stage, WorkspaceStage::HumanConfirm);
    let durable = fixture
        .lifecycle
        .get_workspace_session(&session_id)
        .expect("durable session");
    assert_eq!(durable.status, WorkspaceSessionStatus::WaitingForHuman);

    // 未开页面也可读：直接读 N0 store 断言持久事实。
    let facts = b2_p3_facts(&store);
    let expected_key = format!("{plan_id}:{session_id}:confirmation_trust_waiting");
    assert_eq!(facts.len(), 1, "恰一条停点事实：{facts:?}");
    let fact = &facts[0];
    assert_eq!(fact.key, expected_key);
    assert_eq!(fact.kind, AutomationAttentionKind::PlanQuestion);
    assert_eq!(fact.stage, AutomationAttentionStage::Plan);
    assert_eq!(fact.plan_id.as_deref(), Some(plan_id.as_str()));
    assert_eq!(fact.session_id.as_deref(), Some(session_id.as_str()));
    assert_eq!(fact.reason_code, "b2_p3_fixture_home_failure");
    assert_eq!(fact.gate_id, None, "trust 停点无 durable 门 id，不伪造 focus");
    assert_eq!(fact.attempt_id, None);
    assert!(!fact.occurred_at.is_empty());
}

/// 红测④：同 key 重复停等不增项、occurred_at 首次冻结（N0 upsert 幂等
/// 语义经 P3 写者贯通）。
#[tokio::test]
async fn b2_p3_trust_waiting_repeats_stably_with_frozen_occurred_at() {
    let mut fixture = b2_p3_trust_engine(RunPolicy::AutoIfValid);
    let plan_id = fixture.engine.session().entity_id.clone();
    let session_id = fixture.engine.session().session_id.clone();
    let store = b2_p3_enable_and_bind(&fixture.app_paths, &plan_id, &session_id);
    let expected_key = format!("{plan_id}:{session_id}:confirmation_trust_waiting");

    // 第一次停等：落事实。
    fixture.trust.script_waiting("b2_p3_fixture_home_failure");
    fixture.engine.enter_policy_valid_work_item_plan_compile().await;
    let first = b2_p3_facts(&store);
    assert_eq!(first.len(), 1, "停等落一条事实：{first:?}");
    let occurred_at = first[0].occurred_at.clone();

    // 重复停等：同 key 不增项、occurred_at 首次冻结。
    fixture.trust.script_waiting("b2_p3_fixture_home_failure");
    fixture.engine.enter_policy_valid_work_item_plan_compile().await;
    let second = b2_p3_facts(&store);
    assert_eq!(second.len(), 1, "同 key 重复 upsert 不增项");
    assert_eq!(second[0].key, expected_key);
    assert_eq!(second[0].occurred_at, occurred_at, "occurred_at 首次冻结");
}

/// 红测②：trust Ready（非 LC 会话免登记直接放行）后 resolve 既有停点事实
/// 并自动 Confirmed；auto WI 确认快照 false（finalizer 原值回归不改）。
#[tokio::test]
async fn b2_p3_trust_ready_resolves_fact_and_auto_confirms() {
    let mut fixture = b2_p3_plain_engine(RunPolicy::AutoIfValid);
    let plan_id = fixture.engine.session().entity_id.clone();
    let session_id = fixture.engine.session().session_id.clone();
    let store = b2_p3_enable_and_bind(&fixture.app_paths, &plan_id, &session_id);

    // 预置既有停点事实（模拟此前 trust 停等落下的通知）——Ready 后必须被
    // resolve 消失。
    let binding = store
        .get("project_0001", "issue_0001")
        .unwrap()
        .unwrap()
        .binding_history
        .expect("binding history after bind")
        .current;
    store
        .upsert_automation_attention_fact(
            "project_0001",
            "issue_0001",
            &binding,
            AutomationAttentionFact {
                key: format!("{plan_id}:{session_id}:confirmation_trust_waiting"),
                kind: AutomationAttentionKind::PlanQuestion,
                stage: AutomationAttentionStage::Plan,
                plan_id: Some(plan_id.clone()),
                session_id: Some(session_id.clone()),
                attempt_id: None,
                reason_code: "previously_waiting".to_string(),
                occurred_at: "2026-10-16T00:00:00Z".to_string(),
                choice_id: None,
                gate_id: None,
            },
        )
        .expect("pre-seed waiting fact");
    assert_eq!(b2_p3_facts(&store).len(), 1);

    fixture.engine.enter_policy_valid_work_item_plan_compile().await;

    let durable = fixture
        .lifecycle
        .get_workspace_session(&session_id)
        .expect("durable session");
    assert_eq!(
        durable.status,
        WorkspaceSessionStatus::Confirmed,
        "Ready resolve 后自动 Confirmed"
    );
    assert_eq!(fixture.engine.session().stage, WorkspaceStage::Completed);
    assert!(
        b2_p3_facts(&store).is_empty(),
        "Ready 后停点事实必须 resolve 消失"
    );

    // auto 授权口径：物化 WI 的执行确认快照 false（finalizer 原值回归不改）。
    let work_items = fixture
        .lifecycle
        .list_work_items("project_0001", "issue_0001")
        .expect("list work items");
    let compiled: Vec<_> = work_items
        .iter()
        .filter(|wi| wi.source_work_item_plan_id.as_deref() == Some(plan_id.as_str()))
        .collect();
    assert!(!compiled.is_empty(), "compile 已物化 WI");
    assert!(
        compiled.iter().all(|wi| !wi.require_execution_plan_confirm),
        "auto WI 确认快照 false：{compiled:?}"
    );
}

/// oracle 提示①回归钉：只对精确绑定 enabled enrollment 的会话写事实——
/// 手动 plan（无 enrollment）、错绑 enrollment、手动确认入口三面零写入。
#[tokio::test]
async fn b2_p3_manual_and_misbound_plans_write_no_attention_fact() {
    // 面一：无 enrollment 的自动臂停等——零写入。
    {
        let mut fixture = b2_p3_trust_engine(RunPolicy::AutoIfValid);
        fixture.trust.script_waiting("b2_p3_fixture_home_failure");
        fixture.engine.enter_policy_valid_work_item_plan_compile().await;
        assert_eq!(fixture.engine.session().stage, WorkspaceStage::HumanConfirm);
        let store = IssueAutomationStore::new(fixture.app_paths.clone());
        assert_eq!(
            store.get("project_0001", "issue_0001").unwrap(),
            None,
            "无 enrollment 的手动 plan 零写入"
        );
    }
    // 面二：enrollment 绑到别的 session——错绑零写入。
    {
        let mut fixture = b2_p3_trust_engine(RunPolicy::AutoIfValid);
        let plan_id = fixture.engine.session().entity_id.clone();
        b2_p3_enable_and_bind(&fixture.app_paths, &plan_id, "workspace_session_foreign");
        fixture.trust.script_waiting("b2_p3_fixture_home_failure");
        fixture.engine.enter_policy_valid_work_item_plan_compile().await;
        assert!(
            b2_p3_facts(&IssueAutomationStore::new(fixture.app_paths.clone())).is_empty(),
            "错绑会话零写入"
        );
    }
    // 面三：手动确认入口（auto_confirm=false）不经 P3 停点，正常 compile 后
    // 等待人工确认——即使绑定存在也不写任何事实。
    {
        let mut fixture = b2_p3_plain_engine(RunPolicy::Interactive);
        let plan_id = fixture.engine.session().entity_id.clone();
        let session_id = fixture.engine.session().session_id.clone();
        b2_p3_enable_and_bind(&fixture.app_paths, &plan_id, &session_id);
        // 手动臂不设 trust 前置（非 LC 会话本就免登记），即使 trust 未就绪
        // 也不属于 P3 停点范围。
        fixture.engine.enter_work_item_plan_compile().await;
        assert!(
            b2_p3_facts(&IssueAutomationStore::new(fixture.app_paths.clone())).is_empty(),
            "手动确认入口零写入"
        );
        assert_eq!(
            fixture.engine.session().stage,
            WorkspaceStage::HumanConfirm,
            "手动臂 compile 后等待人工确认"
        );
    }
}

/// 红测③：compile 失败打开恢复门即落 plan_question（focus=真实 compile
/// transaction），恢复动作成功关门即 resolve；Continue 不新建 transaction。
#[tokio::test]
async fn b2_p3_compile_recovery_gate_upserts_fact_and_resolves_on_continue() {
    let (_root, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let app_paths = lifecycle.app_paths();
    let session_id = engine.session().session_id.clone();
    let store = b2_p3_enable_and_bind(&app_paths, &plan_id, &session_id);

    // part_09 既有配方：真实编译后模拟「committing 中断」→ 恢复门开。
    let (mut compile_tx, accepted_drafts) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_b2_p3_recovery",
        "2026-10-16T00:00:00Z",
    );
    let plan_store = engine.work_item_plan_store().unwrap();
    plan_store.put_compile_transaction(&compile_tx).unwrap();
    engine.compile_initial_plan_revision(&accepted_drafts).unwrap();
    compile_tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
    compile_tx.failure_reason = Some("simulated crash after active plan".to_string());
    plan_store.put_compile_transaction(&compile_tx).unwrap();
    engine
        .enter_work_item_plan_compile_recovery(Some(
            "simulated crash after active plan".to_string(),
        ))
        .await;

    // 门开：plan_question 落 N0 store，focus 取真实 gate（compile_id）。
    let facts = b2_p3_facts(&store);
    let expected_key = format!("{plan_id}:compile_b2_p3_recovery:compile_recovery");
    let fact = facts
        .iter()
        .find(|fact| fact.key == expected_key)
        .unwrap_or_else(|| panic!("恢复门打开必须落 plan_question 事实：{facts:?}"));
    assert_eq!(fact.kind, AutomationAttentionKind::PlanQuestion);
    assert_eq!(fact.stage, AutomationAttentionStage::Plan);
    assert_eq!(fact.plan_id.as_deref(), Some(plan_id.as_str()));
    assert_eq!(fact.session_id.as_deref(), Some(session_id.as_str()));
    assert_eq!(
        fact.gate_id.as_deref(),
        Some("compile_b2_p3_recovery"),
        "focus 取真实 gate（durable compile transaction）"
    );
    assert_eq!(fact.reason_code, "work_item_plan_compile_recovery_required");

    // 恢复成功关门：事实消失；Continue 复用原 transaction（不重跑已提交
    // compile、不伪造 gate）。
    let outcome = engine
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .unwrap();
    assert_eq!(outcome, WorkItemPlanCompileRecoveryOutcome::HumanConfirm);
    assert!(
        b2_p3_facts(&store).is_empty(),
        "恢复门成功关闭后事实必须消失"
    );
    let transactions = plan_store
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert_eq!(
        transactions.len(),
        1,
        "Continue 不新建 compile transaction（不重跑已提交 compile）"
    );
}

/// 红测④：事实写失败上抛（经 EngineEvent::Error），不吞错、不假成功；
/// 停等本身仍 fail-closed（不 compile）。
#[tokio::test]
async fn b2_p3_attention_fact_write_failure_surfaces_engine_error() {
    let mut fixture = b2_p3_trust_engine(RunPolicy::AutoIfValid);
    let plan_id = fixture.engine.session().entity_id.clone();
    let session_id = fixture.engine.session().session_id.clone();
    b2_p3_enable_and_bind(&fixture.app_paths, &plan_id, &session_id);
    // 破坏 enrollment 文件：绑定/写读取失败必须上抛。
    std::fs::write(
        fixture
            .app_paths
            .issue_root("project_0001", "issue_0001")
            .join("automation-enrollment.json"),
        b"{ corrupted enrollment",
    )
    .expect("corrupt enrollment for write failure");
    fixture.trust.script_waiting("b2_p3_fixture_home_failure");

    fixture.engine.enter_policy_valid_work_item_plan_compile().await;

    assert_eq!(
        fixture.engine.session().stage,
        WorkspaceStage::HumanConfirm,
        "写失败仍不 compile，落人工确认门"
    );
    let errors = b2_p3_collect_errors(&mut fixture.event_rx);
    assert!(
        errors.iter().any(|message| message.contains("plan_question")),
        "事实写失败必须上抛：{errors:?}"
    );
}

/// 红测⑤对照：manual 授权口径的 plan（require_execution_plan_confirm=true）
/// 即使被 AutoIfValid 自动确认，finalizer 原值复制仍为 true（manual true
/// 仍拦执行确认；finalizer.rs 不改）。
#[tokio::test]
async fn b2_p3_manual_require_execution_plan_confirm_true_survives_auto_confirm() {
    let mut fixture = b2_p3_plain_engine(RunPolicy::AutoIfValid);
    let plan_id = fixture.engine.session().entity_id.clone();
    let mut plan = fixture
        .lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .unwrap();
    plan.options.require_execution_plan_confirm = true;
    crate::product::json_store::write_json(
        &fixture
            .app_paths
            .issue_root("project_0001", "issue_0001")
            .join("issue-work-item-plans")
            .join(format!("{plan_id}.json")),
        &plan,
    )
    .expect("flip plan options to manual confirm gate");

    fixture.engine.enter_policy_valid_work_item_plan_compile().await;

    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.engine.session().session_id)
        .unwrap();
    assert_eq!(durable.status, WorkspaceSessionStatus::Confirmed);
    let work_items = fixture
        .lifecycle
        .list_work_items("project_0001", "issue_0001")
        .unwrap();
    let compiled: Vec<_> = work_items
        .iter()
        .filter(|wi| wi.source_work_item_plan_id.as_deref() == Some(plan_id.as_str()))
        .collect();
    assert!(!compiled.is_empty(), "compile 已物化 WI");
    assert!(
        compiled.iter().all(|wi| wi.require_execution_plan_confirm),
        "manual true 原值仍拦（finalizer 原值回归）：{compiled:?}"
    );
}
