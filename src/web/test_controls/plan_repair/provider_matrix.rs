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
    let gateway = matrix_provider_gateway(root, provider.clone(), adapter.clone())?;
    let coding =
        coding::run_coding_provider_roles(root, provider.clone(), adapter.clone(), gateway).await?;

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
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    /// gateway 的 validated 启动入口：逻辑 target 的 provider 必须经
    /// `LogicalCodebaseProviderGateway` 的 prepared launch（trait 默认实现恒拒：
    /// `cross_cutting/streaming_provider/mod.rs` 的
    /// "streaming provider validated start is not supported by this adapter"）。
    /// 本替身与 `start` 同源（同一脚本化产出队列）。
    async fn start_validated(
        &self,
        input: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, _launch) = input.into_parts();
        self.start(input, cancel).await
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
            let mut value: serde_json::Value =
                serde_json::from_str(&output).expect("matrix work-item output must be JSON");
            value
                .as_object_mut()
                .expect("matrix work-item output must be an object")
                .insert("nonce".to_string(), serde_json::json!("MATRIX01"));
            let full_output = format!(
                "<ARIA_STRUCTURED_OUTPUT nonce=\"MATRIX01\">{value}</ARIA_STRUCTURED_OUTPUT>"
            );
            return ProviderCompletion::plain(full_output, provider_session_id);
        }
        return ProviderCompletion::plain(output, provider_session_id);
    };
    let mut value: serde_json::Value =
        serde_json::from_str(&output).expect("matrix structured output must be JSON");
    value
        .as_object_mut()
        .expect("matrix structured output must be an object")
        .insert("nonce".to_string(), serde_json::json!(contract.nonce));
    let full_output = format!(
        "<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">{value}</ARIA_STRUCTURED_OUTPUT>",
        contract.nonce
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

/// 同步 adapter 桩：gateway 工厂构造签名需要，本夹具只走 streaming 路径。
struct MatrixStubSyncAdapter;

impl crate::cross_cutting::provider_adapter::ProviderAdapter for MatrixStubSyncAdapter {
    fn run(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
    ) -> Result<
        crate::protocol::contracts::AdapterOutput,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        Err(
            crate::cross_cutting::provider_adapter::ProviderAdapterError::provider_unavailable(
                "plan repair provider matrix sync adapter is not expected to run",
            ),
        )
    }
}

/// always-available 健康源：gateway 在 spawn 前复验 provider 可用性，测试环境
/// 无真实 provider 二进制，直挂恒健康源（同 wiga_gate_fixture 先例）。
struct MatrixAlwaysHealthyProviderHealth;

impl crate::cross_cutting::provider_availability_gate::ProviderHealthSource
    for MatrixAlwaysHealthyProviderHealth
{
    fn snapshot(
        &self,
    ) -> std::sync::Arc<crate::cross_cutting::provider_health::ProviderHealthSnapshot> {
        let checked_at = chrono::Utc::now();
        let entry =
            |provider: ProviderName| crate::cross_cutting::provider_health::ProviderHealthEntry {
                provider,
                command: "matrix-health".to_string(),
                available: true,
                version: Some("matrix".to_string()),
                reason_code: None,
                reason: None,
                checked_at,
            };
        std::sync::Arc::new(
            crate::cross_cutting::provider_health::ProviderHealthSnapshot {
                schema_version: 1,
                generation: 1,
                checked_at,
                providers: vec![
                    entry(ProviderName::ClaudeCode),
                    entry(ProviderName::Codex),
                    entry(ProviderName::Pi),
                    entry(ProviderName::KimiCode),
                ],
            },
        )
    }

    fn degraded(&self) -> bool {
        false
    }
}

