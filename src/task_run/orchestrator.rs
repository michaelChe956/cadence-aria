use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::protocol::contracts::{AdapterInput, AdapterOutput};
use crate::protocol::projections::{PlanProjection, ProjectionPayload};
use crate::runtime_units::clarification::PlanningStartChainInput;
use crate::runtime_units::coding::{ExecutionWorktaskInput, run_worktask_execution_chain};
use crate::runtime_units::final_review::{FinalClosureInput, run_final_closure_chain};
use crate::runtime_units::plan_dispatch::run_planning_full_chain;
use crate::task_run::openspec_bootstrap::{
    bootstrap_task_openspec, build_initial_constraint_bundle,
};
use crate::task_run::store::{TaskRunStore, preflight_workspace};
use crate::task_run::types::{TaskRunError, TaskRunOutcome, TaskRunRequest, TaskRunStatus};

pub struct TaskRunOrchestrator;

impl TaskRunOrchestrator {
    pub fn run_with_provider(
        request: TaskRunRequest,
        provider: &dyn ProviderAdapter,
    ) -> Result<TaskRunOutcome, TaskRunError> {
        if !request.non_interactive {
            return Err(TaskRunError::new(
                "task_run_requires_non_interactive",
                "task run requires non_interactive execution",
            ));
        }

        preflight_workspace(&request.workspace)?;
        let provider = TimeoutOverrideProvider::new(provider, request.timeout_secs);

        let task_id = "task_0001".to_string();
        let session_id = format!("sess_{task_id}");
        let store = TaskRunStore::new(&request.workspace, &task_id);
        let task_state_path = store.write_task_state(&json!({
            "task_id": task_id,
            "phase": "intake",
            "change_id": request.change_id,
            "openspec_bootstrap_status": "bootstrap_pending",
        }))?;

        let change_dir = bootstrap_task_openspec(
            &request.workspace,
            &request.change_id,
            &request.request_text,
            &task_state_path,
        )?;
        let initial_bundle = build_initial_constraint_bundle(
            &request.change_id,
            &change_dir,
            &request.request_text,
        )?;
        let planning = run_planning_full_chain(
            PlanningStartChainInput {
                session_id: session_id.clone(),
                task_id: task_id.clone(),
                change_id: request.change_id.clone(),
                workspace_root: request.workspace.clone(),
                worktree_path: Some(request.workspace.to_string_lossy().to_string()),
                intake_brief: json!({
                    "artifact_kind": "intake_brief",
                    "request_summary": request.request_text,
                    "raw_user_request": request.request_text,
                    "repo_context": {
                        "workspace": request.workspace.to_string_lossy(),
                    },
                    "initial_constraints": ["non_interactive_task_run"],
                    "requested_goal": "task_run_e2e",
                }),
                initial_constraint_bundle: initial_bundle,
            },
            &provider,
        )
        .map_err(|error| TaskRunError::new(error.runtime_code(), error.to_string()))?;

        let plan_projection = extract_plan_projection(&planning.plan_projection.payload)?;
        let mut provider_run_refs = planning
            .provider_run_records
            .iter()
            .map(|record| record.provider_run_id.clone())
            .collect::<Vec<_>>();
        let mut canonical_artifact_refs = vec![
            planning.dispatch_ref.artifact_ref_id.clone(),
            planning.plan_ref.artifact_ref_id.clone(),
        ];
        let projection_refs = planning
            .openspec_bundle_after_tasks
            .compiled_from_projection_refs
            .clone();

        for input in execution_inputs(
            &request.workspace,
            &session_id,
            &task_id,
            &planning.dispatch_package,
            &plan_projection,
            &projection_refs,
            &planning.openspec_bundle_after_tasks.constraint_bundle_id,
        )? {
            let execution = run_worktask_execution_chain(input, &provider)
                .map_err(|error| TaskRunError::new("execution_chain_failed", error.to_string()))?;
            provider_run_refs.extend(
                execution
                    .provider_run_records
                    .iter()
                    .map(|record| record.provider_run_id.clone()),
            );
            for (index, artifact) in execution.artifacts.iter().enumerate() {
                let path = store.write_json_artifact(
                    &format!("artifacts/execution/{index:04}.json"),
                    artifact,
                )?;
                if artifact["artifact_kind"] == "testing_report" {
                    store.write_json_report("testing-report.json", artifact)?;
                }
                if let Some(ref_id) = artifact.get("artifact_ref").and_then(Value::as_str) {
                    canonical_artifact_refs.push(ref_id.to_string());
                }
                canonical_artifact_refs.push(path.to_string_lossy().to_string());
            }

            if execution.manual_intervention_reason.is_some() {
                let blocked_path = store.write_json_report(
                    "blocked-report.json",
                    &json!({
                        "task_id": task_id,
                        "status": "blocked_by_gate",
                        "reason": execution.manual_intervention_reason,
                        "next_node": execution.next_node,
                    }),
                )?;
                let report_path = store.write_json_report(
                    "final-report.json",
                    &json!({
                        "task_id": task_id,
                        "change_id": request.change_id,
                        "status": "blocked_by_gate",
                        "blocked_report_path": blocked_path,
                    }),
                )?;
                return Ok(TaskRunOutcome {
                    task_id,
                    change_id: request.change_id,
                    status: TaskRunStatus::BlockedByGate,
                    report_path,
                    openspec_change_dir: change_dir,
                    provider_run_refs,
                    testing_report_path: Some(
                        store.task_root().join("reports/testing-report.json"),
                    ),
                    final_summary_path: None,
                    blocked_report_path: Some(blocked_path),
                });
            }
        }

        let traceability_refs = plan_projection
            .work_packages
            .iter()
            .flat_map(|work_package| work_package.traceability_refs.clone())
            .collect::<Vec<_>>();
        let integration_report = integration_report_from_plan(&plan_projection);
        let integration_report_path =
            store.write_json_report("integration-report.json", &integration_report)?;
        canonical_artifact_refs.push("integration_report_task_0001_0001".to_string());
        let final_result = run_final_closure_chain(
            FinalClosureInput {
                session_id,
                task_id: task_id.clone(),
                projection_refs: projection_refs.clone(),
                constraint_bundle_ref: planning
                    .openspec_bundle_after_tasks
                    .constraint_bundle_id
                    .clone(),
                risk_registry_ref: format!("riskreg_{task_id}_v0001"),
                canonical_artifact_refs,
                traceability_refs,
                context_files: vec![
                    change_dir.join("proposal.md").to_string_lossy().to_string(),
                    change_dir
                        .join("specs/main/spec.md")
                        .to_string_lossy()
                        .to_string(),
                    change_dir.join("design.md").to_string_lossy().to_string(),
                    change_dir.join("tasks.md").to_string_lossy().to_string(),
                ],
            },
            &provider,
        )
        .map_err(|error| TaskRunError::new("final_closure_failed", error.to_string()))?;
        provider_run_refs.extend(
            final_result
                .provider_run_records
                .iter()
                .map(|record| record.provider_run_id.clone()),
        );

        let final_summary_path =
            store.write_json_report("final-summary.json", &final_result.final_summary)?;
        let testing_report_path = store.task_root().join("reports/testing-report.json");
        let report_path = store.write_json_report(
            "final-report.json",
            &json!({
                "task_id": task_id,
                "change_id": request.change_id,
                "status": "completed",
                "openspec_change_dir": change_dir,
                "provider_run_refs": provider_run_refs,
                "testing_report_path": testing_report_path,
                "integration_report_path": integration_report_path,
                "final_summary_path": final_summary_path,
            }),
        )?;

        Ok(TaskRunOutcome {
            task_id,
            change_id: request.change_id,
            status: TaskRunStatus::Completed,
            report_path,
            openspec_change_dir: change_dir,
            provider_run_refs,
            testing_report_path: Some(testing_report_path),
            final_summary_path: Some(final_summary_path),
            blocked_report_path: None,
        })
    }
}

