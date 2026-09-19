use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::cross_cutting::claude_code_provider::ClaudeCodeProvider;
use cadence_aria::cross_cutting::codex_provider::CodexProvider;
use cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError;
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::cross_cutting::streaming_provider::{
    ChoiceOptionData, ChoiceRequestData, ChoiceRequestSource, FakeStreamingProvider,
    ProviderCommand, ProviderEvent, ProviderSession, ProviderStatus, StreamChunk,
    StreamingProviderAdapter, StreamingProviderInput,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::ProviderName;
use cadence_aria::product::models::{AgentRole, NodeDetail, ProviderSnapshot};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterRole};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use cadence_aria::web::test_controls::TestControls;
use cadence_aria::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, HelloRole, ProviderConfigSnapshot,
    ReviewVerdictType, TimelineNodeStatus, TimelineNodeType, WsInMessage, WsOutMessage,
    WsProviderStatus,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tempfile::{TempDir, tempdir};
use tokio::net::TcpListener;
use tokio::sync::{Notify, mpsc};
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

const VALID_STORY_SPEC: &str = "# Story Spec\n\n\
## 范围\n\
来源 source id: Issue issue_0001；生成可审核的候选产物。\n\n\
## 用户故事\n\
作为审核者，我希望候选 Story Spec 结构完整且可追踪。\n\n\
## 功能需求\n\
- [REQ-001] 生成可审核的候选产物。\n\n\
## 成功标准\n\
- [AC-001] 候选产物包含成功标准。\n\n\
## 待确认项\n\
无\n\n\
## 非功能需求\n\
无\n";

const PI_POST_ABORT_OUTPUT: &str = "Pi post-abort output";

const INITIAL_STORY_SPEC: &str = "# Initial Story Spec\n\n\
## 范围\n\
来源 source id: Issue issue_0001；生成初始候选产物。\n\n\
## 用户故事\n\
作为审核者，我希望看到初始候选产物。\n\n\
## 功能需求\n\
- [REQ-001] 生成初始候选产物。\n\n\
## 成功标准\n\
- [AC-001] 初始候选产物可进入审核。\n\n\
## 待确认项\n\
无\n\n\
## 非功能需求\n\
无\n";

const REVISED_STORY_SPEC: &str = "# Revised Story Spec\n\n\
## 范围\n\
来源 source id: Issue issue_0001；补充返修后的候选产物。\n\n\
## 用户故事\n\
作为审核者，我希望返修候选产物保留追踪关系。\n\n\
## 功能需求\n\
- [REQ-001] 补充返修后的候选产物。\n\n\
## 成功标准\n\
- [AC-001] 返修候选产物可进入二次审核。\n\n\
## 待确认项\n\
无\n\n\
## 非功能需求\n\
无\n";

const REVISED_AFTER_RECONNECT_STORY_SPEC: &str = "# Revised After Reconnect\n\n\
## 范围\n\
来源 source id: Issue issue_0001；重连后继续生成返修候选产物。\n\n\
## 用户故事\n\
作为审核者，我希望重连后的返修产物仍可审核。\n\n\
## 功能需求\n\
- [REQ-001] 重连后继续生成返修候选产物。\n\n\
## 成功标准\n\
- [AC-001] 重连后的返修候选产物可进入审核。\n\n\
## 待确认项\n\
无\n\n\
## 非功能需求\n\
无\n";

#[tokio::test]
async fn workspace_ws_hydrates_context_for_existing_empty_session() {
    let root = tempdir().expect("root");
    let repo = create_workspace_session_fixture(&root).await;
    clear_workspace_session_messages(root.path());
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    let initial = recv_json(&mut ws).await;
    match initial {
        WsOutMessage::SessionState { messages, .. } => {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, "system");
            assert!(messages[0].content.contains("登录会话过期"));
            assert!(messages[0].content.contains("描述"));
            assert!(messages[0].content.contains("Repo"));
            assert!(
                messages[0]
                    .content
                    .contains(&repo.path().display().to_string())
            );
            assert!(messages[0].content.contains("登录会话过期提示"));
            assert!(messages[0].content.contains("候选 spec 生成器"));
            assert!(messages[0].content.contains("OpenSpec"));
            assert!(messages[0].content.contains("必须遵守 using-superpowers"));
            assert!(messages[0].content.contains("必须优先通过可用交互机制解决"));
            assert!(
                messages[0]
                    .content
                    .contains("当前 author provider 未声明原生结构化交互能力")
            );
            assert!(messages[0].content.contains("交给 text_fallback"));
            assert!(
                messages[0]
                    .content
                    .contains("不要把 A/B/C 选择题作为最终候选产物正文输出")
            );
            assert!(
                messages[0]
                    .content
                    .contains("不要把可通过当前用户确认解决的问题直接写入待确认项")
            );
            assert!(messages[0].content.contains("```artifact fenced block"));
            assert!(messages[0].content.contains("[REQ-001]"));
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn workspace_ws_replaces_legacy_context_with_generation_brief() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    replace_workspace_session_messages(
        root.path(),
        json!([{
            "role": "system",
            "content": "Workspace 上下文已准备\n\nWorkspace 类型: Story Spec\nIssue: 登录会话过期",
            "created_at": "2026-05-18T00:00:00Z"
        }]),
    );
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url.clone()).await.expect("connect ws");

    let initial = recv_json(&mut ws).await;
    match initial {
        WsOutMessage::SessionState { messages, .. } => {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, "system");
            assert!(messages[0].content.contains("Workspace 生成任务已准备"));
            assert!(messages[0].content.contains("候选 spec 生成器"));
            assert!(messages[0].content.contains("OpenSpec"));
            assert!(messages[0].content.contains("必须遵守 using-superpowers"));
            assert!(messages[0].content.contains("必须优先通过可用交互机制解决"));
            assert!(messages[0].content.contains("不要直接修改 OpenSpec"));
            assert!(!messages[0].content.contains("Workspace 上下文已准备"));
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn workspace_ws_runs_provider_from_repository_path() {
    let root = tempdir().expect("root");
    let repo = create_workspace_session_fixture(&root).await;
    let observed_working_dir = Arc::new(Mutex::new(None));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(WorkingDirRecordingStreamingProvider {
            observed_working_dir: observed_working_dir.clone(),
        }),
    );
    let app = build_web_router(WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url.clone()).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "check repository cwd".to_string(),
        },
    )
    .await;

    let checkpoint = recv_until_message_complete(&mut ws).await;
    assert!(checkpoint.starts_with("cp_"));
    assert_eq!(
        observed_working_dir.lock().unwrap().as_ref(),
        Some(&repo.path().canonicalize().expect("repo canonical path"))
    );

    drop(ws);
    server.abort();
}

// 退役留档（T5/REQ-RET-02）：`workspace_ws_author_text_choice_blocks_reviewer_until_user_answers` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`workspace_ws_reviewer_does_not_resume_author_provider_session` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`workspace_ws_author_recommendation_choice_blocks_reviewer_until_user_answers` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`workspace_ws_claude_author_text_choice_uses_text_fallback_delta_only_followup` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`workspace_ws_streams_persistent_session_and_confirms_lifecycle_entity` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`workspace_ws_reconnect_restores_timeline_and_artifact_versions` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。
