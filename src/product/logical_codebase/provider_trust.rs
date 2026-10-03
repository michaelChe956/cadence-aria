//! REQ-REG-14：Codex/Kimi provider trust 硬前置门——gate 编排与稳定错误。
//!
//! trust 生命周期顺序固定为：LC 根准入（REQ-REG-09）冻结 canonical root
//! → 独立完成所选 Codex/Kimi trust 登记/核验/撤销与副作用审计 → 仅在
//! 全部 Ready 后允许 Claude Code 五步 recipe 创建/启动。登记不属于五步
//! operation、不插入步骤之间；任一失败进入可重试 fail-closed waiting，
//! 不降级、不部分放行。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::provider_gateway::ProviderGatewayError;
use crate::product::logical_codebase::provider_trust_adapters::{
    ProviderTrustEntryState, ProviderTrustHomeAdapter, ProviderTrustWriteOutcome,
};
use crate::product::logical_codebase::provider_trust_store::{
    ProviderTrustAuditRecord, ProviderTrustOwnershipRecord, ProviderTrustStore,
    ProviderTrustWaitingRecord, new_audit_id, trust_now,
};
use crate::product::models::ProviderName;

/// trust 前置门的稳定错误：code 永不本地化；`retryable` 标记自动重试是否
/// 可能改变结果（用户值冲突不会），`retry_action` 是等待面展示的产品动作。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ProviderTrustError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    pub retry_action: String,
}

impl ProviderTrustError {
    /// fail-closed 且可重试（home 写入/读取/并发等瞬态失败）。
    pub fn transient(code: &'static str, message: String) -> Self {
        Self {
            code,
            message,
            retryable: true,
            retry_action: "修复用户级 trust 配置的可写性/一致性后重试初始化".to_string(),
        }
    }

    /// 用户既有值冲突：不覆盖、不删除；等待用户处置后重试重新评估。
    pub fn conflict(code: &'static str, message: String) -> Self {
        Self {
            code,
            message,
            retryable: false,
            retry_action: "用户自行确认该 trust 键的取值后重试；系统不覆盖用户条目".to_string(),
        }
    }

    fn store(error: ProductStoreError) -> Self {
        Self::transient(
            "trust_store_io",
            format!("durable trust fact store failed: {error}"),
        )
    }
}

