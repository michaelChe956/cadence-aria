// 知识库写面(Task 4)的 provider 选择:由 `provider_selection.rs` 通过 `include!` 引入,
// 与既有读分格入口同处一个模块命名空间(私有项互通),实现与新增测试单独成文件以守 1200 行门禁。

/// 知识库写面（Task 4）的 provider 选择入口：与既有
/// [`select_aggregate_provider`] 完全相同的两层判据（L1 health、L2 launch
/// 分格经 `launch_cell_state`，与 gateway 同源），唯一差异是 L2 判定所用
/// action 从 `PlanningReadOnly` 换成 `CodingTargetWrite`——知识库 provider
/// turn 走 Normal 相位 + `CodingTargetWrite`（target = canonical 聚合根），
/// 让「选择期通过」与「起跑被 gateway 放行」落在同一行能力矩阵。
///
/// 候选顺序与四枚失败码与既有入口零差异；**不创建 operation**。
pub async fn select_aggregate_provider_for_write(
    paths: &ProductAppPaths,
    health: &ProviderHealthService,
    _project_id: &str,
) -> Result<ProviderSelectionOutcome, ProductStoreError> {
    select_aggregate_provider_for_write_at(paths, health, _project_id, Utc::now()).await
}

/// 写分格选择器(显式给出「当前时刻」,与 [`select_aggregate_provider_at`]
/// 同理由:快照过期判定与 `decided_at` 必须同一时刻,测试要能固定这一刻)。
pub async fn select_aggregate_provider_for_write_at(
    paths: &ProductAppPaths,
    health: &ProviderHealthService,
    _project_id: &str,
    now: DateTime<Utc>,
) -> Result<ProviderSelectionOutcome, ProductStoreError> {
    let snapshot = health.snapshot();
    if health_snapshot_needs_refresh(snapshot.as_ref(), health.degraded(), now) {
        // 刷新用的 token 与请求生命周期解耦:请求结束不取消正在进行的探测。
        let token = CancellationToken::new();
        if let Err(error) = health.refresh(token).await {
            return Ok(ProviderSelectionOutcome::HealthDegraded {
                detail: error.to_string(),
            });
        }
    }
    let snapshot = health.snapshot();
    let source = StoreBackedProviderCapabilitySource::with_store(ProviderCapabilityStore::new(
        paths.clone(),
    ));
    decide_selection_for_write(&source, snapshot.as_ref(), now)
}

