//! REQ-WSC-25（owner 2026-10-31「不可控性太高」）：候选生成/编译尝试的
//! **append-only 流水**。
//!
//! 落点：**issue 级单文件** `projects/{project}/issues/{issue}/candidate-attempts.jsonl`
//! ——每次候选生成/编译尝试追加一行，永不覆盖。
//!
//! ## 为什么落 issue 级（2026-10-31 真机修订）
//!
//! 原落点是会话目录 `workspace-sessions/{session_id}/candidate-attempts.jsonl`。
//! 但「终止自动化 + 重开」会**删掉旧会话目录**（连带其流水），于是通过率读数
//! 被重跑擦掉：同一个 issue 上一次跑 `first_pass_rate=1.0`，重跑一次后变成
//! `0.0`——上一次那条 `outcome=passed` 的记录**被删没了**。通过率读数的意义
//! 就是**跨时间**看趋势，历史不该被重跑擦掉。
//!
//! 会话清理只删 `workspace-sessions/{session_id}/` 目录与同名记录
//! （`delete_workspace_sessions_for_entity` 与实体删除级联 ④ 皆如此），**不删**
//! issue 目录本身 → 流水落 issue 级即耐得住会话清理。
//!
//! 与既有 `candidate-compile-status.json` 的分工（两者互不替代、互不覆盖）：
//! - `candidate-compile-status.json` = **当前状态**：该会话最近一次尝试的覆盖写
//!   快照（编译成功即覆盖清除）。回答「现在这个候选是不是编译不过」。
//! - `candidate-attempts.jsonl` = **历史序列**：只追加。回答「同一 issue 连续
//!   失败 N 次分别死在哪个阶段、哪个稳定码、哪个结构指纹上」。
//!
//! 写失败只降级为静默：store 返回错误，由调用点吞掉并记入运行日志（policy
//! diagnostics），**不得**因此使候选生成或确认失败（REQ-WSC-25 场景 2）。
//!
//! 字段纪律：复用既有类型——`CandidateAutoRewrite`（自动重写阶段）、
//! `PlanningFailureKind`（typed 失败种类）、`CandidateCompileStatus` 的候选
//! 绑定 ref；不另造同义结构。
//!
//! ## 度量读数（可复制的查法）
//!
//! 程序化入口：[`compute_candidate_attempt_metrics`]（Rust API，口径见其文档）。
//! 命令行等价查法（jq，跨 issue 聚合；口径与 Rust API 一致）：
//!
//! ```bash
//! cat .aria/projects/*/issues/*/candidate-attempts.jsonl \
//! | jq -s --argjson n 5 '
//!   def pct($p): if length == 0 then null else .[(($p * length) | ceil) - 1] end;
//!   (group_by(.issue_id) | map(sort_by(.recorded_at, .attempt_seq))) as $issues
//!   | ([ $issues[] | (map(.outcome) as $o | (($o | index("passed")) // ($o | length))) ] | sort) as $consec
//!   | {
//!       issue_count: ($issues | length),
//!       first_pass_rate: (($issues | map(select(.[0].outcome == "passed")) | length) / ($issues | length)),
//!       consecutive_failure_p50: ($consec | pct(0.5)),
//!       consecutive_failure_p90: ($consec | pct(0.9)),
//!       failure_code_top_n: ([ $issues[][] | select(.outcome == "failed") | .failure.code ] | group_by(.) | map({code: .[0], count: length}) | sort_by(-.count, .code) | .[0:$n]),
//!       lowering_terminal_failure_count: ([ $issues[] | select(.[-1].outcome == "failed" and .[-1].failure.code == "lowering_error") ] | length)
//!     }'
//! ```

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::models::{
    CandidateAutoRewrite, CandidateCompileStatus, WorkspaceSessionRecord,
};
use crate::product::planning_failure::PlanningFailureKind;

use super::LifecycleStore;

