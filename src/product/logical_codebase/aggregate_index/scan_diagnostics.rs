//! Measured facts for one aggregate-index verification, not an activation or skip policy.
use serde::{Deserialize, Serialize};

use super::source_scan::MemberSourceVerdict;

pub(crate) const SCAN_LIMIT_FACT_PREFIX: &str = "source scan limit reached;";
pub(crate) const SCAN_READ_FACT_PREFIX: &str = "source scan read incomplete;";

/// A bounded discriminator for persisted facts; never export the fact text itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScanFailureFact {
    ScanIncomplete,
    CoverageUnknown,
    LimitReached,
    ReadIncomplete,
}

pub(crate) fn scan_failure_fact(code: &str, detail: &str) -> Option<ScanFailureFact> {
    if code != "aggregate_index_source_scan_incomplete" {
        return None;
    }
    let detail = detail.trim_start();
    if detail.starts_with(SCAN_LIMIT_FACT_PREFIX) {
        Some(ScanFailureFact::LimitReached)
    } else if detail.starts_with(SCAN_READ_FACT_PREFIX) {
        Some(ScanFailureFact::ReadIncomplete)
    } else if detail.starts_with("coverage state unknown for included members [") {
        Some(ScanFailureFact::CoverageUnknown)
    } else if detail.starts_with("source scan incomplete (") {
        Some(ScanFailureFact::ScanIncomplete)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanUnknownReason {
    Deadline,
    BatchEntries,
    MemberEntries,
    Depth,
    Cancelled,
    Permission,
    Io,
    GitTimeout,
    GitFailed,
    OutputTruncated,
    MissingCoverage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberScanEvidence {
    pub member: String,
    pub verdict: MemberSourceVerdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ScanUnknownReason>,
    pub visibility_started: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_ms_at_entry: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceScanEvidence {
    pub member_count: usize,
    pub covered_count: usize,
    pub coverage_elapsed_ms: u64,
    pub scan_elapsed_ms: u64,
    /// Only members not covered (including a missing coverage key), in manifest order.
    pub members: Vec<MemberScanEvidence>,
}
