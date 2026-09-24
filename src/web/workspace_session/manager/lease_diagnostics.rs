//! REQ-DLS-03：租约转移 append-only 诊断流。
//!
//! usage-diagnostics.jsonl 同构（`LifecycleStore::append_usage_persistence_diagnostic`
//! 先例）：单行 JSON、只追加、进程内 Mutex 串行化「定位路径 + 追加」临界区。
//! 差异点：打点失败零影响仲裁——本模块所有 IO 错误静默降级（租约仲裁行为
//! 不因诊断流不可写而改变），且调用方保证 `record` 在 manager 状态锁外执行
//! （打点不进仲裁状态机）。
//!
//! 事件枚举（形态二后，见 change design D3）：
//! `hold`（hello 显式获取）/ `self_heal`（REQ-DLS-01 首写自愈授予）/
//! `release`（holder 连接断开释放）/ `write_rejected_stale` /
//! `write_rejected_observer`。attach 不产生事件（REQ-DLS-04 零效应）。

use std::collections::VecDeque;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;

use serde::Serialize;
use serde_json::Value;

use crate::product::lifecycle_store::LifecycleStore;

/// 会话 timeline 根下的租约诊断分区文件名（事后检索脚本契约）。
pub(crate) const LEASE_DIAGNOSTICS_FILE: &str = "lease-diagnostics.jsonl";

/// 行格式 schema 版本。
const LEASE_DIAGNOSTICS_SCHEMA_VERSION: u32 = 1;
/// 端点返回的最近事件序列上限（内存环形缓冲）。
const LEASE_DIAGNOSTICS_RING_CAP: usize = 200;
/// durable 文件滚动上限（超限保留最近半数行，原子 rename 重写）。
const LEASE_DIAGNOSTICS_FILE_CAP_BYTES: u64 = 1024 * 1024;

/// 分区内互斥（usage-diagnostics 同款进程内 Mutex；跨进程单写者由部署层保证）。
static LEASE_DIAGNOSTICS_LOG_MUTEX: StdMutex<()> = StdMutex::new(());

/// 会话级租约诊断分区路径（manager 创建与诊断端点 durable 尾读共用同一命名）。
pub(crate) fn lease_diagnostics_path(
    lifecycle: &LifecycleStore,
    session_id: &str,
) -> Option<PathBuf> {
    lifecycle
        .workspace_timeline_root_for_session(session_id)
        .ok()
        .map(|root| root.join(LEASE_DIAGNOSTICS_FILE))
}

#[derive(Serialize)]
struct LeaseDiagnosticsRecord {
    schema_version: u32,
    recorded_at: String,
    workspace_session_id: String,
    event: &'static str,
    connection_id: String,
    role: &'static str,
    reason: &'static str,
    from_holder: Option<String>,
    to_holder: Option<String>,
    epoch: u64,
}

/// 每会话一个的租约诊断写入面：内存环形缓冲（端点最近序列）+ durable jsonl
/// （append-only、滚动上限）。所有失败静默——REQ-DLS-03 打点失败零影响仲裁。
pub(super) struct LeaseDiagnostics {
    session_id: String,
    path: Option<PathBuf>,
    ring: StdMutex<VecDeque<Value>>,
    max_file_bytes: u64,
}

impl LeaseDiagnostics {
    pub(super) fn new(session_id: &str, path: Option<PathBuf>) -> Self {
        Self {
            session_id: session_id.to_string(),
            path,
            ring: StdMutex::new(VecDeque::new()),
            max_file_bytes: LEASE_DIAGNOSTICS_FILE_CAP_BYTES,
        }
    }
    /// 仅供单测注入滚动上限。
    #[cfg(test)]
    pub(super) fn with_file_cap(mut self, max_file_bytes: u64) -> Self {
        self.max_file_bytes = max_file_bytes;
        self
    }

    /// hello(driver) 显式获取（含接管：from_holder 为旧持有者）。
    pub(super) fn record_hold(&self, connection_id: &str, from_holder: Option<&str>, epoch: u64) {
        self.emit(Self::record(
            "hold",
            connection_id,
            "driver",
            "hello",
            from_holder,
            Some(connection_id),
            epoch,
        ));
    }

    /// REQ-DLS-01 写时自愈授予（仅 holder=None 悬空态）。
    pub(super) fn record_self_heal(&self, connection_id: &str, epoch: u64) {
        self.emit(Self::record(
            "self_heal",
            connection_id,
            "driver",
            "holder_none_first_write",
            None,
            Some(connection_id),
            epoch,
        ));
    }

