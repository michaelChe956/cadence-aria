//! B1-1(波 2):Kimi adapter 的受信资源登记接线测试(先红后绿)。
//!
//! 覆盖:
//! - direct `start_owned` 真实子进程 spawn 后 store 出现 Process 登记(真
//!   PID+`/proc` starttime、scope/run/boot/worktree 与引擎 scope 同源);
//! - 预留失败(store 根不可写)→ 零 spawn(含 `--version` 探测);
//! - 无载体(裸 `start` 与 `start_owned(None)` 委托默认)零登记;
//! - 握手失败/会话错误路径在真实收口(子进程回收)后注销,组无残留;
//! - 在途轮次取消(`KIMI_SESSION_ABORTED` 链)在真实收口后注销,组无残留;
//! - 协议 `Completed` 不提前注销(子进程仍存活时登记仍在);
//! - validated 载体(`ValidatedStreamingProviderInput.launch_ownership`)同样
//!   登记与收口。

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::cross_cutting::process_manager::process_start_time_ticks;
use crate::cross_cutting::provider_resources::{
    ProviderResourceRecord, ProviderRunResourceScope, ResourceScopeKey, StartedIdentity,
    TrustedLaunchOwnership,
};
use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
use crate::product::provider_resource_store::ProviderResourceStore;
use crate::product::reviewer_window_store::tree_key_of;

use super::super::{KimiCodeProvider, StreamingProviderAdapter};
use super::session_tests::{LcKimiResumeFixture, fixture_command, input};

struct B1Env {
    _base: tempfile::TempDir,
    store_root: PathBuf,
    private_root: PathBuf,
}

fn b1_env() -> B1Env {
    let base = tempfile::tempdir().expect("b1 fixture dir");
    let store_root = base.path().join("resource-store");
    let private_root = base.path().join("private-root");
    std::fs::create_dir_all(&private_root).expect("create private root");
    B1Env {
        _base: base,
        store_root,
        private_root,
    }
}

fn b1_scope(
    private_root: &Path,
    store_root: &Path,
    suffix: &str,
) -> (TrustedLaunchOwnership, String) {
    let key = ResourceScopeKey {
        scope_id: format!("code-delivery-attempt-kimi-{suffix}"),
        run_id: format!("role-run-kimi-{suffix}"),
        boot_identity: format!("aria-engine-{}", std::process::id()),
    };
    let worktree = private_root.canonicalize().expect("canonical worktree");
    let tree_key = tree_key_of(&worktree);
    let scope =
        ProviderRunResourceScope::open_for_recovery(&key, None, private_root, store_root, None)
            .expect("open kimi b1 scope")
            .with_worktree_key(&tree_key);
    (
        TrustedLaunchOwnership::mint(Arc::new(scope), "code_delivery provider process: kimi b1"),
        tree_key,
    )
}

fn b1_records(store_root: &Path, key: &ResourceScopeKey) -> Vec<ProviderResourceRecord> {
    ProviderResourceStore::new(store_root.to_path_buf())
        .list_scope(key)
        .expect("list kimi b1 scope")
}

async fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
    for _ in 0..120 {
        if predicate() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

fn b1_identity(record: &ProviderResourceRecord) -> (u32, u64) {
    match record.started_identity.as_ref() {
        Some(StartedIdentity::Process {
            pid,
            start_time_ticks,
        }) => (*pid, *start_time_ticks),
        other => panic!("进程登记必须携带真实 PID+starttime: {other:?}"),
    }
}

/// 进程组残留扫描:按 `/proc/<pid>/stat` 的 pgrp(第 5 字段)枚举组成员。
fn group_members(pgid: u32) -> Vec<u32> {
    let mut members = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return members;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some((_, tail)) = stat.rsplit_once(") ") else {
            continue;
        };
        let parsed = tail
            .split_whitespace()
            .nth(2)
            .and_then(|value| value.parse::<u32>().ok());
        if parsed == Some(pgid) {
            members.push(pid);
        }
    }
    members
}

fn b1_input(
    mode: Option<&str>,
    timeout_secs: u64,
) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
    let mut input = input(None, timeout_secs);
    if let Some(mode) = mode {
        input
            .env_vars
            .insert("KIMI_B1_MODE".to_string(), mode.to_string());
    }
    input
}

