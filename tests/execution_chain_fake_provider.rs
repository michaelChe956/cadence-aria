use cadence_aria::cross_cutting::integration_queue::IntegrationQueue;
use cadence_aria::cross_cutting::provider_adapter::{
    parse_last_structured_output, ProviderAdapter, ProviderAdapterError, STRUCTURED_OUTPUT_END,
    STRUCTURED_OUTPUT_START,
};
use cadence_aria::cross_cutting::worktree::{WorktreeLeaseManager, WorktreeLeaseStatus};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
use cadence_aria::protocol::loop_counters::{LoopCounterName, LoopCounterRegistry};
use cadence_aria::protocol::projections::{ExecutionMode, PlanProjection, WorkPackageProjection};
use cadence_aria::runtime_units::coding::{run_worktask_execution_chain, ExecutionWorktaskInput};
use cadence_aria::runtime_units::execution_setup::{
    run_execution_setup, ExecutionSetupInput, ExecutionSetupUnit,
};
use cadence_aria::runtime_units::integration_execute::{
    run_integration_execute, IntegrationExecuteInput,
};
use cadence_aria::runtime_units::integration_prepare::{
    run_integration_prepare, IntegrationPrepareInput, IntegrationPrepareUnit,
};
use cadence_aria::runtime_units::integration_verify::{
    run_integration_verify, IntegrationVerifyInput,
};
use cadence_aria::runtime_units::RuntimeUnit;
use serde_json::json;
use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

#[test]
fn execution_setup_registers_worktasks_prepares_worktree_and_resolves_routes() {
    let workspace = tempfile::tempdir().expect("workspace");
    let mut manager =
        WorktreeLeaseManager::new("session_001", "task_001", workspace.path(), "main");
    let unit = ExecutionSetupUnit;
    assert_eq!(unit.covered_protocol_nodes(), vec!["N13", "N14", "N15"]);

    let result = run_execution_setup(execution_input(workspace.path()), &mut manager)
        .expect("execution setup");

    assert_eq!(
        result
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["N13", "N14", "N15"]
    );
    assert_eq!(
        result.protocol_steps[0].node_specific_fields["worktask_id"],
        "worktask_001"
    );
    assert_eq!(
        result.protocol_steps[0].node_specific_fields["routing_ref"],
        "dispatch_pkg_001#worktask_001"
    );
    assert_eq!(
        result.protocol_steps[0].node_specific_fields["state"],
        "registered"
    );
    assert_eq!(
        result.protocol_steps[1].node_specific_fields["base_ref"],
        "main"
    );
    assert_eq!(
        result.protocol_steps[1].node_specific_fields["branch_name"],
        "aria/worktask_001"
    );
    assert_eq!(
        result.protocol_steps[2].node_specific_fields["dispatch_package_ref"],
        "dispatch_pkg_001"
    );
    assert_eq!(result.route_contexts[0].source_work_package_id, "WP-001");
    assert_eq!(
        result.route_contexts[0].traceability_refs,
        vec!["REQ-001".to_string()]
    );
    assert_eq!(
        result.route_contexts[0].acceptance_targets,
        vec!["cargo test --test execution_chain_fake_provider".to_string()]
    );

    let lease = manager
        .lease(&result.route_contexts[0].lease_id)
        .expect("lease exists");
    assert_eq!(lease.status, WorktreeLeaseStatus::Acquired);
    assert_eq!(lease.allowed_write_scope, vec!["src/feature/".to_string()]);
    assert!(manager.events().iter().any(|event| {
        event.event_type == "worktree.lease_acquired"
            && event.payload["lease_id"] == lease.lease_id
            && event.payload["worktree_path"].as_str()
                == Some(workspace.path().to_string_lossy().as_ref())
            && event.payload["worktask_id"] == "worktask_001"
    }));
}

