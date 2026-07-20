use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::recovery::fixture_error;
use super::seed::core_v2_contract;
use super::{PlanRepairFixtureError, PlanRepairProviderMatrixResult};
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderCompletion, ProviderEvent, ProviderSession, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::cross_cutting::structured_output::StructuredOutputContract;
use crate::product::models::{
    PlanDefectClass, PlanDefectEvidence, PlanDefectRoute, ProviderName, RepairTarget,
    RepairTargetKind,
};
use crate::product::plan_repair::{PlanDefectConfidence, PlanDefectFinding, PlanDefectSeverity};
use crate::protocol::contracts::AdapterRole;

mod coding;
mod workspace;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_plan_0001";

pub(super) async fn run_provider_matrix(
    root: &Path,
    provider: ProviderName,
) -> Result<PlanRepairProviderMatrixResult, PlanRepairFixtureError> {
    let mut author_contract = core_v2_contract();
    author_contract.identity.kind = "backend".to_string();
    author_contract.write_policy.exclusive_scopes = vec!["src/core.rs".to_string()];

    let defect = upstream_contract_defect("provider_matrix_finding_0001");
    let author_output = serde_json::json!({
        "draft": {
            "outline_id": "outline_core",
            "logical_work_item_id": "wi_core",
            "canonical_contract": author_contract,
            "verification_plan": {
                "checks": author_contract.verification_checks,
            },
        },
    })
    .to_string();
    let plan_review_output = serde_json::json!({
        "verdict": "pass",
        "review_scope": "item",
        "target_outline_id": "outline_core",
        "generation_round_id": "provider_matrix_round_0001",
        "draft_id": "draft_001",
        "summary": "canonical contract and projections are valid",
        "findings": [],
        "affects_items": [{ "target_outline_id": "outline_core" }],
    })
    .to_string();
    let coder_output = serde_json::json!({
        "plan_defect_findings": [defect.clone()],
    })
    .to_string();
    let code_review_output = code_review_output(&defect);
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let adapter = Arc::new(MatrixStreamingProvider::new(
        provider.clone(),
        [
            author_output,
            plan_review_output,
            coder_output,
            code_review_output,
        ],
        prompts.clone(),
    ));
    let workspace = workspace::run_workspace_provider_roles(
        root,
        provider.clone(),
        adapter.clone(),
        &author_contract,
    )
    .await?;
    let coding = coding::run_coding_provider_roles(root, provider.clone(), adapter.clone()).await?;

    let prompt_text = prompts.lock().map_err(fixture_error)?.join("\n");
    let rendered_contract_ids_preserved = prompt_text.contains("contract.workflow")
        && prompt_text.contains("finalization_failure")
        && prompt_text.contains("workflow_explicit_completion");
    Ok(PlanRepairProviderMatrixResult {
        provider,
        rendered_contract_ids_preserved,
        author_contract_ids: workspace.author_contract_ids,
        plan_review_passed: workspace.plan_review_passed,
        coder_defect_class: coding.coder_defect_class,
        code_review_defect_class: coding.code_review_defect_class,
        code_review_route: coding.code_review_route,
        author_draft_artifact_persisted: workspace.author_draft_artifact_persisted,
        plan_review_complete_event_observed: workspace.plan_review_complete_event_observed,
        coding_role_run_count: coding.role_run_count,
        coding_raw_output_ref_count: coding.raw_output_ref_count,
    })
}

struct MatrixStreamingProvider {
    provider: ProviderName,
    outputs: Mutex<VecDeque<String>>,
    prompts: Arc<Mutex<Vec<String>>>,
}

