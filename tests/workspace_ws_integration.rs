use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::cross_cutting::claude_code_provider::ClaudeCodeProvider;
use cadence_aria::cross_cutting::codex_provider::CodexProvider;
use cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError;
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::cross_cutting::streaming_provider::{
    FakeStreamingProvider, ProviderCommand, ProviderEvent, ProviderSession,
    StreamingProviderAdapter, StreamingProviderInput,
};
use cadence_aria::product::models::ProviderName;
use cadence_aria::protocol::contracts::AdapterInput;
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use cadence_aria::web::workspace_ws_types::{
    TimelineNodeStatus, TimelineNodeType, WsInMessage, WsOutMessage,
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
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    let initial = recv_json(&mut ws).await;
    match initial {
        WsOutMessage::SessionState { messages, .. } => {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, "system");
            assert!(messages[0].content.contains("Workspace 生成任务已准备"));
            assert!(messages[0].content.contains("候选 spec 生成器"));
            assert!(messages[0].content.contains("OpenSpec"));
            assert!(messages[0].content.contains("必须遵守 using-superpowers"));
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
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
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
            WsOutMessage::ArtifactUpdate { markdown, .. } => {
                saw_artifact = markdown.contains("Story Spec");
            }
            WsOutMessage::MessageComplete {
                checkpoint_id: next_checkpoint,
                ..
            } => checkpoint_id = Some(next_checkpoint),
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
    let stage = recv_until_stage(&mut ws, "human_confirm").await;
    assert_eq!(stage, "human_confirm");
    drop(ws);

    let (mut reconnected, _) = connect_async(url).await.expect("reconnect ws");
    match recv_json(&mut reconnected).await {
        WsOutMessage::SessionState {
            timeline_nodes,
            artifact_versions,
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
            assert_eq!(artifact_versions.len(), 1);
            assert_eq!(artifact_versions[0].generated_by, ProviderName::Fake);
            assert_eq!(artifact_versions[0].reviewed_by, Some(ProviderName::Fake));
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    drop(reconnected);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_review_decision_continue_runs_revision_and_second_review() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture_with_providers(&root, "fake", "codex", 2).await;
    let author_prompts = Arc::new(Mutex::new(Vec::new()));
    let reviewer_prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ScriptedStreamingProvider::new(
            ["# Initial Story Spec", "# Revised Story Spec"],
            author_prompts.clone(),
        )),
    );
    registry.register(
        ProviderName::Codex,
        Arc::new(ScriptedStreamingProvider::new(
            [
                "需要补充失败路径。\n\n```json\n{\"verdict\":\"revise\",\"summary\":\"补充失败路径\"}\n```",
                "审核通过。\n\n```json\n{\"verdict\":\"pass\",\"summary\":\"可以确认\"}\n```",
            ],
            reviewer_prompts.clone(),
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
            content: "生成 Story Spec".to_string(),
        },
    )
    .await;

    let mut decision_required = false;
    for _ in 0..600 {
        match recv_json(&mut ws).await {
            WsOutMessage::ReviewDecisionRequired { options, .. } => {
                assert!(options.contains(&"continue_with_context".to_string()));
                decision_required = true;
                break;
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    assert!(decision_required, "review decision should be required");

    send_json(
        &mut ws,
        &WsInMessage::ReviewDecisionResponse {
            decision: "continue_with_context".to_string(),
            extra_context: Some("补充登录错误码".to_string()),
        },
    )
    .await;

    let mut saw_revision_stream = false;
    let mut saw_review_pass = false;
    let mut saw_human_confirm = false;
    for _ in 0..600 {
        match recv_json(&mut ws).await {
            WsOutMessage::StreamChunk { content, .. }
                if content.contains("# Revised Story Spec") =>
            {
                saw_revision_stream = true;
            }
            WsOutMessage::ReviewComplete { summary, .. } if summary == "可以确认" => {
                saw_review_pass = true;
            }
            WsOutMessage::StageChange { stage } if stage == "human_confirm" => {
                saw_human_confirm = true;
                break;
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }

    assert!(
        saw_revision_stream,
        "revision output should stream to websocket"
    );
    assert!(saw_review_pass, "second review should pass");
    assert!(
        saw_human_confirm,
        "second review pass should enter human confirm"
    );
    let prompts = author_prompts.lock().unwrap();
    let revision_prompt = prompts.get(1).expect("revision author prompt");
    assert!(revision_prompt.contains("需要补充失败路径"));
    assert!(revision_prompt.contains("补充登录错误码"));
    assert!(revision_prompt.contains("请根据以上审核意见修改产物"));
    assert_eq!(reviewer_prompts.lock().unwrap().len(), 2);

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_rollback_truncates_persistent_messages() {
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
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "first".to_string(),
        },
    )
    .await;
    let first_checkpoint = recv_until_message_complete(&mut ws).await;
    let _ = recv_until_stage(&mut ws, "human_confirm").await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "second".to_string(),
        },
    )
    .await;
    let _second_checkpoint = recv_until_message_complete(&mut ws).await;
    let _ = recv_until_stage(&mut ws, "human_confirm").await;

    send_json(
        &mut ws,
        &WsInMessage::Rollback {
            checkpoint_id: first_checkpoint,
        },
    )
    .await;

    let rolled_back = recv_until_session_state(&mut ws).await;
    match rolled_back {
        WsOutMessage::SessionState {
            messages, stage, ..
        } => {
            assert_eq!(stage, "human_confirm");
            assert_eq!(messages.len(), 3);
            assert!(messages.iter().any(|message| message.role == "system"));
            assert!(messages.iter().any(|message| message.content == "first"));
            assert!(!messages.iter().any(|message| message.content == "second"));
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    let lifecycle = lifecycle_json(root.path()).await;
    let messages = lifecycle["workspace_sessions"][0]["messages"]
        .as_array()
        .expect("messages");
    assert_eq!(messages.len(), 3);
    assert!(messages.iter().any(|message| message["role"] == "system"));
    assert!(
        !messages
            .iter()
            .any(|message| message["content"] == "second")
    );

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_provider_selection_persists_across_reconnect() {
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
    let (mut ws, _) = connect_async(url.clone()).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::ProviderSelect {
            role: "author".to_string(),
            provider: cadence_aria::product::models::ProviderName::Codex,
        },
    )
    .await;
    let updated = recv_until_session_state(&mut ws).await;
    match updated {
        WsOutMessage::SessionState { providers, .. } => {
            assert_eq!(
                serde_json::to_value(providers.author).unwrap(),
                json!("codex")
            );
        }
        other => panic!("expected session_state, got {other:?}"),
    }
    drop(ws);

    let (mut reconnected, _) = connect_async(url).await.expect("reconnect ws");
    let reloaded = recv_json(&mut reconnected).await;
    match reloaded {
        WsOutMessage::SessionState { providers, .. } => {
            assert_eq!(
                serde_json::to_value(providers.author).unwrap(),
                json!("codex")
            );
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    drop(reconnected);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_reconnect_restores_message_checkpoint_ids() {
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
    let (mut ws, _) = connect_async(url.clone()).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "restore checkpoint ids".to_string(),
        },
    )
    .await;
    let checkpoint_id = recv_until_message_complete(&mut ws).await;
    let _ = recv_until_stage(&mut ws, "human_confirm").await;
    drop(ws);

    let (mut reconnected, _) = connect_async(url).await.expect("reconnect ws");
    let reloaded = recv_json(&mut reconnected).await;
    match reloaded {
        WsOutMessage::SessionState { messages, .. } => {
            let assistant = messages
                .iter()
                .find(|message| message.role == "assistant")
                .expect("assistant message");
            assert_eq!(
                assistant.checkpoint_id.as_deref(),
                Some(checkpoint_id.as_str())
            );
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    drop(reconnected);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_user_message_interrupts_active_stream_before_completion() {
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
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: long_message("old_instruction"),
        },
    )
    .await;
    let _first_chunk = recv_until_stream_chunk(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "second_override".to_string(),
        },
    )
    .await;

    for _ in 0..200 {
        match recv_json(&mut ws).await {
            WsOutMessage::StreamChunk { content, .. } if content.contains("second_override") => {
                drop(ws);
                server.abort();
                return;
            }
            WsOutMessage::MessageComplete { .. } => {
                panic!("active stream completed before the interrupting message was applied")
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("interrupting message was not streamed");
}

#[tokio::test]
async fn workspace_ws_abort_discards_partial_stream_without_completion() {
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
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: long_message("abort_instruction"),
        },
    )
    .await;
    let _first_chunk = recv_until_stream_chunk(&mut ws).await;
    send_json(&mut ws, &WsInMessage::Abort).await;

    for _ in 0..80 {
        match recv_json(&mut ws).await {
            WsOutMessage::StageChange { stage } if stage == "prepare_context" => {
                let lifecycle = lifecycle_json(root.path()).await;
                let messages = lifecycle["workspace_sessions"][0]["messages"]
                    .as_array()
                    .expect("messages");
                assert_eq!(messages.len(), 2);
                assert!(messages.iter().any(|message| message["role"] == "system"));
                assert!(messages.iter().any(|message| {
                    message["role"] == "user"
                        && message["content"] == long_message("abort_instruction")
                }));
                drop(ws);
                server.abort();
                return;
            }
            WsOutMessage::MessageComplete { .. } => {
                panic!("aborted stream should not complete a partial assistant message")
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("abort did not return workspace to prepare_context");
}

#[tokio::test]
async fn workspace_ws_disconnect_during_active_run_writes_aborted_by_disconnect() {
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
    let (mut ws, _) = connect_async(url.clone()).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: long_message("disconnect_instruction"),
        },
    )
    .await;
    let _first_chunk = recv_until_stream_chunk(&mut ws).await;
    drop(ws);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let (mut reconnected, _) = connect_async(url).await.expect("reconnect ws");
    match recv_json(&mut reconnected).await {
        WsOutMessage::SessionState {
            stage,
            timeline_nodes,
            active_run_id,
            ..
        } => {
            let last = timeline_nodes.last().expect("timeline node");
            assert_eq!(stage, "prepare_context");
            assert_eq!(active_run_id, None);
            assert_eq!(last.node_type, TimelineNodeType::AbortedByDisconnect);
            assert_eq!(last.status, TimelineNodeStatus::Failed);
            assert!(
                last.summary
                    .as_deref()
                    .is_some_and(|summary| summary.contains("run-1"))
            );
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    drop(reconnected);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_idle_timeout_does_not_close_socket_during_active_run() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, Arc::new(HangingStreamingProvider));
    let state = WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    );
    state
        .test_controls
        .set_server_idle_timeout(Duration::from_millis(30))
        .await;
    let app = build_web_router(state);
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
            content: "start long running provider".to_string(),
        },
    )
    .await;
    let _first_chunk = recv_until_stream_chunk(&mut ws).await;

    let next_message = timeout(Duration::from_millis(120), ws.next()).await;
    assert!(
        next_message.is_err(),
        "idle timeout must not close the socket while a provider run is active"
    );

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_test_control_drop_closes_registered_socket() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let controls = state.test_controls.clone();
    let app = build_web_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    assert!(
        controls
            .drop_workspace_socket("workspace_session_0001")
            .await
    );

    let closed = timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("socket close timeout")
        .expect("socket close frame")
        .expect("valid close frame");
    assert!(matches!(closed, Message::Close(_)));

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_supervised_permission_allows_real_stream_to_complete() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture_with_author(&root, "claude_code").await;
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, Arc::new(FakeStreamingProvider));
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(ClaudeCodeProvider::new(executable_fixture(
            "tests/fixtures/provider/claude_stream_json_fixture.sh",
        ))),
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
            content: "run supervised provider".to_string(),
        },
    )
    .await;

    let permission = recv_until_permission_request(&mut ws).await;
    assert_eq!(permission.tool_name, "Bash");

    send_json(
        &mut ws,
        &WsInMessage::PermissionResponse {
            id: permission.id,
            approved: true,
            reason: None,
        },
    )
    .await;

    let checkpoint = recv_until_message_complete(&mut ws).await;
    assert!(checkpoint.starts_with("cp_"));
    let stage = recv_until_stage(&mut ws, "human_confirm").await;
    assert_eq!(stage, "human_confirm");

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_test_permission_fixture_emits_permission_request_for_fake_provider() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture(&root).await;
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    state
        .test_controls
        .enable_permission_fixture("workspace_session_0001".to_string())
        .await;
    let app = build_web_router(state);
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
            content: "run permission fixture".to_string(),
        },
    )
    .await;

    let permission = recv_until_permission_request(&mut ws).await;
    assert_eq!(permission.tool_name, "Bash");
    assert_eq!(permission.description, "E2E permission fixture request");

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_human_confirm_v2_completes_workspace() {
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
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "confirm with v2 message".to_string(),
        },
    )
    .await;
    assert_eq!(
        recv_until_stage(&mut ws, "human_confirm").await,
        "human_confirm"
    );

    send_json(
        &mut ws,
        &WsInMessage::HumanConfirm {
            decision: cadence_aria::web::workspace_ws_types::HumanConfirmDecision::Confirm,
            payload: None,
        },
    )
    .await;

    assert_eq!(recv_until_stage(&mut ws, "completed").await, "completed");

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_unmatched_permission_response_returns_protocol_error() {
    let root = tempdir().expect("root");
    let _repo = create_workspace_session_fixture_with_author(&root, "claude_code").await;
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, Arc::new(FakeStreamingProvider));
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(ClaudeCodeProvider::new(executable_fixture(
            "tests/fixtures/provider/claude_stream_json_fixture.sh",
        ))),
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
            content: "run supervised provider".to_string(),
        },
    )
    .await;

    let permission = recv_until_permission_request(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::PermissionResponse {
            id: "permission_not_pending".to_string(),
            approved: true,
            reason: Some("wrong request".to_string()),
        },
    )
    .await;

    match recv_until_protocol_error(&mut ws).await {
        WsOutMessage::ProtocolError { code, context, .. } => {
            assert_eq!(code, "PERMISSION_ID_UNMATCHED");
            assert_eq!(
                context
                    .as_ref()
                    .and_then(|value| value.get("permission_id"))
                    .and_then(|value| value.as_str()),
                Some("permission_not_pending")
            );
        }
        other => panic!("expected protocol_error, got {other:?}"),
    }

    send_json(
        &mut ws,
        &WsInMessage::PermissionResponse {
            id: permission.id,
            approved: true,
            reason: None,
        },
    )
    .await;

    let checkpoint = recv_until_message_complete(&mut ws).await;
    assert!(checkpoint.starts_with("cp_"));

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_codex_current_protocol_completes_from_repository_path() {
    let root = tempdir().expect("root");
    let repo = create_workspace_session_fixture_with_author(&root, "codex").await;
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, Arc::new(FakeStreamingProvider));
    registry.register(
        ProviderName::Codex,
        Arc::new(CodexProvider::new(executable_fixture(
            "tests/fixtures/provider/codex_app_server_current_fixture.sh",
        ))),
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
    let initial = recv_json(&mut ws).await;
    match initial {
        WsOutMessage::SessionState { messages, .. } => {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, "system");
            assert!(messages[0].content.contains("Workspace 生成任务已准备"));
            assert!(messages[0].content.contains("OpenSpec"));
            assert!(messages[0].content.contains("using-superpowers"));
            assert!(messages[0].content.contains("Repository 路径"));
            assert!(
                messages[0]
                    .content
                    .contains(&repo.path().display().to_string())
            );
        }
        other => panic!("expected session_state, got {other:?}"),
    }

    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "run codex current protocol".to_string(),
        },
    )
    .await;

    let expected_repo_path = repo
        .path()
        .canonicalize()
        .expect("repo canonical")
        .to_string_lossy()
        .to_string();
    let mut checkpoint = None;
    let mut saw_command_started = false;
    let mut saw_command_completed = false;
    for _ in 0..600 {
        match recv_json(&mut ws).await {
            WsOutMessage::ExecutionEvent { event } if event.event_id == "command_cmd_001" => {
                assert_eq!(serde_json::to_value(&event.kind).unwrap(), json!("command"));
                assert_eq!(event.command.as_deref(), Some("pwd"));
                assert_eq!(event.cwd.as_deref(), Some(expected_repo_path.as_str()));
                match serde_json::to_value(&event.status).unwrap() {
                    value if value == json!("started") => saw_command_started = true,
                    value if value == json!("completed") => {
                        assert_eq!(event.exit_code, Some(0));
                        assert!(
                            event
                                .output
                                .as_deref()
                                .unwrap_or_default()
                                .contains(expected_repo_path.as_str())
                        );
                        saw_command_completed = true;
                    }
                    other => panic!("unexpected command status: {other}"),
                }
            }
            WsOutMessage::MessageComplete {
                checkpoint_id: next_checkpoint,
                ..
            } => {
                checkpoint = Some(next_checkpoint);
                break;
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    assert!(
        saw_command_started,
        "websocket did not emit command started"
    );
    assert!(
        saw_command_completed,
        "websocket did not emit command completed"
    );
    assert!(checkpoint.as_deref().unwrap_or_default().starts_with("cp_"));
    let stage = recv_until_stage(&mut ws, "human_confirm").await;
    assert_eq!(stage, "human_confirm");

    drop(ws);
    server.abort();
}

#[tokio::test]
async fn workspace_ws_reconnect_during_review_decision_can_still_run_revision() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture_with_providers(&root, "fake", "codex", 2).await;
    let author_prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(ScriptedStreamingProvider::new(
            ["# Initial Story Spec", "# Revised After Reconnect"],
            author_prompts.clone(),
        )),
    );
    registry.register(
        ProviderName::Codex,
        Arc::new(ScriptedStreamingProvider::new(
            [
                "需要补充失败路径。\n\n```json\n{\"verdict\":\"revise\",\"summary\":\"补充失败路径\"}\n```",
                "审核通过。\n\n```json\n{\"verdict\":\"pass\",\"summary\":\"可以确认\"}\n```",
            ],
            Arc::new(Mutex::new(Vec::new())),
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
    let (mut ws, _) = connect_async(url.clone()).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    send_json(
        &mut ws,
        &WsInMessage::UserMessage {
            content: "生成 Story Spec".to_string(),
        },
    )
    .await;
    for _ in 0..600 {
        match recv_json(&mut ws).await {
            WsOutMessage::ReviewDecisionRequired { .. } => break,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    drop(ws);

    let (mut reconnected, _) = connect_async(url).await.expect("reconnect ws");
    let _state = recv_json(&mut reconnected).await;
    send_json(
        &mut reconnected,
        &WsInMessage::ReviewDecisionResponse {
            decision: "continue_with_context".to_string(),
            extra_context: Some("重连后补充".to_string()),
        },
    )
    .await;

    let mut saw_revision = false;
    let mut saw_human_confirm = false;
    for _ in 0..600 {
        match recv_json(&mut reconnected).await {
            WsOutMessage::StreamChunk { content, .. }
                if content.contains("# Revised After Reconnect") =>
            {
                saw_revision = true;
            }
            WsOutMessage::StageChange { stage } if stage == "human_confirm" => {
                saw_human_confirm = true;
                break;
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    assert!(saw_revision);
    assert!(saw_human_confirm);
    let prompts = author_prompts.lock().unwrap();
    assert!(prompts[1].contains("需要补充失败路径"));
    assert!(prompts[1].contains("重连后补充"));

    drop(reconnected);
    server.abort();
}

struct WorkingDirRecordingStreamingProvider {
    observed_working_dir: Arc<Mutex<Option<PathBuf>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for WorkingDirRecordingStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        *self.observed_working_dir.lock().unwrap() = Some(input.working_dir);
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: "# Draft".to_string(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: "# Draft".to_string(),
                    provider_session_id: None,
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<
        mpsc::Receiver<cadence_aria::cross_cutting::streaming_provider::StreamChunk>,
        ProviderAdapterError,
    > {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

struct ScriptedStreamingProvider {
    outputs: Mutex<VecDeque<String>>,
    prompts: Arc<Mutex<Vec<String>>>,
}

impl ScriptedStreamingProvider {
    fn new<const N: usize>(outputs: [&str; N], prompts: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into_iter().map(ToOwned::to_owned).collect()),
            prompts,
        }
    }
}

struct HangingStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for HangingStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: "# Draft".to_string(),
                })
                .await;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    command = command_rx.recv() => {
                        match command {
                            Some(ProviderCommand::Abort) | None => return,
                            Some(ProviderCommand::PermissionResponse { .. })
                            | Some(ProviderCommand::ChoiceResponse { .. }) => {}
                        }
                    }
                }
            }
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<
        mpsc::Receiver<cadence_aria::cross_cutting::streaming_provider::StreamChunk>,
        ProviderAdapterError,
    > {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScriptedStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.prompts.lock().unwrap().push(input.prompt);
        let output = self
            .outputs
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted provider output");
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output,
                    provider_session_id: None,
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<
        mpsc::Receiver<cadence_aria::cross_cutting::streaming_provider::StreamChunk>,
        ProviderAdapterError,
    > {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace websocket",
            0,
        ))
    }
}

async fn create_workspace_session_fixture(root: &TempDir) -> TempDir {
    create_workspace_session_fixture_with_author(root, "fake").await
}

async fn create_workspace_session_fixture_with_author(
    root: &TempDir,
    author_provider: &str,
) -> TempDir {
    create_workspace_session_fixture_with_providers(root, author_provider, "fake", 1).await
}

async fn create_workspace_session_fixture_with_providers(
    root: &TempDir,
    author_provider: &str,
    reviewer_provider: &str,
    review_rounds: u32,
) -> TempDir {
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;
    let (status, story_response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title":"登录会话过期提示",
            "author_provider":author_provider,
            "reviewer_provider":reviewer_provider,
            "review_rounds":review_rounds,
            "superpowers_enabled":true,
            "openspec_enabled":true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        story_response["workspace_session"]["workspace_session_id"],
        "workspace_session_0001"
    );
    repo
}

fn clear_workspace_session_messages(root: &std::path::Path) {
    replace_workspace_session_messages(root, json!([]));
}

fn replace_workspace_session_messages(root: &std::path::Path, messages: Value) {
    let session_path = root.join(
        ".aria/projects/project_0001/issues/issue_0001/workspace-sessions/workspace_session_0001.json",
    );
    let mut session: Value =
        serde_json::from_str(&fs::read_to_string(&session_path).expect("workspace session json"))
            .expect("workspace session value");
    session["messages"] = messages;
    fs::write(
        &session_path,
        serde_json::to_string_pretty(&session).expect("workspace session json"),
    )
    .expect("write workspace session");
}

async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn lifecycle_json(root: &std::path::Path) -> Value {
    let app = build_web_router(WebAppState::new(
        root.to_path_buf(),
        WebRuntime::new_fake(root.to_path_buf()),
    ));
    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    lifecycle
}

async fn send_json(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    message: &WsInMessage,
) {
    ws.send(Message::Text(
        serde_json::to_string(message).unwrap().into(),
    ))
    .await
    .expect("send ws message");
}

async fn recv_json(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> WsOutMessage {
    let message = timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("ws message timeout")
        .expect("ws message")
        .expect("valid ws message");
    match message {
        Message::Text(text) => serde_json::from_str(&text).expect("ws json"),
        other => panic!("expected text ws message, got {other:?}"),
    }
}

async fn recv_until_message_complete(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> String {
    for _ in 0..600 {
        match recv_json(ws).await {
            WsOutMessage::MessageComplete { checkpoint_id, .. } => return checkpoint_id,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("message_complete not received");
}

async fn recv_until_stage(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    expected: &str,
) -> String {
    for _ in 0..40 {
        match recv_json(ws).await {
            WsOutMessage::StageChange { stage } if stage == expected => return stage,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("stage_change {expected} not received");
}

async fn recv_until_session_state(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> WsOutMessage {
    for _ in 0..40 {
        match recv_json(ws).await {
            state @ WsOutMessage::SessionState { .. } => return state,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("session_state not received");
}

async fn recv_until_stream_chunk(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> String {
    for _ in 0..40 {
        match recv_json(ws).await {
            WsOutMessage::StreamChunk { content, .. } => return content,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("stream_chunk not received");
}

#[derive(Debug)]
struct PermissionRequestSeen {
    id: String,
    tool_name: String,
    description: String,
}

async fn recv_until_permission_request(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> PermissionRequestSeen {
    for _ in 0..40 {
        match recv_json(ws).await {
            WsOutMessage::PermissionRequest {
                id,
                tool_name,
                description,
                ..
            } => {
                return PermissionRequestSeen {
                    id,
                    tool_name,
                    description,
                };
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("permission_request not received");
}

async fn recv_until_protocol_error(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> WsOutMessage {
    for _ in 0..40 {
        match recv_json(ws).await {
            event @ WsOutMessage::ProtocolError { .. } => return event,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("protocol_error not received");
}

fn long_message(token: &str) -> String {
    (0..80)
        .map(|idx| format!("{token}_{idx}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn executable_fixture(relative_path: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(&path)
            .unwrap_or_else(|error| panic!("fixture metadata {}: {error}", path.display()));
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions)
            .unwrap_or_else(|error| panic!("chmod fixture {}: {error}", path.display()));
    }
    path
}

fn git_repo() -> TempDir {
    let dir = tempdir().expect("repo");
    let status = Command::new("git")
        .args(["init", "--initial-branch", "main"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
    assert!(status.success());
    dir
}
