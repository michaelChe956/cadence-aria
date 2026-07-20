#[tokio::test]
async fn work_item_plan_repair_provider_matrix_runs_parses_routes_and_persists() {
    use cadence_aria::product::models::{PlanDefectClass, PlanDefectRoute, ProviderName};

    for provider in [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ] {
        let root = tempdir().expect("provider matrix root");
        let runtime =
            PlanRepairFixtureRuntime::seed(root.path(), PlanRepairFixtureControl::default())
                .await
                .expect("seed provider matrix fixture");

        let result = runtime
            .run_provider_matrix(provider.clone())
            .await
            .expect("run provider matrix");

        assert_eq!(result.provider, provider);
        assert!(result.rendered_contract_ids_preserved);
        assert!(result.author_contract_ids.contains(&"contract.workflow".to_string()));
        assert!(result.plan_review_passed);
        assert_eq!(result.coder_defect_class, PlanDefectClass::UpstreamContractInvalid);
        assert_eq!(
            result.code_review_defect_class,
            PlanDefectClass::UpstreamContractInvalid
        );
        assert_eq!(result.code_review_route, PlanDefectRoute::PlanRepair);
        assert!(result.author_draft_artifact_persisted);
        assert!(result.plan_review_complete_event_observed);
        assert_eq!(result.coding_role_run_count, 2);
        assert_eq!(result.coding_raw_output_ref_count, 2);
    }
}
