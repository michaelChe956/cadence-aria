// 当前协议/流式族：agent message delta、completed-only 输出与命令执行
// 事件的 wire 级验证。
// 从 tests/mod.rs 拆出以满足 large_file_guard 的 1200 行上限。

use super::*;

#[tokio::test]
async fn codex_provider_handles_current_app_server_protocol_and_agent_message_delta() {
    let fixture = executable_fixture("tests/fixtures/provider/codex_app_server_current_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;

    assert!(completed.contains("# Story Spec"));
    assert!(completed.contains("## 功能需求"));
    assert!(completed.contains("## 成功标准"));
}

#[tokio::test]
async fn codex_provider_streams_completed_only_agent_messages() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_completed_only_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let mut saw_text_delta = false;
    let completed = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit completed-only text")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::TextDelta { content } => {
                assert_eq!(content, "Codex completed-only chunk");
                saw_text_delta = true;
            }
            ProviderEvent::Completed(completion) => break completion.full_output,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_) => {}
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    };

    assert!(saw_text_delta);
    assert_eq!(completed, "Codex completed-only chunk");
}

#[tokio::test]
async fn codex_provider_emits_command_execution_events_from_current_protocol() {
    let fixture = executable_fixture("tests/fixtures/provider/codex_app_server_current_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let mut saw_started = false;
    let mut saw_completed = false;
    for _ in 0..20 {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit execution events")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::Execution(event)
                if event.kind == ProviderExecutionEventKind::Command
                    && event.status == ProviderExecutionEventStatus::Started =>
            {
                assert_eq!(event.event_id, "command_cmd_001");
                assert_eq!(event.command.as_deref(), Some("pwd"));
                assert!(event.cwd.is_some());
                saw_started = true;
            }
            ProviderEvent::Execution(event)
                if event.kind == ProviderExecutionEventKind::Command
                    && event.status == ProviderExecutionEventStatus::Completed =>
            {
                assert_eq!(event.event_id, "command_cmd_001");
                assert_eq!(event.command.as_deref(), Some("pwd"));
                assert_eq!(event.exit_code, Some(0));
                assert!(event.output.as_deref().unwrap_or_default().contains('/'));
                saw_completed = true;
            }
            ProviderEvent::Completed(_) if saw_started && saw_completed => return,
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            _ => {}
        }
    }

    assert!(saw_started, "command started event was not emitted");
    assert!(saw_completed, "command completed event was not emitted");
}

use crate::cross_cutting::codex_provider::projection::{
    CodexPolicyProjector as LcProjector, action_text as lc_action_text, lc_boundary_plan,
};
use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
use crate::product::logical_codebase::provider_projection::ProviderPolicyProjector;
use crate::product::logical_codebase::provider_projection::ProviderProjectionInput;

