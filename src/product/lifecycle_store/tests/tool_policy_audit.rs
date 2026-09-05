// Task 3.2（REQ-ENV-09 GC11）：LifecycleStore durable tool-policy-run-audit 分区
// 与四类 canonical 事件的 schema/时序/坏行语义测试。
// 经 include! 引入 tests.rs（large_file_guard 1200 行红线）。

use crate::cross_cutting::tool_policy_audit::{
    DurableToolPolicyEvent, ToolPolicyAuditSink, ToolPolicyAuditLine,
};

/// durable sink fixture：真实 LifecycleStore（临时 .aria 根）。
fn test_tool_policy_audit_sink() -> super::LifecycleStore {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    super::LifecycleStore::new(crate::product::app_paths::ProductAppPaths::new(
        tmp.path().join(".aria"),
    ))
}

fn provider_start_event(provider: &str) -> DurableToolPolicyEvent {
    DurableToolPolicyEvent::ProviderStart(crate::cross_cutting::tool_policy_audit::ProviderStartAudit {
        provider: provider.to_string(),
        role: "orchestrator".to_string(),
        workspace_session_id: "ws-1".to_string(),
        provider_session_id: "thread-1".to_string(),
        tool_policy_canonical_digest: format!("sha256:{provider}-digest"),
        argv: vec!["--mode".to_string(), "rpc".to_string()],
        sandbox: None,
        approval_policy: None,
        provider_version: "provider 1.2.3".to_string(),
        adapter_dialect: "codex-app-server-rpc".to_string(),
    })
}

fn approval_decision_event(category: &str, request_id: &str) -> DurableToolPolicyEvent {
    DurableToolPolicyEvent::ApprovalDecision(crate::cross_cutting::tool_policy_audit::ApprovalDecisionAudit {
        request_id: request_id.to_string(),
        category: category.to_string(),
        server_name: None,
        tool_name: Some("command".to_string()),
        decision: "decline".to_string(),
        reason_code: "policy_denies_write_side".to_string(),
        policy_digest: "sha256:codex-digest".to_string(),
    })
}

fn protocol_warning_event(reason_code: &str) -> DurableToolPolicyEvent {
    DurableToolPolicyEvent::ProtocolWarning(crate::cross_cutting::tool_policy_audit::ProtocolWarningAudit {
        reason_code: reason_code.to_string(),
        method: "item/unknownKind/requestApproval".to_string(),
        occurrence: 1,
    })
}

fn session_terminated_event(reason_code: &str) -> DurableToolPolicyEvent {
    DurableToolPolicyEvent::SessionTerminated(crate::cross_cutting::tool_policy_audit::SessionTerminatedAudit {
        reason_code: reason_code.to_string(),
    })
}

#[tokio::test]
async fn tool_policy_audit_writes_provider_start_once_then_canonical_events() {
    let sink = test_tool_policy_audit_sink();
    sink.append("ws-1", 7, provider_start_event("codex")).unwrap();
    sink.append("ws-1", 7, approval_decision_event("fileChange", "aria-0"))
        .unwrap();
    sink.append("ws-1", 7, protocol_warning_event("unknown"))
        .unwrap();
    sink.append("ws-1", 7, session_terminated_event("unknown_approval_storm"))
        .unwrap();
    let lines = sink.read_tool_policy_lines("ws-1", 7).unwrap();
    assert_eq!(lines[0].event_type(), "provider_start");
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.event_type() == "provider_start")
            .count(),
        1
    );
    assert!(lines.windows(2).all(|pair| pair[0].seq < pair[1].seq));
    // 四类 canonical 事件按写入顺序落盘，且字段含 digest/request id/reason code 契约字段。
    let types: Vec<&str> = lines.iter().map(|line| line.event_type()).collect();
    assert_eq!(
        types,
        vec![
            "provider_start",
            "approval_decision",
            "protocol_warning",
            "session_terminated"
        ]
    );
    let serialized = serde_json::to_string(&lines[0]).unwrap();
    assert!(serialized.contains("sha256:codex-digest"), "{serialized}");
    assert!(serialized.contains("provider 1.2.3"));
    assert!(serialized.contains("codex-app-server-rpc"));
    let serialized = serde_json::to_string(&lines[1]).unwrap();
    assert!(serialized.contains("aria-0"), "{serialized}");
    let serialized = serde_json::to_string(&lines[3]).unwrap();
    assert!(serialized.contains("unknown_approval_storm"), "{serialized}");
}