/// 流水文件名（落 issue 目录：与门卡同族文件，但**不在会话目录内**——
/// 会话目录会被「终止+重开」清掉，流水必须活过它）。
pub const CANDIDATE_ATTEMPTS_FILE_NAME: &str = "candidate-attempts.jsonl";

/// lowering 类失败的稳定码（design §9.3「lowering 类终态失败数」的判据）。
pub const LOWERING_FAILURE_STABLE_CODE: &str = "lowering_error";

/// 进程内串行化（先例 `tool_policy_run_audit.rs`）：**进程内** Mutex 包住
/// 「读既有行 + 幂等判定 + 分配序号 + append」全临界区。跨进程并发写同一
/// 会话流水需部署层保证单写者（本实现不提供跨进程锁，不宣称跨进程单调）。
static CANDIDATE_ATTEMPT_LOG_MUTEX: Mutex<()> = Mutex::new(());

fn lock_attempt_log() -> Result<std::sync::MutexGuard<'static, ()>, ProductStoreError> {
    CANDIDATE_ATTEMPT_LOG_MUTEX
        .lock()
        .map_err(|error| ProductStoreError::Io(format!("lock candidate attempt log: {error}")))
}

/// 尝试所在阶段（design §9.2 `stage(structure|final)`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateAttemptStage {
    /// 候选结构门（结构复验）判定阶段。
    Structure,
    /// Final Compile 阶段。
    Final,
}

/// 一次尝试的结局。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateAttemptOutcome {
    /// 本次尝试编译通过。
    Passed,
    /// 本次尝试编译失败。
    Failed,
}

/// 一次失败的 typed 事实：内容寻址失败身份 + 稳定码 + typed 种类 + 环结构指纹。
///
/// `typed_kinds` 复用既有 `PlanningFailureKind`（结构失败集按序全量保留），
/// 不另造同义的「失败种类」结构。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateAttemptFailure {
    /// 内容寻址失败身份（`planning_compile_failure_<sha256>`）——同失败幂等同 id。
    pub failure_id: String,
    /// 稳定诊断码（如 `final_compile_prepare_failed` / `lowering_error`）。
    pub code: String,
    /// typed 失败种类（复用既有 `PlanningFailureKind`）。
    pub typed_kinds: Vec<PlanningFailureKind>,
    /// 依赖环结构指纹（跨尝试稳定，不随文案/行号变化；非环失败为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_fingerprint: Option<String>,
}

/// 等价规范化审计事件（kind / 原 key / 值 / 映射依据）——与尝试流水同序列。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquivalenceCanonicalization {
    /// 规范化类别（如 `intent_target_repository`）。
    pub kind: String,
    /// 规范化前模型写的 key。
    pub original_key: String,
    /// 规范化后采用的值。
    pub value: String,
    /// 映射依据（如成员表 `physical_by_logical`）。
    pub resolution_basis: String,
}

/// 一次候选生成/编译尝试的 durable 追加记录（append-only 流水的一行）。
///
/// 不使用 `deny_unknown_fields`：append-only 流水会随契约演进增字段，旧读取器
/// 必须仍能读历史行（先例 `planning_failure.rs` 的 serde 兼容纪律）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateAttemptRecord {
    /// 会话内尝试序号（1 起，单调递增）——**由 store 在落盘时分配**。
    pub attempt_seq: u32,
    /// 幂等键：同一次尝试重复落盘零膨胀（由 store 判重）。
    pub attempt_key: String,
    /// 会话标识——由 store 盖印（调用方不得自写）。
    pub session_id: String,
    /// 归属 issue 标识——由 store 盖印。
    pub issue_id: String,
    /// 落盘时间（RFC3339 UTC）。
    pub recorded_at: String,
    /// 尝试所在阶段。
    pub stage: CandidateAttemptStage,
    /// 尝试结局。
    pub outcome: CandidateAttemptOutcome,
    /// 本次尝试对应的 source revision 引用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_revision_ref: Option<String>,
    /// 本次尝试对应的候选 IR 引用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_candidate_ir_ref: Option<String>,
    /// 失败事实（成功尝试为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<CandidateAttemptFailure>,
    /// 自动重写阶段事实（复用既有 `CandidateAutoRewrite`：phase/remaining/repeated）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_rewrite: Option<CandidateAutoRewrite>,
    /// 本次尝试已应用的机械自动修复标识（顺序即应用序）。
    #[serde(default)]
    pub auto_repairs_applied: Vec<String>,
    /// 本次尝试发生的等价规范化审计事件。
    #[serde(default)]
    pub equivalence_canonicalizations: Vec<EquivalenceCanonicalization>,
}

