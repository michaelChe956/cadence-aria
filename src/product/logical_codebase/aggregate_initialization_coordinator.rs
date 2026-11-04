//! Five stable step ID state machine driver for aggregate initialization.
//!
//! `AggregateInitializationCoordinator` advances an
//! [`AggregateInitializationOperation`] through the five stable step IDs defined
//! by [`AggregateInitializationStepKind::V1`]:
//!
//! 1. `machine_skills` — deterministic Cadence code. Drives
//!    [`AggregateSkillsPreparation::prepare_skills`] and persists the source/link
//!    digests, skill paths and warnings. It never impersonates the aggregate
//!    root as a provider cwd and never writes codebase members.
//! 2. `aggregate_preflight` — deterministic Cadence code. Drives
//!    [`AggregatePreflightService::inspect`] which validates the manifest, the
//!    canonical non-Git aggregate root, member main checkouts and that the
//!    aggregate index excludes assets, then emits an immutable `preflight.json`.
//! 3. `pre_check` — the first provider turn and the aggregate root.
//! 4. `rule_and_mcp_config` — the second provider turn, merging the legacy
//!    `rule_config` and `mcp_configuration` initialization stages.
//! 5. `openspec_and_examples` — the third provider turn.
//!
//! The coordinator never lets `machine_skills`/`aggregate_preflight` spawn a
//! provider ([`AggregateInitializationStepKind::is_provider_turn`]); exactly
//! three provider turns run, all after the two deterministic steps. Strict
//! ordering is enforced by the durable store, so a failed step leaves every
//! subsequent step `Pending`. Each step uses an input digest of the form
//! `aggregate-init:{project}:{operation}:{step}:{input_digest}`, checkpoints
//! its output artifact reference before being marked completed, and is
//! retryable/idempotent/cancellable through the store primitives.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::product::app_paths::ProductAppPaths;
use crate::product::audit_snapshot_budget::AUDIT_BUDGET_EXCEEDED_REASON_CODE;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::logical_codebase::aggregate_initialization::{
    AggregateCancellationRecord, AggregateInitializationErrorRecord,
    AggregateInitializationOperation, AggregateInitializationOperationInput,
    AggregateInitializationOperationStatus, AggregateInitializationProfile,
    AggregateInitializationStepKind, AggregateInitializationStepStatus, AggregateProviderSelection,
    AggregateSkillsInitialization, RepositoryTypeEvidence,
};
use crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
use crate::product::logical_codebase::capability_rejection::{
    CapabilityLaunchRejection, REASON_CODE_CAPABILITY_DENIED,
    REASON_CODE_CAPABILITY_EVIDENCE_STALE, REASON_CODE_CAPABILITY_UNVERIFIED,
    classify_capability_launch_rejection,
};
use crate::product::logical_codebase::environment_probe::AggregateEnvironmentProbe;
use crate::product::logical_codebase::legacy_logical_codebase_id;
use crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential;
use crate::product::logical_codebase::provider_trust::{
    ProviderTrustPrecondition, ProviderTrustWaiting,
};
use crate::product::logical_codebase::registration::AggregateRootPreflight;
use crate::product::logical_codebase::store::{LogicalCodebaseManifest, LogicalCodebaseStore};
use crate::product::logical_codebase::types::{CodebaseMemberRecord, RepositoryCheckoutRecord};
use crate::product::models::ProviderName;