    /// holder 连接关闭释放。
    pub(super) fn record_release(&self, connection_id: &str, epoch: u64) {
        self.emit(Self::record(
            "release",
            connection_id,
            "driver",
            "connection_closed",
            Some(connection_id),
            None,
            epoch,
        ));
    }

    /// Driver 写面拒绝（含当时持有者快照，可定案偷窃者身份）。
    pub(super) fn record_write_rejected_stale(
        &self,
        connection_id: &str,
        holder: Option<&str>,
        epoch: u64,
    ) {
        self.emit(Self::record(
            "write_rejected_stale",
            connection_id,
            "driver",
            "lease_mismatch",
            holder,
            holder,
            epoch,
        ));
    }

    /// observer 写面拒绝（既有语义，语义不变性钉子）。
    pub(super) fn record_write_rejected_observer(&self, connection_id: &str) {
        self.emit(Self::record(
            "write_rejected_observer",
            connection_id,
            "observer",
            "observer_write_not_allowed",
            None,
            None,
            0,
        ));
    }

    fn record(
        event: &'static str,
        connection_id: &str,
        role: &'static str,
        reason: &'static str,
        from_holder: Option<&str>,
        to_holder: Option<&str>,
        epoch: u64,
    ) -> LeaseDiagnosticsRecord {
        LeaseDiagnosticsRecord {
            schema_version: LEASE_DIAGNOSTICS_SCHEMA_VERSION,
            recorded_at: String::new(),
            workspace_session_id: String::new(),
            event,
            connection_id: connection_id.to_string(),
            role,
            reason,
            from_holder: from_holder.map(str::to_string),
            to_holder: to_holder.map(str::to_string),
            epoch,
        }
    }

    fn emit(&self, mut record: LeaseDiagnosticsRecord) {
        record.schema_version = LEASE_DIAGNOSTICS_SCHEMA_VERSION;
        record.recorded_at = chrono::Utc::now().to_rfc3339();
        record.workspace_session_id = self.session_id.clone();
        let Ok(value) = serde_json::to_value(&record) else {
            return;
        };
        if let Ok(mut ring) = self.ring.lock() {
            ring.push_back(value.clone());
            while ring.len() > LEASE_DIAGNOSTICS_RING_CAP {
                ring.pop_front();
            }
        }
        self.append_durable(&value);
    }

    /// 端点返回的最近事件序列（内存环形缓冲快照）。
    pub(super) fn recent(&self) -> Vec<Value> {
        self.ring
            .lock()
            .map(|ring| ring.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn append_durable(&self, value: &Value) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let Ok(_guard) = LEASE_DIAGNOSTICS_LOG_MUTEX.lock() else {
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
        // 优先保住「最近转移序列」。旋转后文件 ≤ cap/2 + 单行长度。
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

    /// 滚动上限：超限后文件保持有界，且最近事件仍在、最早事件被滚出。
    #[test]
    fn lease_diagnostics_file_respects_rolling_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(LEASE_DIAGNOSTICS_FILE);
        let diagnostics =
            LeaseDiagnostics::new("session_roll", Some(path.clone())).with_file_cap(600);
        for index in 0..40 {
            diagnostics.record_hold(&format!("connection-{index:03}"), None, index);
        }
        let content = std::fs::read_to_string(&path).expect("rotated jsonl");
        assert!(
            (content.len() as u64) <= 600,
            "滚动后文件必须保持有界，got {} bytes",
            content.len()
        );
        assert!(
            content.contains("connection-039"),
            "最近事件必须保留：{}",
            content
        );
        assert!(
            !content.contains("connection-000"),
            "最早事件应被滚出：{}",
            content
        );
    }

    /// 打点失败零影响：durable 通道不可写（路径被目录占位）时静默降级，
    /// 内存环形缓冲照常记录。
    #[test]
    fn lease_diagnostics_swallows_durable_failure_and_keeps_ring() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(LEASE_DIAGNOSTICS_FILE);
        std::fs::create_dir(&path).expect("occupy path with a directory");
        let diagnostics = LeaseDiagnostics::new("session_broken", Some(path));
        diagnostics.record_hold("conn-a", None, 1);
        diagnostics.record_self_heal("conn-a", 2);
        let recent = diagnostics.recent();
        assert_eq!(
            recent
                .iter()
                .map(|event| event["event"].as_str().unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["hold", "self_heal"],
            "durable 失败不得影响内存缓冲与调用方"
        );
    }
}
