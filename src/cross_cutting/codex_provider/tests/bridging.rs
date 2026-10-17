// 桥接族（permission / request_user_input）：审批与用户输入桥接行为、
// requestUserInput 协议错误路径（bridge 失败 / 写失败）的 wire 级验证。
// 从 tests/mod.rs 拆出以满足 large_file_guard 的 1200 行上限。

use super::*;

#[tokio::test]
async fn codex_provider_bridges_permission_and_completes() {
    let fixture = executable_fixture("tests/fixtures/provider/codex_app_server_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Supervised);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let mut saw_text = false;
    let permission_id = loop {
        match session.events.recv().await.unwrap() {
            ProviderEvent::TextDelta { content } => {
                saw_text = content.contains("Codex fixture chunk");
            }
            ProviderEvent::PermissionRequest(data) => break data.id,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            other => panic!("unexpected event before permission: {other:?}"),
        }
    };
    assert!(saw_text);

    session
        .commands
        .send(ProviderCommand::PermissionResponse {
            id: permission_id,
            approved: true,
            reason: None,
        })
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "Codex fixture chunk");
}

#[tokio::test]
async fn codex_provider_responds_to_current_command_approval_with_json_rpc_result() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_app_server_current_permission_fixture.sh",
    );
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Supervised);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let permission = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit current command approval")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::PermissionRequest(request) => break request,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Completed(completion) => {
                let full_output = completion.full_output;
                panic!("provider completed before permission request: {full_output}")
            }
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_)
            | ProviderEvent::TerminalActivity(_) => {}
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    };
    assert_eq!(permission.tool_name, "command");
    assert!(permission.description.contains("pnpm -C web install"));

    session
        .commands
        .send(ProviderCommand::PermissionResponse {
            id: permission.id,
            approved: true,
            reason: None,
        })
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "permission accepted");
}

#[tokio::test]
async fn codex_provider_bridges_request_user_input_and_completes() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_user_input_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit a choice request")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(request) => break request,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Completed(completion) => {
                let full_output = completion.full_output;
                panic!("provider completed before choice request: {full_output}")
            }
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_)
            | ProviderEvent::TerminalActivity(_) => {}
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    };

    assert_eq!(choice.id, "77");
    assert_eq!(choice.prompt, "请选择复杂度");
    assert_eq!(choice.options[0].id, "O(n)");
    assert_eq!(choice.options[0].description.as_deref(), Some("线性复杂度"));

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["O(n)".to_string()],
            free_text: None,
            answers: vec![],
            receipt: None,
        })
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "Codex received O(n)");
}

#[tokio::test]
async fn codex_provider_bridges_all_request_user_input_questions() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_multi_user_input_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit a choice request")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(request) => break request,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Completed(completion) => {
                let full_output = completion.full_output;
                panic!("provider completed before choice request: {full_output}")
            }
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_)
            | ProviderEvent::TerminalActivity(_) => {}
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    };

    assert_eq!(choice.id, "91");
    assert_eq!(choice.questions.len(), 3);
    assert_eq!(choice.questions[0].id, "startup");
    assert_eq!(choice.questions[0].prompt, "启动自检策略？");
    assert_eq!(choice.questions[1].id, "scope");
    assert_eq!(choice.questions[1].prompt, "影响范围？");
    assert_eq!(choice.questions[2].id, "mcp_events");
    assert_eq!(choice.questions[2].prompt, "MCP 事件输出？");

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec![],
            free_text: None,
            answers: vec![
                ChoiceAnswerData {
                    question_id: "startup".to_string(),
                    selected_option_ids: vec!["每次启动都自检".to_string()],
                    free_text: None,
                },
                ChoiceAnswerData {
                    question_id: "scope".to_string(),
                    selected_option_ids: vec!["Story/Design/Work Item 共享链路".to_string()],
                    free_text: None,
                },
                ChoiceAnswerData {
                    question_id: "mcp_events".to_string(),
                    selected_option_ids: vec!["输出 MCP 事件".to_string()],
                    free_text: None,
                },
            ],
            receipt: None,
        })
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "Codex received all answers");
}

