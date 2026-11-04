//! Structured audit-snapshot budget failure payload.
//!
//! This module exists as its own small module so `crate::product::json_store`
//! can carry the failure payload without depending on
//! `crate::product::logical_codebase` (that would be a reverse dependency:
//! the logical-codebase domain already depends on the store).
//!
//! The payload is pure data: the budget accounting itself lives in the
//! root-recipe receipt store (block 2 slice 6); this module only fixes the
//! *shape* of the failure that crosses the store boundary.
//!
//! Accounting rules owned by the caller (slice 6), fixed here so the shape
//! can carry them:
//!
//! - `cumulative_bytes` is the number of bytes read at the moment the budget
//!   was exceeded (it is therefore `> max_bytes`).
//! - `cumulative_entries` counts the paths that entered the change list and
//!   were charged.
//! - `top_groups` / `member_summaries` only aggregate the *change list*, and
//!   a `.git` subtree never appears in them (the whole `.git` subtree is
//!   skipped before charging).

use serde::{Deserialize, Serialize};

/// Machine code emitted when the audit snapshot exceeded its budget.
/// Registered in the block 2 failure-copy table.
pub const AUDIT_BUDGET_EXCEEDED_REASON_CODE: &str = "aggregate_audit_budget_exceeded";

/// One aggregated spend group of the audit snapshot, e.g. `pms/target` or
/// `root/AGENTS.md`. Groups are sorted by bytes, descending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetGroup {
    /// The group key: `<member directory>/<first-level directory or file>`.
    pub key: String,
    /// The bytes charged to this group.
    pub bytes: u64,
}

/// The contribution of one member repository to the audit snapshot spend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberBudgetSummary {
    /// The member repository name.
    pub member: String,
    /// The bytes charged to this member.
    pub bytes: u64,
    /// The entries charged to this member.
    pub entries: usize,
}

/// The payload of `ProductStoreError::AuditSnapshotBudgetExceeded`.
///
/// It carries the budget limits, what was actually charged, where the budget
/// was crossed (for locating only — not "this file is oversized"), and the
/// per-group / per-member breakdown the failure copy renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditSnapshotBudgetFailure {
    /// The byte budget.
    pub max_bytes: u64,
    /// The bytes charged when the budget was crossed.
    pub cumulative_bytes: u64,
    /// The path being processed when the budget was crossed (locating only).
    pub over_line_path: String,
    /// The entry budget.
    pub max_entries: usize,
    /// The entries charged when the budget was crossed.
    pub cumulative_entries: usize,
    /// The biggest spend groups, bytes descending. Never contains a `.git` key.
    pub top_groups: Vec<BudgetGroup>,
    /// The per-member contribution.
    pub member_summaries: Vec<MemberBudgetSummary>,
}

impl std::fmt::Display for AuditSnapshotBudgetFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "audit_snapshot_budget_exceeded: {}/{} bytes at {}",
            self.cumulative_bytes, self.max_bytes, self.over_line_path
        )
    }
}

