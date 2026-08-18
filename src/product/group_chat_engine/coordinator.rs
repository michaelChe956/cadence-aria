use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;

use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::Semaphore;

use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;
use crate::product::group_chat_store::GroupChatStore;
use crate::product::json_store::ProductStoreError;
use crate::product::models::ProviderName;

use super::agent_turn::{
    AgentTurnError, AgentTurnRuntime, HoldRetryPolicy, SleepFuture, run_agent_turn,
    sleep_with_tokio,
};
use super::claims::{ClaimError, DEFAULT_CLAIM_TIMEOUT, release, release_expired, try_claim};
use super::context::{
    INJECTION_BUDGET_TOKENS, assemble_turn_context_with_summary, maybe_update_rolling_summary,
    render_event,
};
use super::roles::writable_slots;
use super::triage::{NoOneCounter, RoomStateView, TriageInput, TriageOutput, TriageRouter};
use super::types::{
    ArtifactDraft, ArtifactRef, DraftSlotKey, GroupChatSessionRecord, RoleInstance, RoomEvent,
};

/// 两次人类消息之间允许的默认 agent 活动（含 HOLD）上限。
pub const HARD_LOOP_CAP: usize = 12;
/// 默认连续空转窗口。
pub const STALL_WINDOW: usize = 4;

/// provider 名称到流式 adapter 的注册表。
pub type ProviderAdapterRegistry = HashMap<ProviderName, Arc<dyn StreamingProviderAdapter>>;

/// Coordinator 的可注入运行参数。
///
/// 生产默认值遵循设计节奏；测试可以把时间相关参数设为 `Duration::ZERO`，并使用
/// `HoldRetryPolicy::without_delay()`，因此不会发生真实等待。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordinatorConfig {
    pub hard_loop_cap: usize,
    pub stall_window: usize,
    pub max_parallel_per_provider: usize,
    pub spawn_interval: Duration,
    pub rate_limit_backoff: Duration,
    pub hold_retry_policy: HoldRetryPolicy,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            hard_loop_cap: HARD_LOOP_CAP,
            stall_window: STALL_WINDOW,
            max_parallel_per_provider: 2,
            spawn_interval: Duration::from_millis(500),
            rate_limit_backoff: Duration::from_secs(60),
            hold_retry_policy: HoldRetryPolicy::default(),
        }
    }
}

/// 一次人类消息触发的可观测结果。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct CoordinatorRunSummary {
    /// 本轮经 `GroupChatStore::append_event` 写入的所有时间线序号。
    pub appended_seqs: Vec<u64>,
    /// 本轮实际落盘的 HeldEvent 数量。
    pub held_events: usize,
    /// 是否因硬上限或空转检测暂停讨论。
    pub circuit_break: bool,
    /// 是否因连续两次 NoOne 决策写入自然结束提示。
    pub no_one_notice: bool,
}

/// Coordinator 可返回的持久化错误。
#[derive(Debug, thiserror::Error)]
pub enum CoordinatorError {
    #[error("群聊存储失败：{0}")]
    Store(#[from] ProductStoreError),
}

/// 单个 provider 的并发、启动节奏和静默退避状态。
struct ProviderThrottle {
    semaphore: Arc<Semaphore>,
    schedule: Mutex<ProviderSchedule>,
}

#[derive(Clone, Copy)]
struct ProviderSchedule {
    next_spawn_at: Instant,
    muted_until: Instant,
}

impl ProviderThrottle {
    fn new(max_parallel: usize) -> Self {
        let now = Instant::now();
        Self {
            semaphore: Arc::new(Semaphore::new(max_parallel.max(1))),
            schedule: Mutex::new(ProviderSchedule {
                next_spawn_at: now,
                muted_until: now,
            }),
        }
    }

    async fn acquire(&self, spawn_interval: Duration) -> tokio::sync::OwnedSemaphorePermit {
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("群聊 provider 信号量不会关闭");
        let start_at = {
            let mut schedule = self.schedule.lock().expect("provider 节奏锁可用");
            let now = Instant::now();
            let start_at = schedule.next_spawn_at.max(schedule.muted_until).max(now);
            schedule.next_spawn_at = start_at + spawn_interval;
            start_at
        };
        let initial_delay = start_at.saturating_duration_since(Instant::now());
        if !initial_delay.is_zero() {
            tokio::time::sleep(initial_delay).await;
        }

        // 有其他并行 turn 在预定启动前收到 rate-limit 时，它会更新 muted_until。
        // 因而启动前必须再次检查，确保同 provider 的全部角色都进入静默窗口。
        loop {
            let muted_until = self
                .schedule
                .lock()
                .expect("provider 节奏锁可用")
                .muted_until;
            let remaining = muted_until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return permit;
            }
            tokio::time::sleep(remaining).await;
        }
    }

