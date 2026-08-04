#![allow(dead_code)]

pub(crate) struct PromptSegments {
    pub fixed_protocol: String,
    pub identity: String,
    pub unit_records: String,
    pub evidence_digest: String,
    pub graph: String,
    pub diff: String,
    pub retry_diagnostic_reserve: String,
}

impl PromptSegments {
    pub fn join(&self) -> String {
        [
            self.fixed_protocol.as_str(),
            self.identity.as_str(),
            self.unit_records.as_str(),
            self.evidence_digest.as_str(),
            self.graph.as_str(),
            self.diff.as_str(),
            self.retry_diagnostic_reserve.as_str(),
        ]
        .join("")
    }

    pub fn measure(&self) -> PromptBudgetBreakdown {
        let fixed_protocol = self.fixed_protocol.len();
        let identity = self.identity.len();
        let unit_records = self.unit_records.len();
        let evidence_digest = self.evidence_digest.len();
        let graph = self.graph.len();
        let diff = self.diff.len();
        let retry_diagnostic_reserve = self.retry_diagnostic_reserve.len();
        PromptBudgetBreakdown {
            fixed_protocol,
            identity,
            unit_records,
            evidence_digest,
            graph,
            diff,
            retry_diagnostic_reserve,
            total: fixed_protocol
                + identity
                + unit_records
                + evidence_digest
                + graph
                + diff
                + retry_diagnostic_reserve,
        }
    }
}

pub(crate) struct PromptBudgetBreakdown {
    pub fixed_protocol: usize,
    pub identity: usize,
    pub unit_records: usize,
    pub evidence_digest: usize,
    pub graph: usize,
    pub diff: usize,
    pub retry_diagnostic_reserve: usize,
    pub total: usize,
}

pub(crate) struct GroupReviewMaterialSnapshot {
    pub schema_version: u32,
    pub compiler_version: String,
    pub attempt_id: String,
    pub review_request_id: String,
    pub base_branch: String,
    pub final_commit: String,
    pub authoritative_binding_digest: String,
    pub unit_records: Vec<UnitCrossReviewRecord>,
    pub global_graph: GroupReviewGraph,
    pub diff_index: GroupDiffIndex,
    pub deterministic_findings: Vec<DeterministicGroupFinding>,
    pub partition_result: GroupPartitionResult,
    pub content_hash: String,
}

pub(crate) struct UnitCrossReviewRecord {
    pub unit_id: String,
    pub unit_run_id: String,
    pub logical_work_item_id: String,
    pub work_item_revision_id: String,
    pub completion_commit: String,
    pub dependency_ids: Vec<String>,
    pub scope_summary: UnitScopeSummary,
    pub contract_interfaces: Vec<CompactContractInterface>,
    pub evidence_summary: UnitEvidenceSummary,
    pub routing_targets: Vec<CompactRoutingTarget>,
}

pub(crate) struct UnitScopeSummary {
    pub exclusive_scopes: Vec<String>,
    pub forbidden_scopes: Vec<String>,
}

pub(crate) struct CompactContractInterface {
    pub contract_id: String,
    pub direction: String,
    pub capabilities: Vec<String>,
    pub counterparty_unit_run_id: Option<String>,
}

pub(crate) struct UnitEvidenceSummary {
    pub required_command_count: usize,
    pub executed_command_count: usize,
    pub manual_check_count: usize,
    pub missing_refs: Vec<String>,
}

pub(crate) struct CompactRoutingTarget {
    pub reason_code: String,
    pub allowed_route: String,
    pub target_contract_refs: Vec<String>,
}

pub(crate) struct GroupReviewGraph {
    pub contract_edges: Vec<ContractEdge>,
    pub scope_overlaps: Vec<ScopeOverlap>,
    pub commit_reachability: CommitReachability,
    pub requirement_coverage: RequirementCoverage,
}

pub(crate) struct ContractEdge {
    pub contract_id: String,
    pub producer_unit_run_id: String,
    pub consumer_unit_run_ids: Vec<String>,
    pub matched: bool,
}

pub(crate) struct ScopeOverlap {
    pub file_path: String,
    pub unit_run_ids: Vec<String>,
    pub forbidden_hit: bool,
}

pub(crate) struct CommitReachability {
    pub reachable_completion_commits: Vec<String>,
    pub unreachable_completion_commits: Vec<String>,
}

pub(crate) struct RequirementCoverage {
    pub covered: Vec<String>,
    pub missing: Vec<String>,
    pub conflicting: Vec<String>,
}

pub(crate) struct GroupDiffIndex {
    pub files: Vec<DiffFileEntry>,
}

pub(crate) struct DiffFileEntry {
    pub path: String,
    pub insertions: u32,
    pub deletions: u32,
    pub owner_unit_run_ids: Vec<String>,
    pub shared: bool,
    pub ambiguous: bool,
    pub forbidden_scope_hit: bool,
}

pub(crate) struct DeterministicGroupFinding {
    pub kind: String,
    pub related_unit_run_ids: Vec<String>,
    pub detail: String,
}

pub(crate) struct GroupPartitionResult {
    pub shards: Vec<GroupShardSpec>,
    pub cross_shard_edges: Vec<CrossShardEdge>,
}

pub(crate) struct GroupShardSpec {
    pub shard_id: String,
    pub ordered_unit_run_ids: Vec<String>,
    pub partition_rationale: Vec<String>,
}

pub(crate) struct CrossShardEdge {
    pub edge_kind: String,
    pub from_unit_run_id: String,
    pub to_unit_run_id: String,
    pub detail: String,
}

pub(crate) struct GroupGitFacts {
    pub diff_stat: String,
    pub completion_diffs: Vec<CompletionDiff>,
    pub final_diff: String,
}

pub(crate) struct CompletionDiff {
    pub unit_run_id: String,
    pub base_commit: String,
    pub completion_commit: String,
    pub patch: String,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GroupMaterialError {
    #[error("authority_validation_failed: {0}")]
    AuthorityValidation(String),
    #[error("git_fact_error: {0}")]
    GitFact(String),
    #[error("internal: {0}")]
    Internal(String),
}