/// F10(adapter 级):direct/legacy 真实 spawn 的子进程读到宿主 PATH/HOME
/// (fixture 落盘子进程实际环境,测试逐字比对,不靠 adapter 自述)。
fn assert_marker_has_host_environment(marker: &Path) {
    for _ in 0..200 {
        if marker.is_file() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let text = std::fs::read_to_string(marker)
        .unwrap_or_else(|error| panic!("read env marker {}: {error}", marker.display()));
    let host_home = format!("HOME={}", std::env::var("HOME").expect("host HOME"));
    let host_path = format!("PATH={}", std::env::var("PATH").expect("host PATH"));
    assert!(
        text.lines().any(|line| line == host_home),
        "F10:direct/legacy 子进程必须读到宿主 HOME(实测 {text:?})"
    );
    assert!(
        text.lines().any(|line| line == host_path),
        "F10:direct/legacy 子进程必须读到宿主 PATH(实测 {text:?})"
    );
}

/// direct 路径:真实子进程 spawn 后 store 有 Process 登记,归属与引擎
/// scope 同源(scope/run/boot/worktree),身份为真实 PID+starttime;
/// 会话真实收口后注销。
#[tokio::test]
async fn b1_direct_launch_registers_real_process_with_engine_scope() {
    let env = b1_env();
    let (ownership, tree_key) = b1_scope(&env.private_root, &env.store_root, "direct");
    let key = ownership.scope_key().clone();
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_b1_fixture.sh"));

    let mut session = provider
        .start_owned(
            b1_input(None, 30),
            Some(ownership),
            CancellationToken::new(),
        )
        .await
        .expect("owned direct start");

    let records = b1_records(&env.store_root, &key);
    assert_eq!(records.len(), 1, "恰一条 Process 登记: {records:?}");
    let record = &records[0];
    assert_eq!(record.key, key, "scope/run/boot 与引擎 scope 同源");
    assert!(record.resource_id.starts_with("provider-process-"));
    assert_eq!(
        record
            .reserved_labels
            .get("aria.scope_id")
            .map(String::as_str),
        Some(key.scope_id.as_str())
    );
    assert_eq!(
        record
            .reserved_labels
            .get("aria.run_id")
            .map(String::as_str),
        Some(key.run_id.as_str())
    );
    assert_eq!(
        record
            .reserved_labels
            .get("aria.worktree")
            .map(String::as_str),
        Some(tree_key.as_str()),
        "worktree 标签与引擎 scope 同源"
    );
    let (pid, start_time_ticks) = b1_identity(record);
    assert!(pid > 0, "登记必须是真实子进程 PID");
    assert_eq!(
        process_start_time_ticks(pid),
        Some(start_time_ticks),
        "登记身份必须与 /proc starttime 一致(PID 复用防护)"
    );

    // 会话真实收口(子进程退出/组收口)后注销。
    while session.events.recv().await.is_some() {}
    assert!(
        wait_until(|| b1_records(&env.store_root, &key).is_empty()).await,
        "会话真实收口后注销"
    );
}

/// 预留失败 → 零 spawn:store 根不可写时 `start_owned` 直接失败,连
/// `--version` 探测都不得发生。
#[tokio::test]
async fn b1_reserve_failure_spawns_nothing() {
    let env = b1_env();
    let marker = env.private_root.join("spawn-marker");
    let probe = env.private_root.join("kimi-spawn-probe.sh");
    std::fs::write(
        &probe,
        format!(
            "#!/usr/bin/env bash\nprintf 'spawned %s\\n' \"${{1:-}}\" >> '{}'\nexit 1\n",
            marker.display()
        ),
    )
    .expect("write spawn probe");
    std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o755))
        .expect("probe executable");

    let store_root = env.private_root.join("store-is-a-file");
    std::fs::write(&store_root, b"not a directory").expect("store root file");
    let (ownership, _tree_key) = b1_scope(&env.private_root, &store_root, "reserve-fail");
    let provider = KimiCodeProvider::new(probe);

    let result = provider
        .start_owned(
            b1_input(None, 30),
            Some(ownership),
            CancellationToken::new(),
        )
        .await;
    let error = match result {
        Ok(_session) => panic!("预留失败必须上抛,不得启动"),
        Err(error) => error,
    };
    assert!(
        error.details.contains("provider_resources"),
        "失败必须来自预留而非其它路径: {}",
        error.details
    );
    assert!(
        !marker.exists(),
        "预留失败 → 零 spawn(含 --version 探测),marker={}",
        marker.display()
    );
}

