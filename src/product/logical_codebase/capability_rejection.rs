//! Capability launch rejection vocabulary and classifier.
//!
//! Provider capability is a three-state fact (`Confirmed` / `Denied` /
//! `Unknown`; see [`ProviderCapabilityEvidence`]). The gateway error出口 used
//! to flatten the three states into one sentence and to embed the `Debug`
//! rendering of the state, so a user could not tell "not verified yet" from
//! "verified as unsupported".
//!
//! This module owns the two machine-readable artifacts block 2 relies on:
//!
//! - [`capability_launch_state_token`]: the stable lowercase token of a launch
//!   cell state. **Never** derived from `Debug` output.
//! - [`classify_capability_launch_rejection`]: maps an already-rendered detail
//!   string back onto the three rejection classes, so the durable error record
//!   can emit a stable machine code without re-deriving the state.
//!
//! Pure functions only: no I/O, no clock, no provider calls.

use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;

/// Machine code emitted when a launch cell was never verified for this
/// logical codebase (the capability record is missing, or the cell is
/// `Unknown`). Registered in the block 2 failure-copy table.
pub const REASON_CODE_CAPABILITY_UNVERIFIED: &str = "aggregate_provider_capability_unverified";

/// Machine code emitted when a capability probe positively rejected the
/// launch cell. Registered in the block 2 failure-copy table.
pub const REASON_CODE_CAPABILITY_DENIED: &str = "aggregate_provider_capability_denied";

/// Machine code emitted when the stored capability evidence no longer matches
/// the local CLI snapshot. Registered in the block 2 failure-copy table.
pub const REASON_CODE_CAPABILITY_EVIDENCE_STALE: &str =
    "aggregate_provider_capability_evidence_stale";

/// The stable lowercase token of a launch capability state.
///
/// The token is the *only* machine-readable form of the three states: the
/// `Debug` rendering is explicitly forbidden here, because it embeds the
/// variant name and the reason inside parentheses (`Denied { reason: "…" }`)
/// and therefore cannot be matched by a stable rule.
pub fn capability_launch_state_token(evidence: &ProviderCapabilityEvidence) -> &'static str {
    match evidence {
        ProviderCapabilityEvidence::Confirmed => "confirmed",
        ProviderCapabilityEvidence::Denied { .. } => "denied",
        ProviderCapabilityEvidence::Unknown => "unknown",
    }
}

/// The three classes of capability-launch rejection a provider turn can hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityLaunchRejection {
    /// The capability was never verified for this logical codebase. Covers
    /// both the `Unknown` launch cell and the missing capability record.
    Unverified {
        /// The session policy action the launch cell was checked for.
        action: String,
    },
    /// A capability probe positively rejected the launch cell.
    Denied {
        /// The session policy action the launch cell was checked for.
        action: String,
        /// The reason the probe gave.
        reason: String,
    },
    /// The stored capability evidence does not match the local CLI snapshot.
    EvidenceStale {
        /// The raw detail, kept verbatim for diagnostics.
        detail: String,
    },
}

/// Detail fragment emitted when the launch cell was never confirmed.
const LAUNCH_CELL_UNKNOWN: &str = "launch cell is unknown";

/// Detail fragment emitted when a capability probe rejected the launch cell.
/// The reason follows the fragment, separated by a single space.
const LAUNCH_CELL_DENIED: &str = "launch cell is denied; reason: ";

/// Detail fragment emitted when the durable capability record is missing.
const CAPABILITY_RECORD_MISSING: &str = "capability record missing";

/// Detail fragment emitted when the capability snapshot drifted.
const CAPABILITY_SNAPSHOT_MISMATCH: &str = "capability snapshot mismatch";

