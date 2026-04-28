use crate::runtime_units::{
    completed_step, CanonicalNodeInput, DaemonContext, RuntimeUnit, RuntimeUnitError,
    RuntimeUnitResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntakeBrief;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntakeCaptureUnit;

impl RuntimeUnit for IntakeCaptureUnit {
    fn unit_id(&self) -> &'static str {
        "intake_capture"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N01"]
    }

    async fn execute(
        &self,
        _input: CanonicalNodeInput,
        _ctx: &DaemonContext,
    ) -> Result<RuntimeUnitResult, RuntimeUnitError> {
        Ok(RuntimeUnitResult {
            protocol_steps: vec![completed_step("N01")],
            produced_artifact_refs: vec![],
        })
    }
}