    fn back_off(&self, duration: Duration) {
        let mut schedule = self.schedule.lock().expect("provider 节奏锁可用");
        schedule.muted_until = schedule.muted_until.max(Instant::now() + duration);
    }
}

/// 群聊单写者编排器。
///
/// 所有 timeline 落盘都由本类型通过 `GroupChatStore::append_event` 发起；agent turn
/// 只在共享内存快照中发布候选事件，以便并行 turn 的 freshness 门控相互可见。
pub struct Coordinator {
    store: GroupChatStore,
    adapters: ProviderAdapterRegistry,
    router: Box<dyn TriageRouter>,
    config: CoordinatorConfig,
    throttles: HashMap<ProviderName, Arc<ProviderThrottle>>,
}

impl Coordinator {
    pub fn new(
        store: GroupChatStore,
        adapters: HashMap<ProviderName, Arc<dyn StreamingProviderAdapter>>,
        router: Box<dyn TriageRouter>,
    ) -> Self {
        let config = CoordinatorConfig::default();
        let throttles = adapters
            .keys()
            .cloned()
            .map(|provider| {
                (
                    provider,
                    Arc::new(ProviderThrottle::new(config.max_parallel_per_provider)),
                )
            })
            .collect();
        Self {
            store,
            adapters,
            router,
            config,
            throttles,
        }
    }

    /// 用可测试配置替换默认运行参数。
    pub fn with_config(mut self, config: CoordinatorConfig) -> Self {
        self.throttles = self
            .adapters
            .keys()
            .cloned()
            .map(|provider| {
                (
                    provider,
                    Arc::new(ProviderThrottle::new(config.max_parallel_per_provider)),
                )
            })
            .collect();
        self.config = config;
        self
    }

    /// 追加人类消息，并在同一调用中完成「triage → agent turn → 落盘 → triage」闭环。
    ///
    /// 初始 `mentions` 是强制路由：有效角色不会经过 triage。之后 agent 发言仍须通过
    /// triage，直到自然终止、熔断或发言预算耗尽。
    pub async fn on_user_message(
        &mut self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        text: &str,
        mentions: Vec<String>,
    ) -> Result<CoordinatorRunSummary, CoordinatorError> {
        self.on_user_message_inner(project_id, issue_id, session_id, text, mentions, None)
            .await
    }

