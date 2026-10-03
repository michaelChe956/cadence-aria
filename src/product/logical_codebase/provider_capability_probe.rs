//! Provider capability probe/evidence 的 shape validator(Task 2c,REQ-LCG-01)。
//!
//! 冻结接口(计划 Task 2 Interfaces):`ProviderCapabilityProbeService::
//! validate_probe_shape(record, evidence, projection)` 对 2a 的 v2
//! `ProviderCapabilityRecord`、1a 纯 DTO `ProviderBoundaryEvidence` 与
//! `ProviderPolicyProjection` 做**无副作用的三方可比对性校验**:
//!
//! - 字段可比对:`evidence.exact_version == projection.exact_version ==
//!   record.version` 等,provider 身份(type/adapter/wire dialect)三方一致;
//! - digest 形状:`projection_digest` 为 `sha256:` + 64 位小写十六进制;
//! - action/profile 类型:projection action 在 record 矩阵中有对应行,
//!   evidence boundary 模式与 action 读写语义一致;
//! - evidence 引用一致:`record.probe_artifact_ref ↔ evidence.artifact_ref`
//!   (含 action 行 `evidence_ref` 与 probe 元数据 `probed_at`)。
//!
//! 职责边界(§0 E2/E3/E4/E5):本服务**不写 durable**(那是 2d 的
//! `record_verified_probe`,后置 6c)、**不执行真实 probe**(那是 6c 的
//! `ProviderBoundaryProbe::run`);任一维度不可比对即 fail-closed 返回稳定
//! 错误,调用方(2d 导入、4/5/6a 消费 shape)不得把校验失败解释为能力支持。

use crate::cross_cutting::provider_boundary::{ProviderBoundaryEvidence, ProviderBoundaryMode};
use crate::product::logical_codebase::policy::SessionPolicyAction;
use crate::product::logical_codebase::provider_capability_store::{
    PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderCapabilityRecord,
};
use crate::product::logical_codebase::provider_gateway::ProviderRef;
use crate::product::logical_codebase::provider_projection::ProviderPolicyProjection;

/// shape 校验失败(稳定判别码 + 上下文;fail-closed)。
///
/// 变体集合即 2c 的稳定错误面:2d 导入、4/5/6a 消费方按变体/判别码诊断,
/// 不得把任何失败静默当作「通过」或「支持」。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderCapabilityProbeError {
    /// record 自身形状非法(如 schema 版本不是当前 v2)。
    #[error("provider_probe_shape_record_invalid: {0}")]
    RecordShapeInvalid(String),
    /// 三方 provider 身份不可比对(provider_type/adapter/wire dialect 不一致,
    /// 或 evidence 的 provider 无法映射为真实 provider ref,如 Fake)。
    #[error("provider_probe_shape_provider_mismatch: {0}")]
    ProviderMismatch(String),
    /// 三方 exact version 不一致(含空 version;CLI 版本漂移后旧证据不可导入)。
    #[error("provider_probe_shape_version_mismatch: {0}")]
    VersionMismatch(String),
    /// `projection_digest` 形态非法(期望 `sha256:` + 64 位小写十六进制)。
    #[error("provider_probe_shape_digest_invalid: {0}")]
    DigestInvalid(String),
    /// 三方 `projection_digest` 不一致(record action 行/evidence/projection)。
    #[error("provider_probe_shape_digest_mismatch: {0}")]
    DigestMismatch(String),
    /// action 行缺失、或 evidence boundary 模式与 action 读写语义不一致。
    #[error("provider_probe_shape_action_mismatch: {0}")]
    ActionMismatch(String),
    /// 探测证据引用不一致(`record.probe_artifact_ref`/action 行 `evidence_ref`
    /// 与 `evidence.artifact_ref` 不一致或为空)。
    #[error("provider_probe_shape_evidence_ref_mismatch: {0}")]
    EvidenceRefMismatch(String),
    /// 探测元数据不一致(`record.probed_at` 与 `evidence.probed_at` 不符或为空)。
    #[error("provider_probe_shape_probe_metadata_mismatch: {0}")]
    ProbeMetadataMismatch(String),
}

