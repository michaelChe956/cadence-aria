use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{FindingFingerprint, is_lowercase_sha256_hex};
use crate::product::models::workspace::{SingleCandidatePhase, WorkspaceSessionRecord};
use crate::product::planning_failure::{PlanningFailureKind, StructuralAuthorizationKind};

const SCOPE_DIGEST_PREFIX: &str = "review_scope_v1:";
const SCOPE_DIGEST_VERSION: &str = "review-invocation-scope/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingClass {
    MechanicalError,
    Repairable,
    HumanRequired,
    Advisory,
}

impl FindingClass {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::MechanicalError => "mechanical_error",
            Self::Repairable => "repairable",
            Self::HumanRequired => "human_required",
            Self::Advisory => "advisory",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFindingCategory {
    ContractGap,
    SelfContradiction,
    ScopeConflict,
    VerificationUnattributable,
    Completeness,
    Other,
}

impl ReviewFindingCategory {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ContractGap => "contract_gap",
            Self::SelfContradiction => "self_contradiction",
            Self::ScopeConflict => "scope_conflict",
            Self::VerificationUnattributable => "verification_unattributable",
            Self::Completeness => "completeness",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingClassHint {
    Repairable,
    HumanRequired,
    Advisory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifiedFinding {
    pub class: FindingClass,
    pub fingerprint: FindingFingerprint,
    pub category: Option<ReviewFindingCategory>,
    pub severity: String,
    pub message: String,
    pub evidence: Option<String>,
    pub required_action: Option<String>,
    /// F-52（REQ-TOP-04 场景 4）：finding 身份落在无稳定 ID 的措辞域时为
    /// true——重复检测禁自动裁决（fail-safe 人工），durable 旧 JSON 缺省
    /// false（legacy 口径视为稳定）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub identity_unstable: bool,
    pub contract_field: Option<String>,
}

/// Durable snapshot of the one human decision point for a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanGateSnapshot {
    pub findings: Vec<ClassifiedFinding>,
    pub repeated_fingerprints: Vec<FindingFingerprint>,
    pub attempts_used: u32,
    pub manual_repairs_remaining: u32,
    /// C2（REQ-HGC-01）：本 logical gate 内被接受为修订 turn 的反馈轮次，
    /// 与 `manual_repairs_remaining` 预留同一 CAS 原子递增（不双计 policy
    /// 的 repairs_used/manual_repairs_used）。旧会话 durable JSON 缺省 None
    /// ——gate-local 事实缺席，保守呈现「预算历史不可用」，不补计数不改历史。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_feedback_turns: Option<u32>,
    /// C1 Task 4（REQ-C1-GATE-01）：候选快照完整性评估 + 恢复命令账本；
    /// 在 relay/observer 之前随门快照原子落盘。旧会话 durable JSON 缺省
    /// None——不补写，approve/feedback 语义保持既有行为零回归。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_recovery: Option<CandidateSnapshotRecovery>,
    pub trigger: super::HumanReason,
    pub resumable: bool,
}

/// C1 Task 4（REQ-C1-GATE-01）：候选门恢复动作。`Recover`＝恢复原门/原轮次；
/// `Rebuild`＝允许从权威 source/IR/report 重建呈现面（不新建候选权威）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateRecoveryAction {
    Recover,
    Rebuild,
}

/// C1 Task 4：候选快照完整性评估事实。`complete=false` 时 `missing` 列出
/// 缺失/不可读事实，approve 固定 fail-closed（不扣预算、不启动 provider）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateSnapshotRecovery {
    pub complete: bool,
    pub gate_id: String,
    pub source_revision_ref: Option<String>,
    pub source_revision_hash: Option<String>,
    pub plan_candidate_ir_ref: Option<String>,
    pub mechanical_report_ref: Option<String>,
    pub budget_remaining: Option<u32>,
    /// 缺失/不可读事实的诊断清单（如 `source_revision_missing`、
    /// `mechanical_report_unreadable`）；complete=true 时为空。
    pub missing: Vec<String>,
    /// 已完成步骤（inbox 投影：candidate_source_persisted 等）。
    pub completed_steps: Vec<String>,
    /// 恢复命令幂等账本：同 command 同负载重放首次 durable 结果，
    /// 异 payload fail-closed。
    #[serde(default)]
    pub commands: Vec<CandidateRecoveryCommandRecord>,
    pub assessed_at: String,
}

