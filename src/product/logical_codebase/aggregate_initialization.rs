//! Aggregate initialization operation record types.
//!
//! `AggregateInitializationOperation` is intentionally independent of the
//! single-repository six-step `RepositoryInitializationOperation`. It advances
//! through five stable step IDs and never reuses the single-repository
//! `GitFinalize` cut point or `RepositoryStore::create_repository` dependency.
//!
//! The serialized JSON shape is byte-stable: field order, naming and enum
//! casing are part of the durable protocol for aggregate-initialization
//! records persisted beneath
//! `.aria/projects/{project}/logical-codebase/aggregate-initializations/`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Aggregate initialization profile resolved from read-only member-checkout
/// signals. Drives template/precheck selection but never changes the five
/// stable step IDs in [`AggregateInitializationStepKind::V1`].
///
/// `JavaBackend` uses the Maven/Gradle-aware template, `FrontendPnpmVite`
/// uses the pnpm/Vite precheck and never applies Java commands, and `Mixed`
/// composes a namespaced template spanning both. `Unknown` or an
/// in-request profile inconsistency fails preflight closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateInitializationProfile {
    JavaBackend,
    FrontendPnpmVite,
    Mixed,
}

impl AggregateInitializationProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::JavaBackend => "java_backend",
            Self::FrontendPnpmVite => "frontend_pnpm_vite",
            Self::Mixed => "mixed",
        }
    }
}

/// Read-only evidence captured for one member checkout root by a
/// [`crate::product::logical_codebase::aggregate_initialization_coordinator::RepositoryTypeDetector`].
/// Never executes package scripts, `pnpm install`, Node or Java.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryTypeEvidence {
    pub logical_repository_id: String,
    pub repo_type: crate::product::logical_codebase::types::RepositoryType,
    #[serde(default)]
    pub tech_stack: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_digest: Option<String>,
}

/// Stable, durable step identifier for the aggregate initialization flow.
///
/// The serialized string form (`as_str`) is the wire protocol and must not
/// change once a record is persisted; `V1` fixes the canonical order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateInitializationStepKind {
    MachineSkills,
    AggregatePreflight,
    PreCheck,
    RuleAndMcpConfig,
    OpenspecAndExamples,
}