/// 一个 issue 的尝试序列（度量聚合的输入单位）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateIssueAttemptSeries {
    /// issue 标识。
    pub issue_id: String,
    /// 该 issue 的全部尝试记录（内部按时间重排后使用）。
    pub records: Vec<CandidateAttemptRecord>,
}

/// 失败码分布条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateFailureCodeCount {
    /// 稳定诊断码。
    pub code: String,
    /// 出现次数（失败尝试数）。
    pub count: usize,
}

/// REQ-WSC-25 度量读数（design §9.3）。
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateAttemptMetrics {
    /// 参与统计的 issue 数（有尝试记录的 issue）。
    pub issue_count: usize,
    /// 首次尝试即通过的 issue 数。
    pub first_pass_issue_count: usize,
    /// 首次通过率（无样本时 None）。
    pub first_pass_rate: Option<f64>,
    /// 同 issue 连续失败次数 P50（首次生成→首次通过之间的失败尝试数）。
    pub consecutive_failure_p50: Option<f64>,
    /// 同 issue 连续失败次数 P90。
    pub consecutive_failure_p90: Option<f64>,
    /// 失败码分布（按次数降序，同数按码升序）。
    pub failure_code_ranking: Vec<CandidateFailureCodeCount>,
    /// lowering 类终态失败计数（终态失败码为 `lowering_error` 的 issue 数）。
    pub lowering_terminal_failure_count: usize,
}

impl LifecycleStore {
    /// 该 issue 的尝试流水路径（`issues/{issue_id}/candidate-attempts.jsonl`）。
    ///
    /// **issue 级单文件**（2026-10-31 真机修订）：落点从会话目录上移到 issue
    /// 目录——会话目录会被「终止自动化 + 重开」删掉（连带其流水），落会话
    /// 目录的读数会被重跑擦掉（同一 issue 上一次 `first_pass_rate=1.0`，重跑
    /// 一次后变成 0.0）。issue 目录不在会话清理范围内（清理只删
    /// `workspace-sessions/{sid}/` 目录与同名记录），故流水耐得住会话清理。
    fn issue_candidate_attempts_path(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join(CANDIDATE_ATTEMPTS_FILE_NAME)
    }