/// C1 Task 4（REQ-C1-GATE-02）：单条恢复命令的 durable 结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateRecoveryCommandRecord {
    pub command_id: String,
    pub action: CandidateRecoveryAction,
    pub state: crate::product::models::OperationState,
    pub missing: Vec<String>,
    pub recorded_at: String,
}

/// Durable provider-start idempotency record. The key is the source of truth
/// when recovering an interrupted run and counting provider starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderStartLedgerEntry {
    pub provider_start_idempotency_key: String,
    pub started: bool,
    /// F-19（诊断，wire 兼容）：登记的 provider 名（ProviderName 的 serde 文本，
    /// 如 `codex`/`pi`/`claude_code`）。仅 legacy 流登记路径填写；既有 SC/门
    /// 预留路径保持 `None`，旧 JSON 与 wire 输出零变化。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// F-19（诊断，wire 兼容）：登记时间（RFC3339 UTC）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// 教学窗口（REQ-WSC-28C/D4，wire 兼容）：该物理启动归属的教学
    /// attempt 键（sc_generation:{session}:{gen}:attempt:{n}）；None =
    /// 非教学启动。旧 JSON 缺键读 None、序列化不写键（旧 wire 零变化）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_attempt_key: Option<String>,
}

impl ProviderStartLedgerEntry {
    /// SG4a（REQ-SG-05）：实际 launch 观察判别——必须同时具备 started
    /// 事实与登记的 provider/started_at。旧预留占位条目（started=true 但
    /// provider/started_at 缺失）不构成实际启动证据，launch 投影读
    /// Unknown；旧 started 布尔不得冒充 actual start。
    pub fn is_actual_launch_observation(&self) -> bool {
        self.started && self.provider.is_some() && self.started_at.is_some()
    }
}

/// Per-artifact review budget state. A review cycle is identified by the
/// durable artifact key (`outline:<id>`, `draft:<outline_id>`, or `batch:<id>`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReviewCycleState {
    /// Automatic repair consumption for this reviewed artifact only. Session
    /// totals remain observability counters and must not gate a new cycle.
    pub repairs_used: u32,
    pub initial_count: u32,
    pub verification_count: u32,
    /// REQ-TOP-04 场景 6（F-52）：初评相位的 actionable finding 指纹集合——
    /// Verification scope 的 original_fingerprints 权威来源（复评判重边界）。
    /// 旧 durable JSON 缺省空集（fail-safe：消费方回退 seen_fingerprints）。
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub original_fingerprints: BTreeSet<FindingFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunHistory {
    pub seen_fingerprints: BTreeSet<FindingFingerprint>,
    pub repairs_used: u32,
    pub manual_repairs_used: u32,
    pub transitions_used: u32,
    /// Session totals are observability-only. Budget enforcement is exclusively
    /// performed by `review_cycles` so multiple artifacts may each be reviewed.
    pub initial_review_count: u32,
    pub verification_review_count: u32,
    pub review_cycles: BTreeMap<String, ReviewCycleState>,
}

