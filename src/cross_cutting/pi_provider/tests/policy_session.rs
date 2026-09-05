// Task 3.2（REQ-ENV-09/D7）：pi 策略会话的有界握手（id 预生成/传入）、
// provider_start durable 写入、append 失败 kill 链与缺 sink fail-closed 测试。

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use tokio_util::sync::CancellationToken;

use crate::cross_cutting::streaming_provider::{
    ProviderPermissionMode, ProviderToolPolicy, ProviderVersionSupplier, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
use crate::cross_cutting::tool_policy_audit::{DurableToolPolicyEvent, ToolPolicyAuditSink};
use crate::protocol::contracts::{AdapterRole, ProviderType};

use super::*;

fn policy_version_supplier() -> ProviderVersionSupplier {
    std::sync::Arc::new(|| Ok("pi 0.83.0-policy-fixture".to_string()))
}

fn policy_pi_input(
    resume_id: Option<String>,
    audit_sink: Option<std::sync::Arc<dyn ToolPolicyAuditSink>>,
) -> StreamingProviderInput {
    let mut input = streaming_input_for_test(resume_id);
    input.role = AdapterRole::Orchestrator;
    input.tool_policy = Some(ProviderToolPolicy::deny_file_write_builtins());
    input.audit_sink = audit_sink;
    input
}

/// fake pi：--version 打印兼容版本；rpc 模式下登记 PID（kill 链断言）并读 stdin
/// 到 EOF 后退出。
#[cfg(unix)]
fn policy_pi_fixture(marker: &std::path::Path) -> PathBuf {
    write_executable(
        marker.parent().expect("marker parent"),
        "fake-policy-pi",
        &format!(
            "if [ \"$1\" = \"--version\" ]; then echo 0.83.0; exit 0; fi\necho $$ > {}\nwhile IFS= read -r line; do :; done",
            marker.display()
        ),
    )
}

fn plain_policy_pi_fixture() -> PathBuf {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = write_executable(
        temp.path(),
        "fake-policy-pi-plain",
        "if [ \"$1\" = \"--version\" ]; then echo 0.83.0; exit 0; fi\nwhile IFS= read -r line; do :; done",
    );
    let _ = temp.keep();
    path
}

#[cfg(unix)]
fn assert_process_exits(pid: u32) {
    let deadline = std::time::Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        let alive = std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !alive {
            return;
        }
        assert!(
            start.elapsed() < deadline,
            "policy session child (pid {pid}) must be terminated after append failure"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn read_marker_pid(marker: &std::path::Path) -> Option<u32> {
    // 等待 fixture 登记 PID；append 失败 kill 可能快于 fixture 写 marker——
    // 此时子进程已死（marker 永不出现），同样证明 kill 链生效。
    let deadline = std::time::Duration::from_secs(2);
    let start = std::time::Instant::now();
    loop {
        if let Ok(content) = std::fs::read_to_string(marker)
            && let Ok(pid) = content.trim().parse::<u32>()
        {
            return Some(pid);
        }
        if start.elapsed() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn pi_policy_start_pregenerates_session_id_and_writes_provider_start() {
    let sink = RecordingToolPolicyAuditSink::new();
    let provider =
        PiProvider::new(plain_policy_pi_fixture()).with_version_supplier(policy_version_supplier());
    let input = policy_pi_input(None, Some(sink.clone().bound()));

    let session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("pi policy session starts");

    // pi 握手 = id 预生成：fresh 策略会话必须返回预生成的 native session id。
    let native_id = session
        .native_session_id
        .clone()
        .expect("pi policy session must carry a pre-generated native session id");
    assert!(!native_id.trim().is_empty());

    // provider_start：首条 durable 事件，含冻结 argv（--session-id + denylist）、
    // digest、version（supplier 注入）与 dialect。
    let events = sink.events();
    assert_eq!(events.len(), 1, "only provider_start is written at start");
    let DurableToolPolicyEvent::ProviderStart(record) = &events[0] else {
        panic!("expected provider_start");
    };
    assert_eq!(record.provider, "pi");
    assert_eq!(record.dialect, PI_POLICY_DIALECT);
    assert_eq!(record.provider_version, "pi 0.83.0-policy-fixture");
    assert_eq!(record.native_session_id, native_id);
    assert!(record.argv.contains(&"--session-id".to_string()));
    assert!(record.argv.contains(&native_id));
    assert!(record.argv.contains(&"--exclude-tools".to_string()));
    assert!(record.argv.contains(&"edit,write".to_string()));
    assert!(!record.tool_policy_digest.is_empty());
    assert_eq!(record.sandbox, None);
    assert_eq!(record.approval_policy, None);
}

#[cfg(unix)]
#[tokio::test]
async fn pi_policy_start_reuses_resume_session_id_as_native_id() {
    let sink = RecordingToolPolicyAuditSink::new();
    let provider =
        PiProvider::new(plain_policy_pi_fixture()).with_version_supplier(policy_version_supplier());
    let input = policy_pi_input(
        Some("pi-session-resume-policy".to_string()),
        Some(sink.clone().bound()),
    );

    let session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("pi policy resume session starts");
    assert_eq!(
        session.native_session_id.as_deref(),
        Some("pi-session-resume-policy")
    );
    let DurableToolPolicyEvent::ProviderStart(record) = &sink.events()[0] else {
        panic!("expected provider_start");
    };
    assert_eq!(record.native_session_id, "pi-session-resume-policy");
}

#[cfg(unix)]
#[tokio::test]
async fn pi_policy_start_without_sink_or_version_fails_closed() {
    let provider = PiProvider::new(plain_policy_pi_fixture());
    // 缺 sink：fail-closed。
    let Err(error) = provider
        .start(policy_pi_input(None, None), CancellationToken::new())
        .await
    else {
        panic!("policy session without sink must fail closed");
    };
    assert!(
        error.details.contains("audit sink is required"),
        "unexpected error: {}",
        error.details
    );

    // 缺 version supplier（3.2 无默认探测）：fail-closed。
    let provider = PiProvider::new(plain_policy_pi_fixture());
    let Err(error) = provider
        .start(
            policy_pi_input(None, Some(RecordingToolPolicyAuditSink::new().bound())),
            CancellationToken::new(),
        )
        .await
    else {
        panic!("policy session without version must fail closed");
    };
    assert!(
        error.details.contains("version is unavailable"),
        "unexpected error: {}",
        error.details
    );
}

#[cfg(unix)]
#[tokio::test]
async fn pi_policy_start_append_failure_kills_child_and_fails_closed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let marker = temp.path().join("policy-pi.pid");
    let provider = PiProvider::new(policy_pi_fixture(&marker))
        .with_version_supplier(policy_version_supplier());
    // 首次 append 即失败（provider_start 写入失败）。
    let sink = RecordingToolPolicyAuditSink::failing_after(0);

    let Err(error) = provider
        .start(
            policy_pi_input(None, Some(sink.clone().bound())),
            CancellationToken::new(),
        )
        .await
    else {
        panic!("provider_start append failure must fail the session");
    };
    assert!(
        error.details.contains("provider_start audit append failed"),
        "unexpected error: {}",
        error.details
    );

    // adapter 在返回前终止子进程（kill 链）：marker 已写则等待进程退出；
    // marker 未写则子进程在登记前即被终止。
    if let Some(pid) = read_marker_pid(&marker) {
        assert_process_exits(pid);
    }
}

#[test]
fn pi_policy_input_fixture_keeps_env_vars_bounded() {
    // 静态守卫：测试 helper 不携带环境注入（避免 fixture 漂移）。
    let input = policy_pi_input(None, None);
    assert_eq!(input.env_vars, BTreeMap::new());
    assert_eq!(input.timeout_secs, 60);
    assert!(input.permission_mode == ProviderPermissionMode::Auto);
    assert_eq!(input.provider_type, ProviderType::Pi);
    let _ = Ordering::SeqCst;
}
