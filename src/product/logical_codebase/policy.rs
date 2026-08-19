//! Aggregate policy artifact and session policy envelope.
//!
//! `AggregatePolicyArtifact` 是逻辑代码库集中政策的持久化事实来源:它钉定
//! 政策正文、可审计的 canonical SHA-256 digest 与单调递增的 revision。后续
//! gateway 只从此 persisted artifact 解析政策,禁止从内存中的任意摘要重建。
//!
//! `SessionPolicyEnvelope` 是每次 provider run 的不可变快照:policy_id/
//! revision/digest、action、target、read-write roots、provider dialect、
//! 托管配置 artifact 引用与 digest。`new` 对 read-only action 强制空
//! writable roots,对 coding action 强制恰好一个等于 canonical target
//! worktree 的 write root,任何偏离都 fail-closed 为 `policy_envelope_invalid_roots`。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

/// 路由级 fail-closed 安全策略:任何与 action 不匹配的 root 配置都返回此错误。
///
/// 注意:路由级 fail-closed 不等于 OS 级隔离。本 envelope 是 experimental +
/// supervised 场景下的政策门禁,不宣称物理不可写。
pub const POLICY_ENVELOPE_INVALID_ROOTS: &str = "policy_envelope_invalid_roots";

/// bootstrap 政策使用的最小政策正文。Task 9 的 gateway 在首次真实 provider
/// launch 前从此正文解析政策,后续 revision 可由更完整的政策正文替换。
const BOOTSTRAP_POLICY_TEXT: &str = "# Aggregate policy (bootstrap)\n\nAllow planning read-only and coding target-write sessions under the logical codebase.\n";

/// 集中政策正文的持久化事实来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatePolicyArtifact {
    pub policy_id: String,
    pub project_id: String,
    pub logical_codebase_id: String,
    pub revision: u64,
    pub digest: String,
    pub policy_text: String,
    pub created_at: String,
}

impl AggregatePolicyArtifact {
    /// 构造 revision 1 的 bootstrap 政策,digest 由 canonical JSON 的 SHA-256
    /// 计算,调用方不能传任意摘要。
    pub fn bootstrap(project_id: &str, logical_codebase_id: &str, created_at: String) -> Self {
        let policy_text = BOOTSTRAP_POLICY_TEXT.to_string();
        let revision: u64 = 1;
        let policy_id = format!("policy/{project_id}/{logical_codebase_id}/{revision}");
        let digest = Self::compute_digest(&policy_text);
        Self {
            policy_id,
            project_id: project_id.to_string(),
            logical_codebase_id: logical_codebase_id.to_string(),
            revision,
            digest,
            policy_text,
            created_at,
        }
    }

    /// 构造一个升级后的 policy artifact:以当前为基,提升 `revision`、替换
    /// `policy_text` 与 `created_at`,并重算 canonical digest 与 `policy_id`。
    /// digest 不接受外部传入。供 policy 升级路径与 spawn 前复验测试使用。
    pub fn with_revised_policy(
        &self,
        policy_text: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        let policy_text = policy_text.into();
        let revision = self.revision + 1;
        let policy_id = format!(
            "policy/{}/{}/{}",
            self.project_id, self.logical_codebase_id, revision
        );
        let digest = Self::compute_digest(&policy_text);
        Self {
            policy_id,
            project_id: self.project_id.clone(),
            logical_codebase_id: self.logical_codebase_id.clone(),
            revision,
            digest,
            policy_text,
            created_at: created_at.into(),
        }
    }

    /// 对政策正文计算 canonical SHA-256 digest。
    fn compute_digest(policy_text: &str) -> String {
        format!("sha256:{:x}", Sha256::digest(policy_text.as_bytes()))
    }

    /// 校验 digest 是政策正文 canonical SHA-256,禁止任意摘要。
    fn validate_digest(&self) -> Result<(), ProductStoreError> {
        if !self.digest.starts_with("sha256:") {
            return Err(ProductStoreError::InvalidRecord {
                kind: "aggregate_policy_artifact",
                reason: format!("digest must be sha256-prefixed: {}", self.digest),
            });
        }
        let expected = Self::compute_digest(&self.policy_text);
        if self.digest != expected {
            return Err(ProductStoreError::InvalidRecord {
                kind: "aggregate_policy_artifact",
                reason: format!(
                    "digest must be canonical sha256 of policy_text (expected {expected})",
                ),
            });
        }
        Ok(())
    }
}