/// Errors surfaced by aggregate-initialization coordinator operations. Every
/// variant carries enough context for the operation record to be marked failed
/// with a retryable/action classification.
#[derive(Debug, thiserror::Error)]
pub enum AggregateInitializationError {
    #[error("aggregate initialization operation not found: {id}")]
    NotFound { id: String },
    #[error("aggregate initialization state machine rejected transition for {id}: {detail}")]
    StateRejected {
        id: String,
        detail: String,
        retryable: bool,
    },
    #[error("aggregate skills preparation failed: {reason}")]
    SkillsPreparation { reason: String, retryable: bool },
    /// 带结构化成因的 skills 准备失败。`cause` 是结构化子码
    /// （`skills_unavailable_<stage>` / `skills_sync_failed_<layer>` /
    /// `skills_digest_failed`），`reason` 仍是人读的原始原因。
    #[error("aggregate skills preparation failed: {reason}")]
    SkillsPreparationDiagnosed {
        reason: String,
        retryable: bool,
        cause: String,
    },
    #[error("aggregate preflight failed: {reason}")]
    Preflight { reason: String, retryable: bool },
    #[error("aggregate policy publication failed: {reason}")]
    PolicyPublication { reason: String, retryable: bool },
    #[error("provider turn {step:?} failed: {reason}")]
    ProviderTurn {
        step: AggregateInitializationStepKind,
        reason: String,
        retryable: bool,
    },
    /// 带诊断的 provider turn 失败。与 [`Self::ProviderTurn`] 共用同一
    /// `reason_code` / `action` 映射，只是额外带上 provider 名与诊断
    /// （成因子码 + 被丢弃的输出）。
    #[error("provider turn {step:?} failed: {reason}")]
    ProviderTurnDiagnosed {
        step: AggregateInitializationStepKind,
        reason: String,
        retryable: bool,
        provider: Option<String>,
        diagnostic: TurnDiagnostic,
    },
    /// 环境前置失败。`code` **就是**最终的 `reason_code`
    /// （`codegraph_missing` / `codegraph_version_mismatch` /
    /// `aggregate_root_not_writable` / `skills_commands_missing`）。
    #[error("aggregate environment precondition failed: {code}: {reason}")]
    EnvironmentPrecondition {
        code: String,
        reason: String,
        retryable: bool,
    },
    #[error("provider trust gate is waiting before the recipe may start")]
    TrustWaiting { waiting: Box<ProviderTrustWaiting> },
    #[error("cancellation requested")]
    Cancelled,
    #[error("aggregate initialization store error: {0}")]
    Store(#[from] ProductStoreError),
}

/// The structured diagnostic carried by a diagnosed provider-turn failure.
///
/// `cause` is a machine sub-code (e.g. `capability_unverified`), `output` is
/// the bounded, already-redacted provider output that the failure branches
/// used to drop on the floor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnDiagnostic {
    /// The machine sub-code of the failure, when one is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The bounded provider output, when one was captured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

impl AggregateInitializationError {
    fn not_found(id: impl Into<String>) -> Self {
        Self::NotFound { id: id.into() }
    }

    fn state(id: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::StateRejected {
            id: id.into(),
            detail: detail.into(),
            retryable: true,
        }
    }

    /// Convert into a persisted error record classification for
    /// `finish_failed`.
    fn into_error_record(self) -> AggregateInitializationErrorRecord {
        self.error_record()
    }

