#[tokio::test]
async fn web_work_item_plan_repair_recovers_every_fault_point_without_duplicate_identity() {
    for fault_point in PlanRepairFaultPoint::ALL {
        let root = tempdir().expect("fixture root");
        let runtime = PlanRepairFixtureRuntime::seed(
            root.path(),
            PlanRepairFixtureControl {
                fault_point: Some(fault_point),
            },
        )
        .await
        .unwrap_or_else(|error| panic!("seed {fault_point:?}: {error}"));

        let interrupted = runtime
            .drive_until_fault()
            .await
            .expect_err("configured fault point must interrupt the fixture");
        assert_eq!(interrupted.fault_point(), Some(fault_point));
        drop(runtime);

        let restarted = PlanRepairFixtureRuntime::reopen(root.path())
            .await
            .unwrap_or_else(|error| panic!("reopen {fault_point:?}: {error}"));
        let recovered = restarted
            .recover_to_completion()
            .await
            .unwrap_or_else(|error| panic!("recover {fault_point:?}: {error}"));

        assert_eq!(recovered.repair_request_count, 1, "{fault_point:?}");
        assert_eq!(
            recovered.amendment_reference_ids.len(),
            1,
            "{fault_point:?}"
        );
        assert_eq!(
            recovered.unique_amendment_reference_ids, 1,
            "{fault_point:?}"
        );
        assert!(
            recovered.amendment_reference_ids[0].starts_with("plan_amendment_"),
            "{fault_point:?}"
        );
        assert_eq!(
            recovered.amendment_artifact_ids, recovered.amendment_reference_ids,
            "{fault_point:?}"
        );
        assert_eq!(
            recovered.unique_amendment_artifact_ids, 1,
            "{fault_point:?}"
        );
        assert_eq!(
            recovered.logical_active_revision_ids["wi_core"], "work_item_revision_wi_core_0002",
            "{fault_point:?}"
        );
        assert_eq!(
            recovered.unit_run_ids.len(),
            recovered.unique_unit_run_ids,
            "{fault_point:?}"
        );
        assert_eq!(
            recovered.handoff_revision_ids,
            vec!["handoff_revision_0001", "handoff_revision_0002"],
            "{fault_point:?}"
        );
        assert_eq!(
            recovered.handoff_revision_ids.len(),
            recovered.unique_handoff_revision_ids,
            "{fault_point:?}"
        );
        assert_eq!(
            recovered.current_resolved_handoff_revision_ids,
            vec!["handoff_revision_0002"],
            "{fault_point:?}"
        );
    }
}
