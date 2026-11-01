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
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::automation::{
    EnrollmentError, EnrollmentTerminationRequest, IssueAutomationEnrollment,
};
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
    /// 单 issue 失败退避（进程级内存态，只用于节流重试）：Err 轮累积、
    /// 窗口内跳过；成功轮清零。条目规模上界 = issue 数。
    backoff: tokio::sync::Mutex<BTreeMap<(String, String), ReconcileBackoff>>,
}

/// 单 issue 失败退避状态：`next_attempt_at` 之前不再重试该 issue。
#[derive(Debug, Clone, Copy)]
struct ReconcileBackoff {
    failures: u32,
    next_attempt_at: std::time::Instant,
}

/// 首败退避 2s，此后每败翻倍，最长 5 分钟；避免不可自愈/瞬时错误按
/// tick 间隔（缺省 2s）高频刷屏。
const RECONCILE_BACKOFF_BASE: std::time::Duration = std::time::Duration::from_secs(2);
const RECONCILE_BACKOFF_MAX: std::time::Duration = std::time::Duration::from_secs(300);

fn reconcile_backoff_delay(failures: u32) -> std::time::Duration {
    let exponent = failures.saturating_sub(1).min(8);
    RECONCILE_BACKOFF_BASE
        .saturating_mul(1u32 << exponent)
        .min(RECONCILE_BACKOFF_MAX)
}

impl AutopilotOrchestrator {
    pub fn new(state: WebAppState, config: OrchestratorConfig) -> Self {
        Self {
            state,
            config,
            cursor: tokio::sync::Mutex::new(None),
            backoff: tokio::sync::Mutex::new(BTreeMap::new()),
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
            let failure = format!(
                "ensure enrolled plan failed: {}: {}",
                error.code, error.message
            );
            match error.code.as_str() {
                "automation_enrollment_conflict" | "automation_enrollment_invalid_scope" => {
                    return Ok(ReconcileOutcome::NeedsHuman);
                }
                _ => {}
            }
            // 绑定实体（plan/session）已被删除是不可自愈事实：同源 CAS 置
            // terminated（与人工终止同一 durable 事实），本轮停在 NeedsHuman；
            // 之后每轮在 terminated 显式跳过，绝不再 Err 无限重试。
            if bound_entity_missing(state, &enrollment)? {
                terminate_deleted_binding(state, &enrollment, &failure)?;
                return Ok(ReconcileOutcome::NeedsHuman);
            }
            return Err(failure);
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
            if self.reconcile_in_backoff(project_id, issue_id).await {
                // 退避窗口内：本轮跳过重试（不打印、不累积），游标照常推进，
                // 避免同一不可自愈错误按 tick 间隔刷屏。
                last = Some((project_id.clone(), issue_id.clone()));
                continue;
            }
            match self.reconcile(&self.state, project_id, issue_id).await {
                Ok(_) => self.clear_reconcile_backoff(project_id, issue_id).await,
                Err(message) => {
                    eprintln!(
                        "wiga autopilot reconcile failed for {project_id}/{issue_id}: {message}"
                    );
                    self.record_reconcile_failure(project_id, issue_id).await;
                }
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

    /// 该 issue 是否仍在失败退避窗口内。
    async fn reconcile_in_backoff(&self, project_id: &str, issue_id: &str) -> bool {
        let backoff = self.backoff.lock().await;
        backoff
            .get(&(project_id.to_string(), issue_id.to_string()))
            .is_some_and(|entry| entry.next_attempt_at > std::time::Instant::now())
    }

    /// 记录一次失败并推进指数退避（2s → 4s → … → 5 分钟封顶）。
    async fn record_reconcile_failure(&self, project_id: &str, issue_id: &str) {
        let key = (project_id.to_string(), issue_id.to_string());
        let mut backoff = self.backoff.lock().await;
        let entry = backoff.entry(key).or_insert(ReconcileBackoff {
            failures: 0,
            next_attempt_at: std::time::Instant::now(),
        });
        entry.failures = entry.failures.saturating_add(1);
        entry.next_attempt_at = std::time::Instant::now() + reconcile_backoff_delay(entry.failures);
    }

    /// 成功一轮即清零：错误消失后立即恢复正常 tick 节律。
    async fn clear_reconcile_backoff(&self, project_id: &str, issue_id: &str) {
        self.backoff
            .lock()
            .await
            .remove(&(project_id.to_string(), issue_id.to_string()));
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

/// 绑定实体被删除的可证明判定：enrollment 的 durable plan/session 在其
/// issue 作用域内已不可见（列表里没有该 id）。只在 ensure 失败后调用：
/// 仅「可证明缺失」成立；列表自身读失败返回 Err（落退避），绝不误终止。
fn bound_entity_missing(
    state: &WebAppState,
    enrollment: &IssueAutomationEnrollment,
) -> Result<bool, String> {
    let paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let lifecycle = LifecycleStore::new(paths);
    if let Some(plan_id) = enrollment.plan_id.as_deref() {
        let plans = lifecycle
            .list_issue_work_item_plans(&enrollment.project_id, &enrollment.issue_id)
            .map_err(|error| format!("bound plan list unreadable: {error}"))?;
        if !plans.iter().any(|plan| plan.id == plan_id) {
            return Ok(true);
        }
    }
    if let Some(session_id) = enrollment.session_id.as_deref() {
        let sessions = lifecycle
            .list_workspace_sessions(&enrollment.project_id, &enrollment.issue_id)
            .map_err(|error| format!("bound session list unreadable: {error}"))?;
        if !sessions.iter().any(|session| session.id == session_id) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 绑定实体被删除是不可自愈事实：走与人工终止同源的 CAS 把 enrollment 置
/// terminated（`terminated_at` + `enabled=false` 同锁落盘），此后 reconcile
/// 显式跳过，绝不再逐 tick 重试 ensure。同 revision 上的命令 id 稳定 →
/// 重放幂等；CAS 竞态/存储失败以 Err 交回调用方（由 tick 退避节流）。
fn terminate_deleted_binding(
    state: &WebAppState,
    enrollment: &IssueAutomationEnrollment,
    observed: &str,
) -> Result<(), String> {
    let paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let store = IssueAutomationStore::new(paths);
    let request = EnrollmentTerminationRequest {
        command_id: format!(
            "wiga-reconcile-terminate-{}-r{}",
            enrollment.enrollment_id, enrollment.policy_revision
        ),
        expected_enrollment_id: enrollment.enrollment_id.clone(),
        expected_policy_revision: enrollment.policy_revision,
        reason: format!("bound entity deleted: {observed}"),
    };
    match store.terminate_plan(&enrollment.project_id, &enrollment.issue_id, request) {
        Ok(_) => {
            eprintln!(
                "wiga autopilot terminated enrollment {}/{}: bound entity deleted ({observed})",
                enrollment.project_id, enrollment.issue_id
            );
            Ok(())
        }
        // enrollment 已消失：无可终止，下一轮自然 NoEnrollment。
        Err(EnrollmentError::NotFound) => Ok(()),
        Err(error) => Err(format!(
            "terminate enrollment after bound entity deletion failed: {error}"
        )),
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

include!("autopilot_orchestrator_parts/coding_chain.inc.rs");

#[cfg(test)]
mod tests {
    include!("autopilot_orchestrator_parts/tests.inc.rs");
}

include!("autopilot_orchestrator_parts/test_mods.inc.rs");