struct TimeoutOverrideProvider<'a> {
    inner: &'a dyn ProviderAdapter,
    timeout_secs: u64,
}

impl<'a> TimeoutOverrideProvider<'a> {
    fn new(inner: &'a dyn ProviderAdapter, timeout_secs: u64) -> Self {
        Self {
            inner,
            timeout_secs,
        }
    }
}

impl ProviderAdapter for TimeoutOverrideProvider<'_> {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        let mut input = input.clone();
        input.timeout = self.timeout_secs;
        self.inner.run(&input)
    }
}

fn integration_report_from_plan(plan_projection: &PlanProjection) -> Value {
    let worktasks = plan_projection
        .work_packages
        .iter()
        .map(|work_package| format!("work_{}", work_package.work_package_id.replace('-', "_")))
        .collect::<Vec<_>>();
    json!({
        "artifact_kind": "integration_report",
        "artifact_ref": "integration_report_task_0001_0001",
        "integrated_worktasks": worktasks,
        "status": "completed",
        "node_specific_fields": {
            "integration_commit_sha": null,
            "post_merge_sha": null,
            "rollback_ref": null,
            "next_decision": "final_review",
        },
    })
}

fn extract_plan_projection(payload: &ProjectionPayload) -> Result<PlanProjection, TaskRunError> {
    match payload {
        ProjectionPayload::PlanProjection(plan) => Ok(plan.clone()),
        _ => Err(TaskRunError::new(
            "plan_projection_missing",
            "planning result did not contain plan projection",
        )),
    }
}

fn execution_inputs(
    workspace: &Path,
    session_id: &str,
    task_id: &str,
    dispatch_package: &Value,
    plan_projection: &PlanProjection,
    projection_refs: &[String],
    constraint_bundle_ref: &str,
) -> Result<Vec<ExecutionWorktaskInput>, TaskRunError> {
    let routes = dispatch_package
        .pointer("/_aria/worktask_routing")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            TaskRunError::new(
                "dispatch_routing_missing",
                "dispatch package has no routing",
            )
        })?;
    let mut inputs = Vec::new();
    for (index, route) in routes.iter().enumerate() {
        let source_work_package_id = route
            .get("source_work_package_id")
            .and_then(Value::as_str)
            .unwrap_or("WT-001")
            .to_string();
        let worktask_id = route
            .get("worktask_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("worktask_{:03}", index + 1));
        let allowed_write_scope = route
            .get("allowed_write_scope")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        inputs.push(ExecutionWorktaskInput {
            session_id: session_id.to_string(),
            task_id: task_id.to_string(),
            worktask_id,
            source_work_package_id,
            worktree_path: PathBuf::from(workspace),
            allowed_write_scope,
            dispatch_package: dispatch_package.clone(),
            plan_projection: plan_projection.clone(),
            projection_refs: projection_refs.to_vec(),
            constraint_bundle_ref: constraint_bundle_ref.to_string(),
            risk_registry_ref: format!("riskreg_{task_id}_v0001"),
            context_files: Vec::new(),
        });
    }
    if inputs.is_empty() {
        return Err(TaskRunError::new(
            "dispatch_routing_empty",
            "dispatch package did not produce worktask routing",
        ));
    }
    Ok(inputs)
}