#[allow(clippy::derivable_impls)]
impl Default for RunHistory {
    fn default() -> Self {
        Self {
            seen_fingerprints: BTreeSet::new(),
            repairs_used: 0,
            manual_repairs_used: 0,
            transitions_used: 0,
            initial_review_count: 0,
            verification_review_count: 0,
            review_cycles: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunBudgets {
    pub max_repairs: u32,
    pub max_transitions: u32,
    pub max_manual_repairs: u32,
}

impl Default for RunBudgets {
    fn default() -> Self {
        Self {
            max_repairs: 1,
            max_transitions: 12,
            max_manual_repairs: 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPolicy {
    Interactive,
    AutoIfValid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanFlowKind {
    Legacy,
    SingleCandidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPhase {
    Initial,
    Verification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "phase",
    rename_all = "snake_case",
    deny_unknown_fields,
    try_from = "RawReviewInvocationScope"
)]
pub enum ReviewInvocationScope {
    Initial {
        initial_revision_id: String,
        scope_digest: String,
    },
    Verification {
        original_fingerprints: BTreeSet<FindingFingerprint>,
        repaired_revision_id: String,
        mechanical_report_ref: String,
        /// REQ-TOP-04 场景 6/7（F-52）：本 invocation 链锚定的初评候选 ref
        /// （cycle key 派生源）。纯元数据，不参与 scope_digest；旧 durable
        /// 记录缺省 None → fail-safe 退化用 repaired_revision_id 派生 key。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cycle_anchor_revision_id: Option<String>,
        scope_digest: String,
    },
}

impl ReviewInvocationScope {
    pub fn initial(initial_revision_id: impl Into<String>) -> Self {
        let initial_revision_id = initial_revision_id.into();
        let scope_digest = scope_digest_for_initial(&initial_revision_id);
        Self::Initial {
            initial_revision_id,
            scope_digest,
        }
    }

    pub fn verification(
        original_fingerprints: BTreeSet<FindingFingerprint>,
        repaired_revision_id: impl Into<String>,
        mechanical_report_ref: impl Into<String>,
        cycle_anchor_revision_id: Option<String>,
    ) -> Self {
        let repaired_revision_id = repaired_revision_id.into();
        let mechanical_report_ref = mechanical_report_ref.into();
        // anchor 是纯元数据，不参与 scope_digest（复评身份仍由指纹集+refs 锚定）。
        let scope_digest = scope_digest_for_verification(
            &original_fingerprints,
            &repaired_revision_id,
            &mechanical_report_ref,
        );
        Self::Verification {
            original_fingerprints,
            repaired_revision_id,
            mechanical_report_ref,
            cycle_anchor_revision_id,
            scope_digest,
        }
    }

    pub fn validate_digest(&self) -> Result<(), InvalidReviewInvocationScopeDigest> {
        let expected = match self {
            Self::Initial {
                initial_revision_id,
                ..
            } => scope_digest_for_initial(initial_revision_id),
            Self::Verification {
                original_fingerprints,
                repaired_revision_id,
                mechanical_report_ref,
                ..
            } => scope_digest_for_verification(
                original_fingerprints,
                repaired_revision_id,
                mechanical_report_ref,
            ),
        };
        let actual = self.scope_digest();

        if is_valid_scope_digest(actual) && actual == expected {
            Ok(())
        } else {
            Err(InvalidReviewInvocationScopeDigest)
        }
    }

    pub fn scope_digest(&self) -> &str {
        match self {
            Self::Initial { scope_digest, .. } | Self::Verification { scope_digest, .. } => {
                scope_digest
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidReviewInvocationScopeDigest;

impl fmt::Display for InvalidReviewInvocationScopeDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("review invocation scope digest is invalid")
    }
}

impl std::error::Error for InvalidReviewInvocationScopeDigest {}

#[derive(Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum RawReviewInvocationScope {
    Initial {
        initial_revision_id: String,
        scope_digest: String,
    },
    Verification {
        original_fingerprints: BTreeSet<FindingFingerprint>,
        repaired_revision_id: String,
        mechanical_report_ref: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cycle_anchor_revision_id: Option<String>,
        scope_digest: String,
    },
}

impl TryFrom<RawReviewInvocationScope> for ReviewInvocationScope {
    type Error = InvalidReviewInvocationScopeDigest;

    fn try_from(scope: RawReviewInvocationScope) -> Result<Self, Self::Error> {
        let scope = match scope {
            RawReviewInvocationScope::Initial {
                initial_revision_id,
                scope_digest,
            } => Self::Initial {
                initial_revision_id,
                scope_digest,
            },
            RawReviewInvocationScope::Verification {
                original_fingerprints,
                repaired_revision_id,
                mechanical_report_ref,
                cycle_anchor_revision_id,
                scope_digest,
            } => Self::Verification {
                original_fingerprints,
                repaired_revision_id,
                mechanical_report_ref,
                cycle_anchor_revision_id,
                scope_digest,
            },
        };
        scope.validate_digest()?;
        Ok(scope)
    }
}

fn scope_digest_for_initial(initial_revision_id: &str) -> String {
    let mut canonical = canonical_prefix("initial");
    write_canonical_scalar(&mut canonical, "initial_revision_id");
    write_canonical_scalar(&mut canonical, initial_revision_id);
    format!(
        "{SCOPE_DIGEST_PREFIX}{}",
        hex::encode(Sha256::digest(canonical))
    )
}

fn scope_digest_for_verification(
    original_fingerprints: &BTreeSet<FindingFingerprint>,
    repaired_revision_id: &str,
    mechanical_report_ref: &str,
) -> String {
    let mut canonical = canonical_prefix("verification");
    write_canonical_scalar(&mut canonical, "mechanical_report_ref");
    write_canonical_scalar(&mut canonical, mechanical_report_ref);
    write_canonical_scalar(&mut canonical, "original_fingerprints");
    for fingerprint in original_fingerprints {
        write_canonical_scalar(&mut canonical, &fingerprint.0);
    }
    write_canonical_scalar(&mut canonical, "repaired_revision_id");
    write_canonical_scalar(&mut canonical, repaired_revision_id);
    format!(
        "{SCOPE_DIGEST_PREFIX}{}",
        hex::encode(Sha256::digest(canonical))
    )
}

fn canonical_prefix(phase: &str) -> Vec<u8> {
    let mut canonical = Vec::new();
    write_canonical_scalar(&mut canonical, SCOPE_DIGEST_VERSION);
    write_canonical_scalar(&mut canonical, phase);
    canonical
}

fn write_canonical_scalar(canonical: &mut Vec<u8>, value: &str) {
    canonical.extend_from_slice(value.len().to_string().as_bytes());
    canonical.push(b':');
    canonical.extend_from_slice(value.as_bytes());
}

fn is_valid_scope_digest(value: &str) -> bool {
    value
        .strip_prefix(SCOPE_DIGEST_PREFIX)
        .is_some_and(is_lowercase_sha256_hex)
}

// ---- SC 生成教学重驱耐久窗口（REQ-WSC-28B 模型面，bound-sc-generation-teaching）----
/// 批准规则版本（随窗口冻结，规则变更不得回溯改写既有结论）；
/// attempt 键前缀 sc_generation:{session}:{gen}:attempt:{n}。
pub const SC_TEACHING_APPROVAL_RULE_VERSION: &str = "bound-sc-teaching-window-v1";
pub const SC_TEACHING_ATTEMPT_KEY_PREFIX: &str = "sc_generation";

/// 建立教学窗口的绑定事实输入（除 failure_source_ref 外全部必填）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScTeachingWindowRequest {
    /// 判据②：provider 路径合法且具备真实启动观察能力（caller 判定）。
    pub provider_path_admissible: bool,
    /// 初次调用尚无候选 source ⇒ None（显式记录，不伪造候选引用）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_source_ref: Option<String>,
    /// 判据④：失败原文 hash（64 位 sha256 hex）。
    pub failure_source_sha256: String,
    /// 判据④：可定位 typed 事实（非空）。
    #[serde(default)]
    pub failure_typed_kinds: Vec<PlanningFailureKind>,
    pub business_request_digest: String,
    #[serde(default)]
    pub material_digests: Vec<String>,
    pub context_digest: String,
    /// 允许修改闭包（文本描述）。
    pub allowed_edit_closure: String,
    /// 机器指令 digest。
    pub instruction_digest: String,
    /// prompt 构造器引用（如 "build_work_item_plan_compile_reredrive_prompt"）。
    pub prompt_ref: String,
    /// 完整 prompt 的 sha256 hex。
    pub prompt_digest: String,
}

/// 四项判据的可判定拒绝原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScTeachingApprovalRejection {
    NoProvableAuthorization,
    ProviderPathNotAdmissible,
    NoRemainingPhysicalChance,
    IncompleteFailureFacts,
}

/// 教学尝试终态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScTeachingAttemptOutcome {
    Delivered,
    FailedTerminal,
    Stopped,
}

/// 状态机 Claimed → ActualStarted → Terminal；LaunchUnknown 可恢复。
/// KnownNotStarted 不设独立状态：确定未启动保持 Claimed，允许在同一
/// attempt key/原授权下继续同一 claim，不增加机会（design D2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScTeachingAttemptLifecycle {
    Claimed,
    ActualStarted,
    LaunchUnknown,
    Terminal { outcome: ScTeachingAttemptOutcome },
}

/// 窗口关闭原因（attempt 终态对齐映射 Delivered→Delivered、
/// FailedTerminal→Exhausted、Stopped→Stopped）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScTeachingWindowCloseReason {
    Delivered,
    Exhausted,
    Stopped,
}

/// 窗口主状态：Active | Closed{reason}（关闭不删除窗口，保留事实）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScTeachingWindowStatus {
    Active,
    Closed { reason: ScTeachingWindowCloseReason },
}

/// 单次教学尝试的耐久状态：attempt_no ∈ 1|2（1 = 原调用，建立时固化
/// Terminal{FailedTerminal}）；launch_ledger_key =
/// single_candidate_author:{session}:{n}，None = 已 claim 尚未启动。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScTeachingAttemptState {
    pub attempt_no: u8,
    /// sc_generation:{session}:{gen}:attempt:{n}（教学 attempt 身份键）。
    pub attempt_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_ledger_key: Option<String>,
    pub lifecycle: ScTeachingAttemptLifecycle,
}

/// SC 生成链的耐久教学授权窗口（一条 generation 链一条，append-only：
/// 显式 rearm 开新初轮时追加新窗口，已关闭窗口保留事实不删除）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScGenerationTeachingWindow {
    /// = 建立时 windows.len()（锁内派生）。
    pub generation_seq: u32,
    pub authorization_kind: StructuralAuthorizationKind,
    pub original_session_id: String,
    /// 原生成 phase 绑定（建立时为 Generate）。
    pub original_run_phase: SingleCandidatePhase,
    /// RFC3339 UTC。
    pub established_at: String,
    /// 冻结的绑定事实（业务请求/材料/失败 source+hash/context/闭包/指令/prompt）。
    pub request: ScTeachingWindowRequest,
    /// = sha256(business_request_digest ‖ material_digests ‖ context_digest ‖ failure_source_sha256)。
    pub frozen_input_digest: String,
    /// 通用字段：本 change 只定义，值由 explicit-sc-author-effort 填写。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_execution_policy_digest: Option<String>,
    /// 「窗口建立即批准」（owner 裁决 2026-11-09）。
    pub repair_once_approved: bool,
    /// 冻结的批准规则版本。
    pub approval_rule_version: String,
    /// 核验时的原授权引用（建立时前缀最后一个 launch 键）。
    pub approval_checked_against: String,
    /// ∈ {1,2}；approved ⇒ 2，否则 1。
    pub max_attempts: u8,
    #[serde(default)]
    pub attempts: Vec<ScTeachingAttemptState>,
    pub status: ScTeachingWindowStatus,
    /// 本链初轮授权前的前缀非教学条目数快照（gen0 = 0）。
    pub chain_non_teaching_base: usize,
    /// 关闭时快照（供下一条链做 base；saturating 差）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_teaching_total_at_close: Option<usize>,
}

