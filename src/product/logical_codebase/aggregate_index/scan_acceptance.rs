use std::collections::BTreeMap;
use std::path::Path;

use super::AggregateIndexError;
use super::scan_diagnostics::{
    MemberScanEvidence, SCAN_LIMIT_FACT_PREFIX, SCAN_READ_FACT_PREFIX, ScanUnknownReason,
};
use super::source_scan::{
    MemberSourceVerdict, MemberSourceVisibility, SourceScanBudget, scan_member_source,
};

// The order is the existing public failure precedence. A missing coverage key is
// not an uncovered member: never try to infer a verdict by scanning that directory.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Block {
    DiskSource,
    WorktreesOnly,
    ScanUnknown,
    MissingCoverage,
}

fn block(row: &MemberScanEvidence) -> Option<Block> {
    match row.verdict {
        MemberSourceVerdict::SourceInNormalPath => Some(Block::DiskSource),
        MemberSourceVerdict::SourceOnlyInWorktrees => Some(Block::WorktreesOnly),
        MemberSourceVerdict::Unknown if row.reason == Some(ScanUnknownReason::MissingCoverage) => {
            Some(Block::MissingCoverage)
        }
        MemberSourceVerdict::Unknown => Some(Block::ScanUnknown),
        MemberSourceVerdict::NoSource => None,
    }
}

pub(super) fn cross_verify_uncovered(
    root: &Path,
    covered: &BTreeMap<String, bool>,
    names: &[String],
    visibility: &dyn MemberSourceVisibility,
    budget: &mut SourceScanBudget<'_>,
) -> (
    Vec<MemberScanEvidence>,
    Result<Vec<String>, AggregateIndexError>,
) {
    let mut rows = Vec::new();
    for member in names {
        let row = match covered.get(member) {
            Some(true) => continue,
            Some(false) => {
                let remaining_ms_at_entry = budget.remaining_ms();
                let observation = scan_member_source(&root.join(member), visibility, budget);
                MemberScanEvidence {
                    member: member.clone(),
                    verdict: observation.verdict,
                    reason: observation.reason,
                    visibility_started: observation.visibility_started,
                    remaining_ms_at_entry,
                }
            }
            None => MemberScanEvidence {
                member: member.clone(),
                verdict: MemberSourceVerdict::Unknown,
                reason: Some(ScanUnknownReason::MissingCoverage),
                visibility_started: false,
                remaining_ms_at_entry: None,
            },
        };
        rows.push(row);
    }
    let decision = if let Some(kind) = rows.iter().filter_map(block).min() {
        Err(blocked(kind, &rows))
    } else if rows.len() < names.len() {
        Ok(rows.iter().map(|row| row.member.clone()).collect())
    } else {
        Err(AggregateIndexError::Failed {
            code: "aggregate_index_all_members_source_less",
            message: format!(
                "no source found after bounded disk scan under included members [{}]; add a member that contains source",
                names.join(", ")
            ),
        })
    };
    (rows, decision)
}

fn blocked(kind: Block, rows: &[MemberScanEvidence]) -> AggregateIndexError {
    let mut selected = rows
        .iter()
        .filter(|row| block(row) == Some(kind))
        .peekable();
    let first_reason = selected.peek().and_then(|row| row.reason);
    let mut same_reason = true;
    let mut names = String::new();
    for row in selected {
        if !names.is_empty() {
            names.push_str(", ");
        }
        names.push_str(&row.member);
        same_reason &= row.reason == first_reason;
    }
    let (code, message) = match kind {
        Block::DiskSource => (
            "aggregate_index_member_coverage_failed",
            format!(
                "source found on disk; no indexed files under included members [{names}]; check member paths and ignore rules, then rebuild"
            ),
        ),
        Block::WorktreesOnly => (
            "aggregate_index_member_source_excluded",
            format!(
                "source exists only under .worktrees/ for included members [{names}]; move it into the member main checkout"
            ),
        ),
        Block::MissingCoverage => (
            "aggregate_index_source_scan_incomplete",
            format!(
                "coverage state unknown for included members [{names}]; no indexed files and no disk scan evidence"
            ),
        ),
        Block::ScanUnknown => {
            use ScanUnknownReason::*;
            let fact = match first_reason.filter(|_| same_reason) {
                Some(Deadline | Depth | BatchEntries | MemberEntries) => SCAN_LIMIT_FACT_PREFIX,
                Some(Permission | Io | GitFailed | GitTimeout | OutputTruncated) => {
                    SCAN_READ_FACT_PREFIX
                }
                _ => "source scan incomplete (permission/io/depth/entry/time limit)",
            };
            (
                "aggregate_index_source_scan_incomplete",
                format!("{fact} for included members [{names}]; cannot confirm absence of source"),
            )
        }
    };
    AggregateIndexError::Failed { code, message }
}
