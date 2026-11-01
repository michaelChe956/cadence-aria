//! Task 9a(REQ-ENV-04 kimi 补齐):LC 显式 resume 的 durable audit 门测试。
//! 从 session_tests.rs 拆出以满足 large_file_guard 的登记上限(纯移动,
//! 行为不变):匹配续接 + legacy/漂移零 child 拒绝 + 无 MCP bundle 的生产
//! 构造面。
use crate::cross_cutting::kimi_code_provider::projection;
use crate::cross_cutting::kimi_code_provider::projection::{
    KIMI_LC_APPROVAL_POLICY, KIMI_NATIVE_MCP_SOURCE, KimiPolicyProjector,
};
use crate::cross_cutting::kimi_code_provider::{KIMI_POLICY_DIALECT, KimiCodeProvider};
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderPermissionMode, StreamingProviderAdapter as _,
};
use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
use crate::cross_cutting::tool_policy_audit::{
    DurableToolPolicyEvent, LcProjectionAudit, ProviderStartAudit,
};
use crate::product::logical_codebase::provider_gateway::ProviderRef;
use crate::product::logical_codebase::provider_projection::{
    ProviderPolicyProjector as _, ProviderProjectionInput,
};
use crate::protocol::contracts::AdapterRole;
use tokio_util::sync::CancellationToken;

use super::session_tests::{LcKimiResumeFixture, lc_kimi_resume_fixture, terminal_events};

/// Task 9a 期望投影:与 adapter 裸 validate(`lc_projection=None`)分支同源
/// 装配——同一 envelope/capability ref/role/permission/approval 策略/native
/// MCP 来源(`KimiCodeProvider::new` 无 Aria bundle)/boundary,因此投影摘要
/// 与 adapter 侧算出的字面量逐字可比。
#[cfg(unix)]
fn t09a_kimi_expected_lc_projection(
    fixture: &LcKimiResumeFixture,
    exact_version: &str,
) -> crate::product::logical_codebase::provider_projection::ProviderPolicyProjection {
    let validated = fixture
        .gateway()
        .validate(fixture.coding_request())
        .expect("lc coding launch validates");
    let envelope = validated.envelope().clone();
    let boundary = projection::lc_boundary_plan(&envelope).expect("kimi boundary plan");
    let projection_input = ProviderProjectionInput::new(
        envelope.clone(),
        ProviderRef::kimi_code(validated.capability_snapshot_ref()),
        envelope.action,
        AdapterRole::Executor,
        ProviderPermissionMode::Auto,
        None,
        KIMI_LC_APPROVAL_POLICY.to_string(),
        KIMI_NATIVE_MCP_SOURCE.to_string(),
        envelope.config_artifact_ref.clone(),
        String::new(),
        Some(boundary),
    );
    KimiPolicyProjector::new(exact_version)
        .project(&projection_input)
        .expect("expected kimi lc projection")
}

/// Task 9a 存档 provider_start(与 adapter 写侧同形).
#[cfg(unix)]
fn t09a_kimi_stored_record(
    native_id: &str,
    lc_projection: Option<LcProjectionAudit>,
) -> ProviderStartAudit {
    ProviderStartAudit {
        provider: "kimi-code".to_string(),
        role: "executor".to_string(),
        workspace_session_id: "ws-test".to_string(),
        provider_session_id: native_id.to_string(),
        tool_policy_canonical_digest: "sha256:seed".to_string(),
        argv: Vec::new(),
        sandbox: None,
        approval_policy: None,
        provider_version: "kimi 0.34.0".to_string(),
        adapter_dialect: KIMI_POLICY_DIALECT.to_string(),
        lc_projection,
    }
}

