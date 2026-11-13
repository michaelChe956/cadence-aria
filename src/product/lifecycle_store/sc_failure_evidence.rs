//! Strict SC failure evidence store（Plan §2.3 前 6 个冻结接口）。
//!
//! 形状：`workspace-timelines/<session>/sc-failure-evidence/<chain_id>/` 下的
//! append-only 事实日志 `rounds.jsonl` + 只读教学提示正文
//! `prompts/<ordinal>-<sha256>.txt`。
//!
//! 冻结接口（§2.3 前 6 个；另加 P05 lookup）：
//! 1. `sc_failure_chain_id` —— 长度前缀哈希的 chain 身份（无歧义拼接）；
//! 2. `append_sc_failure_evidence` —— 逻辑槽位幂等追加；
//! 3. `read_sc_failure_evidence` —— 带已验证前缀的读回；
//! 4. `put_sc_teaching_prompt` —— 不可变教学提示正文；
//! 5. `read_sc_teaching_prompt` —— 按 locator 校验摘要后读回；
//! 6. `read_sc_split_output` —— 真实 split run 产物读回；
//! 7. `find_sc_failure_round` —— 按 scope + 业务键 + 原始节点 + 轮节点 + ordinal
//!    找回已绑定的真实轮（只读；实现见 `sc_failure_evidence_lookup.rs`）。
//!
//! 严格性要点：
//! - 逻辑槽键（`entry_key`）不含正文摘要：同槽异文是冲突，不得变成新键逃过冲突；
//! - 日志条目序号必须连续递增；坏行/断号/跨 scope 都带已验证前缀报错，不跳行吞下；
//! - 写准入：会话必须是 live 的 SC single-candidate WorkItemPlan，且所属实体
//!   没有删除墓碑（purging/deleted）；
//! - 落盘：文件锁串行化 + append 后 `sync_all` + 回读校验。

use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::product::coding_attempt_store::locking::ExclusiveFileLock;
use crate::product::entity_deletion_store::{EntityDeletionStore, EntityScope};
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{WorkspaceSessionRecord, WorkspaceType};
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::product::work_item_split_engine::parse::WorkItemSplitProviderRunHandle;
use crate::web::workspace_ws_types::{
    ScEvidenceRef, ScFailureChainIdentity, ScFailureClass, ScFailureScope, ScMissingEvidence,
    ScPackageCompleteness, ScPromptDisposition, ScRoundIdentity, ScRoundLaunchState,
    ScTeachingPromptEvidence,
};

/// 事实日志的 schema 版本（读侧遇到未知版本按坏行处理，不猜测）。
const SC_FAILURE_EVIDENCE_SCHEMA_VERSION: u32 = 1;
/// workspace-timelines 下的证据分区名。
const SC_FAILURE_EVIDENCE_DIR: &str = "sc-failure-evidence";
/// 事实日志文件名。
const SC_FAILURE_EVIDENCE_LOG: &str = "rounds.jsonl";
/// 事实日志的排它锁文件名（与日志同目录，`.{name}.lock` 约定）。
const SC_FAILURE_EVIDENCE_LOCK: &str = ".rounds.jsonl.lock";
/// 教学提示正文目录名。
const SC_FAILURE_PROMPT_DIR: &str = "prompts";
/// split run 产物 locator 前缀（`{prefix}/{session_id}/{role_run_seq}`）。
const SC_FAILURE_SPLIT_LOCATOR_PREFIX: &str = "sc-split-output/v1";
/// 教学提示 locator 前缀（`{prefix}/{session}/{chain_id}/{ordinal}/{sha256}`）。
const SC_FAILURE_PROMPT_LOCATOR_PREFIX: &str = "sc-teaching-prompt/v1";
/// chain 身份前缀。
const SC_FAILURE_CHAIN_ID_PREFIX: &str = "chain-v1";

const CODE_CHAIN_INVALID: &str = "sc_failure_chain_invalid";
const CODE_ENTRY_CONFLICT: &str = "sc_failure_entry_conflict";
const CODE_IO: &str = "sc_failure_io";
const CODE_LOCATOR_INVALID: &str = "sc_failure_locator_invalid";
const CODE_LOG_CORRUPT: &str = "sc_failure_log_corrupt";
const CODE_LOG_SEQUENCE_GAP: &str = "sc_failure_log_sequence_gap";
const CODE_PROMPT_DIGEST_MISMATCH: &str = "sc_failure_prompt_digest_mismatch";
const CODE_PROMPT_MISSING: &str = "sc_failure_prompt_missing";
const CODE_RAW_MISSING: &str = "sc_failure_raw_missing";
const CODE_ROUND_NOT_BOUND: &str = "sc_failure_round_not_bound";
const CODE_SCOPE_MISMATCH: &str = "sc_failure_scope_mismatch";
const CODE_SESSION_NOT_LIVE: &str = "sc_failure_session_not_live";
const CODE_SESSION_NOT_SC: &str = "sc_failure_session_not_sc";

/// 事实日志里的一条严格证据记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScFailureEvidenceEntry {
    pub schema_version: u32,
    pub entry_seq: u64,
    pub entry_key: String,
    pub chain: ScFailureChainIdentity,
    pub event: ScFailureEvidenceEvent,
}

