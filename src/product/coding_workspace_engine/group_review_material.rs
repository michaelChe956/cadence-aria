#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use sha2::{Digest, Sha256};

use super::group_review_types::*;
use super::plan_defect_routing::AuthoritativeGroupReviewerBinding;
use crate::cross_cutting::worktree::scope_allows_path;
use crate::product::coding_evaluation_context::redact_sensitive_patterns;
use crate::product::coding_models::{ReviewRequest, UnitReviewConclusionSnapshot};
use crate::product::work_item_contract::ContractCompatibilityPolicy;

const COMPILER_VERSION: &str = "group-review-material-compiler-v1";
const SHARD_DIFF_BUDGET: usize = 10_500;
const REDUCTION_DIFF_BUDGET: usize = 8_000;
const MAX_UNIT_RECORD_BYTES: usize = 850;

#[derive(serde::Serialize)]
pub(crate) struct GroupReviewMaterialSnapshotDraft {
    pub(crate) schema_version: u32,
    pub(crate) compiler_version: String,
    pub(crate) attempt_id: String,
    pub(crate) review_request_id: String,
    pub(crate) base_branch: String,
    pub(crate) final_commit: String,
    pub(crate) authoritative_binding_digest: String,
    pub(crate) unit_records: Vec<UnitCrossReviewRecord>,
    pub(crate) global_graph: GroupReviewGraph,
    pub(crate) diff_index: GroupDiffIndex,
    pub(crate) deterministic_findings: Vec<DeterministicGroupFinding>,
    pub(crate) partition_result: GroupPartitionResult,
    pub(crate) content_hash: String,
}

pub(crate) trait ShardPromptMeasurer {
    fn measure_shard(
        &self,
        snapshot: &GroupReviewMaterialSnapshotDraft,
        shard: &GroupShardSpec,
    ) -> usize;
}

pub(crate) fn compile_group_review_material(
    bindings: &[AuthoritativeGroupReviewerBinding],
    snapshots: &[UnitReviewConclusionSnapshot],
    review_request: &ReviewRequest,
    git_facts: &GroupGitFacts,
    measurer: &dyn ShardPromptMeasurer,
    quality_target_bytes: usize,
) -> Result<GroupReviewMaterialSnapshot, GroupMaterialError> {
    let bindings = sorted_bindings(bindings)?;
    validate_authority(&bindings, snapshots, review_request)?;
    let snapshot_by_run = snapshots
        .iter()
        .map(|snapshot| (snapshot.unit_run_id.as_str(), snapshot))
        .collect::<BTreeMap<_, _>>();

    let (diff_index, completion_paths) = build_diff_index(&bindings, git_facts);
    let (global_graph, mut findings) =
        deterministic_checks(&bindings, &snapshot_by_run, &completion_paths, git_facts);
    let mut unit_records = Vec::with_capacity(bindings.len());
    for binding in &bindings {
        unit_records.push(compact_record(
            binding,
            snapshot_by_run[&binding.run.id.as_str()],
        )?);
    }
    unit_records.sort_by(|left, right| left.unit_run_id.cmp(&right.unit_run_id));

    let affinity_edges = affinity_edges(&bindings, &diff_index);
    let mut partition_result = initial_partition(&bindings, &affinity_edges);
    let mut draft = GroupReviewMaterialSnapshotDraft {
        schema_version: 1,
        compiler_version: COMPILER_VERSION.to_string(),
        attempt_id: review_request.attempt_id.clone(),
        review_request_id: review_request.id.clone(),
        base_branch: review_request.base_branch.clone(),
        final_commit: review_request.commit_sha.clone(),
        authoritative_binding_digest: binding_digest(&bindings)?,
        unit_records,
        global_graph,
        diff_index,
        deterministic_findings: Vec::new(),
        partition_result: GroupPartitionResult {
            shards: Vec::new(),
            cross_shard_edges: Vec::new(),
        },
        content_hash: String::new(),
    };
    repartition_for_budget(
        &mut partition_result,
        &affinity_edges,
        &draft,
        measurer,
        quality_target_bytes,
    );
    draft.partition_result = partition_result;
    draft.diff_index.shard_selections = shard_selections(&draft, &bindings, SHARD_DIFF_BUDGET);
    draft.diff_index.reduction_selection =
        reduction_selection(&draft, &bindings, REDUCTION_DIFF_BUDGET);
    findings.sort_by(finding_order);
    draft.deterministic_findings = findings;
    finalize(draft)
}

