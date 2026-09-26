use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::cross_cutting::choice_delivery::{ChoiceDeliverySignal, ChoiceReplyState};
use crate::cross_cutting::streaming_provider::ChoiceAnswerData;
use crate::web::choice_reply::{ChoiceReplyStatus, ChoiceResponseRequest};
use crate::web::workspace_session::ChoiceReplyError;

#[derive(Clone, Default)]
pub struct CodingRunRegistry {
    inner: Arc<StdMutex<CodingRunRegistryInner>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CodingAttemptRunKey {
    project_id: String,
    issue_id: String,
    attempt_id: String,
}

impl CodingAttemptRunKey {
    pub fn new(
        project_id: impl Into<String>,
        issue_id: impl Into<String>,
        attempt_id: impl Into<String>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            issue_id: issue_id.into(),
            attempt_id: attempt_id.into(),
        }
    }

    pub fn from_attempt(attempt: &CodingExecutionAttempt) -> Self {
        Self::new(&attempt.project_id, &attempt.issue_id, &attempt.id)
    }
}

#[derive(Default)]
struct CodingRunRegistryInner {
    next_run_id: u64,
    runs: HashMap<CodingAttemptRunKey, HashMap<u64, CodingRunEntry>>,
    reservations: HashMap<CodingAttemptRunKey, u64>,
    exclusive_runs: HashMap<CodingAttemptRunKey, u64>,
    retired_attempts: HashSet<CodingAttemptRunKey>,
    attempt_guards: HashMap<CodingAttemptRunKey, Arc<AsyncMutex<()>>>,
    attempt_mutation_guards: HashMap<CodingAttemptRunKey, Arc<AsyncMutex<()>>>,
    named_guards: HashMap<String, Arc<AsyncMutex<()>>>,
    /// P0 1.3（Task 9）：coding choice 应答 claim（短锁登记；提交/回执锁外）。
    choice_claims: HashMap<(CodingAttemptRunKey, String), CodingChoiceClaimRecord>,
    /// run 终态后保留的有界终态登记（同 command 查询面）。
    finished_choice_status: HashMap<CodingAttemptRunKey, VecDeque<ChoiceReplyStatus>>,
}

struct CodingRunEntry {
    command_tx: mpsc::Sender<CodingRunnerCommand>,
    completion_tx: watch::Sender<bool>,
    cancellation: CancellationToken,
    /// P0 1.3：每次启动生成且永不复用的 run 化身——run_id/token 是可重启
    /// 归零的进程内序号，不能作为「同一 run」的应答身份判据。
    run_incarnation: String,
}

/// P0 1.3（Task 9）：与 workspace 侧 ChoiceClaimRecord 同语义的 coding 版。
struct CodingChoiceClaimRecord {
    command_id: String,
    /// 完整 answers 指纹——幂等重发判据。
    fingerprint: String,
    status: ChoiceReplyState,
    receipt: ChoiceDeliverySignal,
    incarnation: String,
}

/// 同 command 终态登记上限（超出淘汰最旧）。
const CODING_FINISHED_CHOICE_STATUS_CAP: usize = 32;

fn coding_answers_fingerprint(answers: &[ChoiceAnswerData]) -> String {
    serde_json::to_string(answers).unwrap_or_default()
}

/// 从完整 answers 派生 legacy 单题字段（与 workspace 侧一致）。
fn coding_legacy_fields_from_answers(
    answers: &[ChoiceAnswerData],
) -> (Vec<String>, Option<String>) {
    let selected = answers
        .iter()
        .flat_map(|answer| answer.selected_option_ids.iter().cloned())
        .collect::<Vec<_>>();
    let free_text = answers.iter().find_map(|answer| {
        answer
            .free_text
            .clone()
            .filter(|text| !text.trim().is_empty())
    });
    (selected, free_text)
}

fn coding_claim_status(
    record: &CodingChoiceClaimRecord,
    choice_id: &str,
) -> ChoiceReplyStatus {
    ChoiceReplyStatus {
        command_id: record.command_id.clone(),
        expected_run_id: record.incarnation.clone(),
        choice_id: choice_id.to_string(),
        state: record.status,
    }
}

fn push_coding_finished_status(
    inner: &mut CodingRunRegistryInner,
    key: &CodingAttemptRunKey,
    status: ChoiceReplyStatus,
) {
    let queue = inner
        .finished_choice_status
        .entry(key.clone())
        .or_default();
    queue.push_back(status);
    while queue.len() > CODING_FINISHED_CHOICE_STATUS_CAP {
        queue.pop_front();
    }
}

/// run 终态（remove/abort/retire）：该 attempt 的旧 claim 一律 Expired，
/// 回执同步置位，状态保留有界查询面。
fn retire_coding_choice_claims(inner: &mut CodingRunRegistryInner, key: &CodingAttemptRunKey) {
    let keys: Vec<String> = inner
        .choice_claims
        .keys()
        .filter(|(attempt_key, _)| attempt_key == key)
        .map(|(_, choice_id)| choice_id.clone())
        .collect();
    for choice_id in keys {
        if let Some(mut record) = inner.choice_claims.remove(&(key.clone(), choice_id.clone())) {
            record.receipt.expire();
            record.status = ChoiceReplyState::Expired;
            push_coding_finished_status(
                inner,
                key,
                ChoiceReplyStatus {
                    command_id: record.command_id,
                    expected_run_id: record.incarnation,
                    choice_id,
                    state: ChoiceReplyState::Expired,
                },
            );
        }
    }
}