impl AggregateInitializationStepKind {
    /// Canonical five-step layout for the aggregate initialization flow.
    pub const V1: [Self; 5] = [
        Self::MachineSkills,
        Self::AggregatePreflight,
        Self::PreCheck,
        Self::RuleAndMcpConfig,
        Self::OpenspecAndExamples,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MachineSkills => "machine_skills",
            Self::AggregatePreflight => "aggregate_preflight",
            Self::PreCheck => "pre_check",
            Self::RuleAndMcpConfig => "rule_and_mcp_config",
            Self::OpenspecAndExamples => "openspec_and_examples",
        }
    }

    /// Steps that drive a real provider turn through the logical-codebase
    /// gateway. Machine skills and preflight are deterministic Cadence code
    /// and must not spawn a provider.
    pub const fn is_provider_turn(self) -> bool {
        matches!(
            self,
            Self::PreCheck | Self::RuleAndMcpConfig | Self::OpenspecAndExamples
        )
    }

    pub fn parse_from_str(value: &str) -> Option<Self> {
        Self::V1.iter().copied().find(|kind| kind.as_str() == value)
    }

    pub fn index(self) -> usize {
        Self::V1
            .iter()
            .position(|candidate| *candidate == self)
            .expect("aggregate initialization step kind is always in V1")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateInitializationStepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    /// 判定为「该步骤没有必要跑」后的终态（owner 2026-10-29）。只有
    /// provider-turn 步骤可以 skip，且必须先有
    /// [`AggregateSkillsInitialization`] 的跳过判定依据；绝不静默忽略。
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateInitializationStepRecord {
    pub step_id: AggregateInitializationStepKind,
    pub status: AggregateInitializationStepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    /// Digest of the step input (`aggregate-init:{project}:{operation}:{step}:{input_digest}`)
    /// captured when the step started running, so a persisted checkpoint can be
    /// matched against a replayed request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_digest: Option<String>,
    /// Artifact reference captured by `checkpoint_step_output` before the step
    /// is marked completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_artifact_ref: Option<String>,
    /// 步骤被跳过时的原因（reason code）。只对 `Skipped` 状态存在，是
    /// 「确实没必要跑」的记录面证据；旧记录反序列化缺省 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
}

impl AggregateInitializationStepStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateInitializationOperationStatus {
    Created,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Frozen projection of a single member's initialization-relevant evidence
/// (checkout, revision, profile digest). Populated by the deterministic
/// preflight step and never by a provider turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateMemberInitializationProjection {
    pub logical_repository_id: String,
    pub checkout_id: String,
    pub revision: String,
    pub dirty: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateCancellationRecord {
    pub reason_code: String,
    pub cancelled_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateInitializationErrorRecord {
    pub stage: String,
    pub reason_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_summary: Option<String>,
    pub retryable: bool,
    pub action: String,
    /// 结构化成因子码（如 `skills_unavailable_clone` /
    /// `aggregate_provider_capability_unverified`）。缺省表示「原因不可判定」。
    /// `#[serde(default)]` 保持旧记录兼容。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// 产生该失败的那家 provider（wire 名）。缺省表示失败与 provider 无关。
    /// `#[serde(default)]` 保持旧记录兼容。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

impl AggregateInitializationErrorRecord {
    pub fn interrupted() -> Self {
        Self {
            stage: "aggregate_initialization".to_string(),
            reason_code: "aggregate_initialization_interrupted".to_string(),
            stderr_summary: None,
            retryable: true,
            action: "服务在聚合初始化完成前中断；检查可能的部分修改后重新提交".to_string(),
            cause: None,
            provider: None,
        }
    }
}

/// 聚合根上「Cadence-skills 已初始化」的判定文件（相对聚合根路径）。
///
/// 判定只看这两个文件；产品自管的 `<聚合根>/.aria/` 绝不参与判定——该
/// 目录不是 skills 跑过的证据。
pub const AGGREGATE_SKILLS_MARKER_ENTRY: &str = "AGENTS.md";
pub const AGGREGATE_SKILLS_MARKER_LANGUAGE_RULE: &str = ".claude/rules/language.md";

/// 聚合根已初始化 → 判定四条 Cadence-skills 命令没必要的 reason code。
pub const AGGREGATE_SKILLS_SKIPPED_REASON: &str = "aggregate_root_already_initialized";
/// 显式「重新初始化（聚合）」强制走完整五步的 reason code。
pub const AGGREGATE_SKILLS_FORCED_REASON: &str = "explicit_reinitialization_requested";

/// 四条 Cadence-skills 命令（`PreCheck`/`RuleAndMcpConfig`/
/// `OpenspecAndExamples`）是否必要的 durable 判定类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateSkillsInitializationKind {
    /// 聚合根尚未做过 Cadence-skills 初始化：四条命令必须跑。
    Required,
    /// 聚合根已做过 Cadence-skills 初始化：跳过四条命令，只做聚合政策。
    Skipped,
    /// 显式重新初始化：即使已初始化也强制跑完四条命令。
    Forced,
}

impl AggregateSkillsInitializationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Skipped => "skipped",
            Self::Forced => "forced",
        }
    }
}

/// 跳过判定的一条文件证据：相对聚合根路径 + 冻结的内容摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateSkillsMarker {
    pub relative_path: String,
    pub digest: String,
}

/// 聚合初始化对「四条 Cadence-skills 命令是否必要」的 durable 判定依据。
///
/// `kind = skipped` 时 `markers` 必须是两个判定文件各自的 `sha256` 摘要；
/// 判定落盘后重放/续跑不覆盖（跳过必须在 operation 记录里可读到）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateSkillsInitialization {
    pub kind: AggregateSkillsInitializationKind,
    pub reason_code: String,
    pub detected_at: String,
    #[serde(default)]
    pub markers: Vec<AggregateSkillsMarker>,
}