#[tokio::test]
async fn codex_provider_request_user_input_emits_protocol_error_on_bridge_failure() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_user_input_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let cancel = CancellationToken::new();
    let mut session = provider.start(input, cancel.clone()).await.unwrap();

    let _choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => {
                panic!("provider failed before choice: {message}")
            }
            _ => {}
        }
    };

    // 取消 provider 以强制 bridge 失败。
    cancel.cancel();

    let mut saw_protocol_error = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .unwrap_or(None)
    {
        match event {
            ProviderEvent::ProtocolError {
                code,
                message,
                context,
            } if code == "request_user_input_unresolved" => {
                assert!(
                    message.contains("requestUserInput"),
                    "unexpected message: {message}"
                );
                assert!(
                    message.contains("unresolved"),
                    "unexpected message: {message}"
                );
                let context = context.expect("protocol error should include context");
                assert_eq!(
                    context.get("question_id").and_then(Value::as_str),
                    Some("complexity")
                );
                saw_protocol_error = true;
                break;
            }
            ProviderEvent::Completed(_) => {
                panic!("expected protocol error before completion")
            }
            ProviderEvent::Failed { message } => {
                panic!("expected protocol error before failure: {message}")
            }
            _ => {}
        }
    }
    assert!(
        saw_protocol_error,
        "expected request_user_input_unresolved protocol error after bridge failure"
    );
}

// 写失败注入确定性化（RR-3 单例销账，wave4-passive-flaky）：原形态=真实进程 fixture
// （codex_request_user_input_peer_closes_fixture.sh 关闭 stdin 读端→EPIPE）叠加
// TEST_TIMEOUT 墙钟窗口——全量首跑在冷缓存/高并发负载下事件窗口可被饿死（历史
// 首跑红/单跑绿，stage4-c3-t5 §3.6/w1 登记）。验证方式迁移（d7525220 kimi terminal
// 同款先例）：受控内存流（tokio duplex）直接驱动 run_codex_session_loop，stdin
// 读端在触发写的 ChoiceResponse 发出之前 drop——写失败由因果定序保证，无真实
// 进程/无墙钟竞态；断言与原形态逐字一致（code/message/question_id）。
#[tokio::test]
async fn codex_provider_request_user_input_emits_protocol_error_on_write_failure() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (mut server_stdout, provider_stdout) = tokio::io::duplex(64 * 1024);
    let (provider_stdin, server_stdin) = tokio::io::duplex(64 * 1024);
    let peer = JsonRpcPeer::new(provider_stdout, provider_stdin)
        .with_outbound_id_namespace(OutboundIdNamespace::Aria);

    let (event_tx, mut session_events) = mpsc::channel(32);
    let bridge = ApprovalBridge::new(ProviderPermissionMode::Auto, event_tx.clone());
    let commands = bridge.command_sender();
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let cancel = CancellationToken::new();

    let session_loop = tokio::spawn(async move {
        run_codex_session_loop(
            peer,
            bridge,
            event_tx,
            input,
            cancel,
            CodexSessionHandshake {
                resume_session_id: None,
                thread_id: Some("codex_input_thread".to_string()),
            },
        )
        .await
    });

    // 服务端泵：应答 turn/start（复用出站原生 id），随后回放 requestUserInput 请求
    // （id=88/question=confirm/options=是|否——逐字取自已退役的 peer_closes 真实
    // fixture，保证 parse 路径与原形态一致）。
    let mut server_stdin_reader = BufReader::new(server_stdin);
    let mut turn_start_line = String::new();
    server_stdin_reader
        .read_line(&mut turn_start_line)
        .await
        .expect("provider should send turn/start");
    let turn_start_id = serde_json::from_str::<Value>(&turn_start_line)
        .expect("turn/start should be valid JSON")
        .get("id")
        .cloned()
        .expect("turn/start should carry an id");
    let turn_start_response = json!({
        "jsonrpc": "2.0",
        "id": turn_start_id,
        "result": { "turn": { "id": "codex_input_turn", "status": "inProgress" } },
    });
    server_stdout
        .write_all(format!("{turn_start_response}\n").as_bytes())
        .await
        .expect("server should answer turn/start");

    let request_user_input = json!({
        "jsonrpc": "2.0",
        "id": 88,
        "method": "item/tool/requestUserInput",
        "params": {
            "threadId": "codex_input_thread",
            "turnId": "codex_input_turn",
            "itemId": "ask_1",
            "questions": [{
                "id": "confirm",
                "header": "确认",
                "question": "继续？",
                "options": [{"label": "是"}, {"label": "否"}]
            }]
        }
    });
    server_stdout
        .write_all(format!("{request_user_input}\n").as_bytes())
        .await
        .expect("server should emit requestUserInput");

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session_events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => {
                panic!("provider failed before choice: {message}")
            }
            _ => {}
        }
    };

    // 确定性写失败注入：drop stdin 读端严格先于发出触发写的 ChoiceResponse——
    // provider 侧写失败时序由因果定序锁定（duplex 对端 drop 后写返回 BrokenPipe，
    // 与真实管道 EPIPE 同语义），不再依赖进程调度竞态。
    drop(server_stdin_reader);

    commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["是".to_string()],
            free_text: None,
            answers: vec![],
            receipt: None,
        })
        .await
        .expect("send choice response");

    let mut saw_protocol_error = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session_events.recv())
        .await
        .expect("provider should emit events")
    {
        match event {
            ProviderEvent::ProtocolError {
                code,
                message,
                context,
            } if code == "request_user_input_unresolved" => {
                assert!(
                    message.contains("requestUserInput"),
                    "unexpected message: {message}"
                );
                assert!(
                    message.contains("unresolved"),
                    "unexpected message: {message}"
                );
                let context = context.expect("protocol error should include context");
                assert_eq!(
                    context.get("question_id").and_then(Value::as_str),
                    Some("confirm")
                );
                saw_protocol_error = true;
                break;
            }
            ProviderEvent::Completed(_) => {
                panic!("expected protocol error before completion")
            }
            ProviderEvent::Failed { message } => {
                panic!("expected protocol error before failure: {message}")
            }
            _ => {}
        }
    }
    assert!(
        saw_protocol_error,
        "expected request_user_input_unresolved protocol error when JSON-RPC response write fails"
    );

    // 写失败必须以错误终止会话循环（原形态经 provider.start 包装任务发 Failed；
    // 迁移后直接断言会话循环返回 Err，覆盖同一契约）。
    let session_result = tokio::time::timeout(TEST_TIMEOUT, session_loop)
        .await
        .expect("session loop should finish after write failure")
        .expect("session task should join");
    assert!(
        session_result.is_err(),
        "write failure must terminate the session loop with an error"
    );
}