fn sorted_bindings(
    bindings: &[AuthoritativeGroupReviewerBinding],
) -> Result<Vec<&AuthoritativeGroupReviewerBinding>, GroupMaterialError> {
    let mut sorted = bindings.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| order_key(left).cmp(&order_key(right)));
    let mut ids = BTreeSet::new();
    if sorted
        .iter()
        .any(|binding| !ids.insert(binding.run.id.as_str()))
    {
        return Err(GroupMaterialError::AuthorityValidation(
            "duplicate_unit_run_binding".to_string(),
        ));
    }
    Ok(sorted)
}

fn order_key(binding: &AuthoritativeGroupReviewerBinding) -> (u32, &str) {
    (binding.order_index, binding.run.unit_id.as_str())
}

fn validate_authority(
    bindings: &[&AuthoritativeGroupReviewerBinding],
    snapshots: &[UnitReviewConclusionSnapshot],
    review_request: &ReviewRequest,
) -> Result<(), GroupMaterialError> {
    let runs = bindings
        .iter()
        .map(|binding| binding.run.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for snapshot in snapshots {
        if snapshot.attempt_id != review_request.attempt_id
            || !runs.contains(snapshot.unit_run_id.as_str())
        {
            return Err(GroupMaterialError::AuthorityValidation(
                "snapshot_binding_mismatch".to_string(),
            ));
        }
        if !seen.insert(snapshot.unit_run_id.as_str()) {
            return Err(GroupMaterialError::AuthorityValidation(
                "duplicate_unit_review_snapshot".to_string(),
            ));
        }
    }
    if seen.len() != runs.len() {
        return Err(GroupMaterialError::AuthorityValidation(
            "unit_review_snapshot_missing".to_string(),
        ));
    }
    Ok(())
}

fn binding_digest(
    bindings: &[&AuthoritativeGroupReviewerBinding],
) -> Result<String, GroupMaterialError> {
    let values = bindings
        .iter()
        .map(|binding| {
            format!(
                "{}:{}:{}:{}",
                binding.order_index,
                binding.run.id,
                binding.run.work_item_revision_id,
                binding.run.canonical_contract_hash
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(sha256(&values))
}

fn compact_record(
    binding: &AuthoritativeGroupReviewerBinding,
    snapshot: &UnitReviewConclusionSnapshot,
) -> Result<UnitCrossReviewRecord, GroupMaterialError> {
    let projection = &binding.projection_binding.projection;
    let mut dependencies = projection
        .input_contract_checks
        .iter()
        .map(|input| input.contract_id.clone())
        .collect::<Vec<_>>();
    let mut contracts =
        projection
            .input_contract_checks
            .iter()
            .map(|input| CompactContractInterface {
                contract_id: input.contract_id.clone(),
                direction: "input".to_string(),
                capabilities: sorted_unique(input.required_capabilities.clone()),
                counterparty_unit_run_id: None,
            })
            .chain(projection.output_contract_checks.iter().map(|output| {
                CompactContractInterface {
                    contract_id: output.contract_id.clone(),
                    direction: "output".to_string(),
                    capabilities: sorted_unique(output.capabilities.clone()),
                    counterparty_unit_run_id: None,
                }
            }))
            .collect::<Vec<_>>();
    dependencies = sorted_unique(dependencies);
    contracts.sort_by(|left, right| {
        left.contract_id
            .cmp(&right.contract_id)
            .then(left.direction.cmp(&right.direction))
    });
    let mut record = UnitCrossReviewRecord {
        unit_id: binding.run.unit_id.clone(),
        unit_run_id: binding.run.id.clone(),
        logical_work_item_id: binding.projection_binding.logical_work_item_id.clone(),
        work_item_revision_id: binding.run.work_item_revision_id.clone(),
        completion_commit: binding.run.completion_commit.clone().unwrap_or_default(),
        dependency_ids: dependencies,
        scope_summary: UnitScopeSummary {
            exclusive_scopes: sorted_unique(projection.scope_policy.exclusive_scopes.clone()),
            forbidden_scopes: sorted_unique(projection.scope_policy.forbidden_scopes.clone()),
        },
        contract_interfaces: contracts,
        evidence_summary: UnitEvidenceSummary {
            required_command_count: projection
                .verification_evidence_rules
                .iter()
                .filter(|check| check.required && check.command.is_some())
                .count(),
            executed_command_count: snapshot
                .evidence_refs
                .iter()
                .filter(|value| !value.trim().is_empty())
                .count(),
            manual_check_count: projection
                .verification_evidence_rules
                .iter()
                .filter(|check| check.required && check.manual_instruction.is_some())
                .count(),
            missing_refs: Vec::new(),
        },
        routing_targets: projection
            .blocker_routing
            .iter()
            .map(|rule| CompactRoutingTarget {
                reason_code: rule.reason_code.clone(),
                allowed_route: format!("{:?}", rule.route),
                target_contract_refs: sorted_unique(rule.target_contract_refs.clone()),
            })
            .collect(),
    };
    trim_record(&mut record)?;
    Ok(record)
}

fn trim_record(record: &mut UnitCrossReviewRecord) -> Result<(), GroupMaterialError> {
    while serde_json::to_vec(&*record).map_or(0, |bytes| bytes.len()) > MAX_UNIT_RECORD_BYTES {
        if record.contract_interfaces.pop().is_some()
            || record.routing_targets.pop().is_some()
            || record.dependency_ids.pop().is_some()
            || record.scope_summary.exclusive_scopes.pop().is_some()
            || record.scope_summary.forbidden_scopes.pop().is_some()
        {
            continue;
        }
        return Err(GroupMaterialError::Internal(
            "unit_cross_review_record_exceeds_850_bytes".to_string(),
        ));
    }
    Ok(())
}

fn deterministic_checks(
    bindings: &[&AuthoritativeGroupReviewerBinding],
    snapshots: &BTreeMap<&str, &UnitReviewConclusionSnapshot>,
    completion_paths: &BTreeMap<String, BTreeSet<String>>,
    git_facts: &GroupGitFacts,
) -> (GroupReviewGraph, Vec<DeterministicGroupFinding>) {
    let mut findings = Vec::new();
    let mut outputs =
        BTreeMap::<(String, String), (&AuthoritativeGroupReviewerBinding, Vec<String>)>::new();
    for binding in bindings {
        for output in &binding.projection_binding.projection.output_contract_checks {
            outputs.insert(
                (
                    binding.projection_binding.logical_work_item_id.clone(),
                    output.contract_id.clone(),
                ),
                (*binding, sorted_unique(output.capabilities.clone())),
            );
        }
    }
    let mut contract_edges = Vec::new();
    for binding in bindings {
        for input in &binding.projection_binding.projection.input_contract_checks {
            let (producer, _provided, matched) = outputs
                .get(&(
                    input.provider_logical_work_item_id.clone(),
                    input.contract_id.clone(),
                ))
                .map(|(producer, provided)| {
                    let matching = match input.compatibility_policy {
                        ContractCompatibilityPolicy::RequireAll => input
                            .required_capabilities
                            .iter()
                            .all(|value| provided.contains(value)),
                        ContractCompatibilityPolicy::RequireAny => input
                            .required_capabilities
                            .iter()
                            .any(|value| provided.contains(value)),
                    };
                    (*producer, provided.clone(), matching)
                })
                .unwrap_or((binding, Vec::new(), false));
            let producer_id = producer.run.id.clone();
            contract_edges.push(ContractEdge {
                contract_id: input.contract_id.clone(),
                producer_unit_run_id: producer_id.clone(),
                consumer_unit_run_ids: vec![binding.run.id.clone()],
                matched,
            });
            if !matched {
                findings.push(finding(
                    "contract_missing_or_capability_mismatch",
                    vec![producer_id, binding.run.id.clone()],
                    &input.contract_id,
                ));
            }
        }
    }
    contract_edges.sort_by(|left, right| {
        left.contract_id
            .cmp(&right.contract_id)
            .then(left.producer_unit_run_id.cmp(&right.producer_unit_run_id))
    });

    let mut scope_overlaps = Vec::new();
    for (path, owners) in completion_paths {
        let forbidden = bindings.iter().any(|binding| {
            binding
                .projection_binding
                .projection
                .scope_policy
                .forbidden_scopes
                .iter()
                .any(|scope| scope_allows_path(scope, path, true))
        });
        if forbidden {
            findings.push(finding(
                "forbidden_scope_hit",
                owners.iter().cloned().collect(),
                path,
            ));
        }
        if owners.len() > 1 || forbidden {
            scope_overlaps.push(ScopeOverlap {
                file_path: path.clone(),
                unit_run_ids: owners.iter().cloned().collect(),
                forbidden_hit: forbidden,
            });
        }
    }
    scope_overlaps.sort_by(|left, right| left.file_path.cmp(&right.file_path));

    let known_commits = git_facts
        .completion_commit_in_final
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut reachable = Vec::new();
    let mut unreachable = Vec::new();
    for binding in bindings {
        match binding.run.completion_commit.as_deref() {
            Some(commit) if known_commits.contains(commit) || commit == git_facts.final_commit => {
                reachable.push(commit.to_string())
            }
            Some(commit) => {
                unreachable.push(commit.to_string());
                findings.push(finding(
                    "completion_commit_unreachable",
                    vec![binding.run.id.clone()],
                    commit,
                ));
            }
            None => findings.push(finding(
                "completion_commit_missing",
                vec![binding.run.id.clone()],
                "missing",
            )),
        }
        let snapshot = snapshots[&binding.run.id.as_str()];
        if snapshot
            .evidence_refs
            .iter()
            .any(|reference| reference.trim().is_empty())
        {
            findings.push(finding(
                "evidence_attribution_missing",
                vec![binding.run.id.clone()],
                "empty_evidence_ref",
            ));
        }
    }
    reachable.sort();
    unreachable.sort();

    let mut requirement_owners = BTreeMap::<String, BTreeSet<String>>::new();
    for binding in bindings {
        for check in &binding.projection_binding.projection.requirement_matrix {
            for requirement in &check.requirement_refs {
                requirement_owners
                    .entry(requirement.clone())
                    .or_default()
                    .insert(binding.run.id.clone());
            }
        }
    }
    let mut covered = Vec::new();
    let mut conflicting = Vec::new();
    for (requirement, owners) in requirement_owners {
        if owners.len() > 1 {
            conflicting.push(requirement);
        } else {
            covered.push(requirement);
        }
    }
    for requirement in &conflicting {
        findings.push(finding(
            "requirement_coverage_ambiguous",
            Vec::new(),
            requirement,
        ));
    }
    (
        GroupReviewGraph {
            contract_edges,
            scope_overlaps,
            commit_reachability: CommitReachability {
                reachable_completion_commits: reachable,
                unreachable_completion_commits: unreachable,
            },
            requirement_coverage: RequirementCoverage {
                covered,
                missing: Vec::new(),
                conflicting,
            },
        },
        findings,
    )
}

fn finding(kind: &str, mut ids: Vec<String>, detail: &str) -> DeterministicGroupFinding {
    ids.sort();
    ids.dedup();
    DeterministicGroupFinding {
        kind: kind.to_string(),
        related_unit_run_ids: ids,
        detail: detail.to_string(),
    }
}
fn finding_order(
    left: &DeterministicGroupFinding,
    right: &DeterministicGroupFinding,
) -> std::cmp::Ordering {
    left.kind
        .cmp(&right.kind)
        .then(left.detail.cmp(&right.detail))
        .then(left.related_unit_run_ids.cmp(&right.related_unit_run_ids))
}

fn affinity_edges(
    bindings: &[&AuthoritativeGroupReviewerBinding],
    diff_index: &GroupDiffIndex,
) -> Vec<CrossShardEdge> {
    let mut edges = BTreeSet::new();
    for (index, left) in bindings.iter().enumerate() {
        for right in bindings.iter().skip(index + 1) {
            let left_projection = &left.projection_binding.projection;
            let right_projection = &right.projection_binding.projection;
            let handoff = left_projection.output_contract_checks.iter().any(|output| {
                right_projection
                    .input_contract_checks
                    .iter()
                    .any(|input| input.contract_id == output.contract_id)
            }) || right_projection
                .output_contract_checks
                .iter()
                .any(|output| {
                    left_projection
                        .input_contract_checks
                        .iter()
                        .any(|input| input.contract_id == output.contract_id)
                });
            let contract = contract_ids(left_projection)
                .iter()
                .any(|left_contract| contract_ids(right_projection).contains(left_contract));
            let shared = diff_index.files.iter().any(|file| {
                file.owner_unit_run_ids.contains(&left.run.id)
                    && file.owner_unit_run_ids.contains(&right.run.id)
            });
            for (present, kind) in [
                (handoff, "handoff_dependency"),
                (shared, "shared_file"),
                (contract, "contract_boundary"),
            ] {
                if present {
                    let (from, to) = if left.run.id <= right.run.id {
                        (left.run.id.clone(), right.run.id.clone())
                    } else {
                        (right.run.id.clone(), left.run.id.clone())
                    };
                    edges.insert((kind.to_string(), from, to));
                }
            }
        }
    }
    edges
        .into_iter()
        .map(
            |(edge_kind, from_unit_run_id, to_unit_run_id)| CrossShardEdge {
                detail: edge_kind.clone(),
                edge_kind,
                from_unit_run_id,
                to_unit_run_id,
            },
        )
        .collect()
}

fn contract_ids(
    projection: &crate::product::work_item_projection::ReviewerWorkItemProjection,
) -> BTreeSet<String> {
    projection
        .input_contract_checks
        .iter()
        .map(|contract| contract.contract_id.clone())
        .chain(
            projection
                .output_contract_checks
                .iter()
                .map(|contract| contract.contract_id.clone()),
        )
        .collect()
}

fn initial_partition(
    bindings: &[&AuthoritativeGroupReviewerBinding],
    edges: &[CrossShardEdge],
) -> GroupPartitionResult {
    let ids = bindings
        .iter()
        .map(|binding| binding.run.id.clone())
        .collect::<Vec<_>>();
    let keys = bindings
        .iter()
        .map(|binding| (binding.run.id.clone(), order_key(binding)))
        .collect::<BTreeMap<_, _>>();
    let mut adjacent = BTreeMap::<String, BTreeSet<String>>::new();
    for id in &ids {
        adjacent.entry(id.clone()).or_default();
    }
    for edge in edges {
        adjacent
            .entry(edge.from_unit_run_id.clone())
            .or_default()
            .insert(edge.to_unit_run_id.clone());
        adjacent
            .entry(edge.to_unit_run_id.clone())
            .or_default()
            .insert(edge.from_unit_run_id.clone());
    }
    let mut visited = BTreeSet::new();
    let mut components = Vec::new();
    for id in &ids {
        if !visited.insert(id.clone()) {
            continue;
        }
        let mut queue = VecDeque::from([id.clone()]);
        let mut component = Vec::new();
        while let Some(current) = queue.pop_front() {
            component.push(current.clone());
            for next in &adjacent[&current] {
                if visited.insert(next.clone()) {
                    queue.push_back(next.clone());
                }
            }
        }
        component.sort_by(|left, right| keys[left].cmp(&keys[right]));
        components.push(component);
    }
    components.sort_by(|left, right| keys[&left[0]].cmp(&keys[&right[0]]));
    let mut groups = Vec::<Vec<String>>::new();
    let mut current = Vec::new();
    for component in components {
        for piece in component.chunks(4) {
            if current.len() + piece.len() > 4 {
                groups.push(std::mem::take(&mut current));
            }
            current.extend(piece.iter().cloned());
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    partition_from_groups(groups, edges)
}

fn partition_from_groups(
    groups: Vec<Vec<String>>,
    edges: &[CrossShardEdge],
) -> GroupPartitionResult {
    let shards = groups
        .into_iter()
        .enumerate()
        .map(|(index, ids)| GroupShardSpec {
            shard_id: format!("group_shard_{:04}", index + 1),
            ordered_unit_run_ids: ids,
            partition_rationale: vec!["deterministic_affinity_partition".to_string()],
        })
        .collect::<Vec<_>>();
    let locations = shards
        .iter()
        .flat_map(|shard| {
            shard
                .ordered_unit_run_ids
                .iter()
                .map(move |id| (id.as_str(), shard.shard_id.as_str()))
        })
        .collect::<BTreeMap<_, _>>();
    let mut cross = edges
        .iter()
        .filter(|edge| {
            locations[edge.from_unit_run_id.as_str()] != locations[edge.to_unit_run_id.as_str()]
        })
        .cloned()
        .collect::<Vec<_>>();
    cross.sort_by(|left, right| {
        left.edge_kind
            .cmp(&right.edge_kind)
            .then(left.from_unit_run_id.cmp(&right.from_unit_run_id))
            .then(left.to_unit_run_id.cmp(&right.to_unit_run_id))
    });
    GroupPartitionResult {
        shards,
        cross_shard_edges: cross,
    }
}

fn repartition_for_budget(
    partition: &mut GroupPartitionResult,
    edges: &[CrossShardEdge],
    draft: &GroupReviewMaterialSnapshotDraft,
    measurer: &dyn ShardPromptMeasurer,
    target: usize,
) {
    let mut groups = partition
        .shards
        .iter()
        .map(|shard| shard.ordered_unit_run_ids.clone())
        .collect::<Vec<_>>();
    let mut index = 0;
    while index < groups.len() {
        let candidate = GroupShardSpec {
            shard_id: format!("budget_probe_{index}"),
            ordered_unit_run_ids: groups[index].clone(),
            partition_rationale: Vec::new(),
        };
        if groups[index].len() > 1 && measurer.measure_shard(draft, &candidate) > target {
            let split_at = groups[index].len().div_ceil(2);
            let right = groups[index].split_off(split_at);
            groups.insert(index + 1, right);
        } else {
            index += 1;
        }
    }
    *partition = partition_from_groups(groups, edges);
}

fn build_diff_index(
    bindings: &[&AuthoritativeGroupReviewerBinding],
    facts: &GroupGitFacts,
) -> (GroupDiffIndex, BTreeMap<String, BTreeSet<String>>) {
    let mut path_owners = BTreeMap::<String, BTreeSet<String>>::new();
    let mut stats = BTreeMap::<String, (u32, u32)>::new();
    let mut hunks = Vec::new();
    let mut completion_diffs = facts.completion_diffs.iter().collect::<Vec<_>>();
    completion_diffs.sort_by(|left, right| left.unit_run_id.cmp(&right.unit_run_id));
    for diff in completion_diffs {
        let parsed = if diff.hunks.is_empty() {
            parse_hunks(&diff.patch, &diff.unit_run_id)
        } else {
            diff.hunks.clone()
        };
        let parsed_stats = if diff.file_stats.is_empty() {
            stats_from_patch(&diff.patch)
        } else {
            diff.file_stats.clone()
        };
        for stat in parsed_stats {
            let entry = stats.entry(stat.path.clone()).or_default();
            entry.0 += stat.insertions;
            entry.1 += stat.deletions;
            path_owners
                .entry(stat.path)
                .or_default()
                .insert(diff.unit_run_id.clone());
        }
        for mut hunk in parsed {
            let raw_hunk = format!("{}\n{}", hunk.header, hunk.body);
            let redacted_hunk = redact_sensitive_patterns(&raw_hunk);
            hunk.redacted = hunk.redacted || redacted_hunk != raw_hunk;
            let (header, body) = split_redacted_hunk(&redacted_hunk);
            hunk.header = header;
            hunk.body = body;
            hunks.push(hunk);
        }
    }
    for mut hunk in parse_hunks(&facts.final_diff, "") {
        let raw_hunk = format!("{}\n{}", hunk.header, hunk.body);
        let redacted_hunk = redact_sensitive_patterns(&raw_hunk);
        hunk.redacted = redacted_hunk != raw_hunk;
        let (header, body) = split_redacted_hunk(&redacted_hunk);
        hunk.header = header;
        hunk.body = body;
        stats.entry(hunk.path.clone()).or_default();
        path_owners.entry(hunk.path.clone()).or_default();
        hunks.push(hunk);
    }
    let mut files = stats
        .into_iter()
        .map(|(path, (insertions, deletions))| {
            let owners = path_owners.get(&path).cloned().unwrap_or_default();
            let ambiguous = owners.len() != 1;
            let forbidden = bindings.iter().any(|binding| {
                binding
                    .projection_binding
                    .projection
                    .scope_policy
                    .forbidden_scopes
                    .iter()
                    .any(|scope| scope_allows_path(scope, &path, true))
            });
            DiffFileEntry {
                path,
                insertions,
                deletions,
                owner_unit_run_ids: owners.iter().cloned().collect(),
                shared: owners.len() >= 2,
                ambiguous,
                forbidden_scope_hit: forbidden,
            }
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    hunks.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.hunk_index.cmp(&right.hunk_index))
            .then(left.owner_unit_run_ids.cmp(&right.owner_unit_run_ids))
            .then(left.content_hash.cmp(&right.content_hash))
    });
    (
        GroupDiffIndex {
            files,
            hunks,
            shard_selections: Vec::new(),
            reduction_selection: ReductionDiffSelection {
                fragments: Vec::new(),
                total_cross_shard_hunks: 0,
            },
        },
        path_owners,
    )
}

fn split_redacted_hunk(value: &str) -> (String, String) {
    let (header, body) = value.split_once('\n').unwrap_or((value, ""));
    (header.to_string(), body.to_string())
}

fn parse_hunks(patch: &str, owner: &str) -> Vec<DiffHunk> {
    let mut path = String::new();
    let mut header: Option<String> = None;
    let mut body = String::new();
    let mut result = Vec::new();
    let mut index = 0;
    let flush = |result: &mut Vec<DiffHunk>,
                 path: &str,
                 header: &mut Option<String>,
                 body: &mut String,
                 index: &mut usize| {
        if let Some(header) = header.take() {
            let raw = format!("{header}\n{body}");
            result.push(DiffHunk {
                hunk_index: *index,
                path: path.to_string(),
                owner_unit_run_ids: if owner.is_empty() {
                    Vec::new()
                } else {
                    vec![owner.to_string()]
                },
                header,
                body: std::mem::take(body),
                redacted: false,
                content_hash: sha256(&raw),
            });
            *index += 1;
        }
    };
    for line in patch.lines() {
        if let Some(value) = line.strip_prefix("diff --git a/") {
            flush(&mut result, &path, &mut header, &mut body, &mut index);
            path = value.split(" b/").nth(1).unwrap_or(value).to_string();
        } else if line.starts_with("@@") {
            flush(&mut result, &path, &mut header, &mut body, &mut index);
            header = Some(line.to_string());
        } else if header.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    flush(&mut result, &path, &mut header, &mut body, &mut index);
    result
}
fn stats_from_patch(patch: &str) -> Vec<DiffFileStat> {
    let mut values = BTreeMap::<String, (u32, u32)>::new();
    for hunk in parse_hunks(patch, "") {
        let entry = values.entry(hunk.path).or_default();
        for line in hunk.body.lines() {
            if line.starts_with('+') && !line.starts_with("+++") {
                entry.0 += 1;
            }
            if line.starts_with('-') && !line.starts_with("---") {
                entry.1 += 1;
            }
        }
    }
    values
        .into_iter()
        .map(|(path, (insertions, deletions))| DiffFileStat {
            path,
            insertions,
            deletions,
        })
        .collect()
}

fn diff_level(file: &DiffFileEntry, bindings: &[&AuthoritativeGroupReviewerBinding]) -> char {
    if file.forbidden_scope_hit {
        'A'
    } else if file.ambiguous {
        'B'
    } else if file.shared {
        'C'
    } else if file.owner_unit_run_ids.iter().any(|id| {
        bindings.iter().any(|binding| {
            binding.run.id == *id
                && (!binding
                    .projection_binding
                    .projection
                    .input_contract_checks
                    .is_empty()
                    || !binding
                        .projection_binding
                        .projection
                        .output_contract_checks
                        .is_empty())
                && binding
                    .projection_binding
                    .projection
                    .scope_policy
                    .exclusive_scopes
                    .iter()
                    .any(|scope| scope_allows_path(scope, &file.path, true))
        })
    }) {
        'D'
    } else {
        'E'
    }
}

fn shard_selections(
    draft: &GroupReviewMaterialSnapshotDraft,
    bindings: &[&AuthoritativeGroupReviewerBinding],
    budget: usize,
) -> Vec<ShardDiffSelection> {
    draft
        .partition_result
        .shards
        .iter()
        .map(|shard| {
            let members = shard.ordered_unit_run_ids.iter().collect::<BTreeSet<_>>();
            let files = draft
                .diff_index
                .files
                .iter()
                .filter(|file| {
                    file.owner_unit_run_ids
                        .iter()
                        .any(|id| members.contains(&id))
                        || file.ambiguous
                })
                .collect::<Vec<_>>();
            ShardDiffSelection {
                shard_id: shard.shard_id.clone(),
                fragments: select_fragments(
                    &files,
                    &draft.diff_index.hunks,
                    bindings,
                    budget,
                    true,
                ),
                total_hunks_in_shard: draft
                    .diff_index
                    .hunks
                    .iter()
                    .filter(|hunk| {
                        hunk.owner_unit_run_ids
                            .iter()
                            .any(|id| members.contains(&id))
                    })
                    .count(),
            }
        })
        .collect()
}
fn reduction_selection(
    draft: &GroupReviewMaterialSnapshotDraft,
    bindings: &[&AuthoritativeGroupReviewerBinding],
    budget: usize,
) -> ReductionDiffSelection {
    let locations = draft
        .partition_result
        .shards
        .iter()
        .flat_map(|shard| {
            shard
                .ordered_unit_run_ids
                .iter()
                .map(move |id| (id.as_str(), shard.shard_id.as_str()))
        })
        .collect::<BTreeMap<_, _>>();
    let files = draft
        .diff_index
        .files
        .iter()
        .filter(|file| {
            let level = diff_level(file, bindings);
            matches!(level, 'A' | 'B')
                || matches!(level, 'C' | 'D')
                    && file
                        .owner_unit_run_ids
                        .iter()
                        .filter_map(|id| locations.get(id.as_str()))
                        .collect::<BTreeSet<_>>()
                        .len()
                        > 1
        })
        .collect::<Vec<_>>();
    let fragments = select_fragments(&files, &draft.diff_index.hunks, bindings, budget, false);
    ReductionDiffSelection {
        total_cross_shard_hunks: fragments.len(),
        fragments,
    }
}
fn file_header(file: &DiffFileEntry) -> String {
    format!(
        "path: {}\nstat: +{} -{}\n",
        file.path, file.insertions, file.deletions
    )
}

fn select_fragments(
    files: &[&DiffFileEntry],
    hunks: &[DiffHunk],
    bindings: &[&AuthoritativeGroupReviewerBinding],
    budget: usize,
    include_e_headers: bool,
) -> Vec<SelectedDiffFragment> {
    let mut files = files.to_vec();
    files.sort_by(|left, right| {
        diff_level(left, bindings)
            .cmp(&diff_level(right, bindings))
            .then(left.path.cmp(&right.path))
    });
    let mut remaining = budget;
    let mut result = Vec::new();
    for file in files {
        let level = diff_level(file, bindings);
        if level == 'E' && !include_e_headers {
            continue;
        }
        let header = file_header(file);
        let header_body = utf8_prefix(&header, remaining).to_string();
        let header_truncated = header_body.len() < header.len();
        remaining = remaining.saturating_sub(header_body.len());
        let candidates = hunks
            .iter()
            .filter(|hunk| hunk.path == file.path)
            .collect::<Vec<_>>();
        if level == 'E' {
            result.push(SelectedDiffFragment {
                path: file.path.clone(),
                level,
                body: header_body,
                hunk_content_hash: String::new(),
                redacted: false,
                truncated: header_truncated,
                not_shown_count: candidates.len(),
            });
            continue;
        }
        for (index, hunk) in candidates.iter().enumerate() {
            let text = format!("{}\n{}", hunk.header, hunk.body);
            let was_redacted = hunk.redacted;
            let allowed = utf8_prefix(&text, remaining);
            let truncated = allowed.len() < text.len();
            remaining = remaining.saturating_sub(allowed.len());
            let body = if index == 0 {
                format!("{header_body}{allowed}")
            } else {
                allowed.to_string()
            };
            result.push(SelectedDiffFragment {
                path: file.path.clone(),
                level,
                body,
                hunk_content_hash: hunk.content_hash.clone(),
                redacted: was_redacted,
                truncated,
                not_shown_count: candidates.len().saturating_sub(index + 1),
            });
            if truncated || remaining == 0 {
                break;
            }
        }
    }
    result
}
fn utf8_prefix(value: &str, max: usize) -> &str {
    if value.len() <= max {
        return value;
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn finalize(
    draft: GroupReviewMaterialSnapshotDraft,
) -> Result<GroupReviewMaterialSnapshot, GroupMaterialError> {
    let value = serde_json::to_vec(&draft_for_hash(&draft))
        .map_err(|error| GroupMaterialError::Internal(error.to_string()))?;
    Ok(GroupReviewMaterialSnapshot {
        schema_version: draft.schema_version,
        compiler_version: draft.compiler_version,
        attempt_id: draft.attempt_id,
        review_request_id: draft.review_request_id,
        base_branch: draft.base_branch,
        final_commit: draft.final_commit,
        authoritative_binding_digest: draft.authoritative_binding_digest,
        unit_records: draft.unit_records,
        global_graph: draft.global_graph,
        diff_index: draft.diff_index,
        deterministic_findings: draft.deterministic_findings,
        partition_result: draft.partition_result,
        content_hash: hex::encode(Sha256::digest(value)),
    })
}
fn draft_for_hash(draft: &GroupReviewMaterialSnapshotDraft) -> &GroupReviewMaterialSnapshotDraft {
    draft
}
fn sha256(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn sorted_unique(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}
