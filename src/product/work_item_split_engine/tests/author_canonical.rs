fn canonical_author_output(outline_id: &str, logical_work_item_id: &str) -> serde_json::Value {
    let contract =
        crate::product::work_item_contract::canonical_contract_fixture(logical_work_item_id);
    let verification_checks = contract.verification_checks.clone();
    serde_json::json!({
        "draft": {
            "outline_id": outline_id,
            "logical_work_item_id": logical_work_item_id,
            "canonical_contract": contract,
            "verification_plan": {
                "checks": verification_checks
            }
        }
    })
}

#[test]
fn work_item_plan_author_canonical_parses_canonical_contract_candidate() {
    let candidate = parse_work_item_draft_output(canonical_author_output("outline_core", "wi_core"))
        .expect("canonical draft output");

    assert_eq!(candidate.outline_id, "outline_core");
    assert_eq!(candidate.logical_work_item_id, "wi_core");
    assert_eq!(
        candidate
            .canonical_contract_candidate
            .identity
            .logical_work_item_id,
        "wi_core"
    );
    assert_eq!(
        candidate.verification_plan.checks,
        candidate.canonical_contract_candidate.verification_checks
    );
}

#[test]
fn work_item_plan_author_canonical_accepts_structured_json_object_only() {
    let raw_json = canonical_author_output("outline_core", "wi_core").to_string();
    let structured: serde_json::Value = serde_json::from_str(&raw_json).expect("raw json");

    assert!(parse_work_item_draft_output(structured).is_ok());

    for unstructured in [
        format!("```json\n{raw_json}\n```"),
        format!("Here is the requested draft:\n{raw_json}"),
        format!("{raw_json}\nDraft generation complete."),
    ] {
        let error = parse_work_item_draft_output(serde_json::json!(unstructured))
            .expect_err("free text must not be scanned for JSON");
        assert_eq!(error.code, "work_item_draft_parse_error");
    }
}

#[test]
fn work_item_plan_author_canonical_rejects_legacy_implementation_context() {
    let mut output = canonical_author_output("outline_core", "wi_core");
    output["draft"]["implementation_context"] =
        serde_json::json!("legacy coder-facing narrative");

    let error = parse_work_item_draft_output(output).expect_err("legacy field must be rejected");

    assert_eq!(error.code, "work_item_draft_forbidden_field");
}

#[test]
fn work_item_plan_author_canonical_rejects_logical_identity_mismatch() {
    let mut output = canonical_author_output("outline_core", "wi_core");
    output["draft"]["logical_work_item_id"] = serde_json::json!("wi_other");

    let error = parse_work_item_draft_output(output).expect_err("identity mismatch");

    assert_eq!(error.code, "work_item_draft_identity_mismatch");
}

