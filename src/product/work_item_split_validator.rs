use std::collections::{HashMap, HashSet};

use crate::cross_cutting::worktree::scopes_may_overlap;
use crate::product::models::{
    IssueWorkItemPlan, LifecycleWorkItemRecord, RepositoryProfile, RepositoryProfileConfidence,
    VerificationCommandSafety, VerificationCommandSource, VerificationPlan, WorkItemDraftCandidate,
    WorkItemKind, WorkItemOutline, WorkItemPlanOutline, WorkItemSplitFinding,
    WorkItemSplitFindingSeverity,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemSplitValidationReport {
    pub findings: Vec<WorkItemSplitFinding>,
}

impl WorkItemSplitValidationReport {
    pub fn has_errors(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == WorkItemSplitFindingSeverity::Error)
    }
}

pub struct WorkItemSplitValidator;

impl WorkItemSplitValidator {
    pub fn validate(
        plan: &IssueWorkItemPlan,
        work_items: &[LifecycleWorkItemRecord],
        repository_profile: Option<&RepositoryProfile>,
        verification_plans: &[VerificationPlan],
    ) -> WorkItemSplitValidationReport {
        let mut findings = Vec::new();
        validate_plan_membership(plan, work_items, &mut findings);
        validate_dependencies(plan, work_items, &mut findings);
        validate_scopes_and_budgets(plan, work_items, &mut findings);
        validate_semantics(plan, work_items, &mut findings);
        validate_verification_plans(
            plan,
            work_items,
            repository_profile,
            verification_plans,
            &mut findings,
        );
        WorkItemSplitValidationReport { findings }
    }
}

pub struct WorkItemPlanOutlineValidator;

impl WorkItemPlanOutlineValidator {
    pub fn validate(outline: &WorkItemPlanOutline) -> WorkItemSplitValidationReport {
        let mut findings = Vec::new();
        validate_outline_ids(outline, &mut findings);
        validate_outline_traceability_and_scopes(outline, &mut findings);
        let edges = validate_outline_dependencies(outline, &mut findings);
        validate_outline_dependency_cycles(outline, &edges, &mut findings);
        validate_outline_scope_conflicts(outline, &edges, &mut findings);
        WorkItemSplitValidationReport { findings }
    }
}

pub struct WorkItemDraftLocalValidator;

impl WorkItemDraftLocalValidator {
    pub fn validate(
        current: &WorkItemDraftCandidate,
        accepted_dependencies: &[WorkItemDraftCandidate],
        current_outline: &WorkItemOutline,
    ) -> WorkItemSplitValidationReport {
        let mut findings = Vec::new();
        validate_draft_matches_outline(current, current_outline, &mut findings);
        validate_draft_scopes(current, &mut findings);
        validate_draft_verification_plan(current, &mut findings);
        validate_draft_direct_dependency_scopes(current, accepted_dependencies, &mut findings);
        WorkItemSplitValidationReport { findings }
    }
}

fn validate_draft_matches_outline(
    current: &WorkItemDraftCandidate,
    current_outline: &WorkItemOutline,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    if current.outline_id != current_outline.outline_id {
        findings.push(error(
            "draft_outline_mismatch",
            format!(
                "draft outline_id {} does not match current outline {}",
                current.outline_id, current_outline.outline_id
            ),
            vec![
                current.outline_id.clone(),
                current_outline.outline_id.clone(),
            ],
        ));
    }
}

fn validate_draft_scopes(
    current: &WorkItemDraftCandidate,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    if current.exclusive_write_scopes.is_empty() {
        findings.push(error(
            "write_scope_required",
            format!(
                "draft {} must include at least one exclusive_write_scope",
                current.outline_id
            ),
            vec![current.outline_id.clone()],
        ));
    }
}

fn validate_draft_verification_plan(
    current: &WorkItemDraftCandidate,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    let defined_gate_ids = draft_verification_defined_gate_ids(&current.verification_plan);
    let required_gates = current
        .verification_plan
        .get("required_gates")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    for gate in required_gates {
        let Some(gate_id) = gate.as_str() else {
            findings.push(error(
                "verification_required_gate_invalid",
                format!(
                    "draft {} has a non-string required gate",
                    current.outline_id
                ),
                vec![current.outline_id.clone()],
            ));
            continue;
        };
        if !defined_gate_ids.contains(gate_id) {
            findings.push(error(
                "verification_required_gate_missing",
                format!(
                    "draft {} requires gate {} which is not defined in its verification_plan",
                    current.outline_id, gate_id
                ),
                vec![current.outline_id.clone()],
            ));
        }
    }
}