// ---- F3 修复轮 P1-1：D6/D7 冻结字段名与必需字段严格性 ----

#[test]
fn tool_policy_audit_schema_freezes_d6_d7_persisted_field_names() {
    // D6/D7 冻结字段名逐字对账：provider_start/approval_decision 的 durable 序列化
    // 键名必须与契约一致（不得以别名落盘）。
    let sink = test_tool_policy_audit_sink();
    sink.append("ws-frozen", 0, provider_start_event("codex")).unwrap();
    sink.append("ws-frozen", 0, approval_decision_event("file_change", "aria-0"))
        .unwrap();
    let lines = sink.read_tool_policy_lines("ws-frozen", 0).unwrap();
    let start = serde_json::to_value(&lines[0]).unwrap();
    assert_eq!(start["workspace_session_id"], "ws-frozen", "{start}");
    assert!(start.get("provider_session_id").is_some(), "{start}");
    assert!(start.get("tool_policy_canonical_digest").is_some(), "{start}");
    assert!(start.get("adapter_dialect").is_some(), "{start}");
    assert!(start.get("provider_version").is_some(), "{start}");
    let decision = serde_json::to_value(&lines[1]).unwrap();
    for field in [
        "server_name",
        "tool_name",
        "reason_code",
        "policy_digest",
        "category",
        "request_id",
        "decision",
    ] {
        assert!(decision.get(field).is_some(), "approval_decision missing {field}: {decision}");
    }
}

#[test]
fn tool_policy_audit_required_fields_must_be_present_when_parsing() {
    // 必需字段缺失 = 解析失败：不得以 `#[serde(default)]` 宽容出无指纹记录。
    let sink = test_tool_policy_audit_sink();
    sink.append("ws-strict", 0, provider_start_event("codex")).unwrap();
    let lines = sink.read_tool_policy_lines("ws-strict", 0).unwrap();
    let base = serde_json::to_string(&lines[0]).unwrap();
    for field in [
        "workspace_session_id",
        "provider_session_id",
        "tool_policy_canonical_digest",
        "provider_version",
        "adapter_dialect",
        "schema_version",
        "seq",
    ] {
        let mut value = serde_json::from_str::<serde_json::Value>(&base).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove(field)
            .unwrap_or_else(|| panic!("fixture must carry {field}: {base}"));
        let line = serde_json::to_string(&value).unwrap();
        assert!(
            serde_json::from_str::<ToolPolicyAuditLine>(&line).is_err(),
            "missing required field {field} must fail parsing"
        );
    }
}

// ---- F3 修复轮 P1-4：resume drift 的 superseded 事件必须落在被取代旧 run 的文件 ----

/// fake pi：rpc 模式下读 stdin 到 EOF 后退出（策略会话 start 只需子进程可拉起）。
#[cfg(unix)]
fn fake_pi_rpc_fixture(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("fake-pi-rpc");
    std::fs::write(&path, "#!/bin/sh\nwhile IFS= read -r line; do :; done\n").expect("write fixture");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fixture");
    path
}