/// capability probe/evidence shape 校验服务(Task 2c 冻结接口)。
///
/// 无状态、无副作用:不持有 durable store(2d 后续在同服务上追加
/// `record_verified_probe` 时注入),不执行 probe。任何输入组合都只做
/// 三方逐字段比对并返回稳定错误。
#[derive(Debug, Clone, Copy, Default)]
pub struct ProviderCapabilityProbeService;

impl ProviderCapabilityProbeService {
    pub fn new() -> Self {
        Self
    }

    /// 冻结签名:只验证三方可比对性(字段可比对/digest 形状/action 类型/
    /// evidence 引用),不写 durable(2d)、不执行 probe(6c);不一致即
    /// fail-closed。
    pub fn validate_probe_shape(
        &self,
        _record: &ProviderCapabilityRecord,
        _evidence: &ProviderBoundaryEvidence,
        _projection: &ProviderPolicyProjection,
    ) -> Result<(), ProviderCapabilityProbeError> {
        // TODO(task-2c 阶段 2):实现真实三方形状比对;当前恒拒(fail-closed
        // 桩),保证阶段 1 红测试可编译且必然失败。
        Err(ProviderCapabilityProbeError::RecordShapeInvalid(
            "task-2c 阶段 1 编译桩:形状校验未实现".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
    use crate::product::logical_codebase::policy::{
        PolicyTarget, ProviderDialect, ProviderWireDialect,
    };
    use crate::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, ProviderActionCapability, ProviderActionMatrix, RootRecipeEvidence,
    };
    use crate::product::logical_codebase::provider_gateway::{
        ProviderRefType, ResumeEvidenceState,
    };
    use crate::product::models::ProviderName;
    use crate::protocol::contracts::AdapterRole;

    const PROBE_VERSION: &str = "1.42.0";
    const PROBE_ARTIFACT_REF: &str = "probe://boundary/kimi-code/0001";
    const PROBE_PROBED_AT: &str = "2026-10-03T08:00:00Z";

    /// 由 seed 生成形态合法且互不相同的 digest(`sha256:` + 64 位小写十六进制)。
    fn digest64(seed: u8) -> String {
        let hex: String = (0..64)
            .map(|i| char::from_digit((u32::from(seed) + i) % 16, 16).expect("hex digit"))
            .collect();
        format!("sha256:{hex}")
    }

    fn projection_with(
        provider_type: ProviderRefType,
        adapter_dialect: ProviderDialect,
        wire_dialect: ProviderWireDialect,
        action: SessionPolicyAction,
        digest: &str,
    ) -> ProviderPolicyProjection {
        ProviderPolicyProjection::new(
            provider_type,
            adapter_dialect,
            wire_dialect,
            PROBE_VERSION.to_string(),
            action,
            AdapterRole::Executor,
            ProviderPermissionMode::Auto,
            None,
            "never".to_string(),
            "client-service".to_string(),
            PathBuf::from("/lc-root"),
            PathBuf::from("/work/api/.worktrees/issue_1"),
            PolicyTarget::checkout("logical_repo", "checkout_1", "/work/api/.worktrees/issue_1"),
            vec![PathBuf::from("/aggregate")],
            vec![PathBuf::from("/work/api/.worktrees/issue_1")],
            "sha256:trust".to_string(),
            "sha256:config".to_string(),
            "sha256:mcp".to_string(),
            "probe://boundary/plan/1".to_string(),
            "sha256:capability-profile".to_string(),
            digest.to_string(),
        )
    }

    fn evidence_with(
        provider: ProviderName,
        exact_version: &str,
        boundary_mode: ProviderBoundaryMode,
        digest: &str,
    ) -> ProviderBoundaryEvidence {
        ProviderBoundaryEvidence::new(
            provider,
            exact_version.to_string(),
            boundary_mode,
            digest.to_string(),
            PROBE_ARTIFACT_REF.to_string(),
            PROBE_PROBED_AT.to_string(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn record_with_row(
        provider_type: ProviderRefType,
        adapter_dialect: ProviderDialect,
        wire_dialect: ProviderWireDialect,
        action: SessionPolicyAction,
        row_digest: &str,
        row_evidence_ref: &str,
    ) -> ProviderCapabilityRecord {
        let row = ProviderActionCapability {
            action,
            launch: ProviderCapabilityEvidence::Confirmed,
            resume: ProviderCapabilityEvidence::Unknown,
            write_boundary: ProviderCapabilityEvidence::Confirmed,
            projection_digest: row_digest.to_string(),
            evidence_ref: row_evidence_ref.to_string(),
        };
        ProviderCapabilityRecord {
            provider_type,
            schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
            version: PROBE_VERSION.to_string(),
            adapter_dialect,
            wire_dialect,
            capability_snapshot_ref: "cap_managed_snapshot".to_string(),
            evidence: CapabilityEvidence::FixtureVerified,
            resume_evidence: ResumeEvidenceState::Confirmed,
            supported_actions: Vec::new(),
            action_matrix: ProviderActionMatrix::from_rows(vec![row]).expect("唯一 action 行"),
            trust: ProviderCapabilityEvidence::Unknown,
            probed_at: Some(PROBE_PROBED_AT.to_string()),
            probe_artifact_ref: Some(PROBE_ARTIFACT_REF.to_string()),
            root_recipe_evidence: RootRecipeEvidence::None,
        }
    }

    /// 三方一致的 KimiCode 三元组(action 与边界模式由调用方显式指定)。
    fn consistent_triple(
        action: SessionPolicyAction,
        mode: ProviderBoundaryMode,
    ) -> (
        ProviderCapabilityRecord,
        ProviderBoundaryEvidence,
        ProviderPolicyProjection,
    ) {
        let digest = digest64(7);
        (
            record_with_row(
                ProviderRefType::KimiCode,
                ProviderDialect::KimiAcpV1,
                ProviderWireDialect::KimiAcp,
                action,
                &digest,
                PROBE_ARTIFACT_REF,
            ),
            evidence_with(ProviderName::KimiCode, PROBE_VERSION, mode, &digest),
            projection_with(
                ProviderRefType::KimiCode,
                ProviderDialect::KimiAcpV1,
                ProviderWireDialect::KimiAcp,
                action,
                &digest,
            ),
        )
    }

    fn service() -> ProviderCapabilityProbeService {
        ProviderCapabilityProbeService::new()
    }

    /// 断言 fail-closed 拒绝且稳定判别码(每变体的 Display 前缀唯一,钉住变体)。
    fn expect_mismatch(result: Result<(), ProviderCapabilityProbeError>, code: &str) {
        let err = result.expect_err("shape 校验必须 fail-closed 拒绝");
        assert!(
            err.to_string().starts_with(code),
            "期望稳定判别码 {code},实际 {}",
            err
        );
    }

    /// 一致三元组逐 action × 边界模式通过 shape 校验:钉住
    /// version/digest/action 行/evidence 引用四类可比对维度的合法面。
    #[test]
    fn lcg_t02_probe_shape_validates_version_digest_action_references() {
        for (action, mode) in [
            (
                SessionPolicyAction::PlanningReadOnly,
                ProviderBoundaryMode::ReadOnly,
            ),
            (
                SessionPolicyAction::CodingTargetWrite,
                ProviderBoundaryMode::TargetWriteOnly,
            ),
            (
                SessionPolicyAction::ReviewReadOnly,
                ProviderBoundaryMode::ReadOnly,
            ),
        ] {
            let (record, evidence, projection) = consistent_triple(action, mode);
            service()
                .validate_probe_shape(&record, &evidence, &projection)
                .unwrap_or_else(|err| panic!("{action:?} 一致三元组应通过: {err}"));
        }
    }

    /// 逐维漂移(version/digest/evidence 引用)fail-closed 且判别码稳定。
    #[test]
    fn lcg_t02_probe_shape_mismatch_rejects_with_stable_error() {
        let action = SessionPolicyAction::CodingTargetWrite;
        let mode = ProviderBoundaryMode::TargetWriteOnly;

        // record.version 漂移:CLI 版本漂移后旧证据不可导入。
        let (mut record, evidence, projection) = consistent_triple(action, mode);
        record.version = "9.9.9".to_string();
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_version_mismatch",
        );

        // evidence.exact_version 与 record/projection 漂移。
        let (record, _, projection) = consistent_triple(action, mode);
        let evidence = evidence_with(ProviderName::KimiCode, "1.43.0", mode, &digest64(7));
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_version_mismatch",
        );

        // evidence.projection_digest 漂移(形态合法但值不同)。
        let evidence = evidence_with(ProviderName::KimiCode, PROBE_VERSION, mode, &digest64(9));
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_digest_mismatch",
        );

        // record action 行 projection_digest 漂移。
        let record = record_with_row(
            ProviderRefType::KimiCode,
            ProviderDialect::KimiAcpV1,
            ProviderWireDialect::KimiAcp,
            action,
            &digest64(11),
            PROBE_ARTIFACT_REF,
        );
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_digest_mismatch",
        );

        // record.probe_artifact_ref 缺失。
        let (mut record, evidence, projection) = consistent_triple(action, mode);
        record.probe_artifact_ref = None;
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_evidence_ref_mismatch",
        );

        // record.probe_artifact_ref 漂移。
        let (mut record, evidence, projection) = consistent_triple(action, mode);
        record.probe_artifact_ref = Some("probe://other/0002".to_string());
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_evidence_ref_mismatch",
        );

