use cadence_aria::cross_cutting::integration_queue::IntegrationQueue;
use cadence_aria::cross_cutting::openspec_constraints::build_openspec_source_manifest;
use cadence_aria::cross_cutting::provider_adapter::{
    ProviderAdapter, ProviderAdapterError, STRUCTURED_OUTPUT_END, STRUCTURED_OUTPUT_START,
    parse_last_structured_output,
};
use cadence_aria::cross_cutting::worktree::WorktreeLeaseManager;
use cadence_aria::protocol::constraints::{
    BundleStatus, CoverageModel, DesignConstraints, OpenSpecConstraintBundle, ProposalConstraints,
    RequirementConstraints, TaskConstraints, TraceabilityRequirements,
};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
use cadence_aria::protocol::projections::{PlanProjection, ProjectionPayload};
use cadence_aria::runtime_units::coding::{ExecutionWorktaskInput, run_worktask_execution_chain};
use cadence_aria::runtime_units::execution_setup::{ExecutionSetupInput, run_execution_setup};
use cadence_aria::runtime_units::final_review::{FinalClosureInput, run_final_closure_chain};
use cadence_aria::runtime_units::integration_execute::{
    IntegrationExecuteInput, run_integration_execute,
};
use cadence_aria::runtime_units::integration_prepare::{
    IntegrationPrepareInput, run_integration_prepare,
};
use cadence_aria::runtime_units::integration_verify::{
    IntegrationVerifyInput, run_integration_verify,
};
use cadence_aria::runtime_units::plan_dispatch::run_planning_full_chain;
use cadence_aria::runtime_units::spec_gate_review::PlanningStartChainInput;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

#[test]
fn phase1_smoke_runs_from_planning_dispatch_to_final_summary() {
    let workspace = tempfile::tempdir().expect("workspace");
    let change_id = "sample-change";
    prepare_change_dir(workspace.path(), change_id);
    let provider = SmokeProvider::default();

    let planning = run_planning_full_chain(planning_input(workspace.path(), change_id), &provider)
        .expect("planning chain");
    let ProjectionPayload::PlanProjection(plan_projection) =
        planning.plan_projection.payload.clone()
    else {
        panic!("expected plan projection");
    };
    let routing = planning.dispatch_package["_aria"]["worktask_routing"][0].clone();
    let worktask_id = routing["worktask_id"].as_str().expect("worktask id");
    let source_work_package_id = routing["source_work_package_id"]
        .as_str()
        .expect("source work package id");
    let traceability_refs = string_array(&routing["traceability_refs"]);
    let repo = prepare_git_worktask_repo(workspace.path(), worktask_id);

    let mut lease_manager = WorktreeLeaseManager::new("session_001", "task_001", &repo, "main");
    let execution_setup = run_execution_setup(
        ExecutionSetupInput {
            session_id: "session_001".to_string(),
            task_id: "task_001".to_string(),
            dispatch_package_ref: planning.dispatch_ref.artifact_ref_id.clone(),
            dispatch_package: planning.dispatch_package.clone(),
            plan_projection: serde_json::to_value(&plan_projection).expect("plan projection json"),
            worktree_path: repo.clone(),
            base_ref: "main".to_string(),
        },
        &mut lease_manager,
    )
    .expect("execution setup");
    assert_eq!(execution_setup.route_contexts.len(), 1);

    let route_context = &execution_setup.route_contexts[0];
    assert_eq!(route_context.source_work_package_id, source_work_package_id);
    let execution = run_worktask_execution_chain(
        execution_input(
            &repo,
            &planning.dispatch_package,
            &plan_projection,
            route_context,
        ),
        &provider,
    )
    .expect("worktask execution");
    assert_eq!(execution.next_node, "M20");

    let mut queue = IntegrationQueue::default();
    let integration_prepare = run_integration_prepare(
        IntegrationPrepareInput {
            session_id: "session_001".to_string(),
            task_id: "task_001".to_string(),
            worktask_id: worktask_id.to_string(),
            worktree_path: repo.clone(),
            integration_worktree_path: workspace.path().join("integration-worktree"),
            integration_branch: "aria/integration/task_001".to_string(),
            base_ref: "main".to_string(),
            allowed_write_scope: route_context.allowed_write_scope.clone(),
        },
        &mut queue,
    )
    .expect("integration prepare");
    let integration_execute = run_integration_execute(IntegrationExecuteInput {
        worktask_id: worktask_id.to_string(),
        integration_worktree_path: integration_prepare.integration_worktree_path.clone(),
        candidate_commit_sha: integration_prepare.candidate_commit_sha.clone(),
        pre_merge_sha: integration_prepare.pre_merge_sha.clone(),
    })
    .expect("integration execute");
    let integration_verify = run_integration_verify(IntegrationVerifyInput {
        worktask_id: worktask_id.to_string(),
        integration_worktree_path: integration_prepare.integration_worktree_path.clone(),
        pre_merge_sha: integration_prepare.pre_merge_sha.clone(),
        verify_passed: true,
    })
    .expect("integration verify");

    let final_closure = run_final_closure_chain(
        FinalClosureInput {
            session_id: "session_001".to_string(),
            task_id: "task_001".to_string(),
            projection_refs: vec![
                planning.design_projection.projection_id.clone(),
                planning.plan_projection.projection_id.clone(),
            ],
            constraint_bundle_ref: planning
                .openspec_bundle_after_tasks
                .constraint_bundle_id
                .clone(),
            risk_registry_ref: "risk_registry_001".to_string(),
            canonical_artifact_refs: execution
                .artifacts
                .iter()
                .filter_map(|artifact| {
                    artifact
                        .get("artifact_ref")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .chain([planning.dispatch_ref.artifact_ref_id.clone()])
                .collect(),
            traceability_refs: traceability_refs.clone(),
            context_files: vec![
                "tests/fixtures/artifacts/spec.md".to_string(),
                "tests/fixtures/projections/plan_projection.json".to_string(),
                "tests/fixtures/openspec/constraint_bundle.json".to_string(),
            ],
        },
        &provider,
    )
    .expect("final closure");

    let mut nodes = Vec::new();
    nodes.extend(
        planning
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str()),
    );
    nodes.extend(
        execution_setup
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str()),
    );
    nodes.extend(
        execution
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str()),
    );
    nodes.extend(
        integration_prepare
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str()),
    );
    nodes.push(integration_execute.protocol_step.node_id.as_str());
    nodes.push(integration_verify.protocol_step.node_id.as_str());
    nodes.extend(
        final_closure
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str()),
    );

    assert_eq!(
        nodes,
        vec![
            "N04", "N05", "N06", "N07", "N08", "N10", "N11", "N12", "N13", "N14", "N15", "N16",
            "N17", "N18", "N20", "N21", "N22", "N23", "N24", "N25", "N27", "N28",
        ]
    );
    assert_eq!(integration_execute.next_decision, "verify");
    assert_eq!(integration_verify.next_decision, "N25");
    assert_eq!(
        final_closure.final_review["_aria"]["coverage_summary"],
        json!({
            "closed": traceability_refs,
            "uncovered": [],
            "exempted": []
        })
    );
    assert_eq!(
        final_closure.final_summary["overall_status"],
        "closed_successfully"
    );
}