/// 无载体 = 零登记:裸 `start` 与 `start_owned(None)`(委托 trait 默认 →
/// 裸 start 语义)都不落任何 Process 登记。
#[tokio::test]
async fn b1_start_without_carrier_registers_nothing() {
    let env = b1_env();
    let (ownership, _tree_key) = b1_scope(&env.private_root, &env.store_root, "no-carrier");
    let key = ownership.scope_key().clone();
    // scope 只用于给出枚举键;无归属请求不消费它。
    drop(ownership);
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_b1_fixture.sh"));

    // F10:direct/legacy 真实 spawn 环境含宿主 PATH/HOME(fixture 落盘)。
    let env_marker = tempfile::tempdir()
        .expect("env marker dir")
        .keep()
        .join("kimi-b1-env.marker");
    let mut bare_input = b1_input(Some("no_hold"), 30);
    bare_input.env_vars.insert(
        "KIMI_B1_ENV_MARKER".to_string(),
        env_marker.display().to_string(),
    );
    let mut session = provider
        .start(bare_input, CancellationToken::new())
        .await
        .expect("bare direct start");
    assert_marker_has_host_environment(&env_marker);
    assert!(
        b1_records(&env.store_root, &key).is_empty(),
        "裸 start 无归属零登记"
    );
    while session.events.recv().await.is_some() {}
    assert!(
        b1_records(&env.store_root, &key).is_empty(),
        "裸 start 收口后仍零登记"
    );

    // `start_owned(None)` 委托默认 = 同一裸语义。
    let mut session = provider
        .start_owned(
            b1_input(Some("no_hold"), 30),
            None,
            CancellationToken::new(),
        )
        .await
        .expect("owned start without carrier");
    assert!(
        b1_records(&env.store_root, &key).is_empty(),
        "start_owned(None) 无归属零登记"
    );
    while session.events.recv().await.is_some() {}
    assert!(
        b1_records(&env.store_root, &key).is_empty(),
        "start_owned(None) 收口后仍零登记"
    );
}

/// 握手失败/会话错误路径:子进程真实回收后注销,进程组无残留。
#[tokio::test]
async fn b1_handshake_failure_releases_after_real_quiesce() {
    let env = b1_env();
    let (ownership, _tree_key) = b1_scope(&env.private_root, &env.store_root, "handshake");
    let key = ownership.scope_key().clone();
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_b1_fixture.sh"));

    let mut session = provider
        .start_owned(
            b1_input(Some("handshake_fail"), 30),
            Some(ownership),
            CancellationToken::new(),
        )
        .await
        .expect("owned direct start");

    // 子进程仍在时(错误应答后短暂存活)登记与身份可见。
    let observed = b1_records(&env.store_root, &key);
    assert_eq!(observed.len(), 1, "spawn 后登记在案: {observed:?}");
    let (pid, start_time_ticks) = b1_identity(&observed[0]);

    // 错误路径收口:Failed 终态后通道终结,登记随后注销。
    let mut failed = false;
    while let Some(event) = session.events.recv().await {
        if matches!(
            event,
            crate::cross_cutting::streaming_provider::ProviderEvent::Failed { .. }
        ) {
            failed = true;
        }
    }
    assert!(failed, "握手失败必须发终态 Failed");
    assert!(
        wait_until(|| b1_records(&env.store_root, &key).is_empty()).await,
        "错误路径真实收口后注销"
    );
    assert!(
        process_start_time_ticks(pid) != Some(start_time_ticks),
        "登记身份(pid,starttime)已真实消失"
    );
    assert!(
        wait_until(|| group_members(pid).is_empty()).await,
        "进程组无残留: {:?}",
        group_members(pid)
    );
}

