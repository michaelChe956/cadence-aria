//! P1 WIGA Task 6：薄编排器——从 durable 事实 reconcile，启动扫描与有界
//! 漏唤醒补偿（REQ-WIGA-03）。
//!
//! 事件（PUT 成功等）只是 wake hint；权威推进只来自每轮 `reconcile` 重读
//! durable enrollment/intent/plan/session。有状态部分仅限公平游标；动作与
//! 进度一律由 Task 4/5 的存储与生成面推导。

use std::collections::BTreeMap;

use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::issue_store::IssueStore;
use crate::product::project_store::ProjectStore;
use crate::web::handlers::lifecycle::plan_preparation::ensure_enrolled_plan;
use crate::web::plan_generation::{PlanGenerationOutcome, start_plan_generation_once};
use crate::web::state::WebAppState;

/// 单次 reconcile 的 durable 推导结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileOutcome {
    /// 无 enrollment / 已禁用：后台零动作。
    NoEnrollment,
    /// 停等人（choice/门/compile recovery/WaitingForHuman）。
    AwaitingHuman,
    /// 绑定链就绪（plan/session 已唯一创建并绑定）。
    Prepared,
    /// 生成动作在途或已派发（含活 run 只观察）。
    Generating,
    /// fail-closed 停点（授权漂移/损坏/不可证明的分诊），只待人。
    NeedsHuman,
}

/// 有界 tick 配置；缺省 2s 间隔、每轮最多 32 issue。
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub tick_interval: std::time::Duration,
    pub max_issues_per_tick: usize,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            tick_interval: std::time::Duration::from_secs(2),
            max_issues_per_tick: 32,
        }
    }
}

pub struct AutopilotOrchestrator {
    state: WebAppState,
    config: OrchestratorConfig,
    /// 公平游标：上一轮处理到的 (project_id, issue_id)；满轮后归零重扫。
    cursor: tokio::sync::Mutex<Option<(String, String)>>,
}

impl AutopilotOrchestrator {
    pub fn new(state: WebAppState, config: OrchestratorConfig) -> Self {
        Self {
            state,
            config,
            cursor: tokio::sync::Mutex::new(None),
        }
    }

    /// 单 issue reconcile：每次读 fresh enrollment，缺失/禁用 → NoEnrollment；
    /// 无绑定先 EnsurePreparedPlan（幂等补偿），再按 durable 停点分诊
    /// StartPlanGeneration。授权漂移/损坏 fail-closed 为 NeedsHuman，
    /// 不猜最近 plan。
    pub async fn reconcile(
        &self,
        state: &WebAppState,
        project_id: &str,
        issue_id: &str,
    ) -> Result<ReconcileOutcome, String> {
        let paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
        let store = IssueAutomationStore::new(paths);
        let Some(enrollment) = store
            .get(project_id, issue_id)
            .map_err(|error| format!("automation enrollment unreadable: {error}"))?
        else {
            return Ok(ReconcileOutcome::NoEnrollment);
        };
        if !enrollment.enabled {
            return Ok(ReconcileOutcome::NoEnrollment);
        }

        // EnsurePreparedPlan（幂等）：锁内唯一创建/绑定；源/目标/意图漂移由
        // web 回调与冻结 intent fail-closed（不在此重复校验）。
        if let Err(error) = ensure_enrolled_plan(state, &enrollment).await {
            return Ok(match error.code.as_str() {
                "automation_enrollment_conflict" | "automation_enrollment_invalid_scope" => {
                    ReconcileOutcome::NeedsHuman
                }
                _ => {
                    return Err(format!(
                        "ensure enrolled plan failed: {}: {}",
                        error.code, error.message
                    ))
                }
            });
        }

        // 真正发起前重读 fresh enrollment（bind revision+1；Disable 竞态收口）。
        let fresh = store
            .get(project_id, issue_id)
            .map_err(|error| format!("automation enrollment unreadable: {error}"))?
            .ok_or_else(|| "automation enrollment vanished after preparation".to_string())?;
        if !fresh.enabled {
            return Ok(ReconcileOutcome::NoEnrollment);
        }
        // P1 WIGA Task 7：人工停点判定——durable 最近 compile Failed/
        // RecoveryRequired、开启中的人工门轮次、provider run 挂起的 choice
        // 都不推进下一动作，只等待 P0 REST 人手解除（REQ-WIGA-02/REQ-CG-04）。
        if let Some(outcome) = human_stop_point(state, &fresh).await? {
            return Ok(outcome);
        }
        match start_plan_generation_once(state, &fresh).await? {
            PlanGenerationOutcome::Running | PlanGenerationOutcome::AlreadyActive => {
                Ok(ReconcileOutcome::Generating)
            }
            PlanGenerationOutcome::WaitingForHuman => Ok(ReconcileOutcome::AwaitingHuman),
            PlanGenerationOutcome::NeedsHuman => Ok(ReconcileOutcome::NeedsHuman),
        }
    }

