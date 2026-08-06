#![allow(dead_code)]

use super::group_review_types::{PromptBudgetBreakdown, PromptSegments};

pub(crate) const GROUP_REVIEW_QUALITY_TARGET_BYTES: usize = 28 * 1024;
pub(crate) const GROUP_REVIEW_HARD_CAP_BYTES: usize = 30 * 1024;
pub(crate) const GROUP_REVIEW_CAPACITY_LIMIT: usize = 20;
pub(crate) const REPAIR_PROMPT_BYTE_CAP: usize = 16 * 1024;
pub(crate) const GROUP_REVIEW_SHARD_MAX_FINDINGS: usize = 8;
pub(crate) const GROUP_REVIEW_REDUCTION_MAX_FINDINGS: usize = 16;
pub(crate) const GROUP_REVIEW_DEFAULT_CONCURRENCY: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BudgetDecision {
    Send,
    SendWithWarning,
    Overflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BudgetConfigurationError {
    QualityTargetNotBelowHardCap,
    HardCapExceedsMaximum,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapacityDecision {
    WithinCapacity,
    CapacityExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindingsDecision {
    WithinLimit,
    FindingsExceeded,
}

impl PromptSegments {
    pub(crate) fn join(&self) -> String {
        [
            self.fixed_protocol.as_str(),
            self.identity.as_str(),
            self.routing_authority.as_str(),
            self.unit_records.as_str(),
            self.evidence_digest.as_str(),
            self.graph.as_str(),
            self.diff.as_str(),
            self.retry_diagnostic_reserve.as_str(),
        ]
        .join("")
    }

    pub(crate) fn measure(&self) -> PromptBudgetBreakdown {
        let fixed_protocol = self.fixed_protocol.len();
        let identity = self.identity.len();
        let routing_authority = self.routing_authority.len();
        let unit_records = self.unit_records.len();
        let evidence_digest = self.evidence_digest.len();
        let graph = self.graph.len();
        let diff = self.diff.len();
        let retry_diagnostic_reserve = self.retry_diagnostic_reserve.len();
        PromptBudgetBreakdown {
            fixed_protocol,
            identity,
            routing_authority,
            unit_records,
            evidence_digest,
            graph,
            diff,
            retry_diagnostic_reserve,
            total: fixed_protocol
                + identity
                + routing_authority
                + unit_records
                + evidence_digest
                + graph
                + diff
                + retry_diagnostic_reserve,
        }
    }
}

pub(crate) fn decide_budget(total_bytes: usize, quality_target: usize) -> BudgetDecision {
    if total_bytes > GROUP_REVIEW_HARD_CAP_BYTES {
        BudgetDecision::Overflow
    } else if total_bytes <= quality_target {
        BudgetDecision::Send
    } else {
        BudgetDecision::SendWithWarning
    }
}

pub(crate) fn validate_budget_configuration(
    quality_target: usize,
    hard_cap: usize,
) -> Result<(), BudgetConfigurationError> {
    if hard_cap > GROUP_REVIEW_HARD_CAP_BYTES {
        return Err(BudgetConfigurationError::HardCapExceedsMaximum);
    }
    if quality_target >= hard_cap {
        return Err(BudgetConfigurationError::QualityTargetNotBelowHardCap);
    }
    Ok(())
}

pub(crate) fn check_capacity(unit_count: usize) -> CapacityDecision {
    if unit_count > GROUP_REVIEW_CAPACITY_LIMIT {
        CapacityDecision::CapacityExceeded
    } else {
        CapacityDecision::WithinCapacity
    }
}

pub(crate) fn check_shard_findings(finding_count: usize) -> FindingsDecision {
    check_findings(finding_count, GROUP_REVIEW_SHARD_MAX_FINDINGS)
}

pub(crate) fn check_reduction_findings(finding_count: usize) -> FindingsDecision {
    check_findings(finding_count, GROUP_REVIEW_REDUCTION_MAX_FINDINGS)
}

fn check_findings(finding_count: usize, limit: usize) -> FindingsDecision {
    if finding_count > limit {
        FindingsDecision::FindingsExceeded
    } else {
        FindingsDecision::WithinLimit
    }
}

/// Reads `GROUP_REVIEW_SHARD_CONCURRENCY`; values below one or invalid values use the default.
pub(crate) fn group_review_shard_concurrency() -> usize {
    concurrency_from_value(
        std::env::var("GROUP_REVIEW_SHARD_CONCURRENCY")
            .ok()
            .as_deref(),
    )
}

fn concurrency_from_value(value: Option<&str>) -> usize {
    value
        .and_then(|value| value.parse().ok())
        .filter(|&value| value >= 1)
        .unwrap_or(GROUP_REVIEW_DEFAULT_CONCURRENCY)
}