#[test]
fn execution_setup_rejects_worktask_routing_without_plan_work_package() {
    let workspace = tempfile::tempdir().expect("workspace");
    let mut input = execution_input(workspace.path());
    input.dispatch_package["_aria"]["worktask_routing"][0]["source_work_package_id"] =
        json!("WP-MISSING");
    let mut manager =
        WorktreeLeaseManager::new("session_001", "task_001", workspace.path(), "main");

    let error = run_execution_setup(input, &mut manager).expect_err("missing source package");

    assert_eq!(error.code, "worktask_source_work_package_missing");
    assert!(error.message.contains("WP-MISSING"));
}

#[test]
fn fake_provider_runs_n16_to_n18_happy_path_with_normalized_traceability() {
    let workspace = tempfile::tempdir().expect("workspace");
    let provider = ScriptedExecutionProvider::happy();

    let result =
        run_worktask_execution_chain(execution_worktask_input(workspace.path()), &provider)
            .expect("execution chain");

    assert_eq!(
        result
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["N16", "N17", "N18"]
    );
    assert_eq!(
        provider.seen_output_schemas(),
        vec![
            "schema://aria/artifacts/coding_report/v1".to_string(),
            "schema://aria/artifacts/testing_report/v1".to_string(),
            "schema://aria/artifacts/code_review_report/v1".to_string(),
        ]
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
                "phase1_profile_validate",
                "openspec_coverage",
                "checkpoint",
            ],
            "{} must use the unified execution chain",
            trace.node_id
        );
    }
    for artifact_kind in ["coding_report", "testing_report", "code_review_report"] {
        let artifact = result
            .artifacts
            .iter()
            .find(|artifact| artifact["artifact_kind"] == artifact_kind)
            .expect("artifact exists");
        assert_eq!(
            artifact["_aria"]["traceability_refs"],
            json!(["req-001", "dd-001", "task-001"])
        );
        assert_eq!(
            artifact["_aria"]["provider_run_refs"]
                .as_array()
                .expect("provider runs")
                .len(),
            1
        );
    }
    assert_eq!(result.next_node, "M20");
    assert_eq!(result.rework_counter, 0);
}

#[test]
fn provider_candidate_traceability_refs_are_candidates_not_trusted_coverage() {
    let workspace = tempfile::tempdir().expect("workspace");
    let provider = ScriptedExecutionProvider::with_candidate_refs(["req-999"]);

    let result =
        run_worktask_execution_chain(execution_worktask_input(workspace.path()), &provider).expect(
            "execution chain should reject unknown candidate refs without closing coverage",
        );

    let coding_report = result
        .artifacts
        .iter()
        .find(|artifact| artifact["artifact_kind"] == "coding_report")
        .expect("coding report");
    assert_eq!(
        coding_report["_aria"]["traceability_refs"],
        json!(["req-001", "dd-001", "task-001"])
    );
    assert!(
        !coding_report["_aria"]["traceability_refs"]
            .as_array()
            .expect("traceability refs")
            .iter()
            .any(|value| value == "req-999"),
        "provider candidate ref must not become trusted coverage"
    );
}

#[test]
fn testing_failure_routes_to_rework_then_back_to_testing_and_review() {
    let workspace = tempfile::tempdir().expect("workspace");
    let provider = ScriptedExecutionProvider::testing_fails_then_passes();

    let result =
        run_worktask_execution_chain(execution_worktask_input(workspace.path()), &provider)
            .expect("execution chain with rework");

    assert_eq!(
        result
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["N16", "N17", "N19", "N17", "N18"]
    );
    assert_eq!(result.rework_counter, 1);
    assert_eq!(
        result.protocol_steps[2].node_specific_fields["rework_scope"]["source"],
        "testing_report_worktask_001_0001"
    );
    assert_eq!(
        result.protocol_steps[2].node_specific_fields["superseded_report_refs"],
        json!(["coding_report_worktask_001_0001"])
    );
    assert!(result
        .workflow_skills_activated
        .contains(&"systematic-debugging".to_string()));
    assert_eq!(result.next_node, "M20");
}