    /// 一轮有界扫描：ProjectStore::list → IssueStore::list，按稳定序从公平
    /// 游标推进，每轮最多 `max_issues_per_tick` 个 issue；单 issue 错误留下
    /// 可见诊断并继续扫描其余 issue（不吞读取错误为 off）。返回处理数。
    pub async fn reconcile_all_once(&self) -> Result<usize, String> {
        let paths = ProductAppPaths::new(self.state.workspace_root.join(".aria"));
        let projects = ProjectStore::new(paths.clone())
            .list()
            .map_err(|error| format!("project list unreadable: {error}"))?;
        let mut ordered: Vec<(String, String)> = Vec::new();
        let mut issues_by_project = BTreeMap::new();
        for project in &projects {
            let issues = IssueStore::new(paths.clone())
                .list(&project.id)
                .map_err(|error| format!("issue list unreadable for {}: {error}", project.id))?;
            issues_by_project.insert(project.id.clone(), issues);
        }
        for (project_id, issues) in &issues_by_project {
            for issue in issues {
                ordered.push((project_id.clone(), issue.id.clone()));
            }
        }

        let start_index = {
            let cursor = self.cursor.lock().await;
            match &*cursor {
                Some((project_id, issue_id)) => ordered
                    .iter()
                    .position(|(p, i)| p == project_id && i == issue_id)
                    .map(|position| position + 1)
                    .unwrap_or(0),
                None => 0,
            }
        };
        let budget = self.config.max_issues_per_tick.min(ordered.len());
        let mut processed = 0usize;
        let mut last: Option<(String, String)> = None;
        for offset in 0..budget {
            let index = (start_index + offset) % ordered.len();
            let (project_id, issue_id) = &ordered[index];
            if let Err(message) = self.reconcile(&self.state, project_id, issue_id).await {
                eprintln!("wiga autopilot reconcile failed for {project_id}/{issue_id}: {message}");
            }
            processed += 1;
            last = Some((project_id.clone(), issue_id.clone()));
        }
        let mut cursor = self.cursor.lock().await;
        // 满轮（本轮预算未用尽或已绕回起点）→ 游标归零重扫。
        let wrapped = budget < self.config.max_issues_per_tick
            || start_index + budget >= ordered.len();
        *cursor = if wrapped { None } else { last };
        Ok(processed)
    }

    /// 后台有界 tick：由 `serve_web` 持有，随服务器生命周期运行；PUT 成功的
    /// 唤醒只提前触发下一轮，不提供顺序承诺（漏唤醒由固定间隔兜底）。
    pub async fn run(self: std::sync::Arc<Self>, mut wake: tokio::sync::watch::Receiver<bool>) {
        loop {
            let interval = self.config.tick_interval;
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = wake.changed() => {}
            }
            if let Err(message) = self.reconcile_all_once().await {
                eprintln!("wiga autopilot scan round failed: {message}");
            }
        }
    }
}

