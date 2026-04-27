use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::provider_run::provider_run_record_from_output;
use crate::protocol::contracts::{
    AdapterInput, AdapterOutput, ApprovalPolicy, ProviderRunRecord, RuntimeRole, SandboxMode,
};
use crate::protocol::enums::{
    AdapterCompatibilityId, AdapterInputRefId, AdapterOutputRefId, ConstraintCheckId,
    ContextPackageId, NodeId, ProviderCapabilityId, ProviderRunId, TraceabilityBindingId,
};

pub struct ProviderRouter {
    adapter: Box<dyn ProviderAdapter>,
}

impl ProviderRouter {
    pub fn new(adapter: Box<dyn ProviderAdapter>) -> Self {
        Self { adapter }
    }

    pub fn run(
        &self,
        request: ProviderRunRequest,
        input: AdapterInput,
    ) -> Result<(AdapterOutput, ProviderRunRecord), ProviderAdapterError> {
        let output = self.adapter.run(&input)?;
        let record = provider_run_record_from_output(&request, &input, &output);
        Ok((output, record))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRunRequest {
    pub provider_run_id: ProviderRunId,
    pub node_id: NodeId,
    pub runtime_role: RuntimeRole,
    pub provider_capability_ref: ProviderCapabilityId,
    pub adapter_compatibility_ref: AdapterCompatibilityId,
    pub context_package_ref: ContextPackageId,
    pub adapter_input_ref: AdapterInputRefId,
    pub adapter_output_ref: AdapterOutputRefId,
    pub approval_policy: ApprovalPolicy,
    pub sandbox_mode: SandboxMode,
    pub constraint_check_ref: Option<ConstraintCheckId>,
    pub traceability_binding_refs: Vec<TraceabilityBindingId>,
}
