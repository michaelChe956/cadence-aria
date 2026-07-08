use super::*;

const SINGLE_AGENT_SESSION_CONTEXT_TOKEN_LIMIT: u32 = 20_000;

pub(crate) fn validate_outline_ids(
    outline: &WorkItemPlanOutline,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
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

pub(crate) fn validate_outline_traceability_and_scopes(
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
        match item.estimated_context_tokens {
            Some(value) if value > 0 && value < SINGLE_AGENT_SESSION_CONTEXT_TOKEN_LIMIT => {}
            Some(0) | None => findings.push(error(
                "outline_budget_required",
                format!(
                    "outline {} must include estimated_context_tokens between 1 and 19999",
                    item.outline_id
                ),
                vec![item.outline_id.clone()],
            )),
            Some(value) => findings.push(error(
                "outline_exceeds_single_session_budget",
                format!(
                    "outline {} estimated_context_tokens {} exceeds the single-agent session budget of <20000",
                    item.outline_id, value
                ),
                vec![item.outline_id.clone()],
            )),
        }
        match item.session_fit.as_ref() {
            Some(WorkItemOutlineSessionFit::FitsSingleAgentSession) => {}
            Some(WorkItemOutlineSessionFit::TooLargeMustSplit) => findings.push(error(
                "outline_too_large_must_split",
                format!(
                    "outline {} is marked too_large_must_split and must be split before draft generation",
                    item.outline_id
                ),
                vec![item.outline_id.clone()],
            )),
            None => findings.push(error(
                "outline_session_fit_required",
                format!(
                    "outline {} must declare session_fit=fits_single_agent_session",
                    item.outline_id
                ),
                vec![item.outline_id.clone()],
            )),
        }
    }
}

pub(crate) fn validate_outline_dependencies(
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
        let from_known = outline_ids.contains(edge.from_outline_id.as_str());
        let to_known = outline_ids.contains(edge.to_outline_id.as_str());
        if !from_known {
            findings.push(error(
                "dependency_not_in_outline",
                format!(
                    "dependency graph references missing from_outline_id {}",
                    edge.from_outline_id
                ),
                vec![edge.from_outline_id.clone()],
            ));
        }
        if !to_known {
            findings.push(error(
                "dependency_not_in_outline",
                format!(
                    "dependency graph references missing to_outline_id {}",
                    edge.to_outline_id
                ),
                vec![edge.to_outline_id.clone()],
            ));
        }
    }

    validate_outline_dependency_graph_projection(outline, &edges, findings);

    edges
}

fn validate_outline_dependency_graph_projection(
    outline: &WorkItemPlanOutline,
    derived_edges: &HashSet<(String, String)>,
    findings: &mut Vec<WorkItemSplitFinding>,
) {
    if outline.dependency_graph.is_empty() {
        return;
    }

    let outline_ids: HashSet<&str> = outline
        .work_item_outlines
        .iter()
        .map(|item| item.outline_id.as_str())
        .collect();
    let declared_edges: HashSet<(String, String)> = outline
        .dependency_graph
        .iter()
        .filter(|edge| {
            outline_ids.contains(edge.from_outline_id.as_str())
                && outline_ids.contains(edge.to_outline_id.as_str())
        })
        .map(|edge| (edge.from_outline_id.clone(), edge.to_outline_id.clone()))
        .collect();

    for (dependency, item) in derived_edges {
        if declared_edges.contains(&(dependency.clone(), item.clone())) {
            continue;
        }
        if declared_edges.contains(&(item.clone(), dependency.clone())) {
            findings.push(error(
                "dependency_graph_direction_reversed",
                format!(
                    "outline {item} depends_on {dependency}, so dependency_graph must contain {dependency} -> {item}; found reversed {item} -> {dependency}"
                ),
                vec![dependency.clone(), item.clone()],
            ));
        } else {
            findings.push(error(
                "dependency_graph_mismatch",
                format!(
                    "outline {item} depends_on {dependency}, so dependency_graph must contain {dependency} -> {item}"
                ),
                vec![dependency.clone(), item.clone()],
            ));
        }
    }

    for (from, to) in declared_edges.difference(derived_edges) {
        if derived_edges.contains(&(to.clone(), from.clone())) {
            continue;
        }
        findings.push(error(
            "dependency_graph_mismatch",
            format!(
                "dependency_graph edge {from} -> {to} is not derived from work_item_outlines[].depends_on"
            ),
            vec![from.clone(), to.clone()],
        ));
    }
}

pub(crate) fn validate_outline_dependency_cycles(
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
            "outline depends_on dependencies contain a cycle",
            Vec::new(),
        ));
    }
}

pub(crate) fn validate_outline_scope_conflicts(
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
