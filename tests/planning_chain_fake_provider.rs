use cadence_aria::cross_cutting::provider_adapter::{
    ProviderAdapter, ProviderAdapterError, STRUCTURED_OUTPUT_END, STRUCTURED_OUTPUT_START,
    parse_last_structured_output,
};
use cadence_aria::protocol::constraints::{
    BundleStatus, CoverageModel, DesignConstraints, OpenSpecConstraintBundle, ProposalConstraints,
    RequirementConstraints, TaskConstraints, TraceabilityRequirements,
};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
use cadence_aria::protocol::projections::ProjectionPayload;
use cadence_aria::runtime_units::plan_dispatch::run_planning_full_chain;
use cadence_aria::runtime_units::spec_gate_review::{
    PlanningStartChainInput, run_planning_start_chain,
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

#[test]
fn fake_provider_runs_n04_to_n07_with_spec_writeback_and_requirement_gate() {
    let workspace = tempfile::tempdir().expect("workspace");
    let change_id = "sample-change".to_string();
    let change_dir = prepare_change_dir(workspace.path(), &change_id);

    let provider = ScriptedPlanningProvider::default();
    let result = run_planning_start_chain(planning_input(workspace.path(), &change_id), &provider)
        .expect("planning start chain");

    assert_eq!(
        result
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["N04", "N05", "N06", "N07"]
    );
    assert_eq!(
        provider.seen_output_schemas(),
        vec![
            "schema://aria/artifacts/clarification_record/v1".to_string(),
            "schema://aria/artifacts/spec/v1".to_string(),
            "schema://aria/advisory/spec_gate_review/v1".to_string(),
            "schema://aria/artifacts/design/v1".to_string(),
        ]
    );
    assert_eq!(
        result.clarification_record["artifact_kind"],
        json!("clarification_record")
    );
    assert_eq!(result.spec_gate_decision["decision"], json!("pass"));
    assert!(
        result
            .spec_projection
            .projection_id
            .starts_with("proj_spec_projection_"),
        "N05 must compile SpecProjection"
    );
    assert!(
        result
            .design_projection
            .projection_id
            .starts_with("proj_design_projection_"),
        "N07 must compile DesignProjection"
    );
    assert_eq!(result.spec_writeback_stale_status, BundleStatus::Stale);
    assert!(
        !result
            .openspec_bundle_after_spec
            .requirement_constraints
            .requirement_ids
            .is_empty(),
        "N06 pass must recompile nonempty requirement_constraints before N07"
    );
    assert!(
        fs::read_to_string(change_dir.join("specs/main/spec.md"))
            .expect("written openspec spec")
            .contains("#### Requirement: REQ-001"),
        "N06 pass must write canonical spec into OpenSpec spec.md"
    );

    let n06_trace = result
        .node_traces
        .iter()
        .find(|trace| trace.node_id == "N06")
        .expect("N06 trace");
    assert!(
        !n06_trace
            .consumed_constraint_kinds
            .contains(&"requirement_constraints".to_string()),
        "N06 must not consume requirement_constraints before pass"
    );
    let n07_trace = result
        .node_traces
        .iter()
        .find(|trace| trace.node_id == "N07")
        .expect("N07 trace");
    assert_eq!(
        n07_trace.consumed_constraint_kinds,
        vec!["requirement_constraints".to_string()]
    );

    for trace in &result.node_traces {
        assert_eq!(
            trace.execution_chain,
            vec![
                "canonical_node_input",
                "projection_or_bundle",
                "provider_context_package",
                "adapter_input",
                "provider_call",
                "provider_run_record",
                "normalize_output",
                "artifact_validate",
                "checkpoint",
            ],
            "{} must use the unified execution chain",
            trace.node_id
        );
    }
    assert_eq!(result.provider_run_records.len(), 4);
    assert!(
        result.checkpoint_paths.iter().all(|path| path.exists()),
        "each node must write a checkpoint snapshot"
    );
}

#[test]
fn planning_provider_retries_first_parse_error_for_node() {
    let workspace = tempfile::tempdir().expect("workspace");
    let change_id = "sample-change".to_string();
    prepare_change_dir(workspace.path(), &change_id);
    let provider = FlakySpecProvider::default();

    let result = run_planning_start_chain(planning_input(workspace.path(), &change_id), &provider)
        .expect("planning start chain should retry transient parse error");

    assert!(
        result
            .spec_projection
            .projection_id
            .starts_with("proj_spec_projection_")
    );
    assert_eq!(*provider.spec_attempts.lock().expect("attempts"), 2);
    let n05_record: serde_json::Value = serde_json::from_slice(
        &fs::read(
            workspace
                .path()
                .join(".aria/runtime/tasks/task_0001/provider-runs/prun_task_0001_n05.json"),
        )
        .expect("read provider run"),
    )
    .expect("provider run json");
    assert_eq!(n05_record["retry_count"], json!(1));
}

#[test]
fn fake_provider_runs_n04_to_n12_happy_path_with_design_tasks_and_dispatch() {
    let workspace = tempfile::tempdir().expect("workspace");
    let change_id = "sample-change".to_string();
    let change_dir = prepare_change_dir(workspace.path(), &change_id);
    let provider = ScriptedPlanningProvider::default();

    let result = run_planning_full_chain(planning_input(workspace.path(), &change_id), &provider)
        .expect("planning full chain");

    assert_eq!(
        result
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["N04", "N05", "N06", "N07", "N08", "N10", "N11", "N12"]
    );
    assert_eq!(result.design_review["review_decision"], json!("pass"));
    assert_eq!(result.design_writeback_stale_status, BundleStatus::Stale);
    assert_eq!(
        result
            .openspec_bundle_after_design
            .design_constraints
            .design_decision_ids,
        vec!["DD-001".to_string()]
    );
    assert!(
        fs::read_to_string(change_dir.join("design.md"))
            .expect("written design")
            .contains("DD-001"),
        "N08 pass must write stable design.md before N10/N11"
    );
    assert_eq!(result.readiness_check["ready"], json!(true));

    let ProjectionPayload::PlanProjection(plan_payload) = &result.plan_projection.payload else {
        panic!("expected plan projection payload");
    };
    assert_eq!(plan_payload.work_packages[0].work_package_id, "wt-001");
    assert_eq!(result.tasks_writeback_stale_status, BundleStatus::Stale);
    assert_eq!(
        result.openspec_bundle_after_tasks.task_constraints.task_ids,
        vec!["TASK-001".to_string(), "TASK-002".to_string()]
    );
    assert!(
        fs::read_to_string(change_dir.join("tasks.md"))
            .expect("written tasks")
            .contains("TASK-001"),
        "N11 must write OpenSpec task constraints"
    );

    assert_eq!(
        result.dispatch_package["artifact_kind"],
        json!("dispatch_package")
    );
    let routing = result.dispatch_package["_aria"]["worktask_routing"]
        .as_array()
        .expect("routing array");
    assert_eq!(routing[0]["source_work_package_id"], json!("wt-001"));
    assert_eq!(routing[0]["execution_mode"], json!("agent_only"));
    assert!(
        routing
            .iter()
            .all(|item| item.get("source_work_package_id").is_some())
    );

    for node_id in ["N10", "N11", "N12"] {
        assert!(
            result
                .protocol_steps
                .iter()
                .any(|step| step.node_id == node_id),
            "plan_dispatch must report separate {node_id} protocol step"
        );
        assert!(
            result.checkpoint_paths.iter().any(|path| path
                .file_name()
                .is_some_and(|name| name.to_string_lossy() == format!("{node_id}.json"))),
            "plan_dispatch must write {node_id} checkpoint"
        );
    }
    let readiness_prompts =
        provider.seen_prompts_for_schema("schema://aria/artifacts/readiness_check/v1");
    assert!(
        readiness_prompts[0].contains("[spec_projection_payload]")
            && readiness_prompts[0].contains("[design_projection_payload]")
            && readiness_prompts[0].contains("design_decisions"),
        "N10 prompt must inline projection payloads for real providers without tools"
    );
    let plan_prompts = provider.seen_prompts_for_schema("schema://aria/artifacts/plan/v1");
    assert!(
        plan_prompts[0].contains("[spec_projection_payload]")
            && plan_prompts[0].contains("[design_projection_payload]")
            && plan_prompts[0].contains("success_criteria"),
        "N11 prompt must inline spec/design projection payloads"
    );
    let dispatch_prompts =
        provider.seen_prompts_for_schema("schema://aria/artifacts/dispatch_package/v1");
    assert!(
        dispatch_prompts[0].contains("[plan_projection_payload]")
            && dispatch_prompts[0].contains("work_packages"),
        "N12 prompt must inline plan projection payload"
    );
}

#[test]
fn planning_chain_writes_openspec_design_from_synthesized_projection_ids() {
    let workspace = tempfile::tempdir().expect("workspace");
    let change_id = "sample-change".to_string();
    let change_dir = prepare_change_dir(workspace.path(), &change_id);
    let provider = NoIdDesignProvider::default();

    let result = run_planning_full_chain(planning_input(workspace.path(), &change_id), &provider)
        .expect("planning full chain");

    assert_eq!(
        result
            .openspec_bundle_after_design
            .design_constraints
            .design_decision_ids,
        vec!["DEC-001".to_string()]
    );
    assert_eq!(
        result
            .openspec_bundle_after_design
            .design_constraints
            .component_ids,
        vec!["CMP-001".to_string()]
    );
    let written_design = fs::read_to_string(change_dir.join("design.md")).expect("written design");
    assert!(written_design.contains("[DEC-001]"));
    assert!(written_design.contains("[CMP-001]"));
    assert_eq!(
        result
            .openspec_bundle_after_tasks
            .task_constraints
            .related_design_decision_ids_by_task
            .get("TASK-001"),
        Some(&vec!["DEC-001".to_string()])
    );
}

#[test]
fn planning_chain_dispatch_does_not_hardcode_aria_cargo_verification_commands() {
    let workspace = tempfile::tempdir().expect("workspace");
    let change_id = "sample-change".to_string();
    prepare_change_dir(workspace.path(), &change_id);
    let provider = ScriptedPlanningProvider::default();

    let result = run_planning_full_chain(planning_input(workspace.path(), &change_id), &provider)
        .expect("planning full chain");

    let routing = result
        .dispatch_package
        .get("worktask_routing")
        .and_then(Value::as_array)
        .expect("worktask routing");
    assert!(
        routing.iter().all(|route| route
            .get("verification_commands")
            .and_then(Value::as_array)
            .is_some_and(|commands| commands.is_empty())),
        "N12 must not inject Aria-internal cargo tests into target worktree routing"
    );
}

#[test]
fn planning_chain_allows_package_json_when_plan_work_package_targets_test_script() {
    let workspace = tempfile::tempdir().expect("workspace");
    let change_id = "sample-change".to_string();
    prepare_change_dir(workspace.path(), &change_id);
    let provider = PackageJsonPlanProvider::default();

    let result = run_planning_full_chain(planning_input(workspace.path(), &change_id), &provider)
        .expect("planning full chain");

    let routing = result
        .dispatch_package
        .get("worktask_routing")
        .and_then(Value::as_array)
        .expect("worktask routing");
    let package_route = routing
        .iter()
        .find(|route| route["source_work_package_id"] == json!("wt-003"))
        .expect("package.json route");
    assert_eq!(
        package_route["allowed_write_scope"],
        json!(["src/", "tests/", "package.json"])
    );
}

#[test]
fn fake_provider_routes_revise_review_to_n09_and_back_to_n08() {
    let workspace = tempfile::tempdir().expect("workspace");
    let change_id = "sample-change".to_string();
    prepare_change_dir(workspace.path(), &change_id);
    let provider = ScriptedPlanningProvider::with_review_decisions(["revise", "pass"]);

    let result = run_planning_full_chain(planning_input(workspace.path(), &change_id), &provider)
        .expect("planning full chain with revise");

    assert_eq!(
        result
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "N04", "N05", "N06", "N07", "N08", "N09", "N08", "N10", "N11", "N12"
        ]
    );
    assert_eq!(
        result
            .design_revision_record
            .as_ref()
            .expect("revision record")["artifact_kind"],
        json!("design_revision_record")
    );
    assert!(
        result.design_markdown.contains("修订后"),
        "N09 must provide the revised design consumed by the second N08"
    );
    assert!(
        result
            .superseded_artifact_refs
            .contains(&result.initial_design_ref.artifact_ref_id),
        "N09 must mark the pre-revision design as superseded"
    );
    assert_eq!(result.design_review["review_decision"], json!("pass"));
    assert_eq!(
        provider
            .seen_output_schemas()
            .into_iter()
            .filter(|schema| schema == "schema://aria/artifacts/design_review/v1")
            .count(),
        2,
        "N08 must run again after N09 revision"
    );
    let design_review_prompts =
        provider.seen_prompts_for_schema("schema://aria/artifacts/design_review/v1");
    assert!(
        design_review_prompts[1].contains("修订后 REPL 只作为客户端"),
        "second N08 prompt must include revised design markdown from N09"
    );
    let design_revision_prompts =
        provider.seen_prompts_for_schema("schema://aria/artifacts/design_revision_record/v1");
    assert_eq!(design_revision_prompts.len(), 1);
    assert!(
        design_revision_prompts[0].contains("REPL 只作为客户端"),
        "N09 prompt must inline the current design markdown because real providers run without tools"
    );
    assert!(
        design_revision_prompts[0].contains("finding-001"),
        "N09 prompt must inline concrete design review findings"
    );
    assert!(
        design_revision_prompts[0].contains("[spec_projection_payload]")
            && design_revision_prompts[0].contains("success_criteria"),
        "N09 prompt must inline spec projection payload so revisions preserve original spec constraints"
    );
    assert!(
        design_revision_prompts[0]
            .contains("revised_design_markdown 必须包含 canonical Design heading"),
        "N09 prompt must explicitly preserve canonical Design headings"
    );
}