    async fn on_user_message_inner(
        &mut self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        text: &str,
        mentions: Vec<String>,
        draft_target: Option<DraftTarget>,
    ) -> Result<CoordinatorRunSummary, CoordinatorError> {
        let mut session = self.store.load_session(project_id, issue_id, session_id)?;
        let mut events = self.read_events(project_id, issue_id, session_id)?;
        let mut summary = CoordinatorRunSummary::default();
        let user_event = RoomEvent::UserMessage {
            text: text.to_string(),
            mentions: mentions.clone(),
        };
        let user_seq = self.append(
            project_id,
            issue_id,
            session_id,
            &mut events,
            &mut summary,
            user_event,
        )?;
        // 摘要维护位于上下文组装之前；本地 v1 实现仅用于接线验证，真实小模型后续接入。
        self.refresh_rolling_summary(&mut session, &events);
        self.store.save_session_snapshot(&session)?;

        let mut triggers = VecDeque::from([Trigger {
            seq: user_seq,
            text: text.to_string(),
            last_speaker: None,
            forced_mentions: mentions,
        }]);
        let mut no_one = NoOneCounter::default();
        let mut spoken_for_trigger = HashSet::new();
        let mut draft_assigned = false;

        while let Some(trigger) = triggers.pop_front() {
            if self.agent_activity_count(&events) >= self.config.hard_loop_cap {
                self.append_circuit_notice(
                    project_id,
                    issue_id,
                    session_id,
                    &mut events,
                    &mut summary,
                )?;
                break;
            }
            if Self::stall_circuit_breaks(&events, self.config.stall_window) {
                self.append_circuit_notice(
                    project_id,
                    issue_id,
                    session_id,
                    &mut events,
                    &mut summary,
                )?;
                break;
            }

            let selected = self.select_roles(&session, &trigger, &spoken_for_trigger);
            if selected.is_empty() {
                if no_one.observe(&TriageOutput::NoOneNeedsToRespond).is_none() {
                    // NoOne 的第二次确认没有新消息，因而显式复检同一个稳定快照。
                    let second = self.route(&session, &trigger);
                    if let Some(notice) = no_one.observe(&second) {
                        self.append(
                            project_id,
                            issue_id,
                            session_id,
                            &mut events,
                            &mut summary,
                            notice,
                        )?;
                        summary.no_one_notice = true;
                    }
                }
                continue;
            }
            no_one.on_new_trigger();

            let selected_ids = selected
                .iter()
                .map(|role| role.id.clone())
                .collect::<Vec<_>>();
            for role_id in selected_ids {
                spoken_for_trigger.insert((trigger.seq, role_id));
            }

            let mut scheduled = self.run_turns(&session, &events, selected).await;
            if let Some(target) = draft_target.as_ref()
                && !draft_assigned
            {
                draft_assigned = decorate_draft_event(&mut scheduled.events, target);
            }
            let events_before_turn = events.len();
            let provider_error_roles = scheduled
                .completed
                .iter()
                .filter(|completed| !completed.rate_limited && completed.error.is_some())
                .map(|completed| completed.role.id.clone())
                .collect::<Vec<_>>();
            let rate_limited_role_ids = scheduled
                .completed
                .iter()
                .filter(|completed| completed.rate_limited)
                .map(|completed| completed.role.id.clone())
                .collect::<HashSet<_>>();
            for completed in &scheduled.completed {
                if completed.rate_limited
                    && let Some(throttle) = self.throttles.get(&completed.provider)
                {
                    throttle.back_off(self.config.rate_limit_backoff);
                }
                // 速率限制是 provider 级静默退避，不写入聊天室。
            }

            // 先按 agent turn 产生的时间顺序写入 HOLD/消息，再为失败角色记录终态，
            // 避免 provider_error 早于同一 turn 已产生的 freshness HOLD。
            let published = self.persist_new_turn_events(
                project_id,
                issue_id,
                session_id,
                &mut events,
                &mut summary,
                scheduled
                    .events
                    .into_iter()
                    .filter(|event| !belongs_to_role(event, &rate_limited_role_ids))
                    .collect(),
            )?;
            for role_instance_id in provider_error_roles {
                let held = RoomEvent::HeldEvent {
                    role_instance_id,
                    reason: "provider_error".into(),
                    cursor_after: events.len() as u64,
                };
                self.append_with_cap(
                    project_id,
                    issue_id,
                    session_id,
                    &mut events,
                    &mut summary,
                    held,
                )?;
            }

            // 若并行批次刚好触碰硬上限，未落盘的候选绝不能污染 session 快照。
            // 只把确实写入时间线的角色结果纳入快照，再以权威 cursor 对齐。
            self.apply_persisted_role_state(
                &mut session,
                &scheduled.completed,
                &events[events_before_turn..],
            );
            // 上下文组装后再次维护，确保本轮新增事件达到窗口阈值时立即持久化。
            self.refresh_rolling_summary(&mut session, &events);
            self.store.save_session_snapshot(&session)?;

            if summary.circuit_break
                || self.agent_activity_count(&events) >= self.config.hard_loop_cap
                || Self::stall_circuit_breaks(&events, self.config.stall_window)
            {
                self.append_circuit_notice(
                    project_id,
                    issue_id,
                    session_id,
                    &mut events,
                    &mut summary,
                )?;
                break;
            }

            for (seq, event) in published {
                if let RoomEvent::AgentMessage {
                    role_instance_id,
                    text,
                    ..
                } = event
                {
                    triggers.push_back(Trigger {
                        seq,
                        text,
                        last_speaker: Some(role_instance_id),
                        forced_mentions: Vec::new(),
                    });
                }
            }
        }

        self.store.save_session_snapshot(&session)?;
        Ok(summary)
    }