/// 证据事件（`kind` 区分种类；`entry_key` 只表达逻辑槽位，不含正文摘要）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ScFailureEvidenceEvent {
    RoundBound {
        round: ScRoundIdentity,
    },
    FailureObserved {
        round_id: String,
        failure_class: Option<ScFailureClass>,
        blocking_reasons: Vec<String>,
        repair_instructions: Vec<String>,
        raw: ScEvidenceRef,
        source: ScEvidenceRef,
        existing_failure_id: Option<String>,
        attempt_key: Option<String>,
    },
    PromptPrepared {
        evidence: ScTeachingPromptEvidence,
    },
    PromptUnavailable {
        evidence: ScTeachingPromptEvidence,
    },
    ChainTerminal {
        last_failure_round_id: String,
        last_physical_round_id: Option<String>,
        terminal_class: String,
        terminal_reason: String,
        launch_observations: Vec<(String, ScRoundLaunchState)>,
        completeness: ScPackageCompleteness,
        missing_evidence: Vec<ScMissingEvidence>,
    },
}

/// 一次读回的严格证据视图（含已验证前缀字节）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScFailureEvidenceRead {
    pub entries: Vec<ScFailureEvidenceEntry>,
    pub verified_prefix_bytes: Vec<u8>,
    pub evidence_through_entry_key: String,
    pub missing_evidence: Vec<ScMissingEvidence>,
}

/// 严格证据 store 的稳定错误（`code`/`step` 为契约面）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScFailureEvidenceError {
    pub code: String,
    pub step: String,
    pub round_id: Option<String>,
    pub detail: String,
    pub publication_incomplete: bool,
    pub verified_prefix: Option<Box<ScFailureEvidenceRead>>,
}

impl fmt::Display for ScFailureEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} (step={}, round_id={:?}, detail={})",
            self.code, self.step, self.round_id, self.detail
        )
    }
}

impl std::error::Error for ScFailureEvidenceError {}

/// 教学提示写入请求（写体参数打包，避免长参数列表）。
struct ScPromptWrite<'a> {
    failed_round_id: &'a str,
    target_round_ordinal: u32,
    builder_id: &'a str,
    text: &'a str,
    disposition: ScPromptDisposition,
}

/// chain 身份：会话 / 业务键 / 原始节点 / 原始 role_run_seq 的长度前缀哈希。
///
/// 长度前缀避免拼接歧义（`ab`+`c` 与 `a`+`bc` 不得同键）。
pub(crate) fn sc_failure_chain_id(
    session_id: &str,
    business_key: &str,
    original_node_id: &str,
    original_role_run_seq: u64,
) -> Result<String, ScFailureEvidenceError> {
    for (name, value) in [
        ("session_id", session_id),
        ("business_key", business_key),
        ("original_node_id", original_node_id),
    ] {
        if value.is_empty() || value.contains('/') || value.contains('\\') {
            return Err(chain_invalid_error("sc_failure_chain_id", format!("{name} 不能为空或含路径分隔符")));
        }
    }
    // §2.3 冻结配方：四段各用 u64 大端字节长度前缀 + UTF-8 正文；seq 先十进制
    // 字符串化再加前缀；无额外 domain tag（尚无生产 P05 链，不留旧链 alias）。
    let seq_text = original_role_run_seq.to_string();
    let mut hasher = Sha256::new();
    for part in [session_id, business_key, original_node_id, seq_text.as_str()] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    Ok(format!(
        "{SC_FAILURE_CHAIN_ID_PREFIX}-{}",
        hex_digest(&hasher.finalize())
    ))
}

impl LifecycleStore {
    /// 追加一条严格证据（幂等：同槽同文零膨胀；同槽异文冲突）。
    ///
    /// 写准入：会话必须是 live 的 SC session；整个写体在所属实体的 live-write
    /// lease 内执行（scope lease → 日志锁，锁序不变）。
    pub(crate) fn append_sc_failure_evidence(
        &self,
        chain: &ScFailureChainIdentity,
        event: &ScFailureEvidenceEvent,
    ) -> Result<ScFailureEvidenceEntry, ScFailureEvidenceError> {
        const STEP: &str = "append_sc_failure_evidence";
        let session = self.sc_failure_session(chain, STEP)?;
        let scope = self.sc_failure_entity_scope(&session, STEP)?;
        self.sc_failure_live_admission(&session, STEP)?;
        self.sc_failure_with_live_write(&scope, STEP, || {
            self.sc_failure_append_admitted(chain, event, STEP)
        })
    }

    /// 已准入写体（append）：chain 自洽 → 证据目录 → 日志锁 → 读 → 幂等/冲突
    /// → 事件校验 → 追加。
    fn sc_failure_append_admitted(
        &self,
        chain: &ScFailureChainIdentity,
        event: &ScFailureEvidenceEvent,
        step: &str,
    ) -> Result<ScFailureEvidenceEntry, ScFailureEvidenceError> {
        self.sc_failure_chain_consistent(chain, step)?;
        let directory = self.sc_failure_evidence_dir(chain, step)?;
        let log_path = directory.join(SC_FAILURE_EVIDENCE_LOG);
        let _lock = self.sc_failure_log_lock(&directory, step)?;
        let read = read_sc_failure_log(&log_path, chain, step)?;
        let key = sc_failure_entry_key(event);
        if let Some(existing) = read.entries.iter().find(|entry| entry.entry_key == key) {
            if existing.event == *event {
                return Ok(existing.clone());
            }
            let mut conflict = entry_conflict_error(step, None, "同一逻辑槽位已存在不同正文");
            conflict.verified_prefix = Some(Box::new(read));
            return Err(conflict);
        }
        self.sc_failure_validate_event(chain, &read, event, step)?;
        self.sc_failure_append_entry(&log_path, chain, &read, key, event.clone(), step)
    }

