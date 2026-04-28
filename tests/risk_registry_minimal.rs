use cadence_aria::cross_cutting::provider_context_builder::{
    build_provider_context, ProviderContextBuilderInput,
};
use cadence_aria::cross_cutting::provider_run::{
    allocate_next_risk_id, append_risk_entry, load_risk_registry_snapshot,
    provider_run_record_from_output, recover_risk_registry_snapshot,
    sync_risk_registry_to_snapshot, RiskEntryInput,
};
use cadence_aria::cross_cutting::traceability::{normalize_traceability, TraceabilityIndexes};
use cadence_aria::daemon::checkpoint::RuntimeSnapshot;
use cadence_aria::daemon::state_machine::DaemonState;
use cadence_aria::protocol::artifacts::{risk_ids_from_artifact_refs, RiskStatus};
use cadence_aria::protocol::contracts::{
    AdapterInput, AdapterOutput, AdapterRole, ApprovalPolicy, ProviderType, RuntimeRole,
    SandboxMode, TimeoutStatus,
};
use cadence_aria::protocol::projections::{
    ExecutionMode, PlanProjection, RiskSeverity, WorkPackageProjection,
};
use cadence_aria::protocol::repl_wire::NewTaskRequest;
use serde_json::{json, Value};

#[test]
fn risk_registry_entries_persist_recover_and_bind_to_artifact_refs() {
    let workspace = tempfile::tempdir().expect("workspace");
    let mut daemon = DaemonState::bootstrap(workspace.path()).expect("bootstrap daemon");
    let response = daemon
        .new_task(NewTaskRequest {
            request_text: "实现 risk registry 最小闭环".to_string(),
            requested_change_id: None,
        })
        .expect("new task");
    let task = daemon.task(&response.task_id).expect("task");
    let task_root = workspace
        .path()
        .join(".aria/runtime/tasks")
        .join(&response.task_id);

    let initial = load_risk_registry_snapshot(&task_root).expect("initial registry");
    assert_eq!(initial.risk_registry_ref, task.risk_registry_ref);
    assert!(initial.risks.is_empty());
    assert_eq!(initial.risk_ids, Vec::<String>::new());
    assert_eq!(allocate_next_risk_id(&initial), "risk-001");

    let first = append_risk_entry(
        &task_root,
        RiskEntryInput {
            description: "N08 发现设计评审风险".to_string(),
            severity: RiskSeverity::High,
            status: RiskStatus::Open,
            source_artifact: Some("art_ref_design_review_0001".to_string()),
            source_node: "N08".to_string(),
            resolution: None,
        },
    )
    .expect("append first risk");
    assert_eq!(first.risk_ids, vec!["risk-001".to_string()]);
    assert_eq!(first.risks[0].risk_id, "risk-001");
    assert_eq!(first.risks[0].description, "N08 发现设计评审风险");
    assert_eq!(first.risks[0].severity, RiskSeverity::High);
    assert_eq!(first.risks[0].status, RiskStatus::Open);

    let second = append_risk_entry(
        &task_root,
        RiskEntryInput {
            description: "N10 发现 readiness 风险".to_string(),
            severity: RiskSeverity::Medium,
            status: RiskStatus::Open,
            source_artifact: Some("art_ref_readiness_0001".to_string()),
            source_node: "N10".to_string(),
            resolution: None,
        },
    )
    .expect("append second risk");
    assert_eq!(
        second.risk_ids,
        vec!["risk-001".to_string(), "risk-002".to_string()]
    );

    let registry_json: Value = serde_json::from_slice(
        &std::fs::read(task_root.join("risk-registry/registry.json")).expect("registry json"),
    )
    .expect("registry value");
    assert_eq!(registry_json["risks"][1]["risk_id"], json!("risk-002"));
    assert_eq!(registry_json["risks"][1]["status"], json!("open"));

    let ref_json: Value = serde_json::from_slice(
        &std::fs::read(
            task_root
                .join("risk-registry/refs")
                .join(format!("{}.json", task.risk_registry_ref)),
        )
        .expect("registry ref json"),
    )
    .expect("registry ref value");
    assert_eq!(ref_json["risk_registry_ref_id"], task.risk_registry_ref);
    assert_eq!(ref_json["risk_count"], json!(2));
    assert_eq!(ref_json["path"], json!("risk-registry/registry.json"));
    assert_eq!(ref_json["sha256"].as_str().expect("sha").len(), 64);

    let snapshot_path = task_root.join("snapshots/N08.json");
    let mut snapshot = RuntimeSnapshot::minimal_for_test("N08");
    snapshot.session_id = "sess_risk".to_string();
    snapshot.task_id = response.task_id.clone();
    snapshot.risk_registry.risk_registry_ref = task.risk_registry_ref.clone();
    std::fs::write(
        &snapshot_path,
        serde_json::to_vec_pretty(&snapshot).expect("snapshot json"),
    )
    .expect("write snapshot");
    sync_risk_registry_to_snapshot(&snapshot_path, &second).expect("sync snapshot");
    let snapshot_json: Value =
        serde_json::from_slice(&std::fs::read(&snapshot_path).expect("snapshot"))
            .expect("snapshot value");
    assert_eq!(
        snapshot_json["risk_registry"]["risk_ids"],
        json!(["risk-001", "risk-002"])
    );
    assert_eq!(
        snapshot_json["risk_registry"]["risks"][0]["risk_id"],
        json!("risk-001")
    );

    let recovered_daemon = DaemonState::recover(workspace.path()).expect("recover daemon");
    assert!(recovered_daemon.task(&response.task_id).is_some());
    let recovered_registry =
        recover_risk_registry_snapshot(workspace.path(), &response.task_id).expect("recover risk");
    assert_eq!(recovered_registry.risk_ids, second.risk_ids);

    assert_eq!(
        risk_ids_from_artifact_refs(&[
            "REQ-001".to_string(),
            "risk-001".to_string(),
            "RISK-002".to_string(),
            "dd-001".to_string(),
        ]),
        vec!["risk-001".to_string(), "risk-002".to_string()]
    );

    let mut report = coding_report("work_001");
    let binding = normalize_traceability(
        &mut report,
        vec!["risk-002".to_string()],
        &dispatch_package(),
        &plan_projection(),
        &TraceabilityIndexes::new(vec![
            "req-001".to_string(),
            "risk-001".to_string(),
            "risk-002".to_string(),
        ]),
    )
    .expect("traceability binding");
    assert_eq!(
        binding.related_risk_ids,
        vec!["risk-001".to_string(), "risk-002".to_string()]
    );
}