fn draft_verification_defined_gate_ids(plan: &serde_json::Value) -> HashSet<String> {
    let mut ids = HashSet::new();
    for field in ["commands", "manual_checks"] {
        if let Some(items) = plan.get(field).and_then(|value| value.as_array()) {
            for item in items {
                if let Some(id) = item.get("id").and_then(|value| value.as_str()) {
                    ids.insert(id.to_string());
                }
            }
        }
    }
    ids
}

fn validate_draft_direct_dependency_scopes(
    current: &WorkItemDraftCandidate,
    accepted_dependencies: &[WorkItemDraftCandidate],
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    let direct_dependency_ids: HashSet<&str> = current
        .depends_on_outline_ids
        .iter()
        .map(String::as_str)
        .collect();
    for dependency in accepted_dependencies {
        if !direct_dependency_ids.contains(dependency.outline_id.as_str()) {
            continue;
        }
        if scopes_may_overlap(
            &current.exclusive_write_scopes,
            &dependency.exclusive_write_scopes,
            true,
        ) {
            findings.push(error(
                "direct_dependency_scope_conflict",
                format!(
                    "draft {} overlaps exclusive write scopes with direct dependency {}",
                    current.outline_id, dependency.outline_id
                ),
                vec![current.outline_id.clone(), dependency.outline_id.clone()],
            ));
        }
    }
}

fn validate_outline_ids(outline: &WorkItemPlanOutline, findings: &mut Vec<WorkItemSplitFinding>) {
    let mut seen = HashSet::new();
    let mut duplicated = HashSet::new();
    for item in &outline.work_item_outlines {
        if !seen.insert(item.outline_id.as_str()) {
            duplicated.insert(item.outline_id.clone());
        }
    }

    for outline_id in duplicated {
        findings.push(error(
            "duplicate_outline_id",
            format!("outline id {outline_id} is duplicated"),
            vec![outline_id],
        ));
    }
}

fn validate_outline_traceability_and_scopes(
    outline: &WorkItemPlanOutline,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    for item in &outline.work_item_outlines {
        if item.source_story_spec_ids.is_empty() || item.source_design_spec_ids.is_empty() {
            findings.push(error(
                "traceability_required",
                format!(
                    "outline {} must reference at least one story spec and design spec",
                    item.outline_id
                ),
                vec![item.outline_id.clone()],
            ));
        }
        if item.goal.trim().is_empty() {
            findings.push(error(
                "outline_goal_required",
                format!("outline {} must include a goal", item.outline_id),
                vec![item.outline_id.clone()],
            ));
        }
        if item.scope.is_empty() {
            findings.push(error(
                "outline_scope_required",
                format!("outline {} must include a scope", item.outline_id),
                vec![item.outline_id.clone()],
            ));
        }
        if item.exclusive_write_scopes.is_empty() {
            findings.push(error(
                "write_scope_required",
                format!(
                    "outline {} must include at least one exclusive_write_scope",
                    item.outline_id
                ),
                vec![item.outline_id.clone()],
            ));
        }
    }
}

fn validate_outline_dependencies(
    outline: &WorkItemPlanOutline,
    findings: &mut Vec<WorkItemSplitFinding>,
) -> HashSet<(String, String)> {
    let outline_ids: HashSet<&str> = outline
        .work_item_outlines
        .iter()
        .map(|item| item.outline_id.as_str())
        .collect();
    let mut edges = HashSet::new();

    for item in &outline.work_item_outlines {
        for dependency in &item.depends_on {
            if !outline_ids.contains(dependency.as_str()) {
                findings.push(error(
                    "dependency_not_in_outline",
                    format!(
                        "outline {} depends on {} which is not present",
                        item.outline_id, dependency
                    ),
                    vec![item.outline_id.clone(), dependency.clone()],
                ));
                continue;
            }
            edges.insert((dependency.clone(), item.outline_id.clone()));
        }
    }

    for edge in &outline.dependency_graph {
        if !outline_ids.contains(edge.from_outline_id.as_str()) {
            findings.push(error(
                "dependency_not_in_outline",
                format!(
                    "dependency graph references missing from_outline_id {}",
                    edge.from_outline_id
                ),
                vec![edge.from_outline_id.clone()],
            ));
        }
        if !outline_ids.contains(edge.to_outline_id.as_str()) {
            findings.push(error(
                "dependency_not_in_outline",
                format!(
                    "dependency graph references missing to_outline_id {}",
                    edge.to_outline_id
                ),
                vec![edge.to_outline_id.clone()],
            ));
        }
        edges.insert((edge.from_outline_id.clone(), edge.to_outline_id.clone()));
    }

    edges
}

