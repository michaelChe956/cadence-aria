//! LifecycleStore 的 durable tool-policy-run-audit 分区（Task 3.2，REQ-ENV-09/GC10-11）。
//!
//! 策略角色（pi/claude/codex）使用本分区（`tool-policy-run-audit/`）；Coder、非策略
//! 路径与 kimi 继续使用既有 `execution_event_audit`，两者严格分离（GC10）。
//! 文件 key=`(workspace_session_id, role_run_seq)`，append-only JSONL；
//! `provider_start` 恰为首行且唯一，行 `seq` 从约定起点（0）单调递增；读取坏行
//! 跳过并返回内存告警（不写回 durable 分区）；可解析行违反结构不变量
//! （schema 版本/seq 起点+严格递增/首行 provider_start 唯一）视为结构性损坏，
//! 读取返回 `CorruptAuditFile` fail-closed（写入端据此拒绝追加）；写入失败传播错误。

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::cross_cutting::tool_policy_audit::{
    DurableToolPolicyEvent, ProviderStartAudit, StoredProviderStart,
    TOOL_POLICY_RUN_AUDIT_PARTITION, ToolPolicyAuditError, ToolPolicyAuditLine,
    ToolPolicyAuditReadResult, ToolPolicyAuditReadWarning, ToolPolicyAuditSink,
};
use crate::product::json_store::validate_relative_id;

use super::LifecycleStore;

/// 分区内互斥串行化（P1-5，先例 coding_attempt_store/role_run_event.rs）：包住
/// 「seq 分配+读尾行+append」全临界区。契约⑥单写者假设，无需跨进程锁。
static TOOL_POLICY_AUDIT_LOG_MUTEX: Mutex<()> = Mutex::new(());

/// seq 分配高水位 marker 文件名（追加式 JSONL；崩溃安全：读取取最后可解析行）。
const ROLE_RUN_SEQ_MARKER: &str = "role-run-seq.jsonl";

fn lock_audit_log() -> Result<std::sync::MutexGuard<'static, ()>, ToolPolicyAuditError> {
    TOOL_POLICY_AUDIT_LOG_MUTEX
        .lock()
        .map_err(|error| ToolPolicyAuditError::Io(format!("lock tool policy audit log: {error}")))
}

fn audit_error(error: std::io::Error) -> ToolPolicyAuditError {
    ToolPolicyAuditError::Io(error.to_string())
}

fn validate_audit_identifier(value: &str) -> Result<(), ToolPolicyAuditError> {
    validate_relative_id(value)
        .map_err(|_| ToolPolicyAuditError::InvalidIdentifier(value.to_string()))
}

impl LifecycleStore {
    /// `(workspace_session_id, role_run_seq)` → 分区文件路径。
    fn tool_policy_audit_file(
        &self,
        workspace_session_id: &str,
        role_run_seq: u64,
    ) -> Result<PathBuf, ToolPolicyAuditError> {
        validate_audit_identifier(workspace_session_id)?;
        Ok(self
            .paths
            .root()
            .join(TOOL_POLICY_RUN_AUDIT_PARTITION)
            .join(workspace_session_id)
            .join(format!("{role_run_seq}.jsonl")))
    }

    /// workspace 分区目录路径。
    fn tool_policy_audit_workspace_root(
        &self,
        workspace_session_id: &str,
    ) -> Result<PathBuf, ToolPolicyAuditError> {
        validate_audit_identifier(workspace_session_id)?;
        Ok(self
            .paths
            .root()
            .join(TOOL_POLICY_RUN_AUDIT_PARTITION)
            .join(workspace_session_id))
    }

