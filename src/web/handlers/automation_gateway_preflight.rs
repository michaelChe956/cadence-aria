//! P2 GAP-F(Task 0.2)→ C5 Task 3(REQ-WIGA-C5-PREFLIGHT)→ Task 8
//! (REQ-LCG-06):Enable 前完整角色链同源预检。
//!
//! 逐角色循环:plan_author/coder←author_provider,plan/code/internal
//! reviewer←reviewer_provider。判定与 gateway 同源:显式 provider 映射
//! (`provider_ref_for_name`/`ProviderRef::from_provider_name` 集中
//! fail-closed)+ Task 3 early 资格 verdict(`action_admission_verdict`
//! 只读 durable capability),不产生第二套支持矩阵。一次 422 列全全部
//! 违规角色(role/provider/action/reason_code + capability/projection
//! 引用);LC 的 `gateway_required=false` 无旁路效力;SingleRepository
//! 保留原跳过语义;`test_provider_enabled` 只按角色豁免 Fake。只收集
//! early 可确定的错误,不要求尚不存在的 attempt worktree 或 role-run
//! D4;实际 launch 继续完整门且无豁免。GET 投影、PUT Enable、rebind
//! 三调用点共用同一判定。

use crate::product::logical_codebase::policy::SessionPolicyAction;
use crate::product::logical_codebase::provider_admission_preflight::ProviderAdmissionError;
use crate::product::logical_codebase::provider_capability_store::PROVIDER_CAPABILITY_MIGRATION_REASON_CODES;
use crate::product::logical_codebase::provider_gateway::{
    LogicalCodebaseProviderGateway, PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED,
    PROVIDER_CAPABILITY_WRITE_BOUNDARY_NOT_CONFIRMED, PROVIDER_UNSUPPORTED_FOR_GATEWAY_LAUNCH,
};
use crate::product::models::ProviderName;
use crate::product::work_item_split_engine::engine::provider_ref_for_name;
use crate::web::error::{ApiError, ApiResult};
use serde_json::json;

use super::support::AutomationCarrierResolution;

/// GET 投影/PUT Enable/rebind 共用的稳定错误码(HTTP 422)。
pub(crate) const AUTOMATION_ROLE_CHAIN_UNSUPPORTED: &str = "automation_role_chain_unsupported";

/// gateway 组装失败(同源判定不可用)时的稳定判别码,与 Task 3
/// admission waiting 的兜底码同源——预检 fail-closed,不静默放行。
const PROVIDER_GATEWAY_UNAVAILABLE: &str = "provider_gateway_denied";

/// policy/body 漂移的 gateway Display 判别码前缀(漂移维度保留在 detail;
/// Task 14 冻结消费,不另立第二套漂移码)。
const PROVIDER_GATEWAY_POLICY_DRIFT_MARKER: &str = "provider_gateway_policy_drift";

/// 逐角色违规投影(Task 8 冻结契约):一次 422 列全,携带判定依据的
/// action 与 capability/projection 引用(early 阶段不可确定时为 None)。
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct AutomationRoleChainViolation {
    pub role: String,
    pub provider: String,
    pub action: SessionPolicyAction,
    pub reason_code: String,
    /// 判定读取的 capability snapshot 引用(映射失败等无法确定时 None)。
    pub capability_snapshot_ref: Option<String>,
    /// action 行 projection digest 引用(verdict 拒绝时无法提供,None)。
    pub projection_ref: Option<String>,
}

/// 角色链五角色 → (policy action, adapter role):plan_author 规划只读、
/// coder 目标写、三个 reviewer 评审只读。AdapterRole/permission 参数
/// 目前由 gateway 忽略(Task 7 统一 guard 接线),随 guard 同源演进。
fn role_action_and_adapter_role(
    role: &str,
) -> (SessionPolicyAction, crate::protocol::contracts::AdapterRole) {
    match role {
        "plan_author" => (
            SessionPolicyAction::PlanningReadOnly,
            crate::protocol::contracts::AdapterRole::Orchestrator,
        ),
        "coder" => (
            SessionPolicyAction::CodingTargetWrite,
            crate::protocol::contracts::AdapterRole::Executor,
        ),
        _ => (
            SessionPolicyAction::ReviewReadOnly,
            crate::protocol::contracts::AdapterRole::Reviewer,
        ),
    }
}

/// 从 early verdict 的 waiting 事实提取具体稳定判别码:detail 携带
/// Task 3/Task 14 判别码原文(比泛化 waiting reason 更可诊断),未命中时
/// 沿用 waiting 的 reason_code。
///
/// Task 14(lcg_t14):先逐枚消费冻结迁移判别码全集(与 capability store
/// 冻结清单同源),再沿既有 gateway 分格码;policy/body 漂移以 Display
/// 判别码透传。未命中时回退 waiting 的 reason_code(既有回退零回归)。
fn detailed_reason_code(waiting_reason_code: &str, detail: &str) -> String {
    for code in PROVIDER_CAPABILITY_MIGRATION_REASON_CODES {
        if detail.contains(code) {
            return code.to_string();
        }
    }
    for code in [
        PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED,
        PROVIDER_CAPABILITY_WRITE_BOUNDARY_NOT_CONFIRMED,
        PROVIDER_GATEWAY_POLICY_DRIFT_MARKER,
    ] {
        if detail.contains(code) {
            return code.to_string();
        }
    }
    waiting_reason_code.to_string()
}