#[test]
fn fake_provider_routes_fail_review_to_n09_and_back_to_n08() {
    let workspace = tempfile::tempdir().expect("workspace");
    let change_id = "sample-change".to_string();
    prepare_change_dir(workspace.path(), &change_id);
    let provider = ScriptedPlanningProvider::with_review_decisions(["fail", "pass"]);

    let result = run_planning_full_chain(planning_input(workspace.path(), &change_id), &provider)
        .expect("planning full chain with fail review");

    assert_eq!(
        result
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "N04", "N05", "N06", "N07", "N08", "N09", "N08", "N10", "N11", "N12"
        ]
    );
    assert_eq!(result.design_review["review_decision"], json!("pass"));
}

#[test]
fn planning_chain_treats_changes_requested_review_decision_as_revision_request() {
    let workspace = tempfile::tempdir().expect("workspace");
    let change_id = "sample-change".to_string();
    prepare_change_dir(workspace.path(), &change_id);
    let provider = ScriptedPlanningProvider::with_review_decisions(["changes_requested", "pass"]);

    let result = run_planning_full_chain(planning_input(workspace.path(), &change_id), &provider)
        .expect("changes_requested should route through design revision");

    assert_eq!(
        result
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "N04", "N05", "N06", "N07", "N08", "N09", "N08", "N10", "N11", "N12"
        ]
    );
    assert_eq!(result.design_review["review_decision"], json!("pass"));
}