use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;

/// T12(REQ-LCG-04 改版):Author 承载角色(Orchestrator+deny)的 LC danger
/// 会话沿用策略审批冻结(GC6)——fileChange/commandExecution 在真实 wire 上
/// 得到即时 decline(不经 ApprovalBridge 上抛),MCP accept;thread/start
/// danger-full-access + on-request(LC danger fixture 逐字校验)。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t12_author_danger_lc_session_declines_write_approvals_on_wire() {
    let fixture = LcCodexFixture::new();
    let sink = RecordingToolPolicyAuditSink::new();
    let raw = fixture.lc_input(
        AdapterRole::Orchestrator,
        Some(ProviderToolPolicy::deny_file_write_builtins()),
        ProviderPermissionMode::Auto,
        None,
        Some(sink.clone().bound()),
        fixture.target_worktree(),
    );
    let provider = CodexProvider::new(executable_fixture(
        "tests/fixtures/provider/codex_app_server_lc_danger_policy_approval_fixture.sh",
    ))
    .with_version_supplier(lc_version_supplier());

    let mut session = provider
        .start_lc_validated(
            raw,
            &fixture.envelope(SessionPolicyAction::PlanningReadOnly, Vec::new()),
            "cap_codex_lc_fixture",
            CancellationToken::new(),
        )
        .await
        .expect("author danger lc policy session launches");

    // fixture 对任何未 decline/accept 的审批直接退出失败——完成即证明
    // fileChange/commandExecution=decline、MCP=accept 在 wire 上成立。
    assert_eq!(
        recv_completed(&mut session.events).await,
        "policy approvals done"
    );

    // 统一审计:LC 会话的 provider_start 带 lc_projection(danger 基线;
    // read-only action 无写面引用)。
    let events = sink.events();
    let crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ProviderStart(record) =
        &events[0]
    else {
        panic!("expected provider_start");
    };
    assert_eq!(record.sandbox.as_deref(), Some("danger-full-access"));
    assert_eq!(record.approval_policy.as_deref(), Some("on-request"));
    let lc_projection = record.lc_projection.as_ref().expect("lc projection audit");
    assert_eq!(lc_projection.action, "planning_read_only");
}

