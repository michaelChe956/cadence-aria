//! C3/REQ-HTR-03：degraded 转移 append-only 诊断流。
//!
//! lease-diagnostics.jsonl 同构（`LeaseDiagnostics` 先例）：单行 JSON、只追加、
//! 进程内 Mutex 串行化「定位路径 + 追加」临界区、durable 滚动上限（超限保留
//! 最近半数行）。差异点：无内存环形缓冲（无诊断端点面，事后调查直接读 jsonl）。
//! 所有 IO 失败静默——打点失败零影响投递路径（REQ-HTR-03 仲裁）。
//!
//! 事件枚举：`degraded_enter`（直播队列溢出，携带被丢帧 event_seq）/
//! `degraded_exit`（session_state 恢复基线投递成功，携带基线 event_seq）。
//! 「连接停在 stale 视图」的时段定案依据：enter..exit 之间该连接只可能
//! 收到恢复基线/有界等待投递的关键帧。

use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;

use serde::Serialize;
use serde_json::Value;

use crate::product::lifecycle_store::LifecycleStore;

/// 会话 timeline 根下的 degraded 投递诊断分区文件名（事后检索脚本契约）。
pub(crate) const DEGRADED_DIAGNOSTICS_FILE: &str = "degraded-diagnostics.jsonl";

/// 行格式 schema 版本。
const DEGRADED_DIAGNOSTICS_SCHEMA_VERSION: u32 = 1;
/// durable 文件滚动上限（超限保留最近半数行，原子 rename 重写；与
/// lease-diagnostics 同参数——REQ-HTR-03「打点滚动上限」）。
const DEGRADED_DIAGNOSTICS_FILE_CAP_BYTES: u64 = 1024 * 1024;

/// 分区内互斥（lease-diagnostics 同款进程内 Mutex）。
static DEGRADED_DIAGNOSTICS_LOG_MUTEX: StdMutex<()> = StdMutex::new(());

/// 会话级 degraded 投递诊断分区路径（manager 创建时定位）。
pub(crate) fn degraded_diagnostics_path(
    lifecycle: &LifecycleStore,
    session_id: &str,
) -> Option<PathBuf> {
    lifecycle
        .workspace_timeline_root_for_session(session_id)
        .ok()
        .map(|root| root.join(DEGRADED_DIAGNOSTICS_FILE))
}

#[derive(Serialize)]
struct DegradedDeliveryRecord {
    schema_version: u32,
    recorded_at: String,
    workspace_session_id: String,
    event: &'static str,
    connection_id: String,
    reason: &'static str,
    event_seq: u64,
}

/// 每会话一个的 degraded 转移诊断写入面：durable jsonl（append-only、滚动
/// 上限）。所有失败静默——REQ-HTR-03 打点失败零影响仲裁；调用方保证
/// `record` 在 manager 状态锁外执行（打点不进投递临界区）。
pub(super) struct DegradedDeliveryDiagnostics {
    session_id: String,
    path: Option<PathBuf>,
    max_file_bytes: u64,
}

impl DegradedDeliveryDiagnostics {
    pub(super) fn new(session_id: &str, path: Option<PathBuf>) -> Self {
        Self {
            session_id: session_id.to_string(),
            path,
            max_file_bytes: DEGRADED_DIAGNOSTICS_FILE_CAP_BYTES,
        }
    }

    /// 仅供单测注入滚动上限。
    #[cfg(test)]
    pub(super) fn with_file_cap(mut self, max_file_bytes: u64) -> Self {
        self.max_file_bytes = max_file_bytes;
        self
    }

    /// 直播队列溢出进入 degraded（event_seq 为被丢帧的序号）。
    pub(super) fn record_enter(&self, connection_id: &str, event_seq: u64) {
        self.emit(Self::record(
            "degraded_enter",
            connection_id,
            "outbound_queue_full",
            event_seq,
        ));
    }