/// 真实 LifecycleStore 的 drift 端到端（P1-4 裁决）：superseded_policy_drift 的
/// `session_terminated` 写入被取代旧 run 的文件（其 provider_start 已是首行）；
/// 新 run 文件照常以 provider_start 开启。RecordingSink 不校验首行不变量，
/// 必须用真实 store 测。
#[cfg(unix)]
#[tokio::test]
async fn tool_policy_audit_resume_drift_superseded_lands_on_replaced_run_file() {
    use crate::cross_cutting::pi_provider::{PI_POLICY_DIALECT, PiProvider, TOOL_POLICY_PROVIDER_NAME};
    use crate::cross_cutting::streaming_provider::{
        ProviderToolPolicy, ProviderVersionSupplier, StreamingProviderAdapter as _,
    };
    use crate::cross_cutting::tool_policy_audit::{ProviderStartAudit, RoleRunBoundAuditSink};
    use crate::protocol::contracts::{AdapterRole, ProviderType};

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let store = std::sync::Arc::new(super::LifecycleStore::new(
        crate::product::app_paths::ProductAppPaths::new(tmp.path().join(".aria")),
    ));
    let canonical = crate::cross_cutting::streaming_provider::canonical_tool_policy(
        TOOL_POLICY_PROVIDER_NAME,
        &ProviderToolPolicy::deny_file_write_builtins(),
    )
    .expect("canonical policy");
    // 旧 run（seq 0）：同 native id，但 version 漂移（pi 9.9.9-stale ≠ 当前 supplier）。
    let stale = ProviderStartAudit {
        provider: TOOL_POLICY_PROVIDER_NAME.to_string(),
        role: "orchestrator".to_string(),
        workspace_session_id: "ws-drift".to_string(),
        provider_session_id: "pi-session-drift".to_string(),
        tool_policy_canonical_digest: canonical.digest.clone(),
        argv: Vec::new(),
        sandbox: None,
        approval_policy: None,
        provider_version: "pi 9.9.9-stale".to_string(),
        adapter_dialect: PI_POLICY_DIALECT.to_string(),
    };
    sink_append(store.as_ref(), "ws-drift", 0, stale);
    let next_seq = store.next_tool_policy_role_run_seq("ws-drift").unwrap();
    let bound_sink = RoleRunBoundAuditSink::new(store.clone(), "ws-drift", next_seq).into_sink();

    let provider = PiProvider::new(fake_pi_rpc_fixture(tmp.path()))
        .with_version_supplier(std::sync::Arc::new(|| Ok("pi 0.83.0-policy-fixture".to_string()))
            as ProviderVersionSupplier);
    let input = crate::cross_cutting::streaming_provider::StreamingProviderInput {
        tool_policy: Some(ProviderToolPolicy::deny_file_write_builtins()),
        audit_sink: Some(bound_sink),
        provider_type: ProviderType::Pi,
        role: AdapterRole::Orchestrator,
        prompt: "fixture prompt".to_string(),
        working_dir: tempfile::tempdir().expect("temporary working dir").keep(),
        workspace_session_id: Some("ws-drift".to_string()),
        resume_provider_session_id: Some("pi-session-drift".to_string()),
        permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: std::collections::BTreeMap::new(),
        timeout_secs: 60,
    };
    let session = provider
        .start(input, tokio_util::sync::CancellationToken::new())
        .await
        .expect("drifted resume must supersede and start a fresh session");
    drop(session.events);

    // 旧 run 文件（seq 0）：provider_start 仍为首行，superseded 终止审计追加其后。
    let old_lines = store.read_tool_policy_lines("ws-drift", 0).unwrap();
    assert_eq!(old_lines[0].event_type(), "provider_start");
    assert_eq!(
        old_lines.len(),
        2,
        "superseded session_terminated must land on the replaced run's file"
    );
    assert_eq!(old_lines[1].event_type(), "session_terminated");
    assert!(serde_json::to_string(&old_lines[1]).unwrap().contains("superseded_policy_drift"));
    // 新 run 文件（seq 1）：provider_start 恰为首行，无终止事件前置。
    let new_lines = store.read_tool_policy_lines("ws-drift", 1).unwrap();
    assert_eq!(
        new_lines[0].event_type(),
        "provider_start",
        "new run must start with provider_start"
    );
    assert!(new_lines
        .iter()
        .all(|line| line.event_type() != "session_terminated"));
}

/// 测试内同步 append 辅助（真实 store 的 sink trait 入口）。
#[cfg(unix)]
fn sink_append(
    store: &super::LifecycleStore,
    workspace_session_id: &str,
    role_run_seq: u64,
    record: crate::cross_cutting::tool_policy_audit::ProviderStartAudit,
) {
    use crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink as _;
    store
        .append(
            workspace_session_id,
            role_run_seq,
            DurableToolPolicyEvent::ProviderStart(record),
        )
        .expect("seed old run provider_start");
}

// ---- F3 修复轮 P1-5/P1-6/P2-3：互斥串行、seq 持久化分配与坏文件 fail-closed ----