    /// C1 显式起草路径：先认领指定草稿槽，再复用正常消息闭环，最后将本轮第一条
    /// 可写角色发言落入草稿槽。消息不带槽位时保持 Coordinator 原有纯聊天语义。
    pub async fn on_user_message_with_draft(
        &mut self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        text: &str,
        mentions: Vec<String>,
        draft_slot: Option<DraftSlotKey>,
    ) -> Result<CoordinatorRunSummary, CoordinatorError> {
        let Some(draft_slot) = draft_slot else {
            return self
                .on_user_message(project_id, issue_id, session_id, text, mentions)
                .await;
        };
        let mut session = self.store.load_session(project_id, issue_id, session_id)?;
        self.release_expired_claims(project_id, issue_id, session_id, &mut session)?;
        if !session
            .artifact_lines
            .iter()
            .flat_map(|line| line.drafts.iter())
            .any(|slot| slot.slot_key == draft_slot)
        {
            return Err(CoordinatorError::Store(ProductStoreError::InvalidRecord {
                kind: "group_chat_draft_slot",
                reason: format!("draft slot not found: {}", draft_slot.0),
            }));
        }
        let role = if mentions.is_empty() {
            session
                .roles
                .iter()
                .find(|role| writable_slots(role.role_key).contains(&draft_slot))
                .cloned()
        } else {
            mentions
                .iter()
                .filter_map(|id| session.roles.iter().find(|role| role.id == *id))
                .find(|role| writable_slots(role.role_key).contains(&draft_slot))
                .cloned()
        };
        let Some(role) = role else {
            return self
                .on_user_message(project_id, issue_id, session_id, text, mentions)
                .await;
        };
        let line = session
            .artifact_lines
            .iter_mut()
            .find(|line| line.drafts.iter().any(|slot| slot.slot_key == draft_slot))
            .ok_or_else(|| ProductStoreError::InvalidRecord {
                kind: "group_chat_draft_slot",
                reason: format!("draft slot not found: {}", draft_slot.0),
            })?;
        let claim_event = match try_claim(line, &draft_slot, &role, Utc::now()) {
            Ok(event) => event,
            Err(ClaimError::SlotAlreadyClaimed { .. }) => {
                let mut summary = self
                    .on_user_message(project_id, issue_id, session_id, text, mentions)
                    .await?;
                let seq = self.store.append_event(
                    project_id,
                    issue_id,
                    session_id,
                    RoomEvent::HeldEvent {
                        role_instance_id: role.id,
                        reason: "draft_slot_claimed".to_owned(),
                        cursor_after: self
                            .store
                            .load_events(project_id, issue_id, session_id)?
                            .len() as u64,
                    },
                )?;
                summary.appended_seqs.push(seq);
                summary.held_events += 1;
                return Ok(summary);
            }
            Err(error) => {
                return Err(CoordinatorError::Store(ProductStoreError::InvalidRecord {
                    kind: "group_chat_draft_slot",
                    reason: error.to_string(),
                }));
            }
        };
        let line_kind = line.kind;
        let draft_version = line
            .drafts
            .iter()
            .find(|slot| slot.slot_key == draft_slot)
            .and_then(|slot| slot.current.as_ref())
            .map_or(1, |draft| draft.version + 1);
        let claim_seq = self
            .store
            .append_event(project_id, issue_id, session_id, claim_event)?;
        self.store.save_session_snapshot(&session)?;

        let result = self
            .on_user_message_inner(
                project_id,
                issue_id,
                session_id,
                text,
                vec![role.id.clone()],
                Some(DraftTarget {
                    role_instance_id: role.id.clone(),
                    line: line_kind,
                    slot: draft_slot.clone(),
                    version: draft_version,
                }),
            )
            .await;
        let Ok(mut summary) = result else {
            if let Err(error) = self.release_draft_claim(
                project_id,
                issue_id,
                session_id,
                &role,
                line_kind,
                &draft_slot,
            ) {
                return Err(CoordinatorError::Store(error));
            }
            return result;
        };
        summary.appended_seqs.insert(0, claim_seq);
        let entries = self
            .store
            .load_event_entries(project_id, issue_id, session_id)?;
        let Some((based_on_events, markdown)) =
            entries.iter().rev().find_map(|(seq, event)| match event {
                RoomEvent::AgentMessage {
                    role_instance_id,
                    text,
                    artifact_ref: Some(artifact_ref),
                    ..
                } if role_instance_id == &role.id
                    && artifact_ref.line == line_kind
                    && artifact_ref.slot == draft_slot
                    && artifact_ref.version == draft_version =>
                {
                    Some((*seq, text.clone()))
                }
                _ => None,
            })
        else {
            if let Some(release_seq) = self.release_draft_claim(
                project_id,
                issue_id,
                session_id,
                &role,
                line_kind,
                &draft_slot,
            )? {
                summary.appended_seqs.push(release_seq);
            }
            return Ok(summary);
        };
        let mut session = self.store.load_session(project_id, issue_id, session_id)?;
        let (line_kind, updated) = {
            let line = session
                .artifact_lines
                .iter_mut()
                .find(|line| line.drafts.iter().any(|slot| slot.slot_key == draft_slot))
                .expect("认领成功后草稿槽仍存在");
            if let Some(slot) = line
                .drafts
                .iter_mut()
                .find(|slot| slot.slot_key == draft_slot)
            {
                slot.current = Some(ArtifactDraft {
                    version: draft_version,
                    markdown,
                    author_role_id: role.id.clone(),
                    based_on_events,
                });
                slot.claim = None;
                (line.kind, true)
            } else {
                (line.kind, false)
            }
        };
        if updated {
            session.updated_at = Utc::now().to_rfc3339();
            self.store.save_session_snapshot(&session)?;
            let release_seq = self.store.append_event(
                project_id,
                issue_id,
                session_id,
                RoomEvent::ClaimEvent {
                    role_instance_id: role.id,
                    line: line_kind,
                    slot_key: draft_slot,
                    claimed: false,
                },
            )?;
            summary.appended_seqs.push(release_seq);
        }
        Ok(summary)
    }

