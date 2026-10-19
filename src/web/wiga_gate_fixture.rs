//! P1 WIGA Task 7/8 共享测试 fixture：enrollment 绑定 plan/session 的审批门
//! 与可编译链（accepted contract drafts + source/IR/report refs），供编排器
//! 停等人对照与 plan_confirmed_info 投影测试复用。仅测试构建编译。

#![cfg(test)]

use crate::cross_cutting::streaming_provider::ProviderCommand;
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::workspace::WorkspaceSessionRecord;
use crate::product::models::{
    SingleCandidatePhase, WorkItemDraftRecord, WorkItemDraftStatus, WorkItemGenerationMode,
    WorkItemKind, WorkItemOutline, WorkItemOutlineDependencyEdge, WorkItemOutlineSessionFit,
    WorkItemPlanDraftActiveIndex, WorkItemPlanOutline, WorkspaceSessionStatus,
};
use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason, RunPolicy};
use crate::product::work_item_plan_store::WorkItemPlanStore;
use crate::product::workspace_engine::SingleCandidateCompileCheckpoint;
use crate::product::workspace_engine::WorkItemPlanCompileFinalizerCheckpoint;
use crate::product::workspace_engine::tests::single_candidate_recovery::single_candidate_recovery_record;
use crate::web::autopilot_orchestrator::{AutopilotOrchestrator, ReconcileOutcome};
pub(crate) use crate::web::handlers::automation_enrollment_test_support::{ISSUE_ID, PROJECT_ID};
use crate::web::handlers::automation_enrollment_test_support::{
    put_enrollment, response_json, seed_fixture, seed_single_repository_fixture,
    single_repository_enable_body,
};
use crate::web::handlers::lifecycle::plan_preparation::ensure_enrolled_plan;
use crate::web::state::WebAppState;
use crate::web::workspace_session::WorkspaceSessionManager;
use crate::web::workspace_ws_handler::ProviderRunKind;
use crate::web::workspace_ws_types::{
    ArtifactPayload, ChoiceOption, WorkItemGenerationModeDto, WorkItemPlanOutlineCandidateDto,
    WsOutMessage,
};
use axum::http::StatusCode;
use std::collections::BTreeMap;
use std::sync::Arc;
use tower::ServiceExt;

mod p2_campaign;
pub(crate) use p2_campaign::c5_single_repository_campaign_fixture;
pub(crate) use p2_campaign::p2_enrolled_campaign_fixture;
use p2_campaign::{init_real_main_checkout, normalize_checkout_revision_to_unobserved};

pub(crate) struct EnrolledGateFixture {
    pub(crate) inner: crate::web::handlers::automation_enrollment_test_support::Fixture,
    pub(crate) state: WebAppState,
    lifecycle: LifecycleStore,
    plan_id: String,
    pub(crate) session_id: String,
    manager: Arc<WorkspaceSessionManager>,
    token: u64,
    incarnation: String,
    gate_id: Option<String>,
}

/// 测试态恒健康 provider health 源：CI/测试机上探测不到 claude/codex 二进制
/// 时真实 `ProviderHealthService` 恒 degraded，gateway spawn 前的
/// `ensure_available` 会拒绝 fake registry 承接的 claude_code 方言。P2
/// Fake 全链的 provider 启动经 provider gateway（引擎对带
/// target_snapshot 的 attempt fail-closed 强制），须以恒健康 gate 重建
/// gateway factory（仅测试 fixture，生产路径不受影响）。
struct AlwaysHealthyProviderHealth;

impl crate::cross_cutting::provider_availability_gate::ProviderHealthSource
    for AlwaysHealthyProviderHealth
{
    fn snapshot(
        &self,
    ) -> std::sync::Arc<crate::cross_cutting::provider_health::ProviderHealthSnapshot> {
        let checked_at = chrono::Utc::now();
        let entry = |provider: crate::product::models::ProviderName| {
            crate::cross_cutting::provider_health::ProviderHealthEntry {
                provider,
                command: "fake-health".to_string(),
                available: true,
                version: Some("fake".to_string()),
                reason_code: None,
                reason: None,
                checked_at,
            }
        };
        std::sync::Arc::new(
            crate::cross_cutting::provider_health::ProviderHealthSnapshot {
                schema_version: 1,
                generation: 1,
                checked_at,
                providers: vec![
                    entry(crate::product::models::ProviderName::ClaudeCode),
                    entry(crate::product::models::ProviderName::Codex),
                    entry(crate::product::models::ProviderName::Pi),
                    entry(crate::product::models::ProviderName::KimiCode),
                ],
            },
        )
    }

    fn degraded(&self) -> bool {
        false
    }
}

fn fake_state_with_gateway(root: std::path::PathBuf) -> WebAppState {
    let state = WebAppState::new(
        root.clone(),
        crate::web::runtime::WebRuntime::new_fake(root.clone()),
    );
    let factory = crate::web::gateway_factory::LogicalCodebaseGatewayFactory::new(
            crate::product::app_paths::ProductAppPaths::new(root.join(".aria")),
            state.provider_registry.clone(),
            state.provider_adapter.clone(),
            std::sync::Arc::new(
                crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate::with_host_readiness(
                    std::sync::Arc::new(AlwaysHealthyProviderHealth),
                    std::sync::Arc::new(
                        crate::cross_cutting::provider_availability_gate::AlwaysReadyProviderHost,
                    ),
                ),
            ),
        );
    state.with_gateway_factory(std::sync::Arc::new(factory))
}