    /// 恢复基线投递成功退出 degraded（event_seq 为基线序号）。
    pub(super) fn record_exit(&self, connection_id: &str, event_seq: u64) {
        self.emit(Self::record(
            "degraded_exit",
            connection_id,
            "session_state_baseline_delivered",
            event_seq,
        ));
    }

    fn record(
        event: &'static str,
        connection_id: &str,
        reason: &'static str,
        event_seq: u64,
    ) -> DegradedDeliveryRecord {
        DegradedDeliveryRecord {
            schema_version: DEGRADED_DIAGNOSTICS_SCHEMA_VERSION,
            recorded_at: String::new(),
            workspace_session_id: String::new(),
            event,
            connection_id: connection_id.to_string(),
            reason,
            event_seq,
        }
    }

    fn emit(&self, mut record: DegradedDeliveryRecord) {
        record.schema_version = DEGRADED_DIAGNOSTICS_SCHEMA_VERSION;
        record.recorded_at = chrono::Utc::now().to_rfc3339();
        record.workspace_session_id = self.session_id.clone();
        let Ok(value) = serde_json::to_value(&record) else {
            return;
        };
        self.append_durable(&value);
    }

    fn append_durable(&self, value: &Value) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let Ok(_guard) = DEGRADED_DIAGNOSTICS_LOG_MUTEX.lock() else {
            return;
        };
        let Ok(line) = serde_json::to_string(value) else {
            return;
        };
        if let Some(parent) = path.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return;
        }
        // 滚动上限：超限时保留最近半数行（原子 rename 重写），durable 证据
        // 优先保住「最近转移序列」（lease-diagnostics 同构）。
        if let Ok(metadata) = std::fs::metadata(path)
            && metadata.len().saturating_add(line.len() as u64) > self.max_file_bytes
            && Self::rotate(path).is_err()
        {
            return;
        }
        let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
            return;
        };
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }

    fn rotate(path: &Path) -> std::io::Result<()> {
        let content = std::fs::read_to_string(path)?;
        let lines: Vec<&str> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        if lines.len() < 2 {
            return Ok(());
        }
        let keep = &lines[lines.len() / 2..];
        let temp = path.with_extension("jsonl.rotate");
        let mut file = File::create(&temp)?;
        for line in keep {
            writeln!(file, "{line}")?;
        }
        file.flush()?;
        drop(file);
        std::fs::rename(&temp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REQ-HTR-03：打点滚动上限——超限后保留最近半数行。
    #[test]
    fn degraded_diagnostics_file_respects_rolling_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(DEGRADED_DIAGNOSTICS_FILE);
        let diagnostics =
            DegradedDeliveryDiagnostics::new("session_cap", Some(path.clone())).with_file_cap(1024);

        for seq in 0..64 {
            diagnostics.record_enter("conn", seq);
        }

        let content = std::fs::read_to_string(&path).expect("diagnostics file");
        let lines: Vec<&str> = content.lines().filter(|line| !line.is_empty()).collect();
        assert!(
            content.len() as u64
                <= 1024 + lines.last().map(|line| line.len()).unwrap_or(0) as u64 + 8,
            "旋转后文件不得超过 cap+单行：{} bytes / {} lines",
            content.len(),
            lines.len()
        );
        let events: Vec<Value> = lines
            .iter()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        // 最近事件必须保住（保最近半数）。
        assert_eq!(
            events.last().expect("kept tail event")["event_seq"],
            63,
            "滚动必须保留最近事件"
        );
    }

    /// REQ-HTR-03：打点失败零影响——路径不可写（被目录占位）时静默不 panic。
    #[test]
    fn degraded_diagnostics_swallows_durable_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(DEGRADED_DIAGNOSTICS_FILE);
        std::fs::create_dir(&path).expect("occupy path with a directory");

        let diagnostics = DegradedDeliveryDiagnostics::new("session_silent", Some(path));
        diagnostics.record_enter("conn", 1);
        diagnostics.record_exit("conn", 2);
    }
}