    /// 读取 durable 分区文件：可解析行逐行结构校验（F3 最终审）——
    /// `schema_version` 必须为当前冻结版本、行 `seq` 从约定起点（0，与 append
    /// 首行分配 `unwrap_or(0)` 同源）严格递增、`provider_start` 恰为首行且唯一；
    /// 任一违反视为结构性损坏，返回 `CorruptAuditFile` fail-closed（写入端据此
    /// 拒绝在损坏文件上追加）。坏 JSON 行仍为跳过+内存告警（既有行为，不写回
    /// 分区；告警行不参与结构校验，两侧可解析行间 seq 只需严格递增）。
    pub fn read_tool_policy_lines_with_warnings(
        &self,
        workspace_session_id: &str,
        role_run_seq: u64,
    ) -> Result<ToolPolicyAuditReadResult, ToolPolicyAuditError> {
        use crate::cross_cutting::tool_policy_audit::TOOL_POLICY_AUDIT_SCHEMA_VERSION;

        let path = self.tool_policy_audit_file(workspace_session_id, role_run_seq)?;
        let mut result = ToolPolicyAuditReadResult::default();
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(result),
            Err(error) => return Err(audit_error(error)),
        };
        // F3：结构校验状态——首行必须 provider_start 且 seq==0，后续行 seq 严格
        // 递增且不再出现 provider_start（唯一）。
        let mut last_seq: Option<u64> = None;
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line_no = index as u32 + 1;
            let line = line.map_err(audit_error)?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ToolPolicyAuditLine>(&line) {
                Ok(parsed) => {
                    if parsed.schema_version != TOOL_POLICY_AUDIT_SCHEMA_VERSION {
                        return Err(ToolPolicyAuditError::CorruptAuditFile { line_no });
                    }
                    let is_provider_start = parsed.event_type() == "provider_start";
                    match last_seq {
                        None => {
                            if parsed.seq != 0 || !is_provider_start {
                                return Err(ToolPolicyAuditError::CorruptAuditFile { line_no });
                            }
                        }
                        Some(previous) => {
                            if parsed.seq <= previous || is_provider_start {
                                return Err(ToolPolicyAuditError::CorruptAuditFile { line_no });
                            }
                        }
                    }
                    last_seq = Some(parsed.seq);
                    result.events.push(parsed);
                }
                Err(_) => result.warnings.push(ToolPolicyAuditReadWarning {
                    reason_code: "invalid_json_line".to_string(),
                    line_no,
                }),
            }
        }
        Ok(result)
    }

    /// 读取 durable 分区文件（坏行静默跳过）。
    pub fn read_tool_policy_lines(
        &self,
        workspace_session_id: &str,
        role_run_seq: u64,
    ) -> Result<Vec<ToolPolicyAuditLine>, ToolPolicyAuditError> {
        Ok(self
            .read_tool_policy_lines_with_warnings(workspace_session_id, role_run_seq)?
            .events)
    }

    /// durable 分区文件是否包含指定事件类型（读取告警不写回分区）。
    pub fn contains_tool_policy_event(
        &self,
        workspace_session_id: &str,
        role_run_seq: u64,
        event_type: &str,
    ) -> bool {
        self.read_tool_policy_lines(workspace_session_id, role_run_seq)
            .map(|lines| lines.iter().any(|line| line.event_type() == event_type))
            .unwrap_or(false)
    }

    /// 为 workspace 分配下一个 `role_run_seq`（扫描既有分区文件取 max+1）。
    /// 分配的 seq 随 run 落盘持久化：该 run 的 `provider_start` 首行即 durable 记录。
    pub fn next_tool_policy_role_run_seq(
        &self,
        workspace_session_id: &str,
    ) -> Result<u64, ToolPolicyAuditError> {
        // P1-5：与 append 同锁互斥。P1-6：分配即持久化——高水位 marker 追加式
        // 落盘，provider_start 写失败/崩溃后 seq 不得被复用；跨进程依 marker
        // 单调。既有无 marker 分区回退按文件 max 推导（兼容）。
        let _guard = lock_audit_log()?;
        let root = self.tool_policy_audit_workspace_root(workspace_session_id)?;
        let marker = root.join(ROLE_RUN_SEQ_MARKER);
        let marker_high = read_role_run_seq_marker(&marker)?;
        let file_high = scan_max_role_run_seq(&root)?;
        let next = marker_high.max(file_high).map(|seq| seq + 1).unwrap_or(0);
        if let Some(parent) = marker.parent() {
            std::fs::create_dir_all(parent).map_err(audit_error)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&marker)
            .map_err(audit_error)?;
        writeln!(file, "{{\"allocated\":{next}}}").map_err(audit_error)?;
        file.flush().map_err(audit_error)?;
        Ok(next)
    }
}