impl AggregateSkillsInitialization {
    /// 「聚合根已完成 Cadence-skills 初始化」的跳过判定。
    pub fn skipped(detected_at: String, markers: Vec<AggregateSkillsMarker>) -> Self {
        Self {
            kind: AggregateSkillsInitializationKind::Skipped,
            reason_code: AGGREGATE_SKILLS_SKIPPED_REASON.to_string(),
            detected_at,
            markers,
        }
    }

    /// 「显式重新初始化」的强制判定：无视检测结果跑完整五步。
    pub fn forced(detected_at: String) -> Self {
        Self {
            kind: AggregateSkillsInitializationKind::Forced,
            reason_code: AGGREGATE_SKILLS_FORCED_REASON.to_string(),
            detected_at,
            markers: Vec::new(),
        }
    }

    pub fn is_skipped(&self) -> bool {
        self.kind == AggregateSkillsInitializationKind::Skipped
    }
}

/// 检测聚合根是否已做过 Cadence-skills 初始化：两个判定文件都存在时返回
/// 它们的冻结摘要（顺序固定为 `AGENTS.md`、`.claude/rules/language.md`），
/// 任一缺失返回 `None`。
///
/// 不可观测（IO/权限失败）按 fail-closed 报错：绝不把读不到的文件静默
/// 当成「未初始化」去重跑四条命令。
pub fn detect_skills_initialization(
    root: &std::path::Path,
) -> Result<Option<Vec<AggregateSkillsMarker>>, crate::product::json_store::ProductStoreError> {
    use crate::product::json_store::ProductStoreError;
    use sha2::{Digest, Sha256};

    let mut markers = Vec::with_capacity(2);
    for relative_path in [
        AGGREGATE_SKILLS_MARKER_ENTRY,
        AGGREGATE_SKILLS_MARKER_LANGUAGE_RULE,
    ] {
        let path = root.join(relative_path);
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => {
                let bytes = std::fs::read(&path).map_err(|error| {
                    ProductStoreError::Io(format!("read {}: {error}", path.display()))
                })?;
                markers.push(AggregateSkillsMarker {
                    relative_path: relative_path.to_string(),
                    digest: format!("sha256:{:x}", Sha256::digest(&bytes)),
                });
            }
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "inspect {}: {error}",
                    path.display()
                )));
            }
        }
    }
    Ok(Some(markers))
}

/// 「本 LC 的聚合初始化不需要 provider」时冻结进 `input.provider` 的 wire 名。
///
/// 它**不是**一家 provider：`provider_from_wire` 解不出这个串，因此执行期
/// 真要跑 provider turn 时必然 fail-closed（`provider_unsupported_for_
/// aggregate_recipe`），绝不静默回落到别的 provider。
pub const AGGREGATE_PROVIDER_NOT_REQUIRED: &str = "not_required";

/// 聚合初始化对「provider 是否必需」的判定结论（创建期）。
///
/// 零 provider turn 的充要条件：聚合根已具备两个判定文件
/// （`AGENTS.md` + `.claude/rules/language.md`）⇒ 四条 Cadence-skills 命令
/// 没必要跑 ⇒ 整条链零 provider turn ⇒ **不需要任何 provider**。
///
/// 与执行期的跳过判定同源（同一个 [`detect_skills_initialization`] 原语 +
/// 同一对判定文件常量），只是把「要不要 provider」这件事提前到创建期认出来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggregateProviderRequirement {
    /// 整条链零 provider turn：不需要 provider（免过 provider 选择门）。
    NotRequired {
        /// 两个判定文件的冻结摘要（判定依据，顺序固定）。
        markers: Vec<AggregateSkillsMarker>,
    },
    /// 需要 provider：必须过 provider 选择门（REQ-BOOT-07）。
    Required,
}

impl AggregateProviderRequirement {
    /// 是否「不需要 provider」（整条链零 provider turn）。
    pub fn is_not_required(&self) -> bool {
        matches!(self, Self::NotRequired { .. })
    }
}

