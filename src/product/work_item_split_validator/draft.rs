use super::*;

pub(crate) fn validate_draft_matches_outline(
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

pub(crate) fn validate_draft_scopes(
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

pub(crate) fn validate_draft_verification_plan(
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

pub(crate) fn validate_draft_direct_dependency_scopes(
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