/// marker 高水位（坏行跳过取最大可解析值；缺失→ None）。
fn read_role_run_seq_marker(marker: &std::path::Path) -> Result<Option<u64>, ToolPolicyAuditError> {
    let file = match std::fs::File::open(marker) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(audit_error(error)),
    };
    let mut high: Option<u64> = None;
    for line in BufReader::new(file).lines() {
        let line = line.map_err(audit_error)?;
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line)
            && let Some(allocated) = value.get("allocated").and_then(|value| value.as_u64())
            && high.is_none_or(|current| allocated > current)
        {
            high = Some(allocated);
        }
    }
    Ok(high)
}

/// 分区文件 max role_run_seq（非数字 stem 跳过，marker 不参与）。
fn scan_max_role_run_seq(root: &std::path::Path) -> Result<Option<u64>, ToolPolicyAuditError> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(audit_error(error)),
    };
    let mut max: Option<u64> = None;
    for entry in entries {
        let entry = entry.map_err(audit_error)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".jsonl") else {
            continue;
        };
        if let Ok(seq) = stem.parse::<u64>()
            && max.is_none_or(|current| seq > current)
        {
            max = Some(seq);
        }
    }
    Ok(max)
}

impl ToolPolicyAuditSink for LifecycleStore {
    fn append(
        &self,
        workspace_session_id: &str,
        role_run_seq: u64,
        mut event: DurableToolPolicyEvent,
    ) -> Result<(), ToolPolicyAuditError> {
        // P1-5：进程内互斥包住「读尾行+seq 分配+append」全临界区。
        let _guard = lock_audit_log()?;
        let path = self.tool_policy_audit_file(workspace_session_id, role_run_seq)?;
        // D6 冻结：`workspace_session_id` 进入事件 DTO，且以文件 key 为准（落盘
        // 记录与所在分区位置永远一致，不受调用方填充遗漏影响）。
        if let DurableToolPolicyEvent::ProviderStart(record) = &mut event {
            record.workspace_session_id = workspace_session_id.to_string();
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(audit_error)?;
        }
        // append 前读取既有行：provider_start 首行/唯一约束与 seq 单调分配都基于
        // durable 现状计算。P2-3：既有文件存在坏行（含首行不可解析）即损坏，
        // fail-closed 拒绝追加，不得在损坏文件上继续写。
        let existing_result =
            self.read_tool_policy_lines_with_warnings(workspace_session_id, role_run_seq)?;
        if let Some(warning) = existing_result.warnings.first() {
            return Err(ToolPolicyAuditError::CorruptAuditFile {
                line_no: warning.line_no,
            });
        }
        let existing = existing_result.events;
        if matches!(event, DurableToolPolicyEvent::ProviderStart(_)) {
            if existing
                .iter()
                .any(|line| line.event_type() == "provider_start")
            {
                return Err(ToolPolicyAuditError::DuplicateProviderStart);
            }
            if !existing.is_empty() {
                return Err(ToolPolicyAuditError::ProviderStartRequired);
            }
        } else if !existing
            .first()
            .is_some_and(|line| line.event_type() == "provider_start")
        {
            return Err(ToolPolicyAuditError::ProviderStartRequired);
        }
        let next_seq = existing.last().map(|line| line.seq + 1).unwrap_or(0);
        let serialized = serde_json::to_string(&ToolPolicyAuditLine::from_event(next_seq, event))
            .map_err(|error| ToolPolicyAuditError::Serde(error.to_string()))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(audit_error)?;
        writeln!(file, "{serialized}").map_err(audit_error)?;
        file.flush().map_err(audit_error)?;
        Ok(())
    }