/// watch 终态等待（同 workspace wait_for_terminal_choice_state）。
async fn coding_wait_terminal_choice_state(
    observer: &mut tokio::sync::watch::Receiver<ChoiceReplyState>,
) -> ChoiceReplyState {
    loop {
        let current = *observer.borrow();
        if !matches!(
            current,
            ChoiceReplyState::Submitting | ChoiceReplyState::Resolving
        ) {
            return current;
        }
        if observer.changed().await.is_err() {
            return *observer.borrow();
        }
    }
}

pub struct CodingRunRegistration {
    pub(crate) run_id: u64,
    pub(crate) cancellation: CancellationToken,
}

impl CodingRunRegistration {
    pub fn run_id(&self) -> u64 {
        self.run_id
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

pub(crate) struct CodingAttemptMutationLease {
    _guard: OwnedMutexGuard<()>,
}

pub struct CodingRunReservation {
    registry: CodingRunRegistry,
    attempt_key: CodingAttemptRunKey,
    reservation_id: u64,
    released: bool,
}

impl CodingRunReservation {
    pub(crate) fn activate_cancellable(
        mut self,
        command_tx: mpsc::Sender<CodingRunnerCommand>,
    ) -> Option<CodingRunRegistration> {
        let mut inner = self
            .registry
            .inner
            .lock()
            .expect("coding run registry lock");
        if inner.retired_attempts.contains(&self.attempt_key)
            || inner.reservations.get(&self.attempt_key) != Some(&self.reservation_id)
        {
            return None;
        }
        inner.reservations.remove(&self.attempt_key);
        let cancellation = CancellationToken::new();
        let (completion_tx, _completion_rx) = watch::channel(false);
        inner
            .runs
            .entry(self.attempt_key.clone())
            .or_default()
            .insert(
                self.reservation_id,
                CodingRunEntry {
                    command_tx,
                    completion_tx,
                    cancellation: cancellation.clone(),
                    run_incarnation: uuid::Uuid::new_v4().to_string(),
                },
            );
        inner
            .exclusive_runs
            .insert(self.attempt_key.clone(), self.reservation_id);
        self.released = true;
        Some(CodingRunRegistration {
            run_id: self.reservation_id,
            cancellation,
        })
    }

    pub fn release(mut self) {
        self.registry
            .release_reservation(&self.attempt_key, self.reservation_id);
        self.released = true;
    }
}

impl Drop for CodingRunReservation {
    fn drop(&mut self) {
        if !self.released {
            self.registry
                .release_reservation(&self.attempt_key, self.reservation_id);
        }
    }
}

impl CodingRunRegistry {
    pub fn insert_cancellable(
        &self,
        attempt_key: &CodingAttemptRunKey,
        command_tx: mpsc::Sender<CodingRunnerCommand>,
    ) -> Option<CodingRunRegistration> {
        let mut inner = self.inner.lock().expect("coding run registry lock");
        if inner.retired_attempts.contains(attempt_key)
            || inner.reservations.contains_key(attempt_key)
            || inner.exclusive_runs.contains_key(attempt_key)
        {
            return None;
        }
        inner.next_run_id += 1;
        let run_id = inner.next_run_id;
        let cancellation = CancellationToken::new();
        let (completion_tx, _completion_rx) = watch::channel(false);
        inner.runs.entry(attempt_key.clone()).or_default().insert(
            run_id,
            CodingRunEntry {
                command_tx,
                completion_tx,
                cancellation: cancellation.clone(),
                run_incarnation: uuid::Uuid::new_v4().to_string(),
            },
        );
        Some(CodingRunRegistration {
            run_id,
            cancellation,
        })
    }

    pub fn remove(&self, attempt_key: &CodingAttemptRunKey, run_id: u64) {
        let mut inner = self.inner.lock().expect("coding run registry lock");
        if inner.exclusive_runs.get(attempt_key) == Some(&run_id) {
            inner.exclusive_runs.remove(attempt_key);
        }
        if let Some(runs) = inner.runs.get_mut(attempt_key) {
            if let Some(entry) = runs.remove(&run_id) {
                entry.completion_tx.send_replace(true);
            }
            if runs.is_empty() {
                inner.runs.remove(attempt_key);
                retire_coding_choice_claims(&mut inner, attempt_key);
            }
        }
    }

    pub async fn abort_attempt(&self, attempt_key: &CodingAttemptRunKey) -> usize {
        let runners = {
            let mut inner = self.inner.lock().expect("coding run registry lock");
            inner.retired_attempts.insert(attempt_key.clone());
            inner.reservations.remove(attempt_key);
            inner
                .runs
                .get(attempt_key)
                .map(|runs| {
                    runs.iter()
                        .map(|(run_id, entry)| {
                            (
                                *run_id,
                                entry.command_tx.clone(),
                                entry.cancellation.clone(),
                                entry.completion_tx.subscribe(),
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let mut sent = 0;
        for (run_id, _, cancellation, _) in &runners {
            // D①（诊断打点，不改行为）：指名取消者。registry abort_attempt 对
            // attempt_key 下每个 runner token 的取消，是握手期 cancel 被
            // provider_stream 的 biased-select masking 成「handshake cancelled」
            // 错误文案时，唯一不被掩盖的信号源（H1/H2 勘察确证入口）。
            // aria 二进制未安装 tracing subscriber；生产构建 eprintln 直写（同 D① 可见性修正），测试构建保留 tracing 捕获断言。
            #[cfg(not(test))]
            eprintln!(
                "[aria-cancellation] registry abort_attempt cancelling runner token trigger=registry_abort_attempt project_id={} issue_id={} attempt_id={} run_id={}",
                attempt_key.project_id, attempt_key.issue_id, attempt_key.attempt_id, run_id
            );
            #[cfg(test)]
            tracing::warn!(
                trigger = "registry_abort_attempt",
                project_id = %attempt_key.project_id,
                issue_id = %attempt_key.issue_id,
                attempt_id = %attempt_key.attempt_id,
                run_id = %run_id,
                "cancellation site: coding_run_registry::abort_attempt cancelling runner token"
            );
            cancellation.cancel();
            sent += 1;
        }
        for (run_id, sender, _, _) in &runners {
            match sender.try_send(CodingRunnerCommand::AbortAttempt) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    self.remove(attempt_key, *run_id);
                }
                Err(mpsc::error::TrySendError::Full(_)) => {}
            }
        }
        for (_, _, _, mut completion_rx) in runners {
            while !*completion_rx.borrow() {
                if completion_rx.changed().await.is_err() {
                    break;
                }
            }
        }
        sent
    }

    /// F-43：取该 attempt 当前 runner 的命令通道（交互应答经注册表投递的后备）。
    ///
    /// 页面刷新/新开 WS 连接时，新 socket 不持有 runner 句柄（`spawn_coding_runner`
    /// 只把 `command_tx` 留给启动它的连接），而 runner 仍在等 choice/permission
    /// 应答——作答必须回落到这里，否则被拒或被静默丢弃，「界面有卡、点了没反应」
    /// 仍是死锁（与 `abort_attempt` 同源的 attempt 级路由）。
    ///
    /// 同一 attempt 并存多条 run（测试 fixture 可做到）时无法判定应答归属，返回
    /// `None`，调用方保持既有行为。
    pub fn command_sender(
        &self,
        attempt_key: &CodingAttemptRunKey,
    ) -> Option<mpsc::Sender<CodingRunnerCommand>> {
        let inner = self.inner.lock().expect("coding run registry lock");
        let runs = inner.runs.get(attempt_key)?;
        match runs.len() {
            1 => runs.values().next().map(|entry| entry.command_tx.clone()),
            _ => None,
        }
    }

    pub fn runner_count(&self, attempt_key: &CodingAttemptRunKey) -> usize {
        self.inner
            .lock()
            .expect("coding run registry lock")
            .runs
            .get(attempt_key)
            .map(HashMap::len)
            .unwrap_or(0)
    }

    pub fn try_reserve_attempt(
        &self,
        attempt_key: &CodingAttemptRunKey,
    ) -> Option<CodingRunReservation> {
        let mut inner = self.inner.lock().expect("coding run registry lock");
        if inner.retired_attempts.contains(attempt_key)
            || inner
                .runs
                .get(attempt_key)
                .is_some_and(|runs| !runs.is_empty())
            || inner.reservations.contains_key(attempt_key)
        {
            return None;
        }
        inner.next_run_id += 1;
        let reservation_id = inner.next_run_id;
        inner
            .reservations
            .insert(attempt_key.clone(), reservation_id);
        Some(CodingRunReservation {
            registry: self.clone(),
            attempt_key: attempt_key.clone(),
            reservation_id,
            released: false,
        })
    }

    pub async fn lock_attempt(&self, attempt_key: &CodingAttemptRunKey) -> OwnedMutexGuard<()> {
        let guard = {
            let mut inner = self.inner.lock().expect("coding run registry lock");
            Arc::clone(
                inner
                    .attempt_guards
                    .entry(attempt_key.clone())
                    .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
            )
        };
        guard.lock_owned().await
    }

    pub async fn lock_named(&self, name: &str) -> OwnedMutexGuard<()> {
        let guard = {
            let mut inner = self.inner.lock().expect("coding run registry lock");
            Arc::clone(
                inner
                    .named_guards
                    .entry(name.to_string())
                    .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
            )
        };
        guard.lock_owned().await
    }

    pub(crate) async fn lock_attempt_mutation(
        &self,
        attempt_key: &CodingAttemptRunKey,
    ) -> CodingAttemptMutationLease {
        let guard = {
            let mut inner = self.inner.lock().expect("coding run registry lock");
            Arc::clone(
                inner
                    .attempt_mutation_guards
                    .entry(attempt_key.clone())
                    .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
            )
        };
        CodingAttemptMutationLease {
            _guard: guard.lock_owned().await,
        }
    }

    pub fn has_active_recovery_reservation(&self, attempt_key: &CodingAttemptRunKey) -> bool {
        self.inner
            .lock()
            .expect("coding run registry lock")
            .reservations
            .contains_key(attempt_key)
    }

    pub fn attempt_is_reserved_or_running(&self, attempt_key: &CodingAttemptRunKey) -> bool {
        let inner = self.inner.lock().expect("coding run registry lock");
        inner.reservations.contains_key(attempt_key)
            || inner
                .runs
                .get(attempt_key)
                .is_some_and(|runs| !runs.is_empty())
    }

    fn release_reservation(&self, attempt_key: &CodingAttemptRunKey, reservation_id: u64) {
        let mut inner = self.inner.lock().expect("coding run registry lock");
        if inner.reservations.get(attempt_key) == Some(&reservation_id) {
            inner.reservations.remove(attempt_key);
        }
    }

    // ------------------------------------------------------------------
    // P0 1.3（Task 9）：coding choice 应答 claim 门面——REST 与 WS 共用。
    // 短锁仲裁（不 .await）、锁外提交；Delivered 只由 provider 等待者推进。
    // ------------------------------------------------------------------

    /// 当前唯一活跃 run 的化身；无 run 或多 run 并存（测试 fixture 形态）时
    /// 返回 None——应答无法判定归属，调用方按 Expired/旧语义处理。
    pub fn active_run_incarnation(&self, attempt_key: &CodingAttemptRunKey) -> Option<String> {
        let inner = self.inner.lock().expect("coding run registry lock");
        let runs = inner.runs.get(attempt_key)?;
        match runs.len() {
            1 => runs
                .values()
                .next()
                .map(|entry| entry.run_incarnation.clone()),
            _ => None,
        }
    }

    /// 短临界区认领：唯一赢家；同 command 同 payload 幂等；异内容 Conflict；
    /// 无 run / 旧 run / 终态后旧 command 一律 Expired。
    pub fn claim_choice(
        &self,
        attempt_key: &CodingAttemptRunKey,
        choice_id: &str,
        request: &ChoiceResponseRequest,
    ) -> Result<(ChoiceReplyStatus, bool), ChoiceReplyError> {
        let mut inner = self.inner.lock().expect("coding run registry lock");
        let Some(incarnation) = inner
            .runs
            .get(attempt_key)
            .filter(|runs| runs.len() == 1)
            .and_then(|runs| runs.values().next())
            .map(|entry| entry.run_incarnation.clone())
        else {
            // 与 workspace 侧同语义：无活跃 run 时认领一律 Expired；
            // 终态登记只经 choice_status（GET）查询面可见。
            return Err(ChoiceReplyError::Expired);
        };
        if request.expected_run_id != incarnation {
            return Err(ChoiceReplyError::Expired);
        }
        // 已终态登记的同 command：幂等返回原状态或按旧 incarnation 拒绝。
        if let Some(finished) = inner
            .finished_choice_status
            .get(attempt_key)
            .and_then(|queue| {
                queue
                    .iter()
                    .find(|status| status.command_id == request.command_id
                        && status.choice_id == choice_id)
            })
        {
            if finished.expected_run_id == incarnation {
                return Ok((finished.clone(), false));
            }
            return Err(ChoiceReplyError::Expired);
        }
        let key = (attempt_key.clone(), choice_id.to_string());
        let fingerprint = coding_answers_fingerprint(&request.answers);
        if let Some(record) = inner.choice_claims.get(&key) {
            if record.command_id == request.command_id && record.fingerprint == fingerprint {
                return Ok((coding_claim_status(record, choice_id), false));
            }
            return Err(ChoiceReplyError::Conflict);
        }
        let (receipt, _observer) = ChoiceDeliverySignal::new();
        let record = CodingChoiceClaimRecord {
            command_id: request.command_id.clone(),
            fingerprint,
            status: ChoiceReplyState::Submitting,
            receipt,
            incarnation: incarnation.clone(),
        };
        let status = coding_claim_status(&record, choice_id);
        inner.choice_claims.insert(key, record);
        Ok((status, true))
    }

    /// 锁外向当前 run 提交已认领的应答；成功入队仅推进 Resolving。
    pub async fn submit_claimed_choice(
        &self,
        attempt_key: &CodingAttemptRunKey,
        choice_id: &str,
        request: &ChoiceResponseRequest,
    ) -> Result<ChoiceReplyStatus, ChoiceReplyError> {
        let (command_tx, receipt, incarnation) = {
            let inner = self.inner.lock().expect("coding run registry lock");
            let key = (attempt_key.clone(), choice_id.to_string());
            let Some(record) = inner.choice_claims.get(&key) else {
                return Err(ChoiceReplyError::Unknown);
            };
            if record.command_id != request.command_id {
                return Err(ChoiceReplyError::Conflict);
            }
            let Some(entry) = inner
                .runs
                .get(attempt_key)
                .filter(|runs| runs.len() == 1)
                .and_then(|runs| runs.values().next())
            else {
                return Err(ChoiceReplyError::Expired);
            };
            if entry.run_incarnation != record.incarnation {
                return Err(ChoiceReplyError::Expired);
            }
            (
                entry.command_tx.clone(),
                record.receipt.clone(),
                record.incarnation.clone(),
            )
        };
        let (selected_option_ids, free_text) =
            coding_legacy_fields_from_answers(&request.answers);
        let sent = command_tx
            .send(CodingRunnerCommand::ChoiceResponse {
                id: choice_id.to_string(),
                selected_option_ids,
                free_text,
                answers: request.answers.clone(),
                receipt: Some(receipt),
            })
            .await
            .is_ok();
        let mut inner = self.inner.lock().expect("coding run registry lock");
        let key = (attempt_key.clone(), choice_id.to_string());
        let Some(record) = inner.choice_claims.get_mut(&key) else {
            return Err(ChoiceReplyError::Unknown);
        };
        record.status = if sent {
            ChoiceReplyState::Resolving
        } else {
            ChoiceReplyState::Rejected
        };
        record.receipt.mark_resolving();
        if !sent {
            // 入队失败：结构拒绝，立即终态化并登记。
            let mut finished = inner.choice_claims.remove(&key).unwrap();
            finished.receipt.reject();
            let status = ChoiceReplyStatus {
                command_id: finished.command_id,
                expected_run_id: incarnation,
                choice_id: choice_id.to_string(),
                state: ChoiceReplyState::Rejected,
            };
            push_coding_finished_status(&mut inner, attempt_key, status.clone());
            return Ok(status);
        }
        Ok(coding_claim_status(record, choice_id))
    }

    /// 同 command 状态查询：优先以回执 watch 当前值为准（GET 面不需要等待
    /// 也能反映 provider 等待者已推进的真实状态）。
    pub fn choice_status(
        &self,
        attempt_key: &CodingAttemptRunKey,
        choice_id: &str,
        command_id: &str,
    ) -> Result<ChoiceReplyStatus, ChoiceReplyError> {
        let mut inner = self.inner.lock().expect("coding run registry lock");
        let key = (attempt_key.clone(), choice_id.to_string());
        if let Some(record) = inner.choice_claims.get_mut(&key) {
            if record.command_id == command_id {
                // 回执信号是唯一权威：watch 已终态则同步收敛投影并登记。
                let live = *record.receipt.subscribe().borrow();
                if !matches!(
                    live,
                    ChoiceReplyState::Submitting | ChoiceReplyState::Resolving
                ) && record.status != live
                {
                    record.status = live;
                }
                let status = coding_claim_status(record, choice_id);
                if matches!(
                    live,
                    ChoiceReplyState::Delivered
                        | ChoiceReplyState::Rejected
                        | ChoiceReplyState::Expired
                ) && inner.choice_claims.contains_key(&key)
                {
                    if let Some(mut removed) = inner.choice_claims.remove(&key) {
                        removed.status = live;
                        push_coding_finished_status(
                            &mut inner,
                            attempt_key,
                            ChoiceReplyStatus {
                                command_id: removed.command_id,
                                expected_run_id: removed.incarnation,
                                choice_id: choice_id.to_string(),
                                state: live,
                            },
                        );
                    }
                }
                return Ok(status);
            }
            return Err(ChoiceReplyError::Conflict);
        }
        inner
            .finished_choice_status
            .get(attempt_key)
            .and_then(|queue| {
                queue
                    .iter()
                    .find(|status| status.command_id == command_id && status.choice_id == choice_id)
            })
            .cloned()
            .map(Ok)
            .unwrap_or(Err(ChoiceReplyError::Unknown))
    }

    /// 等待回执终态；deadline 到期返回最新状态（不吞错误、不新发）。
    /// 终态临界区内直查终态登记，不嵌套调用 choice_status（T8 自死锁教训）。
    pub async fn wait_choice_receipt(
        &self,
        attempt_key: &CodingAttemptRunKey,
        choice_id: &str,
        command_id: &str,
        deadline: std::time::Duration,
    ) -> Result<ChoiceReplyStatus, ChoiceReplyError> {
        let observer = {
            let inner = self.inner.lock().expect("coding run registry lock");
            let key = (attempt_key.clone(), choice_id.to_string());
            inner
                .choice_claims
                .get(&key)
                .filter(|record| record.command_id == command_id)
                .map(|record| record.receipt.subscribe())
        };
        let Some(mut observer) = observer else {
            return self.choice_status(attempt_key, choice_id, command_id);
        };
        let final_state = match tokio::time::timeout(
            deadline,
            coding_wait_terminal_choice_state(&mut observer),
        )
        .await
        {
            Ok(state) => state,
            Err(_) => {
                return self.choice_status(attempt_key, choice_id, command_id);
            }
        };
        let mut inner = self.inner.lock().expect("coding run registry lock");
        let key = (attempt_key.clone(), choice_id.to_string());
        if let Some(record) = inner.choice_claims.get_mut(&key) {
            if record.command_id != command_id {
                return Err(ChoiceReplyError::Conflict);
            }
            record.status = final_state;
            let finished = ChoiceReplyStatus {
                command_id: record.command_id.clone(),
                expected_run_id: record.incarnation.clone(),
                choice_id: choice_id.to_string(),
                state: final_state,
            };
            if matches!(
                final_state,
                ChoiceReplyState::Delivered
                    | ChoiceReplyState::Rejected
                    | ChoiceReplyState::Expired
            ) {
                inner.choice_claims.remove(&key);
                push_coding_finished_status(&mut inner, attempt_key, finished.clone());
            }
            return Ok(finished);
        }
        // record 已被并发终态化（choice_status 收敛/retire）：在已持锁的
        // 临界区内直查终态登记，禁止嵌套加锁。
        inner
            .finished_choice_status
            .get(attempt_key)
            .and_then(|queue| {
                queue
                    .iter()
                    .find(|status| status.command_id == command_id && status.choice_id == choice_id)
            })
            .cloned()
            .map(Ok)
            .unwrap_or(Err(ChoiceReplyError::Unknown))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D①（红→绿）：abort_attempt 取消 runner token 时必须打点名取消者
    /// （attempt_key + run_id + 触发原因）——biased-select masking 下，这是唯一
    /// 不被握手错误文案掩盖的信号源。
    #[tokio::test]
    async fn abort_attempt_logs_cancellation_site_with_attempt_and_run_ids() {
        use crate::cross_cutting::tracing_capture::SharedTracingCapture;

        let registry = CodingRunRegistry::default();
        let attempt = CodingAttemptRunKey::new("project_diag", "issue_diag", "coding_attempt_diag");
        let (command_tx, command_rx) = mpsc::channel(1);
        let run_id = registry
            .insert_cancellable(&attempt, command_tx)
            .expect("diag runner")
            .run_id;
        drop(command_rx); // 接收端关闭：abort 无需等待移除即完成

        let capture = SharedTracingCapture::global();
        let sent = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            registry.abort_attempt(&attempt),
        )
        .await
        .expect("abort completes with closed receiver");

        assert_eq!(sent, 1);
        let logs = capture.captured();
        assert!(logs.contains("registry_abort_attempt"), "logs: {logs}");
        assert!(logs.contains("project_diag"), "logs: {logs}");
        assert!(logs.contains("issue_diag"), "logs: {logs}");
        assert!(logs.contains("coding_attempt_diag"), "logs: {logs}");
        assert!(logs.contains(&format!("run_id={run_id}")), "logs: {logs}");
    }

    #[tokio::test]
    async fn aborts_all_runs_for_attempt_and_removes_them() {
        let registry = CodingRunRegistry::default();
        let attempt = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001");
        let other = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0002");
        let (first_tx, mut first_rx) = mpsc::channel(1);
        let (second_tx, mut second_rx) = mpsc::channel(1);
        let (other_tx, mut other_rx) = mpsc::channel(1);

        let first_run_id = registry
            .insert_cancellable(&attempt, first_tx)
            .expect("first runner")
            .run_id;
        let second_run_id = registry
            .insert_cancellable(&attempt, second_tx)
            .expect("second runner")
            .run_id;
        registry
            .insert_cancellable(&other, other_tx)
            .expect("other runner");

        assert_eq!(registry.runner_count(&attempt), 2);
        let registry_for_abort = registry.clone();
        let attempt_for_abort = attempt.clone();
        let abort =
            tokio::spawn(async move { registry_for_abort.abort_attempt(&attempt_for_abort).await });
        assert_eq!(
            first_rx.recv().await.expect("first abort"),
            CodingRunnerCommand::AbortAttempt
        );
        assert_eq!(
            second_rx.recv().await.expect("second abort"),
            CodingRunnerCommand::AbortAttempt
        );
        tokio::task::yield_now().await;
        assert!(!abort.is_finished(), "abort must wait for runner removal");
        assert_eq!(registry.runner_count(&attempt), 2);
        registry.remove(&attempt, first_run_id);
        assert!(!abort.is_finished(), "abort must wait for every runner");
        registry.remove(&attempt, second_run_id);
        assert_eq!(abort.await.expect("abort task"), 2);
        assert_eq!(registry.runner_count(&attempt), 0);
        assert_eq!(registry.runner_count(&other), 1);
        assert!(other_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn abort_retires_attempt_and_revokes_pending_reservation() {
        let registry = CodingRunRegistry::default();
        let attempt = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001");
        let reservation = registry
            .try_reserve_attempt(&attempt)
            .expect("pending reservation");

        assert_eq!(registry.abort_attempt(&attempt).await, 0);
        let (late_tx, _late_rx) = mpsc::channel(1);
        assert!(reservation.activate_cancellable(late_tx.clone()).is_none());
        assert!(registry.insert_cancellable(&attempt, late_tx).is_none());
        assert!(registry.try_reserve_attempt(&attempt).is_none());
        assert!(!registry.attempt_is_reserved_or_running(&attempt));
    }

    #[tokio::test]
    async fn abort_completes_when_command_receiver_is_closed() {
        let registry = CodingRunRegistry::default();
        let attempt = CodingAttemptRunKey::new(
            "project_0001",
            "issue_0001",
            "coding_attempt_closed_receiver",
        );
        let (command_tx, command_rx) = mpsc::channel(1);
        registry
            .insert_cancellable(&attempt, command_tx)
            .expect("closed receiver runner");
        drop(command_rx);

        let sent = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            registry.abort_attempt(&attempt),
        )
        .await
        .expect("abort must not wait forever after command receiver closes");

        assert_eq!(sent, 1);
        assert_eq!(registry.runner_count(&attempt), 0);
    }

    #[tokio::test]
    async fn cancellable_abort_ignores_full_command_channel_and_waits_for_removal() {
        let registry = CodingRunRegistry::default();
        let attempt = CodingAttemptRunKey::new(
            "project_0001",
            "issue_0001",
            "coding_attempt_cancellable_backpressure",
        );
        let (command_tx, _command_rx) = mpsc::channel(1);
        command_tx
            .try_send(CodingRunnerCommand::AbortAttempt)
            .expect("fill command channel");
        let registration = registry
            .insert_cancellable(&attempt, command_tx)
            .expect("cancellable runner");
        let cancellation = registration.cancellation.clone();
        let run_id = registration.run_id;
        let registry_for_runner = registry.clone();
        let attempt_for_runner = attempt.clone();
        let runner = tokio::spawn(async move {
            cancellation.cancelled().await;
            registry_for_runner.remove(&attempt_for_runner, run_id);
        });

        let cancelled = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            registry.abort_attempt(&attempt),
        )
        .await
        .expect("abort must not wait for command channel capacity");

        runner.await.expect("runner cancellation observer");
        assert_eq!(cancelled, 1);
        assert_eq!(registry.runner_count(&attempt), 0);
    }

    #[test]
    fn scopes_reservations_by_attempt_identity() {
        let registry = CodingRunRegistry::default();
        let first = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001");
        let second = CodingAttemptRunKey::new("project_0001", "issue_0002", "coding_attempt_0001");

        let first_reservation = registry
            .try_reserve_attempt(&first)
            .expect("first reservation");
        let second_reservation = registry
            .try_reserve_attempt(&second)
            .expect("second scoped reservation");

        assert!(registry.has_active_recovery_reservation(&first));
        assert!(registry.has_active_recovery_reservation(&second));
        first_reservation.release();
        second_reservation.release();
    }

    #[tokio::test]
    async fn aborts_only_exact_attempt_identity() {
        let registry = CodingRunRegistry::default();
        let first = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001");
        let second = CodingAttemptRunKey::new("project_0001", "issue_0002", "coding_attempt_0001");
        let (first_tx, mut first_rx) = mpsc::channel(1);
        let (second_tx, mut second_rx) = mpsc::channel(1);
        registry
            .insert_cancellable(&first, first_tx)
            .expect("first runner");
        let second_run_id = registry
            .insert_cancellable(&second, second_tx)
            .expect("second runner")
            .run_id;

        let registry_for_abort = registry.clone();
        let second_for_abort = second.clone();
        let abort =
            tokio::spawn(async move { registry_for_abort.abort_attempt(&second_for_abort).await });
        assert_eq!(
            second_rx.recv().await.expect("second abort"),
            CodingRunnerCommand::AbortAttempt
        );
        tokio::task::yield_now().await;
        assert!(
            !abort.is_finished(),
            "abort must wait for exact runner removal"
        );
        assert_eq!(registry.runner_count(&first), 1);
        assert_eq!(registry.runner_count(&second), 1);
        assert!(first_rx.try_recv().is_err());
        registry.remove(&second, second_run_id);
        assert_eq!(abort.await.expect("abort task"), 1);
        assert_eq!(registry.runner_count(&second), 0);
    }

    // ------------------------------------------------------------------
    // P0 1.3（Task 9）：coding choice 应答 claim 门面——与 workspace 侧
    // （Task 7）同语义：短锁仲裁唯一赢家、同 command 幂等、异 payload 冲突、
    // run 终态后旧命令 Expired；Delivered 只能由 provider 等待者推进。
    // ------------------------------------------------------------------

    use crate::cross_cutting::choice_delivery::ChoiceReplyState;
    use crate::cross_cutting::streaming_provider::ChoiceAnswerData;
    use crate::web::choice_reply::ChoiceResponseRequest;
    use crate::web::workspace_session::ChoiceReplyError;

    fn choice_answers() -> Vec<ChoiceAnswerData> {
        vec![
            ChoiceAnswerData {
                question_id: "q-1".to_string(),
                selected_option_ids: vec!["yes".to_string()],
                free_text: None,
            },
            ChoiceAnswerData {
                question_id: "q-2".to_string(),
                selected_option_ids: vec!["no".to_string()],
                free_text: None,
            },
        ]
    }

    fn choice_request(command_id: &str, incarnation: &str) -> ChoiceResponseRequest {
        ChoiceResponseRequest {
            command_id: command_id.to_string(),
            expected_run_id: incarnation.to_string(),
            answers: choice_answers(),
        }
    }

    #[tokio::test]
    async fn coding_choice_reply_claim_arbitrates_and_delivers_exactly_once() {
        let registry = CodingRunRegistry::default();
        let key = CodingAttemptRunKey::new("project_1", "issue_1", "attempt_0001");
        let (command_tx, mut command_rx) = mpsc::channel(8);
        let registration = registry
            .insert_cancellable(&key, command_tx)
            .expect("register runner");
        let incarnation = registry
            .active_run_incarnation(&key)
            .expect("active run incarnation");

        let request = choice_request("cmd-one", &incarnation);
        let (first, won) = registry
            .claim_choice(&key, "ch-1", &request)
            .expect("first claim");
        assert!(won);
        assert_eq!(first.state, ChoiceReplyState::Submitting);
        assert_eq!(first.choice_id, "ch-1");

        // 同 command 同 payload：幂等返回、不再二发。
        let (retry, won_again) = registry
            .claim_choice(&key, "ch-1", &request)
            .expect("idempotent claim");
        assert!(!won_again);
        assert_eq!(retry.state, ChoiceReplyState::Submitting);

        // 同 command 异 payload → Conflict；另一 command → Conflict。
        let mut divergent = choice_request("cmd-one", &incarnation);
        divergent.answers[0].selected_option_ids = vec!["no".to_string()];
        assert_eq!(
            registry.claim_choice(&key, "ch-1", &divergent).unwrap_err(),
            ChoiceReplyError::Conflict
        );
        assert_eq!(
            registry
                .claim_choice(&key, "ch-1", &choice_request("cmd-two", &incarnation))
                .unwrap_err(),
            ChoiceReplyError::Conflict
        );

        // 提交：runner 收到恰一条命令，answers 原样 + receipt 在场。
        registry
            .submit_claimed_choice(&key, "ch-1", &request)
            .await
            .expect("submit");
        match command_rx.recv().await.expect("runner command") {
            CodingRunnerCommand::ChoiceResponse {
                id,
                answers,
                receipt: Some(receipt),
                ..
            } => {
                assert_eq!(id, "ch-1");
                assert_eq!(answers.len(), 2);
                assert_eq!(answers[1].question_id, "q-2");
                // mpsc 入队 ≠ Delivered：等待者解析前 wait 只见 Resolving。
                receipt.mark_resolving();
                let paused = registry
                    .wait_choice_receipt(
                        &key,
                        "ch-1",
                        "cmd-one",
                        std::time::Duration::from_millis(80),
                    )
                    .await
                    .expect("paused wait");
                assert_eq!(paused.state, ChoiceReplyState::Resolving);
                receipt.deliver();
            }
            other => panic!("expected ChoiceResponse command, got {other:?}"),
        }
        assert!(command_rx.try_recv().is_err(), "不得二发 runner command");

        // Delivered 后 wait 收敛；同 command 查询持久可见。
        let delivered = registry
            .wait_choice_receipt(
                &key,
                "ch-1",
                "cmd-one",
                std::time::Duration::from_millis(500),
            )
            .await
            .expect("delivered wait");
        assert_eq!(delivered.state, ChoiceReplyState::Delivered);
        let status = registry
            .choice_status(&key, "ch-1", "cmd-one")
            .expect("status query");
        assert_eq!(status.state, ChoiceReplyState::Delivered);

        registry.remove(&key, registration.run_id());
    }

    #[tokio::test]
    async fn coding_choice_reply_rejects_missing_or_retired_run() {
        let registry = CodingRunRegistry::default();
        let key = CodingAttemptRunKey::new("project_1", "issue_1", "attempt_0002");

        // 无 runner → Expired（410 语义）。
        assert_eq!(
            registry
                .claim_choice(&key, "ch-1", &choice_request("cmd-x", "any"))
                .unwrap_err(),
            ChoiceReplyError::Expired
        );

        let (command_tx, _command_rx) = mpsc::channel(8);
        let registration = registry
            .insert_cancellable(&key, command_tx)
            .expect("register runner");
        let incarnation = registry.active_run_incarnation(&key).unwrap();
        let request = choice_request("cmd-one", &incarnation);
        assert!(registry.claim_choice(&key, "ch-1", &request).unwrap().1);

        // 旧 expected_run_id（异 incarnation）→ Expired。
        assert_eq!(
            registry
                .claim_choice(&key, "ch-1", &choice_request("cmd-old", "stale-incarnation"))
                .unwrap_err(),
            ChoiceReplyError::Expired
        );

        // run 结束：旧 claim 置 Expired，同 command 查询见终态。
        registry.remove(&key, registration.run_id());
        assert_eq!(
            registry.claim_choice(&key, "ch-1", &request).unwrap_err(),
            ChoiceReplyError::Expired
        );
        assert_eq!(
            registry.choice_status(&key, "ch-1", "cmd-one").unwrap().state,
            ChoiceReplyState::Expired
        );

        // 未知 command → Unknown（404 语义）。
        assert_eq!(
            registry.choice_status(&key, "ch-1", "cmd-unknown").unwrap_err(),
            ChoiceReplyError::Unknown
        );
    }

    #[tokio::test]
    async fn coding_choice_reply_concurrent_claims_have_single_winner() {
        let registry = CodingRunRegistry::default();
        let key = CodingAttemptRunKey::new("project_1", "issue_1", "attempt_0003");
        let (command_tx, mut command_rx) = mpsc::channel(8);
        let registration = registry
            .insert_cancellable(&key, command_tx)
            .expect("register runner");
        let incarnation = registry.active_run_incarnation(&key).unwrap();

        // REST/WS 两路并发同 command+payload：唯一赢家，另一路幂等复看。
        let registry_b = registry.clone();
        let incarnation_b = incarnation.clone();
        let key_b = key.clone();
        let second = tokio::spawn(async move {
            let request = choice_request("cmd-race", &incarnation_b);
            registry_b
                .claim_choice(&key_b, "ch-race", &request)
                .expect("second claim")
        });
        let request = choice_request("cmd-race", &incarnation);
        let (first_status, first_won) = registry
            .claim_choice(&key, "ch-race", &request)
            .expect("first claim");
        let (second_status, second_won) = second.await.expect("join claim");
        assert!(
            first_won ^ second_won,
            "恰好一个赢家：{first_won}/{second_won}"
        );
        assert_eq!(first_status.command_id, second_status.command_id);

        // 唯一赢家提交后 runner 只收到一条命令。
        registry
            .submit_claimed_choice(&key, "ch-race", &request)
            .await
            .expect("submit");
        assert!(command_rx.recv().await.is_some());
        assert!(command_rx.try_recv().is_err(), "并发下不得二发");
        registry.remove(&key, registration.run_id());
    }
}