#[test]
fn review_revise_routes_to_rework_and_rechecks_before_ready() {
    let workspace = tempfile::tempdir().expect("workspace");
    let provider = ScriptedExecutionProvider::review_revises_then_passes();

    let result =
        run_worktask_execution_chain(execution_worktask_input(workspace.path()), &provider)
            .expect("execution chain with review revise");

    assert_eq!(
        result
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["N16", "N17", "N18", "N19", "N17", "N18"]
    );
    assert_eq!(result.rework_counter, 1);
    assert_eq!(
        result.protocol_steps[3].node_specific_fields["rework_scope"]["source"],
        "code_review_report_worktask_001_0001"
    );
    assert_eq!(result.next_node, "M20");
}

#[test]
fn rework_counter_limit_routes_to_manual_intervention_hold() {
    let workspace = tempfile::tempdir().expect("workspace");
    let provider = ScriptedExecutionProvider::testing_always_fails();
    assert_eq!(
        LoopCounterRegistry::phase1().threshold(LoopCounterName::Rework),
        3
    );

    let result =
        run_worktask_execution_chain(execution_worktask_input(workspace.path()), &provider)
            .expect("execution chain reaches manual hold");

    assert_eq!(result.next_node, "X08");
    assert_eq!(result.rework_counter, 4);
    assert_eq!(
        result.manual_intervention_reason.as_deref(),
        Some("rework_limit_exceeded")
    );
    assert_eq!(
        result
            .protocol_steps
            .last()
            .expect("manual hold step")
            .node_id,
        "X08"
    );
}