#[test]
fn planning_chain_stops_design_revision_loop_at_registered_threshold() {
    let workspace = tempfile::tempdir().expect("workspace");
    let change_id = "sample-change".to_string();
    prepare_change_dir(workspace.path(), &change_id);
    let provider =
        ScriptedPlanningProvider::with_review_decisions(["fail", "fail", "fail", "fail", "pass"]);

    let error = run_planning_full_chain(planning_input(workspace.path(), &change_id), &provider)
        .expect_err("design revision loop must stop at the registered threshold");

    assert!(
        error.to_string().contains("design_revision_limit_exceeded"),
        "unexpected error: {error}"
    );
}

#[test]
fn planning_chain_records_node_enter_and_exit_events() {
    let workspace = tempfile::tempdir().expect("workspace");
    let change_id = "sample-change".to_string();
    prepare_change_dir(workspace.path(), &change_id);
    let provider = ScriptedPlanningProvider::default();

    run_planning_full_chain(planning_input(workspace.path(), &change_id), &provider)
        .expect("planning full chain");

    let event_log = workspace
        .path()
        .join(".aria/runtime/tasks/task_0001/logs/node-events.jsonl");
    let events = fs::read_to_string(&event_log)
        .unwrap_or_else(|error| panic!("read {}: {error}", event_log.display()));
    assert!(
        events.contains(r#""event_kind":"node_enter""#) && events.contains(r#""node_id":"N04""#),
        "N04 enter event missing: {events}"
    );
    assert!(
        events.contains(r#""event_kind":"node_exit""#)
            && events.contains(r#""node_id":"N12""#)
            && events.contains(r#""status":"completed""#),
        "N12 completed exit event missing: {events}"
    );
}

#[test]
fn planning_chain_restores_openspec_change_after_provider_side_effects() {
    let workspace = tempfile::tempdir().expect("workspace");
    let change_id = "sample-change".to_string();
    let change_dir = prepare_change_dir(workspace.path(), &change_id);
    let provider = DeletesOpenspecOnDesignProvider::default();
    let mut input = planning_input(workspace.path(), &change_id);
    input.worktree_path = Some(workspace.path().to_string_lossy().to_string());

    let result = run_planning_full_chain(input, &provider)
        .expect("planning chain should restore protected openspec files");

    assert_eq!(result.design_review["review_decision"], json!("pass"));
    assert!(
        change_dir.join("proposal.md").exists(),
        "proposal.md must be restored after provider side effects"
    );
    assert!(
        change_dir.join("specs/main/spec.md").exists(),
        "spec.md must be restored after provider side effects"
    );
    assert!(
        change_dir.join("design.md").exists(),
        "design.md must be restored before Aria writes stable design"
    );
    assert!(
        change_dir.join("tasks.md").exists(),
        "tasks.md must be restored after provider side effects"
    );
}

#[derive(Debug)]
struct ScriptedPlanningProvider {
    output_schemas: Mutex<Vec<String>>,
    prompts: Mutex<Vec<(String, String)>>,
    review_decisions: Mutex<VecDeque<String>>,
}

#[derive(Default)]
struct DeletesOpenspecOnDesignProvider {
    delegate: ScriptedPlanningProvider,
}

#[derive(Default)]
struct NoIdDesignProvider {
    delegate: ScriptedPlanningProvider,
}

#[derive(Default)]
struct PackageJsonPlanProvider {
    delegate: ScriptedPlanningProvider,
}

#[derive(Default)]
struct FlakySpecProvider {
    spec_attempts: Mutex<u32>,
    delegate: ScriptedPlanningProvider,
}

impl ProviderAdapter for FlakySpecProvider {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        if input.output_schema == "schema://aria/artifacts/spec/v1" {
            let mut attempts = self.spec_attempts.lock().expect("attempts");
            *attempts += 1;
            if *attempts == 1 {
                return Err(ProviderAdapterError::parse_error(
                    "missing structured output sentinel",
                    "provider log without sentinel",
                    "",
                ));
            }
        }
        self.delegate.run(input)
    }
}

impl ProviderAdapter for DeletesOpenspecOnDesignProvider {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        let output = self.delegate.run(input)?;
        if input.output_schema == "schema://aria/artifacts/design/v1"
            && let Some(worktree_path) = &input.worktree_path
        {
            let change_dir = Path::new(worktree_path).join("openspec/changes/sample-change");
            fs::remove_dir_all(change_dir).expect("delete openspec change dir");
        }
        Ok(output)
    }
}

impl ProviderAdapter for NoIdDesignProvider {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        match input.output_schema.as_str() {
            "schema://aria/artifacts/design/v1" => provider_output(json!({
                "artifact_kind": "design",
                "markdown": no_id_design_markdown()
            })),
            "schema://aria/artifacts/plan/v1" => provider_output(json!({
                "artifact_kind": "plan",
                "markdown": dec_traceability_plan_markdown()
            })),
            _ => self.delegate.run(input),
        }
    }
}

impl ProviderAdapter for PackageJsonPlanProvider {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        match input.output_schema.as_str() {
            "schema://aria/artifacts/plan/v1" => provider_output(json!({
                "artifact_kind": "plan",
                "markdown": package_json_plan_markdown()
            })),
            _ => self.delegate.run(input),
        }
    }
}

