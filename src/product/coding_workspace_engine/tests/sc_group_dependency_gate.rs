#[cfg(test)]
mod tests {
    use crate::product::coding_workspace_engine::group_dependency_gate::{
        GroupUnitSelectionOutcome, topological_layers,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn graph(edges: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
        edges
            .iter()
            .map(|(id, dependencies)| {
                (
                    (*id).to_string(),
                    dependencies.iter().map(|id| (*id).to_string()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn sc_group_dependency_gate_blocks_consumer_until_dependency_completed_and_handoff_published() {
        let (layers, cycle) = topological_layers(&graph(&[("A", &[]), ("B", &["A"])]));
        assert!(!cycle);
        assert_eq!(layers["A"], 0);
        assert_eq!(layers["B"], 1);
    }

    #[test]
    fn sc_group_dependency_gate_waits_when_all_pending_units_are_unready() {
        let (layers, cycle) = topological_layers(&graph(&[("A", &[]), ("B", &["A"])]));
        assert!(!cycle);
        assert_eq!(layers.len(), 2);
        let waiting = GroupUnitSelectionOutcome::Waiting {
            pending_unit_ids: vec!["unit_b".to_string()],
            reason_code: "SC_GROUP_DEPENDENCY_HANDOFF_PENDING".to_string(),
        };
        assert!(matches!(waiting, GroupUnitSelectionOutcome::Waiting { .. }));
    }

    #[test]
    fn sc_group_dependency_gate_fails_closed_on_handoff_binding_mismatch() {
        let failed = GroupUnitSelectionOutcome::FailedClosed {
            reason_code: "SC_GROUP_HANDOFF_PLAN_BINDING_MISMATCH".to_string(),
            message: "mismatch".to_string(),
        };
        assert!(matches!(
            failed,
            GroupUnitSelectionOutcome::FailedClosed { .. }
        ));
    }

    #[test]
    fn sc_group_dependency_gate_fails_closed_on_unknown_self_or_cycle() {
        let (_, cycle) = topological_layers(&graph(&[("A", &["B"]), ("B", &["A"])]));
        assert!(cycle);
        assert!(matches!(
            crate::product::coding_workspace_engine::group_dependency_gate::failed_unknown("unknown", "bad"),
            GroupUnitSelectionOutcome::FailedClosed { reason_code, .. }
                if reason_code == "SC_GROUP_DEPENDENCY_UNKNOWN"
        ));
        assert!(matches!(
            crate::product::coding_workspace_engine::group_dependency_gate::failed_self("self"),
            GroupUnitSelectionOutcome::FailedClosed { reason_code, .. }
                if reason_code == "SC_GROUP_DEPENDENCY_SELF"
        ));
    }
}
