//! F-24 挂起 provider choice 帧的登记、移除与补发辅助。

use std::collections::HashSet;

use super::{ActiveRun, WorkspaceSessionManager};
use crate::web::workspace_ws_types::WsOutMessage;

impl WorkspaceSessionManager {
    /// F-24：注册挂起 provider choice 帧（按 id 幂等）。无活跃 run 时丢弃——
    /// 该 choice 无应答通道，重投只会制造 stale 卡。
    pub fn register_pending_choice_frame(&self, frame: WsOutMessage) {
        let Some(run) = self.active_run_ref() else {
            return;
        };
        let mut pending = run
            .pending_choices
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !pending
            .iter()
            .any(|existing| choice_frame_id(existing) == choice_frame_id(&frame))
        {
            pending.push(frame);
        }
    }

    /// F-24：应答到达时移除挂起帧。返回 false 表示该 id 不在挂起集（stale 应答）。
    pub fn remove_pending_choice_frame(&self, id: &str) -> bool {
        let Some(run) = self.active_run_ref() else {
            return false;
        };
        let mut pending = run
            .pending_choices
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = pending.len();
        pending.retain(|frame| choice_frame_id(frame) != Some(id));
        before != pending.len()
    }

    /// F-24：当前活跃 run 的挂起 choice 帧（供 attach/重订阅/degraded 恢复补发）。
    pub(crate) fn pending_choice_frames(&self) -> Vec<WsOutMessage> {
        self.active_run_ref()
            .map(|run| {
                run.pending_choices
                    .lock()
                    .map(|pending| pending.clone())
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    }

    fn active_run_ref(&self) -> Option<ActiveRun> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_run
            .clone()
    }
}

/// F-24：从 wire 帧提取 choice id（仅 choice_request 帧有）。
pub(super) fn choice_frame_id(frame: &WsOutMessage) -> Option<&str> {
    match frame {
        WsOutMessage::ChoiceRequest { id, .. } => Some(id),
        _ => None,
    }
}

pub(super) fn choice_ids_in_frames(frames: &[String]) -> HashSet<String> {
    frames
        .iter()
        .filter_map(|json| {
            let value: serde_json::Value = serde_json::from_str(json).ok()?;
            let id = value.get("id")?.as_str()?;
            (value.get("type")?.as_str()? == "choice_request").then(|| id.to_string())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// P0 1.3（REQ-WIGA-05）：choice 应答 claim 门面——REST 与 WS 共用同一仲裁。
// 先在短临界区认领（不等待 engine mutex、不 .await），锁外向当前 run 的
// command_tx 提交；回执只在 provider 等待者真正接收（Delivered）后收敛。
// ---------------------------------------------------------------------------

use std::sync::Arc;
use std::time::Duration;

use super::ManagerState;
use crate::cross_cutting::choice_delivery::{ChoiceDeliverySignal, ChoiceReplyState};
use crate::cross_cutting::streaming_provider::{ChoiceAnswerData, ProviderCommand};
use crate::web::choice_reply::{ChoiceReplyStatus, ChoiceResponseRequest};

/// 有界终态登记上限（同 command 查询面；超出淘汰最旧）。
const FINISHED_CHOICE_STATUS_CAP: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoiceReplyError {
    /// 未知 choice/command（HTTP 404）。
    Unknown,
    /// 同 command 异 payload 或另一 claimant 占用（HTTP 409）。
    Conflict,
    /// 旧 run/无活跃 run（HTTP 410）。
    Expired,
}

pub(crate) struct ChoiceClaimRecord {
    pub command_id: String,
    /// 同一 choice/run 身份下的完整 answers 指纹——幂等重发判据。
    pub fingerprint: String,
    pub status: ChoiceReplyState,
    pub receipt: Option<ChoiceDeliverySignal>,
}

fn answers_fingerprint(answers: &[ChoiceAnswerData]) -> String {
    serde_json::to_string(answers).unwrap_or_default()
}

fn reply_status(
    record: &ChoiceClaimRecord,
    choice_id: &str,
    run_incarnation: &str,
) -> ChoiceReplyStatus {
    ChoiceReplyStatus {
        command_id: record.command_id.clone(),
        expected_run_id: run_incarnation.to_string(),
        choice_id: choice_id.to_string(),
        state: record.status,
    }
}

fn push_finished_status(state: &mut ManagerState, status: ChoiceReplyStatus) {
    state.finished_choice_status.push_back(status);
    while state.finished_choice_status.len() > FINISHED_CHOICE_STATUS_CAP {
        state.finished_choice_status.pop_front();
    }
}

/// run 终态（finish/abort/supersede）：旧 claim 一律 Expired，回执同步置位，
/// 状态保留有界查询面（同 command 可查，不可再送新 run）。
pub(super) fn retire_claims_for_run(state: &mut ManagerState, run_incarnation: &str) {
    let keys: Vec<(String, String)> = state
        .choice_claims
        .keys()
        .filter(|(incarnation, _)| incarnation == run_incarnation)
        .cloned()
        .collect();
    for key in keys {
        let choice_id = key.1.clone();
        if let Some(mut record) = state.choice_claims.remove(&key) {
            if let Some(receipt) = record.receipt.as_ref() {
                receipt.expire();
            }
            record.status = ChoiceReplyState::Expired;
            push_finished_status(
                state,
                ChoiceReplyStatus {
                    command_id: record.command_id.clone(),
                    expected_run_id: run_incarnation.to_string(),
                    choice_id,
                    state: ChoiceReplyState::Expired,
                },
            );
        }
    }
}

/// 从完整 answers 派生 legacy 单题字段（selected/free_text）——完整 answers
/// 才是权威载荷，legacy 字段仅供旧 provider 消费面。
fn legacy_fields_from_answers(answers: &[ChoiceAnswerData]) -> (Vec<String>, Option<String>) {
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

impl WorkspaceSessionManager {
    /// 当前活跃 run 化身（WS 缺省绑定与 pending 投影注入共用）。
    pub fn active_run_incarnation(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_run
            .as_ref()
            .map(|run| run.run_incarnation.clone())
    }

    /// WS 缺省绑定：不带 command_id/expected_run_id 的应答只绑当前唯一 run
    /// （无活跃 run → None，调用方走既有 TextFallback 分支）。
    pub fn bind_current_run_request(
        &self,
        choice_id: &str,
        answers: Vec<ChoiceAnswerData>,
    ) -> Option<ChoiceResponseRequest> {
        let incarnation = self.active_run_incarnation()?;
        Some(ChoiceResponseRequest {
            command_id: uuid::Uuid::new_v4().to_string(),
            expected_run_id: incarnation,
            answers,
        })
        .map(|request| {
            let _ = choice_id;
            request
        })
    }

    /// 短临界区认领：唯一赢家；同 command 同 payload 幂等返回原状态。
    /// 不等待 engine mutex、锁内不 .await。
    pub fn claim_choice(
        &self,
        choice_id: &str,
        request: &ChoiceResponseRequest,
    ) -> Result<(ChoiceReplyStatus, bool), ChoiceReplyError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(run) = state.active_run.clone() else {
            return Err(ChoiceReplyError::Expired);
        };
        if request.expected_run_id != run.run_incarnation {
            return Err(ChoiceReplyError::Expired);
        }
        let key = (run.run_incarnation.clone(), choice_id.to_string());
        let fingerprint = answers_fingerprint(&request.answers);
        // 已终态（Delivered/Expired）的同 command：幂等返回原状态或按旧 run
        // 拒绝——绝不重新认领、不二发。
        if let Some(finished) = state
            .finished_choice_status
            .iter()
            .find(|status| status.command_id == request.command_id && status.choice_id == choice_id)
        {
            if finished.expected_run_id == run.run_incarnation {
                return Ok((finished.clone(), false));
            }
            return Err(ChoiceReplyError::Expired);
        }
        if let Some(record) = state.choice_claims.get(&key) {
            if record.command_id == request.command_id && record.fingerprint == fingerprint {
                return Ok((reply_status(record, choice_id, &run.run_incarnation), false));
            }
            return Err(ChoiceReplyError::Conflict);
        }
        let (receipt, _observer) = ChoiceDeliverySignal::new();
        let record = ChoiceClaimRecord {
            command_id: request.command_id.clone(),
            fingerprint,
            status: ChoiceReplyState::Submitting,
            receipt: Some(receipt),
        };
        let status = reply_status(&record, choice_id, &run.run_incarnation);
        state.choice_claims.insert(key, record);
        Ok((status, true))
    }

    /// 锁外向当前 run 提交已认领的应答；成功入队仅推进 Resolving——
    /// Delivered 只能由 provider 等待者推进（两层回执）。
    pub async fn submit_claimed_choice(
        self: &Arc<Self>,
        choice_id: &str,
        request: &ChoiceResponseRequest,
    ) -> Result<ChoiceReplyStatus, ChoiceReplyError> {
        let (command_tx, receipt, incarnation) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(run) = state.active_run.clone() else {
                return Err(ChoiceReplyError::Expired);
            };
            if request.expected_run_id != run.run_incarnation {
                return Err(ChoiceReplyError::Expired);
            }
            let key = (run.run_incarnation.clone(), choice_id.to_string());
            let Some(record) = state.choice_claims.get(&key) else {
                return Err(ChoiceReplyError::Unknown);
            };
            if record.command_id != request.command_id {
                return Err(ChoiceReplyError::Conflict);
            }
            (
                run.command_tx.clone(),
                record.receipt.clone(),
                run.run_incarnation.clone(),
            )
        };
        let (selected_option_ids, free_text) = legacy_fields_from_answers(&request.answers);
        let sent = command_tx
            .send(ProviderCommand::ChoiceResponse {
                id: choice_id.to_string(),
                selected_option_ids,
                free_text,
                answers: request.answers.clone(),
                receipt: receipt.clone(),
            })
            .await
            .is_ok();
        let status = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let key = (incarnation.clone(), choice_id.to_string());
            if let Some(record) = state.choice_claims.get_mut(&key) {
                record.status = if sent {
                    ChoiceReplyState::Resolving
                } else {
                    ChoiceReplyState::Rejected
                };
                reply_status(record, choice_id, &incarnation)
            } else {
                ChoiceReplyStatus {
                    command_id: request.command_id.clone(),
                    expected_run_id: incarnation.clone(),
                    choice_id: choice_id.to_string(),
                    state: ChoiceReplyState::Rejected,
                }
            }
        };
        // Delivered 收敛看护：无论 REST/WS 谁提交，Delivered 后摘 pending 帧
        // 并广播收敛的 session_state；终态即固化状态。
        if receipt.is_some() {
            let manager = Arc::clone(self);
            let choice_id = choice_id.to_string();
            let incarnation = incarnation.clone();
            let command_id = request.command_id.clone();
            tokio::spawn(async move {
                let mut observer = manager
                    .choice_receipt_observer(&choice_id, &command_id)
                    .await;
                let Some(observer) = observer.as_mut() else {
                    return;
                };
                if tokio::time::timeout(
                    Duration::from_secs(30),
                    wait_for_terminal_choice_state(observer),
                )
                .await
                .is_ok()
                    && *observer.borrow() == ChoiceReplyState::Delivered
                {
                    manager.remove_pending_choice_frame(&choice_id);
                    manager.finalize_delivered_choice(&incarnation, &choice_id);
                    manager.broadcast_current_session_state();
                }
            });
        }
        Ok(status)
    }

    async fn choice_receipt_observer(
        self: &Arc<Self>,
        choice_id: &str,
        command_id: &str,
    ) -> Option<tokio::sync::watch::Receiver<ChoiceReplyState>> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state
            .choice_claims
            .values()
            .find(|record| {
                record.command_id == command_id
                    && state.choice_claims.keys().any(|(_, id)| id == choice_id)
            })
            .and_then(|record| record.receipt.as_ref())
            .map(|receipt| receipt.subscribe())
    }

    fn finalize_delivered_choice(&self, run_incarnation: &str, choice_id: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = (run_incarnation.to_string(), choice_id.to_string());
        if let Some(mut record) = state.choice_claims.remove(&key) {
            record.status = ChoiceReplyState::Delivered;
            push_finished_status(
                &mut state,
                ChoiceReplyStatus {
                    command_id: record.command_id,
                    expected_run_id: run_incarnation.to_string(),
                    choice_id: choice_id.to_string(),
                    state: ChoiceReplyState::Delivered,
                },
            );
        }
    }

    /// 同 command 状态查询（活跃 claim 或有界终态登记）。
    pub fn choice_status(
        &self,
        choice_id: &str,
        command_id: &str,
    ) -> Result<ChoiceReplyStatus, ChoiceReplyError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for ((incarnation, id), record) in state.choice_claims.iter() {
            if id == choice_id && record.command_id == command_id {
                return Ok(reply_status(record, choice_id, incarnation));
            }
        }
        state
            .finished_choice_status
            .iter()
            .find(|status| status.command_id == command_id && status.choice_id == choice_id)
            .cloned()
            .map(Ok)
            .unwrap_or(Err(ChoiceReplyError::Unknown))
    }