        // 行级 evidence_ref 漂移(未指向同一 probe 工件)。
        let record = record_with_row(
            ProviderRefType::KimiCode,
            ProviderDialect::KimiAcpV1,
            ProviderWireDialect::KimiAcp,
            action,
            &digest64(7),
            "probe://other/0002",
        );
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_evidence_ref_mismatch",
        );
    }

    /// digest 形态非法(缺前缀/长度错/大写 hex/空)拒绝。
    #[test]
    fn lcg_t02_probe_shape_rejects_invalid_digest_shape() {
        let action = SessionPolicyAction::CodingTargetWrite;
        let mode = ProviderBoundaryMode::TargetWriteOnly;

        // projection digest 形态非法。
        let bad_digests = [
            "deadbeef".to_string(),
            "sha256:short".to_string(),
            format!("sha256:{}", "A".repeat(64)),
        ];
        for bad in &bad_digests {
            let digest = digest64(7);
            let record = record_with_row(
                ProviderRefType::KimiCode,
                ProviderDialect::KimiAcpV1,
                ProviderWireDialect::KimiAcp,
                action,
                &digest,
                PROBE_ARTIFACT_REF,
            );
            let evidence = evidence_with(ProviderName::KimiCode, PROBE_VERSION, mode, &digest);
            let projection = projection_with(
                ProviderRefType::KimiCode,
                ProviderDialect::KimiAcpV1,
                ProviderWireDialect::KimiAcp,
                action,
                bad,
            );
            expect_mismatch(
                service().validate_probe_shape(&record, &evidence, &projection),
                "provider_probe_shape_digest_invalid",
            );
        }

        // evidence digest 为空。
        let (record, _, projection) = consistent_triple(action, mode);
        let evidence = evidence_with(ProviderName::KimiCode, PROBE_VERSION, mode, "");
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_digest_invalid",
        );

        // 行 digest 缺前缀。
        let record = record_with_row(
            ProviderRefType::KimiCode,
            ProviderDialect::KimiAcpV1,
            ProviderWireDialect::KimiAcp,
            action,
            "0123abcd",
            PROBE_ARTIFACT_REF,
        );
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_digest_invalid",
        );
    }

    /// provider 身份/action 类型/probe 元数据漂移拒绝(schema/Fake/
    /// dialect 漂移/缺 action 行/边界模式不符/probed_at 不符)。
    #[test]
    fn lcg_t02_probe_shape_rejects_provider_action_and_metadata_mismatches() {
        let action = SessionPolicyAction::CodingTargetWrite;
        let mode = ProviderBoundaryMode::TargetWriteOnly;

        // schema 版本非当前 v2。
        let (mut record, evidence, projection) = consistent_triple(action, mode);
        record.schema_version = 1;
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_record_invalid",
        );

        // evidence provider 为 Fake:无法映射真实 provider ref,fail-closed。
        let (record, _, projection) = consistent_triple(action, mode);
        let evidence = evidence_with(ProviderName::Fake, PROBE_VERSION, mode, &digest64(7));
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_provider_mismatch",
        );

        // record.provider_type 与 evidence/projection 漂移。
        let record = record_with_row(
            ProviderRefType::ClaudeCode,
            ProviderDialect::KimiAcpV1,
            ProviderWireDialect::KimiAcp,
            action,
            &digest64(7),
            PROBE_ARTIFACT_REF,
        );
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_provider_mismatch",
        );

        // adapter dialect 漂移。
        let record = record_with_row(
            ProviderRefType::KimiCode,
            ProviderDialect::ClaudeCodeCliV1,
            ProviderWireDialect::KimiAcp,
            action,
            &digest64(7),
            PROBE_ARTIFACT_REF,
        );
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_provider_mismatch",
        );

        // wire dialect 漂移。
        let record = record_with_row(
            ProviderRefType::KimiCode,
            ProviderDialect::KimiAcpV1,
            ProviderWireDialect::CodexAppServerRpc,
            action,
            &digest64(7),
            PROBE_ARTIFACT_REF,
        );
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_provider_mismatch",
        );

        // projection.provider_type 漂移(与 record/evidence 不一致)。
        let (record, evidence, _) = consistent_triple(action, mode);
        let projection = projection_with(
            ProviderRefType::Codex,
            ProviderDialect::KimiAcpV1,
            ProviderWireDialect::KimiAcp,
            action,
            &digest64(7),
        );
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_provider_mismatch",
        );

        // 矩阵缺 projection action 行(未探测 action 不可导入)。
        let (record, evidence, _) = consistent_triple(action, mode);
        let projection = projection_with(
            ProviderRefType::KimiCode,
            ProviderDialect::KimiAcpV1,
            ProviderWireDialect::KimiAcp,
            SessionPolicyAction::PlanningReadOnly,
            &digest64(7),
        );
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_action_mismatch",
        );

        // boundary 模式与 action 读写语义不一致(Coding 却持只读证据)。
        let (record, _, projection) = consistent_triple(action, mode);
        let evidence = evidence_with(
            ProviderName::KimiCode,
            PROBE_VERSION,
            ProviderBoundaryMode::ReadOnly,
            &digest64(7),
        );
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_action_mismatch",
        );

        // probed_at 缺失/漂移:record 未描述同一探测。
        let (mut record, evidence, projection) = consistent_triple(action, mode);
        record.probed_at = None;
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_probe_metadata_mismatch",
        );
        record.probed_at = Some("2020-01-01T00:00:00Z".to_string());
        expect_mismatch(
            service().validate_probe_shape(&record, &evidence, &projection),
            "provider_probe_shape_probe_metadata_mismatch",
        );
    }
}