/// T12 approval 映射:LC Coding 的 Supervised 档在真实 wire 与统一审计上
/// 都是 approvalPolicy=on-request(独立审批映射;Auto=never 已由 coding
/// 主测试锁定),sandbox 全相位 danger-full-access。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t05_coding_supervised_maps_on_request_approval_on_wire() {
    let fixture = LcCodexFixture::new();
    let target = fixture.target_worktree();
    let sink = RecordingToolPolicyAuditSink::new();
    let marker_root = tempfile::tempdir().expect("supervised marker dir").keep();
    let mut raw = fixture.lc_input(
        AdapterRole::Executor,
        None,
        ProviderPermissionMode::Supervised,
        None,
        Some(sink.clone().bound()),
        fixture.canonical_root(),
    );
    let (_cwd, wire_marker, _spawn_marker) = lc_markers(&mut raw, &marker_root);
    let provider =
        CodexProvider::new(lc_app_server_fixture()).with_version_supplier(lc_version_supplier());

    let mut session = provider
        .start_lc_validated(
            raw,
            &fixture.envelope(SessionPolicyAction::CodingTargetWrite, vec![target.clone()]),
            "cap_codex_lc_fixture",
            CancellationToken::new(),
        )
        .await
        .expect("lc coding supervised launch succeeds");

    assert_eq!(
        recv_completed(&mut session.events).await,
        "lc restricted done"
    );

    let coding_params = lc_wire_params(&wire_marker);
    assert_eq!(coding_params["sandbox"], "danger-full-access");
    assert_eq!(
        coding_params["approvalPolicy"], "on-request",
        "Supervised→on-request 独立审批映射"
    );
    let events = sink.events();
    let crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ProviderStart(record) =
        &events[0]
    else {
        panic!("expected provider_start");
    };
    assert_eq!(record.approval_policy.as_deref(), Some("on-request"));
    assert_eq!(record.sandbox.as_deref(), Some("danger-full-access"));
}

// ==== Task 3.4(B0c):真实漂移零 session/child 与错 ID kill/reap ====

use std::sync::Arc;

use super::approval_policy::{
    ANCHOR_T34_VERSION_INITIAL, ANCHOR_T34_VERSION_REPLACED,
    anchor_t34_combined_app_server_fixture, anchor_t34_lc_harness, anchor_t34_lc_input,
    anchor_t34_markers,
};
use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent;
use crate::product::logical_codebase::provider_gateway::{
    GatewaySessionDisposition, ProviderLaunchAuditContext, ResumeSessionLaunchRequest,
    SessionResumeFingerprint, canonical_target_git_identity,
};

