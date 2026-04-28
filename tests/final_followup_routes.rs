use cadence_aria::cross_cutting::openspec_constraints::{
    build_openspec_source_manifest, compile_constraint_bundle,
};
use cadence_aria::cross_cutting::provider_adapter::{
    parse_last_structured_output, ProviderAdapter, ProviderAdapterError, STRUCTURED_OUTPUT_END,
    STRUCTURED_OUTPUT_START,
};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
use cadence_aria::runtime_units::patch_followup_dispatch::{
    run_final_followup_route, ApprovalDecision, FinalFollowupInput,
};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[test]
fn approved_followup_gate_runs_n26_updates_tasks_recompiles_bundle_and_returns_to_n13() {
    let workspace = tempfile::tempdir().expect("workspace");
    let change_dir = prepare_change_dir(workspace.path());
    let provider = ScriptedFollowupProvider::followup();

    let result = run_final_followup_route(
        followup_input(&change_dir),
        &provider,
        ApprovalDecision::Approved {
            approved_by: "user".to_string(),
        },
    )
    .expect("approved followup");

    assert_eq!(
        result
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["N25", "X01", "N26", "N13"]
    );
    assert!(fs::read_to_string(change_dir.join("tasks.md"))
        .expect("tasks")
        .contains("TASK-002 Follow up bounded patch"));
    assert_eq!(
        result
            .recompiled_bundle
            .expect("bundle")
            .task_constraints
            .task_ids,
        vec!["TASK-001".to_string(), "TASK-002".to_string()]
    );
    assert_eq!(
        result.new_dispatch_package["_aria"]["worktask_routing"][0]["source_task_id"],
        "TASK-002"
    );
    assert_eq!(result.patch_round_counter, 1);
    assert_eq!(
        provider.seen_output_schemas(),
        vec![
            "schema://aria/artifacts/final_review/v1".to_string(),
            "schema://aria/artifacts/dispatch_package/v1".to_string(),
        ]
    );
}

#[test]
fn rejected_followup_does_not_run_n26_and_closes_with_rejected_followup_status() {
    let workspace = tempfile::tempdir().expect("workspace");
    let change_dir = prepare_change_dir(workspace.path());
    let provider = ScriptedFollowupProvider::followup();

    let result = run_final_followup_route(
        followup_input(&change_dir),
        &provider,
        ApprovalDecision::Rejected {
            rejected_by: "user".to_string(),
            reason: "本轮不继续扩展范围".to_string(),
        },
    )
    .expect("rejected followup");

    assert_eq!(
        result
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["N25", "X01", "N27", "N28"]
    );
    assert_eq!(
        result.final_summary["overall_status"],
        "closed_with_rejected_followup"
    );
    assert_eq!(
        result.final_summary["manual_exemptions"][0]["reason"],
        "本轮不继续扩展范围"
    );
    assert!(result.new_dispatch_package.is_null());
}

fn followup_input(change_dir: &Path) -> FinalFollowupInput {
    let manifest = build_openspec_source_manifest(change_dir).expect("manifest");
    let bundle = compile_constraint_bundle(
        &"sample-change".to_string(),
        &manifest,
        vec!["proj_plan_projection_001".to_string()],
        "N25".to_string(),
    )
    .expect("initial bundle");
    FinalFollowupInput {
        session_id: "session_001".to_string(),
        task_id: "task_001".to_string(),
        change_id: "sample-change".to_string(),
        change_dir: change_dir.to_path_buf(),
        projection_refs: vec![
            "proj_spec_projection_001".to_string(),
            "proj_design_projection_001".to_string(),
            "proj_plan_projection_001".to_string(),
        ],
        constraint_bundle_ref: bundle.constraint_bundle_id.clone(),
        current_bundle: bundle,
        risk_registry_ref: "risk_registry_001".to_string(),
        canonical_artifact_refs: vec!["final_review_input".to_string()],
        traceability_refs: vec![
            "req-001".to_string(),
            "dd-001".to_string(),
            "task-001".to_string(),
        ],
        context_files: vec![
            change_dir.join("tasks.md").to_string_lossy().to_string(),
            change_dir
                .join("specs/main/spec.md")
                .to_string_lossy()
                .to_string(),
            change_dir.join("design.md").to_string_lossy().to_string(),
        ],
        patch_round_counter: 0,
    }
}

fn prepare_change_dir(workspace: &Path) -> PathBuf {
    let change_dir = workspace.join("openspec/changes/sample-change");
    fs::create_dir_all(change_dir.join("specs/main")).expect("dirs");
    fs::write(
        change_dir.join("proposal.md"),
        "# Proposal\n\n## Why\n\n- Need bounded closure.\n\n## What Changes\n\n- Add followup tasks.\n",
    )
    .expect("proposal");
    fs::write(
        change_dir.join("specs/main/spec.md"),
        "# Spec\n\n#### Requirement: REQ-001\n\n##### Scenario: SCN-001\n\n- AC-001 closes the request.\n",
    )
    .expect("spec");
    fs::write(
        change_dir.join("design.md"),
        "# Design\n\n## Design Decisions\n\n- [DD-001] Use gated followup dispatch.\n\n## Components\n\n- [CMP-001] Final closure runtime unit.\n",
    )
    .expect("design");
    fs::write(
        change_dir.join("tasks.md"),
        "# Tasks\n\n- [ ] TASK-001 Existing closure task. Reqs: REQ-001; Designs: DD-001; Acceptance: AC-001\n",
    )
    .expect("tasks");
    change_dir
}

#[derive(Debug)]
struct ScriptedFollowupProvider {
    output_schemas: Mutex<Vec<String>>,
}

impl ScriptedFollowupProvider {
    fn followup() -> Self {
        Self {
            output_schemas: Mutex::new(Vec::new()),
        }
    }

    fn seen_output_schemas(&self) -> Vec<String> {
        self.output_schemas.lock().expect("schemas").clone()
    }
}

impl ProviderAdapter for ScriptedFollowupProvider {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.output_schemas
            .lock()
            .expect("schemas")
            .push(input.output_schema.clone());
        let payload = match input.output_schema.as_str() {
            "schema://aria/artifacts/final_review/v1" => json!({
                "artifact_kind": "final_review",
                "overall_decision": "followup",
                "coverage_summary": {
                    "closed": ["req-001", "dd-001", "task-001"],
                    "uncovered": ["task-002"],
                    "exempted": []
                },
                "uncovered_items": ["task-002"],
                "followup_required": true,
                "followup_scope_clear": true
            }),
            "schema://aria/artifacts/dispatch_package/v1" => json!({
                "artifact_kind": "dispatch_package",
                "patch_task_delta": [
                    {
                        "delta_type": "add_task",
                        "task_id": "TASK-002",
                        "description": "Follow up bounded patch",
                        "acceptance_targets": ["AC-001"],
                        "execution_mode": "bounded_patch",
                        "traceability_refs": ["req-001", "dd-001"],
                        "related_requirement_ids": ["REQ-001"],
                        "related_design_decision_ids": ["DD-001"]
                    }
                ],
                "worktask_routing": []
            }),
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