/// [`decide_selection`] 的写分格镜像:流程逐字同源,唯一差异是 L2 判定的
/// action 为 `CodingTargetWrite`(判定实现仍唯一经 `launch_cell_state`)。
/// 既有 `decide_selection` 及其调用方一字不改。
fn decide_selection_for_write(
    source: &StoreBackedProviderCapabilitySource,
    snapshot: &ProviderHealthSnapshot,
    now: DateTime<Utc>,
) -> Result<ProviderSelectionOutcome, ProductStoreError> {
    let mut candidates: Vec<AggregateProviderCandidate> = Vec::new();
    let mut selected: Option<ProviderName> = None;
    let mut needs_verification: Option<ProviderName> = None;
    let mut evidence_stale: Option<ProviderName> = None;

    for provider in AGGREGATE_PROVIDER_CANDIDATE_ORDER {
        let provider_wire = provider_wire_label(&provider).to_string();
        let entry = snapshot.entry(&provider);
        let cli_available = entry.is_some_and(|entry| entry.available);
        let cli_reason_code = entry
            .and_then(|entry| entry.reason_code)
            .map(health_reason_code);
        let display_name = provider_display_name(&provider);
        let (capability_state, skipped_reason, action_hint, warning) = if cli_available {
            let state =
                match ProviderRef::from_provider_name(&provider, MANAGED_CAPABILITY_SNAPSHOT_REF) {
                    Ok(provider_ref) => launch_cell_state(
                        source,
                        &provider_ref,
                        SessionPolicyAction::CodingTargetWrite,
                    )?,
                    // 候选表里没有 Fake:映射失败说明候选表被改坏,fail-closed 当未核验。
                    Err(error) => LaunchCellState::Unverified {
                        detail: error.to_string(),
                    },
                };
            match state {
                LaunchCellState::Confirmed => (
                    "confirmed".to_string(),
                    None,
                    format!("使用 {display_name} 执行本步(该候选已核验)"),
                    None,
                ),
                LaunchCellState::Unverified { .. } => (
                    "unverified".to_string(),
                    Some("needs_verification".to_string()),
                    format!(
                        "先核验 {display_name} 在本逻辑代码库的能力(一次真实探测),通过后重试本步"
                    ),
                    None,
                ),
                LaunchCellState::Denied { reason } => (
                    "denied".to_string(),
                    Some("denied".to_string()),
                    format!("改用下一家 provider,或按拒绝原因修好 {display_name} 后重新核验"),
                    Some(reason),
                ),
                LaunchCellState::EvidenceStale { .. } => (
                    "evidence_stale".to_string(),
                    Some("evidence_stale".to_string()),
                    format!("重新核验 {display_name} 的能力证据后重试本步"),
                    None,
                ),
            }
        } else {
            (
                "not_installed".to_string(),
                Some("cli_unavailable".to_string()),
                format!("装好 {display_name} 的命令行(命令要在 PATH 上)后重试本步"),
                None,
            )
        };

        if cli_available {
            match capability_state.as_str() {
                "confirmed" => {
                    if selected.is_none() {
                        selected = Some(provider);
                    }
                }
                "unverified" => {
                    if needs_verification.is_none() {
                        needs_verification = Some(provider);
                    }
                }
                "evidence_stale" if evidence_stale.is_none() => {
                    evidence_stale = Some(provider);
                }
                _ => {}
            }
        }
        candidates.push(AggregateProviderCandidate {
            provider: provider_wire,
            display_name: display_name.to_string(),
            cli_available,
            cli_reason_code,
            capability_state,
            skipped_reason,
            action_hint,
            warning,
        });
    }

    let table = AggregateProviderSelection {
        selected: selected
            .as_ref()
            .map(|provider| provider_wire_label(provider).to_string())
            .unwrap_or_default(),
        candidates,
        rule_version: PROVIDER_SELECTION_RULE_VERSION.to_string(),
        decided_at: now.to_rfc3339(),
    };
    if selected.is_some() {
        return Ok(ProviderSelectionOutcome::Selected(table));
    }
    if !table.candidates.iter().any(|row| row.cli_available) {
        return Ok(ProviderSelectionOutcome::NoCliAvailable(table));
    }
    if let Some(suggested) = needs_verification.or(evidence_stale) {
        return Ok(ProviderSelectionOutcome::NeedsVerification { suggested, table });
    }
    Ok(ProviderSelectionOutcome::Denied { table })
}

