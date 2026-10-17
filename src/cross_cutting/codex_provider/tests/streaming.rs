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
            | ProviderEvent::ToolPolicyTerminated(_)
            | ProviderEvent::TerminalActivity(_) => {}
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

/// T12(REQ-LCG-04 改版):LC Planning/Review 的真实 RPC params 固定
/// `sandbox=danger-full-access`;approval 按权限模式独立映射(Author 岗位
/// 纪律恒 on-request,全开角色 Auto→never/Supervised→on-request);协议
/// cwd=canonical root(raw working_dir=target 不覆盖);进程 cwd=canonical
/// root;provider_start 审计带统一 `lc_projection`。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t12_planning_review_wire_is_danger_full_access() {
    let fixture = LcCodexFixture::new();
    let canonical_root = fixture.canonical_root();
    let target = fixture.target_worktree();
    let deny = ProviderToolPolicy::deny_file_write_builtins();

    let cases: [(
        SessionPolicyAction,
        AdapterRole,
        Option<ProviderToolPolicy>,
        ProviderPermissionMode,
        &str,
    ); 3] = [
        // Author 岗位纪律(Orchestrator 承载 deny)恒 on-request。
        (
            SessionPolicyAction::PlanningReadOnly,
            AdapterRole::Orchestrator,
            Some(deny),
            ProviderPermissionMode::Auto,
            "on-request",
        ),
        // Reviewer 全开角色(T20 起携 ReviewerVerify 窗口合同):审批独立按
        // 权限模式映射;物理面恒 danger。
        (
            SessionPolicyAction::ReviewReadOnly,
            AdapterRole::Reviewer,
            Some(ProviderToolPolicy::reviewer_verify("w-t20-lc-review")),
            ProviderPermissionMode::Auto,
            "never",
        ),
        (
            SessionPolicyAction::ReviewReadOnly,
            AdapterRole::Reviewer,
            Some(ProviderToolPolicy::reviewer_verify("w-t20-lc-review")),
            ProviderPermissionMode::Supervised,
            "on-request",
        ),
    ];

    for (action, role, tool_policy, permission_mode, expected_approval) in cases {
        let sink = RecordingToolPolicyAuditSink::new();
        let marker_root = tempfile::tempdir().expect("planning marker dir").keep();
        // raw working_dir 故意=target:证明协议 cwd 来自投影(root)而非 raw。
        let mut raw = fixture.lc_input(
            role.clone(),
            tool_policy.clone(),
            permission_mode.clone(),
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
            .unwrap_or_else(|error| {
                panic!("{action:?} lc danger launch must start: {error}")
            });

        assert_eq!(
            session.native_session_id.as_deref(),
            Some("codex-thread-lc")
        );
        assert_eq!(
            recv_completed(&mut session.events).await,
            "lc restricted done"
        );

        // 真实 RPC params:来自 fixture 捕获的 thread/start 请求原文。
        let params = lc_wire_params(&wire_marker);
        assert_eq!(params["sandbox"], "danger-full-access");
        assert_eq!(params["approvalPolicy"], expected_approval);
        assert_eq!(
            params["cwd"].as_str(),
            Some(canonical_root.to_string_lossy().as_ref()),
            "protocol cwd must be the projection's canonical root, not raw working_dir"
        );

        // 进程 cwd:canonical LC root。
        assert_eq!(
            std::fs::read_to_string(&cwd_marker)
                .expect("cwd marker is written")
                .trim(),
            canonical_root.to_string_lossy().as_ref()
        );

        // 统一 launch audit:sandbox/approvalPolicy 与 wire 同源;lc_projection
        // 必填(read-only action 的 boundary 引用为空串)。
        let events = sink.events();
        assert_eq!(events.len(), 1, "exactly one provider_start is written");
        let crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ProviderStart(record) =
            &events[0]
        else {
            panic!("expected provider_start");
        };
        assert_eq!(record.provider, "codex");
        assert_eq!(record.sandbox.as_deref(), Some("danger-full-access"));
        assert_eq!(record.approval_policy.as_deref(), Some(expected_approval));
        let lc_projection = record.lc_projection.as_ref().expect("lc projection audit");
        assert_eq!(lc_projection.action, lc_action_text(action));
        assert!(
            lc_projection.boundary_evidence_ref.is_empty(),
            "read-only action has no write face to reference"
        );
    }
}