#[test]
fn integration_prepare_execute_verify_uses_candidate_commit_and_integration_worktree() {
    let workspace = tempfile::tempdir().expect("workspace");
    let repo = prepare_git_worktask_repo(workspace.path());
    let integration_worktree = workspace.path().join("integration-worktree");
    let mut queue = IntegrationQueue::default();
    let unit = IntegrationPrepareUnit;
    assert_eq!(unit.covered_protocol_nodes(), vec!["N20", "N21", "N22"]);

    let prepare = run_integration_prepare(
        IntegrationPrepareInput {
            session_id: "session_001".to_string(),
            task_id: "task_001".to_string(),
            worktask_id: "worktask_001".to_string(),
            worktree_path: repo.clone(),
            integration_worktree_path: integration_worktree.clone(),
            integration_branch: "aria/integration/task_001".to_string(),
            base_ref: "main".to_string(),
            allowed_write_scope: vec!["src/feature/".to_string()],
        },
        &mut queue,
    )
    .expect("integration prepare");

    assert_eq!(
        prepare
            .protocol_steps
            .iter()
            .map(|step| step.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["N20", "N21", "N22"]
    );
    assert_eq!(
        prepare.protocol_steps[0].node_specific_fields["ready_decision"],
        "ready"
    );
    assert_eq!(
        prepare.protocol_steps[0].node_specific_fields["candidate_commit_sha"],
        prepare.candidate_commit_sha
    );
    assert_eq!(
        prepare.protocol_steps[1].node_specific_fields["queue_position"],
        1
    );
    assert_eq!(
        prepare.protocol_steps[2].node_specific_fields["candidate_commit_sha"],
        prepare.candidate_commit_sha
    );
    assert_eq!(
        queue.records()[0].candidate_commit_sha,
        prepare.candidate_commit_sha
    );
    assert!(integration_worktree.exists());
    assert_eq!(
        git_output(&repo, &["branch", "--show-current"]),
        "aria/worktask_001"
    );

    let execute = run_integration_execute(IntegrationExecuteInput {
        worktask_id: "worktask_001".to_string(),
        integration_worktree_path: integration_worktree.clone(),
        candidate_commit_sha: prepare.candidate_commit_sha.clone(),
        pre_merge_sha: prepare.pre_merge_sha.clone(),
    })
    .expect("integration execute");
    assert_eq!(execute.protocol_step.node_id, "N23");
    assert_eq!(execute.integration_report["status"], "completed");
    assert_eq!(
        execute.integration_report["node_specific_fields"]["integration_commit_sha"],
        execute
            .integration_commit_sha
            .clone()
            .expect("integration commit")
    );
    assert_eq!(execute.next_decision, "verify");
    assert_ne!(execute.post_merge_sha, Some(prepare.pre_merge_sha.clone()));
    assert_eq!(
        git_output(&repo, &["branch", "--show-current"]),
        "aria/worktask_001"
    );

    let verify = run_integration_verify(IntegrationVerifyInput {
        worktask_id: "worktask_001".to_string(),
        integration_worktree_path: integration_worktree,
        pre_merge_sha: prepare.pre_merge_sha,
        verify_passed: true,
    })
    .expect("integration verify");
    assert_eq!(verify.protocol_step.node_id, "N24");
    assert_eq!(verify.verify_decision, "pass");
    assert_eq!(verify.next_decision, "N25");
}

fn execution_input(worktree_path: &std::path::Path) -> ExecutionSetupInput {
    ExecutionSetupInput {
        session_id: "session_001".to_string(),
        task_id: "task_001".to_string(),
        dispatch_package_ref: "dispatch_pkg_001".to_string(),
        dispatch_package: json!({
            "artifact_kind": "dispatch_package",
            "_aria": {
                "worktask_routing": [
                    {
                        "worktask_id": "worktask_001",
                        "source_work_package_id": "WP-001",
                        "execution_mode": "agent_only",
                        "allowed_write_scope": ["src/feature/"]
                    }
                ]
            }
        }),
        plan_projection: json!({
            "work_packages": [
                {
                    "work_package_id": "WP-001",
                    "traceability_refs": ["REQ-001"],
                    "acceptance_targets": ["cargo test --test execution_chain_fake_provider"]
                }
            ]
        }),
        worktree_path: worktree_path.to_path_buf(),
        base_ref: "main".to_string(),
    }
}

fn prepare_git_worktask_repo(workspace: &Path) -> std::path::PathBuf {
    let repo = workspace.join("repo");
    fs::create_dir_all(repo.join("src/feature")).expect("repo dirs");
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.email", "aria@example.invalid"]);
    git(&repo, &["config", "user.name", "Aria Test"]);
    fs::write(repo.join("README.md"), "base\n").expect("readme");
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "base"]);
    git(&repo, &["checkout", "-b", "aria/worktask_001"]);
    fs::write(
        repo.join("src/feature/lib.rs"),
        "pub fn value() -> u32 { 1 }\n",
    )
    .expect("feature file");
    repo
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

