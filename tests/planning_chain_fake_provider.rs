use cadence_aria::cross_cutting::provider_adapter::{
    parse_last_structured_output, ProviderAdapter, ProviderAdapterError, STRUCTURED_OUTPUT_END,
    STRUCTURED_OUTPUT_START,
};
use cadence_aria::protocol::constraints::{
    BundleStatus, CoverageModel, DesignConstraints, OpenSpecConstraintBundle, ProposalConstraints,
    RequirementConstraints, TaskConstraints, TraceabilityRequirements,
};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
use cadence_aria::protocol::projections::ProjectionPayload;
use cadence_aria::runtime_units::plan_dispatch::run_planning_full_chain;
use cadence_aria::runtime_units::spec_gate_review::{
    run_planning_start_chain, PlanningStartChainInput,
};
use serde_json::json;
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
    assert!(routing
        .iter()
        .all(|item| item.get("source_work_package_id").is_some()));

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
        vec!["N04", "N05", "N06", "N07", "N08", "N09", "N08", "N10", "N11", "N12"]
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
}

#[derive(Debug)]
struct ScriptedPlanningProvider {
    output_schemas: Mutex<Vec<String>>,
    review_decisions: Mutex<VecDeque<String>>,
}

impl Default for ScriptedPlanningProvider {
    fn default() -> Self {
        Self::with_review_decisions(["pass"])
    }
}

impl ScriptedPlanningProvider {
    fn with_review_decisions<const N: usize>(decisions: [&str; N]) -> Self {
        Self {
            output_schemas: Mutex::new(Vec::new()),
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
}

impl ProviderAdapter for ScriptedPlanningProvider {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.output_schemas
            .lock()
            .expect("schemas")
            .push(input.output_schema.clone());
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

fn revised_design_markdown() -> &'static str {
    "# Design\n\n## 设计决策\n\n- [DD-001] 修订后 REPL 只作为客户端，daemon 是 runtime truth。Refs: REQ-001\n\n## 公共组件\n\n- [CMP-001] repl_wire JSON envelope schema\n\n## 风险\n\n- [RISK-001] REPL 断连后 daemon 状态不一致。Severity: high; Refs: DD-001\n"
}

fn canonical_plan_markdown() -> &'static str {
    "# Plan\n\n## 工作包\n\n| ID | Description | Execution Mode | Human Reason | Traceability | Acceptance |\n|----|-------------|----------------|--------------|--------------|------------|\n| WT-001 | 实现 REPL wire schema | agent_only | | REQ-001, DD-001, TASK-001 | AC-001 |\n| WT-002 | 实现 daemon handshake | agent_only | | REQ-001, DD-001, TASK-002 | AC-001 |\n\n## 依赖关系\n\n| From | To | Type |\n|------|----|------|\n| WT-001 | WT-002 | blocks |\n"
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
                    "Users need to create runtime tasks from the REPL.".to_string()
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
