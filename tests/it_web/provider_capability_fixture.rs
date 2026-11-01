//! it_web 侧「已确认能力」播种助手。
//!
//! 生产语义（`4749a1fc` 起）：bootstrap 行保持 `Unknown`，只有真实 probe 才能
//! 写入 `Confirmed`（`Unknown + supported_actions` 迁移桥已退役）。因此正向
//! fixture 若要让会话经 gateway 启动，必须在 `ensure_bootstrap` 之后显式物化
//! 「已探测」事实，否则 launch 分格 fail-closed 为
//! `provider_capability_launch_not_confirmed`。
//!
//! 记录形状与 lib target 内的
//! `crate::web::handlers::automation_enrollment_test_support::confirmed_capability_record`
//! 一致（exact version + dialect + 逐 action 三态证据 + host-open 语义标识 +
//! snapshot ref 与 gateway `ProviderRef` 对齐）；该 helper 是 `#[cfg(test)]`
//! 的 lib 私有项，it_web（独立 target）取不到，故在此复刻。
//!
//! 负向场景（断言「能力未确认」被拒）**不得**调用本助手，保留 bootstrap
//! `Unknown` 行即可——本助手只用于正向路径，不放宽产品 fail-closed 面。

use cadence_aria::cross_cutting::provider_boundary::PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN;
use cadence_aria::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::policy::{
    ProviderDialect, ProviderWireDialect, SessionPolicyAction,
};
use cadence_aria::product::logical_codebase::provider_capability_store::{
    CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
    ProviderActionMatrix, ProviderCapabilityRecord, ProviderCapabilityStore, RootRecipeEvidence,
};
use cadence_aria::product::logical_codebase::provider_gateway::{
    ProviderRefType, ResumeEvidenceState,
};

/// 三个 canonical action 行全 `Confirmed` 的 capability 记录（trust 亦
/// `Confirmed`，执行语义 host-open，snapshot ref 与 gateway
/// `claude_code("cap_managed_snapshot")` 对齐）。
fn confirmed_capability_record(provider_type: ProviderRefType) -> ProviderCapabilityRecord {
    let (adapter_dialect, wire_dialect) = match provider_type {
        ProviderRefType::ClaudeCode => (
            ProviderDialect::ClaudeCodeCliV1,
            ProviderWireDialect::ClaudeCodeStreamJson,
        ),
        ProviderRefType::Codex => (
            ProviderDialect::CodexCliV1,
            ProviderWireDialect::CodexAppServerRpc,
        ),
        ProviderRefType::Pi => (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc),
        ProviderRefType::KimiCode => (ProviderDialect::KimiAcpV1, ProviderWireDialect::KimiAcp),
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
        resume_evidence: ResumeEvidenceState::Unsupported,
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
        root_recipe_evidence: RootRecipeEvidence::None,
        execution_semantics: PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN.to_string(),
    }
}

/// 先物化 bootstrap 行（保持生产「未探测」语义），再把指定 provider 覆盖写为
/// `Confirmed`。`lc_id = None` 写 project 级 legacy 子树，`Some` 写 LC 子树。
pub(crate) fn seed_confirmed_capability(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: Option<&str>,
    providers: &[ProviderRefType],
) {
    let store = match lc_id {
        Some(lc_id) => ProviderCapabilityStore::for_lc(paths.clone(), lc_id),
        None => ProviderCapabilityStore::new(paths.clone()),
    };
    store
        .ensure_bootstrap(project_id)
        .expect("bootstrap capability records");
    for provider_type in providers {
        store
            .upsert(project_id, &confirmed_capability_record(*provider_type))
            .expect("seed confirmed capability record");
    }
}

/// 常用形态：仅把 ClaudeCode（gateway 多仓/逻辑路径的默认 provider）覆盖写为
/// `Confirmed`。
pub(crate) fn seed_confirmed_claude(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: Option<&str>,
) {
    seed_confirmed_capability(paths, project_id, lc_id, &[ProviderRefType::ClaudeCode]);
}
