use cadence_aria::cross_cutting::provider_context_builder::{
    ProviderContextBuildError, ProviderContextBuilderInput, build_provider_context,
};
use cadence_aria::protocol::contracts::{
    AdapterRole, CommandClass, PromptSection, ProviderType, RuntimeRole,
    execution_contract_for_node, phase1_node_contract_table, workflow_discipline_for_node,
};
use cadence_aria::protocol::prompt_manifest::{
    PromptRenderError, phase1_prompt_manifest, planning_prompt_manifest, render_prompt_template,
};
use cadence_aria::runtime_units::prompt_template_registry::{
    all_phase1_provider_node_ids, all_planning_node_ids, prompt_template_for_node,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs};

#[test]
fn contract_workflow_and_prompt_registries_cover_n04_to_n12() {
    let nodes = all_planning_node_ids();
    assert_eq!(
        nodes,
        vec![
            "N04", "N05", "N06", "N07", "N08", "N09", "N10", "N11", "N12"
        ]
    );

    let manifest = planning_prompt_manifest();
    assert_eq!(manifest.entries.len(), 9);

    for node_id in nodes {
        let contract = execution_contract_for_node(node_id).expect("contract");
        let workflow = workflow_discipline_for_node(node_id).expect("workflow");
        let template = prompt_template_for_node(node_id).expect("template");

        assert_eq!(contract.node_id, node_id);
        assert_eq!(workflow.node_id, node_id);
        assert_eq!(
            template.template_ref.output_schema_ref,
            contract.output_schema_ref
        );
        assert_eq!(template.template_ref.required_sections, required_sections());
        assert_eq!(template.template_ref.render_order, required_sections());
        assert!(
            workflow
                .superpowers_required
                .contains(&"using-superpowers".to_string())
        );
    }

    assert!(
        workflow_discipline_for_node("N04")
            .expect("N04 workflow")
            .superpowers_required
            .contains(&"brainstorming".to_string())
    );
    assert!(
        workflow_discipline_for_node("N05")
            .expect("N05 workflow")
            .superpowers_required
            .contains(&"brainstorming".to_string())
    );
    assert!(
        workflow_discipline_for_node("N07")
            .expect("N07 workflow")
            .superpowers_required
            .contains(&"brainstorming".to_string())
    );
    assert!(
        workflow_discipline_for_node("N11")
            .expect("N11 workflow")
            .superpowers_required
            .contains(&"writing-plans".to_string())
    );

    let n06 = execution_contract_for_node("N06").expect("N06 contract");
    assert_eq!(n06.runtime_role, RuntimeRole::AdvisoryReviewer);
    assert_eq!(n06.adapter_role, AdapterRole::Reviewer);
    assert!(n06.advisory_only);
}

#[test]
fn context_builder_renders_each_planning_prompt_and_maps_adapter_input() {
    for node_id in all_planning_node_ids() {
        let result = build_provider_context(builder_input(node_id)).expect("context package");
        let contract = execution_contract_for_node(node_id).expect("contract");

        assert_eq!(result.context_package.node_id, node_id);
        assert_eq!(result.context_package.session_id, "session_001");
        assert_eq!(result.context_package.task_id, "task_001");
        assert_eq!(result.context_package.provider_type, contract.provider_type);
        assert_eq!(result.context_package.adapter_role, contract.adapter_role);
        assert_eq!(
            result.context_package.prompt_template.template_id,
            contract.prompt_template_id
        );
        assert_eq!(result.adapter_input.provider_type, contract.provider_type);
        assert_eq!(result.adapter_input.role, contract.adapter_role);
        assert_eq!(
            result.adapter_input.output_schema,
            contract.output_schema_ref
        );
        assert_eq!(result.adapter_input.timeout, contract.timeout_sec);
        assert_eq!(result.adapter_input.max_retries, contract.max_retries);
        assert_eq!(
            result.adapter_input.context_files,
            vec![
                "tests/fixtures/artifacts/spec.md".to_string(),
                "tests/fixtures/openspec/changes/sample-change/proposal.md".to_string()
            ]
        );

        assert_section_order(&result.adapter_input.prompt);
        assert!(
            result
                .adapter_input
                .prompt
                .contains(&format!("node_id={node_id}"))
        );
        assert!(
            result
                .adapter_input
                .prompt
                .contains(&contract.output_schema_ref)
        );
        assert!(
            result
                .adapter_input
                .prompt
                .contains("canonical summary for test")
        );
        assert!(
            result
                .adapter_input
                .prompt
                .contains("constraint summary for test")
        );
    }
}