fn validate_outline_dependency_cycles(
    outline: &WorkItemPlanOutline,
    edges: &HashSet<(String, String)>,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for (from, to) in edges {
        adjacency
            .entry(from.as_str())
            .or_default()
            .push(to.as_str());
    }

    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    let mut cycle_found = false;

    fn dfs<'a>(
        node: &'a str,
        adjacency: &HashMap<&'a str, Vec<&'a str>>,
        visiting: &mut HashSet<&'a str>,
        visited: &mut HashSet<&'a str>,
        cycle_found: &mut bool,
    ) {
        if *cycle_found {
            return;
        }
        visiting.insert(node);
        if let Some(neighbors) = adjacency.get(node) {
            for neighbor in neighbors {
                if visiting.contains(*neighbor) {
                    *cycle_found = true;
                    return;
                }
                if !visited.contains(neighbor) {
                    dfs(neighbor, adjacency, visiting, visited, cycle_found);
                }
            }
        }
        visiting.remove(node);
        visited.insert(node);
    }

    for item in &outline.work_item_outlines {
        if !visited.contains(item.outline_id.as_str()) {
            dfs(
                item.outline_id.as_str(),
                &adjacency,
                &mut visiting,
                &mut visited,
                &mut cycle_found,
            );
        }
    }

    if cycle_found {
        findings.push(error(
            "dependency_cycle",
            "outline dependency graph contains a cycle",
            Vec::new(),
        ));
    }
}

fn validate_outline_scope_conflicts(
    outline: &WorkItemPlanOutline,
    edges: &HashSet<(String, String)>,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    let reachability = compute_reachability(edges);

    for (i, left) in outline.work_item_outlines.iter().enumerate() {
        for right in outline.work_item_outlines.iter().skip(i + 1) {
            if !scopes_may_overlap(
                &left.exclusive_write_scopes,
                &right.exclusive_write_scopes,
                true,
            ) {
                continue;
            }

            let left_reaches_right = reachability
                .get(&left.outline_id)
                .map(|set| set.contains(&right.outline_id))
                .unwrap_or(false);
            let right_reaches_left = reachability
                .get(&right.outline_id)
                .map(|set| set.contains(&left.outline_id))
                .unwrap_or(false);
            if left_reaches_right || right_reaches_left {
                findings.push(error(
                    "write_scope_conflict",
                    format!(
                        "dependent outlines {} and {} have overlapping exclusive write scopes",
                        left.outline_id, right.outline_id
                    ),
                    vec![left.outline_id.clone(), right.outline_id.clone()],
                ));
            } else {
                findings.push(error(
                    "parallel_scope_overlap",
                    format!(
                        "outlines {} and {} have no dependency ordering but their write scopes overlap",
                        left.outline_id, right.outline_id
                    ),
                    vec![left.outline_id.clone(), right.outline_id.clone()],
                ));
            }
        }
    }
}

fn error(
    code: &str,
    message: impl Into<String>,
    work_item_ids: Vec<String>,
) -> WorkItemSplitFinding {
    WorkItemSplitFinding {
        severity: WorkItemSplitFindingSeverity::Error,
        code: code.to_string(),
        message: message.into(),
        work_item_ids,
    }
}

fn warning(
    code: &str,
    message: impl Into<String>,
    work_item_ids: Vec<String>,
) -> WorkItemSplitFinding {
    WorkItemSplitFinding {
        severity: WorkItemSplitFindingSeverity::Warning,
        code: code.to_string(),
        message: message.into(),
        work_item_ids,
    }
}

fn validate_plan_membership(
    plan: &IssueWorkItemPlan,
    work_items: &[LifecycleWorkItemRecord],
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    let plan_ids: HashSet<&str> = plan.work_item_ids.iter().map(String::as_str).collect();

    for item in work_items {
        if !plan_ids.contains(item.id.as_str()) {
            findings.push(error(
                "work_item_not_in_plan",
                format!(
                    "work item {} is not listed in IssueWorkItemPlan.work_item_ids",
                    item.id
                ),
                vec![item.id.clone()],
            ));
        }
    }
}

