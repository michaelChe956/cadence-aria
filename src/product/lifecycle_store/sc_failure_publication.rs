//! P14 publication-store barrier: strict A4 publication store helpers.
//!
//! Frozen binding (do not drift):
//! - receipt path: `<timeline-root>/sc-failure-evidence/<chain_id>/published/<package_id>.json`
//! - `validate_sc_artifact_publication` loads
//!   `load_node_detail(scope.session_id, &receipt.node_id)` and requires the node
//!   `ArtifactRef` to name this exact version/id (never receipt self-attestation);
//! - `put_sc_artifact_published_receipt` performs real SC admission: a healthy
//!   strict read, or an error whose verified prefix is real and non-empty, admits;
//!   no known prefix never writes.

use std::path::PathBuf;

use sha2::{Digest, Sha256};

use super::sc_failure_evidence::{
    sc_failure_chain_id, ScFailureEvidenceEntry, ScFailureEvidenceError, ScFailureEvidenceRead,
};
use super::utils::path_exists;
use super::LifecycleStore;
use crate::product::coding_attempt_store::locking::ExclusiveFileLock;
use crate::product::json_store::ProductStoreError;
use crate::product::models::{ArtifactRef, WorkspaceType};
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, ScArtifactPublishedReceipt, ScFailureChainIdentity,
    ScFailureScope, ScMissingEvidence, ScPackageCompleteness, ScRoundLaunchState,
};

const SC_FAILURE_EVIDENCE_DIR: &str = "sc-failure-evidence";
const SC_FAILURE_EVIDENCE_LOG: &str = "rounds.jsonl";

const CODE_IO: &str = "sc_failure_io";
const CODE_CHAIN_INVALID: &str = "sc_failure_chain_invalid";
const CODE_SCOPE_MISMATCH: &str = "sc_failure_scope_mismatch";
const CODE_SESSION_NOT_SC: &str = "sc_failure_session_not_sc";
const CODE_LOG_CORRUPT: &str = "sc_failure_log_corrupt";
const CODE_RECEIPT_MISSING: &str = "sc_failure_publication_receipt_missing";
const CODE_RECEIPT_CORRUPT: &str = "sc_failure_publication_receipt_corrupt";
const CODE_RECEIPT_MISMATCH: &str = "sc_failure_publication_receipt_mismatch";
const CODE_VERSION_MISSING: &str = "sc_failure_publication_version_missing";
const CODE_NOT_A4: &str = "sc_failure_publication_not_a4";
const CODE_NODE_REF: &str = "sc_failure_publication_node_ref_mismatch";

/// Local (non-durable) terminal projection used when the strict evidence log is
/// only partially readable: the publisher builds from the real verified prefix
/// (never a fabricated append entry / raw digest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScLocalTerminal {
    pub(crate) last_failure_round_id: String,
    pub(crate) last_physical_round_id: Option<String>,
    pub(crate) terminal_class: String,
    pub(crate) terminal_reason: String,
    pub(crate) launch_observations: Vec<(String, ScRoundLaunchState)>,
    pub(crate) completeness: ScPackageCompleteness,
    pub(crate) missing_evidence: Vec<ScMissingEvidence>,
}

/// Builder terminal input: `Durable` keeps the original strict fold; `Local`
/// carries only real verified-prefix facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScTerminalInput<'a> {
    Durable,
    Local(&'a ScLocalTerminal),
}