impl ScGenerationTeachingWindow {
    /// 教学 attempt 身份键。
    pub fn attempt_key(session_id: &str, generation_seq: u32, attempt_no: u8) -> String {
        format!(
            "{SC_TEACHING_ATTEMPT_KEY_PREFIX}:{session_id}:{generation_seq}:attempt:{attempt_no}"
        )
    }

    /// 下一可 claim 的 attempt 号；None = 未批准/已关闭/attempts.len() ≥ max。
    pub fn claimable_attempt_no(&self) -> Option<u8> {
        let consumed = self.attempts.len();
        if !self.repair_once_approved
            || matches!(self.status, ScTeachingWindowStatus::Closed { .. })
            || consumed >= self.max_attempts as usize
        {
            return None;
        }
        Some(consumed as u8 + 1)
    }

    /// 冻结输入校验：Err = 漂移（source/context/prompt 任一不一致）。
    /// 签名按计划书 Interfaces 逐字（Err 载荷为 ()，无细节面）。
    #[allow(clippy::result_unit_err)]
    pub fn verify_frozen_inputs(&self, request: &ScTeachingWindowRequest) -> Result<(), ()> {
        if sc_teaching_frozen_input_digest(request) != self.frozen_input_digest
            || request.prompt_ref != self.request.prompt_ref
            || request.prompt_digest != self.request.prompt_digest
        {
            return Err(());
        }
        Ok(())
    }
}