impl EnrolledGateFixture {
    pub(crate) async fn new() -> Self {
        let inner = seed_fixture(1, true);
        let root_path = inner._root.path().to_path_buf();
        let state = fake_state_with_gateway(root_path.clone());
        // 子 WorkItem 上下文经 issue.repo_id 解析 repo-1：补进 repos.json
        //（seed_logical_codebase 重写后仅剩 physical 成员条目）。SG3b 门
        // 接入后 Final Compile 结构复验会经 plan_baseline_tree 对该路径做
        // fail-closed 基线解析（REQ-PIB main→master 默认链），裸目录即
        // 128 拒绝——须与 new_single_repository 同款真实 git 仓（main 分支
        // + 空提交 + 本地 bare origin）。
        {
            let repos_path = inner.paths.project_root(PROJECT_ID).join("repos.json");
            let mut repositories: Vec<crate::product::models::RepositoryRecord> =
                crate::product::json_store::read_json(&repos_path).unwrap();
            let repo_root = inner._root.path().join("repo-1");
            init_real_main_checkout(&repo_root);
            let now = "2026-09-27T00:00:00Z".to_string();
            repositories.push(crate::product::models::RepositoryRecord {
                id: crate::web::handlers::automation_enrollment_test_support::REPOSITORY_ID
                    .to_string(),
                project_id: PROJECT_ID.to_string(),
                name: "repo-1".to_string(),
                path: repo_root,
                repo_hash: "sha256:fixture-repo-1".to_string(),
                runtime_root: inner._root.path().join("repo-1/.aria/runtime"),
                default_policy_preset: "manual-write".to_string(),
                default_provider_mode: "fake".to_string(),
                created_at: now.clone(),
                logical_repository_id: None,
                primary_checkout_id: None,
                identity_schema_version: 1,
                updated_at: now,
            });
            crate::product::json_store::write_json(&repos_path, &repositories).unwrap();
        }
        // 追加一对「仓库指向有效 physical checkout」的确认 story/design：
        // 绑定 plan 的 IR target 由此解析（seed 的 repo-1 不在有效成员集，
        // compile 会在 target 有效性上 fail-closed）。
        let lifecycle_seed = LifecycleStore::new(inner.paths.clone());
        let story = lifecycle_seed
            .create_story_spec(crate::product::lifecycle_store::CreateStorySpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: "physical-checkout-enroll-a".to_string(),
                title: "gate compile story".to_string(),
                aggregate_codebase: None,
            })
            .unwrap();
        lifecycle_seed
            .append_version(crate::product::lifecycle_store::AppendSpecVersionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: story.id.clone(),
                markdown: "# gate story".to_string(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: None,
            })
            .unwrap();
        lifecycle_seed
            .update_spec_confirmation_status(
                PROJECT_ID,
                ISSUE_ID,
                &story.id,
                crate::product::models::LifecycleConfirmationStatus::Confirmed,
            )
            .unwrap();
        let design = lifecycle_seed
            .create_design_spec(crate::product::lifecycle_store::CreateDesignSpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "gate compile design".to_string(),
                aggregate_codebase: None,
            })
            .unwrap();
        lifecycle_seed
            .append_version(crate::product::lifecycle_store::AppendSpecVersionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: design.id.clone(),
                markdown: "# gate design".to_string(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: None,
            })
            .unwrap();
        lifecycle_seed
            .update_spec_confirmation_status(
                PROJECT_ID,
                ISSUE_ID,
                &design.id,
                crate::product::models::LifecycleConfirmationStatus::Confirmed,
            )
            .unwrap();
        // C1 Task 3：enrollment 显式声明双载体 target → durable binding v1
        //（自动链统一经 load_current_enrollment_binding 校验当前代）。
        let logical_codebase_id =
            crate::product::logical_codebase::LogicalCodebaseStore::new(inner.paths.clone())
                .load_manifest(PROJECT_ID)
                .unwrap()
                .expect("seeded logical codebase manifest")
                .logical_codebase_id
                .to_string();
        // fix 轮 1(Task 3 只读迁移收尾)+Task 8:coding runner 与
        // Enable/rebind 预检的 factory 组装均改为 build_readonly_for_lc
        // (不再隐式 ensure_bootstrap)。enrollment Enable 之前,按 coding
        // runner 同源的 authority resolver 解析 LC 作用域,显式播种自举桩
        // 政策与 capability 记录——断言意图零变化(Enable 预检与 coding
        // run 均经 gateway 校验,材料就绪顺序与消费顺序一致)。
        if let Some(runner_lc_id) =
            crate::product::logical_codebase::RepositoryAuthorityResolver::new(inner.paths.clone())
                .resolve_for_issue(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .and_then(|authority| authority.target.logical_codebase_id)
        {
            let seed_manifest = crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
                inner.paths.clone(),
                &runner_lc_id,
            )
            .load_manifest(PROJECT_ID)
            .unwrap()
            .expect("seeded logical codebase manifest");
            crate::product::logical_codebase::AggregatePolicyArtifactStore::for_lc(
                inner.paths.clone(),
                &runner_lc_id,
            )
            .ensure_bootstrap(&seed_manifest)
            .expect("seed aggregate policy artifact");
            crate::product::logical_codebase::ProviderCapabilityStore::for_lc(
                inner.paths.clone(),
                &runner_lc_id,
            )
            .ensure_bootstrap(PROJECT_ID)
            .expect("seed capability record");
            // 迁移桥退役（4749a1fc）后 bootstrap 行恒为 Unknown：正向路径
            // 必须在自举之后物化「已探测」Confirmed 行，Enable 预检与
            // coding run 才能通过 capability 门（产品 fail-closed 语义不变，
            // 需 Unknown 的负向场景不调用本播种）。
            crate::web::handlers::automation_enrollment_test_support::seed_confirmed_capability_records(
                &inner.paths,
                PROJECT_ID,
                Some(&runner_lc_id),
                &[crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode],
            );
        }
        let body = serde_json::json!({
            "expected_revision": null,
            "command": {
                "type": "enable",
                "selection_key": "gate-fixture-1",
                "source": {
                    "stories": [{"id": story.id, "version": 1}],
                    "designs": [{"id": design.id, "version": 1}]
                },
                "options": {
                    // 逻辑单 target 的 coding 引擎对带 target_snapshot 的 attempt
                    // 强制经 provider gateway（`logical_provider_gateway_required`
                    // fail-closed，Fake 无 gateway 方言不可用作 attempt provider）；
                    // 测试 provider 模式下 registry 将 claude_code 路由到
                    // TestControlledFakeStreamingProvider，行为等同 fake。
                    "author_provider": "claude_code",
                    "reviewer_provider": "claude_code",
                    "review_rounds": 1,
                    "superpowers_enabled": false,
                    "openspec_enabled": false,
                    "plan_options": {
                        "include_integration_tests": false,
                        "include_e2e_tests": false,
                        "force_frontend_backend_split": false,
                        "require_execution_plan_confirm": false
                    }
                },
                "logical_repository_id": crate::web::handlers::automation_enrollment_test_support::SINGLE_LOGICAL_ID
                ,"target": {
                    "kind": "logical_codebase",
                    "logical_codebase_id": logical_codebase_id,
                    "logical_repository_id": crate::web::handlers::automation_enrollment_test_support::SINGLE_LOGICAL_ID
                }
            }
        });
        let app = crate::web::app::build_web_router(state.clone());
        let enable = put_enrollment(&app, body).await;
        assert_eq!(enable.status(), StatusCode::OK);
        assert!(response_json(enable).await["enabled"].as_bool().unwrap());

        let lifecycle = LifecycleStore::new(inner.paths.clone());
        let store = IssueAutomationStore::new(inner.paths.clone());
        let enrollment = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        // 真实 Task 4 补偿面：锁内唯一创建/绑定（不触发生成动作）。
        crate::web::handlers::lifecycle::plan_preparation::ensure_enrolled_plan(
            &state,
            &enrollment,
        )
        .await
        .expect("ensure enrolled plan");
        let bound = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        let plan_id = bound.plan_id.expect("bound plan");
        let session_id = bound.session_id.expect("bound session");

        let manager = state
            .workspace_sessions
            .get_or_create(&session_id, || {
                WorkspaceSessionManager::create(&state, &session_id)
            })
            .await
            .expect("bound session manager");
        // 已派发的生成 run：manager 唯一 run 面真实注册；provider 等待者
        // 持续消费 command 通道（choice 应答 Delivered 的真实对端）。
        let (_run_id, token, _cancel, command_rx, _node) = manager
            .start_run(ProviderRunKind::WorkItemPlanSingleCandidateAuthor, None)
            .await
            .expect("in-flight generation run");
        let incarnation = manager.active_run_incarnation().expect("run incarnation");
        tokio::spawn(deliver_choice_receipts(command_rx));
        Self {
            inner,
            state,
            lifecycle,
            plan_id,
            session_id,
            manager,
            token,
            incarnation,
            gate_id: None,
        }
    }

    /// C5 Task 4：单仓 enrolled 链 fixture——issue.repo_id 指向真实 git
    /// 物理仓（main 基线 + 空提交 + 本地 origin，满足 advance fork 基线
    /// 三面同源解析），enrollment target 为 SingleRepository；plan/session
    /// 绑定与生成 run 注册与 `new` 同源（仅载体与 Enable body 不同）。
    pub(crate) async fn new_single_repository() -> Self {
        let inner = seed_single_repository_fixture();
        init_real_main_checkout(&inner._root.path().join("repo-1"));
        let root_path = inner._root.path().to_path_buf();
        let state = fake_state_with_gateway(root_path.clone());
        let app = crate::web::app::build_web_router(state.clone());
        let mut body = single_repository_enable_body(&inner);
        // gate outline 无 integration 工作项：与 LC fixture 同口径关闭
        // include_integration_tests（编译严格校验会拒绝缺项计划）。
        body["command"]["options"]["plan_options"]["include_integration_tests"] =
            serde_json::json!(false);
        let enable = put_enrollment(&app, body).await;
        assert_eq!(enable.status(), StatusCode::OK);
        assert!(response_json(enable).await["enabled"].as_bool().unwrap());

        let lifecycle = LifecycleStore::new(inner.paths.clone());
        let store = IssueAutomationStore::new(inner.paths.clone());
        let enrollment = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        ensure_enrolled_plan(&state, &enrollment)
            .await
            .expect("ensure enrolled plan");
        let bound = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        let plan_id = bound.plan_id.expect("bound plan");
        let session_id = bound.session_id.expect("bound session");

        let manager = state
            .workspace_sessions
            .get_or_create(&session_id, || {
                WorkspaceSessionManager::create(&state, &session_id)
            })
            .await
            .expect("bound session manager");
        let (_run_id, token, _cancel, command_rx, _node) = manager
            .start_run(ProviderRunKind::WorkItemPlanSingleCandidateAuthor, None)
            .await
            .expect("in-flight generation run");
        let incarnation = manager.active_run_incarnation().expect("run incarnation");
        tokio::spawn(deliver_choice_receipts(command_rx));
        Self {
            inner,
            state,
            lifecycle,
            plan_id,
            session_id,
            manager,
            token,
            incarnation,
            gate_id: None,
        }
    }

    /// 批 2 R4/E1（REQ-WIGA-13）：四仓集合 enrollment fixture——4 成员 LC
    ///（真实 main checkout 就位）＋repo-1 物理仓（issue.repo_id 解析）＋
    /// 成员锚定确认 story/design；Enable target 为集合
    /// `logical_codebase_members`（全部 4 成员）且 `plan_approval=auto`
    ///（AutoIfValid 冻结）；`ensure_enrolled_plan` 锁内唯一创建/绑定
    /// plan/session。不预注册生成 run：自动批准链由
    /// `auto_approve_plan` 的引擎 policy-valid compile 承载（零人工）。
    pub(crate) async fn new_four_repository_collection() -> Self {
        use crate::web::handlers::automation_enrollment_test_support::collection_enable_body;

        let inner = seed_fixture(4, true);
        // 成员 checkout 的权威 `canonical_path` 是 `paths.root()/{checkout}`
        //（`seed_fixture` 记录形态，与 p2/prepared 先例同源）；真仓必须就位
        // 在同一路径，否则 advance 的目标快照捕获（`git rev-parse HEAD`）
        // 落 fail-closed `plan_coding_advance_failed`。
        for checkout in [
            "checkout-enroll-a",
            "checkout-enroll-b",
            "checkout-enroll-c",
            "checkout-enroll-d",
        ] {
            init_real_main_checkout(&inner.paths.root().join(checkout));
        }
        normalize_checkout_revision_to_unobserved(&inner.paths);
        let root_path = inner._root.path().to_path_buf();
        let state = fake_state_with_gateway(root_path.clone());
        // 子 WorkItem 上下文经 issue.repo_id 解析 repo-1：与 `new` 同款真实
        // git 仓（main 分支＋空提交＋本地 bare origin）＋repos.json 登记。
        {
            let repos_path = inner.paths.project_root(PROJECT_ID).join("repos.json");
            let mut repositories: Vec<crate::product::models::RepositoryRecord> =
                crate::product::json_store::read_json(&repos_path).unwrap();
            let repo_root = inner._root.path().join("repo-1");
            init_real_main_checkout(&repo_root);
            let now = "2026-09-27T00:00:00Z".to_string();
            repositories.push(crate::product::models::RepositoryRecord {
                id: crate::web::handlers::automation_enrollment_test_support::REPOSITORY_ID
                    .to_string(),
                project_id: PROJECT_ID.to_string(),
                name: "repo-1".to_string(),
                path: repo_root,
                repo_hash: "sha256:fixture-repo-1".to_string(),
                runtime_root: inner._root.path().join("repo-1/.aria/runtime"),
                default_policy_preset: "manual-write".to_string(),
                default_provider_mode: "fake".to_string(),
                created_at: now.clone(),
                logical_repository_id: None,
                primary_checkout_id: None,
                identity_schema_version: 1,
                updated_at: now,
            });
            crate::product::json_store::write_json(&repos_path, &repositories).unwrap();
        }
        // 成员锚定确认 story/design（member-a 物理仓；seed 的 repo-1 不在
        // 有效成员集，compile 在 target 有效性上 fail-closed）。
        let lifecycle_seed = LifecycleStore::new(inner.paths.clone());
        let story = lifecycle_seed
            .create_story_spec(crate::product::lifecycle_store::CreateStorySpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: "physical-checkout-enroll-a".to_string(),
                title: "four repository story".to_string(),
                aggregate_codebase: None,
            })
            .unwrap();
        lifecycle_seed
            .append_version(crate::product::lifecycle_store::AppendSpecVersionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: story.id.clone(),
                markdown: "# four repository story".to_string(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: None,
            })
            .unwrap();
        lifecycle_seed
            .update_spec_confirmation_status(
                PROJECT_ID,
                ISSUE_ID,
                &story.id,
                crate::product::models::LifecycleConfirmationStatus::Confirmed,
            )
            .unwrap();
        let logical_codebase_ref =
            crate::product::logical_codebase::LogicalCodebaseStore::new(inner.paths.clone())
                .load_manifest(PROJECT_ID)
                .unwrap()
                .expect("seeded logical codebase manifest")
                .logical_codebase_id;
        // 聚合 Design 携带 involved/change_order（REQ-WSC-08 上界子集判定：
        // involved 非空才打开多仓 plan 入口闸；多仓 involved 必须给顺序）。
        // involved=实际三仓（member-4 选中但设计不涉及）。
        let member = |index: u128| {
            crate::product::logical_codebase::LogicalRepositoryId(uuid::Uuid::from_u128(index))
        };
        let design = lifecycle_seed
            .create_design_spec(crate::product::lifecycle_store::CreateDesignSpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "four repository design".to_string(),
                aggregate_codebase: Some(
                    crate::product::lifecycle_store::AggregateDesignSpecScope {
                        logical_codebase_ref,
                        effective_member_ids: vec![member(1), member(2), member(3), member(4)],
                        involved_repository_ids: vec![member(1), member(2), member(3)],
                        change_order: vec![member(1), member(2), member(3)],
                    },
                ),
            })
            .unwrap();
        lifecycle_seed
            .append_version(crate::product::lifecycle_store::AppendSpecVersionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: design.id.clone(),
                markdown: "# four repository design".to_string(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: None,
            })
            .unwrap();
        lifecycle_seed
            .update_spec_confirmation_status(
                PROJECT_ID,
                ISSUE_ID,
                &design.id,
                crate::product::models::LifecycleConfirmationStatus::Confirmed,
            )
            .unwrap();
        // LC 身份与 k3 波2 F3 裁决后的 advance 集合互证同源：PUT canonical
        // Enable 冻结 manifest UUID 形态（`validate_enrollment_scope`），
        // advance drift 比对同一 manifest 身份——fixture 同源取 manifest 值。
        let logical_codebase_id = logical_codebase_ref.to_string();
        // 与 `new` 同源自举桩（Enable 预检与 coding run 的 factory 组装均
        // 走 build_readonly_for_lc）。
        if let Some(runner_lc_id) =
            crate::product::logical_codebase::RepositoryAuthorityResolver::new(inner.paths.clone())
                .resolve_for_issue(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .and_then(|authority| authority.target.logical_codebase_id)
        {
            let seed_manifest = crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
                inner.paths.clone(),
                &runner_lc_id,
            )
            .load_manifest(PROJECT_ID)
            .unwrap()
            .expect("seeded logical codebase manifest");
            crate::product::logical_codebase::AggregatePolicyArtifactStore::for_lc(
                inner.paths.clone(),
                &runner_lc_id,
            )
            .ensure_bootstrap(&seed_manifest)
            .expect("seed aggregate policy artifact");
            crate::product::logical_codebase::ProviderCapabilityStore::for_lc(
                inner.paths.clone(),
                &runner_lc_id,
            )
            .ensure_bootstrap(PROJECT_ID)
            .expect("seed capability record");
            // 迁移桥退役（4749a1fc）后 bootstrap 行恒为 Unknown：正向路径
            // 必须在自举之后物化「已探测」Confirmed 行，Enable 预检与
            // coding run 才能通过 capability 门（产品 fail-closed 语义不变，
            // 需 Unknown 的负向场景不调用本播种）。
            crate::web::handlers::automation_enrollment_test_support::seed_confirmed_capability_records(
                &inner.paths,
                PROJECT_ID,
                Some(&runner_lc_id),
                &[crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode],
            );
        }
        let body = collection_enable_body(&story.id, &design.id, &logical_codebase_id);
        let app = crate::web::app::build_web_router(state.clone());
        let enable = put_enrollment(&app, body).await;
        assert_eq!(enable.status(), StatusCode::OK);
        assert!(response_json(enable).await["enabled"].as_bool().unwrap());

        let lifecycle = LifecycleStore::new(inner.paths.clone());
        let store = IssueAutomationStore::new(inner.paths.clone());
        let enrollment = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        ensure_enrolled_plan(&state, &enrollment)
            .await
            .expect("ensure enrolled plan");
        let bound = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        let plan_id = bound.plan_id.expect("bound plan");
        let session_id = bound.session_id.expect("bound session");

        let manager = state
            .workspace_sessions
            .get_or_create(&session_id, || {
                WorkspaceSessionManager::create(&state, &session_id)
            })
            .await
            .expect("bound session manager");
        Self {
            inner,
            state,
            lifecycle,
            plan_id,
            session_id,
            manager: manager.clone(),
            token: 0,
            incarnation: manager.active_run_incarnation().unwrap_or_default(),
            gate_id: None,
        }
    }

    /// 批 2 R4/E1：AutoIfValid 自动批准——铺四仓归属 accepted drafts（wi_a→
    /// member-1、wi_b→member-2 依赖 wi_a、wi_c→member-3 独立）＋active
    /// index＋Approval 相位（AutoIfValid）后，经引擎 policy-valid compile
    /// 直达 Confirmed（先例 P2 `auto_enrolled_sc_confirms_via_engine_...`；
    /// 全程零 approve/人工门 REST）。
    pub(crate) async fn auto_approve_plan(&mut self) {
        let engine_arc = self.manager.engine();
        let mut engine = engine_arc.lock().await;
        engine.session.artifact = Some(four_repository_outline_candidate_payload());
        let plan_store = WorkItemPlanStore::new(self.inner.paths.clone());
        let logical_codebase_id =
            crate::product::logical_codebase::LogicalCodebaseStore::new(self.inner.paths.clone())
                .load_manifest(PROJECT_ID)
                .unwrap()
                .expect("seeded logical codebase manifest")
                .logical_codebase_id
                .to_string();
        let mut drafts = Vec::new();
        for (outline_id, member_index, depends_on) in [
            ("outline_a", 1u128, Vec::new()),
            ("outline_b", 2u128, vec!["wi_a".to_string()]),
            ("outline_c", 3u128, Vec::new()),
        ] {
            let draft = gate_draft_record_with_member(
                &self.plan_id,
                outline_id,
                &format!("draft_{outline_id}"),
                &logical_codebase_id,
                crate::product::logical_codebase::LogicalRepositoryId(uuid::Uuid::from_u128(
                    member_index,
                )),
                depends_on,
            );
            drafts.push(draft);
        }
        // 跨仓依赖交付：wi_b 要求 wi_a 的 output contract（与既有
        // seed_accepted_plan_drafts 的 draft_b 同款 rebrand）。
        {
            let mut required =
                crate::product::work_item_contract::canonical_contract_fixture("unused")
                    .input_contracts
                    .remove(0);
            required.provider_logical_work_item_id = "wi_a".to_string();
            required.contract_id = "contract.canonical".to_string();
            required.required_capabilities = vec!["stable_hash".to_string()];
            drafts[1]
                .candidate
                .canonical_contract_candidate
                .input_contracts
                .push(required);
        }
        for draft in drafts {
            plan_store
                .put_draft_record(&draft)
                .expect("put four-repository member draft");
        }
        plan_store
            .save_active_index(&four_repository_active_index(&self.plan_id))
            .expect("save four-repository active index");
        single_candidate_recovery_record(
            &self.lifecycle,
            &mut engine,
            SingleCandidatePhase::Approval,
            RunPolicy::AutoIfValid,
        );
        // 通用恢复 helper 把 IR 每 item target 硬编码为会话仓（单成员现场
        // 等价生产 lowering）；四仓集合现场须按 Plan Intent 重放成员物理仓
        //（生产 lowering 的 `resolve_item_target_repository_id` 语义），
        // 否则 Final Compile 结构复验对 wi_b/wi_c 报 target_mismatch。
        {
            let member_physical: std::collections::BTreeMap<
                crate::product::logical_codebase::LogicalRepositoryId,
                String,
            > = crate::product::logical_codebase::LogicalCodebaseStore::new(
                self.inner.paths.clone(),
            )
            .list_members(PROJECT_ID)
            .unwrap()
            .into_iter()
            .map(|member| (member.logical_repository_id, member.physical_repository_id))
            .collect();
            let durable = self.durable();
            let ir_ref = durable
                .plan_candidate_ir_ref
                .clone()
                .expect("candidate IR ref after recovery record");
            let source_store =
                crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
                    self.inner.paths.clone(),
                );
            let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                plan_id: self.plan_id.clone(),
            };
            let mut ir_record = source_store
                .get_plan_candidate_ir(&scope, &ir_ref)
                .expect("load fixture candidate IR");
            // 不可变 store：换新 id 重放（同 source_revision 身份），session
            // 引用改指新记录。
            ir_record.id = format!("{}-four-repo-members", ir_record.id);
            for item in &mut ir_record.ir.items {
                if let Some(crate::product::logical_codebase::EnrollmentTarget::LogicalCodebase {
                    logical_repository_id,
                    ..
                }) = item
                    .contract
                    .intent_contract
                    .as_ref()
                    .map(|intent| &intent.target)
                    && let Some(physical) = member_physical.get(logical_repository_id)
                {
                    item.target_repository_id = physical.clone();
                }
            }
            ir_record.content_hash = String::new();
            ir_record.content_hash = ir_record
                .content_hash()
                .expect("recompute fixture IR content hash");
            let new_ir_ref = source_store
                .put_plan_candidate_ir(PROJECT_ID, ISSUE_ID, &self.plan_id, &ir_record)
                .expect("persist per-member fixture IR");
            let new_ir_id = ir_record.id.clone();
            // mechanical report 的 ir_id 与 provenance 引用校验互锁：同步
            // 换新 id 重放（同 source_revision 身份）。
            let report_ref = durable
                .mechanical_report_ref
                .clone()
                .expect("candidate report ref after recovery record");
            let mut report_record = source_store
                .get_mechanical_report(&scope, &report_ref)
                .expect("load fixture candidate report");
            report_record.id = format!("{}-four-repo-members", report_record.id);
            report_record.ir_id = new_ir_id;
            report_record.content_hash = String::new();
            report_record.content_hash = report_record
                .content_hash()
                .expect("recompute fixture report content hash");
            let new_report_ref = source_store
                .put_mechanical_report(PROJECT_ID, ISSUE_ID, &self.plan_id, &report_record)
                .expect("persist per-member fixture report");
            let mut session = self.durable();
            session.plan_candidate_ir_ref = Some(new_ir_ref);
            session.mechanical_report_ref = Some(new_report_ref);
            crate::product::json_store::write_json(&self.session_path(&session.id), &session)
                .expect("persist session with per-member refs");
            let artifact = engine.session.artifact.clone();
            engine.session =
                crate::product::workspace_engine::WorkspaceSession::from_record(session);
            engine.session.artifact = artifact;
        }
        engine.enter_policy_valid_work_item_plan_compile().await;
        drop(engine);
        // 终态断言由调用方承担（诊断期不在此处提前 panic）。
    }

    /// 批 2 R4/E1：无页面 reconcile 循环（与 p2_campaign 同源）：任一 attempt
    /// 进入 Failed/Aborted/AwaitingManualRecovery 即 panic（真实业务断点）。
    pub(crate) async fn reconcile_until(
        &self,
        predicate: impl Fn(&[crate::product::coding_models::CodingExecutionAttempt]) -> bool,
    ) {
        let worker = AutopilotOrchestrator::new(self.state.clone(), Default::default());
        let mut last_outcome = String::from("no iteration");
        let reached = tokio::time::timeout(std::time::Duration::from_secs(300), async {
            loop {
                let outcome = worker
                    .reconcile(&self.state, PROJECT_ID, ISSUE_ID)
                    .await
                    .expect("r4/e1 campaign reconcile");
                last_outcome = format!("{outcome:?}");
                let attempts = self.coding_attempts();
                for attempt in &attempts {
                    use crate::product::coding_models::CodingAttemptStatus;
                    match attempt.status {
                        CodingAttemptStatus::Failed
                        | CodingAttemptStatus::Aborted
                        | CodingAttemptStatus::AwaitingManualRecovery => panic!(
                            "r4/e1 campaign stopped unexpectedly at {:?}/{:?} reason={:?}",
                            attempt.status, attempt.stage, attempt.manual_recovery_reason
                        ),
                        _ => {}
                    }
                }
                if predicate(&attempts) {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .is_ok();
        if !reached {
            let attempts = self.coding_attempts();
            let detail = attempts
                .iter()
                .map(|attempt| {
                    format!(
                        "{:?}/{:?} claim={} member={:?} policy={:?}",
                        attempt.status,
                        attempt.stage,
                        attempt.start_claim.is_some(),
                        attempt
                            .target_snapshot
                            .as_ref()
                            .map(|snapshot| snapshot.logical_repository_id),
                        attempt.start_run_policy
                    )
                })
                .collect::<Vec<_>>()
                .join("\n  ");
            let enrollment = self.enrollment();
            let facts = enrollment
                .attention_facts
                .iter()
                .map(|fact| format!("{:?}/{:?}", fact.kind, fact.stage))
                .collect::<Vec<_>>()
                .join(",");
            panic!(
                "r4/e1 campaign predicate not reached via reconcile only: last_reconcile={last_outcome}; \
                 enrollment(enabled={}, terminated={}, plan={:?}, session={:?}, facts=[{facts}]); \
                 attempts={}:\n  {detail}",
                enrollment.enabled,
                enrollment.terminated_at.is_some(),
                enrollment.plan_id,
                enrollment.session_id,
                attempts.len(),
            );
        }
    }

    /// 批 2 R4/E1：指定逻辑成员仓的实际桶 attempt（按 target_snapshot 归属）。
    pub(crate) fn attempt_for_member(
        &self,
        member: &crate::product::logical_codebase::LogicalRepositoryId,
    ) -> Option<crate::product::coding_models::CodingExecutionAttempt> {
        self.coding_attempts().into_iter().find(|attempt| {
            attempt
                .target_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.logical_repository_id == *member)
        })
    }

    /// 批 2 R4/E1：四仓 plan 子 WorkItem 会话的完整授权复制 target 集
    ///（finalizer child 精确绑定；集合 target 原样复制）。
    pub(crate) fn child_collection_binding_targets(
        &self,
    ) -> Vec<crate::product::logical_codebase::EnrollmentTarget> {
        self.lifecycle
            .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
            .expect("list child work item sessions")
            .into_iter()
            .filter(|session| {
                session.workspace_type == crate::product::models::WorkspaceType::WorkItem
                    && ["wi_a", "wi_b", "wi_c"].contains(&session.entity_id.as_str())
            })
            .filter_map(|session| {
                session
                    .work_item_child_binding
                    .map(|binding| binding.target)
            })
            .collect()
    }

    /// 真实登记两张待答 choice 卡（run 挂起帧面，与 P0 part_02 同构）。
    pub(crate) async fn open_choice_with_two_questions(&self) {
        self.manager
            .register_pending_choice_frame(gate_choice_frame("choice-gate-1"));
        self.manager
            .register_pending_choice_frame(gate_choice_frame("choice-gate-2"));
    }

    pub(crate) fn provider_start_ledger(&self) -> usize {
        self.durable().provider_start_ledger.len()
    }

    pub(crate) fn durable(&self) -> WorkspaceSessionRecord {
        self.lifecycle
            .get_workspace_session(&self.session_id)
            .expect("durable bound session")
    }

    pub(crate) async fn reconcile(&self) -> Result<ReconcileOutcome, String> {
        let worker = AutopilotOrchestrator::new(self.state.clone(), Default::default());
        worker.reconcile(&self.state, PROJECT_ID, ISSUE_ID).await
    }

    /// 逐张经 P0 REST 作答并等待 Delivered；应答送达后 run 收尾。
    pub(crate) async fn human_answer_via_rest(&self) {
        let app = crate::web::app::build_web_router(self.state.clone());
        for choice_id in ["choice-gate-1", "choice-gate-2"] {
            let body = serde_json::json!({
                "command_id": format!("cmd-{choice_id}"),
                "expected_run_id": self.incarnation,
                "answers": [
                    { "question_id": "q-1", "selected_option_ids": ["yes"], "free_text": null },
                    { "question_id": "q-2", "selected_option_ids": ["no"], "free_text": null },
                ],
            });
            let (status, payload) = post_choice(&app, &self.session_id, choice_id, &body).await;
            assert_eq!(
                status,
                StatusCode::OK,
                "choice REST must deliver: {payload}"
            );
            assert_eq!(payload["state"], "delivered", "{payload}");
        }
        self.manager.finish_run(self.token).await;
        assert!(
            self.manager.pending_choice_frames().is_empty(),
            "delivered choices must leave no pending frames"
        );
    }

    /// durable 铺底（审批门与 Evaluate 相位共用）：engine outline 候选 +
    /// accepted contract drafts + active index，全部经真实 store 面。
    /// `single_candidate_recovery_record` 的 refs 持久化依赖 active
    /// index 已就位，因此先铺本层再落相位记录。
    async fn seed_accepted_plan_drafts(&mut self) {
        let engine_arc = self.manager.engine();
        let mut engine = engine_arc.lock().await;
        engine.session.artifact = Some(gate_outline_candidate_payload());
        let plan_store = WorkItemPlanStore::new(self.inner.paths.clone());
        let draft_a = gate_draft_record(&self.plan_id, "outline_a", "draft_outline_a");
        let mut draft_b = gate_draft_record(&self.plan_id, "outline_b", "draft_outline_b");
        let mut required = crate::product::work_item_contract::canonical_contract_fixture("unused")
            .input_contracts
            .remove(0);
        required.provider_logical_work_item_id = "wi_a".to_string();
        required.contract_id = "contract.canonical".to_string();
        required.required_capabilities = vec!["stable_hash".to_string()];
        draft_b
            .candidate
            .canonical_contract_candidate
            .input_contracts
            .push(required);
        draft_b
            .candidate
            .canonical_contract_candidate
            .handoff_contract
            .provided_contract_refs
            .clear();
        plan_store
            .put_draft_record(&draft_a)
            .expect("put accepted draft a");
        plan_store
            .put_draft_record(&draft_b)
            .expect("put accepted draft b");
        plan_store
            .save_active_index(&gate_active_index(&self.plan_id))
            .expect("save accepted draft index");
        drop(engine);
    }

    /// 把绑定 plan/session 铺成可编译的审批门形态（accepted contract
    /// drafts + active index + outline 候选 + source/IR/report 三 refs，
    /// 全部经真实 store/engine 面）并在「真正执行 approve 的那个引擎」
    /// 上真实开门（时间线 HumanConfirm 节点 + durable WaitingForHuman），
    /// 返回 gate 节点 id。阶段一落 durable 铺底；choice 应答后 run
    /// 收尾可能触发 idle 回收，因此阶段二按 registry 解析当前 manager。
    /// `fail_compile_after_human_approve` 与 Task 10 campaign 的
    /// `confirm_plan_by_human` 共用同一铺底：区别只在 approve 前
    /// 是否注册 compile failpoint。
    async fn seed_approval_gate(&mut self) -> String {
        // 阶段一（durable 铺底）：accepted drafts + SC Approval 相位记录 +
        // 审批门快照。
        self.seed_accepted_plan_drafts().await;
        let engine_arc = self.manager.engine();
        let mut engine = engine_arc.lock().await;
        single_candidate_recovery_record(
            &self.lifecycle,
            &mut engine,
            SingleCandidatePhase::Approval,
            RunPolicy::Interactive,
        );
        drop(engine);
        let mut record = self.durable();
        record.status = WorkspaceSessionStatus::WaitingForHuman;
        record.human_gate_snapshot = Some(HumanGateSnapshot {
            candidate_recovery: None,
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            attempts_used: 0,
            manual_repairs_remaining: 1,
            trigger: HumanReason::NativeHumanRequired,
            resumable: false,
            accepted_feedback_turns: None,
        });
        crate::product::json_store::write_json(&self.session_path(&record.id), &record)
            .expect("persist approval session");

        // 阶段二（当前 manager）：开门必须发生在真正执行 approve 的引擎上。
        let manager = self.current_registry_manager().await;
        let engine_arc = manager.engine();
        let mut engine = engine_arc.lock().await;
        engine.session.artifact = Some(gate_outline_candidate_payload());
        engine.session.stage = crate::product::workspace_engine::WorkspaceStage::HumanConfirm;
        engine.session.session_status = WorkspaceSessionStatus::WaitingForHuman;
        engine
            .enter_human_confirm(Some("人工批准（审批门）".to_string()))
            .await;
        let gate_id = engine.active_timeline_node_id().expect("gate node");
        drop(engine);
        self.gate_id = Some(gate_id.clone());
        gate_id
    }

    /// 当前 registry 解析的 manager（HTTP handler 同一口径）。
    async fn current_registry_manager(&self) -> Arc<WorkspaceSessionManager> {
        let state = self.state.clone();
        let session_id = self.session_id.clone();
        state
            .workspace_sessions
            .get_or_create(&session_id, || {
                WorkspaceSessionManager::create(&state, &session_id)
            })
            .await
            .expect("current registry manager")
    }

    /// Task 10 campaign：人手干净批准（无 failpoint）——铺审批门后单次
    /// P0 REST Approve 即 Confirmed + Completed + plan Confirmed。
    pub(crate) async fn confirm_plan_by_human(&mut self) {
        let gate_id = self.seed_approval_gate().await;
        let app = crate::web::app::build_web_router(self.state.clone());
        let body = serde_json::json!({
            "type": "approve",
            "command_id": "cmd-campaign-approve",
            "expected_gate_id": gate_id,
        });
        let (status, payload) = post_human_action(&app, &self.session_id, &body).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "clean campaign approve must succeed: {status} {payload}"
        );
        assert_eq!(payload["state"], "accepted", "{payload}");
        let durable = self.durable();
        assert_eq!(durable.status, WorkspaceSessionStatus::Confirmed);
        assert_eq!(
            durable.single_candidate_phase,
            Some(SingleCandidatePhase::Completed)
        );
    }

    /// Task 10 campaign：Evaluate 相位 reviewer 经真实引擎跑出网关 503
    /// 失败（`drive_reviewer_provider_session` 消费 `ProviderUnavailable`
    /// 错误会话），返回失败 timeline 节点 id。前置：先收尾 fixture 的
    /// author run，让 manager 空闲（编排器观察面与人工重驱入口一致）。
    pub(crate) async fn fail_reviewer_with_gateway_503(&mut self) -> String {
        self.manager.finish_run(self.token).await;
        // 相位记录的 refs 持久化需要 accepted drafts + active index 先就位。
        self.seed_accepted_plan_drafts().await;
        let engine_arc = self.manager.engine();
        let mut engine = engine_arc.lock().await;
        single_candidate_recovery_record(
            &self.lifecycle,
            &mut engine,
            SingleCandidatePhase::Evaluate,
            RunPolicy::Interactive,
        );
        engine.start_review().await;
        let failed_node_id = engine.active_timeline_node_id().expect("reviewer node");
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        engine
                .drive_reviewer_provider_session(
                    Err(crate::cross_cutting::provider_adapter::ProviderAdapterError::provider_unavailable(
                        "503 No available accounts; Authorization: Bearer secret",
                    )),
                    rx,
                    crate::product::models::ProviderName::ClaudeCode,
                )
                .await;
        drop(engine);
        let durable = self.durable();
        assert_eq!(durable.status, WorkspaceSessionStatus::Failed);
        assert_eq!(
            durable.single_candidate_phase,
            Some(SingleCandidatePhase::Failed)
        );
        failed_node_id
    }

    /// 绑定 session 的全部 durable timeline 节点（503 诊断可见性断言）。
    pub(crate) fn timeline_nodes(&self) -> Vec<crate::web::workspace_ws_types::TimelineNode> {
        self.lifecycle
            .load_timeline_nodes_for_issue_session(PROJECT_ID, ISSUE_ID, &self.session_id)
            .expect("timeline nodes")
    }

    /// 制造「compile 崩溃后人工恢复」的 durable 现场并停在开着的门上：
    /// 1) ProvenancePersisted 边界（落 approval+reservation，compile tx
    ///    尚未落盘——不落 recovery 节点的 422 根因）；
    /// 2) 按 part_03/part_09.rs:451、single_candidate_recovery.rs:700 先例
    ///    注册 FirstChildBindingEnsured finalizer failpoint，第二次 approve
    ///    replay 复用同 compile_id 抵达 finalizer（tx 已落盘）→ Err 分支
    ///    自动标记 RecoveryRequired 并落 recovery 节点，供人工
    ///    CompileRecovery Continue 恢复原事务。
    pub(crate) async fn fail_compile_after_human_approve(&mut self) {
        let gate_id = self.seed_approval_gate().await;
        // 开门后的同一引擎上注册 ProvenancePersisted failpoint（与原始
        // 内联版本等价：同 manager 单引擎、无并发写者）。
        let manager = self.current_registry_manager().await;
        let engine_arc = manager.engine();
        let engine = engine_arc.lock().await;
        let _failpoint = engine.register_single_candidate_compile_failpoint(
            SingleCandidateCompileCheckpoint::ProvenancePersisted,
        );
        drop(engine);

        let app = crate::web::app::build_web_router(self.state.clone());
        let body = serde_json::json!({
            "type": "approve",
            "command_id": "cmd-approve-failpoint",
            "expected_gate_id": gate_id,
        });
        let (status, payload) = post_human_action(&app, &self.session_id, &body).await;
        assert!(
            !status.is_success(),
            "failpoint approve must not succeed: {status} {payload}"
        );
        let message = payload["message"].as_str().unwrap_or_default();
        assert!(
            payload["code"] == "single_candidate_approval_compile_failed"
                || message.contains("compile failed; human gate remains open"),
            "failpoint approve must fail at the compile finalizer: {payload}"
        );
        let durable = self.durable();
        assert_ne!(durable.status, WorkspaceSessionStatus::Confirmed);
        assert_ne!(
            durable.single_candidate_phase,
            Some(SingleCandidatePhase::Completed)
        );

        // 阶段三（finalizer 边界 + replay）：ProvenancePersisted 后 compile
        // tx 未落盘，上面这次失败不落 recovery 节点（REST CompileRecovery
        // 422 根因）。从 durable reservation 读 compile_id 注册 finalizer
        // failpoint；第二次 approve 走 replay 复用同一 compile_id 抵达
        // finalizer（tx 已在绑定光标处落盘）→ Err 分支自动标记
        // RecoveryRequired 并落 recovery 节点。
        let compile_id = self
            .durable()
            .compile_reservation
            .as_ref()
            .expect("provenance boundary must leave durable compile reservation")
            .compile_id
            .clone();
        let replay_gate_id = {
            let engine = manager.engine();
            let engine = engine.lock().await;
            engine
                .active_timeline_node_id()
                .expect("reopened gate node after provenance failure")
        };
        let finalizer_failpoint = {
            let engine = manager.engine();
            let engine = engine.lock().await;
            engine.register_work_item_plan_compile_finalizer_failpoint(
                &compile_id,
                WorkItemPlanCompileFinalizerCheckpoint::FirstChildBindingEnsured,
            )
        };
        let replay_body = serde_json::json!({
            "type": "approve",
            "command_id": "cmd-approve-finalizer-failpoint",
            "expected_gate_id": replay_gate_id,
        });
        let (status, payload) = post_human_action(&app, &self.session_id, &replay_body).await;
        drop(finalizer_failpoint);
        assert!(
            !status.is_success(),
            "finalizer failpoint approve must not succeed: {status} {payload}"
        );
        let message = payload["message"].as_str().unwrap_or_default();
        assert!(
            payload["code"] == "single_candidate_approval_compile_failed"
                || message.contains("compile failed; human gate remains open"),
            "replay approve must fail at the finalizer failpoint: {payload}"
        );
        let durable = self.durable();
        assert_ne!(durable.status, WorkspaceSessionStatus::Confirmed);
        assert_eq!(
            durable.status,
            WorkspaceSessionStatus::WaitingForHuman,
            "finalizer crash must leave the gate open for humans"
        );
        let recovery_gate_id = {
            let engine = manager.engine();
            let engine = engine.lock().await;
            engine
                .active_timeline_node_id()
                .expect("recovery node after finalizer crash")
        };
        assert_ne!(
            recovery_gate_id, replay_gate_id,
            "finalizer crash must activate a compile recovery node"
        );
    }

    /// 人工 CompileRecovery Continue（P0 REST）恢复原事务并提交，再经
    /// 人工 Approve 关门——终态 Confirmed + Completed + plan Confirmed。
    /// 前置：`fail_compile_after_human_approve` 已落 recovery 节点。
    pub(crate) async fn recover_and_confirm_compile(&self) {
        let state = self.state.clone();
        let session_id = self.session_id.clone();
        let manager = state
            .workspace_sessions
            .get_or_create(&session_id, || {
                WorkspaceSessionManager::create(&state, &session_id)
            })
            .await
            .expect("current registry manager");
        let recovery_gate_id = {
            let engine = manager.engine();
            let engine = engine.lock().await;
            engine
                .active_timeline_node_id()
                .expect("recovery gate node")
        };
        let app = crate::web::app::build_web_router(state.clone());
        let recovery_body = serde_json::json!({
            "type": "compile_recovery",
            "command_id": "cmd-recovery-continue",
            "expected_gate_id": recovery_gate_id,
            "action": "continue",
            "reason": null,
        });
        let (status, payload) = post_human_action(&app, &session_id, &recovery_body).await;
        assert_eq!(status, StatusCode::OK, "recovery continue: {payload}");
        assert_eq!(payload["state"], "accepted", "{payload}");

        // 恢复后门重新开启：再次人工 Approve（failpoint 已释放）→ Confirmed。
        let confirm_gate_id = {
            let engine = manager.engine();
            let engine = engine.lock().await;
            engine
                .active_timeline_node_id()
                .expect("reopened gate node")
        };
        let approve_body = serde_json::json!({
            "type": "approve",
            "command_id": "cmd-approve-after-recovery",
            "expected_gate_id": confirm_gate_id,
        });
        let (status, payload) = post_human_action(&app, &session_id, &approve_body).await;
        assert_eq!(status, StatusCode::OK, "approve after recovery: {payload}");
        assert_eq!(payload["state"], "accepted", "{payload}");
        let durable = self.durable();
        assert_eq!(durable.status, WorkspaceSessionStatus::Confirmed);
        assert_eq!(
            durable.single_candidate_phase,
            Some(SingleCandidatePhase::Completed)
        );
    }

    /// 人工 Abandon（P0 REST）：终态后编排器零复活、零额外 provider。
    pub(crate) async fn abandon_via_rest(&self) {
        // 失败 approve 会重开门节点：以「当前 registry 引擎」的活跃门为准
        //（与 HTTP handler 同一解析口径，人手视角）。
        let state = self.state.clone();
        let session_id = self.session_id.clone();
        let manager = state
            .workspace_sessions
            .get_or_create(&session_id, || {
                WorkspaceSessionManager::create(&state, &session_id)
            })
            .await
            .expect("current registry manager");
        let gate_id = {
            let engine = manager.engine();
            let engine = engine.lock().await;
            engine
                .active_timeline_node_id()
                .expect("reopened gate node")
        };
        let app = crate::web::app::build_web_router(self.state.clone());
        let body = serde_json::json!({
            "type": "abandon",
            "command_id": "cmd-abandon-terminal",
            "expected_gate_id": gate_id,
        });
        let (status, payload) = post_human_action(&app, &self.session_id, &body).await;
        assert_eq!(status, StatusCode::OK, "{payload}");
        assert_eq!(payload["state"], "accepted", "{payload}");
        assert_eq!(self.durable().status, WorkspaceSessionStatus::Terminated);
    }

    /// 当前 durable enrollment（真实 IssueAutomationStore 读取）。
    pub(crate) fn enrollment(
        &self,
    ) -> crate::product::models::automation::IssueAutomationEnrollment {
        IssueAutomationStore::new(self.inner.paths.clone())
            .get(PROJECT_ID, ISSUE_ID)
            .expect("load enrollment")
            .expect("enrollment present")
    }

    /// issue 下全部真实 coding attempt（CodingAttemptStore 读取）。
    pub(crate) fn coding_attempts(
        &self,
    ) -> Vec<crate::product::coding_models::CodingExecutionAttempt> {
        crate::product::coding_attempt_store::CodingAttemptStore::new(self.inner.paths.clone())
            .list_attempts_for_issue(PROJECT_ID, ISSUE_ID)
            .expect("list coding attempts")
    }

    /// 进程内已启动 coding runner 总数（真实 registry 事实，不含注册即撤）。
    pub(crate) fn coding_runner_count(&self) -> usize {
        self.coding_attempts()
            .iter()
            .map(|attempt| {
                self.state.coding_runs.runner_count(
                    &crate::web::state::CodingAttemptRunKey::from_attempt(attempt),
                )
            })
            .sum()
    }

    /// issue 下唯一 coding attempt（P2 Task 3+ 共享）：fixture 现场恰一
    /// attempt，多于一条即 fixture 构造编程错误。
    pub(crate) fn attempt(&self) -> crate::product::coding_models::CodingExecutionAttempt {
        let attempts = self.coding_attempts();
        assert_eq!(
            attempts.len(),
            1,
            "fixture expects exactly one coding attempt"
        );
        attempts.into_iter().next().unwrap()
    }

    /// 指定 attempt 的进程内 runner 数（真实 registry 事实）。
    pub(crate) fn runner_count(&self, key: &crate::web::state::CodingAttemptRunKey) -> usize {
        self.state.coding_runs.runner_count(key)
    }

    /// 真实 attempt store（Task 4+ 共享）。
    pub(crate) fn store(&self) -> crate::product::coding_attempt_store::CodingAttemptStore {
        crate::product::coding_attempt_store::CodingAttemptStore::new(self.inner.paths.clone())
    }

    /// 当前唯一 attempt 的 registry key（Task 4/6 共享）。
    pub(crate) fn attempt_key(&self) -> crate::web::state::CodingAttemptRunKey {
        crate::web::state::CodingAttemptRunKey::from_attempt(&self.attempt())
    }

    /// 绑定 plan id（Task 4/6 共享）。
    pub(crate) fn plan_id(&self) -> String {
        self.enrollment().plan_id.expect("bound plan")
    }

    /// 当前 enrollment 的自动首启 origin（冻结 policy 同源身份）。
    pub(crate) fn auto_origin(&self) -> crate::product::coding_models::CodingStartOrigin {
        let enrollment = self.enrollment();
        crate::product::coding_models::CodingStartOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision,
            binding_version: enrollment
                .binding_history
                .as_ref()
                .map(|history| history.current.binding_version),
            target: enrollment.target.clone(),
        }
    }

    /// Task 8 共享：复用原 `prepare_group_final_confirm_from_readiness`
    /// 重复准备（readiness 重写/节点复用由引擎真实行为决定）。
    pub(crate) async fn prepare_group_final_confirm_again(&self) {
        let attempt = self.attempt();
        let event_tx = self.state.coding_sockets.hub_sender(&self.attempt_key());
        let engine = crate::product::coding_workspace_engine::CodingWorkspaceEngine::new(
            self.store(),
            crate::product::git_workspace_service::GitWorkspaceService::new(),
            event_tx,
        );
        engine
            .prepare_group_final_confirm_from_readiness(&attempt)
            .await
            .expect("repeat group final confirm preparation");
    }

    /// Task 8 共享：人手 `handle_final_confirm`（不代点、不直改状态）。
    pub(crate) async fn confirm_final_by_human(&self) {
        let attempt = self.attempt();
        let event_tx = self.state.coding_sockets.hub_sender(&self.attempt_key());
        let engine = crate::product::coding_workspace_engine::CodingWorkspaceEngine::new(
            self.store(),
            crate::product::git_workspace_service::GitWorkspaceService::new(),
            event_tx,
        );
        engine
            .handle_final_confirm(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .await
            .expect("human final confirm");
    }

    /// 「重启」进程替身：同 `.aria`、全新 WebAppState/runtime（内存态清零）。
    /// 与 new() 同源重建恒健康 gateway factory（重启后 gateway 链路同等可用）。
    pub(crate) fn restart_state(&self) -> WebAppState {
        fake_state_with_gateway(self.inner._root.path().to_path_buf())
    }

    pub(crate) fn session_path(&self, session_id: &str) -> std::path::PathBuf {
        self.inner
            .paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("workspace-sessions")
            .join(format!("{session_id}.json"))
    }
}

include!("wiga_gate_fixture/prepared_fixtures.inc.rs");

/// provider 等待者：真实消费 run command 通道，choice 应答即投递回执。
pub(crate) async fn deliver_choice_receipts(mut rx: tokio::sync::mpsc::Receiver<ProviderCommand>) {
    while let Some(command) = rx.recv().await {
        if let ProviderCommand::ChoiceResponse {
            receipt: Some(receipt),
            ..
        } = command
        {
            receipt.mark_resolving();
            receipt.deliver();
        }
    }
}

pub(crate) fn gate_choice_frame(id: &str) -> WsOutMessage {
    WsOutMessage::ChoiceRequest {
        id: id.to_string(),
        prompt: "验收口径歧义需要用户裁定".to_string(),
        options: vec![ChoiceOption {
            id: "option-a".to_string(),
            label: "按全局口径".to_string(),
            description: None,
        }],
        allow_multiple: false,
        allow_free_text: false,
        questions: Vec::new(),
        source: "ask_user_question".to_string(),
    }
}

fn gate_outline() -> WorkItemPlanOutline {
    let item = |outline_id: &str, kind: WorkItemKind, scope: &str| WorkItemOutline {
        target_repository_id: None,
        outline_id: outline_id.to_string(),
        logical_work_item_id: format!("wi_{}", outline_id.strip_prefix("outline_").unwrap()),
        title: outline_id.to_string(),
        kind,
        goal: outline_id.to_string(),
        scope: vec![scope.to_string()],
        non_goals: Vec::new(),
        estimated_context_tokens: Some(12_000),
        session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
        source_story_spec_ids: vec!["story_001".to_string()],
        source_design_spec_ids: vec!["design_001".to_string()],
        exclusive_write_scopes: vec![scope.to_string()],
        forbidden_write_scopes: Vec::new(),
        depends_on: Vec::new(),
        verification_intent: vec![format!("cargo test --locked --lib {outline_id}")],
        trusted_verification_commands: Vec::new(),
        handoff_notes: format!("handoff {outline_id}"),
    };
    let mut outline = WorkItemPlanOutline {
        id: "outline_001".to_string(),
        project_id: "project_001".to_string(),
        issue_id: "issue_001".to_string(),
        source_story_spec_ids: vec!["story_001".to_string()],
        source_design_spec_ids: vec!["design_001".to_string()],
        strategy_summary: "test strategy".to_string(),
        work_item_outlines: vec![
            item("outline_a", WorkItemKind::Backend, "src/a.rs"),
            item("outline_b", WorkItemKind::Frontend, "web/b.ts"),
        ],
        dependency_graph: vec![WorkItemOutlineDependencyEdge {
            from_outline_id: "outline_a".to_string(),
            to_outline_id: "outline_b".to_string(),
        }],
        risks: Vec::new(),
        handoff_strategy: "handoff".to_string(),
        status: "draft".to_string(),
    };
    outline.work_item_outlines[1].depends_on = vec!["outline_a".to_string()];
    outline
}

pub(crate) fn gate_outline_candidate_payload() -> ArtifactPayload {
    ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: gate_outline(),
            design_context_gaps: vec![],
            validator_findings: vec![],
            context_blockers: vec![],
            current_generation_round_id: Some("round_0001".to_string()),
            selected_generation_mode: Some(WorkItemGenerationModeDto::Serial),
        }),
    }
}

