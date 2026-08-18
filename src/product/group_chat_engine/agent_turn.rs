use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::{DEFAULT_PROVIDER_TIMEOUT_SECS, ProviderAdapterError};
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::models::ProviderName;
use crate::protocol::contracts::ProviderType;

use super::context::TurnContext;
use super::prompts::system_prompt_for;
use super::roles::adapter_role_for;
use super::types::{RoleInstance, RoomEvent};

/// 单个 agent turn 最多因 HOLD 重试的次数；初次执行不计入该上限。
pub const MAX_HOLD_RETRIES: u8 = 3;

/// agent turn 最终状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTurnFinalStatus {
    /// 本轮内容已通过全部门控，已交给调用方发布。
    Published,
    /// 连续 HOLD 已达到上限，已发布 `retry_exhausted` 事件。
    RetryExhausted,
}

/// 一次 agent turn 的纯核心结果。
///
/// 本模块只通过 `AgentTurnRuntime::publish_event` 把事件交给调用方，不负责写入任何
/// 持久化介质。`events` 保留本轮提交过的事件，方便 coordinator 做后续仲裁和测试断言。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnOutcome {
    pub status: AgentTurnFinalStatus,
    pub events: Vec<RoomEvent>,
    /// 实际调用 provider 的次数，包含初次执行和每次 HOLD 重试。
    pub provider_attempts: u8,
    /// 本轮自动拒绝的 provider 交互请求数量。
    ///
    /// 人工审批接线延后到后续 UI/迭代；当前群聊 turn 对 PermissionRequest 和
    /// ChoiceRequest 一律回复拒绝，避免 Supervised provider 因无响应而超时阻塞。
    pub denied_requests: u32,
}

/// freshness 门控的判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOrHold {
    /// 生成期间时间线没有新增事件，可以继续检查其他门控并发布。
    Publish,
    /// 生成所依据的快照已过期，必须使用新增事件重新生成。
    Held { new_events: usize },
}

/// HOLD 重试退避策略。
///
/// 默认采用设计约定的 1s、2s、4s。测试可用 `without_delay` 或 `with_delays` 覆盖，
/// 避免因为确定性重试而等待真实时间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldRetryPolicy {
    delays: [Duration; MAX_HOLD_RETRIES as usize],
}

impl Default for HoldRetryPolicy {
    fn default() -> Self {
        Self::with_delays([
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
        ])
    }
}

impl HoldRetryPolicy {
    /// 用调用方提供的退避时长创建策略。
    pub fn with_delays(delays: [Duration; MAX_HOLD_RETRIES as usize]) -> Self {
        Self { delays }
    }

    /// 测试专用的零等待策略。
    pub fn without_delay() -> Self {
        Self::with_delays([Duration::ZERO; MAX_HOLD_RETRIES as usize])
    }

    fn delay_for(&self, retry_index: u8) -> Duration {
        self.delays[retry_index as usize]
    }
}

/// 注入的时钟等待 future。
///
/// 该 future 可安全跨线程传递，生产环境使用 Tokio sleep，测试可注入零等待实现。
pub type SleepFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// 生产环境使用的 Tokio 时钟实现。
pub fn sleep_with_tokio(duration: Duration) -> SleepFuture {
    Box::pin(tokio::time::sleep(duration))
}