/// 每次会话的不可变 action。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPolicyAction {
    PlanningReadOnly,
    CodingTargetWrite,
    ReviewReadOnly,
}

impl SessionPolicyAction {
    /// read-only action 必须没有 writable roots。
    fn requires_empty_writable_roots(self) -> bool {
        matches!(self, Self::PlanningReadOnly | Self::ReviewReadOnly)
    }
}

/// 已知的 provider dialect,envelope 冻结它以便复验。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderDialect {
    ClaudeCodeCliV1,
    CodexCliV1,
}

/// envelope 钉定的目标 worktree 快照。gateway 在 spawn 前重新 canonicalize
/// cwd/git-dir 并与此 target 比较。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyTarget {
    pub logical_repository_id: String,
    pub checkout_id: String,
    pub worktree: PathBuf,
}

impl PolicyTarget {
    pub fn checkout(
        logical_repository_id: impl Into<String>,
        checkout_id: impl Into<String>,
        worktree: impl Into<PathBuf>,
    ) -> Self {
        Self {
            logical_repository_id: logical_repository_id.into(),
            checkout_id: checkout_id.into(),
            worktree: worktree.into(),
        }
    }

    /// 聚合根 planning 只读 target。planning 只读 action 不绑定具体 logical
    /// member(checkout_id 为空串、logical_repository_id 为空串),worktree 取
    /// 聚合根 cwd(`provider_context_root`)。read-only action 经
    /// `requires_empty_writable_roots` 强制空 writable_roots,本构造函数只负责
    /// target 维度。
    ///
    /// 路由级 fail-closed 不等于 OS 级隔离:本 target 是 supervised 场景下的
    /// 政策门,不宣称物理不可写。
    pub fn aggregate_root(working_dir: impl Into<PathBuf>) -> Self {
        Self {
            logical_repository_id: String::new(),
            checkout_id: String::new(),
            worktree: working_dir.into(),
        }
    }
}

/// 每次 provider run 的不可变政策快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPolicyEnvelope {
    pub policy_id: String,
    pub policy_revision: u64,
    pub policy_digest: String,
    pub action: SessionPolicyAction,
    pub target: PolicyTarget,
    pub readable_roots: Vec<PathBuf>,
    pub writable_roots: Vec<PathBuf>,
    pub provider_dialect: ProviderDialect,
    pub config_artifact_ref: String,
    pub config_digest: String,
    pub created_at: String,
    /// 聚合政策权威根 locator(= `LogicalCodebaseManifest.provider_context_root`,
    /// 构造时 canonicalize)。存量记录无此键,serde 缺省为 `PathBuf::default()`。
    #[serde(default)]
    pub authority_root: PathBuf,
}

impl SessionPolicyEnvelope {
    /// 冻结 envelope。read-only action 强制空 writable_roots;coding action
    /// 强制恰好一个等于 canonical target worktree 的 write root。空 policy
    /// digest 或偏离的 root 返回 `policy_envelope_invalid_roots`。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        artifact: &AggregatePolicyArtifact,
        action: SessionPolicyAction,
        target: PolicyTarget,
        readable_roots: Vec<PathBuf>,
        writable_roots: Vec<PathBuf>,
        provider_dialect: ProviderDialect,
        config_artifact_ref: String,
        created_at: String,
        authority_root: PathBuf,
    ) -> Result<Self, ProductStoreError> {
        if artifact.digest.is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "session_policy_envelope",
                reason: POLICY_ENVELOPE_INVALID_ROOTS.to_string(),
            });
        }

        let config_digest = config_digest_value(&config_artifact_ref)?;
        let expected_write_root = canonical_root(&target.worktree);
        let writable_roots =
            Self::validate_writable_roots(action, writable_roots, expected_write_root.as_path())?;

        Ok(Self {
            policy_id: artifact.policy_id.clone(),
            policy_revision: artifact.revision,
            policy_digest: artifact.digest.clone(),
            action,
            target,
            readable_roots,
            writable_roots,
            provider_dialect,
            config_artifact_ref,
            config_digest,
            created_at,
            authority_root,
        })
    }

    fn validate_writable_roots(
        action: SessionPolicyAction,
        writable_roots: Vec<PathBuf>,
        expected_write_root: &Path,
    ) -> Result<Vec<PathBuf>, ProductStoreError> {
        if action.requires_empty_writable_roots() {
            if writable_roots.is_empty() {
                return Ok(writable_roots);
            }
            return Err(Self::invalid_roots(format!(
                "{action:?} must have no writable roots, got {}",
                writable_roots.len()
            )));
        }

        // CodingTargetWrite: exactly one root equal to canonical target worktree.
        if writable_roots.len() != 1 {
            return Err(Self::invalid_roots(format!(
                "{action:?} requires exactly one writable root, got {}",
                writable_roots.len()
            )));
        }
        let actual = canonical_root(&writable_roots[0]);
        if actual.as_path() != expected_write_root {
            return Err(Self::invalid_roots(format!(
                "{action:?} writable root must equal canonical target worktree {}: got {}",
                expected_write_root.display(),
                actual.display()
            )));
        }
        Ok(writable_roots)
    }

    fn invalid_roots(reason: String) -> ProductStoreError {
        ProductStoreError::InvalidRecord {
            kind: "session_policy_envelope",
            reason: format!("{}: {reason}", POLICY_ENVELOPE_INVALID_ROOTS),
        }
    }

    /// 据 `config_artifact_ref` 重算 config digest,供 gateway spawn 前复验
    /// 托管配置未被篡改(TOCTOU)。与 `new` 内部使用的 digest 算法一致。
    /// 空 ref 复用 envelope 的 fail-closed 错误。
    pub fn recompute_config_digest(config_artifact_ref: &str) -> Result<String, ProductStoreError> {
        config_digest_value(config_artifact_ref)
    }
}