    /// 读回严格证据（坏行/断号/跨 scope 都带已验证前缀报错）。
    pub(crate) fn read_sc_failure_evidence(
        &self,
        chain: &ScFailureChainIdentity,
    ) -> Result<ScFailureEvidenceRead, ScFailureEvidenceError> {
        const STEP: &str = "read_sc_failure_evidence";
        let _session = self.sc_failure_session(chain, STEP)?;
        let directory = self.sc_failure_evidence_dir(chain, STEP)?;
        read_sc_failure_log(&directory.join(SC_FAILURE_EVIDENCE_LOG), chain, STEP)
    }

    /// 写入不可变教学提示正文（同槽同文零膨胀；同槽异文冲突）。
    ///
    /// 写准入同 append：整个写体在所属实体的 live-write lease 内执行。
    pub(crate) fn put_sc_teaching_prompt(
        &self,
        chain: &ScFailureChainIdentity,
        failed_round_id: &str,
        target_round_ordinal: u32,
        builder_id: &str,
        text: &str,
        disposition: ScPromptDisposition,
    ) -> Result<ScTeachingPromptEvidence, ScFailureEvidenceError> {
        const STEP: &str = "put_sc_teaching_prompt";
        let session = self.sc_failure_session(chain, STEP)?;
        let scope = self.sc_failure_entity_scope(&session, STEP)?;
        self.sc_failure_live_admission(&session, STEP)?;
        let request = ScPromptWrite {
            failed_round_id,
            target_round_ordinal,
            builder_id,
            text,
            disposition,
        };
        self.sc_failure_with_live_write(&scope, STEP, || {
            self.sc_failure_put_prompt_admitted(chain, request, STEP)
        })
    }

    /// 已准入写体（教学提示）：chain 自洽 → 目录 → 日志锁 → 读 → 同槽幂等/冲突
    /// → 正文不可变守卫 → 事件校验 → 写正文 → 追加日志。
    fn sc_failure_put_prompt_admitted(
        &self,
        chain: &ScFailureChainIdentity,
        request: ScPromptWrite<'_>,
        step: &str,
    ) -> Result<ScTeachingPromptEvidence, ScFailureEvidenceError> {
        let ScPromptWrite {
            failed_round_id,
            target_round_ordinal,
            builder_id,
            text,
            disposition,
        } = request;
        self.sc_failure_chain_consistent(chain, step)?;
        let directory = self.sc_failure_evidence_dir(chain, step)?;
        let digest = sha256_hex(text.as_bytes());
        let bytes = text.len() as u64;
        let locator = sc_failure_prompt_locator(
            &chain.scope.session_id,
            &chain.chain_id,
            target_round_ordinal,
            &digest,
        );
        // 先按借用判类别，再把 disposition 移入 evidence（避免 use-after-move）。
        let unavailable = matches!(&disposition, ScPromptDisposition::Unavailable);
        let evidence = ScTeachingPromptEvidence {
            failed_round_id: failed_round_id.to_string(),
            target_round_ordinal,
            builder_id: builder_id.to_string(),
            prompt: ScEvidenceRef::Available {
                locator,
                sha256: digest.clone(),
                bytes,
            },
            disposition,
        };
        let event = if unavailable {
            ScFailureEvidenceEvent::PromptUnavailable {
                evidence: evidence.clone(),
            }
        } else {
            ScFailureEvidenceEvent::PromptPrepared {
                evidence: evidence.clone(),
            }
        };
        let log_path = directory.join(SC_FAILURE_EVIDENCE_LOG);
        let _lock = self.sc_failure_log_lock(&directory, step)?;
        let read = read_sc_failure_log(&log_path, chain, step)?;
        let key = sc_failure_entry_key(&event);
        if let Some(existing) = read.entries.iter().find(|entry| entry.entry_key == key) {
            if existing.event == event {
                return Ok(evidence);
            }
            let mut conflict = entry_conflict_error(step, Some(failed_round_id.to_string()), "同一教学提示槽位已存在不同正文");
            conflict.verified_prefix = Some(Box::new(read));
            return Err(conflict);
        }
        // 正文不可变：同一 (failed_round_id, target_round_ordinal) 的既有正文摘要必须
        // 一致 —— disposition 只是事件槽的区分，不是换正文的后门（跨 disposition
        // 换正文同样冲突，且冲突前不落正文文件、不改日志）。
        let mut body_conflict = false;
        for entry in &read.entries {
            let (ScFailureEvidenceEvent::PromptPrepared { evidence }
            | ScFailureEvidenceEvent::PromptUnavailable { evidence }) = &entry.event
            else {
                continue;
            };
            if evidence.failed_round_id != failed_round_id
                || evidence.target_round_ordinal != target_round_ordinal
            {
                continue;
            }
            if let ScEvidenceRef::Available { sha256: recorded, .. } = &evidence.prompt
                && recorded != &digest
            {
                body_conflict = true;
                break;
            }
        }
        if body_conflict {
            let mut conflict = entry_conflict_error(step, Some(failed_round_id.to_string()), "同一教学提示槽位已存在不同正文");
            conflict.verified_prefix = Some(Box::new(read));
            return Err(conflict);
        }
        self.sc_failure_validate_event(chain, &read, &event, step)?;
        write_sc_failure_prompt_file(
            &directory
                .join(SC_FAILURE_PROMPT_DIR)
                .join(format!("{target_round_ordinal}-{digest}.txt")),
            text,
            &digest,
            step,
        )?;
        self.sc_failure_append_entry(&log_path, chain, &read, key, event, step)?;
        Ok(evidence)
    }