/// Task 5 Step 1(断言组 325-326 + 329 只读档):LC Planning/Review 的真实
/// RPC params 固定 `sandbox=read-only` + `approvalPolicy=on-request`,协议
/// cwd=canonical root(raw working_dir=target 不覆盖);进程 cwd=canonical
/// root;provider_start 审计带统一 `lc_projection`。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t05_planning_review_wire_is_read_only_on_request() {
    let fixture = LcCodexFixture::new();
    let canonical_root = fixture.canonical_root();
    let target = fixture.target_worktree();

    for action in [
        SessionPolicyAction::PlanningReadOnly,
        SessionPolicyAction::ReviewReadOnly,
    ] {
        let sink = RecordingToolPolicyAuditSink::new();
        let marker_root = tempfile::tempdir().expect("planning marker dir").keep();
        // raw working_dir 故意=target:证明协议 cwd 来自投影(root)而非 raw。
        let mut raw = fixture.lc_input(
            AdapterRole::Reviewer,
            Some(ProviderToolPolicy::deny_file_write_builtins()),
            ProviderPermissionMode::Auto,
            None,
            Some(sink.clone().bound()),
            target.clone(),
        );
        let (cwd_marker, wire_marker, _spawn_marker) = lc_markers(&mut raw, &marker_root);
        let provider = CodexProvider::new(lc_app_server_fixture())
            .with_version_supplier(lc_version_supplier());

        let mut session = provider
            .start_lc_validated(
                raw,
                &fixture.envelope(action, Vec::new()),
                "cap_codex_lc_fixture",
                CancellationToken::new(),
            )
            .await
            .unwrap_or_else(|error| panic!("{action:?} lc launch must be restricted: {error}"));

        assert_eq!(
            session.native_session_id.as_deref(),
            Some("codex-thread-lc")
        );
        assert_eq!(
            recv_completed(&mut session.events).await,
            "lc restricted done"
        );

        // 真实 RPC params(断言组 325-326):来自 fixture 捕获的 thread/start
        // 请求原文。
        let readonly_params = lc_wire_params(&wire_marker);
        assert_eq!(readonly_params["sandbox"], "read-only");
        assert_eq!(readonly_params["approvalPolicy"], "on-request");
        assert_eq!(
            readonly_params["cwd"].as_str(),
            Some(canonical_root.to_string_lossy().as_ref()),
            "protocol cwd must be the projection's canonical root, not raw working_dir"
        );

        // 进程 cwd(断言组 329 只读档观测):canonical LC root。
        assert_eq!(
            std::fs::read_to_string(&cwd_marker)
                .expect("cwd marker is written")
                .trim(),
            canonical_root.to_string_lossy().as_ref()
        );

        // 统一 launch audit:sandbox/approvalPolicy 与 wire 同源;lc_projection
        // 必填(boundary 引用为空串=只读无写面)。
        let events = sink.events();
        assert_eq!(events.len(), 1, "exactly one provider_start is written");
        let crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ProviderStart(record) =
            &events[0]
        else {
            panic!("expected provider_start");
        };
        assert_eq!(record.provider, "codex");
        assert_eq!(record.sandbox.as_deref(), Some("read-only"));
        assert_eq!(record.approval_policy.as_deref(), Some("on-request"));
        let lc_projection = record.lc_projection.as_ref().expect("lc projection audit");
        assert_eq!(lc_projection.action, lc_action_text(action));
        assert!(
            lc_projection.boundary_evidence_ref.is_empty(),
            "read-only action has no write face to reference"
        );
    }
}

/// Task 5 Step 1(断言组 327-330):LC Coding 的真实 RPC params 固定
/// `sandbox=workspace-write` + 协议 cwd=target(投影覆盖 raw),进程 cwd=
/// canonical root(双 cwd 合同);target-only 写面证据缺失时拒绝。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t05_coding_requires_protocol_target_and_target_only_evidence() {
    let fixture = LcCodexFixture::new();
    let canonical_root = fixture.canonical_root();
    let target = fixture.target_worktree();
    let sink = RecordingToolPolicyAuditSink::new();
    let marker_root = tempfile::tempdir().expect("coding marker dir").keep();
    // raw working_dir 故意=root:证明协议 cwd 来自投影(target)而非 raw。
    let mut raw = fixture.lc_input(
        AdapterRole::Executor,
        None,
        ProviderPermissionMode::Auto,
        None,
        Some(sink.clone().bound()),
        canonical_root.clone(),
    );
    let (cwd_marker, wire_marker, _spawn_marker) = lc_markers(&mut raw, &marker_root);
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
        .expect("coding lc launch with target-only evidence succeeds");

    assert_eq!(
        session.native_session_id.as_deref(),
        Some("codex-thread-lc")
    );
    assert_eq!(
        recv_completed(&mut session.events).await,
        "lc restricted done"
    );

    // 真实 RPC params(断言组 327-328):workspace-write + 协议 cwd=target。
    let coding_params = lc_wire_params(&wire_marker);
    assert_eq!(coding_params["sandbox"], "workspace-write");
    assert_eq!(
        coding_params["cwd"].as_str(),
        Some(target.to_string_lossy().as_ref()),
        "protocol cwd must be the projection's target, not raw working_dir"
    );
    assert_eq!(
        coding_params["approvalPolicy"], "never",
        "Auto→never 冻结映射"
    );

    // 进程 cwd(断言组 329):canonical root,双 cwd 合同与协议 target 分离。
    let process_cwd = std::fs::read_to_string(&cwd_marker)
        .expect("cwd marker is written")
        .trim()
        .to_string();
    assert_eq!(process_cwd, canonical_root.to_string_lossy().as_ref());

    // 统一 launch audit:lc_projection 必填且 Coding 的 boundary 引用非空。
    let events = sink.events();
    let crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ProviderStart(record) =
        &events[0]
    else {
        panic!("expected provider_start");
    };
    assert_eq!(record.sandbox.as_deref(), Some("workspace-write"));
    assert_eq!(record.approval_policy.as_deref(), Some("never"));
    let lc_projection = record.lc_projection.as_ref().expect("lc projection audit");
    assert_eq!(lc_projection.action, "coding_target_write");
    assert!(!lc_projection.boundary_evidence_ref.is_empty());

    // 证据缺失(断言组 330):写面非 target-only(多出 root 可写)→ 拒绝,
    // 不得回退 danger-full-access。
    let missing_any_required_evidence = provider
        .start_lc_validated(
            fixture.lc_input(
                AdapterRole::Executor,
                None,
                ProviderPermissionMode::Auto,
                None,
                Some(sink.clone().bound()),
                canonical_root.clone(),
            ),
            &fixture.envelope(
                SessionPolicyAction::CodingTargetWrite,
                vec![target.clone(), canonical_root.clone()],
            ),
            "cap_codex_lc_fixture",
            CancellationToken::new(),
        )
        .await;
    assert!(
        missing_any_required_evidence.is_err(),
        "coding without target-only writable evidence must be rejected"
    );
}

