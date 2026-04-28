use crate::runtime_units::{
    CanonicalNodeInput, DaemonContext, RuntimeUnit, RuntimeUnitError, RuntimeUnitResult,
};
use std::future::Future;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestingUnit;

impl RuntimeUnit for TestingUnit {
    fn unit_id(&self) -> &'static str {
        "testing"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N17"]
    }

    fn execute(
        &self,
        _input: CanonicalNodeInput,
        _ctx: &DaemonContext,
    ) -> impl Future<Output = Result<RuntimeUnitResult, RuntimeUnitError>> + Send {
        async {
            Err(RuntimeUnitError {
                code: "provider_adapter_required".to_string(),
                message: "N17 requires provider execution chain injection".to_string(),
            })
        }
    }
}