impl AuditSnapshotBudgetFailure {
    /// The machine-readable diagnostic of this budget failure.
    ///
    /// It is the single place that renders the budget breakdown, so the
    /// durable error record can carry it verbatim (the failure copy renders
    /// the Chinese narrative from the machine code, while this line is the
    /// raw evidence the user can read in the diagnostics area).
    ///
    /// Shape (one line per section, `\n`-joined, empty sections omitted):
    ///
    /// ```text
    /// audit_snapshot_budget_exceeded: 67108864/67108864 bytes, 3/20000 entries, over at pms/target/app.jar
    /// top_groups: pms/target=4096 bytes, busi/target=4096 bytes
    /// member_summaries: pms=4096 bytes/3 entries, busi=4096 bytes/2 entries
    /// ```
    pub fn diagnostic(&self) -> String {
        let mut lines = vec![format!(
            "audit_snapshot_budget_exceeded: {}/{} bytes, {}/{} entries, over at {}",
            self.cumulative_bytes,
            self.max_bytes,
            self.cumulative_entries,
            self.max_entries,
            self.over_line_path
        )];
        if !self.top_groups.is_empty() {
            lines.push(format!(
                "top_groups: {}",
                self.top_groups
                    .iter()
                    .map(|group| format!("{}={} bytes", group.key, group.bytes))
                    .collect::<Vec<String>>()
                    .join(", ")
            ));
        }
        if !self.member_summaries.is_empty() {
            lines.push(format!(
                "member_summaries: {}",
                self.member_summaries
                    .iter()
                    .map(|member| {
                        format!(
                            "{}={} bytes/{} entries",
                            member.member, member.bytes, member.entries
                        )
                    })
                    .collect::<Vec<String>>()
                    .join(", ")
            ));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure() -> AuditSnapshotBudgetFailure {
        AuditSnapshotBudgetFailure {
            max_bytes: 100,
            cumulative_bytes: 120,
            over_line_path: "pms/target/lib.rs".to_string(),
            max_entries: 10,
            cumulative_entries: 12,
            top_groups: vec![BudgetGroup {
                key: "pms/target".to_string(),
                bytes: 90,
            }],
            member_summaries: vec![MemberBudgetSummary {
                member: "pms".to_string(),
                bytes: 90,
                entries: 3,
            }],
        }
    }

    #[test]
    fn display_reports_cumulative_over_limit_and_path() {
        assert_eq!(
            failure().to_string(),
            "audit_snapshot_budget_exceeded: 120/100 bytes at pms/target/lib.rs"
        );
    }

    #[test]
    fn budget_failure_roundtrips_through_json() {
        let encoded = serde_json::to_string(&failure()).unwrap();
        let decoded: AuditSnapshotBudgetFailure = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, failure());
        assert!(encoded.contains("\"over_line_path\""));
        assert!(encoded.contains("\"top_groups\""));
        assert!(encoded.contains("\"member_summaries\""));
    }

    #[test]
    fn reason_code_constant_is_the_registered_machine_code() {
        assert_eq!(
            AUDIT_BUDGET_EXCEEDED_REASON_CODE,
            "aggregate_audit_budget_exceeded"
        );
    }

    #[test]
    fn diagnostic_renders_budget_groups_and_member_summaries() {
        let failure = AuditSnapshotBudgetFailure {
            max_bytes: 67_108_864,
            cumulative_bytes: 67_108_864,
            over_line_path: "pms/target/app.jar".to_string(),
            max_entries: 20_000,
            cumulative_entries: 3,
            top_groups: vec![
                BudgetGroup {
                    key: "pms/target".to_string(),
                    bytes: 4096,
                },
                BudgetGroup {
                    key: "busi/target".to_string(),
                    bytes: 4096,
                },
            ],
            member_summaries: vec![
                MemberBudgetSummary {
                    member: "pms".to_string(),
                    bytes: 4096,
                    entries: 3,
                },
                MemberBudgetSummary {
                    member: "busi".to_string(),
                    bytes: 4096,
                    entries: 2,
                },
            ],
        };
        assert_eq!(
            failure.diagnostic(),
            "audit_snapshot_budget_exceeded: 67108864/67108864 bytes, 3/20000 entries, over at pms/target/app.jar\n\
             top_groups: pms/target=4096 bytes, busi/target=4096 bytes\n\
             member_summaries: pms=4096 bytes/3 entries, busi=4096 bytes/2 entries"
        );
    }

    #[test]
    fn diagnostic_omits_empty_sections() {
        let failure = AuditSnapshotBudgetFailure {
            max_bytes: 100,
            cumulative_bytes: 120,
            over_line_path: "pms/target/lib.rs".to_string(),
            max_entries: 10,
            cumulative_entries: 12,
            top_groups: Vec::new(),
            member_summaries: Vec::new(),
        };
        assert_eq!(
            failure.diagnostic(),
            "audit_snapshot_budget_exceeded: 120/100 bytes, 12/10 entries, over at pms/target/lib.rs"
        );
    }
}