/// 单 provider trust 登记生命周期（trait 按 (project, lc, provider, root)
/// 调用点 scope；与计划文档相比仅前置了 `project_id` 路径参数，类型与
/// 字段名不变——durable 布局需要 project scope 才能定位 LC 子树）。
pub trait ProviderTrustRegistry: Send + Sync {
    fn ensure_trusted(
        &self,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustRegistration, ProviderTrustError>;

    fn verify_trusted(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustVerification, ProviderTrustError>;

    fn revoke_trusted(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustRevocation, ProviderTrustError>;
}

/// Task 3b(REQ-LCG-02/03):只读 trust source——GET/early eligibility 与
/// spawn 前复验共用的零副作用通道。
///
/// 与 [`ProviderTrustRegistry::verify_trusted`] 的区别:后者会向 durable
/// `ProviderTrustStore` 追加 verify audit 记录(副作用),不得用于
/// GET/early 或 spawn 复验;本 trait 的实现只允许经
/// [`ProviderTrustHomeAdapter::read_state`]/[`ProviderTrustHomeAdapter::digest`]
/// 读取用户级工件事实,禁止 ensure/revoke、禁止写 audit/ownership。
pub trait ProviderTrustSource: Send + Sync {
    fn verify_trusted(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustVerification, ProviderGatewayError>;
}

/// 只读 home 背书实现:factory 生产装配(只读 home adapter + 只读
/// ownership 查询,绝不写入)。
pub struct ReadonlyProviderTrustSource {
    paths: ProductAppPaths,
    lc_id: Option<String>,
    adapters: Vec<Arc<dyn ProviderTrustHomeAdapter>>,
}

impl ReadonlyProviderTrustSource {
    pub fn new(
        paths: ProductAppPaths,
        lc_id: Option<String>,
        adapters: Vec<Arc<dyn ProviderTrustHomeAdapter>>,
    ) -> Self {
        Self {
            paths,
            lc_id,
            adapters,
        }
    }

    /// 按 LC 作用域装配;lc 作用域未知(legacy 无别名)时 ownership 查询
    /// 跳过(只读事实缺失不构成伪造)。
    pub fn for_lc(paths: ProductAppPaths, lc_id: &str) -> Self {
        Self::new(paths, Some(lc_id.to_string()), Self::production_adapters())
    }

    /// factory 生产装配:按 LC 作用域(可空,legacy 无别名)注入生产
    /// trust adapters(Codex/Kimi——需要 workspace trust 的两家;Claude/Pi
    /// 无用户级 trust 工件,不进入本 source)。
    pub fn production_for_scope(paths: ProductAppPaths, lc_id: Option<String>) -> Self {
        Self::new(paths, lc_id, Self::production_adapters())
    }

    fn production_adapters() -> Vec<Arc<dyn ProviderTrustHomeAdapter>> {
        // production() 在 HOME 缺失(无用户级 home 可读)时返回 None——
        // 该 provider 无只读 trust 工件可查,不进入本 source。
        let codex =
            crate::product::logical_codebase::provider_trust_adapters::CodexTrustAdapter::production()
                .map(|adapter| Arc::new(adapter) as Arc<dyn ProviderTrustHomeAdapter>);
        let kimi =
            crate::product::logical_codebase::provider_trust_adapters::KimiTrustAdapter::production()
                .map(|adapter| Arc::new(adapter) as Arc<dyn ProviderTrustHomeAdapter>);
        codex.into_iter().chain(kimi).collect()
    }

    fn adapter_for(
        &self,
        provider: &ProviderName,
    ) -> Result<Arc<dyn ProviderTrustHomeAdapter>, ProviderGatewayError> {
        self.adapters
            .iter()
            .find(|adapter| adapter.provider() == *provider)
            .cloned()
            .ok_or_else(|| {
                ProviderGatewayError::ProviderUnavailable(format!(
                    "no read-only trust adapter is configured for {provider:?}"
                ))
            })
    }
}

impl ProviderTrustSource for ReadonlyProviderTrustSource {
    fn verify_trusted(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustVerification, ProviderGatewayError> {
        let trust_error = |error: ProviderTrustError| {
            ProviderGatewayError::ProviderUnavailable(format!("{}: {}", error.code, error.message))
        };
        let adapter = self.adapter_for(provider)?;
        if !canonical_root.is_absolute() || canonical_root.file_name().is_none() {
            return Err(ProviderGatewayError::ProviderUnavailable(format!(
                "invalid canonical root for read-only trust check: {}",
                canonical_root.display()
            )));
        }
        let trust_key = adapter.trust_key(canonical_root);
        let state = adapter.read_state(canonical_root).map_err(trust_error)?;
        let trusted = matches!(state, ProviderTrustEntryState::Trusted);
        let detail = match state {
            ProviderTrustEntryState::Trusted => "entry present and trusted".to_string(),
            ProviderTrustEntryState::Untrusted { .. } => {
                "key holds a non-trusted value".to_string()
            }
            ProviderTrustEntryState::Missing => "entry absent".to_string(),
        };
        let ownership = match &self.lc_id {
            Some(scope) => {
                let store = ProviderTrustStore::for_lc(self.paths.clone(), scope);
                store
                    .load_ownership(project_id, provider_wire_label(provider), &trust_key)
                    .map_err(|error| {
                        ProviderGatewayError::ProviderUnavailable(format!(
                            "read trust ownership: {error}"
                        ))
                    })?
                    .map(|record| {
                        if record.owned {
                            ProviderTrustOwnership::LcManaged
                        } else {
                            ProviderTrustOwnership::UserOwned
                        }
                    })
            }
            None => None,
        };
        // T3R2-F2:形参 lc_id 与 source 装配作用域必须一致——调用侧携带
        // 的 LC 作用域(空串=无作用域)若与装配不一致,说明装配漂移,
        // fail-closed(本 change 语义:不静默采信任一侧)。
        let requested_scope = if lc_id.is_empty() { None } else { Some(lc_id) };
        if requested_scope != self.lc_id.as_deref() {
            return Err(ProviderGatewayError::ProviderUnavailable(format!(
                "read-only trust source scope mismatch: requested {requested_scope:?}, source {:?}",
                self.lc_id
            )));
        }
        Ok(ProviderTrustVerification {
            provider: provider.clone(),
            canonical_root: canonical_root.to_path_buf(),
            trust_key,
            trusted,
            ownership,
            detail,
            verified_at: trust_now(),
        })
    }
}

/// 五步 recipe 的 trust 硬前置门：全部所选 trust Ready 才放行。
pub trait ProviderTrustPrecondition: Send + Sync {
    /// `providers` 中需要 workspace trust 的项（Codex/KimiCode）逐一登记；
    /// 任一失败返回可重试 Waiting（durable waiting fact + 审计），五步
    /// operation 不得创建/启动。全部 Ready（或无需 trust）才返回 Ready。
    fn ensure_before_recipe(
        &self,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        canonical_root: &Path,
        providers: &[ProviderName],
    ) -> ProviderTrustPreparationResult;
}

/// What one ensure pass did to the user-level trust artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderTrustAction {
    /// Wrote the entry (first registration for this root).
    Register,
    /// Entry already trusted and owned by this LC; replay wrote nothing.
    Replay,
    /// Entry already trusted but NOT owned by this LC; reused untouched.
    ReuseUserEntry,
}

/// Ownership proof attached to every registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderTrustOwnership {
    /// This LC created the entry and may revoke it (digest permitting).
    LcManaged,
    /// The user (or another tool) created the entry; never written/revoked.
    UserOwned,
}

/// Terminal state of one provider's registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderTrustRegistrationResult {
    Ready,
    Waiting { reason_code: String },
}

/// One provider's ensure result（计划契约字段：
/// provider/root/key/action/before_digest/after_digest/ownership/result）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTrustRegistration {
    pub provider: ProviderName,
    pub canonical_root: PathBuf,
    pub trust_key: String,
    pub action: ProviderTrustAction,
    pub before_digest: Option<String>,
    pub after_digest: Option<String>,
    pub ownership: ProviderTrustOwnership,
    pub result: ProviderTrustRegistrationResult,
    pub registered_at: String,
}

/// One provider's verify result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTrustVerification {
    pub provider: ProviderName,
    pub canonical_root: PathBuf,
    pub trust_key: String,
    pub trusted: bool,
    pub ownership: Option<ProviderTrustOwnership>,
    pub detail: String,
    pub verified_at: String,
}

