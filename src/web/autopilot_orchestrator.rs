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

/// crash-window 注入（automation_crash_window）是进程级全局：驱动
/// ensure_enrolled_plan/reconcile 且涉及窗口武装的测试经此锁互斥
///（p1 四中窗测试 × g8 孤儿重驱测试，并行会互偷窗口）。守卫需跨
/// await 持有整个测试体，故用 tokio 异步锁（std 锁跨 await 会触发
/// await_holding_lock，且多线程 runtime 下有同线程重入死锁风险）。
#[cfg(test)]
static CRASH_WINDOW_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
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
        // 批 2 T3（REQ-WIGA-09）：terminated 是不可恢复终态——显式跳过
        // （enabled=false 已拦，此处再按 terminated_at 收口，防复活）。
        if enrollment.terminated_at.is_some() || !enrollment.enabled {
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
                    ));
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
        let wrapped =
            budget < self.config.max_issues_per_tick || start_index + budget >= ordered.len();
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
    // T21 前置:先补偿未关闭 reviewer 窗口(停旧登记资源且确认静止→
    // 先持久新发现违规→按原 manifest 续恢复→三域复核/人工门→释放),
    // 再放行 claimed-run/autopilot 分诊——不得沿旧序在同树先启 runner。
    // 单窗失败不停扫(局部坏窗其它无冲突树继续);未收口树由
    // recover_claimed_attempt_at_startup 的租约闸门 fail-closed。
    let (startup_event_tx, _startup_event_rx) = tokio::sync::mpsc::channel(64);
    let startup_engine = crate::product::coding_workspace_engine::CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace_service::GitWorkspaceService::new(),
        startup_event_tx,
    );
    for outcome in startup_engine
        .compensate_open_reviewer_windows_at_startup()
        .await
    {
        if !outcome.released {
            eprintln!(
                "[aria-startup] reviewer window {} (attempt {}) kept locked: state={} reason={}",
                outcome.window_id,
                outcome.attempt_id,
                outcome.state.as_str(),
                outcome.manual_attention.as_deref().unwrap_or("unresolved")
            );
        }
    }
    // F2(审查修复轮 1):分诊恢复的 runner 只有 5s 阶段门,若探活仍在
    // 途中,provider_for 会撞初始 degraded 闸门被误转 AwaitingManualRecovery
    // (F-14 面)。补偿之后、分诊之前先行刷新(与 serve_web 的 notice 语义
    // 分离:此处只刷新不重复发 probe notice;test_provider_enabled 时跳过
    // 真实探测,与 refresh_provider_health_for_startup 同守卫)。
    if !state.test_provider_enabled {
        state.refresh_provider_health().await;
    }

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
                // Bug2-B(Task 4)：每 attempt 分诊前的 gate-only 对账——收敛
                // 门的崩溃窗口，零 provider 调用；失败显式记录不阻断扫描。
                if let Err(error) = reconcile_blocked_gates_for_attempt_at_startup(&store, &attempt)
                {
                    eprintln!("startup gate reconcile failed for {}: {error}", attempt.id);
                }
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