// ----------------------------------------------------------------------
// Task 4：知识库写分格选择入口（`select_aggregate_provider_for_write`）的测试。
// 与既有 `tests` 模块互为兄弟模块；既有模块的夹具是私有的，为不改既有测试
// 一字，此处只复制本用例所需的最小夹具。
// ----------------------------------------------------------------------
#[cfg(test)]
mod write_cell_tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::cross_cutting::aria_state_paths::AriaStatePaths;
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };
    use crate::cross_cutting::provider_boundary::PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN;
    use crate::cross_cutting::provider_health::ProviderHealthClock;
    use crate::product::logical_codebase::policy::{ProviderDialect, ProviderWireDialect};
    use crate::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
        ProviderActionMatrix,
    };
    use crate::product::logical_codebase::provider_gateway::{
        ProviderRefType, ResumeEvidenceState,
    };

    /// 假命令执行器:所有命令都「存在」且探测成功。
    struct AllPresentRunner;

    #[async_trait::async_trait]
    impl BoundedCommandRunner for AllPresentRunner {
        async fn run(
            &self,
            _request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            Ok(BoundedCommandResult {
                exit_code: Some(0),
                stdout: "9.9.9".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                duration_ms: 1,
            })
        }
    }

    /// 假时钟:固定「现在」,让快照过期判定可复现。
    struct FixedClock {
        now: DateTime<Utc>,
    }

    impl ProviderHealthClock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.now
        }
    }

    fn health_service(workspace_root: &Path, now: DateTime<Utc>) -> ProviderHealthService {
        ProviderHealthService::with_dependencies(
            AriaStatePaths::from_workspace_root(workspace_root),
            Arc::new(AllPresentRunner),
            Arc::new(FixedClock { now }),
            Duration::from_secs(5),
            4096,
        )
    }

    /// 两行能力矩阵：`PlanningReadOnly.launch = Confirmed` +
    /// `CodingTargetWrite.launch = <write_launch>`（Pi）。
    fn pi_dual_action_record(write_launch: ProviderCapabilityEvidence) -> ProviderCapabilityRecord {
        ProviderCapabilityRecord {
            provider_type: ProviderRefType::Pi,
            schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
            version: "1.0.0".to_string(),
            adapter_dialect: ProviderDialect::PiRpcV1,
            wire_dialect: ProviderWireDialect::PiRpc,
            capability_snapshot_ref: MANAGED_CAPABILITY_SNAPSHOT_REF.to_string(),
            evidence: CapabilityEvidence::ProductionVerified,
            resume_evidence: ResumeEvidenceState::Confirmed,
            supported_actions: Vec::new(),
            action_matrix: ProviderActionMatrix::from_rows(vec![
                ProviderActionCapability {
                    action: SessionPolicyAction::PlanningReadOnly,
                    launch: ProviderCapabilityEvidence::Confirmed,
                    resume: ProviderCapabilityEvidence::Unknown,
                    write_boundary: ProviderCapabilityEvidence::Unknown,
                    projection_digest: "projection-digest-planning".to_string(),
                    evidence_ref: "probe://planning".to_string(),
                },
                ProviderActionCapability {
                    action: SessionPolicyAction::CodingTargetWrite,
                    launch: write_launch,
                    resume: ProviderCapabilityEvidence::Unknown,
                    write_boundary: ProviderCapabilityEvidence::Unknown,
                    projection_digest: "projection-digest-write".to_string(),
                    evidence_ref: "probe://write".to_string(),
                },
            ])
            .expect("planning + write rows"),
            trust: ProviderCapabilityEvidence::Confirmed,
            probed_at: Some("2026-11-03T00:00:00Z".to_string()),
            probe_artifact_ref: Some("probe://artifact".to_string()),
            execution_semantics: PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN.to_string(),
        }
    }

    /// 只确认 `PlanningReadOnly` 行、未确认 `CodingTargetWrite` 行的记录：
    /// 既有入口（读分格）应选中它；写分格入口必须以「需核验」结论返回，
    /// 不得静默放行。
    #[tokio::test]
    async fn provider_selection_for_write_uses_coding_target_write_cell() {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let now = Utc::now();
        let health = health_service(root.path(), now);
        let store = ProviderCapabilityStore::new(paths.clone());
        store
            .upsert(&pi_dual_action_record(ProviderCapabilityEvidence::Unknown))
            .expect("upsert dual-action record");

        // 对照：既有入口按 `PlanningReadOnly` 分格判定 ⇒ 选中 Pi。
        let read_outcome = select_aggregate_provider_at(&paths, &health, "project_0001", now)
            .await
            .expect("read selection");
        assert!(
            matches!(read_outcome, ProviderSelectionOutcome::Selected(_)),
            "planning-confirmed record must be selected by the read entry: {read_outcome:?}"
        );

        // 新入口按 `CodingTargetWrite` 分格判定 ⇒ 同一记录是「需核验」。
        let write_outcome =
            select_aggregate_provider_for_write_at(&paths, &health, "project_0001", now)
                .await
                .expect("write selection");
        // 失败码断言放在 match 前：match 会把 `suggested` 部分移动出值。
        assert_eq!(
            write_outcome.reason_code(),
            Some(REASON_CODE_PROVIDER_NEEDS_VERIFICATION)
        );
        match write_outcome {
            ProviderSelectionOutcome::NeedsVerification { suggested, .. } => {
                assert_eq!(suggested, ProviderName::Pi);
            }
            other => {
                panic!("write entry must fail closed on an unconfirmed write cell, got {other:?}")
            }
        }
    }
}
