//! usage 事件 durable 落盘失败的诊断留痕（REQ-NDR-05）。
//!
//! `emit_execution_event` 的 durable 落盘此前是 `let _ =`（失败静默）。现在 usage 事件
//! 落盘失败必须可定责，按档位降级，且都不改 gate / provider 会话行为：
//!
//! 1. 节点 detail 内 upsert 诊断 execution event（首选：复用 F-46 的 execution_events
//!    通道，durable 且用户可见；事件 id 稳定，同 role 重复失败 upsert 覆盖；
//!    `output=None` 不携带 usage 负载，前端 token 解析与详情摘要都会跳过它）；
//! 2. 节点 detail 通道本身不可写（节点缺失 / detail 损坏 / 写盘失败）时 → 会话级
//!    append-only `usage-diagnostics.jsonl`（durable，脚本可检索）；
//! 3. 连降级分区也不可写 → 结构化 warn（`stage="persist"`）。
//!
//! detail 一律有界：错误文本按字符截断，schema 版本固定 1。

use super::*;

/// 诊断 detail 的 schema 版本（事后调查脚本契约）。
const USAGE_PERSIST_DIAGNOSTIC_VERSION: u32 = 1;
/// 单条错误文本的字符上界（store 错误可能内嵌路径与系统错误串）。
const USAGE_PERSIST_ERROR_MAX_CHARS: usize = 200;

/// usage 落盘失败诊断的落地档位（可观测性降级链的结果）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UsagePersistDiagnostic {
    /// 写进节点 detail 的诊断 execution event（首选）。
    NodeDetail,
    /// 节点 detail 不可写，落到会话级 `usage-diagnostics.jsonl`。
    Journal,
    /// 两档都不可写，只剩结构化 warn。
    LogOnly,
}

/// 诊断 execution event：`kind=usage`、`output=None`（不携带 usage 负载）、
/// `event_id = "{原事件 id}_persist_diag"`（稳定，重复失败 upsert 覆盖）。
pub(crate) fn usage_persist_diagnostic_event(
    event_id: &str,
    error: &str,
) -> ProviderExecutionEvent {
    ProviderExecutionEvent {
        event_id: format!("{event_id}_persist_diag"),
        kind: ProviderExecutionEventKind::Usage,
        status: ProviderExecutionEventStatus::Failed,
        title: "Usage persistence failed".to_string(),
        detail: Some(
            serde_json::json!({
                "diagnostic_version": USAGE_PERSIST_DIAGNOSTIC_VERSION,
                "persist_stage": "persist",
                "event_id": event_id,
                "error": bounded_usage_persist_error(error),
            })
            .to_string(),
        ),
        command: None,
        cwd: None,
        output: None,
        exit_code: None,
    }
}

/// 会话级 journal 行（append-only、可检索；字段与节点 detail 诊断同源）。
pub(crate) fn usage_persist_diagnostic_record(
    session_id: &str,
    node_id: &str,
    event_id: &str,
    error: &str,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": USAGE_PERSIST_DIAGNOSTIC_VERSION,
        "recorded_at": chrono::Utc::now().to_rfc3339(),
        "workspace_session_id": session_id,
        "node_id": node_id,
        "event_id": event_id,
        "stage": "persist",
        "error": bounded_usage_persist_error(error),
    })
}

/// REQ-NDR-05 的有界原则：诊断文本不得随 store 错误全文无限增长。
fn bounded_usage_persist_error(error: &str) -> String {
    error.chars().take(USAGE_PERSIST_ERROR_MAX_CHARS).collect()
}

impl WorkspaceEngine {
    /// usage 事件 durable 落盘失败的可观测化（REQ-NDR-05：不得静默）。
    ///
    /// 返回实际落地的档位（用例据此断言降级链）；任何档位失败都不向上传播错误，
    /// 以保证 provider 会话与 gate 行为不受诊断写影响。
    pub(crate) async fn record_usage_persist_diagnostic(
        &mut self,
        node_id: &str,
        event_id: &str,
        error: &str,
    ) -> UsagePersistDiagnostic {
        let diagnostic = usage_persist_diagnostic_event(event_id, error);
        let diagnostic_json = execution_event_json(&diagnostic);
        let in_band = self
            .update_node_detail(node_id, |detail| {
                upsert_execution_event_json(&mut detail.execution_events, diagnostic_json);
            })
            .await;
        if in_band.is_ok() {
            return UsagePersistDiagnostic::NodeDetail;
        }
        let record =
            usage_persist_diagnostic_record(&self.session.session_id, node_id, event_id, error);
        match self.append_usage_persist_diagnostic_record(&record) {
            Ok(()) => UsagePersistDiagnostic::Journal,
            Err(journal_error) => {
                tracing::warn!(
                    target: "workspace_engine",
                    stage = "persist",
                    node_id,
                    event_id,
                    error = %bounded_usage_persist_error(error),
                    journal_error = %journal_error,
                    "usage persistence diagnostic unavailable"
                );
                UsagePersistDiagnostic::LogOnly
            }
        }
    }

    fn append_usage_persist_diagnostic_record(
        &self,
        record: &serde_json::Value,
    ) -> Result<(), String> {
        let Some(store) = &self.lifecycle_store else {
            return Err("lifecycle store unavailable".to_string());
        };
        store
            .append_usage_persistence_diagnostic(&self.session.session_id, record)
            .map_err(|error| error.to_string())
    }
}