/// Task 9a kimi 补齐(REQ-ENV-04;与 codex/pi/claude 三家同形):LC 显式
/// resume 在 child 前比较全 LC audit(完整字面量含投影)。匹配的完整 LC
/// 存档照常续接(native id 保持、child 正常启动);旧 audit 缺 projection
/// 或投影摘要漂移都拒绝 resume——拒绝发生在 spawn 之前(零 child、零新
/// provider_start),漂移存档追加 superseded 终止审计;绝不在 adapter 内
/// 清 resume id 后静默 fresh。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t09a_kimi_lc_resume_audit_gate_zero_child_on_legacy_or_drift() {
    let fixture = LcKimiResumeFixture::new();
    let expected = t09a_kimi_expected_lc_projection(&fixture, "kimi 0.34.0");
    let matching_lc_projection = LcProjectionAudit {
        action: projection::action_text(expected.action()).to_string(),
        wire_dialect: projection::wire_dialect_text(expected.wire_dialect()).to_string(),
        capability_projection_digest: expected.capability_projection_digest().to_string(),
        projection_digest: expected.projection_digest().to_string(),
        boundary_evidence_ref: expected.boundary_evidence_ref().to_string(),
    };
    let resume_native_id = "kimi-session-resume-t09a";

    // 1)匹配的完整 LC 存档 → 续接:session/load 确认同 id,child 正常启动,
    //    provider_start 落确认 id 且携带 lc_projection。
    {
        let marker_dir = tempfile::tempdir().expect("t09a matching marker dir");
        let pid_marker = marker_dir.path().join("kimi-t09a-matching.pid");
        let sink = RecordingToolPolicyAuditSink::new();
        sink.with_stored_provider_start(t09a_kimi_stored_record(
            resume_native_id,
            Some(matching_lc_projection.clone()),
        ));
        let mut raw = fixture.lc_streaming_input(
            Some(sink.clone().bound()),
            Some(resume_native_id.to_string()),
        );
        raw.env_vars.insert(
            "KIMI_LOAD_SESSION_ID".to_string(),
            resume_native_id.to_string(),
        );
        raw.env_vars.insert(
            "KIMI_PID_MARKER".to_string(),
            pid_marker.display().to_string(),
        );
        let provider = KimiCodeProvider::new(lc_kimi_resume_fixture(marker_dir.path()));
        // 生产构造面(无 Aria 注入 bundle):MCP 来源为 native 标记,因此
        // "匹配"只能由 durable LC 投影审计成立,不可能由 MCP digest 兜底。
        assert_eq!(provider.lc_mcp_bundle_digest(), KIMI_NATIVE_MCP_SOURCE);
        let mut session = provider
            .start_validated(
                fixture.validated_coding_input(raw),
                CancellationToken::new(),
            )
            .await
            .expect("matching full LC audit must resume the native session");
        assert_eq!(session.native_session_id.as_deref(), Some(resume_native_id));
        // 续接确实启动了 child(正向观测:fixture 在 ACP 循环入口登记 pid)。
        assert!(
            t09c_child_registered(&pid_marker).await,
            "matching LC audit must spawn the kimi child"
        );
        let events = sink.events();
        assert_eq!(
            events.len(),
            1,
            "resumed run writes exactly one provider_start"
        );
        assert!(matches!(
            &events[0],
            DurableToolPolicyEvent::ProviderStart(record)
                if record.provider_session_id == resume_native_id
                    && record.lc_projection.is_some()
        ));
        let terminal = terminal_events(&mut session).await;
        let completion = terminal
            .iter()
            .find_map(|event| match event {
                ProviderEvent::Completed(completion) => Some(completion.clone()),
                _ => None,
            })
            .expect("the resumed kimi session completes");
        assert_eq!(completion.full_output, "lc resume done");
    }

    // 2)旧 audit 缺 projection → 不能 resume LC;投影摘要漂移同理:零 child、
    //    零新 provider_start,不清 id 转 fresh,旧 run 被标记 superseded。
    for (case, record) in [
        ("legacy", t09a_kimi_stored_record(resume_native_id, None)),
        (
            "drifted",
            t09a_kimi_stored_record(
                resume_native_id,
                Some(LcProjectionAudit {
                    projection_digest: "sha256:session-projection-drifted".to_string(),
                    ..matching_lc_projection.clone()
                }),
            ),
        ),
    ] {
        let marker_dir = tempfile::tempdir().expect("t09a reject marker dir");
        let pid_marker = marker_dir.path().join(format!("kimi-t09a-{case}.pid"));
        let sink = RecordingToolPolicyAuditSink::new();
        sink.with_stored_provider_start(record);
        let mut raw = fixture.lc_streaming_input(
            Some(sink.clone().bound()),
            Some(resume_native_id.to_string()),
        );
        raw.env_vars.insert(
            "KIMI_LOAD_SESSION_ID".to_string(),
            resume_native_id.to_string(),
        );
        raw.env_vars.insert(
            "KIMI_PID_MARKER".to_string(),
            pid_marker.display().to_string(),
        );
        let provider = KimiCodeProvider::new(lc_kimi_resume_fixture(marker_dir.path()));

        let rejected = match provider
            .start_validated(
                fixture.validated_coding_input(raw),
                CancellationToken::new(),
            )
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("legacy or drifted LC audit must refuse resume ({case})"),
        };
        assert!(
            rejected.details.contains("resume audit"),
            "rejection must name the resume audit gate: {rejected:?}"
        );
        // 零 child:拒绝发生在 spawn 之前(pid 标记永不出现)。
        assert!(
            !pid_marker.exists(),
            "no kimi child may spawn after a refused LC resume ({case})"
        );
        // 零新 provider_start;legacy 与漂移存档同样被标记 superseded(旧 run
        // 追加终止审计,不启动 fresh provider_start)。
        let events = sink.events();
        assert_eq!(
            events.len(),
            1,
            "{case} record is marked superseded without a fresh provider_start"
        );
        assert!(
            matches!(
                &events[0],
                DurableToolPolicyEvent::SessionTerminated(terminated)
                    if terminated.reason_code == "superseded_policy_drift"
            ),
            "{case} resume must mark the old run superseded"
        );
    }
}