#[test]
fn tool_policy_audit_concurrent_appends_and_allocations_are_serialized() {
    // P1-5：两线程并发 append 同一文件 + 并发分配 seq——进程内互斥包住
    // 「seq 分配+读尾行+append」全临界区，行 seq 无重复、role_run_seq 分配唯一。
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let paths = crate::product::app_paths::ProductAppPaths::new(tmp.path().join(".aria"));
    let store = super::LifecycleStore::new(paths.clone());
    store
        .append("ws-concurrent", 7, provider_start_event("codex"))
        .unwrap();
    let s1 = store.clone();
    let s2 = store.clone();
    let a1 = store.clone();
    let a2 = store.clone();
    let appender = |sink: super::LifecycleStore| {
        std::thread::spawn(move || {
            for _ in 0..25 {
                sink.append(
                    "ws-concurrent",
                    7,
                    approval_decision_event("file_change", "aria-0"),
                )
                .expect("concurrent append");
            }
        })
    };
    let allocator = |sink: super::LifecycleStore| {
        std::thread::spawn(move || {
            let mut allocated = Vec::new();
            for _ in 0..25 {
                allocated.push(sink.next_tool_policy_role_run_seq("ws-concurrent").unwrap());
            }
            allocated
        })
    };
    let handles_appends = vec![appender(s1), appender(s2)];
    let handles_alloc = vec![allocator(a1), allocator(a2)];
    for handle in handles_appends {
        handle.join().expect("appender thread");
    }
    let mut allocated = Vec::new();
    for handle in handles_alloc {
        allocated.extend(handle.join().expect("allocator thread"));
    }
    // 行 seq：provider_start + 50 条决策，全部唯一且单调（文件内顺序）。
    let lines = store.read_tool_policy_lines("ws-concurrent", 7).unwrap();
    assert_eq!(lines.len(), 51);
    let seqs: Vec<u64> = lines.iter().map(|line| line.seq).collect();
    let mut unique = seqs.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), seqs.len(), "行 seq 不得重复");
    assert!(lines.windows(2).all(|pair| pair[0].seq < pair[1].seq));
    // role_run_seq 分配：50 次全部唯一（marker 持久化高水位）。
    let mut unique_alloc = allocated.clone();
    unique_alloc.sort_unstable();
    unique_alloc.dedup();
    assert_eq!(unique_alloc.len(), allocated.len(), "role_run_seq 分配不得重复");
}

#[test]
fn tool_policy_audit_role_run_seq_allocation_persists_immediately_and_is_not_reused() {
    // P1-6：seq 分配随 marker 持久化（provider_start 写失败/崩溃后不得复用），
    // 且跨进程（新 store 实例同根）单调。
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = crate::product::app_paths::ProductAppPaths::new(tmp.path().join(".aria"));
    let store = super::LifecycleStore::new(paths.clone());
    assert_eq!(store.next_tool_policy_role_run_seq("ws-alloc").unwrap(), 0);
    // 未写任何 provider_start（模拟写失败/崩溃）：再次分配不得复用 0。
    assert_eq!(store.next_tool_policy_role_run_seq("ws-alloc").unwrap(), 1);
    // 跨进程：新实例同根继续单调。
    let reopened = super::LifecycleStore::new(paths);
    assert_eq!(reopened.next_tool_policy_role_run_seq("ws-alloc").unwrap(), 2);
    // 兼容：无 marker 的既有分区按文件 max 推导。
    store
        .append("ws-legacy", 5, provider_start_event("codex"))
        .unwrap();
    assert_eq!(store.next_tool_policy_role_run_seq("ws-legacy").unwrap(), 6);
}

#[test]
fn tool_policy_audit_append_fails_closed_on_corrupted_file() {
    // P2-3：append 前校验——文件首行不可解析（或任一行坏行）→ 返回错误，
    // 不得在损坏文件上继续追加。
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join(".aria").join("tool-policy-run-audit").join("ws-corrupt");
    std::fs::create_dir_all(&root).expect("partition dir");
    std::fs::write(root.join("9.jsonl"), "not-json\n").expect("corrupt fixture");
    let store = super::LifecycleStore::new(crate::product::app_paths::ProductAppPaths::new(
        tmp.path().join(".aria"),
    ));
    let result = store.append("ws-corrupt", 9, provider_start_event("codex"));
    assert!(
        result.is_err(),
        "append must fail closed on a corrupted partition file"
    );
    let raw = std::fs::read_to_string(root.join("9.jsonl")).unwrap();
    assert_eq!(raw.lines().count(), 1, "损坏文件不得被追加");
}