/// P1 WIGA Task 7：人工停点判定。只读 durable session/compile 事务/人工门
/// 轮次与 manager 挂起 choice；任一停点返回 `AwaitingHuman`，后台不发下一
/// 动作，解除只经 P0 REST 人手。终态（Confirmed/Failed/Terminated 等）不
/// 在此分诊，交由生成准入按 NeedsHuman 收口。读取失败显式上抛，不吞为 off。
async fn human_stop_point(
    state: &WebAppState,
    enrollment: &crate::product::models::automation::IssueAutomationEnrollment,
) -> Result<Option<ReconcileOutcome>, String> {
    use crate::product::models::HumanGateTurnStatus;
    use crate::product::models::WorkspaceSessionStatus;
    use crate::product::models::outline::WorkItemPlanCompileStatus;

    let Some(session_id) = enrollment.session_id.clone() else {
        return Ok(None);
    };
    let paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(paths.clone());
    let session = lifecycle
        .get_workspace_session(&session_id)
        .map_err(|error| format!("bound session unreadable: {error}"))?;
    // 终态交给生成准入（NeedsHuman 分诊），不在此用人手停点遮蔽。
    if matches!(
        session.status,
        WorkspaceSessionStatus::Confirmed
            | WorkspaceSessionStatus::Failed
            | WorkspaceSessionStatus::Terminated
            | WorkspaceSessionStatus::ChangeRequested
            | WorkspaceSessionStatus::BlockedProviderUnavailable
    ) {
        return Ok(None);
    }
    // 最近 compile 事务 Failed/RecoveryRequired：compile recovery 等人处理。
    if let Some(plan_id) = enrollment.plan_id.as_deref() {
        let transactions = crate::product::work_item_plan_store::WorkItemPlanStore::new(
            paths.clone(),
        )
        .list_compile_transactions(&session.project_id, &session.issue_id, plan_id)
        .map_err(|error| format!("bound plan compile transactions unreadable: {error}"))?;
        let latest_failed = transactions
            .iter()
            .max_by(|left, right| left.created_at.cmp(&right.created_at))
            .is_some_and(|latest| {
                matches!(
                    latest.status,
                    WorkItemPlanCompileStatus::Failed
                        | WorkItemPlanCompileStatus::RecoveryRequired
                )
            });
        if latest_failed {
            return Ok(Some(ReconcileOutcome::AwaitingHuman));
        }
    }
    // 开启中的人工门轮次（Reserved/Running）：修订 run 由人手驱动，不代跑。
    let open_turn = lifecycle
        .list_human_gate_turns(&session_id)
        .map_err(|error| format!("human gate turns unreadable: {error}"))?
        .iter()
        .any(|turn| {
            matches!(
                turn.status,
                HumanGateTurnStatus::Reserved | HumanGateTurnStatus::Running
            )
        });
    if open_turn {
        return Ok(Some(ReconcileOutcome::AwaitingHuman));
    }
    // provider run 挂起的 choice（等待人答复）：只观察，不代答。
    let manager = state
        .workspace_sessions
        .get_or_create(&session_id, || {
            crate::web::workspace_session::WorkspaceSessionManager::create(state, &session_id)
        })
        .await
        .map_err(|error| format!("bound session manager unavailable: {error}"))?;
    if !manager.pending_choice_frames().is_empty() {
        return Ok(Some(ReconcileOutcome::AwaitingHuman));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;
    use crate::web::handlers::automation_enrollment_test_support::{
        ISSUE_ID, PROJECT_ID, enrollment_body, put_enrollment, response_json, seed_fixture,
    };
    use crate::web::state::WebAppState;

    struct OrchestratorFixture {
        inner: crate::web::handlers::automation_enrollment_test_support::Fixture,
        state: WebAppState,
        manual_issue_id: String,
    }

    impl OrchestratorFixture {
        /// 真实播种：project + issue + 单成员 logical codebase + 已确认
        /// story/design + enabled enrollment；另加一个未授权手工 issue。
        async fn new() -> Self {
            let inner = seed_fixture(1, true);
            let root_path = inner._root.path().to_path_buf();
            let state = WebAppState::new(
                root_path.clone(),
                crate::web::runtime::WebRuntime::new_fake(root_path),
            );
            let app = crate::web::app::build_web_router(state.clone());
            let enable = put_enrollment(&app, enrollment_body(&inner, 1, 1)).await;
            assert_eq!(enable.status(), StatusCode::OK);
            assert!(response_json(enable).await["enabled"].as_bool().unwrap());

            // 未授权手工对照 issue（同 project）。
            let manual = crate::product::issue_store::IssueStore::new(inner.paths.clone())
                .create(crate::product::issue_store::CreateProductIssueInput {
                    project_id: PROJECT_ID.to_string(),
                    repo_id: Some("repo-1".to_string()),
                    logical_codebase_id: None,
                    title: "manual issue".to_string(),
                    description: None,
                    change_id: None,
                    base_branch: None,
                })
                .unwrap();
            Self {
                inner,
                state,
                manual_issue_id: manual.id,
            }
        }

        fn lifecycle(&self) -> crate::product::lifecycle_store::LifecycleStore {
            crate::product::lifecycle_store::LifecycleStore::new(self.inner.paths.clone())
        }

        fn bound_plans(&self) -> Vec<crate::product::models::IssueWorkItemPlan> {
            self.lifecycle()
                .list_issue_work_item_plans(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .into_iter()
                .filter(|plan| plan.id.starts_with("issue_work_item_plan_auto_"))
                .collect()
        }

        fn manual_issue_plans(&self) -> Vec<crate::product::models::IssueWorkItemPlan> {
            self.lifecycle()
                .list_issue_work_item_plans(PROJECT_ID, &self.manual_issue_id)
                .unwrap()
        }

        fn sessions(&self) -> Vec<crate::product::models::WorkspaceSessionRecord> {
            self.lifecycle()
                .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
                .unwrap()
        }

        fn automation_store(&self) -> IssueAutomationStore {
            IssueAutomationStore::new(self.inner.paths.clone())
        }

        fn worker(&self) -> AutopilotOrchestrator {
            AutopilotOrchestrator::new(
                self.state.clone(),
                OrchestratorConfig {
                    max_issues_per_tick: 32,
                    ..Default::default()
                },
            )
        }
    }

    #[tokio::test]
    async fn automation_reconcile_scans_only_enrolled_issues_and_reuses_binding() {
        let fixture = OrchestratorFixture::new().await;
        let worker = fixture.worker();
        // 关闭 browser/WS、不送任何事件：两轮补偿只依赖 durable 事实。
        worker.reconcile_all_once().await.unwrap();
        worker.reconcile_all_once().await.unwrap();

        let bound = fixture
            .automation_store()
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap();
        assert!(bound.plan_id.is_some());
        assert!(bound.session_id.is_some());
        assert_eq!(fixture.bound_plans().len(), 1);
        assert!(fixture.manual_issue_plans().is_empty());
        // 绑定 session 恰一且 Interactive。
        let sessions = fixture.sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].run_policy,
            crate::product::work_item_plan_policy::RunPolicy::Interactive
        );
    }

    /// Disable 先于动作认领：零启动、零 plan。
    #[tokio::test]
    async fn automation_reconcile_ignores_disabled_enrollment() {
        let fixture = OrchestratorFixture::new().await;
        let worker = fixture.worker();
        let app = crate::web::app::build_web_router(fixture.state.clone());
        let revision = fixture
            .automation_store()
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap()
            .policy_revision;
        let disable = serde_json::json!({
            "expected_revision": revision,
            "command": {"type": "disable"}
        });
        let response = put_enrollment(&app, disable).await;
        assert_eq!(response.status(), StatusCode::OK);

        let outcome = worker
            .reconcile(&fixture.state, PROJECT_ID, ISSUE_ID)
            .await
            .unwrap();
        assert_eq!(outcome, ReconcileOutcome::NoEnrollment);
        assert!(fixture.bound_plans().is_empty());
    }

    /// 换源重开（保留旧绑定）：fail-closed NeedsHuman，不自动生成、不另建链。
    #[tokio::test]
    async fn automation_reconcile_rejects_divergent_reopen_as_needs_human() {
        let fixture = OrchestratorFixture::new().await;
        let worker = fixture.worker();
        worker
            .reconcile(&fixture.state, PROJECT_ID, ISSUE_ID)
            .await
            .unwrap();
        assert_eq!(fixture.bound_plans().len(), 1);

        let app = crate::web::app::build_web_router(fixture.state.clone());
        let revision = fixture
            .automation_store()
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap()
            .policy_revision;
        let disable = serde_json::json!({
            "expected_revision": revision,
            "command": {"type": "disable"}
        });
        assert_eq!(
            put_enrollment(&app, disable).await.status(),
            StatusCode::OK
        );
        let mut divergent = enrollment_body(&fixture.inner, 1, 1);
        divergent["command"]["options"]["review_rounds"] = serde_json::json!(2);
        divergent["expected_revision"] = serde_json::json!(revision + 1);
        assert_eq!(
            put_enrollment(&app, divergent).await.status(),
            StatusCode::OK
        );

        let outcome = worker
            .reconcile(&fixture.state, PROJECT_ID, ISSUE_ID)
            .await
            .unwrap();
        assert_eq!(outcome, ReconcileOutcome::NeedsHuman);
        assert_eq!(fixture.bound_plans().len(), 1);
    }
}