    /// 按 locator 读回教学提示正文（摘要不符即拒）。
    pub(crate) fn read_sc_teaching_prompt(
        &self,
        chain: &ScFailureChainIdentity,
        locator: &str,
    ) -> Result<String, ScFailureEvidenceError> {
        const STEP: &str = "read_sc_teaching_prompt";
        let _session = self.sc_failure_session(chain, STEP)?;
        let directory = self.sc_failure_evidence_dir(chain, STEP)?;
        let (ordinal, digest) = parse_sc_failure_prompt_locator(
            locator,
            &chain.scope.session_id,
            &chain.chain_id,
        )
        .map_err(|detail| locator_invalid_error(STEP, None, detail))?;
        let path = directory
            .join(SC_FAILURE_PROMPT_DIR)
            .join(format!("{ordinal}-{digest}.txt"));
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {
                return Err(prompt_missing_error(STEP, "教学提示正文文件不存在"));
            }
            Err(failure) => {
                return Err(io_error(STEP, None, format!("读取教学提示正文失败: {failure}")));
            }
        };
        let text = String::from_utf8(bytes).map_err(|_| {
            prompt_digest_error(STEP, "教学提示正文不是 UTF-8")
        })?;
        if sha256_hex(text.as_bytes()) != digest {
            return Err(prompt_digest_error(STEP, "教学提示正文摘要与 locator 不符"));
        }
        Ok(text)
    }

    /// 读回真实 split run 的 provider 原始正文（JSON String 值）。
    pub(crate) fn read_sc_split_output(
        &self,
        scope: &ScFailureScope,
        round: &ScRoundIdentity,
    ) -> Result<String, ScFailureEvidenceError> {
        const STEP: &str = "read_sc_split_output";
        if scope.session_id != round.workspace_session_id {
            return Err(scope_mismatch_error(STEP, Some(round.round_id.clone()), "scope 的 session 与 round 的 workspace_session_id 不一致"));
        }
        if round.run_ref.is_empty() || round.workspace_session_id.is_empty() {
            return Err(locator_invalid_error(STEP, Some(round.round_id.clone()), "round 身份不完整（run_ref / workspace_session_id）"));
        }
        // scope 必须等于该 session 的真实归属（project / issue / plan）。
        let session = self
            .get_workspace_session(&round.workspace_session_id)
            .map_err(|failure| match failure {
                ProductStoreError::NotFound { .. } => scope_mismatch_error(STEP, Some(round.round_id.clone()), "round 的 workspace_session_id 没有对应会话"),
                other => io_error(STEP, Some(round.round_id.clone()), format!("读取会话失败: {other}")),
            })?;
        if session.project_id != scope.project_id
            || session.issue_id != scope.issue_id
            || session.entity_id != scope.plan_id
        {
            return Err(scope_mismatch_error(STEP, Some(round.round_id.clone()), "scope 与真实会话归属不一致"));
        }
        let handle = WorkItemSplitProviderRunHandle {
            run_ref: round.run_ref.clone(),
            workspace_session_id: round.workspace_session_id.clone(),
            role_run_seq: round.role_run_seq,
        };
        let directory = self
            .work_item_split_provider_run_dir(&handle)
            .map_err(|failure| io_error(STEP, Some(round.round_id.clone()), format!("定位 split run 目录失败: {failure}")))?;
        // run.json 身份核对：调用方的 run_ref 必须就是盘上这条 run 的 provider_run_id
        // （错绑 → locator_invalid）；盘上 session/seq 被改写 → 盘上元数据不属于该
        // round（scope_mismatch）。
        let run_path = directory.join("run.json");
        let run_bytes = match std::fs::read(&run_path) {
            Ok(bytes) => bytes,
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {
                return Err(raw_missing_error(STEP, Some(round.round_id.clone()), "split run 记录不存在"));
            }
            Err(failure) => {
                return Err(raw_missing_error(STEP, Some(round.round_id.clone()), format!("读取 split run 记录失败: {failure}")));
            }
        };
        let run: serde_json::Value = serde_json::from_slice(&run_bytes).map_err(|failure| raw_missing_error(STEP, Some(round.round_id.clone()), format!("split run 记录不是合法 JSON: {failure}")))?;
        let recorded_ref = run.get("provider_run_id").and_then(serde_json::Value::as_str).unwrap_or_default();
        if recorded_ref != round.run_ref {
            return Err(locator_invalid_error(STEP, Some(round.round_id.clone()), "split run 的 provider_run_id 与 round.run_ref 不符"));
        }
        let recorded_session = run.get("workspace_session_id").and_then(serde_json::Value::as_str).unwrap_or_default();
        let recorded_seq = run.get("role_run_seq").and_then(serde_json::Value::as_u64).unwrap_or_default();
        if recorded_session != round.workspace_session_id || recorded_seq != round.role_run_seq {
            return Err(scope_mismatch_error(STEP, Some(round.round_id.clone()), "split run 记录的 session/seq 与 round 不符"));
        }
        let path = directory.join("structured_output.json");
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {
                return Err(raw_missing_error(STEP, Some(round.round_id.clone()), "split run 产物不存在"));
            }
            Err(failure) => {
                return Err(raw_missing_error(STEP, Some(round.round_id.clone()), format!("读取 split run 产物失败: {failure}")));
            }
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|failure| raw_missing_error(STEP, Some(round.round_id.clone()), format!("split run 产物不是合法 JSON: {failure}")))?;
        match value {
            serde_json::Value::String(text) => Ok(text),
            _ => Err(raw_missing_error(STEP, Some(round.round_id.clone()), "split run 产物不是 JSON 字符串")),
        }
    }

    /// 证据分区目录（不创建；创建只发生在准入通过后的落盘路径）。
    fn sc_failure_evidence_dir(
        &self,
        chain: &ScFailureChainIdentity,
        step: &str,
    ) -> Result<PathBuf, ScFailureEvidenceError> {
        let root = self
            .workspace_timeline_root_for_session(&chain.scope.session_id)
            .map_err(|failure| io_error(step, None, format!("定位 workspace timeline 失败: {failure}")))?;
        Ok(root.join(SC_FAILURE_EVIDENCE_DIR).join(&chain.chain_id))
    }

    /// 写准入：会话必须是 live 的 SC single-candidate WorkItemPlan，chain scope
    /// 必须与真实会话身份一致。
    fn sc_failure_session(
        &self,
        chain: &ScFailureChainIdentity,
        step: &str,
    ) -> Result<WorkspaceSessionRecord, ScFailureEvidenceError> {
        let session = self
            .get_workspace_session(&chain.scope.session_id)
            .map_err(|failure| match failure {
                ProductStoreError::NotFound { .. } => session_not_live_error(step, "会话不存在或已不可用"),
                other => io_error(step, None, format!("读取会话失败: {other}")),
            })?;
        if session.workspace_type != WorkspaceType::WorkItemPlan
            || session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
        {
            return Err(session_not_sc_error(step, "会话不是 SC single-candidate WorkItemPlan"));
        }
        if chain.scope.project_id != session.project_id
            || chain.scope.issue_id != session.issue_id
            || chain.scope.plan_id != session.entity_id
        {
            return Err(scope_mismatch_error(step, None, "chain scope 与真实会话身份不一致"));
        }
        Ok(session)
    }

    /// 实体归属 scope（live-write lease 的对象）：会话无法归属实体 → fail closed。
    fn sc_failure_entity_scope(
        &self,
        session: &WorkspaceSessionRecord,
        step: &str,
    ) -> Result<EntityScope, ScFailureEvidenceError> {
        self.entity_scope_for_session(session)
            .map_err(|failure| io_error(step, None, format!("实体归属归约失败: {failure}")))?
            .ok_or_else(|| session_not_live_error(step, "会话无法归属实体，拒绝写证据"))
    }

    /// 把整个写体放进所属实体的 live-write lease（仓库既有 scope 准入闸）：lease
    /// 取得失败/墓碑在位 → Conflict → sc_failure_session_not_live；写体自身的严格
    /// 错误原样透出。锁序：scope lease → 日志锁（写体内部）。
    fn sc_failure_with_live_write<T>(
        &self,
        scope: &EntityScope,
        step: &str,
        body: impl FnOnce() -> Result<T, ScFailureEvidenceError>,
    ) -> Result<T, ScFailureEvidenceError> {
        let mut outcome: Option<Result<T, ScFailureEvidenceError>> = None;
        self.with_entity_live_write(scope, || {
            outcome = Some(body());
            Ok(())
        })
        .map_err(|failure| match failure {
            ProductStoreError::Conflict { .. } => session_not_live_error(
                step,
                "所属实体处于删除墓碑（purging/deleted）或删除独占进行中",
            ),
            other => io_error(step, None, format!("写准入失败: {other}")),
        })?;
        let outcome = outcome.ok_or_else(|| io_error(step, None, "写准入未执行写体"))?;
        outcome
    }

    /// 存活准入：所属实体存在删除墓碑（purging/deleted）时拒绝写。
    fn sc_failure_live_admission(
        &self,
        session: &WorkspaceSessionRecord,
        step: &str,
    ) -> Result<(), ScFailureEvidenceError> {
        let scope = self.sc_failure_entity_scope(session, step)?;
        let tombstone = EntityDeletionStore::new(self.app_paths())
            .get(&scope)
            .map_err(|failure| io_error(step, None, format!("读取删除事实失败: {failure}")))?;
        if tombstone.is_some() {
            return Err(session_not_live_error(step, "所属实体处于删除墓碑（purging/deleted）"));
        }
        Ok(())
    }

    /// chain 身份自洽：`chain_id` 必须等于身份四元组的长度前缀哈希。
    fn sc_failure_chain_consistent(
        &self,
        chain: &ScFailureChainIdentity,
        step: &str,
    ) -> Result<(), ScFailureEvidenceError> {
        let expected = sc_failure_chain_id(
            &chain.scope.session_id,
            &chain.business_key,
            &chain.original_node_id,
            chain.original_role_run_seq,
        )
        .map_err(|mut failure| {
            failure.step = step.to_string();
            failure
        })?;
        if expected != chain.chain_id {
            return Err(chain_invalid_error(step, "chain_id 与身份四元组不一致"));
        }
        Ok(())
    }

    /// 事件准入：round 必须先 RoundBound；raw/source locator 必须绑定该 round。
    fn sc_failure_validate_event(
        &self,
        chain: &ScFailureChainIdentity,
        read: &ScFailureEvidenceRead,
        event: &ScFailureEvidenceEvent,
        step: &str,
    ) -> Result<(), ScFailureEvidenceError> {
        match event {
            ScFailureEvidenceEvent::RoundBound { round } => {
                if round.chain_id != chain.chain_id {
                    return Err(scope_mismatch_error(step, Some(round.round_id.clone()), "round 的 chain_id 与 chain 不一致"));
                }
                if round.workspace_session_id != chain.scope.session_id {
                    return Err(scope_mismatch_error(step, Some(round.round_id.clone()), "round 的 workspace_session_id 与 chain scope 不一致"));
                }
                if round.round_id.is_empty() || round.run_ref.is_empty() {
                    return Err(locator_invalid_error(step, Some(round.round_id.clone()), "round 身份不完整（round_id / run_ref）"));
                }
                Ok(())
            }
            ScFailureEvidenceEvent::FailureObserved {
                round_id, raw, source, ..
            } => {
                let bound = sc_failure_bound_round(read, round_id).ok_or_else(|| {
                    round_not_bound_error(step, Some(round_id.clone()), "该 round 尚未 RoundBound")
                })?;
                let expected = sc_failure_split_locator(
                    &bound.workspace_session_id,
                    bound.role_run_seq,
                );
                for (label, reference) in [("raw", raw), ("source", source)] {
                    if let ScEvidenceRef::Available { locator, .. } = reference
                        && locator != &expected
                    {
                        return Err(locator_invalid_error(step, Some(round_id.clone()), format!("{label} locator 与 round 绑定不符")));
                    }
                }
                Ok(())
            }
            ScFailureEvidenceEvent::PromptPrepared { evidence }
            | ScFailureEvidenceEvent::PromptUnavailable { evidence } => {
                if sc_failure_bound_round(read, &evidence.failed_round_id).is_none() {
                    return Err(round_not_bound_error(step, Some(evidence.failed_round_id.clone()), "教学提示的失败 round 尚未 RoundBound"));
                }
                Ok(())
            }
            ScFailureEvidenceEvent::ChainTerminal { .. } => Ok(()),
        }
    }

    /// 事实日志的排它锁（创建证据分区目录亦在此处发生，准入已先通过）。
    fn sc_failure_log_lock(
        &self,
        directory: &Path,
        step: &str,
    ) -> Result<ExclusiveFileLock, ScFailureEvidenceError> {
        ExclusiveFileLock::acquire_at(&directory.join(SC_FAILURE_EVIDENCE_LOCK)).map_err(
            |failure| {
                io_error(step, None, format!("取证据日志锁失败: {failure}"))
            },
        )
    }

    /// 追加一条日志条目（序号连续 + append + sync + 回读校验）。
    fn sc_failure_append_entry(
        &self,
        log_path: &Path,
        chain: &ScFailureChainIdentity,
        read: &ScFailureEvidenceRead,
        key: String,
        event: ScFailureEvidenceEvent,
        step: &str,
    ) -> Result<ScFailureEvidenceEntry, ScFailureEvidenceError> {
        let entry = ScFailureEvidenceEntry {
            schema_version: SC_FAILURE_EVIDENCE_SCHEMA_VERSION,
            entry_seq: read
                .entries
                .last()
                .map(|entry| entry.entry_seq + 1)
                .unwrap_or(1),
            entry_key: key,
            chain: chain.clone(),
            event,
        };
        append_sc_failure_log_line(log_path, &entry, step)?;
        Ok(entry)
    }
}

