use super::*;

pub(crate) fn validate_plan_membership(
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

pub(crate) fn validate_dependencies(
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

pub(crate) fn validate_scopes_and_budgets(
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

pub(crate) fn validate_semantics(
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

pub(crate) fn validate_verification_plans(
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