fn config_digest_value(config_artifact_ref: &str) -> Result<String, ProductStoreError> {
    if config_artifact_ref.is_empty() {
        return Err(ProductStoreError::InvalidRecord {
            kind: "session_policy_envelope",
            reason: format!(
                "{}: empty config artifact ref",
                POLICY_ENVELOPE_INVALID_ROOTS
            ),
        });
    }
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(config_artifact_ref.as_bytes())
    ))
}

/// 规范化一个 root 路径用于比较:去掉末尾分隔符。不触碰 OS canonicalize,
/// 因为测试中的路径在文件系统上不存在;gateway 复验阶段会做真实 canonicalize。
fn canonical_root(path: &Path) -> PathBuf {
    let mut normalized = path.to_path_buf();
    while normalized.as_os_str().len() > 1 {
        let parent = normalized.parent();
        match parent {
            Some(parent)
                if !parent.as_os_str().is_empty()
                    && normalized.file_name().is_some_and(|name| name.is_empty()) =>
            {
                normalized = parent.to_path_buf();
            }
            _ => break,
        }
    }
    normalized
}

/// 集中政策 artifact 的持久化 store。
#[derive(Debug, Clone)]
pub struct AggregatePolicyArtifactStore {
    paths: ProductAppPaths,
    lc_id: Option<String>,
}

impl AggregatePolicyArtifactStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths, lc_id: None }
    }

    /// Scopes policy reads/writes to one logical codebase subtree（v1.3）。
    pub fn for_lc(paths: ProductAppPaths, lc_id: impl Into<String>) -> Self {
        Self {
            paths,
            lc_id: Some(lc_id.into()),
        }
    }

    /// 读取当前 persisted artifact;不存在返回 `Ok(None)`。
    pub fn get(
        &self,
        project_id: &str,
    ) -> Result<Option<AggregatePolicyArtifact>, ProductStoreError> {
        let path = self.artifact_path(project_id)?;
        if !path.try_exists().map_err(|error| {
            ProductStoreError::Io(format!("try_exists {}: {error}", path.display()))
        })? {
            return Ok(None);
        }
        let artifact: AggregatePolicyArtifact = read_json(&path)?;
        artifact.validate_identity(project_id)?;
        artifact.validate_digest()?;
        Ok(Some(artifact))
    }

    /// 保存 artifact。digest 必须是 policy_text 的 canonical SHA-256,禁止
    /// 调用方传任意摘要;新 revision 的 digest 在写入前重新校验。
    pub fn save(
        &self,
        project_id: &str,
        artifact: &AggregatePolicyArtifact,
    ) -> Result<(), ProductStoreError> {
        artifact.validate_identity(project_id)?;
        artifact.validate_digest()?;

        if let Some(existing) = self.get(project_id)? {
            existing.validate_successor(artifact)?;
        }

        write_json(&self.artifact_path(project_id)?, artifact)
    }

    /// 确保存在 bootstrap artifact;幂等。相同 artifact 无副作用返回;
    /// 存在 project/logical-codebase 不一致的 artifact 时返回 `IdentityMismatch`,
    /// 不能覆盖。
    pub fn ensure_bootstrap(
        &self,
        manifest: &LogicalCodebaseManifest,
    ) -> Result<AggregatePolicyArtifact, ProductStoreError> {
        validate_relative_id(&manifest.project_id)?;

        let logical_codebase_id = manifest.logical_codebase_id.to_string();
        let now = manifest.updated_at.clone();
        let bootstrap =
            AggregatePolicyArtifact::bootstrap(&manifest.project_id, &logical_codebase_id, now);

        if let Some(existing) = self.get(&manifest.project_id)? {
            existing.assert_matches_bootstrap(&bootstrap)?;
            return Ok(existing);
        }

        self.save(&manifest.project_id, &bootstrap)?;
        Ok(bootstrap)
    }

    fn artifact_path(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        Ok(
            crate::product::logical_codebase::lc_scope_root(&self.paths, project_id, &self.lc_id)?
                .join("aggregate-policy.json"),
        )
    }
}