fn validate_dependencies(
    plan: &IssueWorkItemPlan,
    work_items: &[LifecycleWorkItemRecord],
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    let plan_ids: HashSet<&str> = plan.work_item_ids.iter().map(String::as_str).collect();
    let item_by_id: HashMap<&str, &LifecycleWorkItemRecord> = work_items
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect();

    // Edges derived from work_item.depends_on (source of truth): dep -> item.
    let mut derived_edges: HashSet<(String, String)> = HashSet::new();
    for item in work_items {
        for dep in &item.depends_on {
            if !plan_ids.contains(dep.as_str()) {
                findings.push(error(
                    "dependency_not_in_plan",
                    format!(
                        "work item {} depends on {} which is not in IssueWorkItemPlan.work_item_ids",
                        item.id, dep
                    ),
                    vec![item.id.clone(), dep.clone()],
                ));
                continue;
            }
            derived_edges.insert((dep.clone(), item.id.clone()));
        }
    }

    // Edges declared in plan.dependency_graph must also be within the plan.
    let mut declared_edges: HashSet<(String, String)> = HashSet::new();
    for edge in &plan.dependency_graph {
        if !plan_ids.contains(edge.from_work_item_id.as_str()) {
            findings.push(error(
                "dependency_not_in_plan",
                format!(
                    "dependency graph references from_work_item_id {} which is not in plan",
                    edge.from_work_item_id
                ),
                vec![edge.from_work_item_id.clone()],
            ));
        }
        if !plan_ids.contains(edge.to_work_item_id.as_str()) {
            findings.push(error(
                "dependency_not_in_plan",
                format!(
                    "dependency graph references to_work_item_id {} which is not in plan",
                    edge.to_work_item_id
                ),
                vec![edge.to_work_item_id.clone()],
            ));
        }
        declared_edges.insert((edge.from_work_item_id.clone(), edge.to_work_item_id.clone()));
    }

    // Consistency: edges derived from work_item.depends_on must exactly match
    // edges declared in IssueWorkItemPlan.dependency_graph.
    let mut mismatched_edges: Vec<&(String, String)> = declared_edges
        .symmetric_difference(&derived_edges)
        .collect();
    mismatched_edges.sort();
    for edge in mismatched_edges {
        findings.push(error(
            "dependency_graph_mismatch",
            format!(
                "dependency graph mismatch: edge {} -> {} differs between IssueWorkItemPlan.dependency_graph and work_item.depends_on",
                edge.0, edge.1
            ),
            vec![edge.0.clone(), edge.1.clone()],
        ));
    }

    // Cycle detection on the derived graph.
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for (from, to) in &derived_edges {
        adjacency
            .entry(from.as_str())
            .or_default()
            .push(to.as_str());
    }

    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    let mut cycle_found = false;

    fn dfs<'a>(
        node: &'a str,
        adjacency: &HashMap<&'a str, Vec<&'a str>>,
        visiting: &mut HashSet<&'a str>,
        visited: &mut HashSet<&'a str>,
        cycle_found: &mut bool,
    ) {
        if *cycle_found {
            return;
        }
        visiting.insert(node);
        if let Some(neighbors) = adjacency.get(node) {
            for neighbor in neighbors {
                if visiting.contains(*neighbor) {
                    *cycle_found = true;
                    return;
                }
                if !visited.contains(neighbor) {
                    dfs(neighbor, adjacency, visiting, visited, cycle_found);
                }
            }
        }
        visiting.remove(node);
        visited.insert(node);
    }

    for item in work_items {
        if !visited.contains(item.id.as_str()) {
            dfs(
                item.id.as_str(),
                &adjacency,
                &mut visiting,
                &mut visited,
                &mut cycle_found,
            );
        }
    }

    if cycle_found {
        findings.push(error(
            "dependency_cycle",
            "dependency graph derived from work_item.depends_on contains a cycle",
            Vec::new(),
        ));
    }

    // Stash reachability for later scope checks.
    let reachability = compute_reachability(&derived_edges);
    validate_scope_conflicts(plan, work_items, &item_by_id, &reachability, findings);
}

fn compute_reachability(edges: &HashSet<(String, String)>) -> HashMap<String, HashSet<String>> {
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for (from, to) in edges {
        adjacency.entry(from.clone()).or_default().push(to.clone());
    }

    let nodes: HashSet<String> = edges
        .iter()
        .flat_map(|(from, to)| [from.clone(), to.clone()])
        .collect();

    let mut reachability: HashMap<String, HashSet<String>> = HashMap::new();
    for node in &nodes {
        let mut reachable = HashSet::new();
        let mut stack: Vec<String> = adjacency.get(node).cloned().unwrap_or_default();
        while let Some(current) = stack.pop() {
            if reachable.insert(current.clone())
                && let Some(neighbors) = adjacency.get(&current)
            {
                for neighbor in neighbors {
                    stack.push(neighbor.clone());
                }
            }
        }
        reachability.insert(node.clone(), reachable);
    }
    reachability
}