impl MatrixStreamingProvider {
    fn new<const N: usize>(
        provider: ProviderName,
        outputs: [String; N],
        prompts: Arc<Mutex<Vec<String>>>,
    ) -> Self {
        Self {
            provider,
            outputs: Mutex::new(outputs.into_iter().collect()),
            prompts,
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for MatrixStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let work_item_split_output = input.role == AdapterRole::WorkItemSplitter;
        let structured_output_contract = input.structured_output_contract.clone();
        self.prompts
            .lock()
            .map_err(|error| matrix_provider_error(error.to_string()))?
            .push(input.prompt);
        let output = self
            .outputs
            .lock()
            .map_err(|error| matrix_provider_error(error.to_string()))?
            .pop_front()
            .ok_or_else(|| matrix_provider_error("scripted provider output exhausted"))?;
        let provider_session_id = format!("provider_matrix_{:?}", self.provider).to_lowercase();
        let completion = matrix_provider_completion(
            output,
            structured_output_contract.as_ref(),
            work_item_split_output,
            Some(provider_session_id),
        );
        let streamed_output = completion.full_output.clone();
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: streamed_output,
                })
                .await;
            let _ = event_tx.send(ProviderEvent::Completed(completion)).await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

fn matrix_provider_completion(
    output: String,
    contract: Option<&StructuredOutputContract>,
    work_item_split_output: bool,
    provider_session_id: Option<String>,
) -> ProviderCompletion {
    let Some(contract) = contract else {
        if work_item_split_output {
            let full_output = format!(
                "<ARIA_STRUCTURED_OUTPUT nonce=\"MATRIX01\">{output}</ARIA_STRUCTURED_OUTPUT nonce=\"MATRIX01\">"
            );
            return ProviderCompletion::plain(full_output, provider_session_id);
        }
        return ProviderCompletion::plain(output, provider_session_id);
    };
    let full_output = format!(
        "<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">{}</ARIA_STRUCTURED_OUTPUT nonce=\"{}\">",
        contract.nonce, output, contract.nonce
    );
    ProviderCompletion::from_output(full_output, Some(contract), provider_session_id)
}

fn upstream_contract_defect(finding_id: &str) -> PlanDefectFinding {
    PlanDefectFinding {
        finding_id: finding_id.to_string(),
        severity: PlanDefectSeverity::Error,
        defect_class: PlanDefectClass::UpstreamContractInvalid,
        reason_code: "upstream_contract_invalid".to_string(),
        message: "registration cannot observe repository finalization failure".to_string(),
        evidence: vec![PlanDefectEvidence {
            kind: "review_finding".to_string(),
            source_ref: "provider_matrix#finding_0001".to_string(),
            message: "finalization_failure capability is missing".to_string(),
        }],
        contract_refs: vec!["contract.workflow".to_string()],
        capability_refs: vec!["finalization_failure".to_string()],
        repair_target: Some(RepairTarget {
            kind: RepairTargetKind::UpstreamWorkItem,
            logical_work_item_ids: vec!["wi_core".to_string()],
            work_item_revision_ids: vec!["work_item_revision_wi_core_0001".to_string()],
        }),
        recommended_route: PlanDefectRoute::PlanRepair,
        confidence: PlanDefectConfidence::High,
    }
}

fn code_review_output(defect: &PlanDefectFinding) -> String {
    serde_json::json!({
        "verdict": "request_changes",
        "summary": "upstream contract must be repaired",
        "findings": [{
            "severity": "error",
            "message": defect.message,
            "required_action": "repair the upstream finalization contract",
            "source_stage": "code_review",
            "evidence": defect.evidence,
            "defect_class": defect.defect_class,
            "reason_code": defect.reason_code,
            "contract_refs": defect.contract_refs,
            "capability_refs": defect.capability_refs,
            "repair_target": defect.repair_target,
            "recommended_route": defect.recommended_route,
            "confidence": defect.confidence,
        }],
        "impact_scope": ["wi_core", "wi_registration"],
        "tested_evidence_refs": ["test_provider_matrix"],
        "diff_refs": ["diff_provider_matrix"],
    })
    .to_string()
}

fn matrix_provider_error(message: impl Into<String>) -> ProviderAdapterError {
    ProviderAdapterError::execution_failed(None, String::new(), message.into(), 0)
}