#[derive(Debug, Default)]
struct SmokeProvider {
    output_schemas: Mutex<Vec<String>>,
}

impl ProviderAdapter for SmokeProvider {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.output_schemas
            .lock()
            .expect("schemas")
            .push(input.output_schema.clone());
        let payload = match input.output_schema.as_str() {
            "schema://aria/artifacts/clarification_record/v1" => json!({
                "artifact_kind": "clarification_record",
                "goal_summary": "串起 Aria 一期闭环 smoke",
                "constraints": ["使用 Docker Rust 环境"],
                "assumptions": ["P1-P3 已完成"],
                "open_questions": [],
                "suggested_scope": "phase1 smoke"
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
            "schema://aria/artifacts/design_review/v1" => json!({
                "artifact_kind": "design_review",
                "review_decision": "pass",
                "findings": []
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
            "schema://aria/artifacts/coding_report/v1" => json!({
                "artifact_kind": "coding_report",
                "artifact_ref": "coding_report_work_wt_001_0001",
                "worktask_id": "work_wt_001",
                "files_modified": ["src/lib.rs"],
                "commands_run": ["cargo test --test phase1_end_to_end_smoke"],
                "candidate_traceability_refs": [],
                "status": "completed"
            }),
            "schema://aria/artifacts/testing_report/v1" => json!({
                "artifact_kind": "testing_report",
                "artifact_ref": "testing_report_work_wt_001_0001",
                "worktask_id": "work_wt_001",
                "commands_run": ["cargo test --test phase1_end_to_end_smoke"],
                "tests_passed": true,
                "failures": [],
                "candidate_traceability_refs": []
            }),
            "schema://aria/artifacts/code_review_report/v1" => json!({
                "artifact_kind": "code_review_report",
                "artifact_ref": "code_review_report_work_wt_001_0001",
                "worktask_id": "work_wt_001",
                "findings": [],
                "blocking": false,
                "candidate_traceability_refs": []
            }),
            "schema://aria/artifacts/final_review/v1" => json!({
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
            "schema://aria/artifacts/final_summary/v1" => json!({
                "artifact_kind": "final_summary",
                "overall_status": "closed_successfully",
                "next_steps": [],
                "remaining_risks": [],
                "closed_items": ["req-001", "dd-001", "task-001"]
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
            files_modified: payload
                .get("files_modified")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            duration_ms: 1,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

fn execution_input(
    worktree_path: &Path,
    dispatch_package: &Value,
    plan_projection: &PlanProjection,
    route_context: &cadence_aria::runtime_units::execution_setup::ExecutionRouteContext,
) -> ExecutionWorktaskInput {
    ExecutionWorktaskInput {
        session_id: "session_001".to_string(),
        task_id: "task_001".to_string(),
        worktask_id: route_context.worktask_id.clone(),
        source_work_package_id: route_context.source_work_package_id.clone(),
        worktree_path: worktree_path.to_path_buf(),
        allowed_write_scope: route_context.allowed_write_scope.clone(),
        dispatch_package: dispatch_package.clone(),
        plan_projection: plan_projection.clone(),
        projection_refs: vec![
            "proj_spec_projection_001".to_string(),
            "proj_design_projection_001".to_string(),
            "proj_plan_projection_001".to_string(),
        ],
        constraint_bundle_ref: "constraint_bundle_task_001".to_string(),
        risk_registry_ref: "risk_registry_001".to_string(),
        context_files: vec![
            "tests/fixtures/artifacts/spec.md".to_string(),
            "tests/fixtures/projections/plan_projection.json".to_string(),
            "tests/fixtures/openspec/constraint_bundle.json".to_string(),
        ],
    }
}

fn planning_input(workspace_root: &Path, change_id: &str) -> PlanningStartChainInput {
    let change_dir = workspace_root.join("openspec/changes").join(change_id);
    let initial_manifest = build_openspec_source_manifest(&change_dir).expect("initial manifest");
    PlanningStartChainInput {
        session_id: "session_001".to_string(),
        task_id: "task_001".to_string(),
        change_id: change_id.to_string(),
        workspace_root: workspace_root.to_path_buf(),
        worktree_path: None,
        intake_brief: json!({
            "artifact_kind": "intake_brief",
            "request_summary": "继续 MVP 内容开发",
            "raw_user_request": "继续",
            "repo_context": {"branch": "feature/aria-phase1-p2"},
            "initial_constraints": ["使用 Docker Rust 环境"],
            "requested_goal": "phase1 smoke"
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

fn prepare_change_dir(workspace_root: &Path, change_id: &str) -> PathBuf {
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

fn prepare_git_worktask_repo(workspace: &Path, worktask_id: &str) -> PathBuf {
    let repo = workspace.join("repo");
    fs::create_dir_all(repo.join("src")).expect("repo dirs");
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.email", "aria@example.invalid"]);
    git(&repo, &["config", "user.name", "Aria Test"]);
    fs::write(repo.join("README.md"), "base\n").expect("readme");
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "base"]);
    git(&repo, &["checkout", "-b", &format!("aria/{worktask_id}")]);
    fs::write(repo.join("src/lib.rs"), "pub fn value() -> u32 { 1 }\n").expect("feature");
    repo
}

fn canonical_spec_markdown() -> &'static str {
    "# Spec\n\n## 功能需求\n\n- [REQ-001] 用户可以通过 REPL 创建任务。Priority: must\n\n## 成功标准\n\n- [AC-001] 输入 new_task 后返回 task_id、phase、intake_ref、change_id。Refs: REQ-001\n"
}

fn canonical_design_markdown() -> &'static str {
    "# Design\n\n## 设计决策\n\n- [DD-001] REPL 只作为客户端，daemon 是 runtime truth。Refs: REQ-001\n\n## 公共组件\n\n- [CMP-001] repl_wire JSON envelope schema\n\n## 风险\n\n- [RISK-001] REPL 断连后 daemon 状态不一致。Severity: high; Refs: DD-001\n"
}

fn canonical_plan_markdown() -> &'static str {
    "# Plan\n\n## 工作包\n\n| ID | Description | Execution Mode | Human Reason | Traceability | Acceptance |\n|----|-------------|----------------|--------------|--------------|------------|\n| WT-001 | 实现一期 smoke 闭环 | agent_only | | REQ-001, DD-001, TASK-001 | AC-001 |\n\n## 依赖关系\n\n| From | To | Type |\n|------|----|------|\n"
}

fn string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
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

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout={}\nstderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