/// 单角色同源判定:显式映射 + Task 3 early 资格 verdict;返回 None
/// 表示该角色 action 证据完整。只读,零 provider 启动、零写入。
fn role_chain_violation(
    gateway: Option<&LogicalCodebaseProviderGateway>,
    role: &str,
    provider: &ProviderName,
) -> Option<AutomationRoleChainViolation> {
    let provider_label = |provider: &ProviderName| {
        serde_json::to_value(provider)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| format!("{provider:?}"))
    };
    let (action, adapter_role) = role_action_and_adapter_role(role);
    // 显式 provider 映射(与 gateway 同源):Fake/未知值 fail-closed,
    // 不回退其它 provider。
    let provider_ref = match provider_ref_for_name(provider) {
        Ok(provider_ref) => provider_ref,
        Err(_error) => {
            return Some(AutomationRoleChainViolation {
                role: role.to_string(),
                provider: provider_label(provider),
                action,
                reason_code: PROVIDER_UNSUPPORTED_FOR_GATEWAY_LAUNCH.to_string(),
                capability_snapshot_ref: None,
                projection_ref: None,
            });
        }
    };
    let Some(gateway) = gateway else {
        // factory 缺失/组装失败:同源判定不可用,缺材料列 reason
        // (不建 worktree/provider),预检 fail-closed。
        return Some(AutomationRoleChainViolation {
            role: role.to_string(),
            provider: provider_label(provider),
            action,
            reason_code: PROVIDER_GATEWAY_UNAVAILABLE.to_string(),
            capability_snapshot_ref: Some(provider_ref.capability_snapshot_ref.clone()),
            projection_ref: None,
        });
    };
    match gateway.action_admission_verdict(
        &provider_ref,
        action,
        &adapter_role,
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
    ) {
        // verdict 通过:admission 携带的两枚引用不构成违规,不另列。
        Ok(_admission) => None,
        Err(ProviderAdmissionError::Waiting {
            reason_code,
            detail,
            ..
        }) => Some(AutomationRoleChainViolation {
            role: role.to_string(),
            provider: provider_label(provider),
            action,
            reason_code: detailed_reason_code(&reason_code, &detail),
            capability_snapshot_ref: Some(provider_ref.capability_snapshot_ref.clone()),
            projection_ref: None,
        }),
        Err(ProviderAdmissionError::Store(_error)) => Some(AutomationRoleChainViolation {
            role: role.to_string(),
            provider: provider_label(provider),
            action,
            reason_code: PROVIDER_GATEWAY_UNAVAILABLE.to_string(),
            capability_snapshot_ref: Some(provider_ref.capability_snapshot_ref.clone()),
            projection_ref: None,
        }),
    }
}

/// 共享核心:角色派生+逐角色同源判定循环。
/// - SingleRepository 载体先走原跳过(单仓不误拒,Review Focus 5/A10);
/// - LC 的 `gateway_required=false` 无旁路效力(Global Constraints 7):
///   标志不参与判定,仍检查全部角色;
/// - `test_provider_enabled` 只按角色豁免 Fake(Pi/KimiCode 不豁免);
/// - 全部角色的违规一次列全(不逐次试错)。
fn validate_role_chain(
    gateway: Option<&LogicalCodebaseProviderGateway>,
    author_provider: &ProviderName,
    reviewer_provider: &ProviderName,
    single_repository_carrier: bool,
    gateway_required: bool,
    test_provider_enabled: bool,
) -> ApiResult<()> {
    if single_repository_carrier {
        return Ok(());
    }
    let _ = gateway_required;
    let mut violations: Vec<AutomationRoleChainViolation> = Vec::new();
    for (role, provider) in [
        ("plan_author", author_provider),
        ("coder", author_provider),
        ("plan_reviewer", reviewer_provider),
        ("code_reviewer", reviewer_provider),
        // internal reviewer 无独立 ProviderName 变体:按同一 reviewer 配置
        // 三值派生先例取 reviewer_provider(enrollment options 的 reviewer
        // 必填,恒在场)。
        ("internal_reviewer", reviewer_provider),
    ] {
        if test_provider_enabled && matches!(provider, ProviderName::Fake) {
            continue;
        }
        if let Some(violation) = role_chain_violation(gateway, role, provider) {
            violations.push(violation);
        }
    }
    if violations.is_empty() {
        return Ok(());
    }
    let details = json!({
        "violations": violations,
        "hint": "reconfigure the automation providers or re-select the automation target",
    });
    Err(ApiError::validation_with_details(
        AUTOMATION_ROLE_CHAIN_UNSUPPORTED,
        "automation role chain has unsupported providers or missing action evidence",
        details,
    ))
}

