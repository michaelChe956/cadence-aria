use std::sync::{Arc, mpsc};
use std::time::Duration;

use serde_json::Value;

use super::types::{ArtifactLine, ArtifactLineKind, GroupChatRoleKey, RoleInstance, RoomEvent};

/// 触发一次 triage 所需的聊天室快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriageInput {
    pub triggering_seq: u64,
    pub last_speaker: Option<String>,
    pub room_state: RoomStateView,
    pub lines: Vec<ArtifactLine>,
}

/// 规则路由需要的最小聊天室状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomStateView {
    pub roles: Vec<RoleInstance>,
    pub triggering_text: String,
}

/// triage 对本次触发的决策。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriageOutput {
    RespondTo(Vec<String>),
    NoOneNeedsToRespond,
}

/// 可插拔的 triage 路由器。
pub trait TriageRouter: Send + Sync {
    fn route(&self, input: &TriageInput) -> TriageOutput;
}

/// 基于职责关键词和产物线状态的确定性兜底路由器。
#[derive(Debug, Clone, Copy, Default)]
pub struct RuleRouter;

impl RuleRouter {
    pub fn new() -> Self {
        Self
    }
}

impl TriageRouter for RuleRouter {
    fn route(&self, input: &TriageInput) -> TriageOutput {
        let text = input.room_state.triggering_text.to_lowercase();
        let design_topic = contains_any(
            &text,
            &[
                "design", "设计", "frontend", "前端", "backend", "后端", "ui", "界面", "api",
                "接口",
            ],
        );
        let review_topic = contains_any(
            &text,
            &["review", "审查", "审核", "评审", "风险", "问题", "校验"],
        );
        let research_topic = contains_any(
            &text,
            &["research", "调研", "调查", "代码现状", "架构现状", "搜索"],
        );
        let frontend_topic = contains_any(
            &text,
            &[
                "frontend",
                "前端",
                "浏览器",
                "界面",
                "ui",
                "react",
                "css",
                "component",
            ],
        );
        let backend_topic = contains_any(
            &text,
            &[
                "backend",
                "后端",
                "服务器",
                "数据库",
                "服务端",
                "api",
                "接口",
                "server",
                "rust",
            ],
        );
        let story_topic = contains_any(
            &text,
            &[
                "story",
                "用户故事",
                "故事",
                "验收标准",
                "acceptance criteria",
            ],
        );
        let issue_topic = contains_any(&text, &["issue", "需求", "澄清", "问题描述"]);
        let draft_topic = contains_any(&text, &["草稿", "版本", "定稿", "draft", "spec"]);

        let design_pending = input
            .lines
            .iter()
            .any(|line| line.kind == ArtifactLineKind::DesignSpec && has_pending_slot(line));
        let issue_pending = input
            .lines
            .iter()
            .any(|line| line.kind == ArtifactLineKind::IssueRefinement && has_pending_slot(line));
        let story_pending = input
            .lines
            .iter()
            .any(|line| line.kind == ArtifactLineKind::StorySpec && has_pending_slot(line));
        let has_reviewable_draft = has_reviewable_draft(&input.lines);
        let mut candidates = Vec::new();

        // Design 话题优先交给两个分节执笔角色；设计线状态也能唤起它们。
        if design_topic || design_pending {
            // 明确讨论“设计”时两个分节都需要获得机会；更窄的前后端关键词
            // 则只唤起对应分节，避免不相关角色抢答。
            if design_topic || frontend_topic || design_pending {
                candidates.extend(role_ids(
                    &input.room_state.roles,
                    GroupChatRoleKey::FrontendDesign,
                ));
            }
            if design_topic || backend_topic || design_pending {
                candidates.extend(role_ids(
                    &input.room_state.roles,
                    GroupChatRoleKey::BackendDesign,
                ));
            }
        }
        if story_topic || issue_topic || story_pending || issue_pending {
            candidates.extend(role_ids(&input.room_state.roles, GroupChatRoleKey::Author));
        }
        if review_topic
            || (draft_topic && has_reviewable_draft)
            || (!design_topic && has_reviewable_draft)
        {
            candidates.extend(role_ids(
                &input.room_state.roles,
                GroupChatRoleKey::Reviewer,
            ));
        }
        if research_topic {
            candidates.extend(role_ids(
                &input.room_state.roles,
                GroupChatRoleKey::Researcher,
            ));
        }

        // 过滤重复角色和自我路由，并限制一次最多两个响应者。
        candidates.retain(|id| input.last_speaker.as_deref() != Some(id.as_str()));
        candidates.dedup();
        candidates.truncate(2);

        if candidates.is_empty() {
            TriageOutput::NoOneNeedsToRespond
        } else {
            TriageOutput::RespondTo(candidates)
        }
    }
}