/// Bug2-B(Task 4)：每 attempt 分诊前的 gate-only 启动对账（serve_web →
/// `reconcile_claimed_coding_runs_once` 内嵌）——只动门、零 provider
/// 调用。规则（修复方案 §三「启动对账」）：
/// a. 根已写完整关闭事实、归档未落（崩溃前缀）→ 统一关闭原语同动作重放
///    补齐归档（幂等，首关事实不重写）；open 根 × 上一代残留归档（重开
///    半完成）→ reopen 原语清理残留归档（代次不动）。
/// b. open 门与 run gate_origin 接受事实一致 → origin 精确匹配幂等补偿
///    关门（仅代次一致的终态 run——Completed/Failed/Blocked/Aborted 仅补
///    记「动作已接受」，不代表业务通过；run 终态已持久化为前提）。
/// c. 归档缺 action 且恢复 journal（活跃或归档）可证 retry_review →
///    按证据回填动作（resolved_at 历史不伪造）。
/// d. 无关联／代次失配／异动作／无证据 → 只告警，durable JSON 不动。
/// e. 恢复 journal 仲裁中（未 completed 且关联该门）→ 不抢先关。
/// 返回本轮收敛／补偿／回填的门数。
fn reconcile_blocked_gates_for_attempt_at_startup(
    store: &crate::product::coding_attempt_store::CodingAttemptStore,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
) -> Result<usize, String> {
    use crate::product::coding_models::CodingRoleRunStatus;

    let (project_id, issue_id, attempt_id) = (&attempt.project_id, &attempt.issue_id, &attempt.id);
    let mut actions = 0usize;

    let active_journal = store
        .get_failed_code_review_recovery_journal(project_id, issue_id, attempt_id)
        .map_err(|error| format!("recovery journal unreadable: {error}"))?;
    let states = store
        .blocked_gate_states_for_startup_reconcile(project_id, issue_id, attempt_id)
        .map_err(|error| format!("blocked gate roots unreadable: {error}"))?;
    let runs = store
        .list_role_runs(project_id, issue_id, attempt_id)
        .map_err(|error| format!("role runs unreadable: {error}"))?;

    for state in states {
        let gate_id = state.gate.gate_id.clone();
        if let Some(pending) = state.pending_resolution {
            // a. 崩溃前缀：以根记录首关事实同动作重放补齐归档。
            store
                .resolve_blocked_gate_with_action(
                    project_id,
                    issue_id,
                    attempt_id,
                    &gate_id,
                    pending.resolved_action.as_deref(),
                )
                .map_err(|error| format!("gate {gate_id} archive completion failed: {error}"))?;
            actions += 1;
            continue;
        }
        if store
            .get_archived_blocked_gate_facts(project_id, issue_id, attempt_id, &gate_id)
            .map_err(|error| format!("gate {gate_id} archive unreadable: {error}"))?
            .is_some()
        {
            // a. 重开半完成残留（open 根 + 上一代归档）：按 reopen 原语清理
            // 残留归档，代次不动——open 根是当前事实，继续走 b 判定。
            store
                .reopen_failed_code_review_gate_for_plan_repair(
                    project_id, issue_id, attempt_id, &gate_id,
                )
                .map_err(|error| {
                    format!("gate {gate_id} residual archive cleanup failed: {error}")
                })?;
            actions += 1;
        }
        // e. 仲裁中的门不抢先关。
        let arbitration_in_flight = active_journal
            .as_ref()
            .is_some_and(|journal| !journal.is_completed() && journal.expected_gate_id == gate_id);
        if arbitration_in_flight {
            eprintln!(
                "[aria-startup] gate {gate_id} on attempt {attempt_id} is under failed-review recovery arbitration; startup gate reconcile will not preempt"
            );
            continue;
        }
        // b. origin 精确匹配（gate_id+代次一致）的终态 run → 幂等补偿关。
        let matching = runs.iter().find(|run| {
            run.gate_origin.as_ref().is_some_and(|origin| {
                origin.gate_id == gate_id && origin.generation == state.generation
            }) && matches!(
                run.status,
                CodingRoleRunStatus::Completed
                    | CodingRoleRunStatus::Failed
                    | CodingRoleRunStatus::Blocked
                    | CodingRoleRunStatus::Aborted
            )
        });
        match matching {
            Some(run) => {
                let origin = run.gate_origin.as_ref().expect("origin checked above");
                let outcome = store
                    .resolve_blocked_gate_for_origin(project_id, issue_id, attempt_id, origin)
                    .map_err(|error| {
                        format!("gate {gate_id} origin compensation failed: {error}")
                    })?;
                if outcome.is_some() {
                    actions += 1;
                } else {
                    eprintln!(
                        "[aria-startup] gate {gate_id} origin compensation no-op (closed by another action or generation moved); left for manual attention"
                    );
                }
            }
            None => {
                // d. 无关联/旧版本 → 只告警，JSON 不动。
                eprintln!(
                    "[aria-startup] open gate {gate_id} (generation {}) on attempt {attempt_id} has no origin-matched terminal run; manual attention required, durable JSON untouched",
                    state.generation
                );
            }
        }
    }

    // c. 归档缺 action 且 journal 可证 retry_review → 按证据回填。
    let archived = store
        .archived_blocked_gate_facts_for_attempt(project_id, issue_id, attempt_id)
        .map_err(|error| format!("archived gates unreadable: {error}"))?;
    for facts in archived {
        if facts.resolved_action.is_some() {
            continue;
        }
        let gate_id = facts.gate.gate_id.clone();
        let journal_proves = active_journal.as_ref().is_some_and(|journal| {
            journal.expected_gate_id == gate_id && journal.retry_role_run_id.is_some()
        }) || store
            .get_archived_failed_code_review_recovery_journal(
                project_id, issue_id, attempt_id, &gate_id,
            )
            .map_err(|error| format!("archived journal unreadable: {error}"))?
            .is_some_and(|journal| journal.retry_role_run_id.is_some());
        if journal_proves {
            store
                .backfill_archived_blocked_gate_action(
                    project_id,
                    issue_id,
                    attempt_id,
                    &gate_id,
                    "retry_review",
                )
                .map_err(|error| format!("gate {gate_id} evidence backfill failed: {error}"))?;
            actions += 1;
        } else {
            eprintln!(
                "[aria-startup] archived gate {gate_id} on attempt {attempt_id} lacks resolved_action and no recovery journal proves retry_review; left as-is for manual reconciliation"
            );
        }
    }
    Ok(actions)
}

