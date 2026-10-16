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

/// 分区内互斥串行化（P1-5，先例 coding_attempt_store/role_run_event.rs）：
/// **进程内** Mutex 包住「seq 分配+读尾行+append」全临界区。仅保证单进程内
/// 串行；跨进程并发写同一分区需部署层保证单写者（本实现不提供跨进程锁，
/// 见 `next_tool_policy_role_run_seq` 的单写者约束说明）。
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
        // D1.3b（plan §0.4）：seq 分配是 durable 写（marker 落盘/分区目录
        // 创建），scope 准入在日志 Mutex 之前取得；purging/deleted 会话拒绝
        // 分配，分区目录不复活。
        self.with_workspace_session_live_write(workspace_session_id, || {
            Ok(self.next_tool_policy_role_run_seq_in_admission(workspace_session_id))
        })
        .map_err(|error| {
            ToolPolicyAuditError::Io(format!("workspace session admission: {error}"))
        })?
    }

    /// `next_tool_policy_role_run_seq` 的已准入内层（D1.3b：外层已持 scope
    /// 共享准入/已按删除事实拒，本函数不重复取 scope）。
    fn next_tool_policy_role_run_seq_in_admission(
        &self,
        workspace_session_id: &str,
    ) -> Result<u64, ToolPolicyAuditError> {
        // P1-5：与 append 同锁互斥。P1-6：分配即持久化——高水位 marker 追加式
        // 落盘，provider_start 写失败/崩溃后 seq 不得被复用。🔴 单写者约束：
        // 互斥与高水位均为**进程内**语义（进程内 Mutex+marker 高水位）；跨进程
        // 并发分配依赖部署层保证同一 workspace 分区单写者，本实现不提供跨进程
        // 锁，不宣称跨进程单调。既有无 marker 分区回退按文件 max 推导（兼容）。
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

    /// Task 1c(carry④/「装配与生命周期」):run-bound launch audit 的统一
    /// 装配点——分配 run-bound `role_run_seq`(与 1b split run handle 同一
    /// 分配器)并组装 `ProviderLaunchAuditContext`(audit_sink=本 store 的
    /// 生产 sink)。coordinator 生产臂等 LC caller 经此复用同一装配,替代
    /// 各调用点手写 seq 分配/sink 绑定,防组装漂移。
    pub fn allocate_launch_audit_context(
        &self,
        workspace_session_id: &str,
    ) -> Result<
        crate::product::logical_codebase::provider_gateway::ProviderLaunchAuditContext,
        ToolPolicyAuditError,
    > {
        let role_run_seq = self.next_tool_policy_role_run_seq(workspace_session_id)?;
        Ok(
            crate::product::logical_codebase::provider_gateway::ProviderLaunchAuditContext {
                workspace_session_id: workspace_session_id.to_string(),
                role_run_seq,
                audit_sink: std::sync::Arc::new(self.clone()),
            },
        )
    }

    /// D1.3b（fix-cascade-deletion，plan §0.2/§0.4）：session 键控辅助写的
    /// 迟到写准入桥——门/checkpoint/audit/usage/split-run 等按
    /// `workspace_session_id` 定位的 durable 写共用。归属归约顺序（真实
    /// 归属，不以 prompt/run_ref 猜 owner）：
    /// ① D1.0 会话身份占用索引（显式 id 创建路径落盘；删除后仍可反查，
    ///    墓碑 fail-closed）；
    /// ② session 记录归约（`resolve_session_scope_lenient`，档案缺失→None）。
    /// 两者都未命中（旧无 owner 档案）→ 不准入，保持原行为。准入在任何
    /// session/对象锁与日志 Mutex 之前取得（§0.4 锁序：scope → 日志 Mutex）。
    pub fn with_workspace_session_live_write<T>(
        &self,
        workspace_session_id: &str,
        operation: impl FnOnce() -> Result<T, crate::product::json_store::ProductStoreError>,
    ) -> Result<T, crate::product::json_store::ProductStoreError> {
        use crate::product::json_store::ProductStoreError;
        let scope = match self
            .entity_operation_store()
            .scope_for_workspace_session(workspace_session_id)
        {
            Ok(scope) => Some(scope),
            Err(ProductStoreError::NotFound { .. }) => None,
            Err(ProductStoreError::Conflict { kind, .. }) if kind == "entity_deleted" => {
                // 身份索引命中且墓碑在位（purging/deleted）：迟到写按删除事实拒。
                return Err(ProductStoreError::Conflict {
                    kind: "entity_deleting",
                    id: workspace_session_id.to_string(),
                });
            }
            Err(other) => return Err(other),
        };
        let scope = match scope {
            Some(scope) => Some(scope),
            None => self.resolve_session_scope_lenient(workspace_session_id)?,
        };
        self.with_optional_entity_live_write(scope.as_ref(), operation)
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
        // D1.3b（plan §0.4）：迟到写防御——scope 准入在进程内日志 Mutex 之前
        // 取得；purging/deleted 会话拒绝追加且分区目录不复活。原 P1-5 互斥与
        // provider_start 首行/唯一、seq 单调合同零变化。
        self.with_workspace_session_live_write(workspace_session_id, || {
            Ok(self.append_in_admission(workspace_session_id, role_run_seq, event))
        })
        .map_err(|error| {
            ToolPolicyAuditError::Io(format!("workspace session admission: {error}"))
        })?
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
    /// `append` 的已准入内层（D1.3b：外层已持 scope 共享准入/已按删除事实拒）。
    fn append_in_admission(
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

    /// B1a（Task 4.1，设计 §5.2）：audit 兜底 clean-cutover——旧
    /// `latest_provider_start_native_id` 的 typed 替代。在 workspace 分区
    /// 内按 (provider, role) 扫描 provider_start，返回最新**有恢复资格**
    /// 候选 {native_id, role_run_seq}（resume 兜底：会话引用缺失时以
    /// spawn 时点审计还原原生 resume，provider 匹配接受 snake/dash 两种
    /// canonical 形态，与审计写入侧一致）。
    ///
    /// 恢复资格过滤（clean-cutover 语义，不再取回已吊销 id）：
    /// - 该 run 文件已含 `session_terminated`（旧 run 被 superseded/终止，
    ///   旧 native 不复活、不捞更旧代）；
    /// - native id 命中调用方传入的 durable tombstone 集合。
    ///
    /// 有 (provider, role) 匹配但全部无资格 → `AllSuperseded`（调用方进
    /// 显式等待，不当出生 fresh）；无任何匹配 → `NoMatch`（事实读取确证
    /// 为空）。
    pub fn latest_provider_start_resume_candidate(
        &self,
        workspace_session_id: &str,
        provider_canonical: &str,
        role: &str,
        superseded_native_ids: &std::collections::BTreeSet<String>,
    ) -> Result<ProviderStartCandidateLookup, ToolPolicyAuditError> {
        let _guard = lock_audit_log()?;
        let root = self.tool_policy_audit_workspace_root(workspace_session_id)?;
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ProviderStartCandidateLookup::NoMatch);
            }
            Err(error) => return Err(audit_error(error)),
        };
        let dashed = provider_canonical.replace('_', "-");
        let mut matched = false;
        let mut eligible: Option<(u64, String)> = None;
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
            let mut run_provider_start: Option<String> = None;
            let mut run_terminated = false;
            for line in lines {
                match line.event {
                    DurableToolPolicyEvent::ProviderStart(record) => {
                        if record.role == role
                            && (record.provider == provider_canonical || record.provider == dashed)
                            && !record.provider_session_id.trim().is_empty()
                        {
                            matched = true;
                            run_provider_start = Some(record.provider_session_id.clone());
                        }
                    }
                    DurableToolPolicyEvent::SessionTerminated(_) => {
                        run_terminated = true;
                    }
                    _ => {}
                }
            }
            let Some(native_id) = run_provider_start else {
                continue;
            };
            if run_terminated || superseded_native_ids.contains(native_id.trim()) {
                continue;
            }
            if eligible.as_ref().is_none_or(|(seq, _)| role_run_seq > *seq) {
                eligible = Some((role_run_seq, native_id));
            }
        }
        Ok(match eligible {
            Some((role_run_seq, native_id)) => {
                ProviderStartCandidateLookup::Eligible(ProviderStartResumeCandidate {
                    role_run_seq,
                    native_id,
                })
            }
            None if matched => ProviderStartCandidateLookup::AllSuperseded,
            None => ProviderStartCandidateLookup::NoMatch,
        })
    }
}