/// Classify an already-rendered provider-turn detail string into one of the
/// three capability-launch rejection classes.
///
/// The decision order is fixed and total:
///
/// 1. `capability snapshot mismatch` → [`CapabilityLaunchRejection::EvidenceStale`]
/// 2. `launch cell is denied; reason: ` → [`CapabilityLaunchRejection::Denied`]
/// 3. `launch cell is unknown` → [`CapabilityLaunchRejection::Unverified`]
/// 4. `capability record missing` → [`CapabilityLaunchRejection::Unverified`]
/// 5. anything else → `None` (the caller keeps its pre-existing machine code)
///
/// `None` is the load-bearing answer: a detail that does not match any
/// capability rule must leave the caller's reason code and action text
/// byte-for-byte unchanged.
pub fn classify_capability_launch_rejection(detail: &str) -> Option<CapabilityLaunchRejection> {
    if detail.contains(CAPABILITY_SNAPSHOT_MISMATCH) {
        return Some(CapabilityLaunchRejection::EvidenceStale {
            detail: detail.to_string(),
        });
    }
    if let Some((_, reason)) = detail.split_once(LAUNCH_CELL_DENIED) {
        return Some(CapabilityLaunchRejection::Denied {
            action: subject_before(detail, LAUNCH_CELL_DENIED),
            reason: reason.trim().to_string(),
        });
    }
    if detail.contains(LAUNCH_CELL_UNKNOWN) {
        return Some(CapabilityLaunchRejection::Unverified {
            action: subject_before(detail, LAUNCH_CELL_UNKNOWN),
        });
    }
    if detail.contains(CAPABILITY_RECORD_MISSING) {
        return Some(CapabilityLaunchRejection::Unverified {
            action: subject_before(detail, CAPABILITY_RECORD_MISSING),
        });
    }
    None
}

/// The subject token that precedes `marker` in `detail`.
///
/// The gateway renders the subject after the stable code and before the
/// marker (`"{code}: {subject} launch cell is unknown"`), so the subject is
/// the last whitespace-separated token of that head, with the `Debug` quotes
/// stripped. An empty string means the detail carried no subject at all.
fn subject_before(detail: &str, marker: &str) -> String {
    let head = match detail.find(marker) {
        Some(index) => &detail[..index],
        None => return String::new(),
    };
    let head = match head.rsplit_once(':') {
        Some((_, tail)) => tail,
        None => head,
    };
    head.split_whitespace()
        .last()
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;

    const LAUNCH_NOT_CONFIRMED: &str = "provider_capability_launch_not_confirmed";

    #[test]
    fn launch_state_token_is_confirmed_denied_unknown() {
        assert_eq!(
            capability_launch_state_token(&ProviderCapabilityEvidence::Confirmed),
            "confirmed"
        );
        assert_eq!(
            capability_launch_state_token(&ProviderCapabilityEvidence::Denied {
                reason: "no resume channel".to_string(),
            }),
            "denied"
        );
        assert_eq!(
            capability_launch_state_token(&ProviderCapabilityEvidence::Unknown),
            "unknown"
        );
    }

    #[test]
    fn launch_state_token_never_leaks_debug_rendering() {
        for evidence in [
            ProviderCapabilityEvidence::Confirmed,
            ProviderCapabilityEvidence::Denied {
                reason: "no resume channel".to_string(),
            },
            ProviderCapabilityEvidence::Unknown,
        ] {
            let token = capability_launch_state_token(&evidence);
            assert!(
                !token.contains('('),
                "token must not embed Debug output: {token}"
            );
            assert!(!token.contains(' '), "token must be a single word: {token}");
            assert!(
                !token.contains('{'),
                "token must not embed Debug output: {token}"
            );
        }
    }

    #[test]
    fn classifier_maps_unknown_launch_cell_to_unverified() {
        let detail = format!("{LAUNCH_NOT_CONFIRMED}: PlanningReadOnly launch cell is unknown");
        assert_eq!(
            classify_capability_launch_rejection(&detail),
            Some(CapabilityLaunchRejection::Unverified {
                action: "PlanningReadOnly".to_string(),
            })
        );
    }

    #[test]
    fn classifier_maps_denied_launch_cell_to_denied_with_reason() {
        let detail = format!(
            "{LAUNCH_NOT_CONFIRMED}: PlanningReadOnly launch cell is denied; reason: no resume channel"
        );
        assert_eq!(
            classify_capability_launch_rejection(&detail),
            Some(CapabilityLaunchRejection::Denied {
                action: "PlanningReadOnly".to_string(),
                reason: "no resume channel".to_string(),
            })
        );
    }

    #[test]
    fn classifier_maps_missing_capability_record_to_unverified() {
        let detail = format!("{LAUNCH_NOT_CONFIRMED}: ClaudeCode capability record missing");
        assert_eq!(
            classify_capability_launch_rejection(&detail),
            Some(CapabilityLaunchRejection::Unverified {
                action: "ClaudeCode".to_string(),
            })
        );
    }

    #[test]
    fn classifier_maps_snapshot_mismatch_to_evidence_stale() {
        let detail = "capability snapshot mismatch".to_string();
        assert_eq!(
            classify_capability_launch_rejection(&detail),
            Some(CapabilityLaunchRejection::EvidenceStale { detail })
        );
    }

    #[test]
    fn classifier_ignores_unrelated_detail() {
        assert_eq!(classify_capability_launch_rejection("unrelated boom"), None);
    }
}