#[test]
fn tool_policy_audit_rejects_duplicate_or_late_provider_start() {
    let sink = test_tool_policy_audit_sink();
    sink.append("ws-2", 0, provider_start_event("codex")).unwrap();
    // 同一 (workspace_session_id, role_run_seq) 文件内 provider_start 必须唯一且为首行。
    let duplicate = sink.append("ws-2", 0, provider_start_event("codex"));
    assert!(duplicate.is_err(), "duplicate provider_start must error");

    // 已有后续事件的文件不允许再补 provider_start（首行约束）。
    sink.append("ws-2", 1, provider_start_event("codex")).unwrap();
    sink.append("ws-2", 1, approval_decision_event("fileChange", "aria-0"))
        .unwrap();
    let late = sink.append("ws-2", 1, provider_start_event("codex"));
    assert!(late.is_err(), "late provider_start must error");

    // 空/不存在文件不允许以非 provider_start 开头。
    let orphan = sink.append("ws-2", 2, protocol_warning_event("unknown"));
    assert!(orphan.is_err(), "canonical file must start with provider_start");
}

#[test]
fn tool_policy_audit_bad_line_reader_skips_and_reports_without_writing_back() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let root = tmp.path().join(".aria").join("tool-policy-run-audit").join("ws-1");
    std::fs::create_dir_all(&root).expect("partition dir");
    let file = root.join("7.jsonl");
    // 首行为完整 provider_start（冻结字段齐全）；第二行坏 JSON。P1-1 后必需字段
    // 缺失的行同样是坏行（见 required_fields 测试）。
    let valid_start = serde_json::to_string(&ToolPolicyAuditLine::from_event(
        0,
        provider_start_event("codex"),
    ))
    .unwrap();
    std::fs::write(&file, format!("{valid_start}\nnot-json\n")).expect("write bad fixture");

    let store = super::LifecycleStore::new(crate::product::app_paths::ProductAppPaths::new(
        tmp.path().join(".aria"),
    ));
    let result = store
        .read_tool_policy_lines_with_warnings("ws-1", 7)
        .unwrap();
    assert_eq!(result.events.len(), 1);
    assert_eq!(result.events[0].event_type(), "provider_start");
    assert_eq!(result.warnings[0].reason_code, "invalid_json_line");
    // 读取告警不得写回 durable 分区：原始两行保持原样。
    let raw = std::fs::read_to_string(&file).unwrap();
    assert_eq!(raw.lines().count(), 2, "reader must not mutate the partition");
    assert!(raw.contains("not-json"));
    assert!(!store.contains_tool_policy_event("ws-1", 7, "protocol_warning"));
}

#[test]
fn tool_policy_audit_line_seq_is_monotonic_across_appends() {
    let sink = test_tool_policy_audit_sink();
    sink.append("ws-3", 4, provider_start_event("pi")).unwrap();
    sink.append("ws-3", 4, approval_decision_event("command_execution", "aria-1"))
        .unwrap();
    let lines = sink.read_tool_policy_lines("ws-3", 4).unwrap();
    assert_eq!(lines[0].seq, 0);
    assert_eq!(lines[1].seq, 1);
    // 跨文件复用同一 role_run_seq（新文件企图覆盖既有 run 记录）由 provider_start
    // 首行唯一性守卫兜底：同 (ws, seq) 再写 provider_start 即 duplicate 错误。
    let reuse = sink.append("ws-3", 4, provider_start_event("pi"));
    assert!(reuse.is_err());
}

#[test]
fn tool_policy_audit_rejects_path_escape_identifiers() {
    let sink = test_tool_policy_audit_sink();
    let escape = sink.append("../escape", 0, provider_start_event("codex"));
    assert!(escape.is_err(), "workspace id path escape must fail closed");
    let escape = sink.append("ws/escape", 0, provider_start_event("codex"));
    assert!(escape.is_err());
}

