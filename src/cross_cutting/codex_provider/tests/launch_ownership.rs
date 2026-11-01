//! B1-1(波 2):Codex adapter 的受信 launch 归属接线测试。
//!
//! 两条真实 spawn 面(raw `start_owned` 与 LC `start_validated`)都在 spawn
//! 点消费同一受信载体:预留(durable,失败零 spawn)→ spawn → 回填真实
//! PID+`/proc/<pid>/stat` starttime → 会话真实收口后注销(协议 `Completed`
//! 不提前注销;错误/取消收口后注销)。
//!
//! 归属 scope 以 engine 真实键型(`scope_id`/`run_id`/boot/`aria.worktree`)
//! 打开;断言 adapter 的 Process 登记与 scope 同源,不新造 scope。
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_resources::{
    OwnedResourceKind, ProviderResourceRecord, ProviderRunResourceScope, ResourceScopeKey,
    StartedIdentity, TrustedLaunchOwnership,
};
use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
use crate::product::provider_resource_store::ProviderResourceStore;
use crate::protocol::contracts::ProviderType;

use super::approval_policy::{
    ANCHOR_T34_VERSION_INITIAL, AnchorT34LcHarness, anchor_t34_lc_harness, anchor_t34_lc_input,
};
use super::*;

// ---------- fixtures ----------

struct B1Fixture {
    _base: tempfile::TempDir,
    store_root: PathBuf,
    private_root: PathBuf,
}

fn b1_fixture() -> B1Fixture {
    let base = tempfile::tempdir().expect("b1 fixture base");
    let store_root = base.path().join("resource-store");
    let private_root = base.path().join("private-root");
    std::fs::create_dir_all(&private_root).expect("create private root");
    B1Fixture {
        _base: base,
        store_root,
        private_root,
    }
}

/// 以 engine 真实键型开 scope(真实 scope/run/boot + canonical worktree →
/// tree_key 标签),不新造 scope。
fn b1_scope(
    fixture: &B1Fixture,
    scope_id: &str,
    run_id: &str,
) -> (ProviderRunResourceScope, String) {
    let worktree = fixture
        .private_root
        .canonicalize()
        .expect("canonical worktree");
    let tree_key = crate::product::reviewer_window_store::tree_key_of(&worktree);
    let key = ResourceScopeKey {
        scope_id: scope_id.to_string(),
        run_id: run_id.to_string(),
        boot_identity: format!("aria-engine-{}", std::process::id()),
    };
    let scope = ProviderRunResourceScope::open_for_recovery(
        &key,
        None,
        &fixture.private_root,
        &fixture.store_root,
        None,
    )
    .expect("open b1 scope")
    .with_worktree_key(&tree_key);
    (scope, tree_key)
}

fn b1_process_records(fixture: &B1Fixture, key: &ResourceScopeKey) -> Vec<ProviderResourceRecord> {
    ProviderResourceStore::new(&fixture.store_root)
        .list_scope(key)
        .expect("list scope")
        .into_iter()
        .filter(|record| matches!(record.kind, OwnedResourceKind::Process))
        .collect()
}