#[test]
fn contract_workflow_and_prompt_registries_cover_p4_execution_and_closure_nodes() {
    let provider_nodes = all_phase1_provider_node_ids();
    for node_id in [
        "N16", "N17", "N18", "N19", "N20", "N24", "N25", "N26", "N27",
    ] {
        assert!(
            provider_nodes.contains(&node_id),
            "{node_id} missing from provider node registry"
        );
    }

    let manifest = phase1_prompt_manifest();
    for node_id in [
        "N16", "N17", "N18", "N19", "N20", "N24", "N25", "N26", "N27",
    ] {
        let contract = execution_contract_for_node(node_id).expect("contract");
        let workflow = workflow_discipline_for_node(node_id).expect("workflow");
        let template = prompt_template_for_node(node_id).expect("template");

        assert_eq!(contract.node_id, node_id);
        assert_eq!(workflow.node_id, node_id);
        assert_eq!(
            template.template_ref.output_schema_ref,
            contract.output_schema_ref
        );
        assert_eq!(template.template_ref.required_sections, required_sections());
        assert_eq!(template.template_ref.render_order, required_sections());
        assert!(
            manifest.entries.iter().any(|entry| {
                entry.node_id == node_id
                    && entry.template_id == contract.prompt_template_id
                    && entry.output_schema_ref == contract.output_schema_ref
            }),
            "{node_id} missing from prompt manifest"
        );
        assert!(
            workflow
                .superpowers_required
                .contains(&"using-superpowers".to_string())
        );
        assert!(
            contract
                .completion_criteria
                .contains(&"emit_aria_structured_output".to_string())
        );
        assert!(
            !contract.forbidden_actions.is_empty(),
            "{node_id} must carry forbidden actions into provider context"
        );
    }

    let n16 = execution_contract_for_node("N16").expect("N16 contract");
    assert_eq!(n16.provider_type, ProviderType::Codex);
    assert_eq!(n16.runtime_role, RuntimeRole::Executor);
    assert_eq!(n16.adapter_role, AdapterRole::Executor);
    assert!(!n16.advisory_only);
    assert_eq!(
        n16.output_schema_ref,
        "schema://aria/artifacts/coding_report/v1"
    );
    assert!(
        n16.allowed_command_classes
            .contains(&CommandClass::FileWrite)
    );
    assert_eq!(
        n16.allowed_write_scope,
        vec!["<worktask_routing.allowed_write_scope>".to_string()]
    );
    assert!(
        workflow_discipline_for_node("N16")
            .expect("N16 workflow")
            .superpowers_required
            .contains(&"test-driven-development".to_string())
    );

    let n17 = execution_contract_for_node("N17").expect("N17 contract");
    assert_eq!(n17.provider_type, ProviderType::Codex);
    assert_eq!(n17.runtime_role, RuntimeRole::Executor);
    assert_eq!(n17.adapter_role, AdapterRole::Executor);
    assert_eq!(n17.allowed_write_scope, Vec::<String>::new());
    assert!(n17.allowed_command_classes.contains(&CommandClass::Test));
    assert!(
        workflow_discipline_for_node("N17")
            .expect("N17 workflow")
            .superpowers_optional
            .contains(&"systematic-debugging".to_string())
    );

    let n18 = execution_contract_for_node("N18").expect("N18 contract");
    assert_eq!(n18.runtime_role, RuntimeRole::Reviewer);
    assert_eq!(n18.adapter_role, AdapterRole::Reviewer);
    assert_eq!(n18.allowed_command_classes, vec![CommandClass::ReadOnly]);

    let n20 = execution_contract_for_node("N20").expect("N20 contract");
    assert_eq!(n20.runtime_role, RuntimeRole::AdvisoryReviewer);
    assert_eq!(n20.adapter_role, AdapterRole::Reviewer);
    assert!(n20.advisory_only);
    assert_eq!(
        n20.output_schema_ref,
        "schema://aria/advisory/ready_advisory/v1"
    );

    let n24 = execution_contract_for_node("N24").expect("N24 contract");
    assert_eq!(n24.runtime_role, RuntimeRole::AdvisoryReviewer);
    assert!(n24.advisory_only);
    assert_eq!(
        n24.output_schema_ref,
        "schema://aria/advisory/integration_verify_advisory/v1"
    );

    let n25 = execution_contract_for_node("N25").expect("N25 contract");
    assert_eq!(n25.provider_type, ProviderType::ClaudeCode);
    assert_eq!(n25.runtime_role, RuntimeRole::Orchestrator);
    assert_eq!(
        n25.output_schema_ref,
        "schema://aria/artifacts/final_review/v1"
    );

    let rows = phase1_node_contract_table();
    assert_eq!(rows.first().expect("first row").node_id, "N13");
    assert_eq!(rows.last().expect("last row").node_id, "N28");
    assert_eq!(rows.len(), 16);
    assert!(
        rows.iter()
            .find(|row| row.node_id == "N23")
            .expect("N23 row")
            .prompt_template_id
            .is_none()
    );
    assert!(
        rows.iter()
            .find(|row| row.node_id == "N25")
            .expect("N25 row")
            .prompt_template_id
            .as_deref()
            == Some("tpl_n25_final_review_v1")
    );
}