fn validate_scope_conflicts(
    plan: &IssueWorkItemPlan,
    _work_items: &[LifecycleWorkItemRecord],
    item_by_id: &HashMap<&str, &LifecycleWorkItemRecord>,
    reachability: &HashMap<String, HashSet<String>>,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    for (i, left_id) in plan.work_item_ids.iter().enumerate() {
        for right_id in plan.work_item_ids.iter().skip(i + 1) {
            let left_reaches_right = reachability
                .get(left_id)
                .map(|set| set.contains(right_id))
                .unwrap_or(false);
            let right_reaches_left = reachability
                .get(right_id)
                .map(|set| set.contains(left_id))
                .unwrap_or(false);

            if left_reaches_right || right_reaches_left {
                continue;
            }

            let Some(left) = item_by_id.get(left_id.as_str()) else {
                continue;
            };
            let Some(right) = item_by_id.get(right_id.as_str()) else {
                continue;
            };

            if scopes_may_overlap(
                &left.exclusive_write_scopes,
                &right.exclusive_write_scopes,
                true,
            ) {
                findings.push(error(
                    "parallel_scope_overlap",
                    format!(
                        "work items {} and {} have no dependency ordering but their write scopes overlap",
                        left_id, right_id
                    ),
                    vec![left_id.clone(), right_id.clone()],
                ));
            }
        }
    }
}

fn validate_scopes_and_budgets(
    _plan: &IssueWorkItemPlan,
    work_items: &[LifecycleWorkItemRecord],
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    const MAX_SUMMARY_CHARS: usize = 50_000;
    const MAX_HANDOFF_CHARS: usize = 20_000;
    const MAX_CODE_CONTEXT_CHARS: usize = 50_000;
    const MAX_CONTEXT_FILE_REFS: usize = 120;
    const MAX_TRACEABILITY_REFS: usize = 80;
    const MAX_DEPENDENCY_HANDOFFS: usize = 5;

    for item in work_items {
        if item.exclusive_write_scopes.is_empty() {
            findings.push(error(
                "write_scope_required",
                format!(
                    "work item {} must have at least one exclusive_write_scope",
                    item.id
                ),
                vec![item.id.clone()],
            ));
        }

        let budget = &item.context_budget;
        if budget.max_summary_chars > MAX_SUMMARY_CHARS
            || budget.max_handoff_chars > MAX_HANDOFF_CHARS
            || budget.max_code_context_chars > MAX_CODE_CONTEXT_CHARS
            || budget.max_context_file_refs > MAX_CONTEXT_FILE_REFS
            || budget.max_traceability_refs > MAX_TRACEABILITY_REFS
            || budget.max_dependency_handoffs > MAX_DEPENDENCY_HANDOFFS
        {
            findings.push(error(
                "context_budget_over_limit",
                format!("work item {} context budget exceeds proxy limits", item.id),
                vec![item.id.clone()],
            ));
        }
    }
}

fn validate_semantics(
    plan: &IssueWorkItemPlan,
    work_items: &[LifecycleWorkItemRecord],
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    let mut has_backend = false;
    let mut has_frontend = false;
    let mut has_integration = false;
    let mut has_e2e = false;

    for item in work_items {
        match item.kind {
            WorkItemKind::Backend => has_backend = true,
            WorkItemKind::Frontend => has_frontend = true,
            WorkItemKind::Integration => has_integration = true,
            WorkItemKind::E2e => has_e2e = true,
            _ => {}
        }

        if item.story_spec_ids.is_empty() || item.design_spec_ids.is_empty() {
            findings.push(error(
                "traceability_refs_required",
                format!(
                    "work item {} must reference at least one story spec and one design spec",
                    item.id
                ),
                vec![item.id.clone()],
            ));
        }
    }

    if plan.options.force_frontend_backend_split && (!has_backend || !has_frontend) {
        findings.push(error(
            "frontend_backend_split_required",
            "force_frontend_backend_split is enabled but the plan does not contain both a backend and a frontend work item",
            Vec::new(),
        ));
    }

    if plan.options.include_integration_tests && !has_integration {
        findings.push(error(
            "integration_work_item_required",
            "include_integration_tests is enabled but the plan does not contain an integration work item",
            Vec::new(),
        ));
    }

    if plan.options.include_e2e_tests && !has_e2e {
        findings.push(error(
            "e2e_work_item_required",
            "include_e2e_tests is enabled but the plan does not contain an e2e work item",
            Vec::new(),
        ));
    }

    if !plan.options.include_integration_tests {
        findings.push(warning(
            "integration_or_e2e_skipped_risk",
            "integration tests are skipped; consider adding manual verification before acceptance",
            Vec::new(),
        ));
    }

    if !plan.options.include_e2e_tests {
        findings.push(warning(
            "integration_or_e2e_skipped_risk",
            "e2e tests are skipped; consider adding manual verification before acceptance",
            Vec::new(),
        ));
    }
}