/// 逻辑槽键：只表达"哪一格"，不含正文摘要（否则异文会变成新键逃过冲突）。
fn sc_failure_entry_key(event: &ScFailureEvidenceEvent) -> String {
    match event {
        ScFailureEvidenceEvent::RoundBound { round } => {
            format!("round/{}", round.round_id)
        }
        ScFailureEvidenceEvent::FailureObserved { round_id, .. } => {
            format!("failureobserved/{round_id}")
        }
        ScFailureEvidenceEvent::PromptPrepared { evidence }
        | ScFailureEvidenceEvent::PromptUnavailable { evidence } => format!(
            "prompt/{}/{}/{}",
            evidence.failed_round_id,
            evidence.target_round_ordinal,
            sc_failure_disposition_key(&evidence.disposition)
        ),
        ScFailureEvidenceEvent::ChainTerminal { .. } => "terminal".to_string(),
    }
}

fn sc_failure_disposition_key(disposition: &ScPromptDisposition) -> &'static str {
    match disposition {
        ScPromptDisposition::Prepared => "prepared",
        ScPromptDisposition::Sent => "sent",
        ScPromptDisposition::RejectedBudget => "rejected_budget",
        ScPromptDisposition::Unavailable => "unavailable",
    }
}

fn sc_failure_bound_round<'a>(
    read: &'a ScFailureEvidenceRead,
    round_id: &str,
) -> Option<&'a ScRoundIdentity> {
    read.entries.iter().find_map(|entry| match &entry.event {
        ScFailureEvidenceEvent::RoundBound { round } if round.round_id == round_id => Some(round),
        _ => None,
    })
}