    /// REQ-WSC-25：追加一次候选生成/编译尝试（append-only，一行一次尝试）。
    ///
    /// 幂等：同一会话内同一 `attempt_key` 重复落盘**零膨胀**（不写第二行，
    /// 返回 false）。序号 `attempt_seq`、会话/issue 身份由 store 盖印（调用方
    /// 写错也不会错）。
    ///
    /// 返回 true＝本次真的追加了一行；false＝同键已存在（幂等空操作）。
    pub fn append_candidate_attempt(
        &self,
        session_id: &str,
        record: &CandidateAttemptRecord,
    ) -> Result<bool, ProductStoreError> {
        validate_relative_id(session_id)?;
        let (_, scope) = self.resolve_session_scope(session_id)?;
        let record = record.clone();
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let _guard = lock_attempt_log()?;
            let session = self.get_workspace_session(session_id)?;
            let path = self.issue_candidate_attempts_path(&session.project_id, &session.issue_id);
            let existing = read_candidate_attempt_lines(&path)?;
            // 幂等只看 **issue 级权威文件**（同会话同 attempt_key）：旧版落点
            // （会话目录）里的同键行不得把新行吃掉——否则那条新行会随会话
            // 清理一起消失（正是本次要修的「读数被擦掉」）。
            if existing
                .iter()
                .any(|item| item.session_id == session.id && item.attempt_key == record.attempt_key)
            {
                return Ok(false);
            }
            // 序号会话内单调递增；跨旧版落点续号（同一会话在升级前后各写过）。
            let legacy =
                read_candidate_attempt_lines(&self.legacy_session_attempts_path(&session))?;
            let mut persisted = record;
            persisted.attempt_seq = next_attempt_seq(&existing, &legacy, &session.id);
            persisted.session_id = session.id.clone();
            persisted.issue_id = session.issue_id.clone();
            append_candidate_attempt_line(&path, &persisted)?;
            Ok(true)
        })
    }

    /// REQ-WSC-25：读该会话的尝试流水（按 `attempt_seq` 升序；坏行跳过）。
    ///
    /// 语义不变：**只返回该会话自己的记录**（issue 级文件按记录里的
    /// `session_id` 过滤——该字段由 store 盖印）。
    pub fn list_candidate_attempts(
        &self,
        session_id: &str,
    ) -> Result<Vec<CandidateAttemptRecord>, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session = self.get_workspace_session(session_id)?;
        let mut records = self.issue_candidate_attempt_records(&session)?;
        records.sort_by_key(|record| record.attempt_seq);
        Ok(records)
    }

    /// REQ-WSC-25：读该 issue 下**全部会话**的尝试流水（按落盘时间升序）——
    /// 度量读数（首次通过率 / 同 issue 连续失败次数）需要跨会话看同一 issue。
    ///
    /// 权威来源是 issue 级单文件（耐得住会话清理）；旧版落点（会话目录内）
    /// 的历史行一并读入，升级前已落盘的记录不得从读数里消失，也不得双计
    /// （按 `(session_id, attempt_key)` 去重）。
    pub fn list_issue_candidate_attempts(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<CandidateAttemptRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let mut records = read_candidate_attempt_lines(
            &self.issue_candidate_attempts_path(project_id, issue_id),
        )?;
        let legacy = self.legacy_session_candidate_attempt_records(project_id, issue_id)?;
        records = merge_attempt_records(records, legacy);
        records.sort_by(|left, right| {
            (
                left.recorded_at.as_str(),
                left.session_id.as_str(),
                left.attempt_seq,
            )
                .cmp(&(
                    right.recorded_at.as_str(),
                    right.session_id.as_str(),
                    right.attempt_seq,
                ))
        });
        Ok(records)
    }

    /// REQ-WSC-25：读该项目下**全部 issue** 的尝试流水序列（每个 issue 一条
    /// 序列）——项目级通过率读数（`issue_count` = N，`first_pass_rate` 才是
    /// 真率；按单个 issue 算出来只能是 0.0/1.0，那不是「率」）。
    ///
    /// 枚举方式＝流水的最终落点（`issues/{issue_id}/candidate-attempts.jsonl`）：
    /// 项目 `issues/` 目录下的子目录逐个读；没有流水的 issue 不产生序列
    /// （分母＝有尝试记录的 issue 数，与既有 metrics 口径一致）。
    pub fn list_project_candidate_attempt_series(
        &self,
        project_id: &str,
    ) -> Result<Vec<CandidateIssueAttemptSeries>, ProductStoreError> {
        validate_relative_id(project_id)?;
        let issues_root = self.paths.project_root(project_id).join("issues");
        let entries = match std::fs::read_dir(&issues_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    issues_root.display()
                )));
            }
        };
        let mut issue_ids = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", issues_root.display()))
            })?;
            let is_dir = entry.file_type().map_err(|error| {
                ProductStoreError::Io(format!(
                    "read {} entry type: {error}",
                    issues_root.display()
                ))
            })?;
            if !is_dir.is_dir() {
                continue;
            }
            if let Some(issue_id) = entry.path().file_name().and_then(|name| name.to_str()) {
                issue_ids.push(issue_id.to_string());
            }
        }
        issue_ids.sort();
        let mut series = Vec::new();
        for issue_id in issue_ids {
            let records = self.list_issue_candidate_attempts(project_id, &issue_id)?;
            if records.is_empty() {
                continue;
            }
            series.push(CandidateIssueAttemptSeries { issue_id, records });
        }
        Ok(series)
    }

    /// 该会话在 issue 级权威文件里的记录 + 旧版落点（会话目录）里的记录
    /// （去重合并）——会话级读的完整来源。
    fn issue_candidate_attempt_records(
        &self,
        session: &WorkspaceSessionRecord,
    ) -> Result<Vec<CandidateAttemptRecord>, ProductStoreError> {
        let issue_records: Vec<CandidateAttemptRecord> = read_candidate_attempt_lines(
            &self.issue_candidate_attempts_path(&session.project_id, &session.issue_id),
        )?
        .into_iter()
        .filter(|record| record.session_id == session.id)
        .collect();
        let legacy = read_candidate_attempt_lines(&self.legacy_session_attempts_path(session))?;
        Ok(merge_attempt_records(issue_records, legacy))
    }

    /// 旧版落点（会话目录内的流水文件）——只读兼容面，升级后不再写入。
    fn legacy_session_attempts_path(&self, session: &WorkspaceSessionRecord) -> PathBuf {
        self.workspace_session_root(session)
            .join(CANDIDATE_ATTEMPTS_FILE_NAME)
    }

    /// 该 issue 下**全部会话**的旧版落点流水（只读兼容面）。
    fn legacy_session_candidate_attempt_records(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<CandidateAttemptRecord>, ProductStoreError> {
        let root = self.workspace_sessions_root(project_id, issue_id);
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    root.display()
                )));
            }
        };
        let mut records = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
            let is_dir = entry.file_type().map_err(|error| {
                ProductStoreError::Io(format!("read {} entry type: {error}", root.display()))
            })?;
            if !is_dir.is_dir() {
                continue;
            }
            let path = entry.path().join(CANDIDATE_ATTEMPTS_FILE_NAME);
            records.extend(read_candidate_attempt_lines(&path)?);
        }
        Ok(records)
    }
}