fn validate_verification_plans(
    plan: &IssueWorkItemPlan,
    work_items: &[LifecycleWorkItemRecord],
    repository_profile: Option<&RepositoryProfile>,
    verification_plans: &[VerificationPlan],
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    if let Some(profile_ref) = &plan.repository_profile_ref {
        match repository_profile {
            Some(profile) if &profile.id == profile_ref => {}
            _ => {
                findings.push(error(
                    "repository_profile_missing",
                    format!(
                        "IssueWorkItemPlan references repository profile {} but it was not provided",
                        profile_ref
                    ),
                    Vec::new(),
                ));
            }
        }
    }

    if let Some(profile) = repository_profile
        && profile.confidence == RepositoryProfileConfidence::Low
    {
        findings.push(warning(
            "repository_profile_low_confidence",
            "repository profile confidence is low; verification commands require manual gate or provider repair",
            Vec::new(),
        ));
    }

    let plan_by_id: HashMap<&str, &VerificationPlan> = verification_plans
        .iter()
        .map(|plan| (plan.id.as_str(), plan))
        .collect();

    let available_plan_ids: HashSet<&str> = verification_plans
        .iter()
        .map(|plan| plan.id.as_str())
        .collect();

    for item in work_items {
        let Some(plan_ref) = &item.verification_plan_ref else {
            findings.push(error(
                "verification_plan_missing",
                format!(
                    "work item {} does not reference a verification plan",
                    item.id
                ),
                vec![item.id.clone()],
            ));
            continue;
        };

        let Some(verification) = plan_by_id.get(plan_ref.as_str()) else {
            findings.push(error(
                "verification_plan_missing",
                format!(
                    "work item {} references verification plan {} which was not provided",
                    item.id, plan_ref
                ),
                vec![item.id.clone()],
            ));
            continue;
        };

        if verification.project_id != item.project_id
            || verification.issue_id != item.issue_id
            || verification.work_item_id != item.id
        {
            findings.push(error(
                "verification_plan_mismatch",
                format!(
                    "verification plan {} does not match project/issue/work_item for work item {}",
                    plan_ref, item.id
                ),
                vec![item.id.clone()],
            ));
        }

        validate_verification_commands(item, verification, findings);
    }

    let declared_plan_ids: HashSet<&str> = plan
        .verification_plan_ids
        .iter()
        .map(String::as_str)
        .collect();
    if declared_plan_ids != available_plan_ids {
        findings.push(error(
            "verification_plan_mismatch",
            "IssueWorkItemPlan.verification_plan_ids does not match the provided verification plans",
            Vec::new(),
        ));
    }
}

fn validate_verification_commands(
    item: &LifecycleWorkItemRecord,
    verification: &VerificationPlan,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    let available_gate_ids: HashSet<&str> = verification
        .commands
        .iter()
        .map(|cmd| cmd.id.as_str())
        .chain(
            verification
                .manual_checks
                .iter()
                .map(|check| check.id.as_str()),
        )
        .collect();

    for command in &verification.commands {
        if command.source != VerificationCommandSource::Provider {
            findings.push(error(
                "verification_command_source_invalid",
                format!(
                    "verification command {} for work item {} has an invalid source; only provider sources are allowed",
                    command.id, item.id
                ),
                vec![item.id.clone()],
            ));
        }

        if !is_cwd_inside_repository(&command.cwd) {
            findings.push(error(
                "verification_command_cwd_outside_repository",
                format!(
                    "verification command {} for work item {} has cwd {} outside the repository",
                    command.id, item.id, command.cwd
                ),
                vec![item.id.clone()],
            ));
        }

        if command.safety == VerificationCommandSafety::NeedsManualReview && command.required {
            findings.push(warning(
                "verification_command_needs_manual_review",
                format!(
                    "required verification command {} for work item {} requires manual review",
                    command.id, item.id
                ),
                vec![item.id.clone()],
            ));
        }

        if is_command_unsafe(&command.command) {
            findings.push(error(
                "verification_command_unsafe",
                format!(
                    "verification command {} for work item {} is unsafe: {}",
                    command.id, item.id, command.command
                ),
                vec![item.id.clone()],
            ));
        }
    }

    for gate_id in &verification.required_gates {
        if !available_gate_ids.contains(gate_id.as_str()) {
            findings.push(error(
                "verification_gate_missing",
                format!(
                    "verification plan {} requires gate {} which is not defined as a command or manual check",
                    verification.id, gate_id
                ),
                vec![item.id.clone()],
            ));
        }
    }
}