/// gateway 装配：逻辑 target（attempt 带冻结快照）的 provider 启动恒经
/// `LogicalCodebaseProviderGateway`（`coding_workspace_engine/provider_stream.rs`
/// 的 `logical_provider_gateway_required` fail-closed 门：无 gateway 时直接启动
/// provider 一律被拒）。装配三件套：
/// ① registry 槽位——gateway 从自己的 registry 取 adapter，矩阵 provider 按
///    provider 名注册（与断言捕获面同一实例）；
/// ② always-available gate（测试环境无真实 provider 二进制）；
/// ③ Confirmed capability 行——生产语义下 bootstrap 行恒 Unknown（4749a1fc），
///    正向路径必须物化「已探测」事实，否则 launch 分格 fail-closed 为
///    `provider_capability_launch_not_confirmed`。
/// 已确认能力记录（三条 canonical action 行全 Confirmed + trust Confirmed +
/// host-open 执行语义 + snapshot ref 与 gateway 的 `ProviderRef` 对齐）。
///
/// it_web 侧有同款复刻（`tests/it_web/provider_capability_fixture.rs`），但那是
/// 独立 target；lib 内的 `web::handlers::automation_enrollment_test_support` 是
/// `#[cfg(test)]` 私有项，取不到 ⇒ 此处复刻（**正向路径专用**：需要 Unknown 的
/// 负向场景不调用本播种，以免放宽产品 fail-closed 面）。
fn confirmed_capability_record(
    provider_type: crate::product::logical_codebase::provider_gateway::ProviderRefType,
) -> crate::product::logical_codebase::provider_capability_store::ProviderCapabilityRecord {
    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
    use crate::product::logical_codebase::policy::{
        ProviderDialect, ProviderWireDialect, SessionPolicyAction,
    };
    use crate::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
        ProviderActionMatrix, ProviderCapabilityRecord,
    };
    let (adapter_dialect, wire_dialect) = match provider_type {
        crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode => (
            ProviderDialect::ClaudeCodeCliV1,
            ProviderWireDialect::ClaudeCodeStreamJson,
        ),
        crate::product::logical_codebase::provider_gateway::ProviderRefType::Codex => (
            ProviderDialect::CodexCliV1,
            ProviderWireDialect::CodexAppServerRpc,
        ),
        crate::product::logical_codebase::provider_gateway::ProviderRefType::Pi => {
            (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc)
        }
        crate::product::logical_codebase::provider_gateway::ProviderRefType::KimiCode => {
            (ProviderDialect::KimiAcpV1, ProviderWireDialect::KimiAcp)
        }
    };
    let row = |action: SessionPolicyAction| ProviderActionCapability {
        action,
        launch: ProviderCapabilityEvidence::Confirmed,
        resume: ProviderCapabilityEvidence::Confirmed,
        write_boundary: ProviderCapabilityEvidence::Confirmed,
        projection_digest: format!("projection-digest-{action:?}"),
        evidence_ref: format!("probe://{action:?}"),
    };
    ProviderCapabilityRecord {
        provider_type,
        schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
        version: "1.4.0".to_string(),
        adapter_dialect,
        wire_dialect,
        capability_snapshot_ref: "cap_managed_snapshot".to_string(),
        evidence: CapabilityEvidence::ProductionVerified,
        resume_evidence:
            crate::product::logical_codebase::provider_gateway::ResumeEvidenceState::Unsupported,
        supported_actions: Vec::new(),
        action_matrix: ProviderActionMatrix::from_rows(vec![
            row(SessionPolicyAction::PlanningReadOnly),
            row(SessionPolicyAction::CodingTargetWrite),
            row(SessionPolicyAction::ReviewReadOnly),
        ])
        .expect("three canonical capability rows"),
        trust: ProviderCapabilityEvidence::Confirmed,
        probed_at: None,
        probe_artifact_ref: None,
        execution_semantics:
            crate::cross_cutting::provider_boundary::PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN
                .to_string(),
    }
}

/// 模块私有：唯一调用方是同文件 `run_provider_matrix`；签名携带本模块私有的
/// `MatrixStreamingProvider`，`pub(super)` 会触发 `private_interfaces`
/// （类型可达性窄于函数可达性）。
fn matrix_provider_gateway(
    root: &Path,
    provider: ProviderName,
    adapter: Arc<MatrixStreamingProvider>,
) -> Result<
    Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>,
    PlanRepairFixtureError,
> {
    let paths = super::seed::fixture_paths(root);
    let lc_id = crate::product::logical_codebase::store::legacy_logical_codebase_id(PROJECT_ID);
    let mut registry = crate::cross_cutting::provider_registry::ProviderRegistry::new();
    registry.register(
        provider.clone(),
        adapter as Arc<dyn crate::cross_cutting::streaming_provider::StreamingProviderAdapter>,
    );
    let registry = Arc::new(registry);
    let capability_store =
        crate::product::logical_codebase::ProviderCapabilityStore::new(paths.clone());
    capability_store.ensure_bootstrap().map_err(fixture_error)?;
    for provider_type in [
        crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode,
        crate::product::logical_codebase::provider_gateway::ProviderRefType::Codex,
    ] {
        capability_store
            .upsert(&confirmed_capability_record(provider_type))
            .map_err(fixture_error)?;
    }
    let factory = crate::web::gateway_factory::LogicalCodebaseGatewayFactory::new(
        paths,
        registry,
        Arc::new(MatrixStubSyncAdapter),
        Arc::new(
            crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate::with_host_readiness(
                Arc::new(MatrixAlwaysHealthyProviderHealth),
                Arc::new(
                    crate::cross_cutting::provider_availability_gate::AlwaysReadyProviderHost,
                ),
            ),
        ),
    );
    let gateway = factory
        .build_for_lc_material_prep(PROJECT_ID, Some(&lc_id))
        .map_err(fixture_error)?;
    Ok(Arc::new(gateway))
}