    fn release_expired_claims(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        session: &mut GroupChatSessionRecord,
    ) -> Result<(), ProductStoreError> {
        let events = release_expired(
            &mut session.artifact_lines,
            Utc::now(),
            DEFAULT_CLAIM_TIMEOUT,
        );
        if events.is_empty() {
            return Ok(());
        }
        for event in events {
            self.store
                .append_event(project_id, issue_id, session_id, event)?;
        }
        session.updated_at = Utc::now().to_rfc3339();
        self.store.save_session_snapshot(session)
    }

    fn release_draft_claim(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        role: &RoleInstance,
        line_kind: super::types::ArtifactLineKind,
        slot_key: &DraftSlotKey,
    ) -> Result<Option<u64>, ProductStoreError> {
        let mut session = self.store.load_session(project_id, issue_id, session_id)?;
        let Some(line) = session
            .artifact_lines
            .iter_mut()
            .find(|line| line.kind == line_kind)
        else {
            return Ok(None);
        };
        let Some(slot) = line.drafts.iter().find(|slot| {
            slot.slot_key == *slot_key
                && slot
                    .claim
                    .as_ref()
                    .is_some_and(|claim| claim.holder_role_id == role.id)
        }) else {
            return Ok(None);
        };
        let slot_key = slot.slot_key.clone();
        let event =
            release(line, &slot_key, role).map_err(|error| ProductStoreError::InvalidRecord {
                kind: "group_chat_draft_slot",
                reason: error.to_string(),
            })?;
        let seq = self
            .store
            .append_event(project_id, issue_id, session_id, event)?;
        session.updated_at = Utc::now().to_rfc3339();
        self.store.save_session_snapshot(&session)?;
        Ok(Some(seq))
    }

    fn route(&self, session: &GroupChatSessionRecord, trigger: &Trigger) -> TriageOutput {
        self.router.route(&TriageInput {
            triggering_seq: trigger.seq,
            last_speaker: trigger.last_speaker.clone(),
            room_state: RoomStateView {
                roles: session.roles.clone(),
                triggering_text: trigger.text.clone(),
            },
            lines: session.artifact_lines.clone(),
        })
    }

    fn select_roles(
        &self,
        session: &GroupChatSessionRecord,
        trigger: &Trigger,
        spoken_for_trigger: &HashSet<(u64, String)>,
    ) -> Vec<RoleInstance> {
        let forced_mentions = !trigger.forced_mentions.is_empty();
        let candidate_ids = if forced_mentions {
            trigger.forced_mentions.clone()
        } else {
            match self.route(session, trigger) {
                TriageOutput::RespondTo(ids) => ids,
                TriageOutput::NoOneNeedsToRespond => Vec::new(),
            }
        };

        let mut unique_ids = HashSet::new();
        let selected = candidate_ids
            .into_iter()
            .filter(|id| unique_ids.insert(id.clone()))
            .filter(|id| !spoken_for_trigger.contains(&(trigger.seq, id.clone())))
            .filter(|id| trigger.last_speaker.as_deref() != Some(id.as_str()))
            .filter_map(|id| session.roles.iter().find(|role| role.id == id).cloned())
            .collect::<Vec<_>>();
        if forced_mentions {
            selected
        } else {
            selected.into_iter().take(2).collect()
        }
    }