/// 创建期的 provider 需求判定（判定前移，owner 2026-11-04）。
///
/// - `full_recipe_requested`（手动「启动聚合初始化」/「重新初始化」）⇒ 永远
///   需要 provider：四条 Cadence-skills 命令必须真跑。
/// - 冷启动自动推进（`full_recipe_requested = false`）+ 两个判定文件都在
///   ⇒ 零 provider turn ⇒ 不需要 provider ⇒ 免过 provider 选择门。
/// - 两个判定文件不在 ⇒ 需要 provider ⇒ 选择门照旧评估（不过 ⇒ 不创建
///   operation，守 REQ-BOOT-07）。
///
/// 读不到判定文件（IO/权限失败）**不等于**「未初始化」：按 fail-closed
/// 上抛，绝不据此跳过四条命令，也绝不把失败改写成「不需要 provider」。
pub fn resolve_aggregate_provider_requirement(
    root: &std::path::Path,
    full_recipe_requested: bool,
) -> Result<AggregateProviderRequirement, crate::product::json_store::ProductStoreError> {
    if full_recipe_requested {
        return Ok(AggregateProviderRequirement::Required);
    }
    match detect_skills_initialization(root)? {
        Some(markers) => Ok(AggregateProviderRequirement::NotRequired { markers }),
        None => Ok(AggregateProviderRequirement::Required),
    }
}

/// C4 Task 6：显式 bootstrap 动作在既有 operation 上的可审计命令记录。
///
/// 同一 `command_id` 重放必须返回同一 durable 结果：动作执行后把
/// command/action/outcome 追加到目标 operation 记录（`#[serde(default)]`
/// 保持旧 JSON 兼容），重放时先查此列表，不再推进状态。这不是新的
/// 状态机——它只是既有 operation 记录上的审计事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateInitializationActionRecord {
    pub command_id: String,
    pub action: String,
    pub outcome: String,
    pub applied_at: String,
}

/// Input captured at create time. `idempotency_key` plus the manifest/policy
/// digests together define the idempotency identity for `create_idempotent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateInitializationOperationInput {
    pub idempotency_key: String,
    pub manifest_revision: u64,
    pub policy_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_evidence_digest: Option<String>,
    pub provider_context_root: PathBuf,
    pub provider: String,
}

/// The provider selection frozen at operation-creation time.
///
/// `selected` is the wire name of the chosen provider and always equals
/// `operation.input.provider`. `candidates` lists **all** candidate providers
/// in owner order (never reordered by outcome), so the record alone explains
/// why the chosen provider was chosen and which ones were skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateProviderSelection {
    /// The chosen provider's wire name.
    pub selected: String,
    /// Every candidate provider, in owner order.
    pub candidates: Vec<AggregateProviderCandidate>,
    /// The rule version that produced this selection.
    pub rule_version: String,
    /// When the selection was decided.
    pub decided_at: String,
}

/// One candidate row of the provider selection table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateProviderCandidate {
    /// The provider's wire name.
    pub provider: String,
    /// The provider's human-facing name.
    pub display_name: String,
    /// Whether the provider's CLI is available on this machine.
    pub cli_available: bool,
    /// Why the CLI is unavailable (one of the health reason codes).
    #[serde(default)]
    pub cli_reason_code: Option<String>,
    /// The launch capability state of this candidate:
    /// `confirmed` / `unverified` / `denied` / `evidence_stale` /
    /// `not_installed`.
    pub capability_state: String,
    /// Why this candidate was skipped: `cli_unavailable` /
    /// `needs_verification` / `denied` / `evidence_stale`.
    #[serde(default)]
    pub skipped_reason: Option<String>,
    /// The executable action for this candidate (never a bare status word).
    pub action_hint: String,
    /// A warning about this candidate (e.g. the known CLI command gaps).
    /// `None` means "not verified / normal".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// Independent aggregate initialization operation record.