#[test]
fn tool_policy_audit_role_run_seq_allocation_is_monotonic_per_workspace() {
    let sink = test_tool_policy_audit_sink();
    assert_eq!(sink.next_tool_policy_role_run_seq("ws-4").unwrap(), 0);
    sink.append("ws-4", 0, provider_start_event("codex")).unwrap();
    assert_eq!(sink.next_tool_policy_role_run_seq("ws-4").unwrap(), 1);
    // 其它 workspace 互不影响。
    assert_eq!(sink.next_tool_policy_role_run_seq("ws-5").unwrap(), 0);
}

impl ToolPolicyAuditLine {
    fn event_type_text(&self) -> &'static str {
        self.event_type()
    }
}

#[test]
fn tool_policy_audit_line_exposes_event_type_for_every_canonical_event() {
    let line = ToolPolicyAuditLine::from_event(0, provider_start_event("codex"));
    assert_eq!(line.event_type_text(), "provider_start");
    let line = ToolPolicyAuditLine::from_event(1, approval_decision_event("file_change", "aria-0"));
    assert_eq!(line.event_type_text(), "approval_decision");
    let line = ToolPolicyAuditLine::from_event(2, protocol_warning_event("unsupported_approval_kind"));
    assert_eq!(line.event_type_text(), "protocol_warning");
    let line = ToolPolicyAuditLine::from_event(3, session_terminated_event("unknown_approval_storm"));
    assert_eq!(line.event_type_text(), "session_terminated");
}

// ---- Task 3.3（REQ-ENV-09/GC9）：resume 冻结三元组比对 ----

use crate::cross_cutting::tool_policy_audit::{
    ResumeDecision, provider_start_record, resume_with_audit_record,
};

#[test]
fn resume_rejects_digest_version_or_dialect_drift_and_missing_record() {
    let stored = provider_start_record("sha256:a", "provider 1.2.3", "codex-app-server-rpc");
    assert!(matches!(
        resume_with_audit_record(Some(stored.clone()), &stored),
        ResumeDecision::Resume
    ));
    assert!(matches!(
        resume_with_audit_record(
            Some(stored.clone()),
            &stored.clone().with_digest("sha256:b")
        ),
        ResumeDecision::RejectSupersedeAndStartNew
    ));
    assert!(matches!(
        resume_with_audit_record(
            Some(stored.clone()),
            &stored.clone().with_version("provider 1.2.4")
        ),
        ResumeDecision::RejectSupersedeAndStartNew
    ));
    assert!(matches!(
        resume_with_audit_record(
            Some(stored.clone()),
            &stored.clone().with_dialect("codex-app-server-rpc-v2")
        ),
        ResumeDecision::RejectSupersedeAndStartNew
    ));
    assert!(matches!(
        resume_with_audit_record(None, &stored),
        ResumeDecision::RejectSupersedeAndStartNew
    ));
}

#[test]
fn tool_policy_audit_resume_lookup_finds_latest_provider_start_by_native_session() {
    use crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink as _;

    let sink = test_tool_policy_audit_sink();
    // 两个 run（seq 0/1）持有同一 native id（resume 重试）；最近（seq 1）生效。
    sink.append(
        "ws-6",
        0,
        DurableToolPolicyEvent::ProviderStart(
            provider_start_record("sha256:a", "provider 1.2.3", "codex-app-server-rpc")
                .with_provider_session_id("thread-shared"),
        ),
    )
    .unwrap();
    sink.append(
        "ws-6",
        1,
        DurableToolPolicyEvent::ProviderStart(
            provider_start_record("sha256:a2", "provider 1.2.3", "codex-app-server-rpc")
                .with_provider_session_id("thread-shared"),
        ),
    )
    .unwrap();
    let found = sink
        .find_latest_tool_policy_provider_start("ws-6", "thread-shared")
        .unwrap()
        .expect("stored provider_start must be found");
    assert_eq!(found.record.tool_policy_canonical_digest, "sha256:a2");
    assert_eq!(found.role_run_seq, 1, "lookup must surface the run location (P1-4)");

    // 其它 native id / 其它 workspace：缺失 → None（resume 决策拒绝并新建）。
    assert!(sink
        .find_latest_tool_policy_provider_start("ws-6", "thread-other")
        .unwrap()
        .is_none());
    assert!(sink
        .find_latest_tool_policy_provider_start("ws-7", "thread-shared")
        .unwrap()
        .is_none());
}