impl Default for ScriptedPlanningProvider {
    fn default() -> Self {
        Self::with_review_decisions(["pass"])
    }
}

fn provider_output(payload: serde_json::Value) -> Result<AdapterOutput, ProviderAdapterError> {
    let stdout = format!(
        "provider log\n{STRUCTURED_OUTPUT_START}\n{}\n{STRUCTURED_OUTPUT_END}\n",
        serde_json::to_string(&payload).expect("payload json")
    );
    let structured_output = parse_last_structured_output(&stdout)?;
    Ok(AdapterOutput {
        exit_code: Some(0),
        stdout,
        stderr: String::new(),
        structured_output,
        files_modified: vec![],
        duration_ms: 1,
        timeout_status: TimeoutStatus::NotTimedOut,
    })
}

impl ScriptedPlanningProvider {
    fn with_review_decisions<const N: usize>(decisions: [&str; N]) -> Self {
        Self {
            output_schemas: Mutex::new(Vec::new()),
            prompts: Mutex::new(Vec::new()),
            review_decisions: Mutex::new(
                decisions
                    .into_iter()
                    .map(ToOwned::to_owned)
                    .collect::<VecDeque<_>>(),
            ),
        }
    }

    fn seen_output_schemas(&self) -> Vec<String> {
        self.output_schemas.lock().expect("schemas").clone()
    }