#[test]
fn context_builder_renders_p4_provider_nodes_and_rejects_missing_required_inputs() {
    for node_id in [
        "N16", "N17", "N18", "N19", "N20", "N24", "N25", "N26", "N27",
    ] {
        let result = build_provider_context(p4_builder_input(node_id)).expect("context package");
        let contract = execution_contract_for_node(node_id).expect("contract");

        assert_eq!(result.context_package.node_id, node_id);
        assert_eq!(result.context_package.provider_type, contract.provider_type);
        assert_eq!(result.adapter_input.provider_type, contract.provider_type);
        assert_eq!(result.adapter_input.role, contract.adapter_role);
        assert_eq!(
            result.adapter_input.output_schema,
            contract.output_schema_ref
        );
        assert!(
            result
                .context_package
                .context_files
                .contains(&"tests/fixtures/artifacts/spec.md".to_string())
        );
        assert!(
            result
                .context_package
                .context_files
                .contains(&"tests/fixtures/projections/plan_projection.json".to_string())
        );
        assert!(
            result
                .context_package
                .context_files
                .contains(&"tests/fixtures/openspec/constraint_bundle.json".to_string())
        );
        assert!(result.context_package.context_files.contains(&format!(
            ".aria/context-packages/task_001/{}.json",
            node_id.to_ascii_lowercase()
        )));
        assert!(result.adapter_input.prompt.contains("forbidden_actions="));
        assert!(result.adapter_input.prompt.contains("completion_criteria="));
        assert!(
            result
                .adapter_input
                .prompt
                .contains("verification_commands=")
        );

        if matches!(node_id, "N16" | "N19") {
            assert_eq!(
                result.context_package.allowed_write_scope,
                vec!["src/feature/".to_string()]
            );
        }
        if matches!(node_id, "N20" | "N24") {
            assert!(result.context_package.advisory_only);
            assert_eq!(result.context_package.adapter_role, AdapterRole::Reviewer);
        }
    }

    let mut missing_projection = p4_builder_input("N16");
    missing_projection.projection_refs.clear();
    assert_eq!(
        build_provider_context(missing_projection).expect_err("projection required"),
        ProviderContextBuildError::MissingProjectionRefs("N16".to_string())
    );

    let mut missing_bundle = p4_builder_input("N16");
    missing_bundle.constraint_bundle_ref.clear();
    assert_eq!(
        build_provider_context(missing_bundle).expect_err("bundle required"),
        ProviderContextBuildError::MissingConstraintBundleRef("N16".to_string())
    );

    let mut missing_worktree = p4_builder_input("N16");
    missing_worktree.worktree_path = None;
    assert_eq!(
        build_provider_context(missing_worktree).expect_err("worktree required"),
        ProviderContextBuildError::MissingWorktreePath("N16".to_string())
    );

    let mut missing_acceptance_targets = p4_builder_input("N16");
    missing_acceptance_targets.canonical_inputs["acceptance_targets"] = json!([]);
    assert_eq!(
        build_provider_context(missing_acceptance_targets)
            .expect_err("acceptance targets required"),
        ProviderContextBuildError::MissingAcceptanceTargets("N16".to_string())
    );
}

