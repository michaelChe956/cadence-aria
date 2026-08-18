use super::roles::writable_slots;
use super::types::{
    ArtifactLine, ArtifactLineKind, DraftSlotKey, GroupChatRoleKey, RoleInstance, RoomEvent,
};

/// 单轮上下文默认的 token 预算。
pub const INJECTION_BUDGET_TOKENS: usize = 16_000;
const SUMMARY_INTERVAL_EVENTS: usize = 20;

/// 注入上下文的事件元数据。
///
/// `seq` 是该事件在 `assemble_turn_context` 输入序列中的显式 1-based 序号，供调用方
/// 记录和诊断注入水位；不需要再从 `TurnContext` 中的字符串位置反推事件来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEvent {
    pub seq: u64,
    pub event: RoomEvent,
    /// 文本是否完整注入；为 false 时该事件不会推进 injection_watermark。
    pub fully_injected: bool,
}

/// 注入上下文的草稿元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelevantDraft {
    pub line: ArtifactLineKind,
    pub slot: DraftSlotKey,
    pub version: u32,
}

/// 传给 agent turn 的窗口化上下文。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TurnContext {
    /// 未读事件的已渲染文本。agent 发言会被标记为不可信上下文。
    pub unread_events: Vec<String>,
    /// `unread_events` 一一对应的事件来源元数据。
    pub unread_event_metadata: Vec<ContextEvent>,
    /// 与当前角色职责相关的滚动摘要；没有摘要时为空。
    pub summary: Option<String>,
    /// 当前角色需要关注的草稿文本。
    pub relevant_drafts: Vec<String>,
    /// `relevant_drafts` 一一对应的草稿来源元数据。
    pub relevant_draft_metadata: Vec<RelevantDraft>,
}

/// 按事件数量的固定窗口触发滚动摘要。
///
/// 调用方应把尚未被上下文窗口消费的事件传入 `events`，并负责持久化返回的摘要。
/// 这样，被预算截断的事件不会因为本次组装而从下一次摘要输入中消失。
pub fn maybe_update_rolling_summary(
    events: &[RoomEvent],
    existing: Option<&str>,
    summarizer: &mut dyn FnMut(&[RoomEvent], Option<&str>) -> String,
) -> Option<String> {
    if events.len() < SUMMARY_INTERVAL_EVENTS {
        return None;
    }

    // 每次压缩最近一个完整窗口；不满 20 条的尾部留给下一次触发。
    // existing 承载更早窗口的滚动结果。
    let complete_events = events.len() / SUMMARY_INTERVAL_EVENTS * SUMMARY_INTERVAL_EVENTS;
    let start = complete_events - SUMMARY_INTERVAL_EVENTS;
    Some(summarizer(&events[start..complete_events], existing))
}