    async fn run_turns(
        &self,
        session: &GroupChatSessionRecord,
        events: &[RoomEvent],
        roles: Vec<RoleInstance>,
    ) -> TurnBatch {
        let shared_events = Arc::new(Mutex::new(events.to_vec()));
        let lines = session.artifact_lines.clone();
        let mut turns = FuturesUnordered::new();

        for mut role in roles {
            let adapter = self.adapters.get(&role.provider).cloned();
            let throttle = self
                .throttles
                .get(&role.provider)
                .cloned()
                .unwrap_or_else(|| {
                    Arc::new(ProviderThrottle::new(self.config.max_parallel_per_provider))
                });
            let config = self.config.clone();
            let turn_events = shared_events.clone();
            let lines = lines.clone();
            let provider = role.provider.clone();
            let initial_context = {
                let snapshot = turn_events.lock().expect("群聊事件锁可用").clone();
                assemble_turn_context_with_summary(
                    &snapshot,
                    &mut role,
                    &lines,
                    INJECTION_BUDGET_TOKENS,
                    session.rolling_summary.as_deref(),
                )
            };
            let events_len_at_start = turn_events.lock().expect("群聊事件锁可用").len();

            turns.push(async move {
                let Some(adapter) = adapter else {
                    return CompletedTurn::provider_error(
                        role,
                        AgentTurnError::ProviderFailed("未配置该角色的 provider adapter".into()),
                    );
                };
                let _permit = throttle.acquire(config.spawn_interval).await;
                let mut read_events = {
                    let turn_events = turn_events.clone();
                    move || turn_events.lock().expect("群聊事件锁可用").clone()
                };
                let mut publish_event = {
                    let turn_events = turn_events.clone();
                    move |event| turn_events.lock().expect("群聊事件锁可用").push(event)
                };
                let mut rebuild_context = {
                    let turn_events = turn_events.clone();
                    let lines = lines.clone();
                    let rolling_summary = session.rolling_summary.clone();
                    move |_: &[RoomEvent], role: &mut RoleInstance| {
                        let snapshot = turn_events.lock().expect("群聊事件锁可用").clone();
                        assemble_turn_context_with_summary(
                            &snapshot,
                            role,
                            &lines,
                            INJECTION_BUDGET_TOKENS,
                            rolling_summary.as_deref(),
                        )
                    }
                };
                let mut sleep = sleep_with_tokio as fn(Duration) -> SleepFuture;
                match run_agent_turn(
                    &mut role,
                    initial_context,
                    adapter.as_ref(),
                    AgentTurnRuntime {
                        events_len_at_start,
                        read_events: &mut read_events,
                        publish_event: &mut publish_event,
                        rebuild_context: &mut rebuild_context,
                        retry_policy: config.hold_retry_policy,
                        sleep: &mut sleep,
                    },
                )
                .await
                {
                    Ok(_) => CompletedTurn::success(role, provider),
                    Err(error) => {
                        let rate_limited = is_rate_limited(&error);
                        if rate_limited {
                            // 在本次 turn 结束的瞬间更新 provider 静默窗口；同一批中
                            // 尚未拿到信号量的其他角色也会在 acquire 时看到该退避。
                            throttle.back_off(config.rate_limit_backoff);
                        }
                        CompletedTurn {
                            role,
                            provider,
                            error: Some(error),
                            rate_limited,
                        }
                    }
                }
            });
        }

        let mut completed = Vec::new();
        while let Some(turn) = turns.next().await {
            completed.push(turn);
        }
        let turn_events = shared_events
            .lock()
            .expect("群聊事件锁可用")
            .iter()
            .skip(events.len())
            .cloned()
            .collect();
        TurnBatch {
            completed,
            events: turn_events,
        }
    }