pub(crate) fn gate_draft_record(
    plan_id: &str,
    outline_id: &str,
    draft_id: &str,
) -> WorkItemDraftRecord {
    let now = chrono::Utc::now().to_rfc3339();
    let logical_work_item_id = format!(
        "wi_{}",
        outline_id.strip_prefix("outline_").unwrap_or(outline_id)
    );
    let mut contract =
        crate::product::work_item_contract::canonical_contract_fixture(&logical_work_item_id);
    contract.identity.title = format!("{outline_id} draft");
    contract.identity.kind = "backend".to_string();
    contract.goal.summary = format!("实现 {outline_id}");
    contract.input_contracts.clear();
    contract.handoff_contract.provided_contract_refs.clear();
    contract.write_policy.exclusive_scopes = vec![format!("src/{outline_id}.rs")];
    contract.write_policy.forbidden_scopes.clear();
    contract.verification_checks[0].check_id = format!("cmd_{outline_id}");
    contract.verification_checks[0].command =
        Some(format!("cargo test --locked --lib {outline_id}"));
    WorkItemDraftRecord {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: plan_id.to_string(),
        draft_id: draft_id.to_string(),
        outline_id: outline_id.to_string(),
        generation_round_id: "round_0001".to_string(),
        batch_id: None,
        attempt_index: 1,
        outline_version_ref: "outline_001".to_string(),
        generation_mode: WorkItemGenerationMode::Serial,
        generation_diagnostics: None,
        candidate: crate::product::models::WorkItemDraftCandidate {
            target_repository_id: None,
            outline_id: outline_id.to_string(),
            logical_work_item_id,
            verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                checks: contract.verification_checks.clone(),
            },
            canonical_contract_candidate: contract,
        },
        status: WorkItemDraftStatus::Accepted,
        active: true,
        superseded_by_draft_id: None,
        supersede_reason: None,
        copied_from_draft_id: None,
        review_node_id: None,
        review_verdict_ref: None,
        generated_from_node_id: "timeline_node_draft".to_string(),
        accepted_at: Some(now.clone()),
        superseded_at: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

pub(crate) fn gate_active_index(plan_id: &str) -> WorkItemPlanDraftActiveIndex {
    WorkItemPlanDraftActiveIndex {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: plan_id.to_string(),
        current_generation_round_id: "round_0001".to_string(),
        outline_state: "confirmed".to_string(),
        active_outline_id: None,
        outline_to_current_draft_id: BTreeMap::from([
            ("outline_a".to_string(), "draft_outline_a".to_string()),
            ("outline_b".to_string(), "draft_outline_b".to_string()),
        ]),
        draft_statuses: BTreeMap::from([
            ("draft_outline_a".to_string(), WorkItemDraftStatus::Accepted),
            ("draft_outline_b".to_string(), WorkItemDraftStatus::Accepted),
        ]),
        batches: vec![],
        updated_at: chrono::Utc::now().to_rfc3339(),
    }
}

/// 批 2 R4/E1：四仓 plan outline——wi_a→member-1、wi_b→member-2（依赖
/// wi_a，跨仓依赖）、wi_c→member-3（独立）；member-4 选中但无 WI（授权
/// 集 A⊋实际桶集 P，验收「选中无 WI 仓不造空 attempt」）。
fn four_repository_outline() -> WorkItemPlanOutline {
    let item = |outline_id: &str, kind: WorkItemKind, scope: &str, target: u128| WorkItemOutline {
        target_repository_id: Some(crate::product::logical_codebase::LogicalRepositoryId(
            uuid::Uuid::from_u128(target),
        )),
        outline_id: outline_id.to_string(),
        logical_work_item_id: format!("wi_{}", outline_id.strip_prefix("outline_").unwrap()),
        title: outline_id.to_string(),
        kind,
        goal: outline_id.to_string(),
        scope: vec![scope.to_string()],
        non_goals: Vec::new(),
        estimated_context_tokens: Some(12_000),
        session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
        source_story_spec_ids: vec!["story_001".to_string()],
        source_design_spec_ids: vec!["design_001".to_string()],
        exclusive_write_scopes: vec![scope.to_string()],
        forbidden_write_scopes: Vec::new(),
        depends_on: Vec::new(),
        verification_intent: vec![format!("cargo test --locked --lib {outline_id}")],
        trusted_verification_commands: Vec::new(),
        handoff_notes: format!("handoff {outline_id}"),
    };
    let mut outline = WorkItemPlanOutline {
        id: "outline_001".to_string(),
        project_id: "project_001".to_string(),
        issue_id: "issue_001".to_string(),
        source_story_spec_ids: vec!["story_001".to_string()],
        source_design_spec_ids: vec!["design_001".to_string()],
        strategy_summary: "four repository test strategy".to_string(),
        work_item_outlines: vec![
            item("outline_a", WorkItemKind::Backend, "src/a.rs", 1),
            item("outline_b", WorkItemKind::Backend, "src/b.rs", 2),
            item("outline_c", WorkItemKind::Backend, "src/c.rs", 3),
        ],
        dependency_graph: vec![WorkItemOutlineDependencyEdge {
            from_outline_id: "outline_a".to_string(),
            to_outline_id: "outline_b".to_string(),
        }],
        risks: Vec::new(),
        handoff_strategy: "handoff".to_string(),
        status: "draft".to_string(),
    };
    outline.work_item_outlines[1].depends_on = vec!["outline_a".to_string()];
    outline
}

pub(crate) fn four_repository_outline_candidate_payload() -> ArtifactPayload {
    ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: four_repository_outline(),
            design_context_gaps: vec![],
            validator_findings: vec![],
            context_blockers: vec![],
            current_generation_round_id: Some("round_0001".to_string()),
            selected_generation_mode: Some(WorkItemGenerationModeDto::Serial),
        }),
    }
}