/// One provider's revoke result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderTrustRevocationOutcome {
    /// LC-owned entry removed after digest verification.
    Removed,
    /// User-owned (or not owned by this LC); left untouched.
    NotOwned,
    /// Nothing to remove.
    EntryMissing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTrustRevocation {
    pub provider: ProviderName,
    pub canonical_root: PathBuf,
    pub trust_key: String,
    pub outcome: ProviderTrustRevocationOutcome,
    pub before_digest: Option<String>,
    pub after_digest: Option<String>,
    pub revoked_at: String,
}

/// Gate verdict for the five-step recipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderTrustPreparationResult {
    Ready {
        registrations: Vec<ProviderTrustRegistration>,
    },
    Waiting {
        waiting: ProviderTrustWaiting,
    },
}

/// Retryable fail-closed waiting surface (stable code + product action).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTrustWaiting {
    pub provider: ProviderName,
    pub canonical_root: PathBuf,
    pub trust_key: String,
    pub reason_code: String,
    pub message: String,
    pub retry_action: String,
    pub recorded_at: String,
}

/// Which providers need user-level workspace trust before an LC session.
pub fn requires_workspace_trust(provider: &ProviderName) -> bool {
    matches!(provider, ProviderName::Codex | ProviderName::KimiCode)
}

/// Wire label persisted into durable trust facts（snake_case，与
/// `ProviderName` serde 序列化一致）。
pub(crate) fn provider_wire_label(provider: &ProviderName) -> &'static str {
    match provider {
        ProviderName::ClaudeCode => "claude_code",
        ProviderName::Codex => "codex",
        ProviderName::Pi => "pi",
        ProviderName::KimiCode => "kimi_code",
        ProviderName::Fake => "fake",
    }
}

/// Home-file backed registry：home 副作用全部经注入的 adapter 发生，
/// durable facts 落在 per-LC scope 的 `ProviderTrustStore`。
pub struct HomeBackedProviderTrustRegistry {
    paths: ProductAppPaths,
    adapters: Vec<Arc<dyn ProviderTrustHomeAdapter>>,
}

impl HomeBackedProviderTrustRegistry {
    pub fn new(paths: ProductAppPaths, adapters: Vec<Arc<dyn ProviderTrustHomeAdapter>>) -> Self {
        Self { paths, adapters }
    }

    fn adapter_for(
        &self,
        provider: &ProviderName,
    ) -> Result<Arc<dyn ProviderTrustHomeAdapter>, ProviderTrustError> {
        self.adapters
            .iter()
            .find(|adapter| adapter.provider() == *provider)
            .cloned()
            .ok_or_else(|| {
                ProviderTrustError::conflict(
                    "unsupported_trust_provider",
                    format!("no trust adapter is configured for {provider:?}"),
                )
            })
    }

    fn validate_root(canonical_root: &Path) -> Result<(), ProviderTrustError> {
        if !canonical_root.is_absolute() || canonical_root.file_name().is_none() {
            return Err(ProviderTrustError::transient(
                "invalid_canonical_root",
                format!(
                    "canonical root must be an absolute path with a file name, got {}",
                    canonical_root.display()
                ),
            ));
        }
        Ok(())
    }

    fn store_for(&self, lc_id: &str) -> ProviderTrustStore {
        ProviderTrustStore::for_lc(self.paths.clone(), lc_id)
    }

    #[allow(clippy::too_many_arguments)]
    fn audit(
        &self,
        store: &ProviderTrustStore,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
        trust_key: &str,
        action: &str,
        outcome: &str,
        before_digest: Option<String>,
        after_digest: Option<String>,
    ) -> Result<(), ProviderTrustError> {
        store
            .append_audit(
                project_id,
                ProviderTrustAuditRecord {
                    audit_id: new_audit_id(),
                    lc_id: lc_id.to_string(),
                    operation_id: non_empty(operation_id),
                    provider: provider_wire_label(provider).to_string(),
                    canonical_root: canonical_root.to_path_buf(),
                    trust_key: trust_key.to_string(),
                    action: action.to_string(),
                    outcome: outcome.to_string(),
                    before_digest,
                    after_digest,
                    recorded_at: trust_now(),
                },
            )
            .map_err(ProviderTrustError::store)
    }