/// T12(REQ-LCG-04 改版):LC Coding 的真实 RPC params 固定
/// `sandbox=danger-full-access` + 协议 cwd=target(投影覆盖 raw),进程
/// cwd=canonical root(双 cwd 合同);写面非 target-only(旧受限权限证明门
/// 废止)同样以 danger 放行——旧拒绝不复存在。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t12_coding_wire_is_danger_full_access_with_dual_cwd() {
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
        .expect("valid danger coding lc launch succeeds");

    assert_eq!(
        session.native_session_id.as_deref(),
        Some("codex-thread-lc")
    );
    assert_eq!(
        recv_completed(&mut session.events).await,
        "lc restricted done"
    );

    // 真实 RPC params:danger-full-access + 协议 cwd=target。
    let coding_params = lc_wire_params(&wire_marker);
    assert_eq!(coding_params["sandbox"], "danger-full-access");
    assert_eq!(
        coding_params["cwd"].as_str(),
        Some(target.to_string_lossy().as_ref()),
        "protocol cwd must be the projection's target, not raw working_dir"
    );
    assert_eq!(
        coding_params["approvalPolicy"], "never",
        "Auto→never 独立审批映射"
    );

    // 进程 cwd:canonical root,双 cwd 合同与协议 target 分离。
    let process_cwd = std::fs::read_to_string(&cwd_marker)
        .expect("cwd marker is written")
        .trim()
        .to_string();
    assert_eq!(process_cwd, canonical_root.to_string_lossy().as_ref());

    // 统一 launch audit:lc_projection 必填且 Coding 的 boundary 引用非空
    //(逻辑交付面引用;物理 sandbox 恒 danger)。
    let events = sink.events();
    let crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ProviderStart(record) =
        &events[0]
    else {
        panic!("expected provider_start");
    };
    assert_eq!(record.sandbox.as_deref(), Some("danger-full-access"));
    assert_eq!(record.approval_policy.as_deref(), Some("never"));
    let lc_projection = record.lc_projection.as_ref().expect("lc projection audit");
    assert_eq!(lc_projection.action, "coding_target_write");
    assert!(!lc_projection.boundary_evidence_ref.is_empty());

    // 写面非 target-only(旧受限权限证明门废止):同样以 danger 放行,
    // 不再被 `codex_danger_full_access_unsupported` 拒绝(独立 run-bound
    // sink——provider_start 首行唯一,不复用首启绑定)。
    let second_sink = RecordingToolPolicyAuditSink::new();
    let mut second_raw = fixture.lc_input(
        AdapterRole::Executor,
        None,
        ProviderPermissionMode::Auto,
        None,
        Some(second_sink.bound()),
        canonical_root.clone(),
    );
    let (_second_cwd, _second_wire, _second_spawn) = lc_markers(&mut second_raw, &marker_root);
    let mut non_target_only = provider
        .start_lc_validated(
            second_raw,
            &fixture.envelope(
                SessionPolicyAction::CodingTargetWrite,
                vec![target.clone(), canonical_root.clone()],
            ),
            "cap_codex_lc_fixture",
            CancellationToken::new(),
        )
        .await
        .expect("non-target-only coding write face must launch danger (retired gate)");
    assert_eq!(
        recv_completed(&mut non_target_only.events).await,
        "lc restricted done"
    );
    let wire = std::fs::read_to_string(&wire_marker).expect("wire marker is rewritten");
    assert!(
        wire.contains("\"sandbox\":\"danger-full-access\""),
        "non-target-only danger launch wires danger-full-access: {wire}"
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
            Some(lc_boundary_plan(&envelope)),
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
    // sandbox 投影冻结面逐字段:mode=danger-full-access(全相位基线),
    // target_root 跟随漂移 target,Coding 的 boundary 引用非空(计划内容
    // 引用;真实 probe evidence 归 6c/2d)。
    assert_eq!(drifted_sandbox.mode().wire_text(), "danger-full-access");
    assert_eq!(drifted_sandbox.target_root(), target_b.as_path());
    assert!(!drifted_sandbox.boundary_evidence_ref().is_empty());
}