    fn seen_prompts_for_schema(&self, output_schema: &str) -> Vec<String> {
        self.prompts
            .lock()
            .expect("prompts")
            .iter()
            .filter(|(schema, _)| schema == output_schema)
            .map(|(_, prompt)| prompt.clone())
            .collect()
    }
}

impl ProviderAdapter for ScriptedPlanningProvider {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.output_schemas
            .lock()
            .expect("schemas")
            .push(input.output_schema.clone());
        self.prompts
            .lock()
            .expect("prompts")
            .push((input.output_schema.clone(), input.prompt.clone()));
        let payload = match input.output_schema.as_str() {
            "schema://aria/artifacts/clarification_record/v1" => json!({
                "artifact_kind": "clarification_record",
                "goal_summary": "实现 Aria 规划链起始节点",
                "constraints": ["使用 Docker Rust 环境"],
                "assumptions": ["P1 已完成"],
                "open_questions": [],
                "suggested_scope": "N04-N07 fake provider chain"
            }),
            "schema://aria/artifacts/spec/v1" => json!({
                "artifact_kind": "spec",
                "markdown": canonical_spec_markdown()
            }),
            "schema://aria/advisory/spec_gate_review/v1" => json!({
                "artifact_kind": "advisory_review",
                "findings": [],
                "blocking_issues": [],
                "decision_recommendation": "pass"
            }),
            "schema://aria/artifacts/design/v1" => json!({
                "artifact_kind": "design",
                "markdown": canonical_design_markdown()
            }),
            "schema://aria/artifacts/design_review/v1" => {
                let decision = self
                    .review_decisions
                    .lock()
                    .expect("review decisions")
                    .pop_front()
                    .unwrap_or_else(|| "pass".to_string());
                json!({
                    "artifact_kind": "design_review",
                    "review_decision": decision,
                    "findings": if decision == "revise" {
                        json!([{"finding_id": "finding-001", "summary": "补充修订设计"}])
                    } else {
                        json!([])
                    }
                })
            }
            "schema://aria/artifacts/design_revision_record/v1" => json!({
                "artifact_kind": "design_revision_record",
                "revision_summary": "根据评审补充设计决策。",
                "resolved_findings": ["finding-001"],
                "revised_design_markdown": revised_design_markdown()
            }),
            "schema://aria/artifacts/readiness_check/v1" => json!({
                "artifact_kind": "readiness_check",
                "ready": true,
                "blocking_items": []
            }),
            "schema://aria/artifacts/plan/v1" => json!({
                "artifact_kind": "plan",
                "markdown": canonical_plan_markdown()
            }),
            "schema://aria/artifacts/dispatch_package/v1" => json!({
                "artifact_kind": "dispatch_package",
                "worktask_routing": []
            }),
            other => panic!("unexpected schema {other}"),
        };
        let stdout = format!(
            "provider log\n{STRUCTURED_OUTPUT_START}\n{}\n{STRUCTURED_OUTPUT_END}\n",
            serde_json::to_string(&payload).expect("payload json")
        );
        let structured_output = parse_last_structured_output(&stdout)?;
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout,
            stderr: String::new(),
            structured_output,
            files_modified: vec![],
            duration_ms: 1,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

fn canonical_spec_markdown() -> &'static str {
    "# Spec\n\n## 功能需求\n\n- [REQ-001] 用户可以通过 REPL 创建任务。Priority: must\n\n## 成功标准\n\n- [AC-001] 输入 new_task 后返回 task_id、phase、intake_ref、change_id。Refs: REQ-001\n"
}