/// 批 2 R4/E1：成员归属 draft——contract 携带 `intent_contract`（
/// LogicalCodebase 恰一成员，R1 归属语义），candidate target 同成员；
/// 下游 draft 额外要求上游 output contract（跨仓依赖交付）。
pub(crate) fn gate_draft_record_with_member(
    plan_id: &str,
    outline_id: &str,
    draft_id: &str,
    logical_codebase_id: &str,
    member: crate::product::logical_codebase::LogicalRepositoryId,
    depends_on: Vec<String>,
) -> WorkItemDraftRecord {
    let mut record = gate_draft_record(plan_id, outline_id, draft_id);
    let logical_work_item_id = record.candidate.logical_work_item_id.clone();
    let contract = &mut record.candidate.canonical_contract_candidate;
    contract.intent_contract = Some(crate::product::work_item_contract::WorkItemIntentContract {
        intent: crate::product::work_item_contract::WorkItemIntent::Create,
        provider_work_item_id: logical_work_item_id.clone(),
        depends_on,
        exclusive_scopes: contract.write_policy.exclusive_scopes.clone(),
        forbidden_scopes: contract.write_policy.forbidden_scopes.clone(),
        target: crate::product::logical_codebase::EnrollmentTarget::LogicalCodebase {
            logical_codebase_id: logical_codebase_id.to_string(),
            logical_repository_id: member,
        },
    });
    record.candidate.target_repository_id = Some(member);
    record
}