// ---------------------------------------------------------------------------
// P1 WIGA Task 7：无 driver choice/人工门/compile recovery 的停等人对照。
// fixture 全部走真实面：enrollment PUT（P0 REST）→ `ensure_enrolled_plan`
//（Task 4 锁内唯一创建/绑定）→ manager 唯一 run 注册 + provider 等待者
// 消费 choice 应答回执；人工侧经 P0 REST 作答/批准，approve 由现有
// finalizer failpoint 在 compile 终结点失败——编排器必须全程只观察。
// ---------------------------------------------------------------------------

mod task7_gates {
    use super::*;
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
    use crate::product::workspace_engine::WorkspaceSession;
    use crate::product::workspace_engine::tests::single_candidate_recovery::single_candidate_recovery_record;
    use crate::web::handlers::automation_enrollment_test_support::{
        ISSUE_ID, PROJECT_ID, enrollment_body, put_enrollment, response_json, seed_fixture,
    };
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

    struct EnrolledGateFixture {
        inner: crate::web::handlers::automation_enrollment_test_support::Fixture,
        state: WebAppState,
        lifecycle: LifecycleStore,
        plan_id: String,
        session_id: String,
        manager: Arc<WorkspaceSessionManager>,
        token: u64,
        incarnation: String,
        gate_id: Option<String>,
    }