#[test]
fn node_specific_fields_snapshot_fixtures_round_trip_minimal_fields() {
    assert_node_specific_fields_fixture(
        "tests/fixtures/snapshots/n13_n24_node_specific_fields.json",
        &[
            ("N13", &["worktask_id", "routing_ref", "state"][..]),
            (
                "N14",
                &["worktree_path", "lease_id", "base_ref", "branch_name"][..],
            ),
            ("N15", &["dispatch_package_ref", "worktask_routing"][..]),
            ("N16", &["coding_report_ref", "changed_files"][..]),
            (
                "N17",
                &["testing_report_ref", "test_results", "coverage_summary"][..],
            ),
            ("N18", &["code_review_report_ref", "findings"][..]),
            ("N19", &["rework_scope", "superseded_report_refs"][..]),
            (
                "N20",
                &["candidate_commit_sha", "ready_decision", "block_reason"][..],
            ),
            ("N21", &["queue_position", "integration_record_id"][..]),
            (
                "N22",
                &[
                    "integration_branch",
                    "pre_merge_sha",
                    "candidate_commit_sha",
                ][..],
            ),
            (
                "N23",
                &[
                    "integration_commit_sha",
                    "post_merge_sha",
                    "rollback_ref",
                    "next_decision",
                ][..],
            ),
            ("N24", &["verify_decision", "rollback_reason"][..]),
        ],
    );
    assert_node_specific_fields_fixture(
        "tests/fixtures/snapshots/n25_n28_node_specific_fields.json",
        &[
            (
                "N25",
                &[
                    "overall_decision",
                    "coverage_summary",
                    "uncovered_items",
                    "manual_exemptions",
                ][..],
            ),
            (
                "N26",
                &[
                    "patch_task_delta",
                    "new_dispatch_package_ref",
                    "patch_round_counter",
                ][..],
            ),
            (
                "N27",
                &["overall_status", "closed_items", "remaining_risks"][..],
            ),
            (
                "N28",
                &["session_closeout_timestamp", "final_checkpoint_ref"][..],
            ),
        ],
    );
}

#[test]
fn prompt_renderer_reports_missing_variable() {
    let template = prompt_template_for_node("N10").expect("N10 template");
    let mut variables = full_variables("N10");
    variables.remove("constraint_summary");

    let error =
        render_prompt_template(&template, &variables).expect_err("missing variable should fail");

    assert_eq!(
        error,
        PromptRenderError::MissingVariable("constraint_summary".to_string())
    );
}

fn builder_input(node_id: &str) -> ProviderContextBuilderInput {
    ProviderContextBuilderInput {
        session_id: "session_001".to_string(),
        task_id: "task_001".to_string(),
        node_id: node_id.to_string(),
        canonical_inputs: json!({
            "artifact_refs": ["art_ref_spec_0001"],
            "risk_registry_ref": "risk_registry_001"
        }),
        canonical_input_summary: "canonical summary for test".to_string(),
        projection_refs: vec!["proj_spec_projection_art_spec_001_0001".to_string()],
        projection_summary: "projection summary for test".to_string(),
        constraint_bundle_ref: "constraint_bundle_openspec_sample-change_0001".to_string(),
        constraint_summary: "constraint summary for test".to_string(),
        context_files: vec![
            "tests/fixtures/artifacts/spec.md".to_string(),
            "tests/fixtures/openspec/changes/sample-change/proposal.md".to_string(),
        ],
        worktree_path: None,
    }
}

