use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};

use crate::product::group_chat_engine::finalize::FinalizeInput;
use crate::product::group_chat_engine::types::{GroupChatRoleKey, RoomEvent};
use crate::web::group_chat_ws_types::{GroupChatWsInMessage, GroupChatWsOutMessage};
use crate::web::handlers::group_chat::collect_refs_for_ws;
use crate::web::state::WebAppState;

/// 群聊独立 WS endpoint。after_seq 通过 query 参数传入，连接后立即重放更早的事件。
#[derive(Debug, serde::Deserialize, Default)]
pub struct GroupChatWsQuery {
    after_seq: Option<u64>,
}

pub async fn group_chat_ws(
    ws: WebSocketUpgrade,
    Path(session_id): Path<String>,
    Query(query): Query<GroupChatWsQuery>,
    State(state): State<WebAppState>,
) -> Response {
    let after_seq = query.after_seq.unwrap_or(0);
    ws.on_upgrade(move |socket| handle_socket(socket, session_id, state, after_seq))
        .into_response()
}

async fn handle_socket(socket: WebSocket, session_id: String, state: WebAppState, after_seq: u64) {
    let Some(engine) = state.group_chat_engine.clone() else {
        send_error_and_close(socket, "group_chat_unavailable", "群聊引擎不可用").await;
        return;
    };
    let Some(session) = (match engine.store.find_session_by_id(&session_id) {
        Ok(session) => session,
        Err(error) => {
            send_error_and_close(socket, "group_chat_session_error", &error.to_string()).await;
            return;
        }
    }) else {
        send_error_and_close(socket, "group_chat_session_not_found", "群聊会话不存在").await;
        return;
    };

    let (mut sender, mut receiver) = socket.split();
    // 先注册再读取和重放，避免重放期间遗漏新落盘事件；序列号去重保证顺序稳定。
    let mut live = state.group_chat_sockets.subscribe(session_id.clone()).await;
    let entries =
        match engine
            .store
            .load_event_entries(&session.project_id, &session.issue_id, &session.id)
        {
            Ok(entries) => entries,
            Err(error) => {
                send_error(&mut sender, "group_chat_timeline_error", &error.to_string()).await;
                return;
            }
        };
    let mut sent_seqs = HashSet::new();
    for (seq, event) in entries.into_iter().filter(|(seq, _)| *seq > after_seq) {
        sent_seqs.insert(seq);
        if !send_message(
            &mut sender,
            &GroupChatWsOutMessage::RoomEvent { seq, event },
        )
        .await
        {
            return;
        }
    }

    loop {
        tokio::select! {
            incoming = receiver.next() => {
                let Some(Ok(message)) = incoming else { return; };
                let Message::Text(text) = message else {
                    if matches!(message, Message::Close(_)) { return; }
                    continue;
                };
                let command: GroupChatWsInMessage = match serde_json::from_str(&text) {
                    Ok(command) => command,
                    Err(error) => {
                        if !send_error(&mut sender, "invalid_message", &error.to_string()).await { return; }
                        continue;
                    }
                };
                let current_session = match engine.store.load_session(
                    &session.project_id,
                    &session.issue_id,
                    &session_id,
                ) {
                    Ok(session) => session,
                    Err(error) => {
                        if !send_error(&mut sender, "group_chat_session_error", &error.to_string()).await {
                            return;
                        }
                        continue;
                    }
                };
                if !handle_command(
                    &mut sender,
                    &state,
                    &engine,
                    &current_session,
                    &session_id,
                    command,
                ).await { return; }
            }
            outbound = live.recv() => {
                let Some(outbound) = outbound else { return; };
                if let GroupChatWsOutMessage::RoomEvent { seq, .. } = &outbound
                    && !sent_seqs.insert(*seq)
                {
                    continue;
                }
                if !send_message(&mut sender, &outbound).await { return; }
            }
        }
    }
}