/// 该会话的下一个尝试序号（会话内单调递增；跨旧版落点续号）。
fn next_attempt_seq(
    issue_records: &[CandidateAttemptRecord],
    legacy_records: &[CandidateAttemptRecord],
    session_id: &str,
) -> u32 {
    issue_records
        .iter()
        .chain(legacy_records.iter())
        .filter(|record| record.session_id == session_id)
        .map(|record| record.attempt_seq)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

/// 合并两组流水记录并按 `(session_id, attempt_key)` 去重——issue 级权威文件
/// 优先（旧版落点的同键行不重复计入，读数既不丢也不双计）。
fn merge_attempt_records(
    primary: Vec<CandidateAttemptRecord>,
    legacy: Vec<CandidateAttemptRecord>,
) -> Vec<CandidateAttemptRecord> {
    let mut merged: Vec<CandidateAttemptRecord> = Vec::with_capacity(primary.len() + legacy.len());
    let mut seen: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::with_capacity(primary.len() + legacy.len());
    for record in primary.into_iter().chain(legacy) {
        if seen.insert((record.session_id.clone(), record.attempt_key.clone())) {
            merged.push(record);
        }
    }
    merged
}

/// REQ-WSC-25：由 typed 编译失败派生一条尝试记录。
///
/// 阶段与 typed 种类**单源于既有 typed 失败**（不从文案/错误码反推）：
/// 结构失败集 → 阶段 `structure` + 集合内全部 `PlanningFailureKind`；
/// 运维类失败 → 阶段 `final` + `PlanningFailureKind::Operational`。
/// 环结构指纹与自动重写阶段取自既有 `CandidateCompileStatus`（不另算）。
pub(crate) fn candidate_attempt_record_from_failure(
    failure: &crate::product::workspace_engine::PlanningCompileFailure,
    status: &CandidateCompileStatus,
    recorded_at: String,
) -> CandidateAttemptRecord {
    use crate::product::workspace_engine::PlanningCompileFailureKind;
    let (stage, typed_kinds) = match &failure.kind {
        PlanningCompileFailureKind::Structure(set) => (
            CandidateAttemptStage::Structure,
            set.failures
                .iter()
                .map(|failure| failure.kind.clone())
                .collect(),
        ),
        PlanningCompileFailureKind::Operational { stable_code, cause } => (
            CandidateAttemptStage::Final,
            vec![PlanningFailureKind::Operational {
                stable_code: stable_code.clone(),
                cause: cause.clone(),
            }],
        ),
    };
    CandidateAttemptRecord {
        attempt_seq: 0,
        // 幂等键：内容寻址失败身份（同一次失败重放＝同一次尝试，不重复膨胀）。
        attempt_key: format!("fail/{}", failure.failure_id),
        session_id: String::new(),
        issue_id: String::new(),
        recorded_at,
        stage,
        outcome: CandidateAttemptOutcome::Failed,
        source_revision_ref: status.source_revision_ref.clone(),
        plan_candidate_ir_ref: status.plan_candidate_ir_ref.clone(),
        failure: Some(CandidateAttemptFailure {
            failure_id: failure.failure_id.clone(),
            code: failure.stable_code().to_string(),
            typed_kinds,
            cycle_fingerprint: status.cycle_fingerprint.clone(),
        }),
        auto_rewrite: status.auto_rewrite.clone(),
        auto_repairs_applied: Vec::new(),
        equivalence_canonicalizations: Vec::new(),
    }
}

/// REQ-WSC-25：由编译成功派生一条尝试记录（成功也必须进历史序列，否则首次
/// 通过率无从计算）。幂等键取本次 compile 运行身份（`compile_id`）；缺席时
/// 退回候选绑定 ref 对。
pub(crate) fn candidate_attempt_record_from_success(
    status: &CandidateCompileStatus,
    compile_id: Option<&str>,
    recorded_at: String,
) -> CandidateAttemptRecord {
    let attempt_key = match compile_id {
        Some(compile_id) => format!("pass/{compile_id}"),
        None => format!(
            "pass/{}/{}",
            status.source_revision_ref.as_deref().unwrap_or("-"),
            status.plan_candidate_ir_ref.as_deref().unwrap_or("-")
        ),
    };
    CandidateAttemptRecord {
        attempt_seq: 0,
        attempt_key,
        session_id: String::new(),
        issue_id: String::new(),
        recorded_at,
        stage: CandidateAttemptStage::Final,
        outcome: CandidateAttemptOutcome::Passed,
        source_revision_ref: status.source_revision_ref.clone(),
        plan_candidate_ir_ref: status.plan_candidate_ir_ref.clone(),
        failure: None,
        auto_rewrite: None,
        auto_repairs_applied: Vec::new(),
        equivalence_canonicalizations: Vec::new(),
    }
}

/// REQ-WSC-25 度量读数（design §9.3 的四个指标）。
///
/// 口径：
/// - **首次通过率**：第一次尝试即 `Passed` 的 issue 比例（分母＝有尝试记录的
///   issue 数）。
/// - **同 issue 连续失败次数**：该 issue 首次通过之前的失败尝试数（未通过的
///   issue 取其全部失败尝试数）；P50/P90 用最近秩法（nearest-rank）。
/// - **失败码 Top N**：全部失败尝试按稳定码聚合（次数降序，同数按码升序）。
/// - **lowering 类终态失败计数**：该 issue 最后一次尝试仍失败且稳定码为
///   `lowering_error` 的 issue 数（终态失败=未被后续通过覆盖）。
pub fn compute_candidate_attempt_metrics(
    issues: &[CandidateIssueAttemptSeries],
) -> CandidateAttemptMetrics {
    let mut issue_count = 0usize;
    let mut first_pass_issue_count = 0usize;
    let mut consecutive_failures: Vec<f64> = Vec::new();
    let mut code_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut lowering_terminal_failure_count = 0usize;

    for issue in issues {
        let mut records = issue.records.clone();
        records.sort_by(|left, right| {
            (left.recorded_at.as_str(), left.attempt_seq)
                .cmp(&(right.recorded_at.as_str(), right.attempt_seq))
        });
        if records.is_empty() {
            continue;
        }
        issue_count += 1;
        if records[0].outcome == CandidateAttemptOutcome::Passed {
            first_pass_issue_count += 1;
        }
        consecutive_failures.push(
            records
                .iter()
                .take_while(|record| record.outcome == CandidateAttemptOutcome::Failed)
                .count() as f64,
        );
        for record in records
            .iter()
            .filter(|record| record.outcome == CandidateAttemptOutcome::Failed)
        {
            if let Some(failure) = record.failure.as_ref() {
                *code_counts.entry(failure.code.clone()).or_insert(0) += 1;
            }
        }
        if let Some(last) = records.last()
            && last.outcome == CandidateAttemptOutcome::Failed
            && last
                .failure
                .as_ref()
                .is_some_and(|failure| failure.code == LOWERING_FAILURE_STABLE_CODE)
        {
            lowering_terminal_failure_count += 1;
        }
    }

    consecutive_failures
        .sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let mut failure_code_ranking: Vec<CandidateFailureCodeCount> = code_counts
        .into_iter()
        .map(|(code, count)| CandidateFailureCodeCount { code, count })
        .collect();
    failure_code_ranking.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.code.cmp(&right.code))
    });

    CandidateAttemptMetrics {
        issue_count,
        first_pass_issue_count,
        first_pass_rate: (issue_count > 0)
            .then_some(first_pass_issue_count as f64 / issue_count as f64),
        consecutive_failure_p50: percentile(&consecutive_failures, 0.5),
        consecutive_failure_p90: percentile(&consecutive_failures, 0.9),
        failure_code_ranking,
        lowering_terminal_failure_count,
    }
}