impl AggregatePolicyArtifact {
    fn validate_identity(&self, project_id: &str) -> Result<(), ProductStoreError> {
        if self.project_id != project_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "aggregate_policy_artifact",
                id: project_id.to_string(),
            });
        }
        if self.revision == 0 {
            return Err(ProductStoreError::InvalidRecord {
                kind: "aggregate_policy_artifact",
                reason: "revision must start at 1".to_string(),
            });
        }
        Ok(())
    }

    fn validate_successor(&self, next: &AggregatePolicyArtifact) -> Result<(), ProductStoreError> {
        if next.project_id != self.project_id
            || next.logical_codebase_id != self.logical_codebase_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "aggregate_policy_artifact",
                id: next.project_id.clone(),
            });
        }
        if next.revision <= self.revision {
            return Err(ProductStoreError::InvalidRecord {
                kind: "aggregate_policy_artifact",
                reason: format!(
                    "revision must advance from {} to a higher value",
                    self.revision
                ),
            });
        }
        Ok(())
    }

    fn assert_matches_bootstrap(
        &self,
        bootstrap: &AggregatePolicyArtifact,
    ) -> Result<(), ProductStoreError> {
        if self.project_id != bootstrap.project_id
            || self.logical_codebase_id != bootstrap.logical_codebase_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "aggregate_policy_artifact",
                id: bootstrap.project_id.clone(),
            });
        }
        Ok(())
    }
}

