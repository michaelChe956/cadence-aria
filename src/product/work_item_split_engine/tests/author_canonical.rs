fn canonical_author_output(outline_id: &str, logical_work_item_id: &str) -> serde_json::Value {
    let contract =
        crate::product::work_item_contract::canonical_contract_fixture(logical_work_item_id);
    let verification_checks = contract.verification_checks.clone();
    serde_json::json!({
        "draft": {
            "outline_id": outline_id,
            "logical_work_item_id": logical_work_item_id,
            "target_repository_id": null,
            "canonical_contract": contract,
            "verification_plan": {
                "checks": verification_checks
            }
        }
    })
}

#[test]
fn work_item_plan_author_canonical_parses_bare_and_enveloped_candidates() {
    let envelope = canonical_author_output("outline_core", "wi_core");
    let bare = envelope["draft"].clone();

    for output in [bare, envelope] {
        let candidate = parse_work_item_draft_output(output).expect("canonical draft output");

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
}

#[test]
fn work_item_plan_author_canonical_bare_and_envelope_remain_closed() {
    let mut envelope = canonical_author_output("outline_core", "wi_core");
    envelope["unexpected_envelope_field"] = serde_json::json!(true);
    let envelope_error =
        parse_work_item_draft_output(envelope).expect_err("envelope must stay closed");
    assert_eq!(envelope_error.code, "work_item_draft_parse_error");

    let envelope = canonical_author_output("outline_core", "wi_core");
    let mut bare = envelope["draft"].clone();
    bare["unexpected_candidate_field"] = serde_json::json!(true);
    let bare_error = parse_work_item_draft_output(bare).expect_err("bare candidate must stay closed");
    assert_eq!(bare_error.code, "work_item_draft_parse_error");
}

#[test]
fn work_item_plan_author_canonical_rejects_unknown_nested_fields_for_bare_and_envelope() {
    let envelope = canonical_author_output("outline_core", "wi_core");
    let base_candidate = envelope["draft"].clone();
    let mut cases = Vec::new();

    let mut candidate = base_candidate.clone();
    candidate["verification_plan"]["unexpected_execution_view_field"] =
        serde_json::json!(true);
    cases.push(("verification_plan top-level", candidate));

    let mut candidate = base_candidate.clone();
    candidate["canonical_contract"]["human_summary"] =
        serde_json::json!("presentation must stay outside Canonical");
    cases.push(("canonical_contract human field", candidate));

    let mut candidate = base_candidate;
    candidate["canonical_contract"]["tasks"][0]["unexpected_task_field"] =
        serde_json::json!(true);
    cases.push(("canonical_contract deep task object", candidate));

    let mut accepted = Vec::new();
    for (case, bare) in cases {
        for (form, output) in [
            ("bare", bare.clone()),
            ("envelope", serde_json::json!({ "draft": bare })),
        ] {
            match parse_work_item_draft_output(output) {
                Ok(_) => accepted.push(format!("{case} / {form}")),
                Err(error) => assert_eq!(error.code, "work_item_draft_parse_error"),
            }
        }
    }

    assert!(
        accepted.is_empty(),
        "nested unknown fields were accepted: {}",
        accepted.join(", ")
    );
}

#[test]
fn work_item_plan_author_canonical_legitimate_nested_structures_roundtrip() {
    let candidate = parse_work_item_draft_output(canonical_author_output(
        "outline_core",
        "wi_core",
    ))
    .expect("canonical draft output");

    let contract_json = serde_json::to_value(&candidate.canonical_contract_candidate)
        .expect("serialize canonical contract");
    let contract_roundtrip = serde_json::from_value::<
        crate::product::work_item_contract::CanonicalWorkItemContract,
    >(contract_json)
    .expect("deserialize canonical contract");
    assert_eq!(
        contract_roundtrip,
        candidate.canonical_contract_candidate
    );

    let execution_view_json =
        serde_json::to_value(&candidate.verification_plan).expect("serialize execution view");
    let execution_view_roundtrip = serde_json::from_value::<
        crate::product::models::WorkItemDraftVerificationPlan,
    >(execution_view_json)
    .expect("deserialize execution view");
    assert_eq!(execution_view_roundtrip, candidate.verification_plan);
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
    let mut envelope = canonical_author_output("outline_core", "wi_core");
    envelope["draft"]["implementation_context"] =
        serde_json::json!("legacy coder-facing narrative");
    let bare = envelope["draft"].clone();

    for output in [bare, envelope] {
        let error =
            parse_work_item_draft_output(output).expect_err("legacy field must be rejected");
        assert_eq!(error.code, "work_item_draft_forbidden_field");
    }
}

#[test]
fn work_item_plan_author_canonical_rejects_logical_identity_mismatch() {
    let mut envelope = canonical_author_output("outline_core", "wi_core");
    envelope["draft"]["logical_work_item_id"] = serde_json::json!("wi_other");
    let bare = envelope["draft"].clone();

    for output in [bare, envelope] {
        let error = parse_work_item_draft_output(output).expect_err("identity mismatch");
        assert_eq!(error.code, "work_item_draft_identity_mismatch");
    }
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
fn work_item_plan_author_canonical_schema_enforces_stable_identity_constraints() {
    let schema: serde_json::Value = serde_json::from_str(
        crate::product::work_item_split_engine::schema::WORK_ITEM_DRAFT_OUTPUT_SCHEMA,
    )
    .expect("draft schema json");
    let draft = &schema["properties"]["draft"];
    let contract = &draft["properties"]["canonical_contract"];
    let properties = &contract["properties"];

    for id_schema in [
        &draft["properties"]["logical_work_item_id"],
        &properties["identity"]["properties"]["logical_work_item_id"],
        &properties["input_contracts"]["items"]["properties"]["contract_id"],
        &properties["input_contracts"]["items"]["properties"]
            ["provider_logical_work_item_id"],
        &properties["output_contracts"]["items"]["properties"]["contract_id"],
        &properties["tasks"]["items"]["properties"]["task_id"],
        &properties["acceptance_criteria"]["items"]["properties"]["criterion_id"],
        &properties["verification_checks"]["items"]["properties"]["check_id"],
        &properties["blocker_rules"]["items"]["properties"]["reason_code"],
        &draft["properties"]["verification_plan"]["properties"]["checks"]["items"]
            ["properties"]["check_id"],
    ] {
        assert_eq!(id_schema["minLength"], 1, "missing minLength: {id_schema}");
    }

    for collection in [
        &properties["input_contracts"],
        &properties["output_contracts"],
        &properties["tasks"],
        &properties["acceptance_criteria"],
        &properties["verification_checks"],
        &properties["blocker_rules"],
        &draft["properties"]["verification_plan"]["properties"]["checks"],
    ] {
        assert_eq!(collection["uniqueItems"], true, "missing uniqueItems: {collection}");
    }

    let handoff = &properties["handoff_contract"]["properties"];
    for reference_list in [
        &handoff["required_fields"],
        &handoff["provided_contract_refs"],
        &handoff["reviewer_check_refs"],
    ] {
        assert_eq!(reference_list["uniqueItems"], true);
        assert_eq!(reference_list["items"]["minLength"], 1);
    }
}

#[test]
fn work_item_plan_author_canonical_schema_requires_non_empty_command_for_required_checks() {
    let schema: serde_json::Value = serde_json::from_str(
        crate::product::work_item_split_engine::schema::WORK_ITEM_DRAFT_OUTPUT_SCHEMA,
    )
    .expect("draft schema json");
    let draft = &schema["properties"]["draft"];
    let contract_check = &draft["properties"]["canonical_contract"]["properties"]
        ["verification_checks"]["items"];
    let projection_check =
        &draft["properties"]["verification_plan"]["properties"]["checks"]["items"];

    for check in [contract_check, projection_check] {
        assert_eq!(check["allOf"][0]["if"]["properties"]["required"]["const"], true);
        assert_eq!(
            check["allOf"][0]["then"]["properties"]["command"]["type"],
            "string"
        );
        assert_eq!(
            check["allOf"][0]["then"]["properties"]["command"]["minLength"],
            1
        );
    }
}

#[test]
fn work_item_plan_outline_schema_requires_closed_trusted_verification_command_catalog() {
    let schema: serde_json::Value = serde_json::from_str(
        crate::product::work_item_split_engine::schema::WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA,
    )
    .expect("outline schema json");
    let outline_item = &schema["properties"]["outline"]["properties"]["work_item_outlines"]
        ["items"];
    let catalog = &outline_item["properties"]["trusted_verification_commands"];

    assert_eq!(catalog["type"], "array");
    assert_eq!(catalog["maxItems"], 3);
    assert_required_closed_array_items(catalog);
    for field in ["command", "cwd", "purpose", "source_ref"] {
        assert_eq!(
            catalog["items"]["properties"][field]["maxLength"],
            match field {
                "command" => 48,
                "cwd" => 16,
                "purpose" => 32,
                "source_ref" => 32,
                _ => unreachable!("only trusted catalog fields are checked"),
            },
            "trusted command catalog field {field} must have a prompt-budget bound"
        );
    }
    assert!(
        outline_item["required"]
            .as_array()
            .expect("outline item required fields")
            .iter()
            .any(|field| field == "trusted_verification_commands"),
        "trusted command catalog must be an explicit outline field"
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
    for stable_identity_field in [
        "contract_id",
        "task_id",
        "criterion_id",
        "check_id",
        "reason_code",
    ] {
        assert!(
            invocation.prompt.contains(stable_identity_field),
            "prompt must require stable identity field {stable_identity_field}: {}",
            invocation.prompt
        );
    }
    assert!(invocation.prompt.contains("handoff_contract 是 Canonical singleton"));
    assert!(invocation.prompt.contains("required_fields"));
    assert!(invocation.prompt.contains("provided_contract_refs"));
    assert!(invocation.prompt.contains("reviewer_check_refs"));
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

#[test]
fn work_item_plan_author_canonical_prompt_is_provider_neutral_about_verification_commands() {
    let mut outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");
    for item in &mut outline.work_item_outlines {
        item.verification_intent.clear();
        item.trusted_verification_commands.clear();
    }

    for outline_id in ["outline_backend", "outline_frontend"] {
        let invocation = build_work_item_draft_invocation(
            &outline,
            outline_id,
            WorkItemGenerationMode::Serial,
            &[],
            None,
        )
        .expect("draft invocation");

        for fixed_command in ["cargo test", "pnpm", "mvn", "gradle"] {
            assert!(
                !invocation.prompt.contains(fixed_command),
                "prompt must not prescribe {fixed_command} for {outline_id}: {}",
                invocation.prompt
            );
        }
        assert!(invocation.prompt.contains("目标仓库的可信证据"));
        assert!(invocation.prompt.contains("不得根据 WorkItemKind 推导"));
        assert!(invocation.prompt.contains("manual/repair/blocker"));
        assert!(
            !invocation
                .prompt
                .contains("每个 draft 必须给出后续 coding agent 可执行的目标、范围、非目标、TDD 顺序、验证命令")
        );
        // 结构化验证方案要求改由 [canonical_field_contract] 简写记号承载（语义保留）。
        assert!(invocation.prompt.contains("verification_plan: obj{checks:"));
        // 逐字段复制硬规则与 field_contract 重复，已删除；精确一致性由 [self_check] 保留。
        assert!(
            invocation
                .prompt
                .contains("verification_plan 与 canonical checks 的逐字段同序相等")
        );
    }
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