    /// Borrowing form of [`Self::into_error_record`]: persists a failure as a
    /// terminal record while the caller still returns the original error to
    /// its caller.
    pub(crate) fn error_record(&self) -> AggregateInitializationErrorRecord {
        let (reason_code, retryable, action, cause, provider, stderr_summary) = match self {
            Self::NotFound { .. } => (
                "aggregate_initialization_store_failed".to_string(),
                true,
                "The aggregate initialization state could not be persisted; query the operation after recovery.".to_string(),
                None,
                None,
                None,
            ),
            Self::Store(ProductStoreError::AuditSnapshotBudgetExceeded(failure)) => (
                AUDIT_BUDGET_EXCEEDED_REASON_CODE.to_string(),
                false,
                AUDIT_BUDGET_EXCEEDED_REASON_CODE.to_string(),
                Some(AUDIT_BUDGET_EXCEEDED_REASON_CODE.to_string()),
                None,
                Some(failure.diagnostic()),
            ),
            Self::Store(_) => (
                "aggregate_initialization_store_failed".to_string(),
                true,
                "The aggregate initialization state could not be persisted; query the operation after recovery.".to_string(),
                None,
                None,
                None,
            ),
            Self::StateRejected { retryable, .. } => (
                "aggregate_initialization_state_rejected".to_string(),
                *retryable,
                "The aggregate initialization state machine rejected the transition; review the operation and resubmit.".to_string(),
                None,
                None,
                None,
            ),
            Self::SkillsPreparation { retryable, reason } => (
                "aggregate_machine_skills_failed".to_string(),
                *retryable,
                reason.clone(),
                None,
                None,
                None,
            ),
            Self::SkillsPreparationDiagnosed {
                retryable,
                reason,
                cause,
            } => (
                "aggregate_machine_skills_failed".to_string(),
                *retryable,
                reason.clone(),
                Some(cause.clone()),
                None,
                None,
            ),
            Self::Preflight { retryable, reason } => (
                "aggregate_preflight_failed".to_string(),
                *retryable,
                reason.clone(),
                None,
                None,
                None,
            ),
            Self::PolicyPublication { retryable, reason } => (
                "aggregate_policy_publication_failed".to_string(),
                *retryable,
                reason.clone(),
                None,
                None,
                None,
            ),
            Self::ProviderTurn {
                step,
                reason,
                retryable,
            } => {
                let (reason_code, action) = provider_turn_failure_code(*step, reason);
                (
                    reason_code,
                    *retryable,
                    action,
                    None,
                    None,
                    None,
                )
            }
            Self::ProviderTurnDiagnosed {
                step,
                reason,
                retryable,
                provider,
                diagnostic,
            } => {
                let (reason_code, action) = provider_turn_failure_code(*step, reason);
                (
                    reason_code,
                    *retryable,
                    action,
                    diagnostic.cause.clone(),
                    provider.clone(),
                    diagnostic.output.clone(),
                )
            }
            Self::EnvironmentPrecondition {
                code,
                reason,
                retryable,
            } => (
                code.clone(),
                *retryable,
                reason.clone(),
                None,
                None,
                None,
            ),
            Self::TrustWaiting { waiting } => (
                "aggregate_initialization_trust_waiting".to_string(),
                true,
                waiting.retry_action.clone(),
                None,
                None,
                None,
            ),
            Self::Cancelled => (
                "aggregate_initialization_cancelled".to_string(),
                false,
                "The aggregate initialization was cancelled.".to_string(),
                None,
                None,
                None,
            ),
        };
        AggregateInitializationErrorRecord {
            stage: "aggregate_initialization".to_string(),
            reason_code,
            stderr_summary,
            retryable,
            action,
            cause,
            provider,
        }
    }
}

/// Map a provider-turn failure onto its machine code and action text.
///
/// The two provider-turn variants share this single mapping, so
/// `ProviderTurn` and `ProviderTurnDiagnosed` can never drift apart. A
/// detail that matches none of the capability rules keeps the pre-existing
/// `aggregate_{step}_failed` code and the raw reason as action, byte for
/// byte.
fn provider_turn_failure_code(
    step: AggregateInitializationStepKind,
    reason: &str,
) -> (String, String) {
    let reason_code = match classify_capability_launch_rejection(reason) {
        Some(CapabilityLaunchRejection::Unverified { .. }) => REASON_CODE_CAPABILITY_UNVERIFIED,
        Some(CapabilityLaunchRejection::Denied { .. }) => REASON_CODE_CAPABILITY_DENIED,
        Some(CapabilityLaunchRejection::EvidenceStale { .. }) => {
            REASON_CODE_CAPABILITY_EVIDENCE_STALE
        }
        None => {
            return (
                format!("aggregate_{}_failed", step.as_str()),
                reason.to_string(),
            );
        }
    };
    (reason_code.to_string(), reason.to_string())
}

type Clock = dyn Fn() -> String + Send + Sync;

/// [`AggregateInitializationCoordinator::resolve_skills_initialization`] 的
/// 两条收敛路径。
#[derive(Debug, Clone)]
enum AggregateSkillsHandling {
    /// 聚合根尚未做过 Cadence-skills 初始化（或显式要求重新初始化）：
    /// 走完整五步（含四条命令）。
    RunFullRecipe,
    /// 聚合根已做过 Cadence-skills 初始化：跳过三个 provider-turn 步骤，
    /// 只做聚合政策发布。判定依据随此值一并携带。
    Skip(crate::product::logical_codebase::aggregate_initialization::AggregateSkillsInitialization),
}

/// Drives `machine_skills`: prepares the shared Cadence skill set and reports
/// the preparation result so the coordinator can persist the digests/paths and
/// warnings. It must not be confused with the single-repository
/// [`crate::product::repository_store::CadenceSkillsPreparation`] flow: this
/// trait owns the aggregate step record lifecycle.
#[async_trait]
pub trait AggregateSkillsPreparation: Send + Sync {
    /// Returns a stable digest of the prepared skill source/link state plus the
    /// machine-readable skill paths and any non-fatal warnings.
    async fn prepare_skills(
        &self,
        project_id: &str,
        operation_id: &str,
        cancellation: CancellationToken,
    ) -> Result<MachineSkillsPreparation, AggregateInitializationError>;
}

/// Persistent, byte-stable summary written by the `machine_skills` step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSkillsPreparation {
    pub source_digest: String,
    pub link_digest: String,
    pub skills_root: PathBuf,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Detects the repository type for one member checkout root without invoking
/// package managers, build tools, package scripts, `pnpm install`, Node or
/// Java. The detector only reads the member main checkout root (no recursion,
/// no symlink following outside the root).
pub trait RepositoryTypeDetector: Send + Sync {
    fn detect(
        &self,
        checkout_root: &std::path::Path,
        logical_repository_id: &str,
    ) -> Result<RepositoryTypeEvidence, AggregateInitializationError>;
}

/// Drives `aggregate_preflight`: deterministic Cadence code that inspects the
/// manifest, canonical non-Git aggregate root, member main checkouts and the
/// aggregate index exclusion of assets, then returns the immutable snapshot.
pub trait AggregatePreflightService: Send + Sync {
    fn inspect(
        &self,
        project_id: &str,
        manifest: &LogicalCodebaseManifest,
        cancellation: &CancellationToken,
    ) -> Result<AggregatePreflightSnapshot, AggregateInitializationError>;