async fn handle_command<S>(
    sender: &mut S,
    state: &WebAppState,
    engine: &Arc<crate::product::group_chat_engine::GroupChatEngine>,
    session: &crate::product::group_chat_engine::types::GroupChatSessionRecord,
    session_id: &str,
    command: GroupChatWsInMessage,
) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
{
    match command {
        GroupChatWsInMessage::Ping => send_message(sender, &GroupChatWsOutMessage::Pong).await,
        GroupChatWsInMessage::SendMessage {
            text,
            mentions,
            draft_slot,
        } => {
            if text.trim().is_empty() {
                return send_error(sender, "invalid_group_chat_message", "text 不能为空").await;
            }
            let selected_roles = turn_role_ids(session, &text, &mentions);
            for role_instance_id in selected_roles {
                if !send_message(
                    sender,
                    &GroupChatWsOutMessage::TurnStarted { role_instance_id },
                )
                .await
                {
                    return false;
                }
            }
            // Coordinator 当前消费 ProviderEvent 流但只返回已落盘事件；v1 暂不转发增量。
            // TODO: 将 ProviderEvent 流接入 TurnDelta，同时保持 RoomEvent 的持久化顺序。
            match engine
                .on_user_message(
                    &session.project_id,
                    &session.issue_id,
                    session_id,
                    &text,
                    mentions,
                    draft_slot,
                )
                .await
            {
                Ok(summary) => publish_summary(state, engine, session, session_id, summary).await,
                Err(error) => {
                    send_error(sender, "group_chat_message_failed", &error.to_string()).await
                }
            }
        }
        GroupChatWsInMessage::AddRole {
            role_key,
            provider,
            display_name,
            permission_mode,
        } => match engine.add_role(
            &session.project_id,
            &session.issue_id,
            session_id,
            role_key,
            provider,
            display_name,
            permission_mode,
        ) {
            Ok(_) => true,
            Err(error) => send_error(sender, "group_chat_role_failed", &error.to_string()).await,
        },
        GroupChatWsInMessage::Finalize {
            line_kind,
            included_slots,
            confirmed_by,
        } => {
            let Ok(entries) =
                engine
                    .store
                    .load_event_entries(&session.project_id, &session.issue_id, session_id)
            else {
                return send_error(sender, "group_chat_timeline_error", "读取时间线失败").await;
            };
            let (provider_run_refs, review_refs) =
                collect_refs_for_ws(&entries, session, line_kind);
            match engine.finalize_line(FinalizeInput {
                project_id: session.project_id.clone(),
                issue_id: session.issue_id.clone(),
                session_id: session_id.to_owned(),
                line_kind,
                included_slots_override: included_slots,
                confirmed_by,
                provider_run_refs,
                review_refs,
            }) {
                Ok(event) => {
                    match engine.store.load_event_entries(
                        &session.project_id,
                        &session.issue_id,
                        session_id,
                    ) {
                        Ok(entries) => {
                            if let Some((seq, _)) = entries
                                .iter()
                                .rev()
                                .find(|(_, candidate)| *candidate == event)
                            {
                                publish(
                                    state,
                                    session_id,
                                    GroupChatWsOutMessage::RoomEvent { seq: *seq, event },
                                )
                                .await;
                                true
                            } else {
                                send_error(
                                    sender,
                                    "group_chat_timeline_error",
                                    "定稿事件未写入时间线",
                                )
                                .await
                            }
                        }
                        Err(error) => {
                            send_error(sender, "group_chat_timeline_error", &error.to_string())
                                .await
                        }
                    }
                }
                Err(error) => {
                    send_error(sender, "group_chat_finalize_failed", &error.to_string()).await
                }
            }
        }
    }
}

/// 计算 TurnStarted 的提示角色；这是瞬态提示帧，可能与 Coordinator 实际 triage 选角不一致。
fn turn_role_ids(
    session: &crate::product::group_chat_engine::types::GroupChatSessionRecord,
    text: &str,
    mentions: &[String],
) -> Vec<String> {
    if !mentions.is_empty() {
        return mentions
            .iter()
            .filter(|id| session.roles.iter().any(|role| role.id == **id))
            .cloned()
            .collect();
    }
    let text = text.to_lowercase();
    let role_key = if text.contains("frontend") || text.contains("前端") {
        GroupChatRoleKey::FrontendDesign
    } else if text.contains("backend") || text.contains("后端") {
        GroupChatRoleKey::BackendDesign
    } else if text.contains("design") || text.contains("设计") {
        GroupChatRoleKey::FrontendDesign
    } else {
        GroupChatRoleKey::Author
    };
    session
        .roles
        .iter()
        .filter(|role| role.role_key == role_key)
        .map(|role| role.id.clone())
        .collect()
}

async fn publish_summary(
    state: &WebAppState,
    engine: &Arc<crate::product::group_chat_engine::GroupChatEngine>,
    session: &crate::product::group_chat_engine::types::GroupChatSessionRecord,
    session_id: &str,
    summary: crate::product::group_chat_engine::coordinator::CoordinatorRunSummary,
) -> bool {
    let entries =
        match engine
            .store
            .load_event_entries(&session.project_id, &session.issue_id, session_id)
        {
            Ok(entries) => entries,
            Err(_) => return false,
        };
    for seq in summary.appended_seqs {
        let Some((_, event)) = entries.iter().find(|(entry_seq, _)| *entry_seq == seq) else {
            continue;
        };
        let outbound = GroupChatWsOutMessage::RoomEvent {
            seq,
            event: event.clone(),
        };
        publish(state, session_id, outbound).await;
        if let RoomEvent::HeldEvent {
            role_instance_id,
            reason,
            ..
        } = event
        {
            publish(
                state,
                session_id,
                GroupChatWsOutMessage::TurnHeld {
                    role_instance_id: role_instance_id.clone(),
                    reason: reason.clone(),
                },
            )
            .await;
        }
    }
    true
}

async fn publish(state: &WebAppState, session_id: &str, message: GroupChatWsOutMessage) {
    state.group_chat_sockets.publish(session_id, message).await;
}

async fn send_message<S>(sender: &mut S, message: &GroupChatWsOutMessage) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
{
    let Ok(json) = serde_json::to_string(message) else {
        return false;
    };
    sender.send(Message::Text(json.into())).await.is_ok()
}

async fn send_error<S>(sender: &mut S, code: &str, message: &str) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
{
    send_message(
        sender,
        &GroupChatWsOutMessage::Error {
            code: code.to_owned(),
            message: message.to_owned(),
        },
    )
    .await
}

async fn send_error_and_close(socket: WebSocket, code: &str, message: &str) {
    let (mut sender, _) = socket.split();
    let _ = send_error(&mut sender, code, message).await;
}