/// 组装角色本轮可见的四层上下文。
///
/// 预算按「人类消息 > 相关草稿 > 未读事件 > 摘要」消耗。每个已渲染事件和草稿
/// 都通过 `TurnContext` 的元数据列表公开其来源，调用方不必从字符串位置推断来源。
pub fn assemble_turn_context(
    events: &[RoomEvent],
    role: &mut RoleInstance,
    lines: &[ArtifactLine],
    budget_tokens: usize,
) -> TurnContext {
    let mut remaining = budget_tokens;
    let mut selected = vec![false; events.len()];
    let mut unread_events = Vec::new();

    // injection_watermark 是实际进入 prompt 的连续时间线位置；水位之后的事件
    // 即使 seen_cursor 已经推进，也必须保留，直到它们实际进入 prompt 或摘要。
    let unread_start = role.injection_watermark as usize;
    let unread: Vec<(usize, &RoomEvent)> = events
        .iter()
        .enumerate()
        .filter(|(index, _)| *index >= unread_start)
        .collect();

    // 第一层：人类消息优先于其他所有未读事件。
    for (index, event) in unread.iter().copied().filter(|(_, event)| is_human(event)) {
        if let Some(selection) = take_text(render_event(event), &mut remaining, false) {
            let fully_injected = selection.is_complete();
            if fully_injected {
                selected[index] = true;
            }
            unread_events.push((
                index,
                selection.into_text(),
                ContextEvent {
                    seq: index as u64 + 1,
                    event: event.clone(),
                    fully_injected,
                },
            ));
        }
    }

    // 第二层：草稿是独立于时间线的交付物，优先于普通未读 agent 发言。
    let relevant_drafts = relevant_draft_entries(role.role_key, lines)
        .into_iter()
        .filter_map(|(draft, metadata)| {
            take_text(draft, &mut remaining, false)
                .map(TextSelection::into_text)
                .map(|rendered| (rendered, metadata))
        })
        .collect::<Vec<_>>();

    // 第三层：普通未读事件。保持时间线顺序，防止同一轮上下文中的讨论倒序。
    for (index, event) in unread.iter().copied().filter(|(_, event)| !is_human(event)) {
        if let Some(selection) = take_text(render_event(event), &mut remaining, is_agent(event)) {
            let fully_injected = selection.is_complete();
            if fully_injected {
                selected[index] = true;
            }
            unread_events.push((
                index,
                selection.into_text(),
                ContextEvent {
                    seq: index as u64 + 1,
                    event: event.clone(),
                    fully_injected,
                },
            ));
        }
    }
    unread_events.sort_by_key(|(index, _, _)| *index);

    // 只有连续的实际注入事件才推进水位。若前面的未读事件被截断，后面的
    // 人类消息或 agent 发言虽已进入本轮，也不能越过它推进水位。
    let mut next = unread_start;
    while next < selected.len() && selected[next] {
        next += 1;
    }
    if next > unread_start {
        role.injection_watermark = next as u64;
    }

    TurnContext {
        unread_events: unread_events
            .iter()
            .map(|(_, rendered, _)| rendered.clone())
            .collect(),
        unread_event_metadata: unread_events
            .into_iter()
            .map(|(_, _, metadata)| metadata)
            .collect(),
        // 摘要由 maybe_update_rolling_summary 注入并持久化；本函数不持有会话状态。
        summary: None,
        relevant_drafts: relevant_drafts
            .iter()
            .map(|(rendered, _)| rendered.clone())
            .collect(),
        relevant_draft_metadata: relevant_drafts
            .into_iter()
            .map(|(_, metadata)| metadata)
            .collect(),
    }
}

fn is_human(event: &RoomEvent) -> bool {
    matches!(event, RoomEvent::UserMessage { .. })
}

fn is_agent(event: &RoomEvent) -> bool {
    matches!(event, RoomEvent::AgentMessage { .. })
}

fn render_event(event: &RoomEvent) -> String {
    match event {
        RoomEvent::UserMessage { text, .. } => text.clone(),
        RoomEvent::AgentMessage {
            role_instance_id,
            text,
            ..
        } => format!(
            "<untrusted_peer_message role=\"{}\">{text}</untrusted_peer_message>",
            role_label(role_instance_id)
        ),
        RoomEvent::ClaimEvent {
            role_instance_id,
            slot_key,
            claimed,
            ..
        } => format!(
            "角色 {role_instance_id} {}草稿槽 {}",
            if *claimed { "认领" } else { "释放" },
            slot_key.0
        ),
        RoomEvent::HeldEvent {
            role_instance_id,
            reason,
            ..
        } => format!("角色 {role_instance_id} 暂缓：{reason}"),
        RoomEvent::FinalizeEvent {
            artifact_line,
            version,
            ..
        } => format!("产物线 {artifact_line:?} 已定稿 {version}"),
        RoomEvent::SystemNotice { text } => format!("系统提示：{text}"),
    }
}