/// Task 8b:三调用点(GET/Enable/rebind)共用的 readonly gateway 组装。
/// `lc_id` 取 resolver 冻结的权威身份(GET/Enable)或 enrollment 声明
/// target 的现有 LC(rebind);经 `build_readonly_for_lc` 只读组装
/// (不运行 `ensure_bootstrap`,GET 不写 policy/capability/trust audit)。
/// factory 缺失或组装失败返回 `None`——预检 fail-closed 列违规
/// (`provider_gateway_denied`),不静默放行、不在读取路径物化自举桩。
pub(crate) fn readonly_preflight_gateway(
    state: &crate::web::state::WebAppState,
    project_id: &str,
    lc_id: Option<&str>,
) -> Option<LogicalCodebaseProviderGateway> {
    let factory = state.gateway_factory()?;
    let lc_id = lc_id?;
    factory
        .build_readonly_for_lc(project_id, Some(lc_id))
        .map_err(|error| {
            tracing::warn!(
                project_id,
                lc_id,
                error = %error,
                "automation role-chain preflight: readonly gateway build failed; failing closed"
            );
            error
        })
        .ok()
}

/// 契约入口(Task 8 签名):GET 投影与 PUT Enable 的载体判定来自
/// `resolve_automation_carrier`(唯一 resolver),同一 carrier 进同一
/// 判定;`gateway` 为调用方经 readonly factory(`build_readonly_for_lc`)
/// 组装的只读 gateway。
pub(crate) fn validate_role_chain_for_enrollment(
    gateway: Option<&LogicalCodebaseProviderGateway>,
    author_provider: &ProviderName,
    reviewer_provider: &ProviderName,
    carrier: &AutomationCarrierResolution,
    gateway_required: bool,
    test_provider_enabled: bool,
) -> ApiResult<()> {
    validate_role_chain(
        gateway,
        author_provider,
        reviewer_provider,
        matches!(
            carrier,
            AutomationCarrierResolution::SingleRepository { .. }
        ),
        gateway_required,
        test_provider_enabled,
    )
}