fn canonical_design_markdown() -> &'static str {
    "# Design\n\n## 设计决策\n\n- [DD-001] REPL 只作为客户端，daemon 是 runtime truth。Refs: REQ-001\n\n## 公共组件\n\n- [CMP-001] repl_wire JSON envelope schema\n\n## 风险\n\n- [RISK-001] REPL 断连后 daemon 状态不一致。Severity: high; Refs: DD-001\n"
}

fn no_id_design_markdown() -> &'static str {
    "# Design\n\n\
## 设计决策\n\n\
| 决策点 | 选择 | 理由 |\n\
|--------|------|------|\n\
| runtime truth | daemon | REQ-001 要求运行时状态统一 |\n\n\
## 公共组件\n\n\
### Runtime Session Store\n\n\
- **职责**: 保存任务运行态\n"
}

fn revised_design_markdown() -> &'static str {
    "# Design\n\n## 设计决策\n\n- [DD-001] 修订后 REPL 只作为客户端，daemon 是 runtime truth。Refs: REQ-001\n\n## 公共组件\n\n- [CMP-001] repl_wire JSON envelope schema\n\n## 风险\n\n- [RISK-001] REPL 断连后 daemon 状态不一致。Severity: high; Refs: DD-001\n"
}

fn canonical_plan_markdown() -> &'static str {
    "# Plan\n\n## 工作包\n\n| ID | Description | Execution Mode | Human Reason | Traceability | Acceptance |\n|----|-------------|----------------|--------------|--------------|------------|\n| WT-001 | 实现 REPL wire schema | agent_only | | REQ-001, DD-001, TASK-001 | AC-001 |\n| WT-002 | 实现 daemon handshake | agent_only | | REQ-001, DD-001, TASK-002 | AC-001 |\n\n## 依赖关系\n\n| From | To | Type |\n|------|----|------|\n| WT-001 | WT-002 | blocks |\n"
}

