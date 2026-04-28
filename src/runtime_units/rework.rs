pub use crate::protocol::loop_counters::{LoopCounterName, LoopCounterRegistry};
use crate::runtime_units::{
    CanonicalNodeInput, DaemonContext, RuntimeUnit, RuntimeUnitError, RuntimeUnitResult,
};
use std::future::Future;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReworkUnit;

impl RuntimeUnit for ReworkUnit {
    fn unit_id(&self) -> &'static str {
        "rework"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N19"]
    }

    fn execute(
        &self,
        _input: CanonicalNodeInput,
        _ctx: &DaemonContext,
    ) -> impl Future<Output = Result<RuntimeUnitResult, RuntimeUnitError>> + Send {
        async {
            Err(RuntimeUnitError {
                code: "provider_adapter_required".to_string(),
                message: "N19 requires provider execution chain injection".to_string(),
            })
        }
    }
}
