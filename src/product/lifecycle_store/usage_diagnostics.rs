//! LifecycleStore 的 usage durable 落盘失败诊断分区（REQ-NDR-05）。
//!
//! 节点 detail 通道本身不可写（节点缺失 / detail 文件损坏 / 写盘失败）时，usage 事件
//! 与节点 detail 级诊断事件都无处可写——此时降级到会话 timeline 根的 append-only
//! JSONL（`usage-diagnostics.jsonl`）：只追加、从不读取，因此不受损坏的 node detail
//! 文件影响，脚本可事后检索（区分「无 usage 数据」与「落盘失败」）。
//!
//! 行格式：单行 JSON；`schema_version` 固定 1，字段构造见
//! `workspace_engine::provider_drive::usage_diagnostic::usage_persist_diagnostic_record`。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::product::json_store::{ProductStoreError, validate_relative_id};

use super::LifecycleStore;

/// 会话 timeline 根下的 durable usage 诊断分区文件名（事后检索脚本契约）。
pub(crate) const USAGE_DIAGNOSTICS_FILE: &str = "usage-diagnostics.jsonl";

/// 分区内互斥串行化（先例 `tool_policy_run_audit` / `evidence_audit` 的进程内 Mutex）：
/// 仅保证单进程内「定位路径 + 追加」临界区串行；跨进程并发写同一分区由部署层
/// 单写者假设保证（本实现不提供跨进程锁）。
static USAGE_DIAGNOSTICS_LOG_MUTEX: Mutex<()> = Mutex::new(());

impl LifecycleStore {
    /// 追加一条 usage 落盘失败诊断到会话级 `usage-diagnostics.jsonl`。
    ///
    /// append-only：不读取既有内容（既有文件损坏不影响追加），写入失败传播错误，
    /// 由调用方降级为结构化 warn（REQ-NDR-05 的最后手段）。
    pub(crate) fn append_usage_persistence_diagnostic(
        &self,
        session_id: &str,
        record: &serde_json::Value,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(session_id)?;
        let _guard = USAGE_DIAGNOSTICS_LOG_MUTEX
            .lock()
            .map_err(|error| ProductStoreError::Io(format!("usage diagnostics lock: {error}")))?;
        let path = self.usage_diagnostics_path(session_id)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                ProductStoreError::Io(format!("create {}: {error}", parent.display()))
            })?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| ProductStoreError::Io(format!("open {}: {error}", path.display())))?;
        serde_json::to_writer(&mut file, record)
            .map_err(|error| ProductStoreError::Json(error.to_string()))?;
        file.write_all(b"\n")
            .map_err(|error| ProductStoreError::Io(format!("write {}: {error}", path.display())))?;
        file.flush()
            .map_err(|error| ProductStoreError::Io(format!("flush {}: {error}", path.display())))?;
        Ok(())
    }

    /// 会话级 usage 诊断 jsonl 路径（调用方与事后调查脚本共用同一命名）。
    pub(crate) fn usage_diagnostics_path(
        &self,
        session_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        Ok(self
            .workspace_timeline_root_for_session(session_id)?
            .join(USAGE_DIAGNOSTICS_FILE))
    }
}