    /// Entry already trusted: refresh ownership/digest facts without writing
    /// the user home file.
    #[allow(clippy::too_many_arguments)]
    fn reuse_trusted_entry(
        &self,
        store: &ProviderTrustStore,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
        trust_key: &str,
        digest: Option<String>,
    ) -> Result<ProviderTrustRegistration, ProviderTrustError> {
        let now = trust_now();
        let provider_label = provider_wire_label(provider);
        let existing = store
            .load_ownership(project_id, provider_label, trust_key)
            .map_err(ProviderTrustError::store)?;
        let lc_managed = existing.as_ref().is_some_and(|record| record.owned);
        store
            .record_ownership(
                project_id,
                ProviderTrustOwnershipRecord {
                    lc_id: lc_id.to_string(),
                    provider: provider_label.to_string(),
                    canonical_root: canonical_root.to_path_buf(),
                    trust_key: trust_key.to_string(),
                    owned: lc_managed,
                    before_digest: digest.clone(),
                    after_digest: digest.clone(),
                    created_at: existing
                        .as_ref()
                        .map(|record| record.created_at.clone())
                        .unwrap_or_else(|| now.clone()),
                    updated_at: now.clone(),
                },
            )
            .map_err(ProviderTrustError::store)?;
        Ok(ProviderTrustRegistration {
            provider: provider.clone(),
            canonical_root: canonical_root.to_path_buf(),
            trust_key: trust_key.to_string(),
            action: if lc_managed {
                ProviderTrustAction::Replay
            } else {
                ProviderTrustAction::ReuseUserEntry
            },
            before_digest: digest.clone(),
            after_digest: digest,
            ownership: if lc_managed {
                ProviderTrustOwnership::LcManaged
            } else {
                ProviderTrustOwnership::UserOwned
            },
            result: ProviderTrustRegistrationResult::Ready,
            registered_at: now,
        })
    }
}

