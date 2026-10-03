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
use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id};
use crate::product::logical_codebase::aggregate_initialization::{
    AggregateInitializationOperation, AggregateInitializationOperationStatus,
    AggregateInitializationStepKind,
};
use crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
use crate::product::logical_codebase::root_recipe_receipt::ROOT_RULE_ENTRY_FILE;

#[cfg(test)]
use publish_faults::FaultPhase;

/// 路由级 fail-closed 安全策略:任何与 action 不匹配的 root 配置都返回此错误。
///
/// 注意:路由级 fail-closed 不等于 OS 级隔离。本 envelope 是 experimental +
/// supervised 场景下的政策门禁,不宣称物理不可写。
pub const POLICY_ENVELOPE_INVALID_ROOTS: &str = "policy_envelope_invalid_roots";

/// bootstrap 政策使用的最小政策正文。Task 9 的 gateway 在首次真实 provider
/// launch 前从此正文解析政策,后续 revision 可由更完整的政策正文替换。
const BOOTSTRAP_POLICY_TEXT: &str = "# Aggregate policy (bootstrap)\n\nAllow planning read-only and coding target-write sessions under the logical codebase.\n";

/// I2:已知自举桩正文的固定 digest(由当前桩原字节独立计算并以测试
/// 钉定,消费端不得复制常量)。Task 2 的 readiness 桩识别与
/// [`AggregatePolicyArtifact::is_bootstrap_placeholder`] 共用此常量。
pub(crate) const BOOTSTRAP_POLICY_DIGEST: &str =
    "sha256:7985b93678372d9dca0cf4489f454a6ec04fdc83f9c1c45497a7542119401373";

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

    /// I2:唯一桩识别谓词。仅比较完整固定正文或 store 已校验的固定
    /// digest,不比较 revision、年龄或关键词——任何"看起来像"的正文都
    /// 不算命中。readiness 消费在 Task 2,本包先保留。
    #[allow(dead_code)]
    pub(crate) fn is_bootstrap_placeholder(&self) -> bool {
        self.policy_text == BOOTSTRAP_POLICY_TEXT || self.digest == BOOTSTRAP_POLICY_DIGEST
    }
}

/// 发布输出记录的基础引用:候选发布所基于的当时 current artifact 快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AggregatePolicyPublicationReference {
    pub(crate) policy_id: String,
    pub(crate) revision: u64,
    pub(crate) digest: String,
}

/// 根政策单来源的相对路径与原字节 SHA-256(sources 不重复保存正文,
/// 完整正文只存在候选 artifact 中)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AggregatePolicySourceDigest {
    pub(crate) relative_path: String,
    pub(crate) digest: String,
}