/// 在途轮次取消:取消(KIMI_SESSION_ABORTED 链)→ 尾部 kill+wait/组收口
/// 后注销登记,进程组无残留。
#[tokio::test]
async fn b1_session_tail_cancel_releases_registration() {
    let env = b1_env();
    let (ownership, _tree_key) = b1_scope(&env.private_root, &env.store_root, "cancel");
    let key = ownership.scope_key().clone();
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_b1_fixture.sh"));
    let cancel = CancellationToken::new();

    let mut session = provider
        .start_owned(
            b1_input(Some("hold_turn"), 30),
            Some(ownership),
            cancel.clone(),
        )
        .await
        .expect("owned in-turn start");

    let records = b1_records(&env.store_root, &key);
    assert_eq!(records.len(), 1, "运行中登记在位: {records:?}");
    let (pid, start_time_ticks) = b1_identity(&records[0]);

    // 在途输出可见(fixture 只发 update 不回 result),随后取消。
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), session.events.recv())
            .await
            .expect("event within bound")
            .expect("session open before cancel");
        match event {
            crate::cross_cutting::streaming_provider::ProviderEvent::TextDelta { content }
                if content == "b1 output" =>
            {
                break;
            }
            crate::cross_cutting::streaming_provider::ProviderEvent::Failed { message } => {
                panic!("provider failed before cancel: {message}");
            }
            _ => {}
        }
    }
    cancel.cancel();
    while session.events.recv().await.is_some() {}
    assert!(
        wait_until(|| b1_records(&env.store_root, &key).is_empty()).await,
        "取消收口后注销"
    );
    assert!(
        wait_until(|| process_start_time_ticks(pid) != Some(start_time_ticks)).await,
        "登记身份已真实消失"
    );
    assert!(
        wait_until(|| group_members(pid).is_empty()).await,
        "进程组无残留: {:?}",
        group_members(pid)
    );
}

/// 协议 `Completed` 不提前注销:事件已发、子进程仍存活时登记仍在;
/// 真实终止(通道终结/组收口)后才注销。
#[tokio::test]
async fn b1_completed_event_does_not_deregister_early() {
    let env = b1_env();
    let (ownership, _tree_key) = b1_scope(&env.private_root, &env.store_root, "completed");
    let key = ownership.scope_key().clone();
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_b1_fixture.sh"));

    let mut session = provider
        .start_owned(
            b1_input(None, 30),
            Some(ownership),
            CancellationToken::new(),
        )
        .await
        .expect("owned direct start");

    let mut saw_completed = false;
    while let Some(event) = session.events.recv().await {
        if matches!(
            event,
            crate::cross_cutting::streaming_provider::ProviderEvent::Completed(_)
        ) {
            saw_completed = true;
            break;
        }
    }
    assert!(saw_completed, "fixture 必须先发协议 Completed");

    let records = b1_records(&env.store_root, &key);
    assert_eq!(records.len(), 1, "Completed 之后登记仍在(不提前注销)");
    let (pid, start_time_ticks) = b1_identity(&records[0]);
    assert_eq!(
        process_start_time_ticks(pid),
        Some(start_time_ticks),
        "Completed 时子进程仍以登记身份存活"
    );

    // 子进程真实终止 → 通道终结 → 注销。
    while session.events.recv().await.is_some() {}
    assert!(
        wait_until(|| b1_records(&env.store_root, &key).is_empty()).await,
        "真实终止后注销"
    );
    assert!(
        wait_until(|| process_start_time_ticks(pid) != Some(start_time_ticks)).await,
        "登记身份已真实消失"
    );
}

/// validated 路径:`ValidatedStreamingProviderInput.launch_ownership` 同样
/// 预留→spawn→回填→真实收口后注销。
#[tokio::test]
async fn b1_validated_launch_registers_and_releases() {
    let env = b1_env();
    let (ownership, tree_key) = b1_scope(&env.private_root, &env.store_root, "validated");
    let key = ownership.scope_key().clone();

    let fixture = LcKimiResumeFixture::new();
    let sink = RecordingToolPolicyAuditSink::new();
    let raw = fixture.lc_streaming_input(Some(sink.clone().bound()), None);
    let validated = fixture
        .validated_coding_input(raw)
        .with_launch_ownership(ownership);

    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_b1_fixture.sh"));
    let mut session = provider
        .start_validated(validated, CancellationToken::new())
        .await
        .expect("validated owned start");

    let records = b1_records(&env.store_root, &key);
    assert_eq!(records.len(), 1, "validated 路径同样登记: {records:?}");
    let record = &records[0];
    assert_eq!(record.key, key);
    assert_eq!(
        record
            .reserved_labels
            .get("aria.worktree")
            .map(String::as_str),
        Some(tree_key.as_str())
    );
    let (pid, start_time_ticks) = b1_identity(record);
    assert_eq!(process_start_time_ticks(pid), Some(start_time_ticks));

    while session.events.recv().await.is_some() {}
    assert!(
        wait_until(|| b1_records(&env.store_root, &key).is_empty()).await,
        "validated 会话真实收口后注销"
    );
}