fn role_label(role_instance_id: &str) -> &str {
    for role in [
        "frontend-design",
        "backend-design",
        "author",
        "reviewer",
        "researcher",
    ] {
        if role_instance_id == role
            || role_instance_id.starts_with(role)
                && role_instance_id.as_bytes().get(role.len()) == Some(&b'-')
        {
            return role;
        }
    }
    role_instance_id
}

fn relevant_draft_entries(
    role_key: GroupChatRoleKey,
    lines: &[ArtifactLine],
) -> Vec<(String, RelevantDraft)> {
    let writable = writable_slots(role_key);
    lines
        .iter()
        .flat_map(|line| line.drafts.iter().map(move |slot| (line, slot)))
        .filter(|(_, slot)| {
            slot.current.is_some()
                && (role_key == GroupChatRoleKey::Reviewer
                    || writable.iter().any(|key| key == &slot.slot_key))
        })
        .filter_map(|(line, slot)| {
            let draft = slot.current.as_ref()?;
            let rendered = if role_key == GroupChatRoleKey::Reviewer {
                format_review_draft(line, &slot.slot_key.0, draft.version, &draft.markdown)
            } else {
                format!(
                    "草稿槽 {}（v{}）：\n{}",
                    slot.slot_key.0, draft.version, draft.markdown
                )
            };
            Some((
                rendered,
                RelevantDraft {
                    line: line.kind,
                    slot: slot.slot_key.clone(),
                    version: draft.version,
                },
            ))
        })
        .collect()
}

fn format_review_draft(line: &ArtifactLine, slot: &str, version: u32, markdown: &str) -> String {
    // ArtifactDraft 只保留当前版本，无法重建旧版本正文；用 unified-diff 形态标出
    // 当前目标稿，后续持久化层提供旧正文后可直接替换为真实 diff。
    let added = markdown
        .lines()
        .map(|line| format!("+{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "审查目标 {:?}/{slot} v{version} diff：\n--- previous\n+++ current\n{added}",
        line.kind
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextSelection {
    text: String,
    complete: bool,
}

impl TextSelection {
    fn complete(text: String) -> Self {
        Self {
            text,
            complete: true,
        }
    }

    fn partial(text: String) -> Self {
        Self {
            text,
            complete: false,
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn into_text(self) -> String {
        self.text
    }
}

fn take_text(text: String, remaining: &mut usize, untrusted_agent: bool) -> Option<TextSelection> {
    if *remaining == 0 {
        return None;
    }
    // token 估算按字符数/4 粗算；这里用 ceil，避免零成本文本穿透预算。
    if estimated_tokens(&text) <= *remaining {
        *remaining -= estimated_tokens(&text);
        return Some(TextSelection::complete(text));
    }

    if untrusted_agent {
        // 不可信包裹必须完整存在；预算不足以容纳包裹时宁可暂不注入。
        let prefix = text.split_once('>')?.0.to_owned() + ">";
        let suffix = "</untrusted_peer_message>";
        let available_chars = remaining.saturating_mul(4);
        if available_chars <= prefix.chars().count() + suffix.chars().count() {
            return None;
        }
        let content_budget = available_chars - prefix.chars().count() - suffix.chars().count();
        let content = text
            .strip_prefix(&prefix)?
            .strip_suffix(suffix)?
            .chars()
            .take(content_budget)
            .collect::<String>();
        let rendered = format!("{prefix}{content}{suffix}");
        let cost = estimated_tokens(&rendered);
        if cost > *remaining {
            return None;
        }
        *remaining -= cost;
        return Some(TextSelection::partial(rendered));
    }

    let max_chars = remaining.saturating_mul(4);
    if max_chars == 0 {
        return None;
    }
    let truncated = text.chars().take(max_chars).collect::<String>();
    let cost = estimated_tokens(&truncated);
    if cost == 0 || cost > *remaining {
        return None;
    }
    *remaining -= cost;
    Some(TextSelection::partial(truncated))
}

fn estimated_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}