fn git_output(cwd: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn execution_worktask_input(workspace_root: &std::path::Path) -> ExecutionWorktaskInput {
    ExecutionWorktaskInput {
        session_id: "session_001".to_string(),
        task_id: "task_001".to_string(),
        worktask_id: "worktask_001".to_string(),
        source_work_package_id: "WP-001".to_string(),
        worktree_path: workspace_root.join("worktree"),
        allowed_write_scope: vec!["src/feature/".to_string()],
        dispatch_package: json!({
            "artifact_kind": "dispatch_package",
            "_aria": {
                "worktask_routing": [
                    {
                        "worktask_id": "worktask_001",
                        "source_work_package_id": "WP-001",
                        "execution_mode": "agent_only",
                        "allowed_write_scope": ["src/feature/"],
                        "traceability_refs": ["req-001", "dd-001", "task-001"],
                        "verification_commands": ["cargo test --test execution_chain_fake_provider"]
                    }
                ]
            }
        }),
        plan_projection: PlanProjection {
            work_packages: vec![WorkPackageProjection {
                work_package_id: "WP-001".to_string(),
                description: "实现执行链".to_string(),
                execution_mode: ExecutionMode::AgentOnly,
                human_required_reason: None,
                traceability_refs: vec![
                    "req-001".to_string(),
                    "dd-001".to_string(),
                    "task-001".to_string(),
                ],
                acceptance_targets: vec![
                    "cargo test --test execution_chain_fake_provider".to_string()
                ],
            }],
            dependencies: vec![],
            parallelism_groups: vec![],
        },
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

#[derive(Debug)]
struct ScriptedExecutionProvider {
    output_schemas: Mutex<Vec<String>>,
    testing_passes: Mutex<VecDeque<bool>>,
    review_decisions: Mutex<VecDeque<String>>,
    candidate_refs: Vec<String>,
}

impl ScriptedExecutionProvider {
    fn happy() -> Self {
        Self::new([true], ["pass"])
    }

    fn testing_fails_then_passes() -> Self {
        Self::new([false, true], ["pass"])
    }

    fn review_revises_then_passes() -> Self {
        Self::new([true, true], ["revise", "pass"])
    }

    fn testing_always_fails() -> Self {
        Self::new([false, false, false, false], ["pass"])
    }

    fn with_candidate_refs<const C: usize>(candidate_refs: [&str; C]) -> Self {
        let mut provider = Self::happy();
        provider.candidate_refs = candidate_refs.into_iter().map(ToOwned::to_owned).collect();
        provider
    }

    fn new<const T: usize, const R: usize>(testing_passes: [bool; T], reviews: [&str; R]) -> Self {
        Self {
            output_schemas: Mutex::new(Vec::new()),
            testing_passes: Mutex::new(testing_passes.into_iter().collect()),
            review_decisions: Mutex::new(reviews.into_iter().map(ToOwned::to_owned).collect()),
            candidate_refs: Vec::new(),
        }
    }

    fn seen_output_schemas(&self) -> Vec<String> {
        self.output_schemas.lock().expect("schemas").clone()
    }
}

impl ProviderAdapter for ScriptedExecutionProvider {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.output_schemas
            .lock()
            .expect("schemas")
            .push(input.output_schema.clone());
        let payload = match input.output_schema.as_str() {
            "schema://aria/artifacts/coding_report/v1" => json!({
                "artifact_kind": "coding_report",
                "artifact_ref": "coding_report_worktask_001_0001",
                "worktask_id": "worktask_001",
                "files_modified": ["src/feature/lib.rs"],
                "commands_run": ["cargo test --test execution_chain_fake_provider"],
                "candidate_traceability_refs": self.candidate_refs.clone(),
                "status": "completed"
            }),
            "schema://aria/artifacts/testing_report/v1" => {
                let passed = self
                    .testing_passes
                    .lock()
                    .expect("testing passes")
                    .pop_front()
                    .unwrap_or(true);
                json!({
                    "artifact_kind": "testing_report",
                    "artifact_ref": "testing_report_worktask_001_0001",
                    "worktask_id": "worktask_001",
                    "commands_run": ["cargo test --test execution_chain_fake_provider"],
                    "tests_passed": passed,
                    "failures": if passed {
                        json!([])
                    } else {
                        json!([{"test": "execution_chain", "message": "fixture failure"}])
                    },
                    "candidate_traceability_refs": []
                })
            }
            "schema://aria/artifacts/code_review_report/v1" => {
                let decision = self
                    .review_decisions
                    .lock()
                    .expect("review decisions")
                    .pop_front()
                    .unwrap_or_else(|| "pass".to_string());
                json!({
                    "artifact_kind": "code_review_report",
                    "artifact_ref": "code_review_report_worktask_001_0001",
                    "worktask_id": "worktask_001",
                    "findings": if decision == "revise" {
                        json!([{"finding_id": "finding-001", "summary": "补充失败项修复"}])
                    } else {
                        json!([])
                    },
                    "blocking": decision == "revise",
                    "candidate_traceability_refs": []
                })
            }
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
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect(),
            duration_ms: 1,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}