/// operation-owned 不可变发布输出:冻结候选 artifact、基础引用、来源
/// 摘要与 rule digest。`lc_id` 是产品 record id,`logical_codebase_id`
/// 是 manifest UUID 字符串,两者不得混用;`artifact.created_at` 兼作
/// 稳定发布/finalize 时间,不另设同义时间字段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AggregatePolicyPublicationOutput {
    pub(crate) project_id: String,
    pub(crate) lc_id: String,
    pub(crate) logical_codebase_id: String,
    pub(crate) operation_id: String,
    pub(crate) canonical_root: PathBuf,
    pub(crate) base_policy: AggregatePolicyPublicationReference,
    pub(crate) artifact: AggregatePolicyArtifact,
    pub(crate) sources: Vec<AggregatePolicySourceDigest>,
    pub(crate) rule_digest: String,
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
    /// 会话 cwd（canonical LC root，Task 2.5 cwd/target 分离合同）。envelope
    /// 冻结它并纳入 resume fingerprint；与 `target.worktree`（成员
    /// checkout/worktree）语义不同，不得回退 member cwd。存量记录无此键，
    /// serde 缺省为 `PathBuf::default()`（与 `authority_root` 同策略）。
    #[serde(default)]
    pub working_directory: PathBuf,
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
        // 会话 cwd（canonical LC root）。与 target（成员 checkout）独立传递，
        // 由 envelope 冻结并纳入 resume fingerprint（Task 2.5）。
        working_directory: PathBuf,
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
            working_directory,
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
    /// 调用方传任意摘要;新 revision 的 digest 在写入前重新校验。与
    /// `ensure_bootstrap`/根政策发布共用同 scope 的 `.aggregate-policy.lock`,
    /// 三方写串行化,防止丢失更新。
    pub fn save(
        &self,
        project_id: &str,
        artifact: &AggregatePolicyArtifact,
    ) -> Result<(), ProductStoreError> {
        with_exact_exclusive_lock(&self.lock_path(project_id)?, || {
            self.save_unlocked(project_id, artifact)
        })
    }

    /// 锁内保存:调用方必须已持有 scope 锁(发布路径复用,绝不重入取锁)。
    fn save_unlocked(
        &self,
        project_id: &str,
        artifact: &AggregatePolicyArtifact,
    ) -> Result<(), ProductStoreError> {
        artifact.validate_identity(project_id)?;
        artifact.validate_digest()?;
        if let Some(existing) = self.get(project_id)? {
            existing.validate_successor(artifact)?;
        }
        write_artifact_durable(&self.artifact_path(project_id)?, artifact)
    }

    /// 确保存在 bootstrap artifact;幂等。相同 artifact 无副作用返回;
    /// 存在 project/logical-codebase 不一致的 artifact 时返回 `IdentityMismatch`,
    /// 不能覆盖。与 save/发布共用同 scope 的 `.aggregate-policy.lock`。
    pub fn ensure_bootstrap(
        &self,
        manifest: &LogicalCodebaseManifest,
    ) -> Result<AggregatePolicyArtifact, ProductStoreError> {
        validate_relative_id(&manifest.project_id)?;

        let logical_codebase_id = manifest.logical_codebase_id.to_string();
        let now = manifest.updated_at.clone();
        let bootstrap =
            AggregatePolicyArtifact::bootstrap(&manifest.project_id, &logical_codebase_id, now);

        with_exact_exclusive_lock(&self.lock_path(&manifest.project_id)?, || {
            if let Some(existing) = self.get(&manifest.project_id)? {
                existing.assert_matches_bootstrap(&bootstrap)?;
                return Ok(existing);
            }
            self.save_unlocked(&manifest.project_id, &bootstrap)?;
            Ok(bootstrap)
        })
    }

    fn artifact_path(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        Ok(
            crate::product::logical_codebase::lc_scope_root(&self.paths, project_id, &self.lc_id)?
                .join("aggregate-policy.json"),
        )
    }

    // -----------------------------------------------------------------
    // I1:operation-owned 根政策发布
    // -----------------------------------------------------------------

    fn scope_root(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        crate::product::logical_codebase::lc_scope_root(&self.paths, project_id, &self.lc_id)
    }

    /// save/ensure_bootstrap/发布三方共用的 scope 锁(同 scope 一把)。
    fn lock_path(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        Ok(self
            .scope_root(project_id)?
            .join(AGGREGATE_POLICY_LOCK_FILE))
    }

    /// 不可变发布输出路径:`<lc_scope_root>/aggregate-initializations/
    /// {operation_id}/policy-publication.json`——不放会被 cancel/recover
    /// 清理的 `staging/`。
    fn publication_output_path(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(operation_id)?;
        Ok(self
            .scope_root(project_id)?
            .join("aggregate-initializations")
            .join(operation_id)
            .join(POLICY_PUBLICATION_FILE))
    }

    /// 同 scope 的 operation store(发布前状态核验用)。
    fn operation_store(&self) -> AggregateInitializationOperationStore {
        match &self.lc_id {
            Some(lc_id) => {
                AggregateInitializationOperationStore::for_lc(self.paths.clone(), lc_id.clone())
            }
            None => AggregateInitializationOperationStore::new(self.paths.clone()),
        }
    }

    /// 当前 store 的 scope 标识(写入输出记录):显式 `for_lc` 用 record
    /// id,legacy 未 scoped store 用 legacy 别名 id。
    fn scoped_lc_id(&self, project_id: &str) -> String {
        self.lc_id
            .clone()
            .unwrap_or_else(|| legacy_logical_codebase_id(project_id))
    }

    /// I1:确定性构造并发布根政策正文。锁外先核验 operation(同 scope/
    /// project/root、Running 且 current_step=OpenspecAndExamples,不放宽
    /// 生产状态核验),再在 scope 锁内冻结候选 revision、来源摘要与发布
    /// 时间。发布顺序固定:不可变输出 → 精确 locator no-clobber 发布 →
    /// read-back 字节/SHA 复验 → current artifact durable 替换。同
    /// operation 重入复用同一候选与时间;来源/身份/base 漂移冲突停等。
    /// 生产接线在 Task 4(生产末命令收口),本包先保留。
    #[allow(dead_code)]
    pub(crate) fn publish_recipe_policy(
        &self,
        manifest: &LogicalCodebaseManifest,
        operation_id: &str,
        canonical_root: &Path,
        created_at: String,
    ) -> Result<AggregatePolicyArtifact, ProductStoreError> {
        validate_relative_id(&manifest.project_id)?;
        validate_relative_id(operation_id)?;
        let operation = self
            .operation_store()
            .get(&manifest.project_id, operation_id)?;
        validate_publication_operation(&operation, manifest, canonical_root)?;
        with_exact_exclusive_lock(&self.lock_path(&manifest.project_id)?, || {
            self.publish_locked(manifest, operation_id, canonical_root, created_at)
        })
    }

    /// I1:读取 operation-owned 发布输出(纯投影,零副作用)。生产接线
    /// 在 Task 4,本包先保留。
    #[allow(dead_code)]
    pub(crate) fn get_recipe_policy_publication(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<Option<AggregatePolicyPublicationOutput>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(operation_id)?;
        let path = self.publication_output_path(project_id, operation_id)?;
        if !path.try_exists().map_err(|error| {
            ProductStoreError::Io(format!("try_exists {}: {error}", path.display()))
        })? {
            return Ok(None);
        }
        let output: AggregatePolicyPublicationOutput = read_json(&path)?;
        if output.project_id != project_id || output.operation_id != operation_id {
            return Err(invalid_publication(format!(
                "publication output at {} does not belong to operation {operation_id}",
                path.display()
            )));
        }
        output.artifact.validate_digest()?;
        Ok(Some(output))
    }

    /// 锁内发布主流程(调用方已持 scope 锁)。
    fn publish_locked(
        &self,
        manifest: &LogicalCodebaseManifest,
        operation_id: &str,
        canonical_root: &Path,
        created_at: String,
    ) -> Result<AggregatePolicyArtifact, ProductStoreError> {
        let project_id = manifest.project_id.as_str();
        let logical_codebase_id = manifest.logical_codebase_id.to_string();
        let scope_lc_id = self.scoped_lc_id(project_id);
        let output_path = self.publication_output_path(project_id, operation_id)?;

        // 首次发布与重入都重新收集来源并复验清单/摘要——漂移冲突停等。
        let sources = collect_policy_sources(canonical_root)?;
        let source_digests: Vec<AggregatePolicySourceDigest> = sources
            .iter()
            .map(|source| AggregatePolicySourceDigest {
                relative_path: source.relative_path.clone(),
                digest: sha256_hex(&source.bytes),
            })
            .collect();
        // rule_digest 只取 AGENTS.md 原字节 SHA-256(与 root_rule_digest 同语义)。
        let rule_digest = sha256_hex(&sources[0].bytes);

        if output_path.exists() {
            // 同 operation 重入:复用不可变候选与时间,只补齐未完成发布。
            let output: AggregatePolicyPublicationOutput = read_json(&output_path)?;
            validate_publication_identity(
                &output,
                manifest,
                operation_id,
                canonical_root,
                &scope_lc_id,
            )?;
            if output.sources != source_digests {
                return Err(invalid_publication(format!(
                    "policy sources drifted since the frozen publication of operation {operation_id}"
                )));
            }
            if output.rule_digest != rule_digest {
                return Err(invalid_publication(format!(
                    "root rule digest drifted since the frozen publication of operation {operation_id}"
                )));
            }
            let candidate = output.artifact.clone();
            self.complete_locator_and_current(project_id, canonical_root, &candidate)?;
            return Ok(candidate);
        }

        // 新候选:base 引用当前 artifact(无则 revision 0);溢出先拒绝。
        let current = self.get(project_id)?;
        let (base_revision, base_policy_id, base_digest) = match &current {
            Some(existing) => (
                existing.revision,
                existing.policy_id.clone(),
                existing.digest.clone(),
            ),
            None => (0, String::new(), String::new()),
        };
        let revision = base_revision.checked_add(1).ok_or_else(|| {
            invalid_publication(format!("policy revision overflow from {base_revision}"))
        })?;
        let policy_text = build_policy_text(&sources)?;
        let artifact = AggregatePolicyArtifact {
            policy_id: format!("policy/{project_id}/{logical_codebase_id}/{revision}"),
            project_id: project_id.to_string(),
            logical_codebase_id,
            revision,
            digest: sha256_hex(policy_text.as_bytes()),
            policy_text,
            created_at,
        };
        let output = AggregatePolicyPublicationOutput {
            project_id: project_id.to_string(),
            lc_id: scope_lc_id,
            logical_codebase_id: artifact.logical_codebase_id.clone(),
            operation_id: operation_id.to_string(),
            canonical_root: canonical_root.to_path_buf(),
            base_policy: AggregatePolicyPublicationReference {
                policy_id: base_policy_id,
                revision: base_revision,
                digest: base_digest,
            },
            artifact: artifact.clone(),
            sources: source_digests,
            rule_digest,
        };
        // 顺序固定:先不可变输出(冻结候选与时间)。
        #[cfg(test)]
        if let Some(error) = publish_faults::trip(&output_path, FaultPhase::OutputWrite) {
            return Err(error);
        }
        let output_bytes = serde_json::to_vec_pretty(&output)
            .map_err(|error| ProductStoreError::Json(error.to_string()))?;
        publish_bytes_no_clobber(&output_path, &output_bytes, None)?;
        self.complete_locator_and_current(project_id, canonical_root, &artifact)?;
        Ok(artifact)
    }

    /// locator no-clobber 发布 → read-back 字节/SHA 复验 → current
    /// artifact durable 替换(复用既有 identity/successor 规则;同候选
    /// 幂等跳过)。
    fn complete_locator_and_current(
        &self,
        project_id: &str,
        canonical_root: &Path,
        artifact: &AggregatePolicyArtifact,
    ) -> Result<(), ProductStoreError> {
        let locator = canonical_root.join(&artifact.policy_id);
        #[cfg(test)]
        if let Some(error) = publish_faults::trip(&locator, FaultPhase::LocatorPublish) {
            return Err(error);
        }
        publish_bytes_no_clobber(
            &locator,
            artifact.policy_text.as_bytes(),
            Some(canonical_root),
        )?;
        let published = std::fs::read(&locator).map_err(|error| {
            ProductStoreError::Io(format!("read back {}: {error}", locator.display()))
        })?;
        if published != artifact.policy_text.as_bytes() || sha256_hex(&published) != artifact.digest
        {
            return Err(invalid_publication(format!(
                "published policy bytes at {} do not match the artifact digest",
                locator.display()
            )));
        }
        if let Some(existing) = self.get(project_id)?
            && existing == *artifact
        {
            return Ok(());
        }
        #[cfg(test)]
        if let Some(error) =
            publish_faults::trip(&self.artifact_path(project_id)?, FaultPhase::ArtifactSave)
        {
            return Err(error);
        }
        self.save_unlocked(project_id, artifact)
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

// 本文件按仓库惯例拆入 `.inc.rs`(同模块命名空间,符号路径与可见性不变),
// 保持每个文件低于 large_file_guard 的 1200 行上限(root_recipe_receipt.rs 同款先例):
// - policy_publication.inc.rs:I1 发布原语——来源收集/正文构造/durable 写入/身份复验。
// - policy_tests.inc.rs:I2/发布测试与 cfg(test) IO 故障注入 seam。

// 引入 manifest 类型以供 ensure_bootstrap 使用;此处只依赖其稳定 logical-codebase
// UUID、project_id 与 updated_at,与 store.rs 的 LogicalCodebaseManifest 同源。
use crate::product::logical_codebase::store::{
    LogicalCodebaseManifest, legacy_logical_codebase_id,
};

include!("policy_publication.inc.rs");
include!("policy_tests.inc.rs");