/// split run 产物 locator 的规范构造（`sc-split-output/v1/{session}/{seq}`）。
/// P08 等真实消费者直接调用本函数，不另立常量让调用方硬编码。
pub(crate) fn sc_failure_split_locator(session_id: &str, role_run_seq: u64) -> String {
    format!("{SC_FAILURE_SPLIT_LOCATOR_PREFIX}/{session_id}/{role_run_seq}")
}

fn sc_failure_prompt_locator(
    session_id: &str,
    chain_id: &str,
    ordinal: u32,
    digest: &str,
) -> String {
    format!("{SC_FAILURE_PROMPT_LOCATOR_PREFIX}/{session_id}/{chain_id}/{ordinal}/{digest}")
}

/// 解析教学提示 locator：版本 / 段数 / 身份 / ordinal / 摘要都必须是本 chain 的。
fn parse_sc_failure_prompt_locator(
    locator: &str,
    session_id: &str,
    chain_id: &str,
) -> Result<(u32, String), String> {
    let parts: Vec<&str> = locator.split('/').collect();
    if parts.len() != 6 || parts[0] != "sc-teaching-prompt" || parts[1] != "v1" {
        return Err("教学提示 locator 的版本或段数非法".to_string());
    }
    if parts[2] != session_id || parts[3] != chain_id {
        return Err("教学提示 locator 与 chain 身份不符".to_string());
    }
    let ordinal: u32 = parts[4]
        .parse()
        .map_err(|_| "教学提示 locator 的 target_round_ordinal 非法".to_string())?;
    let digest = parts[5];
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("教学提示 locator 的摘要非法".to_string());
    }
    Ok((ordinal, digest.to_string()))
}