async fn b1_wait_records(
    fixture: &B1Fixture,
    key: &ResourceScopeKey,
    predicate: impl Fn(&[ProviderResourceRecord]) -> bool + Send,
) -> Vec<ProviderResourceRecord> {
    for _ in 0..poll_attempts_for_test_timeout() {
        let records = b1_process_records(fixture, key);
        if predicate(&records) {
            return records;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    b1_process_records(fixture, key)
}

fn poll_attempts_for_test_timeout() -> usize {
    let timeout = TEST_TIMEOUT.as_millis().max(1);
    usize::try_from(timeout / 5).unwrap_or(1).max(1)
}

#[cfg(target_os = "linux")]
fn proc_start_time_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit_once(')')?.1;
    after_comm.split_whitespace().nth(19)?.parse().ok()
}

async fn wait_for_file(path: &Path) {
    for _ in 0..poll_attempts_for_test_timeout() {
        if path.is_file() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("fixture file never appeared: {}", path.display());
}

async fn wait_for_process_absent(pid: u32) {
    for _ in 0..poll_attempts_for_test_timeout() {
        if !Path::new(&format!("/proc/{pid}")).exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("fixture process {pid} still present after release");
}

/// F10(adapter 级):direct/legacy 真实 spawn 的子进程环境必须继承宿主
/// PATH/HOME(A1 环境回退语义)——读 `/proc/<pid>/environ` 逐字比对。
#[cfg(target_os = "linux")]
fn assert_child_inherits_host_environment(pid: u32) {
    let environ = std::fs::read(format!("/proc/{pid}/environ"))
        .unwrap_or_else(|error| panic!("read /proc/{pid}/environ: {error}"));
    let entries: Vec<String> = environ
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).into_owned())
        .collect();
    let host_home = format!("HOME={}", std::env::var("HOME").expect("host HOME"));
    let host_path = format!("PATH={}", std::env::var("PATH").expect("host PATH"));
    assert!(
        entries.contains(&host_home),
        "F10:direct/legacy 子进程必须继承宿主 HOME(实测 environ 无 {host_home})"
    );
    assert!(
        entries.contains(&host_path),
        "F10:direct/legacy 子进程必须继承宿主 PATH(实测 environ 无宿主 PATH)"
    );
}

fn b1_marker_path(name: &str) -> PathBuf {
    tempfile::tempdir()
        .expect("b1 marker dir")
        .keep()
        .join(name)
}

fn write_fixture(relative_path: &str, body: &str) -> PathBuf {
    let path = tempfile::tempdir()
        .expect("fixture dir")
        .keep()
        .join(relative_path);
    std::fs::write(&path, body).expect("write fixture");
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(&path)
        .unwrap_or_else(|error| panic!("fixture metadata {}: {error}", path.display()))
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions)
        .unwrap_or_else(|error| panic!("chmod fixture {}: {error}", path.display()));
    path
}

/// raw direct fixture:登记真实 PID 后等 hold 文件再吐 Completed 退出
/// (断言期间子进程保持存活,starttime 可读)。
fn b1_raw_fixture() -> PathBuf {
    write_fixture(
        "codex_b1_raw_fixture.sh",
        r#"#!/usr/bin/env bash
set -u
printf '%s\n' "$$" > "$ARIA_FIXTURE_PID"
while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-0}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-1}\",\"result\":{\"thread\":{\"id\":\"codex-thread-b1\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    while [[ ! -f "$ARIA_FIXTURE_HOLD" ]]; do sleep 0.01; done
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-2}\",\"result\":{\"turn\":{\"id\":\"turn-b1\"}}}"
    echo '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"id":"msg-b1","type":"agentMessage","text":"b1 raw done"}}}'
    echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"turnId":"turn-b1"}}'
    exit 0
  fi
done
"#,
    )
}

/// LC validated fixture:同 raw,但经 LC 握手(thread/start)与投影 params。
fn b1_lc_fixture() -> PathBuf {
    write_fixture(
        "codex_b1_lc_fixture.sh",
        r#"#!/usr/bin/env bash
set -u
printf '%s\n' "$$" > "$ARIA_FIXTURE_PID"
while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-0}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-1}\",\"result\":{\"thread\":{\"id\":\"codex-thread-b1\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    while [[ ! -f "$ARIA_FIXTURE_HOLD" ]]; do sleep 0.01; done
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-2}\",\"result\":{\"turn\":{\"id\":\"turn-b1\"}}}"
    echo '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"id":"msg-b1","type":"agentMessage","text":"b1 lc done"}}}'
    echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"turnId":"turn-b1"}}'
    exit 0
  fi
done
"#,
    )
}

/// 协议 Completed 后仍存活一小段的 fixture:Completed 与真实收口之间的
/// 确定窗口用于断言「不提前注销」(0.5s < 测试态 1s 有界 child join)。
fn b1_completed_hold_fixture() -> PathBuf {
    write_fixture(
        "codex_b1_completed_hold_fixture.sh",
        r#"#!/usr/bin/env bash
set -u
while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-0}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-1}\",\"result\":{\"thread\":{\"id\":\"codex-thread-b1\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-2}\",\"result\":{\"turn\":{\"id\":\"turn-b1\"}}}"
    echo '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"id":"msg-b1","type":"agentMessage","text":"b1 completed"}}}'
    echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"turnId":"turn-b1"}}'
    sleep 0.5
    exit 0
  fi
done
"#,
    )
}

