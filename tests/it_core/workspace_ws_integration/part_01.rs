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
use cadence_aria::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, AuthorDecision, ProviderConfigSnapshot, ReviewVerdictType,
    TimelineNodeStatus, TimelineNodeType, WsInMessage, WsOutMessage, WsProviderStatus,
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
use tokio::sync::mpsc;
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

#[tokio::test]
async fn workspace_ws_author_text_choice_blocks_reviewer_until_user_answers() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let provider_state = Arc::new(ChoiceThenArtifactProviderState::default());
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ChoiceThenArtifactProvider {
            state: provider_state.clone(),
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
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "开始生成".to_string(),
        },
    )
    .await;

    let choice = recv_until_choice_request(&mut ws).await;
    assert_eq!(choice.source.as_deref(), Some("text_fallback"));
    assert!(choice.prompt.contains("n <= 0"));
    assert_eq!(choice.options.len(), 3);
    assert_eq!(choice.options[0].id, "A");
    assert_eq!(choice.options[1].id, "B");
    assert_eq!(choice.options[2].id, "C");

    send_json(
        &mut ws,
        &WsInMessage::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["A".to_string()],
            free_text: None,
            answers: vec![],
        },
    )
    .await;

    let checkpoint = recv_until_message_complete(&mut ws).await;
    assert!(checkpoint.starts_with("cp_"));
    accept_author_output(&mut ws).await;
    let stage = recv_until_stage(&mut ws, "human_confirm").await;
    assert_eq!(stage, "human_confirm");

    let prompts = provider_state.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("用户回答了 author 的确认问题"));
    assert!(prompts[1].contains("A. 返回 `0`"));
    assert!(!prompts[1].contains("[system]:"));
    assert!(!prompts[1].contains("[assistant]:"));
    let resume_ids = provider_state.resume_ids.lock().unwrap().clone();
    assert_eq!(resume_ids.len(), 2);
    assert_eq!(resume_ids[0], None);
    assert_eq!(resume_ids[1].as_deref(), Some("author-provider-session-1"));

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_reviewer_does_not_resume_author_provider_session() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture_with_providers(&root, "codex", "codex", 1).await;
    let provider_state = Arc::new(RoleResumeRecordingProviderState::default());
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Codex,
        Arc::new(RoleResumeRecordingProvider {
            state: provider_state.clone(),
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
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "开始生成".to_string(),
        },
    )
    .await;

    let _checkpoint = recv_until_message_complete(&mut ws).await;
    accept_author_output(&mut ws).await;
    let stage = recv_until_stage(&mut ws, "human_confirm").await;
    assert_eq!(stage, "human_confirm");

    assert_eq!(
        provider_state.author_resume_ids.lock().unwrap().as_slice(),
        &[None]
    );
    assert_eq!(
        provider_state
            .reviewer_resume_ids
            .lock()
            .unwrap()
            .as_slice(),
        &[None]
    );

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_author_recommendation_choice_blocks_reviewer_until_user_answers() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let author_prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ScriptedStreamingProvider::new(
            [
                "我会先按仓库要求读取本地规则和相关技能说明，再判断这个 Story Spec 是否还有需要先向你确认的问题。\
                 当前只剩一个会影响 Story Spec 边界的问题需要确认：\n\n\
                 对 `n <= 0` 的输入，这个 issue 希望如何处理？\n\n\
                 推荐选项：只声明本次需求支持 `n >= 1`，`n <= 0` 不纳入当前 issue 范围。  \n\
                 其他可选：返回 `0`；或抛出 `ValueError`。",
                 "# Story Spec\n\n\
                 ## 范围\n\
                 来源 source id: Issue issue_0001；实现 climb_stairs。\n\n\
                 ## 用户故事\n\
                 作为调用方，我需要计算爬楼梯方法数。\n\n\
                 ## 功能需求\n\
                 - [REQ-001] 实现 `climb_stairs(n: i32) -> i32`。\n\n\
                 ## 成功标准\n\
                 - [AC-001] 覆盖 n=1、n=2、n=3、n=5、n=10。\n\n\
                 ## 待确认项\n\
                 无\n\n\
                 ## 非功能需求\n\
                 使用 Python 实现。",
            ],
            author_prompts.clone(),
        )),
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
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "开始生成".to_string(),
        },
    )
    .await;

    let choice = recv_until_choice_request(&mut ws).await;
    assert_eq!(choice.source.as_deref(), Some("text_fallback"));
    assert!(choice.prompt.contains("n <= 0"));
    assert_eq!(choice.options.len(), 3);
    assert!(choice.options[0].label.contains("n >= 1"));
    assert!(choice.options[1].label.contains("返回 `0`"));
    assert!(choice.options[2].label.contains("ValueError"));

    send_json(
        &mut ws,
        &WsInMessage::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["A".to_string()],
            free_text: None,
            answers: vec![],
        },
    )
    .await;

    let checkpoint = recv_until_message_complete(&mut ws).await;
    assert!(checkpoint.starts_with("cp_"));
    accept_author_output(&mut ws).await;
    let stage = recv_until_stage(&mut ws, "human_confirm").await;
    assert_eq!(stage, "human_confirm");

    let prompts = author_prompts.lock().unwrap();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("用户回答了 author 的确认问题"));
    assert!(prompts[1].contains("只声明本次需求支持 `n >= 1`"));
    assert!(!prompts[1].contains("[system]:"));
    assert!(!prompts[1].contains("[assistant]:"));

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_claude_author_text_choice_uses_text_fallback_delta_only_followup() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture_with_author(&root, "claude_code").await;
    let provider_state = Arc::new(ChoiceThenArtifactProviderState::default());
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, Arc::new(FakeStreamingProvider));
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(ChoiceThenArtifactProvider {
            state: provider_state.clone(),
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
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "开始生成".to_string(),
        },
    )
    .await;

    let choice = recv_until_choice_request(&mut ws).await;
    assert_eq!(choice.source.as_deref(), Some("text_fallback"));
    assert!(choice.prompt.contains("n <= 0"));
    assert_eq!(choice.options.len(), 3);

    send_json(
        &mut ws,
        &WsInMessage::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["A".to_string()],
            free_text: None,
            answers: vec![],
        },
    )
    .await;

    let checkpoint = recv_until_message_complete(&mut ws).await;
    assert!(checkpoint.starts_with("cp_"));
    accept_author_output(&mut ws).await;
    let stage = recv_until_stage(&mut ws, "human_confirm").await;
    assert_eq!(stage, "human_confirm");

    let prompts = provider_state.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("用户回答了 author 的确认问题"));
    assert!(prompts[1].contains("A. 返回 `0`"));
    assert!(!prompts[1].contains("[system]:"));
    assert!(!prompts[1].contains("[assistant]:"));
    let resume_ids = provider_state.resume_ids.lock().unwrap().clone();
    assert_eq!(resume_ids.len(), 2);
    assert_eq!(resume_ids[0], None);
    assert_eq!(resume_ids[1].as_deref(), Some("author-provider-session-1"));

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_streams_persistent_session_and_confirms_lifecycle_entity() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
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
        WsOutMessage::SessionState {
            session_id,
            workspace_type,
            stage,
            messages,
            providers,
            ..
        } => {
            assert_eq!(session_id, "workspace_session_0001");
            assert_eq!(
                serde_json::to_value(workspace_type).unwrap(),
                json!("story")
            );
            assert_eq!(stage, "prepare_context");
            assert_eq!(
                serde_json::to_value(providers.author).unwrap(),
                json!("fake")
            );
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, "system");
            assert!(messages[0].content.contains("候选 spec 生成器"));
            assert!(messages[0].content.contains("必须遵守 using-superpowers"));
            assert!(messages[0].content.contains("brainstorming"));
            assert!(messages[0].content.contains("OpenSpec"));
            assert!(messages[0].content.contains("不要直接修改 OpenSpec"));
            assert!(messages[0].content.contains("## 功能需求"));
            assert!(messages[0].content.contains("[REQ-001]"));
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "请生成带验收标准的 Story Spec".to_string(),
        },
    )
    .await;

    let mut stream_chunks = String::new();
    let mut checkpoint_id = None;
    let mut saw_artifact = false;
    let mut saw_human_confirm = false;
    for _ in 0..220 {
        match recv_json(&mut ws).await {
            WsOutMessage::StreamChunk { content, .. } => stream_chunks.push_str(&content),
            WsOutMessage::ArtifactUpdate { payload, .. } => {
                saw_artifact = payload.markdown_or_empty().contains("Story Spec");
            }
            WsOutMessage::MessageComplete {
                checkpoint_id: next_checkpoint,
                ..
            } => checkpoint_id = Some(next_checkpoint),
            WsOutMessage::StageChange { stage } if stage == "author_confirm" => {
                send_json(
                    &mut ws,
                    &WsInMessage::AuthorDecision {
                        decision: AuthorDecision::Accept,
                    },
                )
                .await;
            }
            WsOutMessage::StageChange { stage } if stage == "human_confirm" => {
                saw_human_confirm = true;
                break;
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }

    assert!(stream_chunks.contains("Story Spec"));
    assert!(stream_chunks.contains("## 范围"));
    assert!(stream_chunks.contains("## 功能需求"));
    assert!(stream_chunks.contains("REQ-001"));
    assert!(stream_chunks.contains("AC-001"));
    assert!(
        !stream_chunks.contains("[system]"),
        "workspace output should be generated artifact markdown, not the raw prompt"
    );
    assert!(saw_artifact);
    assert!(checkpoint_id.is_some());
    assert!(saw_human_confirm);

    send_json(&mut ws, &WsInMessage::Confirm).await;
    let confirmed_stage = recv_until_stage(&mut ws, "completed").await;
    assert_eq!(confirmed_stage, "completed");

    let lifecycle = lifecycle_json(root.path()).await;
    assert_eq!(lifecycle["workspace_sessions"][0]["status"], "confirmed");
    assert_eq!(
        lifecycle["story_specs"][0]["confirmation_status"],
        "confirmed"
    );
    assert_eq!(lifecycle["story_specs"][0]["current_version"], 1);
    let version_path = root.path().join(
        ".aria/projects/project_0001/issues/issue_0001/versions/story_spec_0001/version_0001.json",
    );
    let version: Value =
        serde_json::from_str(&fs::read_to_string(version_path).expect("story version file"))
            .expect("story version json");
    assert_eq!(version["version"], 1);
    assert!(
        version["markdown"]
            .as_str()
            .expect("version markdown")
            .contains("Story Spec")
    );

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_reconnect_restores_timeline_and_artifact_versions() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
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
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "请生成 Story Spec".to_string(),
        },
    )
    .await;
    accept_author_output(&mut ws).await;
    let stage = recv_until_stage(&mut ws, "human_confirm").await;
    assert_eq!(stage, "human_confirm");
    drop(ws);

    let (mut reconnected, _) = connect_async(url).await.expect("reconnect ws");
    match recv_json(&mut reconnected).await {
        WsOutMessage::SessionState {
            timeline_nodes,
            artifact_versions,
            artifact_version_summaries,
            ..
        } => {
            assert!(timeline_nodes.iter().any(|node| {
                node.node_type == TimelineNodeType::AuthorRun
                    && node.summary.as_deref() == Some("生成完成")
            }));
            assert!(timeline_nodes.iter().any(|node| {
                node.node_type == TimelineNodeType::ReviewerRun
                    && node.summary.as_deref() == Some("未执行真实 review（Fake 快速路径）")
            }));
            assert_eq!(artifact_versions.len(), 0);
            assert_eq!(artifact_version_summaries.len(), 1);
            assert_eq!(
                artifact_version_summaries[0].generated_by,
                ProviderName::Fake
            );
            assert_eq!(
                artifact_version_summaries[0].reviewed_by,
                Some(ProviderName::Fake)
            );
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    drop(reconnected);
    server.abort();
}