/// 读事实日志：坏行/断号/跨 scope 都停下并带上已验证前缀。
fn read_sc_failure_log(
    log_path: &Path,
    chain: &ScFailureChainIdentity,
    step: &str,
) -> Result<ScFailureEvidenceRead, ScFailureEvidenceError> {
    let bytes = match std::fs::read(log_path) {
        Ok(bytes) => bytes,
        Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ScFailureEvidenceRead {
                entries: Vec::new(),
                verified_prefix_bytes: Vec::new(),
                evidence_through_entry_key: String::new(),
                missing_evidence: Vec::new(),
            });
        }
        Err(failure) => {
            return Err(io_error(step, None, format!("读取证据日志失败: {failure}")));
        }
    };
    let mut entries: Vec<ScFailureEvidenceEntry> = Vec::new();
    let mut verified_bytes = 0usize;
    let mut expected_seq: u64 = 1;
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let newline = bytes[cursor..].iter().position(|byte| *byte == b'\n');
        let (line_end, next) = match newline {
            Some(offset) => (cursor + offset, cursor + offset + 1),
            None => (bytes.len(), bytes.len()),
        };
        let line = &bytes[cursor..line_end];
        cursor = next;
        if line.is_empty() {
            continue;
        }
        let text = std::str::from_utf8(line).map_err(|failure| sc_failure_log_failure( CODE_LOG_CORRUPT, step, &entries, &bytes[..verified_bytes], format!("证据日志行不是 UTF-8: {failure}"), ))?;
        let entry: ScFailureEvidenceEntry = serde_json::from_str(text).map_err(|failure| sc_failure_log_failure( CODE_LOG_CORRUPT, step, &entries, &bytes[..verified_bytes], format!("证据日志行不是合法条目: {failure}"), ))?;
        if entry.schema_version != SC_FAILURE_EVIDENCE_SCHEMA_VERSION {
            return Err(sc_failure_log_failure(
                CODE_LOG_CORRUPT,
                step,
                &entries,
                &bytes[..verified_bytes],
                format!(
                    "证据日志 schema 版本不支持: {}",
                    entry.schema_version
                ),
            ));
        }
        if entry.entry_seq != expected_seq {
            return Err(sc_failure_log_failure(
                CODE_LOG_SEQUENCE_GAP,
                step,
                &entries,
                &bytes[..verified_bytes],
                format!("证据日志序号断档: 期望 {expected_seq}, 实际 {}", entry.entry_seq),
            ));
        }
        if entry.chain != *chain {
            return Err(sc_failure_log_failure(
                CODE_SCOPE_MISMATCH,
                step,
                &entries,
                &bytes[..verified_bytes],
                "证据日志条目属于其他 chain（跨 scope 注入）".to_string(),
            ));
        }
        entries.push(entry);
        expected_seq += 1;
        verified_bytes = cursor;
    }
    Ok(ScFailureEvidenceRead {
        evidence_through_entry_key: entries
            .last()
            .map(|entry| entry.entry_key.clone())
            .unwrap_or_default(),
        entries,
        verified_prefix_bytes: bytes[..verified_bytes].to_vec(),
        missing_evidence: Vec::new(),
    })
}

/// 追加一行事实日志：单次 append + `sync_all` + 回读校验。
fn append_sc_failure_log_line(
    log_path: &Path,
    entry: &ScFailureEvidenceEntry,
    step: &str,
) -> Result<(), ScFailureEvidenceError> {
    if let Some(parent) = log_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|failure| io_error(step, None, format!("创建证据日志目录失败: {failure}")))?;
    }
    let mut payload = serde_json::to_vec(entry).map_err(|failure| io_error(step, None, format!("序列化证据条目失败: {failure}")))?;
    payload.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|failure| io_error(step, None, format!("打开证据日志失败: {failure}")))?;
    file.write_all(&payload).map_err(|failure| io_error(step, None, format!("写入证据日志失败: {failure}")))?;
    file.flush().map_err(|failure| io_error(step, None, format!("刷写证据日志失败: {failure}")))?;
    file.sync_all().map_err(|failure| io_error(step, None, format!("同步证据日志失败: {failure}")))?;
    let readback = std::fs::read(log_path).map_err(|failure| io_error(step, None, format!("回读证据日志失败: {failure}")))?;
    let last = readback
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .next_back()
        .ok_or_else(|| io_error(step, None, "回读证据日志为空"))?;
    let parsed: ScFailureEvidenceEntry = serde_json::from_slice(last).map_err(|failure| io_error(step, None, format!("回读证据日志条目不可解析: {failure}")))?;
    if parsed != *entry {
        return Err(io_error(step, None, "回读证据日志与写入条目不一致"));
    }
    Ok(())
}

/// 写不可变提示正文：同路径同文不重写；异文即拒（不回退覆盖）。
fn write_sc_failure_prompt_file(
    path: &Path,
    text: &str,
    digest: &str,
    step: &str,
) -> Result<(), ScFailureEvidenceError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|failure| io_error(step, None, format!("创建提示目录失败: {failure}")))?;
    }
    match std::fs::read_to_string(path) {
        Ok(existing) => {
            if existing == text {
                return Ok(());
            }
            return Err(prompt_digest_error(step, "提示正文路径已存在不同内容"));
        }
        Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {}
        Err(failure) => {
            return Err(io_error(step, None, format!("读取既有提示正文失败: {failure}")));
        }
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|failure| io_error(step, None, format!("创建提示正文失败: {failure}")))?;
    file.write_all(text.as_bytes()).map_err(|failure| io_error(step, None, format!("写入提示正文失败: {failure}")))?;
    file.flush().map_err(|failure| io_error(step, None, format!("刷写提示正文失败: {failure}")))?;
    file.sync_all().map_err(|failure| io_error(step, None, format!("同步提示正文失败: {failure}")))?;
    let written = std::fs::read_to_string(path).map_err(|failure| io_error(step, None, format!("回读提示正文失败: {failure}")))?;
    if sha256_hex(written.as_bytes()) != digest {
        return Err(prompt_digest_error(step, "回读提示正文摘要与写入不符"));
    }
    Ok(())
}