#[test]
fn work_item_plan_author_canonical_schema_is_required_and_closed() {
    let schema: serde_json::Value = serde_json::from_str(
        crate::product::work_item_split_engine::schema::WORK_ITEM_DRAFT_OUTPUT_SCHEMA,
    )
    .expect("draft schema json");

    assert_required_closed_object(&schema);
    assert_required_closed_object(&schema["properties"]["draft"]);
    assert_required_closed_object(
        &schema["properties"]["draft"]["properties"]["canonical_contract"],
    );
    assert_required_closed_object(
        &schema["properties"]["draft"]["properties"]["canonical_contract"]["properties"]
            ["identity"],
    );
    assert_required_closed_object(
        &schema["properties"]["draft"]["properties"]["canonical_contract"]["properties"]
            ["goal"],
    );
    assert_required_closed_array_items(
        &schema["properties"]["draft"]["properties"]["canonical_contract"]["properties"]
            ["input_contracts"],
    );
    assert_required_closed_array_items(
        &schema["properties"]["draft"]["properties"]["canonical_contract"]["properties"]
            ["output_contracts"],
    );
    assert_required_closed_array_items(
        &schema["properties"]["draft"]["properties"]["canonical_contract"]["properties"]
            ["tasks"],
    );
    assert_required_closed_object(
        &schema["properties"]["draft"]["properties"]["canonical_contract"]["properties"]
            ["write_policy"],
    );
    assert_required_closed_array_items(
        &schema["properties"]["draft"]["properties"]["canonical_contract"]["properties"]
            ["acceptance_criteria"],
    );
    assert_required_closed_array_items(
        &schema["properties"]["draft"]["properties"]["canonical_contract"]["properties"]
            ["verification_checks"],
    );
    assert_required_closed_object(
        &schema["properties"]["draft"]["properties"]["canonical_contract"]["properties"]
            ["handoff_contract"],
    );
    assert_required_closed_array_items(
        &schema["properties"]["draft"]["properties"]["canonical_contract"]["properties"]
            ["blocker_rules"],
    );
    assert_required_closed_array_items(
        &schema["properties"]["draft"]["properties"]["canonical_contract"]["properties"]
            ["design_traceability"],
    );
    assert_required_closed_object(
        &schema["properties"]["draft"]["properties"]["verification_plan"],
    );
    assert_required_closed_array_items(
        &schema["properties"]["draft"]["properties"]["verification_plan"]["properties"]
            ["checks"],
    );

    let contract_properties =
        &schema["properties"]["draft"]["properties"]["canonical_contract"]["properties"];
    assert_eq!(
        contract_properties["identity"]["properties"]["kind"]["enum"],
        serde_json::json!(["backend", "frontend", "integration", "e2e", "docs", "infra", "other"])
    );
    assert_eq!(
        contract_properties["input_contracts"]["items"]["properties"]
            ["compatibility_policy"]["enum"],
        serde_json::json!(["require_all", "require_any"])
    );
    assert_eq!(
        contract_properties["acceptance_criteria"]["items"]["properties"]
            ["required_evidence"]["items"]["enum"],
        serde_json::json!([
            "source_diff",
            "non_zero_test_execution",
            "manual_check",
            "handoff_field"
        ])
    );
    assert_eq!(
        contract_properties["blocker_rules"]["items"]["properties"]["route"]["enum"],
        serde_json::json!([
            "coder_rework",
            "verification_retry",
            "plan_repair_current",
            "plan_repair_upstream",
            "subgraph_replan",
            "story_amendment",
            "design_amendment",
            "operational_gate"
        ])
    );
    assert!(
        !crate::product::work_item_split_engine::schema::WORK_ITEM_DRAFT_OUTPUT_SCHEMA
            .contains("implementation_context")
    );
}

#[test]
fn work_item_plan_author_canonical_prompt_requests_contract_candidate_only() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");
    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
    )
    .expect("draft invocation");

    assert!(invocation.prompt.contains("只输出 Canonical Contract Candidate"));
    assert!(invocation.prompt.contains("canonical_contract"));
    assert!(invocation.prompt.contains("logical_work_item_id"));
    for stable_id in [
        "input contract",
        "output contract",
        "task",
        "acceptance",
        "verification",
        "handoff",
        "blocker rule",
    ] {
        assert!(
            invocation.prompt.contains(stable_id),
            "prompt must require stable ID for {stable_id}: {}",
            invocation.prompt
        );
    }
    assert!(
        invocation
            .prompt
            .contains("不得输出面向 Coder 的长篇 implementation_context")
    );
    assert!(!invocation.prompt.contains("\"implementation_context\":"));
    assert!(
        invocation
            .prompt
            .contains("不要提前生成或渲染 Coder Projection 或 Reviewer Projection")
    );
}

fn assert_required_closed_array_items(schema: &serde_json::Value) {
    assert_eq!(schema["type"], "array");
    assert_required_closed_object(&schema["items"]);
}

fn assert_required_closed_object(schema: &serde_json::Value) {
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], false);
    let mut properties = schema["properties"]
        .as_object()
        .expect("object properties")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let mut required = schema["required"]
        .as_array()
        .expect("required array")
        .iter()
        .map(|value| value.as_str().expect("required string").to_string())
        .collect::<Vec<_>>();
    properties.sort();
    required.sort();
    assert_eq!(required, properties);
}