    /// 等待回执进入终态；deadline 到期返回当前最新状态（不吞错误、不新发）。
    pub async fn wait_choice_receipt(
        &self,
        choice_id: &str,
        command_id: &str,
        deadline: Duration,
    ) -> Result<ChoiceReplyStatus, ChoiceReplyError> {
        let observer = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state
                .choice_claims
                .iter()
                .find(|(_, record)| {
                    record.command_id == command_id
                        && state.choice_claims.keys().any(|(_, id)| id == choice_id)
                })
                .and_then(|((incarnation, _), record)| {
                    record
                        .receipt
                        .as_ref()
                        .map(|receipt| (incarnation.clone(), receipt.subscribe()))
                })
        };
        let Some((incarnation, mut observer)) = observer else {
            return self.choice_status(choice_id, command_id);
        };
        let final_state =
            match tokio::time::timeout(deadline, wait_for_terminal_choice_state(&mut observer))
                .await
            {
                Ok(state) => state,
                Err(_) => return self.choice_status(choice_id, command_id),
            };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = (incarnation.clone(), choice_id.to_string());
        if let Some(record) = state.choice_claims.get_mut(&key)
            && record.command_id == command_id
        {
            record.status = final_state;
            if matches!(
                final_state,
                ChoiceReplyState::Delivered
                    | ChoiceReplyState::Rejected
                    | ChoiceReplyState::Expired
            ) {
                let finished = ChoiceReplyStatus {
                    command_id: record.command_id.clone(),
                    expected_run_id: incarnation.clone(),
                    choice_id: choice_id.to_string(),
                    state: final_state,
                };
                state.choice_claims.remove(&key);
                push_finished_status(&mut state, finished.clone());
                return Ok(finished);
            }
            return Ok(reply_status(record, choice_id, &incarnation));
        }
        self.choice_status(choice_id, command_id)
    }
}

async fn wait_for_terminal_choice_state(
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