/// T12(REQ-LCG-04 改版):exact version 不可得时 LC 启动仍 fail-closed
/// 拒绝(缺版本=0 会话),但不再以废止的 `codex_danger_full_access_unsupported`
/// danger 稳定码拒绝——danger 是当前合法基线,版本不可得是普通准入失败。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t12_unknown_version_fails_closed_zero_child_without_danger_code() {
    let fixture = LcCodexFixture::new();
    let sink = RecordingToolPolicyAuditSink::new();
    let marker_root = tempfile::tempdir().expect("version marker dir").keep();
    let mut raw = fixture.lc_input(
        AdapterRole::Executor,
        None,
        ProviderPermissionMode::Auto,
        None,
        Some(sink.clone().bound()),
        fixture.canonical_root(),
    );
    let (_cwd, _wire, spawn_marker) = lc_markers(&mut raw, &marker_root);
    // supplier 失败:exact version 不可得(supplier seam 优先于真实探测)。
    let provider =
        CodexProvider::new(lc_app_server_fixture()).with_version_supplier(std::sync::Arc::new(
            || Err(crate::cross_cutting::streaming_provider::VersionProbeError::Unavailable),
        ));

    let Err(error) = provider
        .start_lc_validated(
            raw,
            &fixture.envelope(
                SessionPolicyAction::CodingTargetWrite,
                vec![fixture.target_worktree()],
            ),
            "cap_codex_lc_fixture",
            CancellationToken::new(),
        )
        .await
    else {
        panic!("launch with unknown version must still be rejected (fail-closed)");
    };
    assert!(
        error.details.contains("codex lc validated start"),
        "version-unavailable rejection keeps the LC validated start failure surface: {error:?}"
    );
    assert!(
        !error.details.contains("codex_danger_full_access_unsupported"),
        "legal danger baseline must not be judged unsupported: {error:?}"
    );
    assert!(
        !spawn_marker.exists(),
        "no codex child may spawn when the version cannot be observed"
    );
    assert!(sink.events().is_empty());
}

/// Task 5b 受限门深化:gateway 提供的 boundary plan 与 envelope 冻结面不符
/// 时,投影以稳定码 `codex_target_boundary_unverified` 拒绝(不静默采纳
/// 外部 plan,不回退 danger/换 cwd/换 provider)。
#[test]
fn lcg_t05_mismatched_boundary_plan_reports_target_boundary_unverified() {
    let fixture = LcCodexFixture::new();
    let target = fixture.target_worktree();
    let mut envelope =
        fixture.envelope(SessionPolicyAction::CodingTargetWrite, vec![target.clone()]);
    envelope.target = PolicyTarget::checkout("logical_repo_0001", "checkout_0001", target.clone());

    // 形状不符的 provided plan:mode 不是 target-write-only(read-only plan
    // 携带 target),不得替代 envelope 派生。
    let mismatched = crate::cross_cutting::provider_boundary::ProviderBoundaryPlan::new(
        crate::cross_cutting::provider_boundary::ProviderBoundaryMode::ReadOnly,
        fixture.canonical_root(),
        Some(target.clone()),
        Vec::new(),
    );
    let projection_input = ProviderProjectionInput::new(
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
        Some(mismatched),
    );

    let error = LcProjector::new("codex 0.124.0-lc-fixture")
        .project(&projection_input)
        .expect_err("mismatched boundary plan must be rejected");
    assert!(
        error
            .to_string()
            .contains("codex_target_boundary_unverified"),
        "boundary mismatch must carry the stable reason, got: {error}"
    );
}