/// 轮询确认 fixture 已登记 pid(正向观测:child 真的被启动)。
#[cfg(unix)]
async fn t09c_child_registered(pid_marker: &std::path::Path) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if std::fs::read_to_string(pid_marker)
            .ok()
            .and_then(|content| content.trim().parse::<u32>().ok())
            .is_some()
        {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Task 9a kimi 补齐(REQ-ENV-04,生产构造面):`KimiCodeProvider::new`
/// 不注入 Aria MCP bundle,`session/load` 的 bundle digest 校验因此惰性
/// (来源恒 native 标记);这正是旧缺口退化为「只验 MCP」的成因。本测试
/// 钉住:无 bundle 时 LC 显式 resume 仍受 durable `provider_start`
/// `lc_projection` 审计门约束——存档记录来自旧 exact version(裸 validate
/// 路径没有 frozen full projection 的版本复验),投影摘要漂移 ⇒ spawn 前
/// typed 拒绝、零 child、旧 run 标记 superseded。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t09b_kimi_resume_gate_without_mcp_bundle_matches_three_provider_contract() {
    let fixture = LcKimiResumeFixture::new();
    let resume_native_id = "kimi-session-resume-t09b";
    // 旧 run 的存档投影:同一 facts 但 exact version 不同(真实升级场景)。
    let legacy_version_projection = t09a_kimi_expected_lc_projection(&fixture, "kimi 0.33.0");
    let legacy_lc_projection = LcProjectionAudit {
        action: projection::action_text(legacy_version_projection.action()).to_string(),
        wire_dialect: projection::wire_dialect_text(legacy_version_projection.wire_dialect())
            .to_string(),
        capability_projection_digest: legacy_version_projection
            .capability_projection_digest()
            .to_string(),
        projection_digest: legacy_version_projection.projection_digest().to_string(),
        boundary_evidence_ref: legacy_version_projection
            .boundary_evidence_ref()
            .to_string(),
    };
    let current_projection = t09a_kimi_expected_lc_projection(&fixture, "kimi 0.34.0");
    assert_ne!(
        legacy_lc_projection.projection_digest,
        current_projection.projection_digest(),
        "the fixture must exercise a real projection drift"
    );

    let marker_dir = tempfile::tempdir().expect("t09b marker dir");
    let pid_marker = marker_dir.path().join("kimi-t09b.pid");
    let sink = RecordingToolPolicyAuditSink::new();
    sink.with_stored_provider_start(t09a_kimi_stored_record(
        resume_native_id,
        Some(legacy_lc_projection),
    ));
    let mut raw = fixture.lc_streaming_input(
        Some(sink.clone().bound()),
        Some(resume_native_id.to_string()),
    );
    raw.env_vars.insert(
        "KIMI_LOAD_SESSION_ID".to_string(),
        resume_native_id.to_string(),
    );
    raw.env_vars.insert(
        "KIMI_PID_MARKER".to_string(),
        pid_marker.display().to_string(),
    );
    let provider = KimiCodeProvider::new(lc_kimi_resume_fixture(marker_dir.path()));
    // 生产构造:零 Aria 注入 ⇒ MCP 来源为 native 标记(session/load 的
    // bundle digest 校验惰性),拒绝只能来自 durable LC 审计门。
    assert_eq!(provider.lc_mcp_bundle_digest(), KIMI_NATIVE_MCP_SOURCE);

    let rejected = match provider
        .start_validated(
            fixture.validated_coding_input(raw),
            CancellationToken::new(),
        )
        .await
    {
        Err(error) => error,
        Ok(_) => panic!("an exact-version drift must refuse resume without an MCP bundle"),
    };
    assert!(
        rejected.details.contains("resume audit"),
        "rejection must name the durable resume audit gate, not the MCP bundle: {rejected:?}"
    );
    assert!(
        !rejected.details.contains("mcp"),
        "the rejection must not come from the inert MCP bundle check: {rejected:?}"
    );
    assert!(
        !pid_marker.exists(),
        "no kimi child may spawn after a refused LC resume (production constructor)"
    );
    let events = sink.events();
    assert_eq!(
        events.len(),
        1,
        "the superseded old run is marked without any fresh provider_start"
    );
    assert!(
        matches!(
            &events[0],
            DurableToolPolicyEvent::SessionTerminated(terminated)
                if terminated.reason_code == "superseded_policy_drift"
        ),
        "the rejected resume must mark the old run superseded"
    );
}
