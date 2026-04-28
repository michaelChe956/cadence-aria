use crate::runtime_units::{
    completed_step, CanonicalNodeInput, DaemonContext, RuntimeUnit, RuntimeUnitError,
    RuntimeUnitResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskInitResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskInitUnit;

impl RuntimeUnit for TaskInitUnit {
    fn unit_id(&self) -> &'static str {
        "task_init"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N02", "N03"]
    }

    async fn execute(
        &self,
        _input: CanonicalNodeInput,
        _ctx: &DaemonContext,
    ) -> Result<RuntimeUnitResult, RuntimeUnitError> {
        Ok(RuntimeUnitResult {
            protocol_steps: vec![completed_step("N02"), completed_step("N03")],
            produced_artifact_refs: vec![],
        })
    }
}