/// agent turn 依赖的易变房间状态、事件输出与时钟端口。
///
/// `read_events` 应在单写者保护下返回当前完整时间线快照；`publish_event` 将本轮输出
/// 交给调用方维护的内存时间线，以便下一次 freshness 检查可观察到 HOLD。**本模块不做
/// 文件、数据库或任何真实落盘**：B8 coordinator 可以将 `TurnOutcome.events` 和此回调
/// 接收的同一逻辑事件按自己的追加顺序持久化。
pub struct AgentTurnRuntime<'a> {
    /// 初次 provider 调用所依据的时间线长度快照。
    pub events_len_at_start: usize,
    /// 读取当前完整时间线，用于 freshness 与重复门控。
    pub read_events: &'a mut (dyn FnMut() -> Vec<RoomEvent> + Send),
    /// 交付可见事件给调用方；不表示本模块执行持久化。
    pub publish_event: &'a mut (dyn FnMut(RoomEvent) + Send),
    /// HOLD 后基于最新时间线重建上下文。通常委托给 `assemble_turn_context`。
    pub rebuild_context: &'a mut (dyn FnMut(&[RoomEvent], &mut RoleInstance) -> TurnContext + Send),
    /// 测试可注入零等待或短退避，生产调用使用默认策略。
    pub retry_policy: HoldRetryPolicy,
    /// 执行退避的可注入时钟。生产环境传入 `sleep_with_tokio`，测试可传入立即完成的 future。
    pub sleep: &'a mut (dyn FnMut(Duration) -> SleepFuture + Send),
}

/// 执行一个群聊角色的 provider turn，并在输出侧实施 freshness / verbatim-dup 门控。
///
/// provider 只有发出 `ProviderEvent::Completed` 才视为完成。每次 HOLD 都产生可观察的
/// `HeldEvent`，随后按 1s/2s/4s 重新读取时间线并生成；达到三次重试上限时，额外产生
/// `HeldEvent(reason=retry_exhausted)`。正常 AgentMessage 与 HeldEvent 都会推进角色的
/// `seen_cursor`；注入水位由 `assemble_turn_context` 根据连续完整注入结果推进，
/// 发布路径只更新 `seen_cursor`。
///
/// 人工审批接线延后到后续 UI/迭代。当前群聊 turn 收到 provider 的写类权限或选择请求
/// 时会自动发送拒绝命令，避免 Supervised provider 长时间等待，并在 `TurnOutcome` 记录数量。
pub async fn run_agent_turn(
    role: &mut RoleInstance,
    initial_context: TurnContext,
    adapter: &dyn StreamingProviderAdapter,
    runtime: AgentTurnRuntime<'_>,
) -> Result<TurnOutcome, AgentTurnError> {
    let AgentTurnRuntime {
        mut events_len_at_start,
        read_events,
        publish_event,
        rebuild_context,
        retry_policy,
        sleep,
    } = runtime;
    let mut context = initial_context;
    let mut retries = 0;
    let mut provider_attempts = 0;
    let mut emitted = Vec::new();
    let mut denied_requests = 0;

    loop {
        provider_attempts += 1;
        let provider_output =
            collect_completed_output(adapter, build_provider_input(role, &context, retries)?)
                .await?;
        denied_requests += provider_output.denied_requests;
        let text = provider_output.text;
        let current_events = read_events();
        let candidate = TurnOutcome {
            status: AgentTurnFinalStatus::Published,
            events: Vec::new(),
            provider_attempts,
            denied_requests,
        };

        let hold_reason = match publish_or_hold(events_len_at_start, &current_events, &candidate) {
            PublishOrHold::Held { .. } => Some("freshness"),
            PublishOrHold::Publish if is_verbatim_duplicate(&text, role, &current_events) => {
                Some("verbatim_duplicate")
            }
            PublishOrHold::Publish => None,
        };

        if let Some(reason) = hold_reason {
            let cursor_after = current_events.len() as u64;
            if retries == MAX_HOLD_RETRIES {
                // 达到上限时只保留一个明确的终态事件，避免同一轮既展示普通 HOLD 又展示
                // retry_exhausted，调用方可以据此等待下一次人类触发。
                let exhausted = held_event(role, "retry_exhausted", cursor_after);
                publish_event(exhausted.clone());
                emitted.push(exhausted);
                role.seen_cursor = cursor_after;
                return Ok(TurnOutcome {
                    status: AgentTurnFinalStatus::RetryExhausted,
                    events: emitted,
                    provider_attempts,
                    denied_requests,
                });
            }

            let held = held_event(role, reason, cursor_after);
            publish_event(held.clone());
            emitted.push(held);
            role.seen_cursor = cursor_after;

            sleep(retry_policy.delay_for(retries)).await;
            retries += 1;
            let latest_events = read_events();
            events_len_at_start = latest_events.len();
            context = rebuild_context(&latest_events, role);
            continue;
        }

        let cursor_after = events_len_at_start as u64;
        let message = RoomEvent::AgentMessage {
            role_instance_id: role.id.clone(),
            text,
            artifact_ref: None,
            cursor_after,
        };
        publish_event(message.clone());
        emitted.push(message);
        role.seen_cursor = cursor_after;
        return Ok(TurnOutcome {
            status: AgentTurnFinalStatus::Published,
            events: emitted,
            provider_attempts,
            denied_requests,
        });
    }
}