pub(crate) fn four_repository_active_index(plan_id: &str) -> WorkItemPlanDraftActiveIndex {
    WorkItemPlanDraftActiveIndex {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: plan_id.to_string(),
        current_generation_round_id: "round_0001".to_string(),
        outline_state: "confirmed".to_string(),
        active_outline_id: None,
        outline_to_current_draft_id: BTreeMap::from([
            ("outline_a".to_string(), "draft_outline_a".to_string()),
            ("outline_b".to_string(), "draft_outline_b".to_string()),
            ("outline_c".to_string(), "draft_outline_c".to_string()),
        ]),
        draft_statuses: BTreeMap::from([
            ("draft_outline_a".to_string(), WorkItemDraftStatus::Accepted),
            ("draft_outline_b".to_string(), WorkItemDraftStatus::Accepted),
            ("draft_outline_c".to_string(), WorkItemDraftStatus::Accepted),
        ]),
        batches: vec![],
        updated_at: chrono::Utc::now().to_rfc3339(),
    }
}

pub(crate) async fn post_json(
    app: &axum::Router,
    uri: String,
    body: &serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

pub(crate) async fn post_choice(
    app: &axum::Router,
    session_id: &str,
    choice_id: &str,
    body: &serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    post_json(
        app,
        format!("/api/workspace-sessions/{session_id}/choices/{choice_id}/response"),
        body,
    )
    .await
}

pub(crate) async fn post_human_action(
    app: &axum::Router,
    session_id: &str,
    body: &serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    post_json(
        app,
        format!("/api/workspace-sessions/{session_id}/human-actions"),
        body,
    )
    .await
}
