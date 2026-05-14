use cadence_aria::cross_cutting::provider_adapter::{
    ProviderAdapter, ProviderAdapterError, STRUCTURED_OUTPUT_END, STRUCTURED_OUTPUT_START,
    parse_last_structured_output,
};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
use cadence_aria::runtime_units::final_review::{
    FinalClosureError, FinalClosureInput, run_final_closure_chain,
};
use serde_json::json;
use std::sync::Mutex;

#[test]
fn final_review_pass_routes_to_summary_and_session_closeout() {
    let provider = ScriptedFinalProvider::pass();
    let result = run_final_closure_chain(final_input(), &provider).expect("final closure");

    assert_eq!(
        result
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["N25", "N27", "N28"]
    );
    assert_eq!(result.final_review["overall_decision"], "pass");
    assert_eq!(
        result.final_review["_aria"]["coverage_summary"],
        json!({
            "closed": ["req-001", "dd-001", "task-001"],
            "uncovered": [],
            "exempted": []
        })
    );
    assert_eq!(
        result.final_summary["overall_status"],
        "closed_successfully"
    );
    assert_eq!(
        result
            .protocol_steps
            .last()
            .expect("N28")
            .node_specific_fields["final_checkpoint_ref"],
        "checkpoint_task_001_final"
    );
    assert_eq!(
        provider.seen_output_schemas(),
        vec![
            "schema://aria/artifacts/final_review/v1".to_string(),
            "schema://aria/artifacts/final_summary/v1".to_string(),
        ]
    );
}

#[test]
fn final_summary_cannot_add_coverage_not_present_in_final_review() {
    let provider = ScriptedFinalProvider::summary_with_extra_closed_item();

    let error = run_final_closure_chain(final_input(), &provider)
        .expect_err("final summary must not introduce new coverage conclusions");

    assert!(matches!(
        error,
        FinalClosureError::FinalSummaryCoverageUnknown(ref item) if item == "req-999"
    ));
}

fn final_input() -> FinalClosureInput {
    FinalClosureInput {
        session_id: "session_001".to_string(),
        task_id: "task_001".to_string(),
        worktree_path: "tests/fixtures/repos/sample-worktree".to_string(),
        projection_refs: vec![
            "proj_spec_projection_001".to_string(),
            "proj_design_projection_001".to_string(),
            "proj_plan_projection_001".to_string(),
        ],
        constraint_bundle_ref: "constraint_bundle_task_001".to_string(),
        risk_registry_ref: "risk_registry_001".to_string(),
        canonical_artifact_refs: vec![
            "integration_report_worktask_001_0001".to_string(),
            "dispatch_pkg_001".to_string(),
        ],
        traceability_refs: vec![
            "req-001".to_string(),
            "dd-001".to_string(),
            "task-001".to_string(),
        ],
        context_files: vec![
            "tests/fixtures/artifacts/spec.md".to_string(),
            "tests/fixtures/projections/plan_projection.json".to_string(),
            "tests/fixtures/openspec/constraint_bundle.json".to_string(),
        ],
    }
}

#[derive(Debug)]
struct ScriptedFinalProvider {
    output_schemas: Mutex<Vec<String>>,
    final_review: serde_json::Value,
    final_summary: serde_json::Value,
}

impl ScriptedFinalProvider {
    fn pass() -> Self {
        Self {
            output_schemas: Mutex::new(Vec::new()),
            final_review: json!({
                "artifact_kind": "final_review",
                "overall_decision": "pass",
                "coverage_summary": {
                    "closed": ["req-001", "dd-001", "task-001"],
                    "uncovered": [],
                    "exempted": []
                },
                "uncovered_items": [],
                "followup_required": false
            }),
            final_summary: json!({
                "artifact_kind": "final_summary",
                "overall_status": "closed_successfully",
                "next_steps": [],
                "remaining_risks": [],
                "closed_items": ["req-001", "dd-001", "task-001"]
            }),
        }
    }

    fn summary_with_extra_closed_item() -> Self {
        let mut provider = Self::pass();
        provider.final_summary = json!({
            "artifact_kind": "final_summary",
            "overall_status": "closed_successfully",
            "next_steps": [],
            "remaining_risks": [],
            "closed_items": ["req-001", "dd-001", "task-001", "req-999"]
        });
        provider
    }

    fn seen_output_schemas(&self) -> Vec<String> {
        self.output_schemas.lock().expect("schemas").clone()
    }
}

impl ProviderAdapter for ScriptedFinalProvider {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.output_schemas
            .lock()
            .expect("schemas")
            .push(input.output_schema.clone());
        let payload = match input.output_schema.as_str() {
            "schema://aria/artifacts/final_review/v1" => self.final_review.clone(),
            "schema://aria/artifacts/final_summary/v1" => self.final_summary.clone(),
            other => panic!("unexpected schema {other}"),
        };
        let stdout = format!(
            "provider log\n{STRUCTURED_OUTPUT_START}\n{}\n{STRUCTURED_OUTPUT_END}\n",
            serde_json::to_string(&payload).expect("payload json")
        );
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: stdout.clone(),
            stderr: String::new(),
            structured_output: parse_last_structured_output(&stdout)?,
            files_modified: vec![],
            duration_ms: 1,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}