/// 冻结输入摘要（长度前缀规范化拼接后 sha256 hex，防拼接歧义）。
pub(crate) fn sc_teaching_frozen_input_digest(request: &ScTeachingWindowRequest) -> String {
    let mut canonical = Vec::new();
    write_canonical_scalar(&mut canonical, &request.business_request_digest);
    for digest in &request.material_digests {
        write_canonical_scalar(&mut canonical, digest);
    }
    write_canonical_scalar(&mut canonical, &request.context_digest);
    write_canonical_scalar(&mut canonical, &request.failure_source_sha256);
    hex::encode(Sha256::digest(canonical))
}

/// 教学窗口批准判定（四项合取，owner 裁决 D1-a；纯函数，无 IO）。
/// Ok = 原授权引用（建立时前缀最后一个 launch 键）。
pub fn evaluate_sc_teaching_approval(
    record: &WorkspaceSessionRecord,
    request: &ScTeachingWindowRequest,
) -> Result<String, ScTeachingApprovalRejection> {
    // ① single_candidate_phase == Generate 且前缀条目非空（原生成启动可证明）。
    if record.single_candidate_phase != Some(SingleCandidatePhase::Generate) {
        return Err(ScTeachingApprovalRejection::NoProvableAuthorization);
    }
    let prefix = sc_author_launch_prefix(&record.id);
    let original_launch_key = record
        .provider_start_ledger
        .iter()
        .rev()
        .find(|entry| entry.provider_start_idempotency_key.starts_with(&prefix))
        .map(|entry| entry.provider_start_idempotency_key.clone())
        .ok_or(ScTeachingApprovalRejection::NoProvableAuthorization)?;
    // ② provider 路径合法且具备真实启动观察能力。
    if !request.provider_path_admissible {
        return Err(ScTeachingApprovalRejection::ProviderPathNotAdmissible);
    }
    // ③ 本链已消费非教学启动数 = 当前总数 − 上一已关闭窗口快照（无则 0）≤ 1。
    let chain_base = record
        .sc_generation_teaching_windows
        .iter()
        .rev()
        .find(|window| matches!(window.status, ScTeachingWindowStatus::Closed { .. }))
        .map_or(0, |window| window.non_teaching_total_at_close.unwrap_or(0));
    if non_teaching_provider_start_count(record).saturating_sub(chain_base) > 1 {
        return Err(ScTeachingApprovalRejection::NoRemainingPhysicalChance);
    }
    // ④ 失败事实完整。
    if request.failure_source_sha256.is_empty()
        || request.failure_typed_kinds.is_empty()
        || request.context_digest.is_empty()
        || request.prompt_digest.is_empty()
    {
        return Err(ScTeachingApprovalRejection::IncompleteFailureFacts);
    }
    Ok(original_launch_key)
}

