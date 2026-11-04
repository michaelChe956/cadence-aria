use serde::{Deserialize, Serialize};

use crate::web::error::ApiError;

/// One of the five stable aggregate initialization step projections. Never
/// reuses the single-repository `git_finalize` step id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AggregateInitializationStepDto {
    pub step_id: String,
    pub status: String,
}

/// A frozen member projection captured by the deterministic preflight step.
/// Distinct from the single-repository registration DTO: it carries the
/// profile evidence digest, not a GitFinalize warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AggregateMemberProjectionDto {
    pub logical_repository_id: String,
    pub checkout_id: String,
    pub revision: String,
    pub dirty: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_digest: Option<String>,
}

/// Aggregate initialization operation DTO. Shows the five stable steps, the
/// resolved profile, per-member evidence projections and the cancellation
/// record. It never carries the single-repository GitFinalize warning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AggregateInitializationOperationDto {
    pub operation_id: String,
    pub project_id: String,
    pub status: String,
    pub profile: Option<String>,
    pub steps: Vec<AggregateInitializationStepDto>,
    pub current_step: Option<String>,
    pub failed_step: Option<String>,
    pub member_projections: Vec<AggregateMemberProjectionDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellation: Option<AggregateCancellationDto>,
    /// The provider selection frozen at operation-creation time, including the
    /// full candidate table in owner order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_selection: Option<AggregateProviderSelectionDto>,
    pub error: Option<ApiError>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// The provider selection carried by an aggregate initialization operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AggregateProviderSelectionDto {
    /// The chosen provider's wire name.
    pub selected: String,
    /// Every candidate provider, in owner order.
    pub candidates: Vec<AggregateProviderCandidateDto>,
    /// The rule version that produced this selection.
    pub rule_version: String,
    /// When the selection was decided.
    pub decided_at: String,
}

/// One candidate row of the provider selection table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AggregateProviderCandidateDto {
    /// The provider's wire name.
    pub provider: String,
    /// The provider's human-facing name.
    pub display_name: String,
    /// Whether the provider's CLI is available on this machine.
    pub cli_available: bool,
    /// Why the CLI is unavailable.
    #[serde(default)]
    pub cli_reason_code: Option<String>,
    /// The launch capability state of this candidate.
    pub capability_state: String,
    /// Why this candidate was skipped.
    #[serde(default)]
    pub skipped_reason: Option<String>,
    /// The executable action for this candidate.
    pub action_hint: String,
    /// A warning about this candidate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AggregateCancellationDto {
    pub reason_code: String,
    pub cancelled_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateAggregateInitializationRequest {
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CancelAggregateInitializationRequest {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation_dto(
        provider_selection: Option<AggregateProviderSelectionDto>,
    ) -> AggregateInitializationOperationDto {
        AggregateInitializationOperationDto {
            operation_id: "op".to_string(),
            project_id: "project_0001".to_string(),
            status: "created".to_string(),
            profile: None,
            steps: Vec::new(),
            current_step: None,
            failed_step: None,
            member_projections: Vec::new(),
            cancellation: None,
            provider_selection,
            error: None,
            created_at: "2026-11-03T00:00:00Z".to_string(),
            updated_at: "2026-11-03T00:00:00Z".to_string(),
            completed_at: None,
        }
    }

    fn selection_dto() -> AggregateProviderSelectionDto {
        AggregateProviderSelectionDto {
            selected: "pi".to_string(),
            candidates: vec![AggregateProviderCandidateDto {
                provider: "pi".to_string(),
                display_name: "Pi".to_string(),
                cli_available: true,
                cli_reason_code: None,
                capability_state: "confirmed".to_string(),
                skipped_reason: None,
                action_hint: "run the capability probe for this provider".to_string(),
                warning: None,
            }],
            rule_version: "v1".to_string(),
            decided_at: "2026-11-03T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn operation_dto_serializes_provider_selection_with_snake_case_keys() {
        let encoded = serde_json::to_value(operation_dto(Some(selection_dto()))).unwrap();
        let selection = encoded
            .get("provider_selection")
            .expect("provider_selection is serialized")
            .clone();
        assert_eq!(selection.get("selected").unwrap(), "pi");
        assert_eq!(selection.get("rule_version").unwrap(), "v1");
        assert_eq!(selection.get("decided_at").unwrap(), "2026-11-03T00:00:00Z");
        let candidate = selection
            .get("candidates")
            .unwrap()
            .as_array()
            .unwrap()
            .first()
            .unwrap()
            .clone();
        assert_eq!(candidate.get("provider").unwrap(), "pi");
        assert_eq!(candidate.get("display_name").unwrap(), "Pi");
        assert_eq!(candidate.get("cli_available").unwrap(), true);
        assert_eq!(candidate.get("capability_state").unwrap(), "confirmed");
        assert_eq!(
            candidate.get("action_hint").unwrap(),
            "run the capability probe for this provider"
        );
        assert!(candidate.get("cli_reason_code").unwrap().is_null());
        assert!(candidate.get("skipped_reason").unwrap().is_null());
        assert!(candidate.get("warning").is_none());
    }

    #[test]
    fn operation_dto_omits_absent_provider_selection() {
        let encoded = serde_json::to_value(operation_dto(None)).unwrap();
        assert!(encoded.get("provider_selection").is_none());
    }

    #[test]
    fn provider_selection_dto_roundtrips() {
        let dto = selection_dto();
        let encoded = serde_json::to_string(&dto).unwrap();
        let decoded: AggregateProviderSelectionDto = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, dto);
    }
}