fn p4_builder_input(node_id: &str) -> ProviderContextBuilderInput {
    ProviderContextBuilderInput {
        session_id: "session_001".to_string(),
        task_id: "task_001".to_string(),
        node_id: node_id.to_string(),
        canonical_inputs: json!({
            "artifact_refs": [
                "art_ref_spec_0001",
                "art_ref_design_0001",
                "art_ref_plan_0001",
                "art_ref_dispatch_0001"
            ],
            "risk_registry_ref": "risk_registry_001",
            "acceptance_targets": ["cargo test --test execution_chain_fake_provider"],
            "worktask_routing": {
                "worktask_id": "worktask_001",
                "source_work_package_id": "WP-001",
                "allowed_write_scope": ["src/feature/"]
            }
        }),
        canonical_input_summary: "canonical summary for p4 test".to_string(),
        projection_refs: vec![
            "proj_spec_projection_art_spec_001_0001".to_string(),
            "proj_design_projection_art_design_001_0001".to_string(),
            "proj_plan_projection_art_plan_001_0001".to_string(),
        ],
        projection_summary: "projection summary for p4 test".to_string(),
        constraint_bundle_ref: "constraint_bundle_openspec_sample-change_0001".to_string(),
        constraint_summary: "constraint summary for p4 test".to_string(),
        context_files: vec![
            "tests/fixtures/artifacts/spec.md".to_string(),
            "tests/fixtures/projections/plan_projection.json".to_string(),
            "tests/fixtures/openspec/constraint_bundle.json".to_string(),
            format!(
                ".aria/context-packages/task_001/{}.json",
                node_id.to_ascii_lowercase()
            ),
        ],
        worktree_path: Some("tests/fixtures/repos/sample-worktree".to_string()),
    }
}

fn assert_node_specific_fields_fixture(path: &str, required_by_node: &[(&str, &[&str])]) {
    let source = fs::read_to_string(path).expect("fixture readable");
    let value: Value = serde_json::from_str(&source).expect("fixture json");
    let object = value.as_object().expect("fixture object");
    for (node_id, required_fields) in required_by_node {
        let fields = object
            .get(*node_id)
            .unwrap_or_else(|| panic!("{node_id} missing from {path}"));
        let fields = fields.as_object().expect("node_specific_fields object");
        for field in *required_fields {
            assert!(
                fields.contains_key(*field),
                "{node_id}.{field} missing from {path}"
            );
        }
    }

    let encoded = serde_json::to_string(&value).expect("serialize fixture");
    let round_trip: Value = serde_json::from_str(&encoded).expect("deserialize fixture");
    assert_eq!(round_trip, value);
}

fn full_variables(node_id: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("node_id".to_string(), node_id.to_string()),
        ("runtime_role".to_string(), "orchestrator".to_string()),
        ("adapter_role".to_string(), "orchestrator".to_string()),
        ("advisory_only".to_string(), "false".to_string()),
        ("allowed_write_scope".to_string(), "[]".to_string()),
        ("timeout_sec".to_string(), "30".to_string()),
        ("max_retries".to_string(), "1".to_string()),
        (
            "canonical_input_summary".to_string(),
            "canonical summary for test".to_string(),
        ),
        (
            "projection_summary".to_string(),
            "projection summary for test".to_string(),
        ),
        (
            "constraint_summary".to_string(),
            "constraint summary for test".to_string(),
        ),
        (
            "workflow_discipline_summary".to_string(),
            "workflow summary for test".to_string(),
        ),
        (
            "output_schema_summary".to_string(),
            "output schema summary for test".to_string(),
        ),
        ("artifact_kind".to_string(), "readiness_check".to_string()),
        ("forbidden_actions".to_string(), "[]".to_string()),
        ("completion_criteria".to_string(), "[]".to_string()),
        ("verification_commands".to_string(), "[]".to_string()),
    ])
}

fn required_sections() -> Vec<PromptSection> {
    vec![
        PromptSection::System,
        PromptSection::NodeContract,
        PromptSection::CanonicalInputs,
        PromptSection::ProjectionSummary,
        PromptSection::ConstraintSummary,
        PromptSection::WorkflowDiscipline,
        PromptSection::OutputSchema,
        PromptSection::CompletionOrFailure,
    ]
}

fn assert_section_order(prompt: &str) {
    let mut last = 0;
    for section in [
        "[system]",
        "[node_contract]",
        "[canonical_inputs]",
        "[projection_summary]",
        "[constraint_summary]",
        "[workflow_discipline]",
        "[output_schema]",
        "[completion_or_failure]",
    ] {
        let index = prompt.find(section).expect("section exists");
        assert!(index >= last, "{section} rendered out of order");
        last = index;
    }
}