///
/// Byte-stable serialized form: the field set, order and enum casing here are
/// the durable protocol. The single-repository six-step operation store
/// rejects records shaped like this (and vice-versa), so the two flows cannot
/// be confused on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateInitializationOperation {
    pub operation_id: String,
    pub project_id: String,
    pub operation_kind: String,
    pub layout_version: u16,
    pub input: AggregateInitializationOperationInput,
    pub status: AggregateInitializationOperationStatus,
    pub steps: Vec<AggregateInitializationStepRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_step: Option<AggregateInitializationStepKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_step: Option<AggregateInitializationStepKind>,
    #[serde(default)]
    pub member_projections: Vec<AggregateMemberInitializationProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellation: Option<AggregateCancellationRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<AggregateInitializationErrorRecord>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub action_records: Vec<AggregateInitializationActionRecord>,
    /// 「重新初始化（聚合）」入口显式要求跑完整五步；创建时冻结，不受
    /// 聚合根检测影响。`#[serde(default)]` 保持旧记录兼容。
    #[serde(default)]
    pub full_recipe_requested: bool,
    /// 「四条 Cadence-skills 命令是否必要」的判定依据（owner 2026-10-29）。
    /// `skipped` 时三个 provider-turn 步骤被标记 `Skipped`，判定与证据
    /// 必须能从 operation 记录直接读到；旧记录缺省 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills_initialization: Option<AggregateSkillsInitialization>,
    /// provider 选择结果（创建期冻结）。候选表按 owner 顺序落盘，旧记录缺省
    /// `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_selection: Option<AggregateProviderSelection>,
}

/// Layout version for the persisted aggregate-initialization record shape.
pub const AGGREGATE_INITIALIZATION_LAYOUT_VERSION: u16 = 1;

/// Operation kind discriminator persisted as `operation_kind`. Also reused as
/// the kind tag in store errors.
pub const AGGREGATE_INITIALIZATION_OPERATION_KIND: &str = "aggregate_initialization";

impl AggregateInitializationOperation {
    pub fn new(
        operation_id: String,
        project_id: String,
        input: AggregateInitializationOperationInput,
        created_at: String,
    ) -> Self {
        Self {
            operation_id,
            project_id,
            operation_kind: AGGREGATE_INITIALIZATION_OPERATION_KIND.to_string(),
            layout_version: AGGREGATE_INITIALIZATION_LAYOUT_VERSION,
            input,
            status: AggregateInitializationOperationStatus::Created,
            steps: AggregateInitializationStepKind::V1
                .iter()
                .copied()
                .map(|step_id| AggregateInitializationStepRecord {
                    step_id,
                    status: AggregateInitializationStepStatus::Pending,
                    started_at: None,
                    completed_at: None,
                    input_digest: None,
                    output_artifact_ref: None,
                    skip_reason: None,
                })
                .collect(),
            current_step: None,
            failed_step: None,
            member_projections: Vec::new(),
            cancellation: None,
            error: None,
            updated_at: created_at.clone(),
            created_at,
            completed_at: None,
            action_records: Vec::new(),
            full_recipe_requested: false,
            skills_initialization: None,
            provider_selection: None,
        }
    }

    /// 「重新初始化（聚合）」入口：冻结「强制跑完整五步」的创建期事实。
    pub fn with_full_recipe_requested(mut self, full_recipe_requested: bool) -> Self {
        self.full_recipe_requested = full_recipe_requested;
        self
    }

    /// 「provider 选择」入口：冻结创建期的 provider 选择事实（候选表按 owner
    /// 顺序）。选择结果一旦落账就不再随机器状态变化。
    ///
    /// 接受 `AggregateProviderSelection` 或 `Option<AggregateProviderSelection>`
    /// 两种入参：调用方（片4 的选择期入口）手上可能拿到的是「有选择 / 无
    /// 选择」的可选值，直接透传即可。
    pub fn with_provider_selection(
        mut self,
        provider_selection: impl Into<Option<AggregateProviderSelection>>,
    ) -> Self {
        self.provider_selection = provider_selection.into();
        self
    }

    pub fn idempotency_identity(&self) -> AggregateInitializationIdempotencyIdentity {
        AggregateInitializationIdempotencyIdentity {
            project_id: self.project_id.clone(),
            idempotency_key: self.input.idempotency_key.clone(),
            manifest_revision: self.input.manifest_revision,
            policy_digest: self.input.policy_digest.clone(),
            profile_evidence_digest: self.input.profile_evidence_digest.clone(),
        }
    }
}