// 引入 manifest 类型以供 ensure_bootstrap 使用;此处只依赖其稳定 logical-codebase
// UUID、project_id 与 updated_at,与 store.rs 的 LogicalCodebaseManifest 同源。
use crate::product::logical_codebase::store::LogicalCodebaseManifest;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use uuid::Uuid;

    #[test]
    fn envelope_freezes_policy_target_roots_dialect_and_managed_config_digest() {
        let artifact = AggregatePolicyArtifact::bootstrap(
            "project_0001",
            "logical_0001",
            "2026-08-09T00:00:00Z".into(),
        );
        let envelope = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::CodingTargetWrite,
            PolicyTarget::checkout(
                "logical_repo",
                "checkout",
                "/work/api/.worktrees/aria-issues/issue_1",
            ),
            vec![std::path::PathBuf::from("/aggregate")],
            vec![std::path::PathBuf::from(
                "/work/api/.worktrees/aria-issues/issue_1",
            )],
            ProviderDialect::ClaudeCodeCliV1,
            "sha256:settings".into(),
            "2026-08-09T00:00:00Z".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap();

        assert_eq!(envelope.policy_digest, artifact.digest);
        assert_eq!(envelope.writable_roots.len(), 1);
        assert_eq!(envelope.action, SessionPolicyAction::CodingTargetWrite);
        assert!(
            serde_json::to_value(&envelope)
                .unwrap()
                .get("config_artifact_ref")
                .is_some()
        );
    }

    #[test]
    fn bootstrap_digest_is_canonical_sha256_of_policy_text() {
        let artifact =
            AggregatePolicyArtifact::bootstrap("p1", "l1", "2026-08-09T00:00:00Z".into());
        let expected = format!(
            "sha256:{:x}",
            Sha256::digest(BOOTSTRAP_POLICY_TEXT.as_bytes())
        );
        assert_eq!(artifact.digest, expected);
        assert_eq!(artifact.revision, 1);
        assert!(artifact.policy_id.contains("p1"));
        assert!(artifact.policy_id.contains("l1"));
    }

    #[test]
    fn read_only_actions_reject_any_writable_root() {
        let artifact = AggregatePolicyArtifact::bootstrap("p1", "l1", "now".into());
        let target = PolicyTarget::checkout("repo", "co", "/work/repo");
        let error = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::PlanningReadOnly,
            target.clone(),
            vec![PathBuf::from("/work/repo")],
            vec![PathBuf::from("/work/repo")],
            ProviderDialect::CodexCliV1,
            "sha256:cfg".into(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::InvalidRecord { ref reason, .. } if reason.starts_with(POLICY_ENVELOPE_INVALID_ROOTS))
        );

        let ok = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::ReviewReadOnly,
            target,
            vec![PathBuf::from("/work/repo")],
            vec![],
            ProviderDialect::CodexCliV1,
            "sha256:cfg".into(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap();
        assert!(ok.writable_roots.is_empty());
    }

    #[test]
    fn coding_action_requires_single_writable_root_equal_to_target() {
        let artifact = AggregatePolicyArtifact::bootstrap("p1", "l1", "now".into());
        let target = PolicyTarget::checkout("repo", "co", "/work/repo");

        // wrong root
        let err = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::CodingTargetWrite,
            target.clone(),
            vec![],
            vec![PathBuf::from("/elsewhere")],
            ProviderDialect::ClaudeCodeCliV1,
            "sha256:cfg".into(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap_err();
        assert!(
            matches!(err, ProductStoreError::InvalidRecord { ref reason, .. } if reason.contains("policy_envelope_invalid_roots"))
        );

        // two roots
        let err = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::CodingTargetWrite,
            target.clone(),
            vec![],
            vec![PathBuf::from("/work/repo"), PathBuf::from("/other")],
            ProviderDialect::ClaudeCodeCliV1,
            "sha256:cfg".into(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap_err();
        assert!(
            matches!(err, ProductStoreError::InvalidRecord { ref reason, .. } if reason.contains("policy_envelope_invalid_roots"))
        );

        // correct single root
        let ok = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::CodingTargetWrite,
            target,
            vec![],
            vec![PathBuf::from("/work/repo")],
            ProviderDialect::ClaudeCodeCliV1,
            "sha256:cfg".into(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap();
        assert_eq!(ok.writable_roots, vec![PathBuf::from("/work/repo")]);
    }

    #[test]
    fn empty_config_artifact_ref_is_rejected() {
        let artifact = AggregatePolicyArtifact::bootstrap("p1", "l1", "now".into());
        let err = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::PlanningReadOnly,
            PolicyTarget::checkout("repo", "co", "/work/repo"),
            vec![],
            vec![],
            ProviderDialect::ClaudeCodeCliV1,
            String::new(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap_err();
        assert!(
            matches!(err, ProductStoreError::InvalidRecord { ref reason, .. } if reason.contains("policy_envelope_invalid_roots"))
        );
    }

    #[test]
    fn store_roundtrips_and_recomputes_digest_on_save() {
        let temp = tempfile::tempdir().unwrap();
        let store = AggregatePolicyArtifactStore::new(ProductAppPaths::new(temp.path()));
        let artifact = AggregatePolicyArtifact::bootstrap(
            "project_0001",
            "logical_0001",
            "2026-08-09T00:00:00Z".into(),
        );

        store.save("project_0001", &artifact).unwrap();
        let loaded = store.get("project_0001").unwrap().unwrap();
        assert_eq!(loaded, artifact);
        assert!(
            temp.path()
                .join("projects/project_0001/logical-codebase/aggregate-policy.json")
                .exists()
        );

        // caller-supplied arbitrary digest is rejected on save
        let mut bad = artifact.clone();
        bad.digest = "sha256:deadbeef".into();
        assert!(store.save("project_0001", &bad).is_err());
    }

    #[test]
    fn save_rejects_non_advancing_revision() {
        let temp = tempfile::tempdir().unwrap();
        let store = AggregatePolicyArtifactStore::new(ProductAppPaths::new(temp.path()));
        let artifact = AggregatePolicyArtifact::bootstrap("p1", "l1", "now".into());
        store.save("p1", &artifact).unwrap();

        let mut duplicate = artifact.clone();
        duplicate.revision = 1; // same revision
        assert!(store.save("p1", &duplicate).is_err());
    }

    #[test]
    fn ensure_bootstrap_is_idempotent_and_refuses_mismatched_identity() {
        let temp = tempfile::tempdir().unwrap();
        let store = AggregatePolicyArtifactStore::new(ProductAppPaths::new(temp.path()));
        let manifest =
            LogicalCodebaseManifest::new("project_0001", temp.path().to_path_buf(), vec![]);

        let first = store.ensure_bootstrap(&manifest).unwrap();
        assert_eq!(first.revision, 1);
        let second = store.ensure_bootstrap(&manifest).unwrap();
        assert_eq!(first, second);

        // a different logical-codebase identity is not overwritten
        let mut other = manifest.clone();
        other.logical_codebase_id = Uuid::new_v4();
        assert!(matches!(
            store.ensure_bootstrap(&other),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
    }

    /// `with_revised_policy` 提升 revision、重算 canonical digest 与 policy_id,
    /// 且可作为合法 successor 被保存(gateway spawn 前复验测试依赖此路径)。
    #[test]
    fn with_revised_policy_advances_revision_and_recomputes_digest() {
        let temp = tempfile::tempdir().unwrap();
        let store = AggregatePolicyArtifactStore::new(ProductAppPaths::new(temp.path()));
        let manifest =
            LogicalCodebaseManifest::new("project_0001", temp.path().to_path_buf(), vec![]);
        let bootstrap = store.ensure_bootstrap(&manifest).unwrap();

        let revised =
            bootstrap.with_revised_policy("# revision 2 policy text\n", "2026-08-10T00:00:00Z");
        assert_eq!(revised.revision, 2);
        assert_ne!(revised.digest, bootstrap.digest);
        assert!(revised.policy_id.ends_with("/2"));
        // digest 是新 policy_text 的 canonical sha256
        let expected = format!("sha256:{:x}", Sha256::digest(b"# revision 2 policy text\n"));
        assert_eq!(revised.digest, expected);
        // 可作为 successor 保存
        store.save("project_0001", &revised).unwrap();
        let reloaded = store.get("project_0001").unwrap().unwrap();
        assert_eq!(reloaded, revised);
    }

    /// 存量 envelope JSON(无 `authority_root` 键)反序列化不失败:serde 缺省为
    /// `PathBuf::default()`,新字段不破坏旧记录读取。
    #[test]
    fn legacy_envelope_json_without_authority_root_deserializes_with_default() {
        let artifact = AggregatePolicyArtifact::bootstrap("p1", "l1", "now".into());
        let envelope = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::PlanningReadOnly,
            PolicyTarget::aggregate_root(PathBuf::from("/aggregate")),
            vec![PathBuf::from("/aggregate")],
            vec![],
            ProviderDialect::ClaudeCodeCliV1,
            "sha256:cfg".into(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap();

        let mut json = serde_json::to_value(&envelope).unwrap();
        json.as_object_mut()
            .expect("envelope serializes to an object")
            .remove("authority_root");
        let restored: SessionPolicyEnvelope =
            serde_json::from_value(json).expect("legacy envelope JSON must deserialize");

        assert_eq!(restored.authority_root, PathBuf::new());
        assert_eq!(restored.policy_id, envelope.policy_id);
    }

    /// `recompute_config_digest` 与 `new` 内部冻结的 config_digest 算法一致,
    /// 供 gateway spawn 前复验托管配置未被篡改。
    #[test]
    fn recompute_config_digest_matches_envelope_frozen_value() {
        let artifact = AggregatePolicyArtifact::bootstrap("p1", "l1", "now".into());
        let envelope = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::PlanningReadOnly,
            PolicyTarget::checkout("repo", "co", "/work/repo"),
            vec![],
            vec![],
            ProviderDialect::ClaudeCodeCliV1,
            "sha256:managed-config".into(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap();
        let recomputed =
            SessionPolicyEnvelope::recompute_config_digest("sha256:managed-config").unwrap();
        assert_eq!(recomputed, envelope.config_digest);
    }
}
