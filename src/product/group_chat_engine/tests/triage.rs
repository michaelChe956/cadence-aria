use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
use crate::product::group_chat_engine::triage::{
    LlmRouter, NoOneCounter, RoomStateView, RuleRouter, TriageInput, TriageOutput, TriageRouter,
};
use crate::product::group_chat_engine::types::{
    ArtifactLine, ArtifactLineKind, DraftSlot, DraftSlotKey, GroupChatRoleKey, RoleInstance,
    RoomEvent,
};
use crate::product::models::ProviderName;

fn role(id: &str, role_key: GroupChatRoleKey) -> RoleInstance {
    RoleInstance {
        id: id.into(),
        role_key,
        provider: ProviderName::Fake,
        display_name: id.into(),
        permission_mode: ProviderPermissionMode::Auto,
        seen_cursor: 0,
        injection_watermark: 0,
    }
}

fn design_line() -> ArtifactLine {
    ArtifactLine {
        kind: ArtifactLineKind::DesignSpec,
        drafts: vec![
            DraftSlot {
                slot_key: DraftSlotKey("design_frontend".into()),
                current: None,
                claim: None,
            },
            DraftSlot {
                slot_key: DraftSlotKey("design_backend".into()),
                current: None,
                claim: None,
            },
        ],
        finalized_versions: vec![],
        entity_id: None,
        bridge_session_id: None,
    }
}

fn input(text: &str, last_speaker: Option<&str>) -> TriageInput {
    TriageInput {
        triggering_seq: 7,
        last_speaker: last_speaker.map(str::to_owned),
        room_state: RoomStateView {
            roles: vec![
                role("author-1", GroupChatRoleKey::Author),
                role("fe-1", GroupChatRoleKey::FrontendDesign),
                role("be-1", GroupChatRoleKey::BackendDesign),
                role("reviewer-1", GroupChatRoleKey::Reviewer),
                role("researcher-1", GroupChatRoleKey::Researcher),
            ],
            triggering_text: text.into(),
        },
        lines: vec![design_line()],
    }
}

#[test]
fn design_topic_routes_to_frontend_and_backend() {
    let router = RuleRouter;

    assert_eq!(
        router.route(&input("请讨论这个设计方案", None)),
        TriageOutput::RespondTo(vec!["fe-1".into(), "be-1".into()])
    );
}

#[test]
fn rule_router_does_not_route_last_speaker_to_themselves() {
    let router = RuleRouter;

    assert_eq!(
        router.route(&input("请讨论这个设计方案", Some("fe-1"))),
        TriageOutput::RespondTo(vec!["be-1".into()])
    );
}

#[test]
fn llm_parse_failure_falls_back_to_rule_router() {
    let router = LlmRouter::new(|_| Ok("这不是可解析的路由结果".into()));

    assert_eq!(
        router.route(&input("请讨论这个设计方案", None)),
        TriageOutput::RespondTo(vec!["fe-1".into(), "be-1".into()])
    );
}

#[test]
fn no_one_for_two_rounds_emits_notice_and_new_trigger_resets_counter() {
    let mut counter = NoOneCounter::default();

    assert!(
        counter
            .observe(&TriageOutput::NoOneNeedsToRespond)
            .is_none()
    );
    assert_eq!(
        counter.observe(&TriageOutput::NoOneNeedsToRespond),
        Some(RoomEvent::SystemNotice {
            text: "当前讨论暂无待响应方".into(),
        })
    );

    counter.on_new_trigger();
    assert!(
        counter
            .observe(&TriageOutput::NoOneNeedsToRespond)
            .is_none()
    );
}