impl ProviderTrustRegistry for HomeBackedProviderTrustRegistry {
    fn ensure_trusted(
        &self,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustRegistration, ProviderTrustError> {
        let adapter = self.adapter_for(provider)?;
        Self::validate_root(canonical_root)?;
        let store = self.store_for(lc_id);
        let provider_label = provider_wire_label(provider);
        let trust_key = adapter.trust_key(canonical_root);

        let before_digest = adapter.digest(canonical_root)?;
        let state = adapter.read_state(canonical_root)?;
        let (registration, audit_outcome) = match state {
            ProviderTrustEntryState::Trusted => {
                let registration = self.reuse_trusted_entry(
                    &store,
                    project_id,
                    lc_id,
                    provider,
                    canonical_root,
                    &trust_key,
                    before_digest.clone(),
                )?;
                let outcome = if registration.action == ProviderTrustAction::Replay {
                    "replay"
                } else {
                    "reuse_user_entry"
                };
                (registration, outcome)
            }
            ProviderTrustEntryState::Missing => {
                match adapter.write_trusted(canonical_root, before_digest.as_deref())? {
                    ProviderTrustWriteOutcome::AlreadyTrusted => {
                        // Another writer registered the root between the
                        // state read and the write: re-digest and treat as
                        // trusted reuse.
                        let registration = self.reuse_trusted_entry(
                            &store,
                            project_id,
                            lc_id,
                            provider,
                            canonical_root,
                            &trust_key,
                            adapter.digest(canonical_root)?,
                        )?;
                        (registration, "replay")
                    }
                    ProviderTrustWriteOutcome::Created {
                        before_digest: _,
                        after_digest,
                    } => {
                        let now = trust_now();
                        store
                            .record_ownership(
                                project_id,
                                ProviderTrustOwnershipRecord {
                                    lc_id: lc_id.to_string(),
                                    provider: provider_label.to_string(),
                                    canonical_root: canonical_root.to_path_buf(),
                                    trust_key: trust_key.clone(),
                                    owned: true,
                                    before_digest: before_digest.clone(),
                                    after_digest: Some(after_digest.clone()),
                                    created_at: now.clone(),
                                    updated_at: now.clone(),
                                },
                            )
                            .map_err(ProviderTrustError::store)?;
                        (
                            ProviderTrustRegistration {
                                provider: provider.clone(),
                                canonical_root: canonical_root.to_path_buf(),
                                trust_key: trust_key.clone(),
                                action: ProviderTrustAction::Register,
                                before_digest: before_digest.clone(),
                                after_digest: Some(after_digest),
                                ownership: ProviderTrustOwnership::LcManaged,
                                result: ProviderTrustRegistrationResult::Ready,
                                registered_at: now,
                            },
                            "registered",
                        )
                    }
                }
            }
            ProviderTrustEntryState::Untrusted { current_value } => {
                self.audit(
                    &store,
                    project_id,
                    operation_id,
                    lc_id,
                    provider,
                    canonical_root,
                    &trust_key,
                    "ensure",
                    "conflict",
                    before_digest.clone(),
                    before_digest.clone(),
                )?;
                return Err(ProviderTrustError::conflict(
                    adapter.conflict_code(),
                    format!(
                        "trust key {} for {} holds a non-trusted value {:?}; \
                         拒绝覆盖用户既有条目",
                        trust_key,
                        canonical_root.display(),
                        current_value
                    ),
                ));
            }
        };

        store
            .clear_waiting(project_id, provider_label, &trust_key)
            .map_err(ProviderTrustError::store)?;
        self.audit(
            &store,
            project_id,
            operation_id,
            lc_id,
            provider,
            canonical_root,
            &trust_key,
            "ensure",
            audit_outcome,
            registration.before_digest.clone(),
            registration.after_digest.clone(),
        )?;
        Ok(registration)
    }

    fn verify_trusted(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustVerification, ProviderTrustError> {
        let adapter = self.adapter_for(provider)?;
        Self::validate_root(canonical_root)?;
        let store = self.store_for(lc_id);
        let provider_label = provider_wire_label(provider);
        let trust_key = adapter.trust_key(canonical_root);
        let before_digest = adapter.digest(canonical_root)?;
        let state = adapter.read_state(canonical_root)?;
        let trusted = matches!(state, ProviderTrustEntryState::Trusted);
        let (detail, outcome) = match state {
            ProviderTrustEntryState::Trusted => ("entry present and trusted", "trusted"),
            ProviderTrustEntryState::Untrusted { .. } => {
                ("key holds a non-trusted value", "untrusted")
            }
            ProviderTrustEntryState::Missing => ("entry absent", "missing"),
        };
        let ownership = store
            .load_ownership(project_id, provider_label, &trust_key)
            .map_err(ProviderTrustError::store)?
            .map(|record| {
                if record.owned {
                    ProviderTrustOwnership::LcManaged
                } else {
                    ProviderTrustOwnership::UserOwned
                }
            });
        self.audit(
            &store,
            project_id,
            "",
            lc_id,
            provider,
            canonical_root,
            &trust_key,
            "verify",
            outcome,
            before_digest.clone(),
            before_digest.clone(),
        )?;
        Ok(ProviderTrustVerification {
            provider: provider.clone(),
            canonical_root: canonical_root.to_path_buf(),
            trust_key,
            trusted,
            ownership,
            detail: detail.to_string(),
            verified_at: trust_now(),
        })
    }

    fn revoke_trusted(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustRevocation, ProviderTrustError> {
        let adapter = self.adapter_for(provider)?;
        Self::validate_root(canonical_root)?;
        let store = self.store_for(lc_id);
        let provider_label = provider_wire_label(provider);
        let trust_key = adapter.trust_key(canonical_root);
        let before_digest = adapter.digest(canonical_root)?;

        let ownership = store
            .load_ownership(project_id, provider_label, &trust_key)
            .map_err(ProviderTrustError::store)?;
        let Some(ownership) = ownership.filter(|record| record.owned) else {
            // 非本 LC 管理：用户条目原样保留，绝不删除。
            let state = adapter.read_state(canonical_root)?;
            let outcome = match state {
                ProviderTrustEntryState::Missing => ProviderTrustRevocationOutcome::EntryMissing,
                _ => ProviderTrustRevocationOutcome::NotOwned,
            };
            let outcome_label = match outcome {
                ProviderTrustRevocationOutcome::EntryMissing => "entry_missing",
                _ => "not_owned",
            };
            self.audit(
                &store,
                project_id,
                "",
                lc_id,
                provider,
                canonical_root,
                &trust_key,
                "revoke",
                outcome_label,
                before_digest.clone(),
                before_digest.clone(),
            )?;
            return Ok(ProviderTrustRevocation {
                provider: provider.clone(),
                canonical_root: canonical_root.to_path_buf(),
                trust_key,
                outcome,
                before_digest: before_digest.clone(),
                after_digest: before_digest,
                revoked_at: trust_now(),
            });
        };

        // 归属证明存在：撤销前核对摘要，被外部修改即 fail-closed 等待核验。
        if before_digest != ownership.after_digest {
            self.audit(
                &store,
                project_id,
                "",
                lc_id,
                provider,
                canonical_root,
                &trust_key,
                "revoke",
                "digest_mismatch",
                before_digest.clone(),
                ownership.after_digest.clone(),
            )?;
            return Err(ProviderTrustError::transient(
                "revoke_digest_mismatch",
                format!(
                    "trust artifact for {} changed after registration \
                     (recorded {:?}, now {:?}); 保持 fail-closed 等待人工核验",
                    canonical_root.display(),
                    ownership.after_digest,
                    before_digest
                ),
            ));
        }

        adapter.remove_trusted(canonical_root, before_digest.as_deref())?;
        let after_digest = adapter.digest(canonical_root)?;
        store
            .delete_ownership(project_id, provider_label, &trust_key)
            .map_err(ProviderTrustError::store)?;
        self.audit(
            &store,
            project_id,
            "",
            lc_id,
            provider,
            canonical_root,
            &trust_key,
            "revoke",
            "removed",
            before_digest.clone(),
            after_digest.clone(),
        )?;
        Ok(ProviderTrustRevocation {
            provider: provider.clone(),
            canonical_root: canonical_root.to_path_buf(),
            trust_key,
            outcome: ProviderTrustRevocationOutcome::Removed,
            before_digest,
            after_digest,
            revoked_at: trust_now(),
        })
    }
}

impl ProviderTrustPrecondition for HomeBackedProviderTrustRegistry {
    fn ensure_before_recipe(
        &self,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        canonical_root: &Path,
        providers: &[ProviderName],
    ) -> ProviderTrustPreparationResult {
        let mut selected: Vec<ProviderName> = Vec::new();
        for provider in providers {
            if requires_workspace_trust(provider) && !selected.contains(provider) {
                selected.push(provider.clone());
            }
        }
        let mut registrations = Vec::new();
        for provider in selected {
            match self.ensure_trusted(project_id, operation_id, lc_id, &provider, canonical_root) {
                Ok(registration) => registrations.push(registration),
                Err(error) => {
                    let provider_label = provider_wire_label(&provider);
                    let trust_key = self
                        .adapter_for(&provider)
                        .map(|adapter| adapter.trust_key(canonical_root))
                        .unwrap_or_default();
                    let recorded_at = trust_now();
                    // durable waiting fact + 审计：失败也必须可关联、可重试。
                    // 落盘失败不吞掉 gate 判定——仍返回 Waiting（fail-closed）。
                    let store = self.store_for(lc_id);
                    let _ = store.record_waiting(
                        project_id,
                        ProviderTrustWaitingRecord {
                            lc_id: lc_id.to_string(),
                            provider: provider_label.to_string(),
                            canonical_root: canonical_root.to_path_buf(),
                            trust_key: trust_key.clone(),
                            operation_id: non_empty(operation_id),
                            reason_code: error.code.to_string(),
                            message: error.message.clone(),
                            retry_action: error.retry_action.clone(),
                            recorded_at: recorded_at.clone(),
                        },
                    );
                    let _ = self.audit(
                        &store,
                        project_id,
                        operation_id,
                        lc_id,
                        &provider,
                        canonical_root,
                        &trust_key,
                        "gate_waiting",
                        error.code,
                        None,
                        None,
                    );
                    return ProviderTrustPreparationResult::Waiting {
                        waiting: ProviderTrustWaiting {
                            provider,
                            canonical_root: canonical_root.to_path_buf(),
                            trust_key,
                            reason_code: error.code.to_string(),
                            message: error.message,
                            retry_action: error.retry_action,
                            recorded_at,
                        },
                    };
                }
            }
        }
        ProviderTrustPreparationResult::Ready { registrations }
    }
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}
#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::provider_trust_adapters::{
        CodexTrustAdapter, KimiTrustAdapter, ProviderTrustHomeAdapter,
    };
    use crate::product::logical_codebase::provider_trust_store::ProviderTrustStore;
    use crate::product::models::ProviderName;

    use super::{
        HomeBackedProviderTrustRegistry, ProviderTrustAction, ProviderTrustPrecondition,
        ProviderTrustPreparationResult, ProviderTrustRegistry, ProviderTrustRevocationOutcome,
    };

    const PROJECT: &str = "project_0001";
    const LC: &str = "logical_codebase_0001";
    const OPERATION: &str = "aggregate_initialization_0001";

    struct TrustFixture {
        _home: tempfile::TempDir,
        aria: tempfile::TempDir,
        registry: HomeBackedProviderTrustRegistry,
        root: PathBuf,
    }

    impl TrustFixture {
        fn new() -> Self {
            let home = tempfile::TempDir::new().unwrap();
            let aria = tempfile::TempDir::new().unwrap();
            let root = home.path().join("lc-root");
            std::fs::create_dir_all(&root).unwrap();
            let registry = HomeBackedProviderTrustRegistry::new(
                ProductAppPaths::new(aria.path().to_path_buf()),
                vec![
                    Arc::new(CodexTrustAdapter::for_home(home.path())),
                    Arc::new(KimiTrustAdapter::for_home(home.path())),
                ],
            );
            Self {
                _home: home,
                aria,
                registry,
                root,
            }
        }

        fn home(&self) -> &Path {
            self._home.path()
        }

        fn codex_config(&self) -> PathBuf {
            self.home().join(".codex").join("config.toml")
        }

        fn kimi_trust_dir(&self) -> PathBuf {
            self.home().join(".kimi-code").join("workspace-trust")
        }

        fn kimi_key(&self) -> String {
            KimiTrustAdapter::for_home(self.home()).trust_key(&self.root)
        }

        fn store(&self) -> ProviderTrustStore {
            ProviderTrustStore::for_lc(ProductAppPaths::new(self.aria.path().to_path_buf()), LC)
        }
    }

    #[test]
    fn provider_trust_ensure_registers_codex_and_kimi_entries_idempotently() {
        let fixture = TrustFixture::new();
        let providers = [ProviderName::Codex, ProviderName::KimiCode];

        let first = fixture.registry.ensure_before_recipe(
            PROJECT,
            OPERATION,
            LC,
            &fixture.root,
            &providers,
        );
        let ProviderTrustPreparationResult::Ready { registrations } = first else {
            panic!("expected the trust gate to become ready");
        };
        assert_eq!(registrations.len(), 2);

        // Codex：用户级 config.toml 出现 [projects."<root>"] trust_level="trusted"。
        let config = std::fs::read_to_string(fixture.codex_config()).unwrap();
        let header = format!("[projects.\"{}\"]", fixture.root.display());
        assert!(config.contains(&header));
        assert!(config.contains("trust_level = \"trusted\""));

        // Kimi：workspace-trust 出现 wd_<basename>_<sha256[:12]> 记录。
        let kimi_key = fixture.kimi_key();
        assert_eq!(
            kimi_key,
            format!(
                "wd_lc-root_{}",
                sha256_hex_12(&fixture.root.display().to_string())
            )
        );
        let kimi_path = fixture.kimi_trust_dir().join(&kimi_key);
        let kimi_raw = std::fs::read_to_string(&kimi_path).unwrap();
        let kimi: serde_json::Value = serde_json::from_str(&kimi_raw).unwrap();
        assert_eq!(kimi["root"], fixture.root.display().to_string());
        assert!(kimi["trustedAt"].as_u64().unwrap() > 0);

        // 重复 ensure 幂等：home 文件字节不变、块不重复、无重写。
        let second = fixture.registry.ensure_before_recipe(
            PROJECT,
            OPERATION,
            LC,
            &fixture.root,
            &providers,
        );
        let ProviderTrustPreparationResult::Ready { registrations } = second else {
            panic!("expected the replayed trust gate to stay ready");
        };
        assert_eq!(
            std::fs::read_to_string(fixture.codex_config()).unwrap(),
            config
        );
        assert_eq!(std::fs::read_to_string(&kimi_path).unwrap(), kimi_raw);
        assert_eq!(config.matches(&header).count(), 1);
        assert!(registrations.iter().all(|registration| {
            matches!(
                registration.action,
                ProviderTrustAction::Replay | ProviderTrustAction::ReuseUserEntry
            )
        }));

        // 核验面保持 trusted。
        for provider in providers {
            let verification = fixture
                .registry
                .verify_trusted(PROJECT, LC, &provider, &fixture.root)
                .unwrap();
            assert!(
                verification.trusted,
                "expected {provider:?} to stay trusted"
            );
        }
    }

    #[test]
    fn provider_trust_failure_enters_retryable_waiting_without_fallback() {
        let fixture = TrustFixture::new();
        // 写入失败注入：workspace-trust 路径被普通文件占用（与权限无关、
        // 任何环境都稳定失败）。
        std::fs::create_dir_all(fixture.kimi_trust_dir().parent().unwrap()).unwrap();
        std::fs::write(fixture.kimi_trust_dir(), b"not-a-directory").unwrap();

        let providers = [ProviderName::Codex, ProviderName::KimiCode];
        let outcome = fixture.registry.ensure_before_recipe(
            PROJECT,
            OPERATION,
            LC,
            &fixture.root,
            &providers,
        );
        let ProviderTrustPreparationResult::Waiting { waiting } = outcome else {
            panic!("expected the trust gate to enter fail-closed waiting");
        };
        assert_eq!(waiting.provider, ProviderName::KimiCode);
        assert!(!waiting.reason_code.is_empty());
        assert!(waiting.reason_code.starts_with("kimi_"));
        assert!(!waiting.retry_action.is_empty());
        assert_eq!(waiting.canonical_root, fixture.root);

        // durable waiting fact 已落盘且可由 store 关联。
        let waiting_facts = fixture.store().list_waiting(PROJECT).unwrap();
        assert!(waiting_facts.iter().any(|fact| {
            fact.reason_code == waiting.reason_code
                && fact.trust_key == waiting.trust_key
                && fact.lc_id == LC
                && fact.operation_id.as_deref() == Some(OPERATION)
        }));

        // 不降级：Codex 已先成功登记，gate 仍不是 Ready、不部分放行。
        assert!(
            std::fs::read_to_string(fixture.codex_config())
                .unwrap()
                .contains("trust_level = \"trusted\"")
        );

        // 可重试：移除阻断后重试转 Ready，并清空 waiting fact。
        std::fs::remove_file(fixture.kimi_trust_dir()).unwrap();
        let retried = fixture.registry.ensure_before_recipe(
            PROJECT,
            OPERATION,
            LC,
            &fixture.root,
            &providers,
        );
        assert!(matches!(
            retried,
            ProviderTrustPreparationResult::Ready { .. }
        ));
        assert!(fixture.store().list_waiting(PROJECT).unwrap().is_empty());
    }

    #[test]
    fn provider_trust_revoke_removes_entries_and_preserves_user_trust() {
        let fixture = TrustFixture::new();
        // 用户既有 trust：另一 root 的 Codex 条目 + 其余 TOML 内容 + 另一 Kimi 记录。
        let user_root = fixture.home().join("user-project");
        std::fs::create_dir_all(&user_root).unwrap();
        std::fs::create_dir_all(fixture.codex_config().parent().unwrap()).unwrap();
        std::fs::write(
            fixture.codex_config(),
            format!(
                "[projects.\"{}\"]\ntrust_level = \"trusted\"\n\n[mcp_servers.spike]\ncommand = \"npx\"\n",
                user_root.display()
            ),
        )
        .unwrap();
        let user_kimi_key = KimiTrustAdapter::for_home(fixture.home()).trust_key(&user_root);
        std::fs::create_dir_all(fixture.kimi_trust_dir()).unwrap();
        std::fs::write(
            fixture.kimi_trust_dir().join(&user_kimi_key),
            format!("{{\"root\":\"{}\",\"trustedAt\":111}}", user_root.display()),
        )
        .unwrap();
        let user_kimi_before =
            std::fs::read_to_string(fixture.kimi_trust_dir().join(&user_kimi_key)).unwrap();

        let providers = [ProviderName::Codex, ProviderName::KimiCode];
        assert!(matches!(
            fixture.registry.ensure_before_recipe(
                PROJECT,
                OPERATION,
                LC,
                &fixture.root,
                &providers
            ),
            ProviderTrustPreparationResult::Ready { .. }
        ));
        for provider in providers {
            let revocation = fixture
                .registry
                .revoke_trusted(PROJECT, LC, &provider, &fixture.root)
                .unwrap();
            assert_eq!(revocation.outcome, ProviderTrustRevocationOutcome::Removed);
        }

        // 本 LC 根条目被撤销，用户条目逐字节保留。
        let codex_after = std::fs::read_to_string(fixture.codex_config()).unwrap();
        assert!(!codex_after.contains(&format!("[projects.\"{}\"]", fixture.root.display())));
        assert!(!fixture.kimi_trust_dir().join(fixture.kimi_key()).exists());
        assert!(codex_after.contains(&format!("[projects.\"{}\"]", user_root.display())));
        assert_eq!(codex_after.matches("[mcp_servers.spike]").count(), 1);
        assert!(codex_after.contains("command = \"npx\""));
        assert_eq!(
            std::fs::read_to_string(fixture.kimi_trust_dir().join(&user_kimi_key)).unwrap(),
            user_kimi_before
        );

        // 用户自建的本根条目：登记时原样复用、撤销时 NotOwned 且文件不动。
        std::fs::write(
            fixture.codex_config(),
            format!(
                "[projects.\"{}\"]\ntrust_level = \"trusted\"\n",
                fixture.root.display()
            ),
        )
        .unwrap();
        let user_pre_trusted = std::fs::read_to_string(fixture.codex_config()).unwrap();
        assert!(matches!(
            fixture.registry.ensure_before_recipe(
                PROJECT,
                OPERATION,
                LC,
                &fixture.root,
                &[ProviderName::Codex]
            ),
            ProviderTrustPreparationResult::Ready { .. }
        ));
        assert_eq!(
            std::fs::read_to_string(fixture.codex_config()).unwrap(),
            user_pre_trusted
        );
        let revocation = fixture
            .registry
            .revoke_trusted(PROJECT, LC, &ProviderName::Codex, &fixture.root)
            .unwrap();
        assert_eq!(revocation.outcome, ProviderTrustRevocationOutcome::NotOwned);
        assert_eq!(
            std::fs::read_to_string(fixture.codex_config()).unwrap(),
            user_pre_trusted
        );
    }

    #[test]
    fn provider_trust_audit_records_before_after_digests() {
        let fixture = TrustFixture::new();
        let providers = [ProviderName::Codex, ProviderName::KimiCode];
        assert!(matches!(
            fixture.registry.ensure_before_recipe(
                PROJECT,
                OPERATION,
                LC,
                &fixture.root,
                &providers
            ),
            ProviderTrustPreparationResult::Ready { .. }
        ));

        let audits = fixture.store().list_audit(PROJECT).unwrap();
        let ensure_audits: Vec<_> = audits.iter().filter(|a| a.action == "ensure").collect();
        assert_eq!(ensure_audits.len(), 2);
        for audit in &ensure_audits {
            assert_eq!(audit.lc_id, LC);
            assert_eq!(audit.operation_id.as_deref(), Some(OPERATION));
            assert_eq!(audit.canonical_root, fixture.root);
            assert!(!audit.trust_key.is_empty());
            assert!(
                audit
                    .before_digest
                    .as_ref()
                    .is_none_or(|digest| digest.starts_with("sha256:"))
            );
            let after = audit.after_digest.as_ref().expect("ensure after digest");
            assert!(after.starts_with("sha256:"));
            assert_ne!(audit.before_digest.as_ref(), Some(after));
            assert!(!audit.recorded_at.is_empty());
        }

        for provider in providers {
            fixture
                .registry
                .revoke_trusted(PROJECT, LC, &provider, &fixture.root)
                .unwrap();
        }
        let audits = fixture.store().list_audit(PROJECT).unwrap();
        let revoke_audits: Vec<_> = audits.iter().filter(|a| a.action == "revoke").collect();
        assert_eq!(revoke_audits.len(), 2);
        for audit in &revoke_audits {
            // 撤销前摘要必在；撤销后工件可能整体消失（Kimi 记录文件被删），
            // 此时 after 为 None，其余情况必为 sha256 摘要。
            assert!(audit.before_digest.as_ref().unwrap().starts_with("sha256:"));
            assert!(
                audit
                    .after_digest
                    .as_ref()
                    .is_none_or(|digest| digest.starts_with("sha256:"))
            );
        }

        // 审计只含 digest/path/key 事实，不携带敏感凭据体积。
        for audit in &audits {
            assert!(serde_json::to_string(audit).unwrap().len() < 1024);
        }
    }

    fn sha256_hex_12(value: &str) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(value.as_bytes());
        format!("{:x}", digest)[..12].to_string()
    }
}