    fn refresh_rolling_summary(&self, session: &mut GroupChatSessionRecord, events: &[RoomEvent]) {
        let Some(summary) = maybe_update_rolling_summary(
            events,
            session.rolling_summary.as_deref(),
            &mut |window, existing| {
                // v1 本地 summarizer：每个事件取前 200 字，后续接入真实小模型。
                let current = window
                    .iter()
                    .map(render_event)
                    .map(|text| text.chars().take(200).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                match existing.filter(|summary| !summary.is_empty()) {
                    // 未满下一个完整窗口时 maybe_update 会再次给出同一窗口，保持摘要幂等。
                    Some(previous) if previous.ends_with(&current) => previous.to_owned(),
                    Some(previous) => format!("{previous}\n{current}"),
                    None => current,
                }
            },
        ) else {
            return;
        };
        if !summary.is_empty() {
            session.rolling_summary = Some(summary);
            session.updated_at = Utc::now().to_rfc3339();
        }
    }

    fn apply_persisted_role_state(
        &self,
        session: &mut GroupChatSessionRecord,
        completed: &[CompletedTurn],
        persisted_events: &[RoomEvent],
    ) {
        for completed_turn in completed {
            if completed_turn.rate_limited {
                continue;
            }
            let role_id = &completed_turn.role.id;
            let wrote_event = persisted_events.iter().any(|event| match event {
                RoomEvent::AgentMessage {
                    role_instance_id, ..
                }
                | RoomEvent::HeldEvent {
                    role_instance_id, ..
                } => role_instance_id == role_id,
                _ => false,
            });
            if wrote_event
                && let Some(stored_role) = session.roles.iter_mut().find(|role| role.id == *role_id)
            {
                *stored_role = completed_turn.role.clone();
            }
        }
    }

    fn persist_new_turn_events(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        events: &mut Vec<RoomEvent>,
        summary: &mut CoordinatorRunSummary,
        new_events: Vec<RoomEvent>,
    ) -> Result<Vec<(u64, RoomEvent)>, CoordinatorError> {
        let mut published = Vec::new();
        for event in new_events {
            if self.agent_activity_count(events) >= self.config.hard_loop_cap
                && counts_toward_agent_activity(&event)
            {
                summary.circuit_break = true;
                break;
            }
            let seq = self.append(
                project_id,
                issue_id,
                session_id,
                events,
                summary,
                event.clone(),
            )?;
            if matches!(event, RoomEvent::AgentMessage { .. }) {
                published.push((seq, event));
            }
        }
        Ok(published)
    }

    fn append_with_cap(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        events: &mut Vec<RoomEvent>,
        summary: &mut CoordinatorRunSummary,
        event: RoomEvent,
    ) -> Result<(), CoordinatorError> {
        if self.agent_activity_count(events) >= self.config.hard_loop_cap
            && counts_toward_agent_activity(&event)
        {
            summary.circuit_break = true;
            return Ok(());
        }
        self.append(project_id, issue_id, session_id, events, summary, event)?;
        Ok(())
    }

    fn append(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        events: &mut Vec<RoomEvent>,
        summary: &mut CoordinatorRunSummary,
        event: RoomEvent,
    ) -> Result<u64, CoordinatorError> {
        let seq = self
            .store
            .append_event(project_id, issue_id, session_id, event.clone())?;
        if matches!(event, RoomEvent::HeldEvent { .. }) {
            summary.held_events += 1;
        }
        events.push(event);
        summary.appended_seqs.push(seq);
        Ok(seq)
    }

    fn append_circuit_notice(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        events: &mut Vec<RoomEvent>,
        summary: &mut CoordinatorRunSummary,
    ) -> Result<(), CoordinatorError> {
        if !summary.circuit_break {
            summary.circuit_break = true;
        }
        if !events.iter().any(|event| {
            matches!(event, RoomEvent::SystemNotice { text } if text == "讨论已暂停，等待你的输入")
        }) {
            self.append(
                project_id,
                issue_id,
                session_id,
                events,
                summary,
                RoomEvent::SystemNotice {
                    text: "讨论已暂停，等待你的输入".into(),
                },
            )?;
        }
        Ok(())
    }

    fn read_events(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
    ) -> Result<Vec<RoomEvent>, CoordinatorError> {
        Ok(self.store.load_events(project_id, issue_id, session_id)?)
    }

    fn agent_activity_count(&self, events: &[RoomEvent]) -> usize {
        events
            .iter()
            .rev()
            .take_while(|event| !matches!(event, RoomEvent::UserMessage { .. }))
            .filter(|event| counts_toward_agent_activity(event))
            .count()
    }

    /// 判定最后 `window` 条 agent 消息是否都没有引入新参与者且没有草稿版本变化。
    pub fn stall_circuit_breaks(events: &[RoomEvent], window: usize) -> bool {
        if window == 0 {
            return false;
        }
        let events_after_last_user = events
            .iter()
            .rposition(|event| matches!(event, RoomEvent::UserMessage { .. }))
            .map_or(events, |index| &events[index + 1..]);
        let agent_messages = events_after_last_user
            .iter()
            .filter_map(|event| match event {
                RoomEvent::AgentMessage {
                    role_instance_id,
                    artifact_ref,
                    ..
                } => Some((role_instance_id, artifact_ref)),
                _ => None,
            })
            .collect::<Vec<_>>();
        if agent_messages.len() < window {
            return false;
        }
        let split = agent_messages.len() - window;
        let mut known_roles = agent_messages[..split]
            .iter()
            .map(|(role_id, _)| (*role_id).clone())
            .collect::<HashSet<_>>();
        let mut known_draft_versions = agent_messages[..split]
            .iter()
            .filter_map(|(_, artifact_ref)| artifact_ref.as_ref())
            .map(draft_version_key)
            .collect::<HashSet<_>>();
        agent_messages[split..]
            .iter()
            .all(|(role_id, artifact_ref)| {
                let participant_is_new = !known_roles.insert((*role_id).clone());
                let draft_changed = artifact_ref.as_ref().is_some_and(|artifact_ref| {
                    known_draft_versions.insert(draft_version_key(artifact_ref))
                });
                participant_is_new && !draft_changed
            })
    }
}

#[derive(Clone)]
struct Trigger {
    seq: u64,
    text: String,
    last_speaker: Option<String>,
    forced_mentions: Vec<String>,
}

struct DraftTarget {
    role_instance_id: String,
    line: super::types::ArtifactLineKind,
    slot: DraftSlotKey,
    version: u32,
}

struct TurnBatch {
    completed: Vec<CompletedTurn>,
    events: Vec<RoomEvent>,
}

struct CompletedTurn {
    role: RoleInstance,
    provider: ProviderName,
    error: Option<AgentTurnError>,
    rate_limited: bool,
}

impl CompletedTurn {
    fn success(role: RoleInstance, provider: ProviderName) -> Self {
        Self {
            role,
            provider,
            error: None,
            rate_limited: false,
        }
    }