/// rebind 调用点专用:手上只有 enrollment 声明的 target(无 authority
/// resolution),按声明 target 的载体类别进同一判定核心——沿 declared
/// target 读取现有 LC/成员,不重解析 issue 权威载体(契约「rebind 的
/// carrier 取 enrollment 现有 target」)。
pub(crate) fn validate_role_chain_for_declared_enrollment_target(
    gateway: Option<&LogicalCodebaseProviderGateway>,
    author_provider: &ProviderName,
    reviewer_provider: &ProviderName,
    declared: &crate::product::logical_codebase::EnrollmentTarget,
    gateway_required: bool,
    test_provider_enabled: bool,
) -> ApiResult<()> {
    validate_role_chain(
        gateway,
        author_provider,
        reviewer_provider,
        matches!(
            declared,
            crate::product::logical_codebase::EnrollmentTarget::SingleRepository { .. }
        ),
        gateway_required,
        test_provider_enabled,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
    use crate::product::logical_codebase::policy::{
        ProviderDialect, ProviderWireDialect, SessionPolicyAction,
    };
    use crate::product::logical_codebase::production_policy_resolvers::StoreBackedProviderCapabilitySource;
    use crate::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
        ProviderActionMatrix, ProviderCapabilityRecord, ProviderCapabilityStore,
        RootRecipeEvidence,
    };
    use crate::product::logical_codebase::provider_gateway::{
        CODEX_DANGER_FULL_ACCESS_UNSUPPORTED, LogicalCodebaseProviderGateway, PolicyTargetResolver,
        ProviderRefType, ResumeEvidenceState,
    };
    use crate::product::logical_codebase::{
        AuthorityAggregateIndexReference, EnrollmentTarget, RepositoryAuthorityResolution,
        RepositoryTargetKind, ResolvedTargetIdentity,
    };
    use crate::product::models::ProviderName;
    use std::sync::Arc;

    fn lc_carrier() -> AutomationCarrierResolution {
        AutomationCarrierResolution::LogicalCodebase {
            resolution: Box::new(RepositoryAuthorityResolution {
                authority_root: std::path::PathBuf::new(),
                target: ResolvedTargetIdentity {
                    kind: RepositoryTargetKind::LogicalCodebase,
                    repository_id: None,
                    logical_codebase_id: Some("logical_codebase_0001".to_string()),
                    logical_repository_id: None,
                    checkout_id: None,
                    canonical_path: std::path::PathBuf::new(),
                    source_identity_digest: String::new(),
                },
                manifest: None,
                selection: None,
                policy: None,
                aggregate_index: AuthorityAggregateIndexReference {
                    aggregate_index_id: None,
                    membership_revision: None,
                    status: None,
                },
            }),
        }
    }

    fn single_repository_carrier() -> AutomationCarrierResolution {
        AutomationCarrierResolution::SingleRepository {
            target: EnrollmentTarget::SingleRepository {
                repository_id: "repo-1".to_string(),
            },
        }
    }

    fn lc_declared_target() -> EnrollmentTarget {
        EnrollmentTarget::LogicalCodebase {
            logical_codebase_id: "logical_codebase_0001".to_string(),
            logical_repository_id: crate::product::logical_codebase::LogicalRepositoryId(
                uuid::Uuid::nil(),
            ),
        }
    }

    fn violations_of(error: ApiError) -> Vec<serde_json::Value> {
        error
            .details
            .get("violations")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default()
    }

    /// 三调用点(GET/Enable/rebind)共享 verdict 的比较面:违规
    /// role+reason_code 序列。
    fn violation_reasons(error: ApiError) -> Vec<String> {
        violations_of(error)
            .iter()
            .map(|violation| {
                format!(
                    "{}:{}",
                    violation["role"].as_str().unwrap_or_default(),
                    violation["reason_code"].as_str().unwrap_or_default()
                )
            })
            .collect()
    }

    // ---- Task 8 fixture:真实 store-backed capability source + 计数 adapter ----

    /// 预检永不解析 target(`action_admission_verdict` 只读 capability,
    /// 不触达 resolver);占位实现 fail-closed。
    struct UnreachableTargetResolver;
    impl PolicyTargetResolver for UnreachableTargetResolver {
        fn resolve_and_revalidate(
            &self,
            _request: &crate::product::logical_codebase::provider_gateway::SessionLaunchRequest,
        ) -> Result<
            crate::product::logical_codebase::policy::PolicyTarget,
            crate::product::logical_codebase::provider_gateway::ProviderGatewayError,
        > {
            Err(
                crate::product::logical_codebase::provider_gateway::ProviderGatewayError::Target(
                    "role-chain preflight never resolves targets".to_string(),
                ),
            )
        }
    }

    struct CountingStreamingAdapter {
        start_count: std::sync::atomic::AtomicUsize,
    }
    impl CountingStreamingAdapter {
        fn new() -> Self {
            Self {
                start_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }
        fn start_count(&self) -> usize {
            self.start_count.load(std::sync::atomic::Ordering::SeqCst)
        }
    }
    #[async_trait::async_trait]
    impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
        for CountingStreamingAdapter
    {
        async fn start(
            &self,
            _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<
            crate::cross_cutting::streaming_provider::ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        > {
            self.start_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (_event_tx, events) = tokio::sync::mpsc::channel(1);
            let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
            Ok(crate::cross_cutting::streaming_provider::ProviderSession {
                events,
                commands,
                native_session_id: None,
            })
        }
    }

    struct StubSyncAdapter;
    impl crate::cross_cutting::provider_adapter::ProviderAdapter for StubSyncAdapter {
        fn run(
            &self,
            _input: &crate::protocol::contracts::AdapterInput,
        ) -> Result<
            crate::protocol::contracts::AdapterOutput,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        > {
            use crate::protocol::contracts::TimeoutStatus;
            Ok(crate::protocol::contracts::AdapterOutput {
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                structured_output: None,
                files_modified: Vec::new(),
                duration_ms: 0,
                timeout_status: TimeoutStatus::NotTimedOut,
            })
        }
    }

    fn always_available_gate() -> Arc<ProviderAvailabilityGate> {
        use crate::cross_cutting::provider_availability_gate::ProviderHealthSource;
        use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
        use chrono::Utc;

        struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);
        impl ProviderHealthSource for AlwaysHealthy {
            fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
                self.0.clone()
            }
            fn degraded(&self) -> bool {
                false
            }
        }

        let checked_at = Utc::now();
        let snapshot = Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 1,
            checked_at,
            providers: [
                ProviderName::ClaudeCode,
                ProviderName::Codex,
                ProviderName::Pi,
                ProviderName::KimiCode,
            ]
            .into_iter()
            .map(|provider| ProviderHealthEntry {
                provider,
                command: "stub".to_string(),
                available: true,
                version: Some("1.0".to_string()),
                reason_code: None,
                reason: None,
                checked_at,
            })
            .collect(),
        });
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AlwaysHealthy(
            snapshot,
        ))))
    }

    /// capability 记录:`confirmed=true` 三 action 行全 Confirmed 且 trust
    /// Confirmed(证据完整);否则矩阵/边界/trust 全 Unknown(未探测,
    /// 「boundary Unknown/trust 缺失」的 action 证据缺失形态)。
    fn capability_record(
        provider_type: ProviderRefType,
        confirmed: bool,
    ) -> ProviderCapabilityRecord {
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
        let evidence = |confirmed: bool| {
            if confirmed {
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed
            } else {
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Unknown
            }
        };
        let row = |action: SessionPolicyAction| ProviderActionCapability {
            action,
            launch: evidence(confirmed),
            resume: evidence(confirmed),
            write_boundary: evidence(confirmed),
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
            action_matrix: if confirmed {
                ProviderActionMatrix::from_rows(vec![
                    row(SessionPolicyAction::PlanningReadOnly),
                    row(SessionPolicyAction::CodingTargetWrite),
                    row(SessionPolicyAction::ReviewReadOnly),
                ])
                .unwrap()
            } else {
                ProviderActionMatrix::unknown_all()
            },
            trust: evidence(confirmed),
            probed_at: None,
            probe_artifact_ref: None,
            root_recipe_evidence: RootRecipeEvidence::None,
        }
    }

    /// ClaudeCode bootstrap 过渡桥形状:矩阵全 Unknown 但 legacy
    /// supported_actions 全列——require_supported 沿过渡桥放行(既有
    /// root recipe 聚合链零回归语义)。
    fn claude_bootstrap_record() -> ProviderCapabilityRecord {
        let mut record = capability_record(ProviderRefType::ClaudeCode, false);
        record.evidence = CapabilityEvidence::FixtureVerified;
        record.resume_evidence = ResumeEvidenceState::Confirmed;
        record.supported_actions = vec![
            SessionPolicyAction::PlanningReadOnly,
            SessionPolicyAction::CodingTargetWrite,
            SessionPolicyAction::ReviewReadOnly,
        ];
        record
    }

    struct RoleChainFixture {
        _temp: tempfile::TempDir,
        streaming_adapter: Arc<CountingStreamingAdapter>,
        gateway: LogicalCodebaseProviderGateway,
    }

    /// 真实 store-backed gateway + 指定 capability 记录;preflight 全程
    /// 只读(`action_admission_verdict`),registry 挂计数 adapter 以断言
    /// 零 provider 启动。
    fn role_chain_fixture(records: &[ProviderCapabilityRecord]) -> RoleChainFixture {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = ProductAppPaths::new(temp.path().to_path_buf());
        let project_id = "project_t08".to_string();
        let lc_id = "lc_t08".to_string();
        let capability_store = ProviderCapabilityStore::for_lc(paths.clone(), lc_id.clone());
        for record in records {
            capability_store
                .upsert(&project_id, record)
                .expect("upsert");
        }
        let streaming_adapter = Arc::new(CountingStreamingAdapter::new());
        let mut registry = ProviderRegistry::new();
        for provider in [
            ProviderName::ClaudeCode,
            ProviderName::Codex,
            ProviderName::Pi,
            ProviderName::KimiCode,
        ] {
            registry.register(provider, streaming_adapter.clone());
        }
        let authority_root = temp.path().join("authority-root");
        std::fs::create_dir_all(&authority_root).expect("authority root");
        let gateway = LogicalCodebaseProviderGateway::new(
            AggregatePolicyArtifactStore::new(paths.clone()),
            Arc::new(StoreBackedProviderCapabilitySource::for_lc(
                paths, project_id, lc_id,
            )),
            Arc::new(UnreachableTargetResolver),
            Arc::new(registry),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
            authority_root,
        );
        RoleChainFixture {
            _temp: temp,
            streaming_adapter,
            gateway,
        }
    }

    /// Task 8 主链(断言组 401-406):同一次预检列全全部角色的 action
    /// 证据失败——Pi 行未探测(boundary Unknown)、Kimi trust 未建立
    /// (记录 trust Unknown 且行未探测)同场五角色全列;Codex 受限
    /// coding 协议证据缺失经路由禁令列出。预检零 provider 启动。
    #[tokio::test]
    async fn lcg_t08_preflight_lists_all_role_provider_action_evidence_failures() {
        use axum::response::IntoResponse;

        let unprobed = role_chain_fixture(&[
            capability_record(ProviderRefType::Pi, false),
            capability_record(ProviderRefType::KimiCode, false),
        ]);
        let error = validate_role_chain_for_enrollment(
            Some(&unprobed.gateway),
            &ProviderName::Pi,
            &ProviderName::KimiCode,
            &lc_carrier(),
            true,
            false,
        )
        .unwrap_err();
        // HTTP 形态:同一 ApiError 经 IntoResponse 映射 422 + 稳定码。
        let response = error.into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(payload["code"], "automation_role_chain_unsupported");
        let violations = payload["details"]["violations"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let roles: Vec<&str> = violations
            .iter()
            .map(|violation| violation["role"].as_str().unwrap())
            .collect();
        assert_eq!(
            roles,
            [
                "plan_author",
                "coder",
                "plan_reviewer",
                "code_reviewer",
                "internal_reviewer"
            ]
        );
        assert!(
            violations
                .iter()
                .all(|v| v.get("action").is_some() && v.get("reason_code").is_some()),
            "violations must carry action evidence fields: {violations:?}"
        );
        // 缺当前 action 证据的稳定判别码(Pi boundary 行未探测/Kimi
        // trust 缺失均以 launch 分格证据缺失呈现)。
        assert!(
            violations
                .iter()
                .all(|v| v["reason_code"] == "provider_capability_launch_not_confirmed")
        );

        // Codex 未探针记录:plan_author/coder 按各 action 证据缺失列出
        // launch_not_confirmed 稳定码——r47(373fba91)起 Codex 受限裁决已
        // 移至 capability 行(enforce_route_policy 桩化),角色链不再携带
        // codex_danger 码(该码由 capability 面冻结码表承载)。
        let codex = role_chain_fixture(&[
            capability_record(ProviderRefType::Codex, false),
            capability_record(ProviderRefType::KimiCode, false),
        ]);
        let error = validate_role_chain_for_enrollment(
            Some(&codex.gateway),
            &ProviderName::Codex,
            &ProviderName::KimiCode,
            &lc_carrier(),
            true,
            false,
        )
        .unwrap_err();
        let violations = violations_of(error);
        let head: Vec<&str> = violations
            .iter()
            .map(|violation| violation["role"].as_str().unwrap())
            .take(2)
            .collect();
        assert_eq!(head, ["plan_author", "coder"], "{violations:?}");
        assert!(
            violations
                .iter()
                .take(2)
                .all(|v| v["reason_code"] == "provider_capability_launch_not_confirmed"),
            "{violations:?}"
        );

        let provider_start_count =
            unprobed.streaming_adapter.start_count() + codex.streaming_adapter.start_count();
        assert_eq!(provider_start_count, 0);
    }

    /// Task 8(断言组 407):LC `gateway_required=false` 无旁路效力——
    /// 与 true 标志产出同一 verdict。
    #[test]
    fn lcg_t08_lc_gateway_false_still_checks_all_roles() {
        let fixture = role_chain_fixture(&[claude_bootstrap_record()]);
        let lc_false_flag_verdict = violation_reasons(
            validate_role_chain_for_enrollment(
                Some(&fixture.gateway),
                &ProviderName::Pi,
                &ProviderName::ClaudeCode,
                &lc_carrier(),
                false,
                false,
            )
            .unwrap_err(),
        );
        let lc_true_flag_verdict = violation_reasons(
            validate_role_chain_for_enrollment(
                Some(&fixture.gateway),
                &ProviderName::Pi,
                &ProviderName::ClaudeCode,
                &lc_carrier(),
                true,
                false,
            )
            .unwrap_err(),
        );
        assert!(!lc_false_flag_verdict.is_empty());
        assert_eq!(lc_false_flag_verdict, lc_true_flag_verdict);
    }

    /// Task 8(断言组 408):SingleRepository 保留原跳过语义——LC 下
    /// 全违规的组合在单仓载体下不误拒。
    #[test]
    fn lcg_t08_single_repository_skips_lc_predicates() {
        let fixture = role_chain_fixture(&[]);
        let single_repository_verdict = validate_role_chain_for_enrollment(
            Some(&fixture.gateway),
            &ProviderName::Pi,
            &ProviderName::KimiCode,
            &single_repository_carrier(),
            true,
            false,
        );
        assert!(single_repository_verdict.is_ok());
    }

    /// Task 8(断言组 409-410):GET 投影、PUT Enable 从同 carrier,rebind
    /// 沿 declared target——三调用点共享同一 verdict。
    #[test]
    fn lcg_t08_get_enable_rebind_share_verdict() {
        let fixture = role_chain_fixture(&[
            capability_record(ProviderRefType::Pi, false),
            capability_record(ProviderRefType::KimiCode, false),
        ]);
        let get_reasons = violation_reasons(
            validate_role_chain_for_enrollment(
                Some(&fixture.gateway),
                &ProviderName::Pi,
                &ProviderName::KimiCode,
                &lc_carrier(),
                true,
                false,
            )
            .unwrap_err(),
        );
        let enable_reasons = violation_reasons(
            validate_role_chain_for_enrollment(
                Some(&fixture.gateway),
                &ProviderName::Pi,
                &ProviderName::KimiCode,
                &lc_carrier(),
                true,
                false,
            )
            .unwrap_err(),
        );
        let rebind_reasons = violation_reasons(
            validate_role_chain_for_declared_enrollment_target(
                Some(&fixture.gateway),
                &ProviderName::Pi,
                &ProviderName::KimiCode,
                &lc_declared_target(),
                true,
                false,
            )
            .unwrap_err(),
        );
        assert!(!get_reasons.is_empty());
        assert_eq!(get_reasons, enable_reasons);
        assert_eq!(enable_reasons, rebind_reasons);
    }

    /// 主链(1a 四家映射后重钉):Pi author + KimiCode reviewer 的全部
    /// 证据缺失角色一次列全(roles 序即五角色链序);证据完整的
    /// ClaudeCode 组合通过。
    #[test]
    fn role_chain_preflight_lists_every_violating_role_at_once() {
        let fixture = role_chain_fixture(&[
            capability_record(ProviderRefType::Pi, false),
            capability_record(ProviderRefType::KimiCode, false),
            capability_record(ProviderRefType::ClaudeCode, true),
        ]);
        let error = validate_role_chain_for_enrollment(
            Some(&fixture.gateway),
            &ProviderName::Pi,
            &ProviderName::KimiCode,
            &lc_carrier(),
            true,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, AUTOMATION_ROLE_CHAIN_UNSUPPORTED);
        let violations = violations_of(error);
        let roles: Vec<&str> = violations
            .iter()
            .map(|violation| violation["role"].as_str().unwrap())
            .collect();
        assert_eq!(
            roles,
            vec![
                "plan_author",
                "coder",
                "plan_reviewer",
                "code_reviewer",
                "internal_reviewer"
            ],
            "every violating role must be listed at once: {violations:?}"
        );

        // 证据完整的组合零违规。
        assert!(
            validate_role_chain_for_enrollment(
                Some(&fixture.gateway),
                &ProviderName::ClaudeCode,
                &ProviderName::ClaudeCode,
                &lc_carrier(),
                true,
                false
            )
            .is_ok()
        );
    }

    /// 路由阻断:任一角色派生为 Codex(未探针记录)→ 该角色按各 action
    /// 证据缺失列出 launch_not_confirmed 稳定码(r47 373fba91 起 Codex
    /// 受限裁决移至 capability 行,codex_danger 由冻结码表在 capability
    /// 面承载)。
    #[test]
    fn role_chain_preflight_routes_codex_violations() {
        let fixture = role_chain_fixture(&[
            capability_record(ProviderRefType::Codex, false),
            capability_record(ProviderRefType::ClaudeCode, true),
        ]);
        let error = validate_role_chain_for_declared_enrollment_target(
            Some(&fixture.gateway),
            &ProviderName::Codex,
            &ProviderName::ClaudeCode,
            &lc_declared_target(),
            true,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, AUTOMATION_ROLE_CHAIN_UNSUPPORTED);
        let violations = violations_of(error);
        let roles: Vec<&str> = violations
            .iter()
            .map(|violation| violation["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, vec!["plan_author", "coder"]);
        // r47 语义(同 lcg_t08 重钉):角色链对未探针 Codex 记录列
        // launch_not_confirmed 稳定码;codex_danger 由 capability 面承载。
        for violation in &violations {
            assert!(
                violation["reason_code"]
                    .as_str()
                    .unwrap()
                    .contains(PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED),
                "{violation:?}"
            );
        }
    }

    /// 单仓载体整体跳过 gateway 谓词:LC 不支持但本机可用的组合不误拒
    ///（Review Focus 5／A10）。
    #[test]
    fn role_chain_preflight_skips_gateway_predicates_for_single_repository() {
        let fixture = role_chain_fixture(&[]);
        for (author, reviewer) in [
            (ProviderName::Pi, ProviderName::KimiCode),
            (ProviderName::KimiCode, ProviderName::Pi),
        ] {
            assert!(
                validate_role_chain_for_enrollment(
                    Some(&fixture.gateway),
                    &author,
                    &reviewer,
                    &single_repository_carrier(),
                    true,
                    false
                )
                .is_ok(),
                "single-repository carrier must skip gateway predicates"
            );
        }
    }

    /// 旧实名 1/3 重钉(Task 8 新合同):Pi/KimiCode 已是 1a 合法映射,
    /// 「恒静态 unsupported」语义失效——缺当前 action 证据(capability
    /// 记录缺失)时拒,证据完整(行 Confirmed)时通过。
    #[test]
    fn kimi_code_and_pi_are_admitted_only_with_action_evidence() {
        let without_evidence = role_chain_fixture(&[]);
        let with_evidence = role_chain_fixture(&[
            capability_record(ProviderRefType::Pi, true),
            capability_record(ProviderRefType::KimiCode, true),
            // reviewer=ClaudeCode 的三角色同样需要证据完整才放行。
            capability_record(ProviderRefType::ClaudeCode, true),
        ]);
        for provider in [ProviderName::KimiCode, ProviderName::Pi] {
            let error = validate_role_chain_for_declared_enrollment_target(
                Some(&without_evidence.gateway),
                &provider,
                &ProviderName::ClaudeCode,
                &lc_declared_target(),
                true,
                false,
            )
            .unwrap_err();
            assert_eq!(error.code, AUTOMATION_ROLE_CHAIN_UNSUPPORTED);
            let violations = violations_of(error);
            assert!(
                violations.iter().all(|violation| {
                    violation["reason_code"] == "provider_capability_launch_not_confirmed"
                }),
                "missing action evidence must deny via the capability verdict, got: {violations:?}"
            );
            // 证据完整(行 Confirmed)时通过。
            assert!(
                validate_role_chain_for_declared_enrollment_target(
                    Some(&with_evidence.gateway),
                    &provider,
                    &ProviderName::ClaudeCode,
                    &lc_declared_target(),
                    true,
                    false
                )
                .is_ok(),
                "confirmed action rows must admit {provider:?}"
            );
        }
    }

    /// 旧实名 2/3:Codex 在当前固定 danger-full-access sandbox 下被路由
    /// 禁令拒绝(danger profile 无受限协议证据)。
    #[test]
    fn codex_is_rejected_under_current_default_sandbox() {
        let fixture = role_chain_fixture(&[
            capability_record(ProviderRefType::Codex, false),
            capability_record(ProviderRefType::ClaudeCode, true),
        ]);
        let error = validate_role_chain_for_declared_enrollment_target(
            Some(&fixture.gateway),
            &ProviderName::ClaudeCode,
            &ProviderName::Codex,
            &lc_declared_target(),
            true,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, AUTOMATION_ROLE_CHAIN_UNSUPPORTED);
        let violations = violations_of(error);
        assert_eq!(
            violations
                .iter()
                .filter(|violation| violation["role"] == "plan_reviewer")
                .count(),
            1
        );
    }

    /// 旧实名 3/3(重钉):ClaudeCode 通过;Fake 仅测试运行豁免;LC
    /// `gateway_required=false` 无旁路;测试模式不放行 Pi/KimiCode。
    #[test]
    fn claude_code_passes_and_fake_requires_test_run() {
        let fixture = role_chain_fixture(&[claude_bootstrap_record()]);
        assert!(
            validate_role_chain_for_declared_enrollment_target(
                Some(&fixture.gateway),
                &ProviderName::ClaudeCode,
                &ProviderName::ClaudeCode,
                &lc_declared_target(),
                true,
                false
            )
            .is_ok()
        );
        // 真实运行 Fake 无 gateway dialect(映射 fail-closed);仅测试运行豁免。
        assert!(
            validate_role_chain_for_declared_enrollment_target(
                Some(&fixture.gateway),
                &ProviderName::Fake,
                &ProviderName::Fake,
                &lc_declared_target(),
                true,
                false
            )
            .is_err()
        );
        assert!(
            validate_role_chain_for_declared_enrollment_target(
                Some(&fixture.gateway),
                &ProviderName::Fake,
                &ProviderName::Fake,
                &lc_declared_target(),
                true,
                true
            )
            .is_ok()
        );
        // LC 的 gateway_required=false 无旁路效力:仍检查全部角色。
        assert!(
            validate_role_chain_for_declared_enrollment_target(
                Some(&fixture.gateway),
                &ProviderName::KimiCode,
                &ProviderName::KimiCode,
                &lc_declared_target(),
                false,
                false
            )
            .is_err()
        );
        // 测试模式不放行 Pi/KimiCode(缺证据照拒)。
        assert!(
            validate_role_chain_for_declared_enrollment_target(
                Some(&fixture.gateway),
                &ProviderName::Pi,
                &ProviderName::KimiCode,
                &lc_declared_target(),
                true,
                true
            )
            .is_err()
        );
    }

    /// Task 14(lcg_t14 诊断投影):waiting detail 携带任一冻结迁移判别码
    /// 时,`detailed_reason_code` 投影该冻结码(不折叠为泛化 waiting
    /// reason);policy/body 漂移以 gateway Display 判别码透传;未命中时
    /// 沿用 waiting reason(既有回退语义零回归)。
    #[test]
    fn lcg_t14_diagnostic_projection_consumes_frozen_migration_reason_codes() {
        use crate::product::logical_codebase::provider_capability_store::PROVIDER_CAPABILITY_MIGRATION_REASON_CODES;

        for code in PROVIDER_CAPABILITY_MIGRATION_REASON_CODES {
            assert_eq!(
                detailed_reason_code(
                    "provider_capability_not_satisfied",
                    &format!("waiting detail carries {code} verbatim"),
                ),
                code,
                "frozen migration reason codes must be projected verbatim"
            );
        }

        // policy/body 漂移:gateway Display 判别码透传(维度保留在 detail)。
        assert_eq!(
            detailed_reason_code(
                "provider_gateway_denied",
                "provider_gateway_policy_drift: policy_body",
            ),
            "provider_gateway_policy_drift"
        );

        // 既有 gateway 分格码仍优先透传(与本清单并存互补)。
        assert_eq!(
            detailed_reason_code(
                "provider_capability_not_satisfied",
                &format!("detail {PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED} tail"),
            ),
            PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED
        );

        // 未命中任何冻结码:沿用 waiting 的 reason_code(回退零回归)。
        assert_eq!(
            detailed_reason_code("provider_capability_not_satisfied", "unrelated detail"),
            "provider_capability_not_satisfied"
        );
    }
}
