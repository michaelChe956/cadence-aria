//! 逻辑代码库生产入口测试二进制侧的「已确认能力」播种助手。
//!
//! 生产语义：bootstrap 行保持 `Unknown`，只有真实 probe 才能写 `Confirmed`。
//! 聚合初始化的**创建期 provider 选择门**（判定前移，`ff473d9` 起）要求被选
//! provider 的能力已 `Confirmed`，否则创建期即 500
//! `aggregate_initialization_provider_needs_verification`（不创建 operation）。
//! ⇒ 正向夹具必须显式物化「已探测」事实（与 `it_web` 侧
//! `provider_capability_fixture`、lib 侧
//! `automation_enrollment_test_support::seed_confirmed_capability_records` 同款形状）。
//!
//! 记录形状复刻 `it_web` 侧那份（同一 crate 外取不到 lib 的 `#[cfg(test)]`
//! 私有助手）：exact version + dialect + 逐 action 三态证据 + host-open 执行
//! 语义标识 + snapshot ref 与 gateway `ProviderRef` 对齐。
//!
//! capability 记录的**唯一作用域 = 项目级全局**（owner 2026-11-04 裁定：能力
//! 描述的是「本机上某家 provider CLI 能不能跑这个 action」，与逻辑代码库的代码
//! 内容无关 ⇒ 一次核验对项目内全部逻辑代码库同时生效）。

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

/// 先物化 bootstrap 行（保持生产「未探测」语义），再把四家 provider 覆盖写为
/// `Confirmed`（创建期选择门按 owner 顺序取第一家可用者）。
pub(crate) fn seed_confirmed_capabilities(paths: &ProductAppPaths, project_id: &str) {
    let store = ProviderCapabilityStore::new(paths.clone());
    store
        .ensure_bootstrap(project_id)
        .expect("bootstrap capability records");
    for provider_type in [
        ProviderRefType::Pi,
        ProviderRefType::KimiCode,
        ProviderRefType::ClaudeCode,
        ProviderRefType::Codex,
    ] {
        store
            .upsert(project_id, &confirmed_capability_record(provider_type))
            .expect("seed confirmed capability record");
    }
}

/// 只把 **ClaudeCode** 种为 `Confirmed`（bootstrap 行保持其余三家
/// `Unknown`）。生产接线的聚合初始化走 root-recipe 相位时**只接受固定的
/// Claude recipe provider**（`root_recipe_requires_fixed_claude_provider`，
/// 既有契约）——owner 候选序为 Pi → KimiCode → ClaudeCode → Codex，四家
/// 全 Confirmed 会让选择器选中 Pi 并在 recipe 相位被拒。正形 = 只种
/// ClaudeCode，选择器跳过未核验三家选中它（与 src 侧
/// `l2_global_capability.inc.rs::seed_claude_code_confirmed_globally` 同构）。
pub(crate) fn seed_confirmed_claude_capability(paths: &ProductAppPaths, project_id: &str) {
    let store = ProviderCapabilityStore::new(paths.clone());
    store
        .ensure_bootstrap(project_id)
        .expect("bootstrap capability records");
    store
        .upsert(
            project_id,
            &confirmed_capability_record(ProviderRefType::ClaudeCode),
        )
        .expect("seed confirmed ClaudeCode capability record");
}