    fn provider_error(role: RoleInstance, error: AgentTurnError) -> Self {
        let provider = role.provider.clone();
        Self {
            role,
            provider,
            error: Some(error),
            rate_limited: false,
        }
    }
}

fn decorate_draft_event(events: &mut [RoomEvent], target: &DraftTarget) -> bool {
    let Some(RoomEvent::AgentMessage { artifact_ref, .. }) = events.iter_mut().find(|event| {
        matches!(event, RoomEvent::AgentMessage { role_instance_id, .. } if role_instance_id == &target.role_instance_id)
    }) else {
        return false;
    };
    *artifact_ref = Some(ArtifactRef {
        line: target.line,
        slot: target.slot.clone(),
        version: target.version,
    });
    true
}

fn belongs_to_role(event: &RoomEvent, role_ids: &HashSet<String>) -> bool {
    match event {
        RoomEvent::AgentMessage {
            role_instance_id, ..
        }
        | RoomEvent::HeldEvent {
            role_instance_id, ..
        } => role_ids.contains(role_instance_id),
        _ => false,
    }
}

fn draft_version_key(artifact_ref: &ArtifactRef) -> String {
    format!(
        "{:?}:{}:{}",
        artifact_ref.line, artifact_ref.slot.0, artifact_ref.version
    )
}

fn counts_toward_agent_activity(event: &RoomEvent) -> bool {
    // HARD_LOOP_CAP 限制的是 agent 发言数；HOLD 后重新生成的最终 AgentMessage
    // 同样计入，HeldEvent 本身只是透明的门控记录。
    matches!(event, RoomEvent::AgentMessage { .. })
}

fn is_rate_limited(error: &AgentTurnError) -> bool {
    let text = match error {
        AgentTurnError::ProviderStart(error) => {
            format!("{} {} {}", error.details, error.stdout, error.stderr)
        }
        AgentTurnError::ProviderFailed(message) | AgentTurnError::ProviderProtocol(message) => {
            message.clone()
        }
        _ => return false,
    };
    let text = text.to_lowercase();
    text.contains("rate limit") || text.contains("rate_limit") || text.contains("too many requests")
}