/// 会话尾部挂起 fixture:turn/start 已应答后不再完成(取消路径)。
fn b1_hanging_fixture() -> PathBuf {
    write_fixture(
        "codex_b1_hanging_fixture.sh",
        r#"#!/usr/bin/env bash
set -u
printf '%s\n' "$$" > "$ARIA_FIXTURE_PID"
while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-0}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-1}\",\"result\":{\"thread\":{\"id\":\"codex-thread-b1\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-2}\",\"result\":{\"turn\":{\"id\":\"turn-b1\"}}}"
    sleep 30
  fi
done
"#,
    )
}

fn b1_raw_input(pid_path: &Path, hold_path: &Path) -> StreamingProviderInput {
    let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    input.env_vars.insert(
        "ARIA_FIXTURE_PID".to_string(),
        pid_path.display().to_string(),
    );
    input.env_vars.insert(
        "ARIA_FIXTURE_HOLD".to_string(),
        hold_path.display().to_string(),
    );
    input
}

fn b1_version_supplier() -> crate::cross_cutting::streaming_provider::ProviderVersionSupplier {
    Arc::new(|| Ok(ANCHOR_T34_VERSION_INITIAL.to_string()))
}

/// 经真实 gateway validate 产出 LC validated input(载体绑定在 crate 内
/// 受信装配面完成)。
fn b1_validated_lc_input(
    harness: &AnchorT34LcHarness,
    mut raw: StreamingProviderInput,
    ownership: &TrustedLaunchOwnership,
) -> ValidatedStreamingProviderInput {
    raw.audit_sink = Some(RecordingToolPolicyAuditSink::new().bound());
    let validated = harness
        .gateway
        .validate(harness.request.clone())
        .expect("b1 lc validated policy");
    ValidatedStreamingProviderInput::new(raw, validated).with_launch_ownership(ownership.clone())
}

fn b1_lc_harness(fixture: PathBuf) -> (Arc<CodexProvider>, AnchorT34LcHarness) {
    let provider =
        Arc::new(CodexProvider::new(fixture).with_version_supplier(b1_version_supplier()));
    let harness = anchor_t34_lc_harness(provider.clone(), ANCHOR_T34_VERSION_INITIAL);
    (provider, harness)
}

async fn b1_drain(events: &mut mpsc::Receiver<ProviderEvent>) {
    while tokio::time::timeout(TEST_TIMEOUT, events.recv())
        .await
        .expect("session events must terminate (终结保证契约)")
        .is_some()
    {}
}

fn assert_process_identity(record: &ProviderResourceRecord, expected_pid: u32) {
    match record.started_identity.as_ref() {
        Some(StartedIdentity::Process {
            pid,
            start_time_ticks,
        }) => {
            assert_eq!(*pid, expected_pid, "回填的是真实子进程 PID");
            assert_eq!(
                *start_time_ticks,
                proc_start_time_ticks(expected_pid).expect("observe starttime"),
                "回填 /proc starttime"
            );
            assert_ne!(*start_time_ticks, 0);
        }
        other => panic!("spawn 点必须回填真实身份,实际 {other:?}"),
    }
}

fn assert_same_origin(
    record: &ProviderResourceRecord,
    ownership: &TrustedLaunchOwnership,
    tree_key: &str,
) {
    assert_eq!(record.key, *ownership.scope_key(), "同源 scope key");
    assert_eq!(record.resource_id, ownership.resource_id());
    assert!(
        record.resource_id.starts_with("provider-process-"),
        "每次 launch 唯一资源 ID: {}",
        record.resource_id
    );
    assert_eq!(
        record
            .reserved_labels
            .get("aria.scope_id")
            .map(String::as_str),
        Some(ownership.scope_key().scope_id.as_str())
    );
    assert_eq!(
        record
            .reserved_labels
            .get("aria.run_id")
            .map(String::as_str),
        Some(ownership.scope_key().run_id.as_str())
    );
    assert_eq!(
        record
            .reserved_labels
            .get("aria.worktree")
            .map(String::as_str),
        Some(tree_key)
    );
    assert_eq!(
        record.reserved_labels.get("kind").map(String::as_str),
        Some("provider_process")
    );
}

// ---------- tests ----------