    impl EnrolledGateFixture {
        async fn new() -> Self {
            let inner = seed_fixture(1, true);
            let root_path = inner._root.path().to_path_buf();
            let state = WebAppState::new(
                root_path.clone(),
                crate::web::runtime::WebRuntime::new_fake(root_path),
            );
            // 子 WorkItem 上下文经 issue.repo_id 解析 repo-1：补进 repos.json
            //（seed_logical_codebase 重写后仅剩 physical 成员条目）。
            {
                let repos_path = inner.paths.project_root(PROJECT_ID).join("repos.json");
                let mut repositories: Vec<crate::product::models::RepositoryRecord> =
                    crate::product::json_store::read_json(&repos_path).unwrap();
                let repo_root = inner._root.path().join("repo-1");
                std::fs::create_dir_all(&repo_root).unwrap();
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
                        "author_provider": "fake",
                        "reviewer_provider": "fake",
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
                &state, &enrollment,
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

        /// 真实登记两张待答 choice 卡（run 挂起帧面，与 P0 part_02 同构）。
        async fn open_choice_with_two_questions(&self) {
            self.manager
                .register_pending_choice_frame(gate_choice_frame("choice-gate-1"));
            self.manager
                .register_pending_choice_frame(gate_choice_frame("choice-gate-2"));
        }

        fn provider_start_ledger(&self) -> usize {
            self.durable().provider_start_ledger.len()
        }

        fn durable(&self) -> WorkspaceSessionRecord {
            self.lifecycle
                .get_workspace_session(&self.session_id)
                .expect("durable bound session")
        }

        async fn reconcile(&self) -> Result<ReconcileOutcome, String> {
            let worker = AutopilotOrchestrator::new(self.state.clone(), Default::default());
            worker.reconcile(&self.state, PROJECT_ID, ISSUE_ID).await
        }

        /// 逐张经 P0 REST 作答并等待 Delivered；应答送达后 run 收尾。
        async fn human_answer_via_rest(&self) {
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
                assert_eq!(status, StatusCode::OK, "choice REST must deliver: {payload}");
                assert_eq!(payload["state"], "delivered", "{payload}");
            }
            self.manager.finish_run(self.token).await;
            assert!(
                self.manager.pending_choice_frames().is_empty(),
                "delivered choices must leave no pending frames"
            );
        }

        /// 把绑定 plan/session 铺成可编译的审批门形态（accepted contract
        /// drafts + active index + outline 候选 + source/IR/report 三 refs，
        /// 全部经真实 store/engine 面），注册现有 finalizer failpoint 后经
        /// P0 REST Approve——compile 必须在终结点失败且门保持开启。
        async fn fail_compile_after_human_approve(&mut self) {
            // 阶段一（durable 铺底）：accepted contract drafts + active index +
            // source/IR/report 三 refs + 审批门快照，全部走真实 store 面。
            let engine_arc = self.manager.engine();
            let mut engine = engine_arc.lock().await;
            engine.session.artifact = Some(gate_outline_candidate_payload());
            let plan_store = WorkItemPlanStore::new(self.inner.paths.clone());
            let draft_a = gate_draft_record(&self.plan_id, "outline_a", "draft_outline_a");
            let mut draft_b = gate_draft_record(&self.plan_id, "outline_b", "draft_outline_b");
            let mut required =
                crate::product::work_item_contract::canonical_contract_fixture("unused")
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
            single_candidate_recovery_record(
                &self.lifecycle,
                &mut engine,
                SingleCandidatePhase::Approval,
                RunPolicy::Interactive,
            );
            let mut record = self.durable();
            record.status = WorkspaceSessionStatus::WaitingForHuman;
            record.human_gate_snapshot = Some(HumanGateSnapshot {
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
            drop(engine);

            // 阶段二（当前 manager）：choice 应答后 run 收尾会触发 idle 回收，
            // HTTP 侧按 registry 解析当前 manager；outline 候选与 failpoint
            // 必须登记在「真正执行 approve 的那个引擎」上。
            let state = self.state.clone();
            let session_id = self.session_id.clone();
            let manager = state
                .workspace_sessions
                .get_or_create(&session_id, || {
                    WorkspaceSessionManager::create(&state, &session_id)
                })
                .await
                .expect("current registry manager");
            let engine_arc = manager.engine();
            let mut engine = engine_arc.lock().await;
            engine.session.artifact = Some(gate_outline_candidate_payload());
            engine.session.stage = crate::product::workspace_engine::WorkspaceStage::HumanConfirm;
            engine.session.session_status = WorkspaceSessionStatus::WaitingForHuman;
            // 真实开门（时间线 HumanConfirm 节点 + durable WaitingForHuman）。
            engine
                .enter_human_confirm(Some("人工批准（compile 将在 finalizer 失败）".to_string()))
                .await;
            let gate_id = engine.active_timeline_node_id().expect("gate node");
            let _failpoint = engine.register_single_candidate_compile_failpoint(
                SingleCandidateCompileCheckpoint::ProvenancePersisted,
            );
            drop(engine);
            self.gate_id = Some(gate_id.clone());

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
            assert_eq!(
                durable.status,
                WorkspaceSessionStatus::WaitingForHuman,
                "failed compile must leave the gate open for humans"
            );
        }

        /// 人工 Abandon（P0 REST）：终态后编排器零复活、零额外 provider。
        async fn abandon_via_rest(&self) {
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

        fn session_path(&self, session_id: &str) -> std::path::PathBuf {
            self.inner
                .paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("workspace-sessions")
                .join(format!("{session_id}.json"))
        }
    }

    /// provider 等待者：真实消费 run command 通道，choice 应答即投递回执。
    async fn deliver_choice_receipts(mut rx: tokio::sync::mpsc::Receiver<ProviderCommand>) {
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

    fn gate_choice_frame(id: &str) -> WsOutMessage {
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

    fn gate_outline_candidate_payload() -> ArtifactPayload {
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

    fn gate_draft_record(plan_id: &str, outline_id: &str, draft_id: &str) -> WorkItemDraftRecord {
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
        contract.verification_checks[0].command = Some(format!("cargo test --locked --lib {outline_id}"));
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

    fn gate_active_index(plan_id: &str) -> WorkItemPlanDraftActiveIndex {
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
                (
                    "draft_outline_a".to_string(),
                    WorkItemDraftStatus::Accepted,
                ),
                (
                    "draft_outline_b".to_string(),
                    WorkItemDraftStatus::Accepted,
                ),
            ]),
            batches: vec![],
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    async fn post_json(
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

    async fn post_choice(
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

    async fn post_human_action(
        app: &axum::Router,
        session_id: &str,
        body: &serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        post_json(app, format!("/api/workspace-sessions/{session_id}/human-actions"), body).await
    }

    #[tokio::test]
    async fn automation_reconcile_waits_for_choice_and_failed_compile() {
        let mut fixture = Box::new(EnrolledGateFixture::new().await);
        fixture.open_choice_with_two_questions().await;
        let before = fixture.provider_start_ledger();
        assert!(matches!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::AwaitingHuman
        ));
        assert_eq!(fixture.provider_start_ledger(), before);
        fixture.human_answer_via_rest().await;
        fixture.fail_compile_after_human_approve().await;
        assert!(matches!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::AwaitingHuman
        ));
        assert_eq!(fixture.provider_start_ledger(), before);
        // Abandon 后终态：编排器不复活、不追加 provider run。
        fixture.abandon_via_rest().await;
        assert_eq!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::NeedsHuman
        );
        assert_eq!(fixture.provider_start_ledger(), before);
    }
}