fn package_json_plan_markdown() -> &'static str {
    "# Plan\n\n## 工作包\n\n| ID | Description | Execution Mode | Human Reason | Traceability | Acceptance |\n|----|-------------|----------------|--------------|--------------|------------|\n| WT-001 | 实现 climbStairs 源码模块 | agent_only | | REQ-001, DD-001, TASK-001 | AC-001 |\n| WT-002 | 实现 tests/climbStairs.test.js 测试套件 | agent_only | | REQ-001, DD-001, TASK-002 | AC-002 |\n| WT-003 | 在 package.json 注册 test 脚本 node --test，缺失则创建 package.json | agent_only | | REQ-001, DD-001, TASK-003 | AC-007 |\n\n## 依赖关系\n\n| From | To | Type |\n|------|----|------|\n| WT-002 | WT-003 | blocks |\n"
}

fn dec_traceability_plan_markdown() -> &'static str {
    "# Plan\n\n## 工作包\n\n| ID | Description | Execution Mode | Human Reason | Traceability | Acceptance |\n|----|-------------|----------------|--------------|--------------|------------|\n| WT-001 | 实现 runtime session store | agent_only | | REQ-001, DEC-001, TASK-001 | AC-001 |\n\n## 依赖关系\n\n| From | To | Type |\n|------|----|------|\n"
}