/// 前缀 single_candidate_author:{record.id}: 且 provider_attempt_key == None
/// 的条目数（教学启动不占非教学计数）。
pub fn non_teaching_provider_start_count(record: &WorkspaceSessionRecord) -> usize {
    let prefix = sc_author_launch_prefix(&record.id);
    record
        .provider_start_ledger
        .iter()
        .filter(|entry| {
            entry.provider_start_idempotency_key.starts_with(&prefix)
                && entry.provider_attempt_key.is_none()
        })
        .count()
}

/// 1 + 该前缀条目最大数字后缀（无条目 ⇒ 0）；序号单调、与 C 记账解耦。
pub fn next_sc_author_launch_ordinal(record: &WorkspaceSessionRecord) -> usize {
    let prefix = sc_author_launch_prefix(&record.id);
    record
        .provider_start_ledger
        .iter()
        .filter_map(|entry| entry.provider_start_idempotency_key.strip_prefix(&prefix))
        .filter_map(|suffix| suffix.parse::<usize>().ok())
        .max()
        .map_or(0, |max| max + 1)
}

fn sc_author_launch_prefix(session_id: &str) -> String {
    format!("single_candidate_author:{session_id}:")
}

#[cfg(test)]
mod sg4a_launch_ledger_tests {
    use super::*;

    #[test]
    fn sg_04a_actual_launch_requires_started_provider_and_observation_time() {
        // 旧预留占位条目（reserve/attempt 里程经写：started=true、无
        // provider/started_at）不是实际启动证据。
        let legacy_placeholder = ProviderStartLedgerEntry {
            provider_start_idempotency_key: "human_gate:turn_0001:attempt:1".to_string(),
            started: true,
            provider: None,
            started_at: None,
            provider_attempt_key: None,
        };
        assert!(!legacy_placeholder.is_actual_launch_observation());

        // 缺 provider 或缺观察时间同样不算（partial 不冒充完整观察）。
        let without_time = ProviderStartLedgerEntry {
            started_at: None,
            provider: Some("codex".to_string()),
            ..legacy_placeholder.clone()
        };
        assert!(!without_time.is_actual_launch_observation());
        let without_provider = ProviderStartLedgerEntry {
            provider: None,
            started_at: Some("2026-10-13T00:00:00Z".to_string()),
            ..legacy_placeholder.clone()
        };
        assert!(!without_provider.is_actual_launch_observation());

        // 完整登记（真实启动路径带 provider/started_at）才是实际观察。
        let actual = ProviderStartLedgerEntry {
            provider: Some("codex".to_string()),
            started_at: Some("2026-10-13T00:00:00Z".to_string()),
            ..legacy_placeholder
        };
        assert!(actual.is_actual_launch_observation());

        // started=false 永远不是（显式未启动）。
        let not_started = ProviderStartLedgerEntry {
            started: false,
            ..actual
        };
        assert!(!not_started.is_actual_launch_observation());
    }
}