/// 单 attempt 启动恢复分诊（P2 Task 6）：
/// - registry 已有 runner/预约：幂等跳过（并发抢输不是失败）。
/// - Running：可信半启动，走既有 `ensure_runner_for_resumed_attempt`
///   （SC durable-ready 门、物化判定、双启去重全在 helper 内）。
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
    use crate::web::coding_ws_handler::{ResumedAttemptRunner, ensure_runner_for_resumed_attempt};

    let attempt_key = crate::web::state::CodingAttemptRunKey::from_attempt(attempt);
    if state
        .coding_runs
        .attempt_is_reserved_or_running(&attempt_key)
    {
        return Ok(false);
    }
    // 批 2 T3（REQ-WIGA-09）：重启后终止状态不复活——attempt 所属 plan
    // 已被精确绑定 enrollment 终止时零恢复（T1 admission/gate 已拒执行，
    // 此处补启动扫描面）；manual/无绑定原语义。
    if crate::product::coding_attempt_store::automation_plan_terminated_for_attempt(
        &store.paths(),
        attempt,
    )
    .map_err(|error| format!("termination admission check failed: {error}"))?
    .is_some()
    {
        return Ok(false);
    }
    // T21:树上仍有未终态 reviewer 窗口(人工门/停等/恢复中)→ 该树
    // fail-closed:不启动 runner、不采材料;attempt 投影人工恢复面
    // (幂等),其余无冲突树继续。
    if let Some(worktree) = attempt
        .worktree_path
        .as_deref()
        .filter(|worktree| !worktree.as_os_str().is_empty())
    {
        let (gate_event_tx, _gate_event_rx) = tokio::sync::mpsc::channel(1);
        let engine = crate::product::coding_workspace_engine::CodingWorkspaceEngine::new(
            store.clone(),
            crate::product::git_workspace_service::GitWorkspaceService::new(),
            gate_event_tx,
        );
        if engine
            .ensure_no_open_reviewer_window(std::path::Path::new(worktree))
            .is_err()
        {
            if let Err(error) = store
                .transition_to_awaiting_manual_recovery(&attempt.id, "reviewer_window_open_on_tree")
            {
                eprintln!(
                    "[aria-startup] attempt {} manual-recovery projection failed: {error:?}",
                    attempt.id
                );
            }
            return Ok(false);
        }
    }
    match attempt.status {
        CodingAttemptStatus::Running => {
            // 零订阅者 hub：恢复事件不依赖任何存活 socket。
            let event_tx = state.coding_sockets.hub_sender(&attempt_key);
            match ensure_runner_for_resumed_attempt(state, store, &event_tx, &attempt_key, attempt)
                .await
            {
                ResumedAttemptRunner::Restarted { .. } => Ok(true),
                ResumedAttemptRunner::NotNeeded | ResumedAttemptRunner::ManualRecovery { .. } => {
                    Ok(false)
                }
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
        let transactions =
            crate::product::work_item_plan_store::WorkItemPlanStore::new(paths.clone())
                .list_compile_transactions(&session.project_id, &session.issue_id, plan_id)
                .map_err(|error| format!("bound plan compile transactions unreadable: {error}"))?;
        let latest_failed = transactions
            .iter()
            .max_by(|left, right| left.created_at.cmp(&right.created_at))
            .is_some_and(|latest| {
                matches!(
                    latest.status,
                    WorkItemPlanCompileStatus::Failed | WorkItemPlanCompileStatus::RecoveryRequired
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

/// P2 Task 5 + 批 2 R2：Confirmed 编排链分诊。只在「Confirmed +
/// `plan_confirmed_info` 可派生成功 publication/compile + 当前 enrollment
/// 精确绑定」时接管；N0 未解除 advance/coding_start 失败事实先停点
///（不自动 retry）。目标分派（R2）：LC 绑定（旧单成员＋集合）改走
/// `plan_start` 共享服务——先落 CodingPlanRun 再复用全图驱动
///（首启沿 `deterministic_start_command_id`，删 `wiga-start-{attempt}`
/// 旁路）；物理单仓/旧代无 target 保留单 attempt 自动链。失败/漂移一律
/// fail-closed NeedsHuman 并写 N0 失败事实。其余 session 状态返回 None
/// 交生成准入。
async fn coding_chain_stage(
    state: &WebAppState,
    enrollment: &crate::product::models::automation::IssueAutomationEnrollment,
) -> Result<Option<ReconcileOutcome>, String> {
    use crate::product::models::WorkspaceSessionStatus;

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
    let confirmed = crate::web::plan_confirmed_info::plan_confirmed_info(&paths, enrollment)
        .map_err(|error| format!("plan confirmed info unreadable: {error}"))?;
    if confirmed.is_none() {
        return Ok(Some(ReconcileOutcome::NeedsHuman));
    }
    let Some(plan_id) = enrollment.plan_id.clone() else {
        return Ok(Some(ReconcileOutcome::NeedsHuman));
    };

    // 批 2 R2（§5，REQ-WIGA-12）：当前 binding 的 advance/coding_start 失败
    // 事实未解除即停点（NeedsHuman），不自动 retry；解除走人手/T3
    // continue-chain（成功 resolve 原 key 再 wake）。
    if unresolved_drive_failure_fact(enrollment, &plan_id, session_id) {
        return Ok(Some(ReconcileOutcome::NeedsHuman));
    }

    // R2 目标分派：LC 绑定走共享服务全图驱动；物理单仓/旧代无 target
    // 保留单 attempt 自动链（`bind_ready` 要求 logical snapshot，不能硬塞）。
    let lc_bound = matches!(
        enrollment.target.as_ref(),
        Some(crate::product::logical_codebase::EnrollmentTarget::LogicalCodebase { .. })
            | Some(
                crate::product::logical_codebase::EnrollmentTarget::LogicalCodebaseMembers { .. }
            )
    );
    if lc_bound {
        return enrolled_lc_coding_chain(state, &paths, enrollment, &plan_id).await;
    }
    single_attempt_coding_chain(state, &paths, enrollment, &plan_id).await
}

/// 批 2 R2：N0 未解除的 advance/coding_start 失败事实判定（kind=
/// automation_failed、stage∈{advance,coding_start}、plan/session 与绑定
/// 当前代互证）。
fn unresolved_drive_failure_fact(
    enrollment: &crate::product::models::automation::IssueAutomationEnrollment,
    plan_id: &str,
    session_id: &str,
) -> bool {
    use crate::product::models::automation::{AutomationAttentionKind, AutomationAttentionStage};
    enrollment.attention_facts.iter().any(|fact| {
        fact.kind == AutomationAttentionKind::AutomationFailed
            && matches!(
                fact.stage,
                AutomationAttentionStage::Advance | AutomationAttentionStage::CodingStart
            )
            && fact.plan_id.as_deref() == Some(plan_id)
            && fact.session_id.as_deref() == Some(session_id)
    })
}

/// 批 2 R2（REQ-WIGA-03/C5-CHAIN、REQ-MTG-03）：LC 绑定的共享服务驱动
/// 链——先落 run（prepare，稳定 command=enrollment/plan 身份）→ Enrolled
/// advance → 首启；首项 outcome 落盘/在途即 `drive_ready_units_for_plan`
/// fan-out 其余就绪桶（tick 与完成挂点同源复用，per-plan 串行锁同键）；
/// 终局按 run 冻结 targets 的新鲜状态聚合（单桶 Completed≠全 plan 完成）。
async fn enrolled_lc_coding_chain(
    state: &WebAppState,
    paths: &ProductAppPaths,
    enrollment: &crate::product::models::automation::IssueAutomationEnrollment,
    plan_id: &str,
) -> Result<Option<ReconcileOutcome>, String> {
    use crate::product::coding_plan_run::CodingPlanStartOutcome;
    use crate::product::coding_plan_run::{CodingPlanRunPhase, CodingPlanRunStore};
    use crate::web::handlers::{
        EnrolledPlanDriveIdentity, EnrolledPlanStartStep, drive_ready_units_for_plan,
        enrolled_plan_coding_start,
    };

    let Ok(identity) = EnrolledPlanDriveIdentity::load(paths, enrollment) else {
        // 旧代无版本化绑定/内部投影漂移：fail-closed 人工分诊（无绑定
        // 事实写者授权，不写 N0 事实）。
        return Ok(Some(ReconcileOutcome::NeedsHuman));
    };
    let (run, step) = match enrolled_plan_coding_start(state, &identity).await {
        Ok(result) => result,
        Err(error) => {
            // 业务失败沿 N0 写入事实后按停点分诊；事实写失败
            //（automation_attention_fact_write_failed）按 Err 上抛——
            // tick 可见诊断，不吞成静默 NeedsHuman（不冒充已通知）。
            if error.code == "automation_attention_fact_write_failed" {
                return Err(format!(
                    "enrolled plan coding chain attention fact write failed: {}: {}",
                    error.code, error.message
                ));
            }
            return Ok(Some(ReconcileOutcome::NeedsHuman));
        }
    };
    match &step {
        EnrolledPlanStartStep::AdvancedToReady => return Ok(Some(ReconcileOutcome::Advancing)),
        EnrolledPlanStartStep::Recorded(CodingPlanStartOutcome::NeedsHuman { .. })
        | EnrolledPlanStartStep::Existing(CodingPlanRunPhase::NeedsHuman) => {
            return Ok(Some(ReconcileOutcome::NeedsHuman));
        }
        _ => {}
    }

    // 首项 outcome 已落盘 / run 已在途：全图 fan-out（与完成挂点同函数、
    // 同 per-plan 串行锁；重复 tick 同 claim 零新启）。
    let run_store = CodingPlanRunStore::new(paths.clone());
    let mut run = run_store
        .get(&enrollment.project_id, &enrollment.issue_id, plan_id)
        .map_err(|error| format!("coding plan run unreadable: {error}"))?
        .unwrap_or(run);
    if matches!(
        run.phase,
        CodingPlanRunPhase::Started | CodingPlanRunPhase::AlreadyStarted
    ) {
        let _drive_guard = state
            .coding_runs
            .lock_named(&format!(
                "work_item_group:{}:{}:{}",
                enrollment.project_id, enrollment.issue_id, plan_id
            ))
            .await;
        drive_ready_units_for_plan(
            state,
            &enrollment.project_id,
            &enrollment.issue_id,
            plan_id,
            None,
        )
        .await
        .map_err(|error| {
            format!(
                "enrolled plan fan-out failed: {}: {}",
                error.code, error.message
            )
        })?;
        run = run_store
            .get(&enrollment.project_id, &enrollment.issue_id, plan_id)
            .map_err(|error| format!("coding plan run unreadable: {error}"))?
            .unwrap_or(run);
    }
    Ok(Some(aggregate_plan_targets_fresh_status(paths, &run)?))
}

/// 批 2 R2：run 冻结 targets 的新鲜状态聚合（`coding_chain_stage` 终局
/// 分诊）——任一失败/人工恢复 → NeedsHuman；任一人控门 → AwaitingHuman；
/// 其余（Created/Running/Completed/amendment）为执行或观察：单桶
/// Completed≠全 plan 完成（其余桶继续由 fan-out 驱动）。空绑定集
///（ready_bound 等待窗口）视为执行观察。
pub(crate) fn aggregate_plan_targets_fresh_status(
    paths: &ProductAppPaths,
    run: &crate::product::coding_plan_run::CodingPlanRun,
) -> Result<ReconcileOutcome, String> {
    use crate::product::coding_models::CodingAttemptStatus;

    let store = crate::product::coding_attempt_store::CodingAttemptStore::new(paths.clone());
    let mut awaiting_human = false;
    for binding in &run.target_attempts {
        let attempt = store
            .get_attempt(&run.project_id, &run.issue_id, &binding.attempt_id)
            .map_err(|error| format!("run target attempt unreadable: {error}"))?;
        match attempt.status {
            CodingAttemptStatus::Failed
            | CodingAttemptStatus::Aborted
            | CodingAttemptStatus::AwaitingManualRecovery => {
                return Ok(ReconcileOutcome::NeedsHuman);
            }
            CodingAttemptStatus::WaitingForHuman | CodingAttemptStatus::Blocked => {
                awaiting_human = true;
            }
            CodingAttemptStatus::Created
            | CodingAttemptStatus::Running
            | CodingAttemptStatus::Completed
            | CodingAttemptStatus::AwaitingPlanAmendment
            | CodingAttemptStatus::ApplyingPlanAmendment
            | CodingAttemptStatus::AmendmentApplyFailed => {}
        }
    }
    if awaiting_human {
        Ok(ReconcileOutcome::AwaitingHuman)
    } else {
        Ok(ReconcileOutcome::Coding)
    }
}

/// 物理单仓（及旧代无 target enrollment）的单 attempt 自动链（P2 Task 5
/// 原语义，R2 保留）：未 Ready 先 stable-id `advance_plan`（到 Ready 即
/// 止）；Ready 后按 group journal 定位唯一 attempt，按状态分诊或 stable-id
/// AutoStartOnce（`wiga-start-{attempt_id}`）。失败沿 N0 写
/// `automation_failed` 事实（瞬态准入争用除外，下一 tick 重试）。
async fn single_attempt_coding_chain(
    state: &WebAppState,
    paths: &ProductAppPaths,
    enrollment: &crate::product::models::automation::IssueAutomationEnrollment,
    plan_id: &str,
) -> Result<Option<ReconcileOutcome>, String> {
    use crate::product::advance_store::{
        AdvanceInput, AdvanceOutcome, AdvanceStatus, AdvanceStore,
    };
    use crate::product::coding_models::{CodingAttemptStatus, CodingStartOrigin};
    use crate::product::models::automation::AutomationAttentionStage;
    use crate::web::advance_plan::{AdvancePlanOrigin, advance_plan};
    use crate::web::coding_start::{StartCodingCommand, StartCodingOutcome, start_coding_once};
    use crate::web::handlers::upsert_automation_drive_failure_fact;

    let project_id = enrollment.project_id.clone();
    let issue_id = enrollment.issue_id.clone();
    let advance_command_id = format!("wiga-advance-{}-{plan_id}", enrollment.enrollment_id);

    let advance_ready = AdvanceStore::new(paths.clone())
        .get_advance_for_plan(&project_id, &issue_id, plan_id)
        .map_err(|error| format!("advance record unreadable: {error}"))?
        .is_some_and(|record| record.status == AdvanceStatus::Ready);
    if !advance_ready {
        let outcome = advance_plan(
            state,
            AdvanceInput {
                command_id: advance_command_id.clone(),
                project_id: project_id.clone(),
                issue_id: issue_id.clone(),
                plan_id: plan_id.to_string(),
            },
            AdvancePlanOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
            },
        )
        .await;
        let proceed = match &outcome {
            Ok(AdvanceOutcome::Completed { record, .. })
                if record.status == AdvanceStatus::Ready =>
            {
                return Ok(Some(ReconcileOutcome::Advancing));
            }
            Ok(AdvanceOutcome::Replayed { record }) if record.status == AdvanceStatus::Ready => {
                true
            }
            Ok(AdvanceOutcome::Rejected { code, .. })
                if code == "ADVANCE_PLAN_ADMISSION_CONTENDED" =>
            {
                // 瞬态准入争用：不写事实，下一 tick 重试。
                return Ok(Some(ReconcileOutcome::NeedsHuman));
            }
            other => {
                // R2（REQ-WIGA-12）：advance 失败写 N0 事实（checkpoint 取
                // record.updated_at，无 record 首次冻结）；写失败上抛。
                let (reason_code, checkpoint) = match other {
                    Ok(AdvanceOutcome::Rejected { record, code, .. }) => {
                        (code.clone(), record.as_ref().map(|r| r.updated_at.clone()))
                    }
                    Ok(AdvanceOutcome::Completed { record, .. })
                    | Ok(AdvanceOutcome::Replayed { record }) => (
                        "enrolled_advance_not_ready".to_string(),
                        Some(record.updated_at.clone()),
                    ),
                    Err(_) => (
                        "enrolled_advance_failed".to_string(),
                        AdvanceStore::new(paths.clone())
                            .get_advance_for_plan(&project_id, &issue_id, plan_id)
                            .ok()
                            .flatten()
                            .map(|record| record.updated_at),
                    ),
                };
                upsert_automation_drive_failure_fact(
                    paths,
                    enrollment,
                    AutomationAttentionStage::Advance,
                    plan_id,
                    &advance_command_id,
                    None,
                    &reason_code,
                    checkpoint,
                )?;
                return Ok(Some(ReconcileOutcome::NeedsHuman));
            }
        };
        if !proceed {
            return Ok(Some(ReconcileOutcome::NeedsHuman));
        }
    }

    // Ready：group journal 只用于定位唯一 attempt 身份；状态必须从 attempt
    // store 读新鲜值——journal 内嵌快照冻结于初始化时点（恒 Created/
    // PrepareContext），据其分诊会在 attempt 已停等人时重入首启路径，
    // 把 durable claim 误降级 NeedsHuman。
    let journal = crate::product::coding_attempt_store::CodingAttemptStore::new(paths.clone())
        .get_group_initialization(&project_id, &issue_id, plan_id)
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
            let command_id = format!("wiga-start-{}", attempt.id);
            let start = start_coding_once(
                state,
                &project_id,
                &issue_id,
                StartCodingCommand {
                    attempt_id: attempt.id.clone(),
                    command_id: command_id.clone(),
                    origin: CodingStartOrigin::Enrolled {
                        enrollment_id: enrollment.enrollment_id.clone(),
                        policy_revision: enrollment.policy_revision,
                        // C1 Task 1：claim 携带当前 binding 身份（旧
                        // enrollment 无声明时 None，消费侧 fail-closed）。
                        binding_version: enrollment
                            .binding_history
                            .as_ref()
                            .map(|history| history.current.binding_version),
                        target: enrollment.target.clone(),
                    },
                },
            )
            .await;
            match start {
                Ok(StartCodingOutcome::Started { .. })
                | Ok(StartCodingOutcome::AlreadyStarted { .. }) => {
                    Ok(Some(ReconcileOutcome::Coding))
                }
                Ok(StartCodingOutcome::NeedsHuman { attempt_id, reason }) => {
                    // R2（REQ-WIGA-12）：人控分诊是 coding_start 失败停点。
                    upsert_automation_drive_failure_fact(
                        paths,
                        enrollment,
                        AutomationAttentionStage::CodingStart,
                        plan_id,
                        &command_id,
                        Some(attempt_id),
                        &reason,
                        attempt
                            .start_claim
                            .as_ref()
                            .map(|claim| claim.claimed_at.clone()),
                    )?;
                    Ok(Some(ReconcileOutcome::NeedsHuman))
                }
                Err(error) => {
                    upsert_automation_drive_failure_fact(
                        paths,
                        enrollment,
                        AutomationAttentionStage::CodingStart,
                        plan_id,
                        &command_id,
                        Some(attempt.id.clone()),
                        error.code(),
                        attempt
                            .start_claim
                            .as_ref()
                            .map(|claim| claim.claimed_at.clone()),
                    )?;
                    Ok(Some(ReconcileOutcome::NeedsHuman))
                }
            }
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
        assert_eq!(put_enrollment(&app, disable).await.status(), StatusCode::OK);
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

        // crash-window 注入是进程级全局：与同样驱动 ensure_enrolled_plan /
        // reconcile 的 g8_delegated_orphan 测试互斥（并行会互偷窗口）。
        let _crash_window_serial = super::CRASH_WINDOW_SERIAL.lock().await;

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
                    assert_eq!(fixture.provider_start_count(), 0, "崩溃先于 provider 派发");
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

    // ─── 批 2 R2（REQ-WIGA-03/C5-CHAIN、REQ-MTG-03、REQ-WIGA-12）───
    // LC（单成员/集合）绑定改走 plan_start 共享服务：先落 CodingPlanRun
    // 再开独立根、首启沿 deterministic_start_command_id（删 wiga-start
    // 旁路）、Enrolled origin、tick 同 claim 幂等；advance/coding_start
    // 失败写 N0 automation_failed 事实且未解除即停点。

    /// R2 ①②④：单成员 LC 绑定——tick1 先落 run（Advancing、零 runner），
    /// tick2 经共享服务 Enrolled 首启（claim origin=Enrolled、command 为
    /// run.command_id 的确定性派生而非 wiga-start 旁路、单桶物化恰一条
    /// target 绑定），tick3 同 claim 零重启。
    #[tokio::test]
    async fn r2_lc_chain_creates_run_first_and_starts_enrolled_origin() {
        use crate::product::coding_plan_run::{
            CodingPlanRunPhase, CodingPlanRunStore, CodingPlanSelection,
        };

        let fixture = crate::web::wiga_gate_fixture::confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let plan_id = enrollment.plan_id.clone().expect("bound plan");

        // tick1：advance 到 Ready 即止——run 已先落盘、零 runner。
        assert_eq!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::Advancing
        );
        let run_store = CodingPlanRunStore::new(fixture.inner.paths.clone());
        let run = run_store
            .get(PROJECT_ID, ISSUE_ID, &plan_id)
            .unwrap()
            .expect("coding plan run persisted before any root opens");
        assert_eq!(
            run.command_id,
            format!("wiga-advance-{}-{plan_id}", enrollment.enrollment_id),
            "run command uses the stable enrollment/plan identity"
        );
        assert_eq!(run.phase, CodingPlanRunPhase::Prepared);
        assert_eq!(fixture.coding_runner_count(), 0);

        // tick2：共享服务 bind→mark→Enrolled start→record。
        assert_eq!(fixture.reconcile().await.unwrap(), ReconcileOutcome::Coding);
        let run = run_store
            .get(PROJECT_ID, ISSUE_ID, &plan_id)
            .unwrap()
            .expect("run survives the start tick");
        let attempt = fixture.attempt();
        // 单实际桶：引擎空 target_attempts，由共享服务物化恰一条绑定。
        assert_eq!(run.target_attempts.len(), 1);
        assert_eq!(
            run.target_attempts[0].attempt_id, attempt.id,
            "materialized binding carries the real single-bucket attempt"
        );
        let claim = attempt.start_claim.as_ref().expect("enrolled first start");
        assert_eq!(claim.origin, fixture.auto_origin());
        let expected_command = CodingPlanSelection::deterministic_start_command_id(
            PROJECT_ID,
            ISSUE_ID,
            &plan_id,
            &run.command_id,
            &run.target_attempts[0].target_repository_id,
        );
        assert_eq!(
            claim.command_id, expected_command,
            "first start follows the deterministic command; the wiga-start bypass is gone"
        );
        assert_ne!(claim.command_id, format!("wiga-start-{}", attempt.id));
        assert_eq!(fixture.runner_count(&fixture.attempt_key()), 1);

        // tick3：重复唤醒同 claim、零重启。
        assert_eq!(fixture.reconcile().await.unwrap(), ReconcileOutcome::Coding);
        assert_eq!(fixture.runner_count(&fixture.attempt_key()), 1);
        let replay = fixture.attempt();
        assert_eq!(
            replay.start_claim.as_ref().unwrap().command_id,
            expected_command
        );
    }

    /// R2 ⑥：advance 链失败（plan 进入在途 amendment → 共享服务 prepare
    /// fail-closed）写 N0 `automation_failed(stage=advance)` 事实；未解除
    /// 事实令后续 tick 停在该点（不自动 retry）、同 key 不增项、
    /// occurred_at 首次冻结。
    #[tokio::test]
    async fn r2_advance_failure_writes_fact_and_gates_following_ticks() {
        use crate::product::issue_automation_store::IssueAutomationStore;
        use crate::product::models::automation::{
            AutomationAttentionKind, AutomationAttentionStage,
        };
        use crate::product::work_item_revision_store::WorkItemRevisionStore;

        let fixture = crate::web::wiga_gate_fixture::confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let plan_id = enrollment.plan_id.clone().expect("bound plan");
        // 篡改现场：plan 出现在途 amendment（durable lineage 事实）——
        // Confirmed session/compile 投影不受影响，共享服务 prepare
        // fail-closed（coding_plan_run_active_amendment）。
        let revision_store = WorkItemRevisionStore::new(fixture.inner.paths.clone());
        let mut lineage = revision_store
            .get_plan_lineage(PROJECT_ID, ISSUE_ID, &plan_id)
            .unwrap();
        lineage.active_amendment_id = Some("amendment_r2_tamper".to_string());
        crate::product::json_store::write_json(
            &fixture
                .inner
                .paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("work-item-revisions")
                .join(&plan_id)
                .join("lineage.json"),
            &lineage,
        )
        .unwrap();

        assert_eq!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::NeedsHuman
        );

        let store = IssueAutomationStore::new(fixture.inner.paths.clone());
        let saved = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        let command_id = format!("wiga-advance-{}-{plan_id}", enrollment.enrollment_id);
        let expected_key = format!("{plan_id}:{command_id}:advance_failed");
        let fact = saved
            .attention_facts
            .iter()
            .find(|fact| fact.key == expected_key)
            .unwrap_or_else(|| panic!("advance failure fact missing: {:?}", saved.attention_facts));
        assert_eq!(fact.kind, AutomationAttentionKind::AutomationFailed);
        assert_eq!(fact.stage, AutomationAttentionStage::Advance);
        assert_eq!(fact.plan_id.as_deref(), Some(plan_id.as_str()));
        assert_eq!(fact.session_id.as_deref(), enrollment.session_id.as_deref());
        assert!(!fact.reason_code.is_empty());
        let frozen_at = fact.occurred_at.clone();
        assert!(
            fixture.coding_attempts().is_empty(),
            "failed advance must not create attempts"
        );

        // 后续 tick：未解除事实停点（不自动 retry）、同 key 单条、时间冻结。
        assert_eq!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::NeedsHuman
        );
        let saved = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        let matches: Vec<_> = saved
            .attention_facts
            .iter()
            .filter(|fact| fact.key == expected_key)
            .collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].occurred_at, frozen_at);
        assert!(fixture.coding_attempts().is_empty());
    }

    /// R2 ③：Ready 后冻结策略被篡改为 Manual（不给旧 Manual 原地补授权）
    /// ——共享服务仍以 Enrolled 身份准入，fail-closed 零 provider 并写
    /// coding_start 失败事实。
    #[tokio::test]
    async fn r2_tampered_manual_policy_rejects_start_without_downgrade() {
        use crate::product::coding_models::CodingStartRunPolicy;
        use crate::product::issue_automation_store::IssueAutomationStore;
        use crate::product::models::automation::{
            AutomationAttentionKind, AutomationAttentionStage,
        };

        let fixture = crate::web::wiga_gate_fixture::confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let plan_id = enrollment.plan_id.clone().expect("bound plan");
        assert_eq!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::Advancing
        );
        // 篡改：把 Ready attempt 的冻结策略改写为 Manual。
        let attempt = fixture.attempt();
        let mut tampered = attempt.clone();
        tampered.start_run_policy = CodingStartRunPolicy::Manual;
        fixture
            .store()
            .write_coding_attempt_for_test(&tampered)
            .unwrap();

        assert_eq!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::NeedsHuman
        );
        assert_eq!(
            fixture.runner_count(&fixture.attempt_key()),
            0,
            "tampered policy must yield zero provider and no manual downgrade"
        );
        let saved = IssueAutomationStore::new(fixture.inner.paths.clone())
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap();
        assert!(
            saved.attention_facts.iter().any(|fact| {
                fact.kind == AutomationAttentionKind::AutomationFailed
                    && fact.stage == AutomationAttentionStage::CodingStart
                    && fact.plan_id.as_deref() == Some(plan_id.as_str())
            }),
            "coding_start failure fact must be persisted: {:?}",
            saved.attention_facts
        );
    }

    /// k3 波3 fix-2：N0 事实写失败（enrollment 文件不可写→upsert Store
    /// 错误）不得吞成静默 NeedsHuman——该 tick 必须以 Err 产出可见诊断
    ///（automation_attention_fact_write_failed）；恢复可写后同一现场
    /// 正常写事实并按停点分诊。
    #[tokio::test]
    async fn r2_attention_fact_write_failure_surfaces_as_tick_error() {
        use crate::product::work_item_revision_store::WorkItemRevisionStore;

        let fixture = crate::web::wiga_gate_fixture::confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let plan_id = enrollment.plan_id.clone().expect("bound plan");
        // 同 r2_advance_failure 用例的在途 amendment 注入：prepare fail-closed
        // → 服务内尝试写 advance 失败事实。
        let revision_store = WorkItemRevisionStore::new(fixture.inner.paths.clone());
        let mut lineage = revision_store
            .get_plan_lineage(PROJECT_ID, ISSUE_ID, &plan_id)
            .unwrap();
        lineage.active_amendment_id = Some("amendment_r2_k3".to_string());
        crate::product::json_store::write_json(
            &fixture
                .inner
                .paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("work-item-revisions")
                .join(&plan_id)
                .join("lineage.json"),
            &lineage,
        )
        .unwrap();
        // N0 写入口不可写（issue 目录只读→锁文件/落盘 IO 失败形态；
        // write_json 为临时文件+rename，单文件 chmod 拦不住）。
        let enrollment_dir = fixture.inner.paths.issue_root(PROJECT_ID, ISSUE_ID);
        let mut readonly = std::fs::metadata(&enrollment_dir).unwrap().permissions();
        readonly.set_readonly(true);
        std::fs::set_permissions(&enrollment_dir, readonly).unwrap();

        let failure = fixture
            .reconcile()
            .await
            .expect_err("fact write failure must surface as a tick error, not silent needs_human");
        assert!(
            failure.contains("automation_attention_fact_write_failed"),
            "tick diagnostic must carry the fact-write failure code: {failure}"
        );

        // 恢复可写：同一现场正常写事实并分诊停点（不冒充已通知）。
        let mut writable = std::fs::metadata(&enrollment_dir).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)] // 测试恢复可写:unix 下 false 即加回写位。
        writable.set_readonly(false);
        std::fs::set_permissions(&enrollment_dir, writable).unwrap();
        assert_eq!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::NeedsHuman
        );
        let saved = crate::product::issue_automation_store::IssueAutomationStore::new(
            fixture.inner.paths.clone(),
        )
        .get(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .unwrap();
        assert!(
            saved.attention_facts.iter().any(|fact| fact.key
                == format!(
                    "{plan_id}:wiga-advance-{}-{plan_id}:advance_failed",
                    enrollment.enrollment_id
                )),
            "restored write path must persist the advance failure fact: {:?}",
            saved.attention_facts
        );
    }
}

include!("autopilot_orchestrator_parts/test_mods.inc.rs");