/// raw direct 面(非 LC):`start_owned` 的载体在 spawn 点回填真实
/// PID+starttime,Process 记录归属与 engine scope 完全同源;会话真实收口
/// 后注销。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn codex_owned_raw_start_backfills_real_identity_and_deregisters_after_tail() {
    // 环境门：CI 无交互式 provider CLI（真机 codex 进程在 CI 上起得来但不响应；
    // 见 cross_cutting::real_provider_e2e）；跳过时显式打印。
    if crate::cross_cutting::real_provider_e2e::skip_when_ci_lacks_provider(
        "codex_owned_raw_start_backfills_real_identity_and_deregisters_after_tail",
    ) {
        return;
    }
    let fixture = b1_fixture();
    let (scope, tree_key) = b1_scope(&fixture, "code-delivery-b1-raw", "role-run-b1-raw");
    let ownership = TrustedLaunchOwnership::mint(Arc::new(scope), "code_delivery provider process");
    let pid_path = b1_marker_path("fixture.pid");
    let hold_path = b1_marker_path("fixture.hold");
    let provider = CodexProvider::new(b1_raw_fixture());

    let mut session = provider
        .start_owned(
            b1_raw_input(&pid_path, &hold_path),
            Some(ownership.clone()),
            CancellationToken::new(),
        )
        .await
        .expect("owned raw start");

    // 1) Process 登记在 spawn 后立即出现,归属与引擎 scope 同源。
    let records = b1_wait_records(&fixture, ownership.scope_key(), |records| {
        records.len() == 1
    })
    .await;
    assert_eq!(records.len(), 1, "恰一条 Process 登记: {records:?}");
    assert_same_origin(&records[0], &ownership, &tree_key);

    // 2) 真实 PID+starttime:与 fixture 进程自身及 /proc 观察逐字相等。
    wait_for_file(&pid_path).await;
    let fixture_pid = std::fs::read_to_string(&pid_path)
        .expect("read fixture pid")
        .trim()
        .parse::<u32>()
        .expect("fixture pid");
    assert_process_identity(&records[0], fixture_pid);

    // 3) 会话真实收口(子进程退出)后注销。
    std::fs::write(&hold_path, "").expect("release fixture");
    assert_eq!(recv_completed(&mut session.events).await, "b1 raw done");
    b1_drain(&mut session.events).await;
    let records = b1_wait_records(&fixture, ownership.scope_key(), |records| {
        records.is_empty()
    })
    .await;
    assert!(records.is_empty(), "真实收口后注销: {records:?}");
}

/// LC validated 面:载体绑定进 validated input,`start_validated` 在 spawn
/// 点回填同一真实身份;会话收口后注销。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn codex_validated_start_backfills_real_identity_from_bound_carrier() {
    let fixture = b1_fixture();
    let (scope, tree_key) = b1_scope(&fixture, "plan-review-b1-lc", "role-run-b1-lc");
    let ownership = TrustedLaunchOwnership::mint(Arc::new(scope), "plan-review provider process");
    let pid_path = b1_marker_path("fixture.pid");
    let hold_path = b1_marker_path("fixture.hold");
    let (provider, harness) = b1_lc_harness(b1_lc_fixture());

    let mut raw = anchor_t34_lc_input(&harness.authority_root, None);
    raw.env_vars.insert(
        "ARIA_FIXTURE_PID".to_string(),
        pid_path.display().to_string(),
    );
    raw.env_vars.insert(
        "ARIA_FIXTURE_HOLD".to_string(),
        hold_path.display().to_string(),
    );
    let validated = b1_validated_lc_input(&harness, raw, &ownership);

    let mut session = provider
        .start_validated(validated, CancellationToken::new())
        .await
        .expect("validated owned start");
    assert_eq!(
        session.native_session_id.as_deref(),
        Some("codex-thread-b1")
    );

    // 1) 同源登记 + 真实身份回填。
    let records = b1_wait_records(&fixture, ownership.scope_key(), |records| {
        records.len() == 1
    })
    .await;
    assert_eq!(records.len(), 1, "恰一条 Process 登记: {records:?}");
    assert_same_origin(&records[0], &ownership, &tree_key);
    wait_for_file(&pid_path).await;
    let fixture_pid = std::fs::read_to_string(&pid_path)
        .expect("read fixture pid")
        .trim()
        .parse::<u32>()
        .expect("fixture pid");
    assert_process_identity(&records[0], fixture_pid);

    // 2) 收口后注销。
    std::fs::write(&hold_path, "").expect("release fixture");
    assert_eq!(recv_completed(&mut session.events).await, "b1 lc done");
    b1_drain(&mut session.events).await;
    let records = b1_wait_records(&fixture, ownership.scope_key(), |records| {
        records.is_empty()
    })
    .await;
    assert!(records.is_empty(), "LC 会话收口后注销: {records:?}");
}