fn prepare_change_dir(workspace_root: &Path, change_id: &str) -> std::path::PathBuf {
    let change_dir = workspace_root.join("openspec/changes").join(change_id);
    copy_dir(
        Path::new("tests/fixtures/openspec/changes/sample-change"),
        &change_dir,
    );
    fs::write(
        change_dir.join("specs/main/spec.md"),
        "# Main Spec\n\n### ADDED Requirements\n\n",
    )
    .expect("empty initial spec");
    change_dir
}

fn planning_input(workspace_root: &Path, change_id: &str) -> PlanningStartChainInput {
    let change_dir = workspace_root.join("openspec/changes").join(change_id);
    let initial_manifest =
        cadence_aria::cross_cutting::openspec_constraints::build_openspec_source_manifest(
            &change_dir,
        )
        .expect("initial manifest");
    PlanningStartChainInput {
        session_id: "sess_planning".to_string(),
        task_id: "task_0001".to_string(),
        change_id: change_id.to_string(),
        workspace_root: workspace_root.to_path_buf(),
        worktree_path: None,
        intake_brief: json!({
            "artifact_kind": "intake_brief",
            "request_summary": "实现 Aria 规划链起始节点",
            "raw_user_request": "继续 MVP 内容开发",
            "repo_context": {"branch": "feature/aria-phase1-p2"},
            "initial_constraints": ["使用 Docker Rust 环境"],
            "requested_goal": "N04-N12 fake provider chain"
        }),
        initial_constraint_bundle: OpenSpecConstraintBundle {
            constraint_bundle_id: "constraint_bundle_initial".to_string(),
            bundle_version: "openspec.constraint_bundle.v1".to_string(),
            bundle_status: BundleStatus::Ready,
            change_id: change_id.to_string(),
            proposal_constraints: ProposalConstraints {
                business_intent: vec![
                    "Users need to create runtime tasks from the REPL.".to_string(),
                ],
                scope: vec![
                    "Compile task creation rules into the Phase 1 runtime contract.".to_string(),
                ],
                non_goals: vec![],
                impacted_areas: vec![],
            },
            requirement_constraints: RequirementConstraints {
                requirement_ids: vec![],
                scenario_ids: vec![],
                success_criteria_ids: vec![],
            },
            design_constraints: DesignConstraints {
                design_decision_ids: vec![],
                component_ids: vec![],
                risk_ids: vec![],
            },
            task_constraints: TaskConstraints {
                task_ids: vec![],
                task_sequence: vec![],
                related_requirement_ids_by_task: Default::default(),
                related_design_decision_ids_by_task: Default::default(),
                acceptance_target_ids_by_task: Default::default(),
            },
            traceability_requirements: TraceabilityRequirements {
                required_requirement_ids: vec![],
                required_design_decision_ids: vec![],
                required_task_ids: vec![],
                required_acceptance_target_ids: vec![],
            },
            coverage_model: CoverageModel {
                required_ids: vec![],
                covered_ids: vec![],
                uncovered_ids: vec![],
            },
            source_manifest: initial_manifest,
            compiled_from_projection_refs: vec![],
            compiled_at: "2026-04-27T00:00:00Z".to_string(),
            compiled_by_node: "N03".to_string(),
        },
    }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create target dir");
    for entry in fs::read_dir(from).expect("read source dir") {
        let entry = entry.expect("dir entry");
        let source_path = entry.path();
        let target_path = to.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir(&source_path, &target_path);
        } else {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent).expect("create file parent");
            }
            fs::copy(&source_path, &target_path).expect("copy file");
        }
    }
}
