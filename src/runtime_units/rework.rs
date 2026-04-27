use crate::runtime_units::{
    CanonicalNodeInput, DaemonContext, RuntimeUnit, RuntimeUnitError, RuntimeUnitResult,
};
use std::collections::BTreeMap;
use std::future::Future;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopCounterRegistry {
    thresholds: BTreeMap<String, u32>,
}

impl LoopCounterRegistry {
    pub fn phase1() -> Self {
        Self {
            thresholds: BTreeMap::from([
                ("rework_counter".to_string(), 3),
                ("integration_failure_counter".to_string(), 2),
            ]),
        }
    }

    pub fn threshold(&self, name: &str) -> Option<u32> {
        self.thresholds.get(name).copied()
    }
}

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