/// T12 wire 深化:LC danger 会话的 thread/resume 与 thread/start 共享同一
/// 投影——resume 请求在真实 wire 上携带同 sandbox/approvalPolicy/cwd 与
/// 原 threadId,并由应答确认(F1);Author 承载角色(Orchestrator+deny)
/// 的 approval 恒 on-request。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t12_thread_resume_shares_danger_projection_on_wire() {
    let fixture = LcCodexFixture::new();
    let canonical_root = fixture.canonical_root();
    let sink = RecordingToolPolicyAuditSink::new();
    let marker_root = tempfile::tempdir().expect("resume marker dir").keep();
    let mut raw = fixture.lc_input(
        AdapterRole::Orchestrator,
        Some(ProviderToolPolicy::deny_file_write_builtins()),
        ProviderPermissionMode::Auto,
        Some("codex-thread-lc".to_string()),
        Some(sink.clone().bound()),
        fixture.target_worktree(),
    );
    let (_cwd, wire_marker, _spawn_marker) = lc_markers(&mut raw, &marker_root);
    let provider =
        CodexProvider::new(lc_app_server_fixture()).with_version_supplier(lc_version_supplier());

    let mut session = provider
        .start_lc_validated(
            raw,
            &fixture.envelope(SessionPolicyAction::PlanningReadOnly, Vec::new()),
            "cap_codex_lc_fixture",
            CancellationToken::new(),
        )
        .await
        .expect("lc danger resume launch succeeds");

    // F1:原生 thread id 只能来自 thread/resume 应答确认。
    assert_eq!(
        session.native_session_id.as_deref(),
        Some("codex-thread-lc")
    );
    assert_eq!(
        recv_completed(&mut session.events).await,
        "lc restricted done"
    );

    // 真实 wire:thread/resume 与 thread/start 共享同投影 params。
    let resume_request = std::fs::read_to_string(&wire_marker).expect("wire marker is written");
    assert!(
        resume_request.contains("thread/resume"),
        "resume launch must use thread/resume on the wire"
    );
    let resume_params = lc_wire_params(&wire_marker);
    assert_eq!(resume_params["sandbox"], "danger-full-access");
    assert_eq!(resume_params["approvalPolicy"], "on-request");
    assert_eq!(
        resume_params["cwd"].as_str(),
        Some(canonical_root.to_string_lossy().as_ref())
    );
    assert!(
        resume_request.contains("\"threadId\":\"codex-thread-lc\""),
        "thread/resume must carry the original thread id: {resume_request}"
    );
}

/// r19 续修回归(fix 轮 2,oracle 裁决三家同批收口):CLI 子进程在消费
/// initialize 写入后立即 kill -9,同组 `sleep 300` 持有管道写端使 EOF 无限
/// 推迟——会话泵对 initialize 应答的等待(r19 前形态)只能等 60s RPC 超时;
/// exit-watch 竞速必须使会话秒级以 Failed 终结(ProviderSession 终结保证)。
#[cfg(unix)]
#[tokio::test]
async fn codex_child_death_after_initialize_write_terminates_session_promptly() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_dead_child_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start");

    let failure = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            match session
                .events
                .recv()
                .await
                .expect("event channel stays open until terminal event")
            {
                ProviderEvent::Failed { message } => break message,
                _ => {}
            }
        }
    })
    .await
    .expect("child exit must terminate the codex session stream within seconds");
    assert!(
        !failure.is_empty(),
        "terminal failure must carry a cause message"
    );
}