    fn find_provider_start(
        &self,
        _native_provider_session_id: &str,
    ) -> Result<Option<StoredProviderStart>, ToolPolicyAuditError> {
        // 裸 sink 的检索需显式 workspace 维度；bound 装饰器总是走
        // `find_provider_start_by_session`，本入口仅对未绑定调用方兜底。
        Ok(None)
    }

    fn find_provider_start_by_session(
        &self,
        workspace_session_id: &str,
        native_provider_session_id: &str,
    ) -> Result<Option<StoredProviderStart>, ToolPolicyAuditError> {
        self.find_latest_tool_policy_provider_start(
            workspace_session_id,
            native_provider_session_id,
        )
    }
}

impl LifecycleStore {
    /// resume 检索（Task 3.3）：在 workspace 分区内按原生 provider session id
    /// 扫描最近 provider_start（max role_run_seq），返回记录及其所在 run 文件
    /// 定位（P1-4：drift 的 superseded 事件追加到被取代旧 run 的文件）。
    /// 同时覆写 sink trait 默认实现。
    pub fn find_latest_tool_policy_provider_start(
        &self,
        workspace_session_id: &str,
        native_provider_session_id: &str,
    ) -> Result<Option<StoredProviderStart>, ToolPolicyAuditError> {
        // P1-5：与 append 同锁互斥，读到一致的分区快照。
        let _guard = lock_audit_log()?;
        let root = self.tool_policy_audit_workspace_root(workspace_session_id)?;
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(audit_error(error)),
        };
        // 逐文件（role_run_seq 降序不必：取 max seq 的匹配 provider_start）扫描。
        let mut matched: Option<(u64, ProviderStartAudit)> = None;
        for entry in entries {
            let entry = entry.map_err(audit_error)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".jsonl") else {
                continue;
            };
            let Ok(role_run_seq) = stem.parse::<u64>() else {
                continue;
            };
            let lines = self.read_tool_policy_lines(workspace_session_id, role_run_seq)?;
            for line in lines {
                if let DurableToolPolicyEvent::ProviderStart(record) = line.event
                    && record.provider_session_id == native_provider_session_id
                    && matched.as_ref().is_none_or(|(seq, _)| role_run_seq > *seq)
                {
                    matched = Some((role_run_seq, record));
                }
            }
        }
        Ok(matched.map(|(role_run_seq, record)| StoredProviderStart {
            workspace_session_id: workspace_session_id.to_string(),
            role_run_seq,
            record,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_store_sink_implements_run_bound_append_via_decorator() {
        use crate::cross_cutting::tool_policy_audit::RoleRunBoundAuditSink;
        use crate::product::app_paths::ProductAppPaths;

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let bound = RoleRunBoundAuditSink::new(std::sync::Arc::new(store.clone()), "ws-bound", 3)
            .into_sink();
        bound
            .append_bound(DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
                provider: "codex".to_string(),
                ..ProviderStartAudit::default()
            }))
            .expect("bound append writes provider_start");
        let lines = store
            .read_tool_policy_lines("ws-bound", 3)
            .expect("read back");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].event_type(), "provider_start");

        // 裸 sink 的 append_bound 必须 fail-closed（未绑定 run 标识）。
        assert!(matches!(
            store.append_bound(DurableToolPolicyEvent::SessionTerminated(
                crate::cross_cutting::tool_policy_audit::SessionTerminatedAudit {
                    reason_code: "x".to_string(),
                }
            )),
            Err(ToolPolicyAuditError::UnboundSink)
        ));
    }
}