/// Task 5 Step 1:version 漂移使 capability/会话双 digest 失效;协议 cwd
/// (target)漂移使会话全投影 digest 失效——旧证据不得继续 allow。
#[test]
fn lcg_t05_version_or_protocol_cwd_drift_invalidates_projection() {
    let fixture = LcCodexFixture::new();
    let target_a = fixture.target_worktree();
    let target_b = fixture.root.join("member-worktree-b");

    let projection_input = |target: PathBuf| {
        let mut envelope =
            fixture.envelope(SessionPolicyAction::CodingTargetWrite, vec![target.clone()]);
        // 协议 cwd 漂移变体:envelope target 跟随漂移 target(保持 target-only
        // 形状,只变 target 本身)。
        envelope.target =
            PolicyTarget::checkout("logical_repo_0001", "checkout_0001", target.clone());
        ProviderProjectionInput::new(
            envelope.clone(),
            ProviderRef::codex("cap_codex_lc_fixture"),
            SessionPolicyAction::CodingTargetWrite,
            AdapterRole::Executor,
            ProviderPermissionMode::Auto,
            None,
            String::new(),
            String::new(),
            "sha256:managed-config-artifact".to_string(),
            "sha256:trust-lc-fixture".to_string(),
            Some(lc_boundary_plan(&envelope).expect("target-only boundary plan derives")),
        )
    };

    let baseline = LcProjector::new("codex 0.124.0-lc-fixture")
        .project(&projection_input(target_a.clone()))
        .expect("coding projection is produced");
    let original_projection_digest = baseline.projection_digest().to_string();
    let original_capability_digest = baseline.capability_projection_digest().to_string();
    assert_eq!(original_projection_digest.len(), 71);
    assert!(original_projection_digest.starts_with("sha256:"));
    assert_eq!(original_capability_digest.len(), 71);
    assert!(original_capability_digest.starts_with("sha256:"));

    // 1) version 漂移 → profile 摘要与会话摘要都失效(exact version 属于
    //    完整权限 profile)。
    let drifted_version = LcProjector::new("codex 0.125.0-lc-fixture")
        .project(&projection_input(target_a.clone()))
        .expect("projection with drifted version");
    assert_ne!(
        original_capability_digest,
        drifted_version.capability_projection_digest()
    );
    assert_ne!(
        original_projection_digest,
        drifted_version.projection_digest()
    );

    // 2) 协议 cwd(target)漂移 → 会话全投影 digest 失效;投影的 protocol
    //    cwd 跟随新 target(双 cwd:进程 cwd 仍是 root)。
    let drifted_target = LcProjector::new("codex 0.124.0-lc-fixture")
        .project(&projection_input(target_b.clone()))
        .expect("projection with drifted protocol cwd");
    assert_ne!(
        original_projection_digest,
        drifted_target.projection_digest()
    );
    let drifted_sandbox =
        LcProjector::new("codex 0.124.0-lc-fixture").sandbox_projection(&drifted_target);
    assert_eq!(drifted_sandbox.protocol_cwd(), target_b.as_path());
    assert_eq!(
        drifted_sandbox.process_cwd(),
        fixture.canonical_root().as_path()
    );
    // sandbox 投影冻结面逐字段:mode=workspace-write,target_root 跟随漂移
    // target,Coding 的 boundary 引用非空(计划内容引用;真实 probe evidence
    // 归 6c/2d)。
    assert_eq!(drifted_sandbox.mode().wire_text(), "workspace-write");
    assert_eq!(drifted_sandbox.target_root(), target_b.as_path());
    assert!(!drifted_sandbox.boundary_evidence_ref().is_empty());
}
