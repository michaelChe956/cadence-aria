use crate::runtime_units::{
    CanonicalNodeInput, DaemonContext, RuntimeUnit, RuntimeUnitError, RuntimeUnitResult,
};
use std::future::Future;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCloseoutUnit;

impl RuntimeUnit for SessionCloseoutUnit {
    fn unit_id(&self) -> &'static str {
        "session_closeout"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N28"]
    }

    fn execute(
        &self,
        _input: CanonicalNodeInput,
        _ctx: &DaemonContext,
    ) -> impl Future<Output = Result<RuntimeUnitResult, RuntimeUnitError>> + Send {
        async {
            Err(RuntimeUnitError {
                code: "session_closeout_requires_final_state".to_string(),
                message: "N28 requires final closure state".to_string(),
            })
        }
    }
}
