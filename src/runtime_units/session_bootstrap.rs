use crate::runtime_units::{
    completed_step, CanonicalNodeInput, DaemonContext, RuntimeUnit, RuntimeUnitError,
    RuntimeUnitResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionBootstrapUnit;

impl RuntimeUnit for SessionBootstrapUnit {
    fn unit_id(&self) -> &'static str {
        "session_bootstrap"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N00"]
    }

    async fn execute(
        &self,
        _input: CanonicalNodeInput,
        _ctx: &DaemonContext,
    ) -> Result<RuntimeUnitResult, RuntimeUnitError> {
        Ok(RuntimeUnitResult {
            protocol_steps: vec![completed_step("N00")],
            produced_artifact_refs: vec![],
        })
    }
}