impl LifecycleStore {
    /// 写读不可变回执：真实 SC 准入（健康读或受控已验证前缀）→ live lease →
    /// sidecar 排它锁 → `create_new` + `sync_all` + 回读。回执只证明工件已发布，
    /// 不冒充证据日志健康。
    pub(crate) fn put_sc_artifact_published_receipt(
        &self,
        identity: &ScFailureChainIdentity,
        receipt: &ScArtifactPublishedReceipt,
    ) -> Result<(), ScFailureEvidenceError> {
        const STEP: &str = "put_sc_artifact_published_receipt";
        let expected = sc_failure_chain_id(
            &identity.scope.session_id,
            &identity.business_key,
            &identity.original_node_id,
            identity.original_role_run_seq,
        )
        .map_err(|mut failure| {
            failure.step = STEP.to_string();
            failure
        })?;
        if expected != identity.chain_id {
            return Err(publication_error(
                CODE_CHAIN_INVALID,
                STEP,
                "chain_id 与身份四元组不一致",
                false,
            ));
        }
        if receipt.schema_version != 1 || receipt.chain_id != identity.chain_id {
            return Err(publication_error(
                CODE_RECEIPT_MISMATCH,
                STEP,
                "receipt 与身份不一致",
                true,
            ));
        }
        if !is_safe_segment(&receipt.package_id) {
            return Err(publication_error(
                CODE_RECEIPT_MISMATCH,
                STEP,
                "package_id 非法",
                true,
            ));
        }
        self.sc_failure_publication_sc_session(&identity.scope, STEP)?;
        self.sc_failure_publication_admission(identity, STEP)?;

        let path = self
            .sc_failure_receipt_path(
                &identity.scope.session_id,
                &identity.chain_id,
                &receipt.package_id,
            )
            .map_err(|error| publication_error(CODE_IO, STEP, format!("{error}"), true))?;
        let bytes = serde_json::to_vec(receipt).map_err(|error| {
            publication_error(CODE_IO, STEP, format!("serialize receipt: {error}"), true)
        })?;
        let (_, scope) = self
            .resolve_session_scope(&identity.scope.session_id)
            .map_err(|error| publication_error(CODE_IO, STEP, format!("{error}"), true))?;
        let outcome = self
            .with_optional_entity_live_write(scope.as_ref(), || {
                let inner = (|| -> Result<(), ScFailureEvidenceError> {
                    let _lock = ExclusiveFileLock::acquire(&path).map_err(|error| {
                        publication_error(CODE_IO, STEP, format!("lock: {error}"), true)
                    })?;
                    if path_exists(&path).map_err(|error| {
                        publication_error(CODE_IO, STEP, format!("{error}"), true)
                    })? {
                        let existing = std::fs::read(&path).map_err(|error| {
                            publication_error(
                                CODE_IO,
                                STEP,
                                format!("read existing receipt: {error}"),
                                true,
                            )
                        })?;
                        if existing == bytes {
                            return Ok(());
                        }
                        return Err(publication_error(
                            CODE_RECEIPT_MISMATCH,
                            STEP,
                            "receipt 不可变：同 package_id 已存在不同回执",
                            true,
                        ));
                    }
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).map_err(|error| {
                            publication_error(CODE_IO, STEP, format!("mkdir: {error}"), true)
                        })?;
                    }
                    {
                        use std::io::Write;
                        let mut file = std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&path)
                            .map_err(|error| {
                                publication_error(
                                    CODE_IO,
                                    STEP,
                                    format!("create_new: {error}"),
                                    true,
                                )
                            })?;
                        file.write_all(&bytes).map_err(|error| {
                            publication_error(CODE_IO, STEP, format!("write: {error}"), true)
                        })?;
                        file.sync_all().map_err(|error| {
                            publication_error(CODE_IO, STEP, format!("sync: {error}"), true)
                        })?;
                    }
                    let readback = std::fs::read(&path).map_err(|error| {
                        publication_error(CODE_IO, STEP, format!("readback: {error}"), true)
                    })?;
                    if readback != bytes {
                        return Err(publication_error(CODE_IO, STEP, "回读字节不一致", true));
                    }
                    Ok(())
                })();
                Ok(inner)
            })
            .map_err(|error| {
                publication_error(CODE_IO, STEP, format!("live_write: {error}"), true)
            })?;
        outcome
    }

    /// 公开视图屏障：普通版本原样放行；A4 仅在其 durable 回执 + 节点引用与
    /// 持久化版本一致时放行；回执缺失按 pending 剔除；坏回执/坏引用显式 Err
    /// （绝不返回空表冒充成功）。
    pub(crate) fn validate_sc_artifact_publication(
        &self,
        scope: &ScFailureScope,
        version: &ArtifactVersion,
    ) -> Result<(), ScFailureEvidenceError> {
        const STEP: &str = "validate_sc_artifact_publication";
        let raw = self
            .list_artifact_versions(&scope.session_id)
            .map_err(|error| publication_error(CODE_IO, STEP, format!("{error}"), true))?;
        let entry = raw
            .iter()
            .find(|entry| entry.version == version.version)
            .ok_or_else(|| {
                publication_error(CODE_VERSION_MISSING, STEP, "版本不在 raw 表中", true)
            })?;
        let package = match &entry.payload {
            ArtifactPayload::ScFailureRepairPackage {
                sc_failure_repair_package,
            } => sc_failure_repair_package.as_ref(),
            _ => {
                return Err(publication_error(
                    CODE_NOT_A4,
                    STEP,
                    "版本不是 SC 失败修复包",
                    true,
                ))
            }
        };
        if package.session_id != scope.session_id
            || package.project_id != scope.project_id
            || package.issue_id != scope.issue_id
            || package.plan_id != scope.plan_id
        {
            return Err(publication_error(
                CODE_SCOPE_MISMATCH,
                STEP,
                "包 scope 与会话 scope 不一致",
                true,
            ));
        }
        let sessions = self
            .list_workspace_sessions(&scope.project_id, &scope.issue_id)
            .map_err(|error| publication_error(CODE_IO, STEP, format!("{error}"), true))?;
        if !sessions
            .iter()
            .any(|session| session.id == scope.session_id)
        {
            return Err(publication_error(
                CODE_SCOPE_MISMATCH,
                STEP,
                "会话不是真实的 durable 会话",
                true,
            ));
        }
        let digest = sc_failure_package_digest(package).map_err(|mut failure| {
            failure.step = STEP.to_string();
            failure
        })?;
        let receipt_path = self
            .sc_failure_receipt_path(&scope.session_id, &package.chain_id, &package.package_id)
            .map_err(|error| publication_error(CODE_IO, STEP, format!("{error}"), true))?;
        if !path_exists(&receipt_path)
            .map_err(|error| publication_error(CODE_IO, STEP, format!("{error}"), true))?
        {
            return Err(publication_error(
                CODE_RECEIPT_MISSING,
                STEP,
                "回执缺失（pending）",
                true,
            ));
        }
        let bytes = std::fs::read(&receipt_path)
            .map_err(|error| publication_error(CODE_IO, STEP, format!("{error}"), true))?;
        let receipt: ScArtifactPublishedReceipt = serde_json::from_slice(&bytes).map_err(
            |error| {
                publication_error(
                    CODE_RECEIPT_CORRUPT,
                    STEP,
                    format!("回执无法解析: {error}"),
                    true,
                )
            },
        )?;
        if receipt.schema_version != 1
            || receipt.package_id != package.package_id
            || receipt.chain_id != package.chain_id
            || receipt.version != entry.version
            || receipt.package_digest != digest
            || receipt.node_id != entry.source_node_id
        {
            return Err(publication_error(
                CODE_RECEIPT_MISMATCH,
                STEP,
                "回执与包/版本/节点绑定不一致",
                true,
            ));
        }
        let detail = self
            .load_node_detail(&scope.session_id, &receipt.node_id)
            .map_err(|error| publication_error(CODE_IO, STEP, format!("{error}"), true))?;
        let expected_ref = ArtifactRef {
            artifact_id: format!("artifact_version_{:03}", entry.version),
            version: entry.version,
        };
        if detail.artifact_ref.as_ref() != Some(&expected_ref) {
            return Err(publication_error(
                CODE_NODE_REF,
                STEP,
                "节点 ArtifactRef 与版本不一致",
                true,
            ));
        }
        Ok(())
    }

    /// 最终公开版本视图过滤：只在 session_state / 公开 REST 使用；内部 raw 读不变。
    pub(crate) fn filter_sc_published_artifact_versions(
        &self,
        scope: &ScFailureScope,
        versions: Vec<ArtifactVersion>,
    ) -> Result<Vec<ArtifactVersion>, ScFailureEvidenceError> {
        let mut published = Vec::with_capacity(versions.len());
        for version in versions {
            if !matches!(
                &version.payload,
                ArtifactPayload::ScFailureRepairPackage { .. }
            ) {
                published.push(version);
                continue;
            }
            match self.validate_sc_artifact_publication(scope, &version) {
                Ok(()) => published.push(version),
                Err(failure) if failure.code == CODE_RECEIPT_MISSING => continue,
                Err(failure) => return Err(failure),
            }
        }
        Ok(published)
    }

    /// 从真实首行日志重建链身份后再做严格读；坏尾保留已验证前缀（不伪造、
    /// 不回退“最新”），非 SC 身份一律拒绝。
    pub(crate) fn read_sc_failure_chain_for_publication(
        &self,
        scope: &ScFailureScope,
        chain_id: &str,
    ) -> Result<(ScFailureChainIdentity, ScFailureEvidenceRead), ScFailureEvidenceError> {
        const STEP: &str = "read_sc_failure_chain_for_publication";
        if !is_valid_chain_id(chain_id) {
            return Err(publication_error(
                CODE_CHAIN_INVALID,
                STEP,
                "chain_id 非法",
                false,
            ));
        }
        self.sc_failure_publication_sc_session(scope, STEP)?;
        let root = self
            .workspace_timeline_root_for_session(&scope.session_id)
            .map_err(|error| publication_error(CODE_IO, STEP, format!("{error}"), false))?;
        let log = root
            .join(SC_FAILURE_EVIDENCE_DIR)
            .join(chain_id)
            .join(SC_FAILURE_EVIDENCE_LOG);
        let bytes = std::fs::read(&log).map_err(|error| {
            publication_error(
                CODE_IO,
                STEP,
                format!("read {}: {error}", log.display()),
                false,
            )
        })?;
        // 首物理行，不跳过空行：坏/空首行不得退到后续行伪造身份。
        let first_line_end = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap_or(bytes.len());
        let first_line = &bytes[..first_line_end];
        if first_line.iter().all(|byte| byte.is_ascii_whitespace()) {
            return Err(publication_error(
                CODE_LOG_CORRUPT,
                STEP,
                "证据日志首物理行为空",
                false,
            ));
        }
        let entry: ScFailureEvidenceEntry =
            serde_json::from_slice(first_line).map_err(|error| {
                publication_error(
                    CODE_LOG_CORRUPT,
                    STEP,
                    format!("首行无法解析: {error}"),
                    false,
                )
            })?;
        let identity = entry.chain.clone();
        if &identity.scope != scope {
            return Err(publication_error(
                CODE_SCOPE_MISMATCH,
                STEP,
                "首行 chain scope 与会话 scope 不一致",
                false,
            ));
        }
        if identity.chain_id != chain_id {
            return Err(publication_error(
                CODE_CHAIN_INVALID,
                STEP,
                "首行 chain_id 与请求不一致",
                false,
            ));
        }
        let expected = sc_failure_chain_id(
            &identity.scope.session_id,
            &identity.business_key,
            &identity.original_node_id,
            identity.original_role_run_seq,
        )
        .map_err(|mut failure| {
            failure.step = STEP.to_string();
            failure
        })?;
        if expected != chain_id {
            return Err(publication_error(
                CODE_CHAIN_INVALID,
                STEP,
                "chain_id 不能由首行身份重算得到",
                false,
            ));
        }
        let read = self.read_sc_failure_evidence(&identity)?;
        Ok((identity, read))
    }

    /// 会话必须是真实的 SingleCandidate SC 会话（durable session 检查）。
    fn sc_failure_publication_sc_session(
        &self,
        scope: &ScFailureScope,
        step: &str,
    ) -> Result<(), ScFailureEvidenceError> {
        let sessions = self
            .list_workspace_sessions(&scope.project_id, &scope.issue_id)
            .map_err(|error| publication_error(CODE_IO, step, format!("{error}"), false))?;
        let session = sessions
            .iter()
            .find(|session| session.id == scope.session_id)
            .ok_or_else(|| publication_error(CODE_SCOPE_MISMATCH, step, "会话不存在", false))?;
        if session.workspace_type != WorkspaceType::WorkItemPlan
            || session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
        {
            return Err(publication_error(
                CODE_SESSION_NOT_SC,
                step,
                "会话不是 SC 会话",
                false,
            ));
        }
        Ok(())
    }

    /// 真实 SC 准入：健康严格读或受控已验证前缀（条目非空且链身份一致）放行；
    /// 无已知前缀拒绝（不写回执）。
    fn sc_failure_publication_admission(
        &self,
        identity: &ScFailureChainIdentity,
        step: &str,
    ) -> Result<(), ScFailureEvidenceError> {
        match self.read_sc_failure_evidence(identity) {
            Ok(_) => Ok(()),
            Err(mut failure) => {
                let real_prefix = failure.verified_prefix.as_ref().is_some_and(|prefix| {
                    !prefix.entries.is_empty()
                        && prefix.entries.iter().all(|entry| &entry.chain == identity)
                });
                if real_prefix {
                    return Ok(());
                }
                failure.step = step.to_string();
                if !failure.publication_incomplete {
                    failure.publication_incomplete = true;
                }
                Err(failure)
            }
        }
    }

    fn sc_failure_receipt_path(
        &self,
        session_id: &str,
        chain_id: &str,
        package_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        if !is_valid_chain_id(chain_id) {
            return Err(ProductStoreError::InvalidRecord {
                kind: "sc_failure_receipt",
                reason: "chain_id_invalid".to_string(),
            });
        }
        if !is_safe_segment(package_id) {
            return Err(ProductStoreError::InvalidRecord {
                kind: "sc_failure_receipt",
                reason: "package_id_invalid".to_string(),
            });
        }
        Ok(self
            .workspace_timeline_root_for_session(session_id)?
            .join(SC_FAILURE_EVIDENCE_DIR)
            .join(chain_id)
            .join("published")
            .join(format!("{package_id}.json")))
    }
}

/// 包内容摘要：与 P11 builder 的 `package_id` 配方同源（固定 struct 字段序
/// `serde_json::to_vec` 紧凑序列化 → SHA-256 小写 hex）。receipt 与 merge 共用。
pub(crate) fn sc_failure_package_digest(
    package: &crate::web::workspace_ws_types::ScFailureRepairPackage,
) -> Result<String, ScFailureEvidenceError> {
    let bytes = serde_json::to_vec(package).map_err(|error| {
        publication_error(
            CODE_IO,
            "sc_failure_package_digest",
            format!("serialize package: {error}"),
            false,
        )
    })?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn publication_error(
    code: &str,
    step: &str,
    detail: impl Into<String>,
    publication_incomplete: bool,
) -> ScFailureEvidenceError {
    ScFailureEvidenceError {
        code: code.to_string(),
        step: step.to_string(),
        round_id: None,
        detail: detail.into(),
        publication_incomplete,
        verified_prefix: None,
    }
}

fn is_valid_chain_id(chain_id: &str) -> bool {
    let Some(hex) = chain_id.strip_prefix("chain-v1-") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_safe_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
}

#[cfg(test)]
#[path = "sc_failure_publication_tests.rs"]
mod tests;
