use crate::cross_cutting::worktree::{WorktreeLease, WorktreeLeaseManager};
use crate::runtime_units::{
    RuntimeProtocolStep, RuntimeStepStatus, RuntimeUnit, RuntimeUnitError, RuntimeUnitResult,
};
use serde_json::{json, Value};
use std::future::Future;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionSetupInput {
    pub session_id: String,
    pub task_id: String,
    pub dispatch_package_ref: String,
    pub dispatch_package: Value,
    pub plan_projection: Value,
    pub worktree_path: PathBuf,
    pub base_ref: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionRouteContext {
    pub worktask_id: String,
    pub source_work_package_id: String,
    pub lease_id: String,
    pub worktree_path: String,
    pub branch_name: String,
    pub allowed_write_scope: Vec<String>,
    pub traceability_refs: Vec<String>,
    pub acceptance_targets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionSetupResult {
    pub protocol_steps: Vec<RuntimeProtocolStep>,
    pub route_contexts: Vec<ExecutionRouteContext>,
    pub leases: Vec<WorktreeLease>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ExecutionSetupError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionSetupUnit;

impl RuntimeUnit for ExecutionSetupUnit {
    fn unit_id(&self) -> &'static str {
        "execution_setup"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N13", "N14", "N15"]
    }

    fn execute(
        &self,
        _input: crate::runtime_units::CanonicalNodeInput,
        _ctx: &crate::runtime_units::DaemonContext,
    ) -> impl Future<Output = Result<RuntimeUnitResult, RuntimeUnitError>> + Send {
        async {
            Err(RuntimeUnitError {
                code: "execution_setup_requires_worktree_manager".to_string(),
                message: "N13-N15 requires run_execution_setup with WorktreeLeaseManager"
                    .to_string(),
            })
        }
    }
}

pub fn run_execution_setup(
    input: ExecutionSetupInput,
    manager: &mut WorktreeLeaseManager,
) -> Result<ExecutionSetupResult, ExecutionSetupError> {
    let routings = worktask_routing(&input.dispatch_package)?;
    let work_packages = work_packages(&input.plan_projection);
    let mut protocol_steps = Vec::new();
    let mut route_contexts = Vec::new();
    let mut leases = Vec::new();

    for routing in routings {
        let worktask_id = string_field(&routing, "worktask_id")?;
        let source_work_package_id = string_field(&routing, "source_work_package_id")?;
        let allowed_write_scope = string_array_field(&routing, "allowed_write_scope");
        let Some(work_package) = work_packages
            .iter()
            .find(|package| package.work_package_id == source_work_package_id)
        else {
            return Err(ExecutionSetupError {
                code: "worktask_source_work_package_missing".to_string(),
                message: format!(
                    "{source_work_package_id} referenced by {worktask_id} is not present in PlanProjection.work_packages"
                ),
            });
        };
        let branch_name = format!("aria/{worktask_id}");

        protocol_steps.push(RuntimeProtocolStep {
            node_id: "N13".to_string(),
            status: RuntimeStepStatus::Completed,
            node_specific_fields: json!({
                "worktask_id": worktask_id,
                "routing_ref": format!("{}#{}", input.dispatch_package_ref, worktask_id),
                "state": "registered",
            }),
        });

        let lease = manager
            .acquire(&worktask_id, &branch_name, allowed_write_scope.clone())
            .map_err(|error| ExecutionSetupError {
                code: "worktree_lease_failed".to_string(),
                message: error.to_string(),
            })?;
        protocol_steps.push(RuntimeProtocolStep {
            node_id: "N14".to_string(),
            status: RuntimeStepStatus::Completed,
            node_specific_fields: json!({
                "worktree_path": lease.worktree_path,
                "lease_id": lease.lease_id,
                "base_ref": input.base_ref,
                "branch_name": branch_name,
            }),
        });
        protocol_steps.push(RuntimeProtocolStep {
            node_id: "N15".to_string(),
            status: RuntimeStepStatus::Completed,
            node_specific_fields: json!({
                "dispatch_package_ref": input.dispatch_package_ref,
                "worktask_routing": [routing.clone()],
            }),
        });

        route_contexts.push(ExecutionRouteContext {
            worktask_id,
            source_work_package_id,
            lease_id: lease.lease_id.clone(),
            worktree_path: input.worktree_path.to_string_lossy().to_string(),
            branch_name,
            allowed_write_scope,
            traceability_refs: work_package.traceability_refs.clone(),
            acceptance_targets: work_package.acceptance_targets.clone(),
        });
        leases.push(lease);
    }

    Ok(ExecutionSetupResult {
        protocol_steps,
        route_contexts,
        leases,
    })
}

fn worktask_routing(dispatch_package: &Value) -> Result<Vec<Value>, ExecutionSetupError> {
    dispatch_package
        .pointer("/_aria/worktask_routing")
        .and_then(Value::as_array)
        .map(|items| items.to_vec())
        .filter(|items| !items.is_empty())
        .ok_or_else(|| ExecutionSetupError {
            code: "worktask_routing_missing".to_string(),
            message: "_aria.worktask_routing[] is required".to_string(),
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkPackage {
    work_package_id: String,
    traceability_refs: Vec<String>,
    acceptance_targets: Vec<String>,
}

fn work_packages(plan_projection: &Value) -> Vec<WorkPackage> {
    plan_projection
        .get("work_packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            Some(WorkPackage {
                work_package_id: value.get("work_package_id")?.as_str()?.to_string(),
                traceability_refs: string_array_field(value, "traceability_refs"),
                acceptance_targets: string_array_field(value, "acceptance_targets"),
            })
        })
        .collect()
}

fn string_field(value: &Value, field: &str) -> Result<String, ExecutionSetupError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| ExecutionSetupError {
            code: format!("{field}_missing"),
            message: format!("{field} is required"),
        })
}

fn string_array_field(value: &Value, field: &str) -> Vec<String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}
