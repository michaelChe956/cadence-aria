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
    /// fail-closed 停点（授权漂移/损坏/不可证明的分诊），只等人。
    NeedsHuman,
    /// P2 Task 5：Confirmed plan 本轮已请求 stable-id advance（到 Ready 即
    /// 止，不在同轮首启 coding）。
    Advancing,
    /// P2 Task 5：Ready 唯一 attempt 已请求 stable-id AutoStartOnce（含已在
    /// 途/已认领的重复唤醒，不重启）。
    Coding,
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
        // P2 Task 5：Confirmed 编排链——成功确认的 plan 先 stable-id advance
        //（到 Ready 即止），下一轮才对 Ready 唯一 attempt 发 stable-id
        // AutoStartOnce；其余状态仍交生成准入（原语义）。
        if let Some(outcome) = coding_chain_stage(state, &fresh).await? {
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

/// P2 Task 6（§3.2）：无 attach 启动扫描——`serve_web` 构造 state 后、进入
/// 有界 tick 前先行一次。沿 project → issue → attempt 全量遍历，对带 Task 4
/// durable claim 的首启现场与 legacy Running 半启动恢复 runner（事件走
/// attempt 级 hub，零 socket 订阅者也不阻塞）；副作用不可证明的窗口
/// fail-closed 转 AwaitingManualRecovery。返回本轮恢复的 runner 数；单
/// attempt 失败留下可见诊断并继续扫描（不吞错误）。
pub async fn reconcile_claimed_coding_runs_once(state: &WebAppState) -> Result<usize, String> {
    let paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let store = crate::product::coding_attempt_store::CodingAttemptStore::new(paths.clone());
    let projects = ProjectStore::new(paths.clone())
        .list()
        .map_err(|error| format!("startup coding scan: project list unreadable: {error}"))?;
    let mut resumed = 0usize;
    for project in &projects {
        let issues = IssueStore::new(paths.clone())
            .list(&project.id)
            .map_err(|error| {
                format!(
                    "startup coding scan: issue list unreadable for {}: {error}",
                    project.id
                )
            })?;
        for issue in &issues {
            let attempts = match store.list_attempts_for_issue(&project.id, &issue.id) {
                Ok(attempts) => attempts,
                Err(error) => {
                    eprintln!(
                        "startup coding scan: attempts unreadable for {}/{}: {error}",
                        project.id, issue.id
                    );
                    continue;
                }
            };
            for attempt in attempts {
                match recover_claimed_attempt_at_startup(state, &store, &attempt).await {
                    Ok(true) => resumed += 1,
                    Ok(false) => {}
                    Err(error) => {
                        eprintln!(
                            "startup coding scan: reconcile failed for {}: {error}",
                            attempt.id
                        );
                    }
                }
            }
        }
    }
    Ok(resumed)
}

/// 单 attempt 启动恢复分诊（P2 Task 6）：
/// - registry 已有 runner/预约：幂等跳过（并发抢输不是失败）。
/// - Running：可信半启动，走既有 `ensure_runner_for_resumed_attempt`
///（SC durable-ready 门、物化判定、双启去重全在 helper 内）。
/// - Created + claim（Claimed/RunnerRegistered）：同 command/origin 幂等
///   续启——Task 4 单一恢复方法，不隐式新 command。
/// - Created + claim（ProviderMayHaveStarted）且无可信 ledger：外部副作用
///   不可证明 → durable AwaitingManualRecovery（UI 可诊断），零 runner。
/// - Created 无 claim / 终态 / 人工停点态：零动作。
async fn recover_claimed_attempt_at_startup(
    state: &WebAppState,
    store: &crate::product::coding_attempt_store::CodingAttemptStore,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
) -> Result<bool, String> {
    use crate::product::coding_models::{CodingAttemptStatus, CodingStartPhase};
    use crate::web::coding_ws_handler::{
        ResumedAttemptRunner, ensure_runner_for_resumed_attempt,
    };

    let attempt_key = crate::web::state::CodingAttemptRunKey::from_attempt(attempt);
    if state
        .coding_runs
        .attempt_is_reserved_or_running(&attempt_key)
    {
        return Ok(false);
    }
    match attempt.status {
        CodingAttemptStatus::Running => {
            // 零订阅者 hub：恢复事件不依赖任何存活 socket。
            let event_tx = state.coding_sockets.hub_sender(&attempt_key);
            match ensure_runner_for_resumed_attempt(state, store, &event_tx, &attempt_key, attempt)
                .await
            {
                ResumedAttemptRunner::Restarted { .. } => Ok(true),
                ResumedAttemptRunner::NotNeeded
                | ResumedAttemptRunner::ManualRecovery { .. } => Ok(false),
            }
        }
        CodingAttemptStatus::Created => {
            let Some(claim) = attempt.start_claim.as_ref() else {
                return Ok(false);
            };
            match claim.phase {
                CodingStartPhase::Claimed | CodingStartPhase::RunnerRegistered => {
                    let outcome = crate::web::coding_start::start_coding_once(
                        state,
                        &attempt.project_id,
                        &attempt.issue_id,
                        crate::web::coding_start::StartCodingCommand {
                            attempt_id: attempt.id.clone(),
                            command_id: claim.command_id.clone(),
                            origin: claim.origin.clone(),
                        },
                    )
                    .await;
                    match outcome {
                        Ok(crate::web::coding_start::StartCodingOutcome::Started { .. }) => {
                            Ok(true)
                        }
                        Ok(crate::web::coding_start::StartCodingOutcome::AlreadyStarted {
                            ..
                        }) => Ok(false),
                        Ok(crate::web::coding_start::StartCodingOutcome::NeedsHuman {
                            attempt_id,
                            reason,
                        }) => Err(format!(
                            "claimed first start replay triaged human for {attempt_id}: {reason}"
                        )),
                        Err(error) => Err(format!(
                            "claimed first start replay failed for {}: {}",
                            attempt.id, error
                        )),
                    }
                }
                CodingStartPhase::ProviderMayHaveStarted => {
                    // 可信在途事实优先：role run ledger 已有 provider 启动
                    // 证据时交由在途 run/恢复协议，不二次首启也不分诊。
                    let ledger_started = store
                        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
                        .map(|runs| !runs.is_empty())
                        .unwrap_or(false);
                    if ledger_started {
                        return Ok(false);
                    }
                    store
                        .transition_to_awaiting_manual_recovery(
                            &attempt.id,
                            "coding_startup_claim_side_effects_unproven",
                        )
                        .map_err(|error| {
                            format!("mark manual recovery failed for {}: {error:?}", attempt.id)
                        })?;
                    Ok(false)
                }
                CodingStartPhase::NeedsHuman => Ok(false),
            }
        }
        _ => Ok(false),
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

/// P2 Task 5：Confirmed 编排链分诊。只在「Confirmed + `plan_confirmed_info`
/// 可派生成功 publication/compile + 当前 enrollment 精确绑定」时接管：
/// 未 Ready 先 stable-id `advance_plan`（本轮到 Ready 即止）；Ready 后读取
/// group journal 唯一 attempt，按状态分诊或 stable-id AutoStartOnce
///（`wiga-start-{attempt_id}`，同 claim 意图固定）。Failed/Aborted/漂移
/// 一律 NeedsHuman；任何 Replayed 的 Failed/Aborted 记录也人工分诊，
/// 不隐式新 command。其余 session 状态返回 None 交生成准入。
async fn coding_chain_stage(
    state: &WebAppState,
    enrollment: &crate::product::models::automation::IssueAutomationEnrollment,
) -> Result<Option<ReconcileOutcome>, String> {
    use crate::product::advance_store::{AdvanceOutcome, AdvanceStatus, AdvanceStore};
    use crate::product::coding_models::{CodingAttemptStatus, CodingStartOrigin};
    use crate::product::models::WorkspaceSessionStatus;
    use crate::web::advance_plan::{AdvancePlanOrigin, advance_plan};
    use crate::web::coding_start::{StartCodingCommand, StartCodingOutcome, start_coding_once};

    let Some(session_id) = enrollment.session_id.as_deref() else {
        return Ok(None);
    };
    let paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(paths.clone());
    let session = lifecycle
        .get_workspace_session(session_id)
        .map_err(|error| format!("bound session unreadable: {error}"))?;
    if session.status != WorkspaceSessionStatus::Confirmed {
        return Ok(None);
    }
    // 成功 publication/compile 必须可由 P1 只读投影派生（含 enrollment
    // enabled/精确绑定与 compile reservation/事务 Committed）；不可派生
    // 即 fail-closed 人工分诊。
    let confirmed =
        crate::web::plan_confirmed_info::plan_confirmed_info(&paths, enrollment)
            .map_err(|error| format!("plan confirmed info unreadable: {error}"))?;
    if confirmed.is_none() {
        return Ok(Some(ReconcileOutcome::NeedsHuman));
    }
    let Some(plan_id) = enrollment.plan_id.clone() else {
        return Ok(Some(ReconcileOutcome::NeedsHuman));
    };
    let project_id = enrollment.project_id.clone();
    let issue_id = enrollment.issue_id.clone();

    let advance_ready = AdvanceStore::new(paths.clone())
        .get_advance_for_plan(&project_id, &issue_id, &plan_id)
        .map_err(|error| format!("advance record unreadable: {error}"))?
        .is_some_and(|record| record.status == AdvanceStatus::Ready);
    if !advance_ready {
        let outcome = advance_plan(
            state,
            crate::product::advance_store::AdvanceInput {
                command_id: format!("wiga-advance-{}-{plan_id}", enrollment.enrollment_id),
                project_id: project_id.clone(),
                issue_id: issue_id.clone(),
                plan_id: plan_id.clone(),
            },
            AdvancePlanOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
            },
        )
        .await;
        return Ok(Some(match outcome {
            Ok(AdvanceOutcome::Completed { record, .. })
                if record.status == AdvanceStatus::Ready =>
            {
                ReconcileOutcome::Advancing
            }
            Ok(AdvanceOutcome::Replayed { record })
                if record.status == AdvanceStatus::Ready =>
            {
                ReconcileOutcome::Advancing
            }
            _ => ReconcileOutcome::NeedsHuman,
        }));
    }

    // Ready：group journal 只用于定位唯一 attempt 身份；状态必须从 attempt
    // store 读新鲜值——journal 内嵌快照冻结于初始化时点（恒 Created/
    // PrepareContext），据其分诊会在 attempt 已停等人时重入首启路径，
    // 把 durable claim 误降级 NeedsHuman。
    let journal = crate::product::coding_attempt_store::CodingAttemptStore::new(paths.clone())
        .get_group_initialization(&project_id, &issue_id, &plan_id)
        .map_err(|error| format!("group initialization journal unreadable: {error}"))?;
    let attempt = journal.attempt;
    let attempt = crate::product::coding_attempt_store::CodingAttemptStore::new(paths.clone())
        .get_attempt(&project_id, &issue_id, &attempt.id)
        .map_err(|error| format!("group attempt unreadable: {error}"))?;
    match attempt.status {
        CodingAttemptStatus::Running
        | CodingAttemptStatus::AwaitingPlanAmendment
        | CodingAttemptStatus::ApplyingPlanAmendment
        | CodingAttemptStatus::AmendmentApplyFailed
        | CodingAttemptStatus::Completed => Ok(Some(ReconcileOutcome::Coding)),
        CodingAttemptStatus::WaitingForHuman | CodingAttemptStatus::Blocked => {
            Ok(Some(ReconcileOutcome::AwaitingHuman))
        }
        CodingAttemptStatus::Failed
        | CodingAttemptStatus::Aborted
        | CodingAttemptStatus::AwaitingManualRecovery => Ok(Some(ReconcileOutcome::NeedsHuman)),
        CodingAttemptStatus::Created => {
            let start = start_coding_once(
                state,
                &project_id,
                &issue_id,
                StartCodingCommand {
                    attempt_id: attempt.id.clone(),
                    command_id: format!("wiga-start-{}", attempt.id),
                    origin: CodingStartOrigin::Enrolled {
                        enrollment_id: enrollment.enrollment_id.clone(),
                        policy_revision: enrollment.policy_revision,
                    },
                },
            )
            .await;
            Ok(Some(match start {
                Ok(StartCodingOutcome::Started { .. })
                | Ok(StartCodingOutcome::AlreadyStarted { .. }) => ReconcileOutcome::Coding,
                Ok(StartCodingOutcome::NeedsHuman { .. }) | Err(_) => {
                    ReconcileOutcome::NeedsHuman
                }
            }))
        }
    }
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

        /// 绑定 plan 名下的 session（无绑定时为空——崩溃现场观察用）。
        fn sessions_for_bound_plan(&self) -> Vec<crate::product::models::WorkspaceSessionRecord> {
            let binding = self
                .automation_store()
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .unwrap();
            match binding.plan_id {
                Some(plan_id) => self
                    .sessions()
                    .into_iter()
                    .filter(|session| session.entity_id == plan_id)
                    .collect(),
                None => Vec::new(),
            }
        }

        /// 绑定 plan session 的 durable provider 启动账目条数。
        fn provider_start_count(&self) -> usize {
            let binding = self
                .automation_store()
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .unwrap();
            match binding.session_id {
                Some(session_id) => self
                    .lifecycle()
                    .get_workspace_session(&session_id)
                    .unwrap()
                    .provider_start_ledger
                    .len(),
                None => 0,
            }
        }

        /// 绑定自动化 session 的 durable 记录（重启后 RunPolicy 断言用）。
        fn automation_session(&self) -> crate::product::models::WorkspaceSessionRecord {
            let session_id = self
                .automation_store()
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .unwrap()
                .session_id
                .expect("bound automation session");
            self.lifecycle().get_workspace_session(&session_id).unwrap()
        }

        /// 「重启」：全新 WebAppState/runtime（进程替身）——内存态清零，
        /// 只保留磁盘 durable 事实。
        fn restart_state(&self) -> WebAppState {
            let root = self.inner._root.path().to_path_buf();
            WebAppState::new(
                root.clone(),
                crate::web::runtime::WebRuntime::new_fake(root),
            )
        }

        /// 重启后两轮有界扫描补偿 + 一轮定向分诊，返回绑定 issue 的最终
        /// reconcile 结果。
        async fn restart_and_reconcile(&self) -> ReconcileOutcome {
            let state = self.restart_state();
            let worker = AutopilotOrchestrator::new(
                state.clone(),
                OrchestratorConfig {
                    max_issues_per_tick: 32,
                    ..Default::default()
                },
            );
            worker.reconcile_all_once().await.expect("restart scan 1");
            worker.reconcile_all_once().await.expect("restart scan 2");
            worker
                .reconcile(&state, PROJECT_ID, ISSUE_ID)
                .await
                .expect("restart targeted reconcile")
        }

        /// 等待重启后派发的 provider run 把启动账目落盘（reserve 先于任何
        /// provider 交互，有界轮询即可）。
        async fn await_provider_start(&self) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while self.provider_start_count() == 0 {
                assert!(
                    std::time::Instant::now() < deadline,
                    "provider start ledger never persisted after restart"
                );
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
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

    /// P1 WIGA Task 10（2.4）：四中窗一次性中断——进程在 intent/plan/
    /// session 落盘后、检查点 EngineStarted 落盘后崩溃，重启（全新
    /// state/runtime）只凭 durable 事实补偿后：恰一绑定 plan/session、
    /// provider 绝不重发（可证明窗口恰一次派发；EngineStarted 后不可证明
    /// 则 fail-closed 人工分诊零派发）、RunPolicy 恒 Interactive
    /// （REQ-WIGA-03、REQ-WIGA-02）。
    #[tokio::test]
    async fn automation_p1_crash_windows_keep_one_plan_and_never_reissue_provider() {
        use crate::product::issue_automation_store::automation_crash_window::{self, CrashWindow};

        for window in [
            CrashWindow::AfterIntentSaved,
            CrashWindow::AfterPlanSaved,
            CrashWindow::AfterSessionSaved,
            CrashWindow::AfterEngineStarted,
        ] {
            let fixture = OrchestratorFixture::new().await;
            // 「进程」在窗口处崩溃：链路以错误中止，durable 停在中窗。
            let interrupt = automation_crash_window::register(window);
            let message = fixture
                .worker()
                .reconcile(&fixture.state, PROJECT_ID, ISSUE_ID)
                .await
                .expect_err("interrupt must abort the in-flight pass");
            assert!(
                message.contains("ensure enrolled plan failed")
                    || message.contains("automation_crash_window"),
                "{window:?}: crash must abort mid-chain, got: {message}"
            );
            drop(interrupt); // 崩溃即进程消失（含未触发的注册）。

            // 崩溃现场逐窗核对（durable 事实，不带内存态）。
            let binding_after_crash = fixture
                .automation_store()
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .unwrap();
            match window {
                CrashWindow::AfterIntentSaved => {
                    assert!(binding_after_crash.plan_id.is_none());
                    assert!(fixture.bound_plans().is_empty());
                    assert!(fixture.sessions().is_empty());
                }
                CrashWindow::AfterPlanSaved => {
                    assert!(binding_after_crash.plan_id.is_none(), "绑定未写回");
                    assert_eq!(fixture.bound_plans().len(), 1, "plan 已落盘");
                    assert!(
                        fixture.sessions_for_bound_plan().is_empty(),
                        "session 尚未创建"
                    );
                }
                CrashWindow::AfterSessionSaved => {
                    assert!(binding_after_crash.plan_id.is_none(), "绑定未写回");
                    let plans = fixture.bound_plans();
                    assert_eq!(plans.len(), 1);
                    let orphan_plan_id = plans[0].id.clone();
                    assert_eq!(
                        fixture
                            .sessions()
                            .into_iter()
                            .filter(|session| session.entity_id == orphan_plan_id)
                            .count(),
                        1,
                        "session 已落盘但绑定未写回"
                    );
                }
                CrashWindow::AfterEngineStarted => {
                    assert!(binding_after_crash.plan_id.is_some());
                    assert_eq!(fixture.bound_plans().len(), 1);
                    assert_eq!(fixture.sessions_for_bound_plan().len(), 1);
                    assert_eq!(
                        fixture.provider_start_count(),
                        0,
                        "崩溃先于 provider 派发"
                    );
                }
            }

            // 重启：全新 state/runtime 只凭 durable 事实补偿。
            let outcome = fixture.restart_and_reconcile().await;
            assert_eq!(fixture.bound_plans().len(), 1, "{window:?}: 恰一 plan");
            assert_eq!(
                fixture.sessions_for_bound_plan().len(),
                1,
                "{window:?}: 恰一 session"
            );
            assert_eq!(
                fixture.automation_session().run_policy,
                crate::product::work_item_plan_policy::RunPolicy::Interactive,
                "{window:?}: 绑定 session 恒 Interactive"
            );
            match window {
                // 可证明 provider 未被触达的窗口：重启后正常恰一次派发。
                CrashWindow::AfterIntentSaved
                | CrashWindow::AfterPlanSaved
                | CrashWindow::AfterSessionSaved => {
                    fixture.await_provider_start().await;
                    assert_eq!(
                        fixture.provider_start_count(),
                        1,
                        "{window:?}: 恰一次 provider 启动"
                    );
                }
                // 越过 EngineStarted 后崩溃：无法证明 provider 未被触达，
                // fail-closed 人工分诊，绝不重发。
                CrashWindow::AfterEngineStarted => {
                    assert_eq!(outcome, ReconcileOutcome::NeedsHuman);
                    assert_eq!(
                        fixture.provider_start_count(),
                        0,
                        "{window:?}: 不可证明时不重发 provider"
                    );
                    // 再次「重启」多轮补偿：仍不重发。
                    let again = fixture.restart_and_reconcile().await;
                    assert_eq!(again, ReconcileOutcome::NeedsHuman);
                    assert_eq!(fixture.provider_start_count(), 0);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// P1 WIGA Task 7：无 driver choice/人工门/compile recovery 的停等人对照。
// fixture 全部走真实面：enrollment PUT（P0 REST）→ `ensure_enrolled_plan`
//（Task 4 锁内唯一创建/绑定）→ manager 唯一 run 注册 + provider 等待者
// 消费 choice 应答回执；人工侧经 P0 REST 作答/批准，approve 由现有
// finalizer failpoint 在 compile 终结点失败——编排器必须全程只观察。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod task7_gates {
    use super::*;
    use crate::web::wiga_gate_fixture::*;

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

// ---------------------------------------------------------------------------
// P2 Task 5（tasks.md §3.1）：Confirmed plan 无 socket 独立 advance 到
// Ready，下一轮才 stable-id AutoStartOnce；重复唤醒不重启同一 attempt。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod p2_coding_chain {
    use super::*;
    use crate::web::state::CodingAttemptRunKey;
    use crate::web::wiga_gate_fixture::*;

    #[tokio::test]
    async fn confirmed_enrollment_advances_then_starts_without_coding_socket() {
        let fixture = confirmed_enrolled_fixture().await;
        let worker = AutopilotOrchestrator::new(fixture.state.clone(), Default::default());
        assert_eq!(
            worker.reconcile(&fixture.state, PROJECT_ID, ISSUE_ID).await.unwrap(),
            ReconcileOutcome::Advancing
        );
        let ready = fixture.coding_attempts().into_iter().next().unwrap();
        assert_eq!(
            fixture.state.coding_runs.runner_count(
                &CodingAttemptRunKey::from_attempt(&ready)
            ),
            0,
            "advance must not sneak provider start into the same transition"
        );
        assert_eq!(
            worker.reconcile(&fixture.state, PROJECT_ID, ISSUE_ID).await.unwrap(),
            ReconcileOutcome::Coding
        );
        assert_eq!(
            fixture
                .state
                .coding_runs
                .runner_count(&CodingAttemptRunKey::from_attempt(&ready)),
            1
        );
        // 重复唤醒：同 attempt 不重启（durable claim + registry 单飞）。
        assert_eq!(
            worker.reconcile(&fixture.state, PROJECT_ID, ISSUE_ID).await.unwrap(),
            ReconcileOutcome::Coding
        );

        assert_eq!(
            fixture.coding_attempts().into_iter().next().unwrap().id,
            ready.id
        );
        assert!(fixture
            .coding_attempts()
            .into_iter()
            .next()
            .unwrap()
            .start_claim
            .is_some());
    }


    /// D2：Confirmed 后 disable 先胜——未消费的 AutoStartOnce/advance 许可
    /// 不再生效，零 provider。
    #[tokio::test]
    async fn disabled_after_confirm_yields_no_enrollment_before_any_start() {
        let fixture = confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        crate::product::issue_automation_store::IssueAutomationStore::new(
            fixture.inner.paths.clone(),
        )
        .compare_and_set(
            PROJECT_ID,
            ISSUE_ID,
            Some(enrollment.policy_revision),
            crate::product::models::automation::EnrollmentWriteCommand::Disable,
        )
        .expect("disable enrollment");
        let worker = AutopilotOrchestrator::new(fixture.state.clone(), Default::default());
        assert_eq!(
            worker.reconcile(&fixture.state, PROJECT_ID, ISSUE_ID).await.unwrap(),
            ReconcileOutcome::NoEnrollment
        );
        assert!(fixture.coding_attempts().is_empty());
        assert_eq!(fixture.coding_runner_count(), 0);
    }
}

// ---------------------------------------------------------------------------
// P2 Task 10（tasks.md §3.4）：跨层 campaign——人工批准 Confirmed plan 后，
// 无页面 reconcile 独立 advance→单发首启→Fake runner 后台跑到 durable
// `WaitingForHuman ∧ FinalConfirm`；编排器不代点，人手 handle_final_confirm
// 才 Completed。503 失败分诊单独成证（GAP-E/G/H：durable 诊断 + 零自动重驱）。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod p2_campaign {
    use super::*;
    use crate::product::coding_models::{CodingAttemptStatus, CodingExecutionStage};
    use crate::web::wiga_gate_fixture::*;

    #[tokio::test]
    async fn p2_campaign_requires_human_final_confirm_after_socketless_run() {
        let mut fixture = p2_enrolled_campaign_fixture().await;
        fixture.confirm_plan_by_human().await;
        fixture.reconcile_until_coding_waiting_for_human().await;
        let attempt = fixture.attempt();
        assert_eq!(attempt.stage, CodingExecutionStage::FinalConfirm);
        assert_eq!(attempt.status, CodingAttemptStatus::WaitingForHuman);
        assert_eq!(fixture.runner_start_claims(&attempt.id), 1);
        // 等待期间编排器只观察（人工 Final Confirm 不被代点、不隐式重驱）。
        let worker = AutopilotOrchestrator::new(fixture.gate.state.clone(), Default::default());
        assert_eq!(
            worker
                .reconcile(&fixture.gate.state, PROJECT_ID, ISSUE_ID)
                .await
                .unwrap(),
            ReconcileOutcome::AwaitingHuman
        );
        assert_eq!(fixture.attempt().status, CodingAttemptStatus::WaitingForHuman);
        fixture.gate.confirm_final_by_human().await;
        assert_eq!(fixture.attempt().status, CodingAttemptStatus::Completed);
        assert_eq!(fixture.runner_start_claims(&attempt.id), 1);
    }

    /// GAP-E/G/H campaign 侧：reviewer 网关 503 失败 durable 收敛 + 可见诊断，
    /// 编排器分诊 NeedsHuman 且重复唤醒零重驱（provider 账目不增）。
    #[tokio::test]
    async fn p2_campaign_gateway_503_reviewer_failure_is_human_triage_without_redrive() {
        let mut fixture = p2_enrolled_campaign_fixture().await;
        let failed_node_id = fixture.gate.fail_reviewer_with_gateway_503().await;
        // GAP-H：分类诊断落 durable 节点摘要，脱敏（不泄凭据）。
        let summary = fixture
            .gate
            .timeline_nodes()
            .iter()
            .find(|node| node.node_id == failed_node_id)
            .unwrap()
            .summary
            .clone()
            .unwrap_or_default();
        assert!(
            summary.contains("provider_gateway_503_no_accounts"),
            "503 diagnostic must be durable: {summary}"
        );
        assert!(!summary.contains("secret"), "diagnostic must redact credentials");
        let before = fixture.gate.provider_start_ledger();
        let worker = AutopilotOrchestrator::new(fixture.gate.state.clone(), Default::default());
        for _ in 0..3 {
            assert_eq!(
                worker
                    .reconcile(&fixture.gate.state, PROJECT_ID, ISSUE_ID)
                    .await
                    .unwrap(),
                ReconcileOutcome::NeedsHuman
            );
        }
        assert_eq!(
            fixture.gate.provider_start_ledger(),
            before,
            "repeated reconcile must not re-drive the failed reviewer"
        );
        assert_eq!(fixture.manual_issue_runner_start_claims(), 0);
    }
}