/// Fields compared by `create_idempotent` to decide whether a retried create
/// returns the same record or a conflict.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AggregateInitializationIdempotencyIdentity {
    pub project_id: String,
    pub idempotency_key: String,
    pub manifest_revision: u64,
    pub policy_digest: String,
    pub profile_evidence_digest: Option<String>,
}

#[cfg(test)]
mod provider_requirement_tests {
    use super::{
        AGGREGATE_SKILLS_MARKER_ENTRY, AGGREGATE_SKILLS_MARKER_LANGUAGE_RULE,
        AggregateProviderRequirement, resolve_aggregate_provider_requirement,
    };

    /// 写两个判定文件（聚合根已做过 Cadence-skills 初始化的形态）。
    fn write_both_markers(root: &std::path::Path) {
        std::fs::create_dir_all(root.join(".claude").join("rules")).expect("rules dir");
        std::fs::write(root.join(AGGREGATE_SKILLS_MARKER_ENTRY), "# agents\n").expect("agents");
        std::fs::write(
            root.join(AGGREGATE_SKILLS_MARKER_LANGUAGE_RULE),
            "# language\n",
        )
        .expect("language rule");
    }

    #[test]
    fn both_marker_files_mean_no_provider_is_required() {
        let temp = tempfile::tempdir().expect("temp");
        write_both_markers(temp.path());

        let requirement =
            resolve_aggregate_provider_requirement(temp.path(), false).expect("requirement");
        assert!(
            requirement.is_not_required(),
            "two marker files mean the whole chain needs zero provider turn"
        );
        let AggregateProviderRequirement::NotRequired { markers } = &requirement else {
            panic!("expected NotRequired, got {requirement:?}");
        };
        assert_eq!(markers.len(), 2, "both marker files are the evidence");
        assert_eq!(markers[0].relative_path, AGGREGATE_SKILLS_MARKER_ENTRY);
        assert_eq!(
            markers[1].relative_path,
            AGGREGATE_SKILLS_MARKER_LANGUAGE_RULE
        );
        assert!(
            markers
                .iter()
                .all(|marker| marker.digest.starts_with("sha256:"))
        );
    }

    #[test]
    fn a_missing_marker_file_still_requires_a_provider() {
        let temp = tempfile::tempdir().expect("temp");
        std::fs::write(
            temp.path().join(AGGREGATE_SKILLS_MARKER_ENTRY),
            "# agents\n",
        )
        .expect("agents");
        assert_eq!(
            resolve_aggregate_provider_requirement(temp.path(), false).expect("requirement"),
            AggregateProviderRequirement::Required,
            "one missing marker file still needs a provider"
        );

        let temp = tempfile::tempdir().expect("temp");
        std::fs::create_dir_all(temp.path().join(".claude").join("rules")).expect("rules dir");
        std::fs::write(
            temp.path().join(AGGREGATE_SKILLS_MARKER_LANGUAGE_RULE),
            "# language\n",
        )
        .expect("language rule");
        assert_eq!(
            resolve_aggregate_provider_requirement(temp.path(), false).expect("requirement"),
            AggregateProviderRequirement::Required,
            "one missing marker file still needs a provider"
        );
    }

    #[test]
    fn an_absent_root_still_requires_a_provider() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("never-created");
        assert_eq!(
            resolve_aggregate_provider_requirement(&root, false).expect("requirement"),
            AggregateProviderRequirement::Required,
            "an unreadable/absent root must not be treated as initialized"
        );
    }

    #[test]
    fn a_forced_full_recipe_always_requires_a_provider() {
        let temp = tempfile::tempdir().expect("temp");
        write_both_markers(temp.path());
        assert_eq!(
            resolve_aggregate_provider_requirement(temp.path(), true).expect("requirement"),
            AggregateProviderRequirement::Required,
            "an explicit full recipe always runs the four commands, so it needs a provider"
        );
    }
}