    /// Re-scopes this service to one logical codebase subtree. The default
    /// returns `None` for scope-agnostic implementations (e.g. test doubles);
    /// the production [`DeterministicAggregatePreflightService`] overrides it
    /// so member/checkout reads resolve inside the named codebase.
    fn rescoped(&self, _lc_id: &str) -> Option<Arc<dyn AggregatePreflightService>> {
        None
    }
}

/// [`AggregateProviderTurnDriver::run_turn`] 的单 turn 入参：root-recipe
/// turn 的一次性上下文（字段语义与既有平铺参数一一对应）。
pub struct AggregateProviderTurnRequest<'a> {
    pub project_id: &'a str,
    pub operation_id: &'a str,
    pub step: AggregateInitializationStepKind,
    pub preflight: &'a AggregatePreflightSnapshot,
    pub lc_id: Option<&'a str>,
    pub bootstrap: BootstrapPhaseCredential,
    pub cancellation: CancellationToken,
    /// 片4（REQ-BOOT-07，provider 走 request）：本轮实际使用的 provider
    ///（wire 语义的 `ProviderName`，由 durable `operation.input.provider`
    /// 解出）。MUST NOT 存进 driver 字段——driver 是 state 级单例。
    pub provider: &'a ProviderName,
}

/// 跳过式聚合初始化（聚合根已完成 Cadence-skills 初始化，四条命令没
/// 必要跑）的政策发布收口：读政策源 → 合成正文 → 发布不可变 artifact →
/// 签发「跳过式」最终 receipt。
///
/// 实现必须复用与末命令收口同一发布链（`publish_recipe_policy`），只是
/// receipt 语义换成「没跑命令」的证据形态；判定依据由 coordinator 落盘，
/// 这里只负责证明「政策真的发布了、receipt 真的冻结了」。
#[async_trait]
pub trait AggregatePolicyPublication: Send + Sync {
    /// `lc_id` 为 coordinator 的 LC scope（未 scope 时退化为 legacy 别名），
    /// 与 provider-turn 收口使用同一 scope 语义。
    async fn publish_skipped(
        &self,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        canonical_root: &std::path::Path,
        decision: &AggregateSkillsInitialization,
    ) -> Result<(), AggregateInitializationError>;

    /// 本 operation 是否已有任何 root recipe 命令审计事实。`Ok(true)` 关闭
    /// 跳过判定窗口：命令**已经跑过**是「没必要跑」的反证（典型来源：
    /// 旧版本中断后被显式续跑的 operation）。实现读不到审计事实时
    /// fail-closed 报错，绝不把不可观测当成「没跑过」。
    fn has_audited_commands(
        &self,
        _project_id: &str,
        _operation_id: &str,
        _lc_id: &str,
    ) -> Result<bool, AggregateInitializationError> {
        Ok(false)
    }
}

