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
        tool_policy_digest: format!("sha256:{provider}-digest"),
        argv: vec!["--mode".to_string(), "rpc".to_string()],
        sandbox: None,
        approval_policy: None,
        provider_version: "provider 1.2.3".to_string(),
        dialect: "codex-app-server-rpc".to_string(),
        native_session_id: "thread-1".to_string(),
    })
}

fn approval_decision_event(category: &str, request_id: &str) -> DurableToolPolicyEvent {
    DurableToolPolicyEvent::ApprovalDecision(crate::cross_cutting::tool_policy_audit::ApprovalDecisionAudit {
        request_id: request_id.to_string(),
        category: category.to_string(),
        decision: "decline".to_string(),
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
    std::fs::write(
        &file,
        concat!(
            "{\"schema_version\":1,\"seq\":0,\"event_type\":\"provider_start\"}\n",
            "not-json\n"
        ),
    )
    .expect("write bad fixture");

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