/// Task 3.4(B0c)Step 1 红测试:真实 CLI 命令 identity 变化(重写命令文件)
/// 后,重观测 version 与冻结 full 投影不一致 → typed 拒、零 adapter start
/// 足迹、零新 child;adapter 纵深(以陈旧冻结投影直启)在 child 前 typed 拒;
/// thread/resume 真实同 ID 确认,错 ID 应答 kill/reap 不伪称零 child。
///
/// 三口径:①版本观测(probe/supplier 调用,identity 换 key 后允许再观测);
/// ②adapter start 足迹(provider_start 审计落盘);③协议 child(mismatch
/// 尝试的独立 spawn marker)。mismatch 比较自身零 ②③;错 ID 是已启动 child
/// 后的 runtime 失败,kill/reap 并如实记录。
#[cfg(unix)]
#[tokio::test]
async fn anchor_t34_real_drift_zero_session_child() {
    let fixture_path = anchor_t34_combined_app_server_fixture(ANCHOR_T34_VERSION_INITIAL);
    let provider = Arc::new(CodexProvider::new(fixture_path.clone()));
    let recording =
        crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink::new();
    let harness = anchor_t34_lc_harness(provider.clone(), ANCHOR_T34_VERSION_INITIAL);

    // 1) prepare:真实 CLI ①观测恰一次,冻结 full 候选。
    let marker_root = tempfile::tempdir().expect("t34 drift marker dir").keep();
    let mut happy_input =
        anchor_t34_lc_input(&harness.authority_root, Some("codex-thread-lc".to_string()));
    let (_happy_cwd, _happy_wire, happy_spawn, _happy_pid) =
        anchor_t34_markers(&mut happy_input, &marker_root, "happy");
    let candidate = harness
        .gateway
        .prepare_lc_streaming_candidate(
            happy_input,
            harness.request.clone(),
            ProviderLaunchAuditContext {
                workspace_session_id: "ws-anchor-t34-drift".to_string(),
                role_run_seq: 1,
                audit_sink: recording.clone(),
            },
            CancellationToken::new(),
        )
        .await
        .expect("real cli codex prepare must freeze a full candidate");
    assert_eq!(
        candidate.facts().exact_version(),
        ANCHOR_T34_VERSION_INITIAL
    );
    let prepared_projection_digest = candidate.full_projection().projection_digest().to_string();

    // 2) full-matching resume → 真实 start:thread/resume 应答确认同 ID。
    let previous = SessionResumeFingerprint::from_envelope(
        candidate.validated().envelope(),
        candidate.full_projection(),
        candidate.validated().action_evidence_digest(),
        &canonical_target_git_identity(&candidate.validated().envelope().target.worktree),
    );
    let GatewaySessionDisposition::Resume(resume_validated) = harness
        .gateway
        .resume_or_start_with_projection(
            ResumeSessionLaunchRequest {
                launch: harness.request.clone(),
                previous_fingerprint: previous,
                previous_session_id: "sess-anchor-t34-drift".to_string(),
            },
            candidate.full_projection().clone(),
        )
        .expect("full-matching fingerprint must resume")
    else {
        panic!("expected a Resume disposition");
    };
    let mut session = provider
        .start_validated(
            ValidatedStreamingProviderInput::new(
                candidate.input().clone(),
                resume_validated.clone(),
            ),
            CancellationToken::new(),
        )
        .await
        .expect("prepared projection must start the confirmed native thread");
    assert_eq!(
        session.native_session_id.as_deref(),
        Some("codex-thread-lc"),
        "thread/resume confirmation must equal the requested id"
    );
    assert_eq!(
        recv_completed(&mut session.events).await,
        "lc restricted done"
    );
    assert!(
        happy_spawn.exists(),
        "the confirmed resume really spawned once"
    );

    let provider_start_count = || {
        recording
            .events()
            .iter()
            .filter(|event| matches!(event, DurableToolPolicyEvent::ProviderStart(_)))
            .count()
    };
    assert_eq!(provider_start_count(), 1);
    let prepared_audit_digest = recording
        .events()
        .iter()
        .find_map(|event| match event {
            DurableToolPolicyEvent::ProviderStart(record) => record
                .lc_projection
                .as_ref()
                .map(|lc| lc.projection_digest.clone()),
            _ => None,
        })
        .expect("the confirmed start wrote an lc projection audit");
    assert_eq!(prepared_projection_digest, prepared_audit_digest);

    // 3) 错 ID 应答(请求 codex-thread-wrong,应答 codex-thread-lc):已启动
    //    child 后的 runtime 失败——kill/reap、错误如实记录「未恢复」,零新
    //    provider_start(不伪称零 spawn)。
    let wrong_marker_root = tempfile::tempdir().expect("t34 wrong-id marker dir").keep();
    // 复用 prepare 绑定的 run-bound 面(sink/会话身份),只换 resume id 与
    // 观测 marker——与真实 caller 的错 ID 重试同一输入形态。
    let mut wrong_input = candidate.input().clone();
    wrong_input.resume_provider_session_id = Some("codex-thread-wrong".to_string());
    let (_wrong_cwd, _wrong_wire, wrong_spawn, wrong_pid) =
        anchor_t34_markers(&mut wrong_input, &wrong_marker_root, "wrong");
    let wrong_result = provider
        .start_validated(
            ValidatedStreamingProviderInput::new(wrong_input, resume_validated.clone()),
            CancellationToken::new(),
        )
        .await;
    let Err(wrong_error) = wrong_result else {
        panic!("a mismatched thread/resume confirmation must fail the start");
    };
    assert!(
        wrong_error.details.contains("session NOT resumed")
            && wrong_error
                .details
                .contains("the started child was killed and reaped"),
        "wrong-id rejection must record the not-resumed outcome without claiming zero spawn: {wrong_error:?}"
    );
    assert_eq!(
        provider_start_count(),
        1,
        "no fresh provider_start may be written for a resume the native side never confirmed"
    );
    assert!(
        wrong_spawn.exists(),
        "the wrong-id leg really spawned a child"
    );
    let wrong_native_child_killed_and_reaped =
        match super::approval_policy::wait_for_child_pid_marker_public(&wrong_pid) {
            Some(pid) => {
                super::approval_policy::assert_child_terminated_public(
                    pid,
                    "a wrong-id LC thread/resume handshake",
                );
                true
            }
            None => false,
        };
    assert!(wrong_native_child_killed_and_reaped);

    // 4) 真实漂移:重写命令文件(同路径新 identity:版本串回显 1.1.0)→
    //    TTL 换 key 重观测;与冻结 full 投影比对 typed 拒,比较自身零 ②③。
    std::thread::sleep(std::time::Duration::from_millis(50));
    let replaced_path = anchor_t34_combined_app_server_fixture(ANCHOR_T34_VERSION_REPLACED);
    std::fs::copy(&replaced_path, &fixture_path).expect("rewrite the command file in place");
    // ①身份变更后允许再次真实观测(不同 key,不串旧值)。
    let current = harness
        .gateway
        .reobserve_lc_prepare_facts(&candidate, CancellationToken::new())
        .await
        .expect("reobservation must re-probe under the new command identity key");
    assert_eq!(current.exact_version(), ANCHOR_T34_VERSION_REPLACED);
    let typed_rejection = candidate
        .assert_frozen_version(&current)
        .expect_err("a replaced command identity must be typed-rejected against frozen facts");
    assert!(
        matches!(
            &typed_rejection,
            crate::product::logical_codebase::ProviderGatewayError::PolicyDrift { dimension }
                if dimension == "provider_version"
        ),
        "unexpected rejection shape: {typed_rejection:?}"
    );
    // mismatch 比较自身零 ②③:无新 provider_start、无新 child(mismatch
    // 尝试使用独立 spawn marker)。
    let mismatch_marker_root = tempfile::tempdir().expect("t34 mismatch marker dir").keep();
    // 同上:复用 run-bound 面,只换观测 marker(陈旧冻结投影 + 已漂移的
    // 当前观测必须在 child 前被拒)。
    let mut mismatch_input = candidate.input().clone();
    let (_m_cwd, _m_wire, mismatch_spawn, _m_pid) =
        anchor_t34_markers(&mut mismatch_input, &mismatch_marker_root, "mismatch");
    let mismatch_result = provider
        .start_validated(
            ValidatedStreamingProviderInput::new(mismatch_input, resume_validated),
            CancellationToken::new(),
        )
        .await;
    let Err(mismatch_error) = mismatch_result else {
        panic!("a stale frozen projection must be refused before spawn (adapter defense)");
    };
    assert!(
        mismatch_error
            .details
            .contains("codex_lc_prepared_version_drift"),
        "adapter-side drift rejection must carry the stable marker: {mismatch_error:?}"
    );
    assert!(
        !mismatch_spawn.exists(),
        "no codex child may spawn for a drifted prepared projection"
    );
    let mismatch_adapter_starts = provider_start_count() - 1;
    let mismatch_children = usize::from(mismatch_spawn.exists());
    assert_eq!(mismatch_adapter_starts, 0);
    assert_eq!(mismatch_children, 0);
}