fn is_cwd_inside_repository(cwd: &str) -> bool {
    if cwd.is_empty() {
        return true;
    }
    if cwd.starts_with('/') {
        return false;
    }
    cwd.split('/').all(|part| part != "..")
}

fn is_command_unsafe(command: &str) -> bool {
    let normalized = command.to_ascii_lowercase();
    let dangerous_substrings = [
        "rm -rf /",
        "rm -rf /*",
        "git reset --hard",
        "git clean -fdx",
        "> /",
        ">> /",
        "> ../",
        ">> ../",
        "| sh",
        "| bash",
        "mkfs",
        "dd if=",
    ];
    for pattern in dangerous_substrings {
        if normalized.contains(pattern) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::models::{
        WorkItemDraftCandidate, WorkItemOutline, WorkItemOutlineDependencyEdge,
    };

    #[test]
    fn outline_validator_rejects_duplicate_outline_ids() {
        let mut outline = valid_outline();
        outline.work_item_outlines[1].outline_id = "outline_backend".to_string();

        let report = WorkItemPlanOutlineValidator::validate(&outline);

        assert_has_code(&report, "duplicate_outline_id");
    }

    #[test]
    fn outline_validator_rejects_missing_dependency() {
        let mut outline = valid_outline();
        outline.work_item_outlines[1].depends_on = vec!["outline_missing".to_string()];

        let report = WorkItemPlanOutlineValidator::validate(&outline);

        assert_has_code(&report, "dependency_not_in_outline");
    }

    #[test]
    fn outline_validator_rejects_dependency_cycle() {
        let mut outline = valid_outline();
        outline.work_item_outlines[0].depends_on = vec!["outline_frontend".to_string()];

        let report = WorkItemPlanOutlineValidator::validate(&outline);

        assert_has_code(&report, "dependency_cycle");
    }

    #[test]
    fn outline_validator_requires_traceability_and_write_scopes() {
        let mut outline = valid_outline();
        outline.work_item_outlines[0].source_story_spec_ids.clear();
        outline.work_item_outlines[0].source_design_spec_ids.clear();
        outline.work_item_outlines[0].goal.clear();
        outline.work_item_outlines[0].scope.clear();
        outline.work_item_outlines[0].exclusive_write_scopes.clear();

        let report = WorkItemPlanOutlineValidator::validate(&outline);

        assert_has_code(&report, "traceability_required");
        assert_has_code(&report, "outline_goal_required");
        assert_has_code(&report, "outline_scope_required");
        assert_has_code(&report, "write_scope_required");
    }

    #[test]
    fn outline_validator_detects_direct_scope_conflict() {
        let mut outline = valid_outline();
        outline.work_item_outlines[1].depends_on.clear();
        outline.dependency_graph.clear();
        outline.work_item_outlines[1].exclusive_write_scopes =
            vec!["src/product/api.rs".to_string()];

        let report = WorkItemPlanOutlineValidator::validate(&outline);

        assert_has_code(&report, "parallel_scope_overlap");
    }

    #[test]
    fn outline_validator_detects_dependent_scope_conflict() {
        let mut outline = valid_outline();
        outline.work_item_outlines[1].exclusive_write_scopes =
            vec!["src/product/api.rs".to_string()];

        let report = WorkItemPlanOutlineValidator::validate(&outline);

        assert_has_code(&report, "write_scope_conflict");
    }

    #[test]
    fn local_validator_allows_valid_single_draft() {
        let outline = valid_outline();
        let current_outline = outline.work_item_outlines[1].clone();
        let dependency = valid_draft_candidate("outline_backend", vec![]);
        let current = valid_draft_candidate("outline_frontend", vec!["outline_backend"]);

        let report =
            WorkItemDraftLocalValidator::validate(&current, &[dependency], &current_outline);

        assert!(
            !report.has_errors(),
            "expected valid local draft, got {:?}",
            report.findings
        );
    }

    #[test]
    fn local_validator_blocks_missing_write_scope() {
        let outline = valid_outline();
        let current_outline = outline.work_item_outlines[0].clone();
        let mut current = valid_draft_candidate("outline_backend", vec![]);
        current.exclusive_write_scopes.clear();

        let report = WorkItemDraftLocalValidator::validate(&current, &[], &current_outline);

        assert_has_code(&report, "write_scope_required");
    }

    #[test]
    fn local_validator_blocks_required_gate_missing() {
        let outline = valid_outline();
        let current_outline = outline.work_item_outlines[0].clone();
        let mut current = valid_draft_candidate("outline_backend", vec![]);
        current.verification_plan["required_gates"] = serde_json::json!(["cmd_missing"]);

        let report = WorkItemDraftLocalValidator::validate(&current, &[], &current_outline);

        assert_has_code(&report, "verification_required_gate_missing");
    }

    #[test]
    fn local_validator_blocks_scope_conflict_with_direct_dependency() {
        let outline = valid_outline();
        let current_outline = outline.work_item_outlines[1].clone();
        let dependency = valid_draft_candidate("outline_backend", vec![]);
        let mut current = valid_draft_candidate("outline_frontend", vec!["outline_backend"]);
        current.exclusive_write_scopes = vec!["src/product/api.rs".to_string()];

        let report =
            WorkItemDraftLocalValidator::validate(&current, &[dependency], &current_outline);

        assert_has_code(&report, "direct_dependency_scope_conflict");
    }

    fn assert_has_code(report: &WorkItemSplitValidationReport, code: &str) {
        assert!(
            report.findings.iter().any(|finding| finding.code == code),
            "expected code {code}, got {:?}",
            report.findings
        );
    }

    fn valid_outline() -> WorkItemPlanOutline {
        WorkItemPlanOutline {
            id: "outline_artifact_1".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: vec!["story_spec_0001".to_string()],
            source_design_spec_ids: vec!["design_spec_0001".to_string()],
            strategy_summary: "后端先行，前端随后接入".to_string(),
            work_item_outlines: vec![
                WorkItemOutline {
                    outline_id: "outline_backend".to_string(),
                    title: "后端 API".to_string(),
                    kind: WorkItemKind::Backend,
                    goal: "实现 API".to_string(),
                    scope: vec!["src/product".to_string()],
                    non_goals: vec![],
                    source_story_spec_ids: vec!["story_spec_0001".to_string()],
                    source_design_spec_ids: vec!["design_spec_0001".to_string()],
                    exclusive_write_scopes: vec!["src/product/api.rs".to_string()],
                    forbidden_write_scopes: vec!["web/**".to_string()],
                    depends_on: vec![],
                    verification_intent: vec!["cargo test --locked --lib api".to_string()],
                    handoff_notes: "提供 API contract".to_string(),
                },
                WorkItemOutline {
                    outline_id: "outline_frontend".to_string(),
                    title: "前端 UI".to_string(),
                    kind: WorkItemKind::Frontend,
                    goal: "接入 API".to_string(),
                    scope: vec!["web/src".to_string()],
                    non_goals: vec![],
                    source_story_spec_ids: vec!["story_spec_0001".to_string()],
                    source_design_spec_ids: vec!["design_spec_0001".to_string()],
                    exclusive_write_scopes: vec!["web/src/session.ts".to_string()],
                    forbidden_write_scopes: vec!["src/product/**".to_string()],
                    depends_on: vec!["outline_backend".to_string()],
                    verification_intent: vec!["pnpm -C web test".to_string()],
                    handoff_notes: "消费 API contract".to_string(),
                },
            ],
            dependency_graph: vec![WorkItemOutlineDependencyEdge {
                from_outline_id: "outline_backend".to_string(),
                to_outline_id: "outline_frontend".to_string(),
            }],
            risks: vec![],
            handoff_strategy: "后端输出 contract 给前端".to_string(),
            status: "draft".to_string(),
        }
    }

    fn valid_draft_candidate(
        outline_id: &str,
        depends_on_outline_ids: Vec<&str>,
    ) -> WorkItemDraftCandidate {
        WorkItemDraftCandidate {
            outline_id: outline_id.to_string(),
            title: format!("Draft {outline_id}"),
            kind: WorkItemKind::Backend,
            goal: "实现局部 work item".to_string(),
            implementation_context: "实现必要代码并保持 handoff。".to_string(),
            exclusive_write_scopes: if outline_id == "outline_backend" {
                vec!["src/product/api.rs".to_string()]
            } else {
                vec!["web/src/session.ts".to_string()]
            },
            forbidden_write_scopes: vec![],
            depends_on_outline_ids: depends_on_outline_ids
                .into_iter()
                .map(str::to_string)
                .collect(),
            required_handoff_from_outline_ids: vec![],
            handoff_summary: "handoff summary".to_string(),
            verification_plan: serde_json::json!({
                "commands": [
                    {
                        "id": "cmd_test",
                        "label": "test",
                        "command": "cargo test --locked --lib api",
                        "cwd": "",
                        "purpose": "验证局部 draft",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved",
                        "source": "provider"
                    }
                ],
                "manual_checks": [],
                "required_gates": ["cmd_test"]
            }),
        }
    }
}
