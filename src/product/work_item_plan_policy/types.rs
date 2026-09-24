use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{FindingFingerprint, is_lowercase_sha256_hex};

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
    pub trigger: super::HumanReason,
    pub resumable: bool,
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