/// 无归属请求(`start` 直调 / `start_owned(None)`)不产生任何登记——
/// 不得裸登记,也不得把无归属面伪造为有归属。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn codex_start_without_carrier_registers_nothing() {
    let fixture = b1_fixture();
    let (scope, _tree_key) = b1_scope(&fixture, "code-delivery-b1-none", "role-run-b1-none");
    let key = scope.key().clone();
    // scope 只用于给出枚举键;无归属请求不消费它。
    let _ownership = TrustedLaunchOwnership::mint(Arc::new(scope), "unused");
    let pid_path = b1_marker_path("fixture.pid");
    let hold_path = b1_marker_path("fixture.hold");
    let provider = CodexProvider::new(b1_raw_fixture());

    let mut session = provider
        .start_owned(
            b1_raw_input(&pid_path, &hold_path),
            None,
            CancellationToken::new(),
        )
        .await
        .expect("bare direct start");
    // F10:direct/legacy 真实 spawn 环境含宿主 PATH/HOME。
    wait_for_file(&pid_path).await;
    let fixture_pid = std::fs::read_to_string(&pid_path)
        .expect("read fixture pid")
        .trim()
        .parse::<u32>()
        .expect("fixture pid");
    assert_child_inherits_host_environment(fixture_pid);
    assert!(
        b1_process_records(&fixture, &key).is_empty(),
        "无归属零登记"
    );
    std::fs::write(&hold_path, "").expect("release fixture");
    b1_drain(&mut session.events).await;
    assert!(b1_process_records(&fixture, &key).is_empty());
}

/// 预留失败 → 零 spawn:登记落盘失败时 adapter 的 spawn 从未发生。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn codex_owned_start_reserve_failure_means_zero_spawn() {
    let base = tempfile::tempdir().expect("b1 reserve-fail base");
    let private_root = base.path().join("private-root");
    std::fs::create_dir_all(&private_root).expect("create private root");
    // store 根是普通文件 → reserve 落盘必败。
    let store_root = base.path().join("store-is-a-file");
    std::fs::write(&store_root, "not a directory").expect("write store blocker");
    let scope = ProviderRunResourceScope::open_for_recovery(
        &ResourceScopeKey {
            scope_id: "code-delivery-b1-reserve-fail".to_string(),
            run_id: "role-run-b1-reserve-fail".to_string(),
            boot_identity: format!("aria-engine-{}", std::process::id()),
        },
        None,
        &private_root,
        &store_root,
        None,
    )
    .expect("open scope with blocked store");
    let ownership = TrustedLaunchOwnership::mint(Arc::new(scope), "code_delivery provider process");
    let key = ownership.scope_key().clone();

    let marker = base.path().join("spawned-marker");
    let provider = CodexProvider::new(write_fixture(
        "codex_b1_spawn_marker_fixture.sh",
        "#!/usr/bin/env bash\nset -u\nprintf 'spawned\\n' >> \"$ARIA_FIXTURE_MARKER\"\nexit 0\n",
    ));
    let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    input.env_vars.insert(
        "ARIA_FIXTURE_MARKER".to_string(),
        marker.display().to_string(),
    );

    let error = match provider
        .start_owned(input, Some(ownership), CancellationToken::new())
        .await
    {
        Ok(_) => panic!("预留失败必须上抛,不得启动"),
        Err(error) => error,
    };
    assert!(
        error.details.contains("provider_resources_store"),
        "预留失败原因须可诊断: {}",
        error.details
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!marker.exists(), "预留失败零 spawn(adapter 从未执行)");
    // store 整体不可写:连枚举都失败,证明没有任何 durable 登记落盘。
    assert!(
        ProviderResourceStore::new(&store_root)
            .list_scope(&key)
            .is_err(),
        "blocked store 不可能有登记"
    );
}