/// 检查从 provider 调用开始到准备发布期间，时间线是否已经改变。
///
/// `outcome` 保留在参数中以固定门控边界：调用方必须在拥有候选产出的同一临界区调用
/// 本函数，而不能把 freshness 检查提前到 provider 调用前。当前 freshness 本身只依赖
/// 时间线快照，verbatim-dup 门控由 `run_agent_turn` 在此检查通过后执行。
pub fn publish_or_hold(
    events_len_at_start: usize,
    current_events: &[RoomEvent],
    _outcome: &TurnOutcome,
) -> PublishOrHold {
    let new_events = current_events.len().saturating_sub(events_len_at_start);
    if new_events == 0 {
        PublishOrHold::Publish
    } else {
        PublishOrHold::Held { new_events }
    }
}

fn build_provider_input(
    role: &RoleInstance,
    context: &TurnContext,
    retry: u8,
) -> Result<StreamingProviderInput, AgentTurnError> {
    let working_dir = std::env::current_dir().map_err(AgentTurnError::WorkingDirectory)?;
    Ok(StreamingProviderInput {
        provider_type: provider_type_for(&role.provider),
        role: adapter_role_for(role.role_key),
        prompt: build_turn_prompt(role, context, retry),
        working_dir,
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode: role.permission_mode.clone(),
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
    })
}

fn build_turn_prompt(role: &RoleInstance, context: &TurnContext, retry: u8) -> String {
    let mut prompt = format!(
        "{}\n\n群聊 Agent Turn\n角色：{}（{}）\n群聊重试轮次：{retry}\n\n",
        system_prompt_for(role.role_key),
        role.display_name,
        role.id
    );
    if let Some(summary) = context.summary.as_deref() {
        prompt.push_str("滚动摘要：\n");
        prompt.push_str(summary);
        prompt.push_str("\n\n");
    }
    if !context.relevant_drafts.is_empty() {
        prompt.push_str("相关草稿：\n");
        prompt.push_str(&context.relevant_drafts.join("\n\n"));
        prompt.push_str("\n\n");
    }
    if !context.unread_events.is_empty() {
        prompt.push_str("未读时间线：\n");
        prompt.push_str(&context.unread_events.join("\n"));
        prompt.push_str("\n\n");
    }
    prompt.push_str(
        "请只基于以上已发布上下文给出本角色的下一条发言。不要复述最近一条他人消息；\n\
         若没有实质内容，请明确说明无新增意见。\n",
    );
    prompt
}

struct ProviderTurnOutput {
    text: String,
    denied_requests: u32,
}