/// 日志类错误的统一形状：带上已验证前缀（坏行/断号之后的行不被跳行吞下）。
fn sc_failure_log_failure(
    code: &str,
    step: &str,
    entries: &[ScFailureEvidenceEntry],
    verified_bytes: &[u8],
    detail: String,
) -> ScFailureEvidenceError {
    let mut failure = error(code, step, None, detail, true);
    failure.verified_prefix = Some(Box::new(ScFailureEvidenceRead {
        evidence_through_entry_key: entries
            .last()
            .map(|entry| entry.entry_key.clone())
            .unwrap_or_default(),
        entries: entries.to_vec(),
        verified_prefix_bytes: verified_bytes.to_vec(),
        missing_evidence: Vec::new(),
    }));
    failure
}

/// `CODE_CHAIN_INVALID` 的统一构造（`publication_incomplete = false`）。
fn chain_invalid_error(step: &str, detail: impl Into<String>) -> ScFailureEvidenceError {
    error(CODE_CHAIN_INVALID, step, None, detail, false)
}

/// `CODE_ENTRY_CONFLICT` 的统一构造（带 round 归属，`publication_incomplete = false`）。
fn entry_conflict_error(
    step: &str,
    round_id: Option<String>,
    detail: impl Into<String>,
) -> ScFailureEvidenceError {
    error(CODE_ENTRY_CONFLICT, step, round_id, detail, false)
}

/// `CODE_IO` 的统一构造（带 round 归属，`publication_incomplete = true`）。
fn io_error(
    step: &str,
    round_id: Option<String>,
    detail: impl Into<String>,
) -> ScFailureEvidenceError {
    error(CODE_IO, step, round_id, detail, true)
}

/// `CODE_LOCATOR_INVALID` 的统一构造（带 round 归属，`publication_incomplete = false`）。
fn locator_invalid_error(
    step: &str,
    round_id: Option<String>,
    detail: impl Into<String>,
) -> ScFailureEvidenceError {
    error(CODE_LOCATOR_INVALID, step, round_id, detail, false)
}

/// `CODE_PROMPT_DIGEST_MISMATCH` 的统一构造（`publication_incomplete = true`）。
fn prompt_digest_error(step: &str, detail: impl Into<String>) -> ScFailureEvidenceError {
    error(CODE_PROMPT_DIGEST_MISMATCH, step, None, detail, true)
}

/// `CODE_PROMPT_MISSING` 的统一构造（`publication_incomplete = true`）。
fn prompt_missing_error(step: &str, detail: impl Into<String>) -> ScFailureEvidenceError {
    error(CODE_PROMPT_MISSING, step, None, detail, true)
}

/// `CODE_RAW_MISSING` 的统一构造（带 round 归属，`publication_incomplete = true`）。
fn raw_missing_error(
    step: &str,
    round_id: Option<String>,
    detail: impl Into<String>,
) -> ScFailureEvidenceError {
    error(CODE_RAW_MISSING, step, round_id, detail, true)
}

/// `CODE_ROUND_NOT_BOUND` 的统一构造（带 round 归属，`publication_incomplete = false`）。
fn round_not_bound_error(
    step: &str,
    round_id: Option<String>,
    detail: impl Into<String>,
) -> ScFailureEvidenceError {
    error(CODE_ROUND_NOT_BOUND, step, round_id, detail, false)
}

/// `CODE_SCOPE_MISMATCH` 的统一构造（带 round 归属，`publication_incomplete = false`）。
fn scope_mismatch_error(
    step: &str,
    round_id: Option<String>,
    detail: impl Into<String>,
) -> ScFailureEvidenceError {
    error(CODE_SCOPE_MISMATCH, step, round_id, detail, false)
}

/// `CODE_SESSION_NOT_LIVE` 的统一构造（`publication_incomplete = true`）。
fn session_not_live_error(step: &str, detail: impl Into<String>) -> ScFailureEvidenceError {
    error(CODE_SESSION_NOT_LIVE, step, None, detail, true)
}

/// `CODE_SESSION_NOT_SC` 的统一构造（`publication_incomplete = true`）。
fn session_not_sc_error(step: &str, detail: impl Into<String>) -> ScFailureEvidenceError {
    error(CODE_SESSION_NOT_SC, step, None, detail, true)
}

fn error(
    code: &str,
    step: &str,
    round_id: Option<String>,
    detail: impl Into<String>,
    publication_incomplete: bool,
) -> ScFailureEvidenceError {
    ScFailureEvidenceError {
        code: code.to_string(),
        step: step.to_string(),
        round_id,
        detail: detail.into(),
        publication_incomplete,
        verified_prefix: None,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_digest(&hasher.finalize())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[path = "tests/sc_failure_evidence_fixture.rs"]
mod fixture;
#[cfg(test)]
#[path = "tests/sc_failure_evidence.rs"]
mod tests;
#[cfg(test)]
#[path = "tests/sc_failure_evidence_boundaries.rs"]
mod boundary_tests;
#[cfg(test)]
#[path = "tests/sc_failure_round_lookup.rs"]
mod round_lookup_tests;
#[path = "sc_failure_evidence_lookup.rs"]
mod lookup;