/// LC 握手失败:错误链 kill+wait 子进程后,按登记身份收口并注销。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn codex_validated_handshake_failure_releases_registration_after_tail() {
    let fixture = b1_fixture();
    let (scope, _tree_key) = b1_scope(&fixture, "plan-review-b1-fail", "role-run-b1-fail");
    let ownership = TrustedLaunchOwnership::mint(Arc::new(scope), "plan-review provider process");
    let key = ownership.scope_key().clone();
    let (provider, harness) = b1_lc_harness(executable_fixture(
        "tests/fixtures/provider/codex_app_server_resume_missing_thread_id_fixture.sh",
    ));
    let raw = anchor_t34_lc_input(
        &harness.authority_root,
        Some("codex-thread-123".to_string()),
    );
    let validated = b1_validated_lc_input(&harness, raw, &ownership);

    let error = match provider
        .start_validated(validated, CancellationToken::new())
        .await
    {
        Ok(_) => panic!("LC 握手失败必须上抛"),
        Err(error) => error,
    };
    assert!(
        error.details.contains("native resume not confirmed"),
        "握手失败原因须显式声明已启动 child 被终止: {}",
        error.details
    );
    // 错误路径返回前已完成收口注销(不留 durable 记录;注销即证明组无残留)。
    let records = b1_wait_records(&fixture, &key, |records| records.is_empty()).await;
    assert!(records.is_empty(), "握手失败收口后注销: {records:?}");
}

/// 会话尾部取消:Aborted 收口(kill+wait)后注销登记。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn codex_session_tail_cancel_releases_registration() {
    let fixture = b1_fixture();
    let (scope, _tree_key) = b1_scope(&fixture, "code-delivery-b1-cancel", "role-run-b1-cancel");
    let ownership = TrustedLaunchOwnership::mint(Arc::new(scope), "code_delivery provider process");
    let key = ownership.scope_key().clone();
    let pid_path = b1_marker_path("fixture.pid");
    let provider = CodexProvider::new(b1_hanging_fixture());
    let cancel = CancellationToken::new();
    let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    input.env_vars.insert(
        "ARIA_FIXTURE_PID".to_string(),
        pid_path.display().to_string(),
    );

    let mut session = provider
        .start_owned(input, Some(ownership), cancel.clone())
        .await
        .expect("owned hanging start");
    let records = b1_wait_records(&fixture, &key, |records| records.len() == 1).await;
    assert_eq!(records.len(), 1, "运行中登记在位: {records:?}");
    wait_for_file(&pid_path).await;
    let pid = std::fs::read_to_string(&pid_path)
        .expect("read fixture pid")
        .trim()
        .parse::<u32>()
        .expect("fixture pid");
    cancel.cancel();
    b1_drain(&mut session.events).await;
    let records = b1_wait_records(&fixture, &key, |records| records.is_empty()).await;
    assert!(records.is_empty(), "取消收口后注销: {records:?}");
    wait_for_process_absent(pid).await;
}

/// 协议 `Completed` 不提前注销:Completed 事件之后、会话真实收口之前登记
/// 仍在;子进程退出(收口可证)后才注销。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn codex_completed_event_does_not_deregister_early() {
    let fixture = b1_fixture();
    let (scope, _tree_key) = b1_scope(
        &fixture,
        "code-delivery-b1-completed",
        "role-run-b1-completed",
    );
    let ownership = TrustedLaunchOwnership::mint(Arc::new(scope), "code_delivery provider process");
    let key = ownership.scope_key().clone();
    let provider = CodexProvider::new(b1_completed_hold_fixture());

    let mut session = provider
        .start_owned(
            streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto),
            Some(ownership),
            CancellationToken::new(),
        )
        .await
        .expect("owned completed start");
    let records = b1_wait_records(&fixture, &key, |records| records.len() == 1).await;
    assert_eq!(records.len(), 1, "运行中登记在位: {records:?}");

    // 1) 收到协议 Completed。
    let mut saw_completed = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .expect("completion event")
    {
        if let ProviderEvent::Completed(_) = event {
            saw_completed = true;
            break;
        }
    }
    assert!(saw_completed, "fixture 必须先发 Completed");
    // 2) Completed 之后、会话真实收口之前:登记仍在。
    let records = b1_process_records(&fixture, &key);
    assert_eq!(records.len(), 1, "协议 Completed 不提前注销: {records:?}");
    // 3) 子进程退出 → 收口可证 → 注销。
    b1_drain(&mut session.events).await;
    let records = b1_wait_records(&fixture, &key, |records| records.is_empty()).await;
    assert!(records.is_empty(), "真实收口后注销: {records:?}");
}