async fn collect_completed_output(
    adapter: &dyn StreamingProviderAdapter,
    input: StreamingProviderInput,
) -> Result<ProviderTurnOutput, AgentTurnError> {
    let mut session = adapter.start(input, CancellationToken::new()).await?;
    let mut text_deltas = String::new();
    let mut denied_requests = 0;

    while let Some(event) = session.events.recv().await {
        match event {
            ProviderEvent::TextDelta { content } => text_deltas.push_str(&content),
            ProviderEvent::Completed(completion) => {
                let text = if completion.readable_output.is_empty() {
                    text_deltas
                } else {
                    completion.readable_output
                };
                if text.is_empty() {
                    return Err(AgentTurnError::EmptyCompletion);
                }
                return Ok(ProviderTurnOutput {
                    text,
                    denied_requests,
                });
            }
            ProviderEvent::PermissionRequest(request) => {
                deny_permission_request(&session.commands, request.id).await?;
                denied_requests += 1;
            }
            ProviderEvent::ChoiceRequest(request) => {
                deny_choice_request(&session.commands, request.id).await?;
                denied_requests += 1;
            }
            ProviderEvent::Failed { message } => {
                return Err(AgentTurnError::ProviderFailed(message));
            }
            ProviderEvent::ProtocolError { message, .. } => {
                return Err(AgentTurnError::ProviderProtocol(message));
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                return Err(AgentTurnError::PermissionTimeout(permission_id));
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
        }
    }

    Err(AgentTurnError::StreamClosedBeforeCompletion)
}

fn provider_type_for(provider: &ProviderName) -> ProviderType {
    match provider {
        ProviderName::ClaudeCode => ProviderType::ClaudeCode,
        ProviderName::Codex => ProviderType::Codex,
        ProviderName::Pi => ProviderType::Pi,
        ProviderName::KimiCode => ProviderType::KimiCode,
        ProviderName::Fake => ProviderType::Fake,
    }
}

fn is_verbatim_duplicate(text: &str, role: &RoleInstance, events: &[RoomEvent]) -> bool {
    let Some(latest_other_text) = events.iter().rev().find_map(|event| match event {
        RoomEvent::UserMessage { text, .. } => Some(text.as_str()),
        RoomEvent::AgentMessage {
            role_instance_id,
            text,
            ..
        } if role_instance_id != &role.id => Some(text.as_str()),
        _ => None,
    }) else {
        return false;
    };

    text.trim() == latest_other_text.trim()
}

const INTERACTIVE_REQUEST_DENY_REASON: &str = "群聊 Agent Turn 暂不支持人工审批，已自动拒绝";

async fn deny_permission_request(
    commands: &tokio::sync::mpsc::Sender<crate::cross_cutting::streaming_provider::ProviderCommand>,
    id: String,
) -> Result<(), AgentTurnError> {
    commands
        .send(
            crate::cross_cutting::streaming_provider::ProviderCommand::PermissionResponse {
                id,
                approved: false,
                reason: Some(INTERACTIVE_REQUEST_DENY_REASON.to_owned()),
            },
        )
        .await
        .map_err(|_| AgentTurnError::RequestDenyFailed)
}

async fn deny_choice_request(
    commands: &tokio::sync::mpsc::Sender<crate::cross_cutting::streaming_provider::ProviderCommand>,
    id: String,
) -> Result<(), AgentTurnError> {
    commands
        .send(
            crate::cross_cutting::streaming_provider::ProviderCommand::ChoiceResponse {
                id,
                selected_option_ids: Vec::new(),
                free_text: Some(INTERACTIVE_REQUEST_DENY_REASON.to_owned()),
                answers: Vec::new(),
            },
        )
        .await
        .map_err(|_| AgentTurnError::RequestDenyFailed)
}

fn held_event(role: &RoleInstance, reason: &str, cursor_after: u64) -> RoomEvent {
    RoomEvent::HeldEvent {
        role_instance_id: role.id.clone(),
        reason: reason.to_owned(),
        cursor_after,
    }
}

/// agent turn 的 provider / 流协议失败。
#[derive(Debug, thiserror::Error)]
pub enum AgentTurnError {
    #[error("无法获取当前工作目录：{0}")]
    WorkingDirectory(std::io::Error),
    #[error("无法启动 agent provider：{0}")]
    ProviderStart(#[from] ProviderAdapterError),
    #[error("agent provider 失败：{0}")]
    ProviderFailed(String),
    #[error("agent provider 协议错误：{0}")]
    ProviderProtocol(String),
    #[error("agent provider 权限请求超时：{0}")]
    PermissionTimeout(String),
    #[error("agent provider 交互请求自动拒绝发送失败")]
    RequestDenyFailed,
    #[error("agent provider 在完成前关闭事件流")]
    StreamClosedBeforeCompletion,
    #[error("agent provider 完成事件不含可发布文本")]
    EmptyCompletion,
}