#[test]
fn provider_run_record_does_not_carry_risk_registry_ref_and_context_requires_it() {
    let record = provider_run_record_from_output(
        &cadence_aria::cross_cutting::provider_router::ProviderRunRequest {
            provider_run_id: "prun_001".to_string(),
            node_id: "N08".to_string(),
            runtime_role: RuntimeRole::Reviewer,
            provider_capability_ref: "cap_fake".to_string(),
            adapter_compatibility_ref: "compat_fake".to_string(),
            context_package_ref: "ctx_001".to_string(),
            adapter_input_ref: "ain_001".to_string(),
            adapter_output_ref: "aout_001".to_string(),
            approval_policy: ApprovalPolicy::OnRequest,
            sandbox_mode: SandboxMode::WorkspaceWrite,
            constraint_check_ref: None,
            traceability_binding_refs: vec![],
        },
        &adapter_input(json!({"risk_registry_ref": "riskreg_task_0001_v0001"})),
        &AdapterOutput {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            structured_output: Some(json!({"artifact_kind": "design_review"})),
            files_modified: vec![],
            duration_ms: 1,
            timeout_status: TimeoutStatus::NotTimedOut,
        },
    );
    let record_value = serde_json::to_value(record).expect("record json");
    assert!(
        record_value.get("risk_registry_ref").is_none(),
        "ProviderRunRecord must not directly carry risk_registry_ref"
    );

    let error = build_provider_context(builder_input_without_risk_registry_ref())
        .expect_err("missing risk_registry_ref must fail");
    assert_eq!(
        error.to_string(),
        "risk_registry_ref is required in canonical_inputs"
    );
}

fn builder_input_without_risk_registry_ref() -> ProviderContextBuilderInput {
    ProviderContextBuilderInput {
        session_id: "session_001".to_string(),
        task_id: "task_001".to_string(),
        node_id: "N08".to_string(),
        canonical_inputs: json!({"artifact_refs": ["art_ref_design_0001"]}),
        canonical_input_summary: "design without risk registry".to_string(),
        projection_refs: vec!["proj_design_projection_art_design_001_0001".to_string()],
        projection_summary: "projection summary".to_string(),
        constraint_bundle_ref: "constraint_bundle_001".to_string(),
        constraint_summary: "constraint summary".to_string(),
        context_files: vec![],
        worktree_path: None,
    }
}

fn adapter_input(canonical_inputs: Value) -> AdapterInput {
    AdapterInput {
        provider_type: ProviderType::Fake,
        role: AdapterRole::Reviewer,
        worktree_path: None,
        prompt: canonical_inputs.to_string(),
        context_files: vec![],
        output_schema: "schema://aria/artifacts/design_review/v1".to_string(),
        timeout: 30,
        max_retries: 1,
    }
}

fn plan_projection() -> PlanProjection {
    PlanProjection {
        work_packages: vec![WorkPackageProjection {
            work_package_id: "wt-001".to_string(),
            description: "实现风险绑定".to_string(),
            execution_mode: ExecutionMode::AgentOnly,
            human_required_reason: None,
            traceability_refs: vec!["req-001".to_string(), "risk-001".to_string()],
            acceptance_targets: vec!["ac-001".to_string()],
        }],
        dependencies: vec![],
        parallelism_groups: vec![],
    }
}

fn dispatch_package() -> Value {
    json!({
        "artifact_kind": "dispatch_package",
        "_aria": {
            "worktask_routing": [
                {
                    "worktask_id": "work_001",
                    "source_work_package_id": "wt-001",
                    "execution_mode": "agent_only",
                    "allowed_write_scope": ["src/"],
                    "traceability_refs": ["req-001", "risk-001"],
                    "verification_commands": ["cargo test -j 1"]
                }
            ]
        }
    })
}

fn coding_report(worktask_id: &str) -> Value {
    json!({
        "artifact_kind": "coding_report",
        "artifact_ref": "art_ref_coding_report_0001",
        "worktask_id": worktask_id,
        "files_modified": ["src/lib.rs"],
        "commands_run": ["cargo test -j 1"],
        "candidate_traceability_refs": [],
        "status": "completed",
        "_aria": {
            "profile_version": "phase1.v1"
        }
    })
}
