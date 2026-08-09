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
}

impl AggregateInitializationStepStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
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
}

impl AggregateInitializationErrorRecord {
    pub fn interrupted() -> Self {
        Self {
            stage: "aggregate_initialization".to_string(),
            reason_code: "aggregate_initialization_interrupted".to_string(),
            stderr_summary: None,
            retryable: true,
            action: "服务在聚合初始化完成前中断；检查可能的部分修改后重新提交".to_string(),
        }
    }
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
        }
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