/// B1a（Task 4.1）：audit 兜底候选查找结果——区分「确证为空」与
/// 「有历史但全部吊销」（后者不得当出生 fresh）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStartCandidateLookup {
    /// 最新有恢复资格的 provider_start（native id + 其 run 的 role_run_seq）。
    Eligible(ProviderStartResumeCandidate),
    /// 有 (provider, role) 匹配的 provider_start，但全部已终止/吊销——
    /// 旧代不复活，也不能当出生 fresh（调用方显式等待）。
    AllSuperseded,
    /// 无任何匹配 provider_start（事实读取确证为空）。
    NoMatch,
}

/// B1a（Task 4.1）：audit 兜底 typed 候选 {native_id, role_run_seq}——
/// LC 候选的 `role_run_seq` 必须来自真实 run 绑定（文件 stem），不伪造。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderStartResumeCandidate {
    pub role_run_seq: u64,
    pub native_id: String,
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

    /// Task 1c:run-bound launch audit 装配点——seq 分配与 1b split handle
    /// 同一分配器(同 workspace 单调递增),产出的 context sink 可直接收口
    /// provider_start(run-bound append 首行)。
    #[test]
    fn allocate_launch_audit_context_binds_run_bound_seq_and_production_sink() {
        use crate::cross_cutting::tool_policy_audit::RoleRunBoundAuditSink;
        use crate::product::app_paths::ProductAppPaths;

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));

        let first = store
            .allocate_launch_audit_context("ws-lc-ctx")
            .expect("allocate first launch audit context");
        assert_eq!(first.workspace_session_id, "ws-lc-ctx");
        assert_eq!(first.role_run_seq, 0, "fresh workspace 首个 seq 为 0");

        // 生产 sink 直接收口 provider_start:run-bound append 恰为首行。
        RoleRunBoundAuditSink::new(first.audit_sink.clone(), "ws-lc-ctx", first.role_run_seq)
            .into_sink()
            .append_bound(DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
                provider: "claude".to_string(),
                provider_session_id: "fake-session".to_string(),
                ..ProviderStartAudit::default()
            }))
            .expect("bound append writes provider_start");

        let second = store
            .allocate_launch_audit_context("ws-lc-ctx")
            .expect("allocate second launch audit context");
        assert_eq!(
            second.role_run_seq, 1,
            "同一 workspace 的 run-bound seq 必须单调递增(与 split handle 同一分配器)"
        );

        let lines = store
            .read_tool_policy_lines("ws-lc-ctx", 0)
            .expect("read back first run");
        assert_eq!(
            lines.first().map(|line| line.event_type()),
            Some("provider_start")
        );
    }
}