/// 最近秩分位（nearest-rank；样本为空 → None）。
fn percentile(sorted: &[f64], quantile: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (quantile * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted.get(rank.min(sorted.len()) - 1).copied()
}

/// 读流水行（坏行跳过：append-only 流水的一行损坏不得让整条历史不可读）。
fn read_candidate_attempt_lines(
    path: &Path,
) -> Result<Vec<CandidateAttemptRecord>, ProductStoreError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "open {}: {error}",
                path.display()
            )));
        }
    };
    let mut records = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<CandidateAttemptRecord>(&line) {
            records.push(record);
        }
    }
    Ok(records)
}

/// 追加一行（append-only：只追加，不改写既有行）。
fn append_candidate_attempt_line(
    path: &Path,
    record: &CandidateAttemptRecord,
) -> Result<(), ProductStoreError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| ProductStoreError::Io(format!("open {}: {error}", path.display())))?;
    let line = serde_json::to_string(record)
        .map_err(|error| ProductStoreError::Json(error.to_string()))?;
    writeln!(file, "{line}")
        .map_err(|error| ProductStoreError::Io(format!("append {}: {error}", path.display())))?;
    file.flush()
        .map_err(|error| ProductStoreError::Io(format!("flush {}: {error}", path.display())))?;
    Ok(())
}