/// Drives a single provider turn for one of `pre_check`, `rule_and_mcp_config`
/// or `openspec_and_examples`. Each call is one Claude turn rooted at the
/// aggregate root; the driver must not run for `machine_skills` or
/// `aggregate_preflight`. Task 1.4 (REQ-BOOT-04): every root-recipe turn runs
/// in the aggregate bootstrap phase — the coordinator derives the
/// [`BootstrapPhaseCredential`] from the durable Running operation (same-lc
/// store) and hands it to the driver so admission can waive only the
/// "root rules not generated yet" check.
#[async_trait]
pub trait AggregateProviderTurnDriver: Send + Sync {
    async fn run_turn(
        &self,
        request: AggregateProviderTurnRequest<'_>,
    ) -> Result<String, AggregateInitializationError>;
}

/// Immutable projection of one member's main checkout captured by
/// `aggregate_preflight`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatePreflightMemberProjection {
    pub logical_repository_id: String,
    pub checkout_id: String,
    pub canonical_path: String,
    pub git_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

/// Immutable snapshot written to `preflight.json` by the `aggregate_preflight`
/// step. The canonical aggregate root, every member's main checkout identity
/// and the index-exclusion marker are all byte-stable.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatePreflightSnapshot {
    pub aggregate_root: String,
    pub index_excludes_assets: bool,
    pub members: Vec<AggregatePreflightMemberProjection>,
    pub manifest_revision: u64,
    pub manifest_digest: String,
    /// REQ-BOOT-09：`advance_remaining` 的 codegraph 前置探测结果落 durable
    /// 预检快照。旧记录缺省 `false`（未探测），不伪造成可用。
    #[serde(default)]
    pub codegraph_available: bool,
    /// 实测 CLI 版本（`codegraph --version`）。未探测时为 `None`。
    #[serde(default)]
    pub codegraph_version: Option<String>,
}

/// Orchestrates the five-step aggregate initialization state machine. The
/// coordinator is retryable: any failure marks the operation failed with a
/// classified error record and leaves subsequent steps `Pending`; interrupted
/// operations are recovered through [`Self::recover_interrupted`].
pub struct AggregateInitializationCoordinator {
    paths: ProductAppPaths,
    lc_id: Option<String>,
    operations: AggregateInitializationOperationStore,
    skills: Arc<dyn AggregateSkillsPreparation>,
    preflight: Arc<dyn AggregatePreflightService>,
    provider: Arc<dyn AggregateProviderTurnDriver>,
    detector: Arc<dyn RepositoryTypeDetector>,
    clock: Arc<Clock>,
    /// Task 1.4（REQ-REG-14）：五步 recipe 的 trust 硬前置门。`None` 表示该
    /// coordinator 未装配 trust 门——`execute_with_trust` 对此 fail-closed；
    /// 既有 `execute` 路径不受影响（handler 接线由 Task 1.8 完成）。
    trust: Option<Arc<dyn ProviderTrustPrecondition>>,
    /// 跳过式聚合初始化（不需要四条 Cadence-skills 命令）的政策发布收口。
    /// `None` 表示该依赖图没有发布面：一旦判定要跳过就 fail-closed，绝不
    /// 静默跳过政策发布。
    policy: Option<Arc<dyn AggregatePolicyPublication>>,
    /// 环境前置探测（REQ-BOOT-09）：`advance_remaining` 开头调用，失败即
    /// 以 `step: None` 让 operation 失败。`None` 表示该依赖图未装配探针
    /// （测试默认），生产由 `production_dependencies` 注入
    /// [`crate::product::logical_codebase::environment_probe::CodeGraphEnvironmentProbe`]。
    environment_probe: Option<Arc<dyn AggregateEnvironmentProbe>>,
    /// skills 链接层校验的 HOME 依据（与生产 skills manager 同源）。`None`
    /// 表示未装配：跳过链接层校验（既有测试构造点零迁移）。
    skills_home: Option<PathBuf>,
}

// 以下各职责模块通过文本包含(`include!`)进入此文件,保持模块命名空间与公开 API 不变。
// 拆分仅为满足 large_file_guard 的 1200 行上限,不改变任何行为。

include!("coordinator_frozen_provider.inc.rs");
include!("coordinator_provider_turn.inc.rs");
include!("coordinator_lifecycle.inc.rs");
include!("coordinator_lifecycle_provider_setup.inc.rs");
include!("coordinator_preflight.inc.rs");
include!("coordinator_profile.inc.rs");
include!("coordinator_tests.inc.rs");