/// 小模型 triage 调用的回调桩。
///
/// 回调返回 JSON 数组（如 `["reviewer-1"]`）或逗号分隔的角色 ID；调用失败、
/// 超时或结果无法解析时统一回退到 `RuleRouter`。真实 provider 接线由后续任务完成。
pub struct LlmRouter<F> {
    call: Arc<F>,
    fallback: RuleRouter,
}

impl<F> LlmRouter<F>
where
    F: Fn(&TriageInput) -> Result<String, String> + Send + Sync + 'static,
{
    pub fn new(call: F) -> Self {
        Self {
            call: Arc::new(call),
            fallback: RuleRouter,
        }
    }
}

impl<F> TriageRouter for LlmRouter<F>
where
    F: Fn(&TriageInput) -> Result<String, String> + Send + Sync + 'static,
{
    fn route(&self, input: &TriageInput) -> TriageOutput {
        let call = Arc::clone(&self.call);
        let snapshot = input.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call(&snapshot)))
                .ok()
                .and_then(Result::ok);
            let _ = sender.send(result);
        });

        let parsed = receiver
            .recv_timeout(Duration::from_secs(30))
            .ok()
            .flatten()
            .and_then(|raw| parse_role_ids(&raw, input));

        parsed.map_or_else(
            || self.fallback.route(input),
            |ids| {
                if ids.is_empty() {
                    TriageOutput::NoOneNeedsToRespond
                } else {
                    TriageOutput::RespondTo(ids)
                }
            },
        )
    }
}

/// 跟踪连续的 NoOne 决策。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoOneCounter {
    consecutive_no_one: u8,
}

impl NoOneCounter {
    /// 新的人类消息、草稿或已落盘 agent 发言开始时调用，HOLD 重试不调用。
    pub fn on_new_trigger(&mut self) {
        self.consecutive_no_one = 0;
    }

    /// 观察一次 triage 决策；连续第二次 NoOne 时返回待落盘的系统事件。
    pub fn observe(&mut self, output: &TriageOutput) -> Option<RoomEvent> {
        if !matches!(output, TriageOutput::NoOneNeedsToRespond) {
            // RespondTo 打断连续 NoOne 轮次；只有连续两轮无人响应才提示人类。
            self.consecutive_no_one = 0;
            return None;
        }

        self.consecutive_no_one = self.consecutive_no_one.saturating_add(1);
        (self.consecutive_no_one == 2).then(|| RoomEvent::SystemNotice {
            text: "当前讨论暂无待响应方".into(),
        })
    }

    pub fn consecutive_no_one(&self) -> u8 {
        self.consecutive_no_one
    }
}

fn parse_role_ids(raw: &str, input: &TriageInput) -> Option<Vec<String>> {
    let trimmed = raw.trim();
    let ids = if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        match value {
            Value::Array(values) => values
                .into_iter()
                .map(|value| value.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>(),
            Value::Object(mut object) => {
                if object
                    .remove("no_one_needs_to_respond")
                    .and_then(|value| value.as_bool())
                    == Some(true)
                {
                    Some(Vec::new())
                } else {
                    object
                        .remove("respond_to")
                        .or_else(|| object.remove("responders"))
                        .or_else(|| object.remove("role_ids"))
                        .and_then(|value| match value {
                            Value::Array(values) => values
                                .into_iter()
                                .map(|value| value.as_str().map(str::to_owned))
                                .collect::<Option<Vec<_>>>(),
                            _ => None,
                        })
                }
            }
            Value::String(value)
                if matches!(value.as_str(), "none" | "no_one_needs_to_respond") =>
            {
                Some(Vec::new())
            }
            _ => None,
        }
    } else if matches!(
        trimmed,
        "none" | "no_one_needs_to_respond" | "NoOneNeedsToRespond"
    ) {
        Some(Vec::new())
    } else if trimmed.is_empty() {
        None
    } else {
        Some(
            trimmed
                .split([',', '\n'])
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .collect(),
        )
    }?;

    let known_ids = input
        .room_state
        .roles
        .iter()
        .map(|role| role.id.as_str())
        .collect::<Vec<_>>();
    if ids.iter().any(|id| !known_ids.contains(&id.as_str())) {
        return None;
    }

    let mut ids = ids;
    ids.retain(|id| input.last_speaker.as_deref() != Some(id.as_str()));
    ids.dedup();
    ids.truncate(2);
    Some(ids)
}

fn role_ids(roles: &[RoleInstance], role_key: GroupChatRoleKey) -> Vec<String> {
    roles
        .iter()
        .filter(|role| role.role_key == role_key)
        .map(|role| role.id.clone())
        .collect()
}

fn has_pending_slot(line: &ArtifactLine) -> bool {
    line.drafts.iter().any(|slot| slot.current.is_none())
}

fn has_reviewable_draft(lines: &[ArtifactLine]) -> bool {
    lines
        .iter()
        .any(|line| line.drafts.iter().any(|slot| slot.current.is_some()))
}

fn contains_any(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|keyword| text.contains(keyword))
}
