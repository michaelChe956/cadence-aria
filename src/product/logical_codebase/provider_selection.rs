//! 聚合冷启动的 provider 选择器:能力判定与选择器**同源**。
//!
//! 「同源」是结构性保证,不是约定:
//! - 判定只有一份实现:`launch_cell_state_from_record`(纯函数,判定 record 的
//!   launch 分格)与 `launch_cell_state`(载入 record 后判定)。
//! - gateway 的 `require_supported`(`production_policy_resolvers.rs`)与本模块的
//!   `select_aggregate_provider` **都调上面这两个函数**。
//!
//! 于是「被判定为可用」与「被选中」永远是同一个结论:不存在「网关放行但选择器
//! 不选」或反之的两套逻辑。
//!
//! 两层判据(REQ-BOOT-07):
//! - **L1 命令可用**:provider health 快照的 `available` + 六值原因码;仅当
//!   `generation == 0` / 快照降级 / `checked_at` 早于 `now - 10min` 时才刷新一次。
//! - **L2 能力已核验**:本逻辑代码库 `PlanningReadOnly.launch` 分格为 `Confirmed`。
//!
//! **本模块不创建 operation**:判定「无可用候选」时只返回失败结论,由调用方决定
//! 是否落失败记录。

use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use crate::cross_cutting::provider_health::{
    ProviderHealthReasonCode, ProviderHealthService, ProviderHealthSnapshot,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::aggregate_initialization::{
    AggregateProviderCandidate, AggregateProviderSelection,
};
use crate::product::logical_codebase::policy::SessionPolicyAction;
use crate::product::logical_codebase::production_policy_resolvers::StoreBackedProviderCapabilitySource;
use crate::product::logical_codebase::provider_capability_store::{
    MANAGED_CAPABILITY_SNAPSHOT_REF, ProviderCapabilityRecord, ProviderCapabilityStore,
};
use crate::product::logical_codebase::provider_gateway::{
    PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED, ProviderGatewayError, ProviderRef,
};
use crate::product::logical_codebase::provider_trust::provider_wire_label;
use crate::product::models::ProviderName;

/// 聚合冷启动的 provider 候选顺序(owner 顺序,冻结)。
pub const AGGREGATE_PROVIDER_CANDIDATE_ORDER: [ProviderName; 4] = [
    ProviderName::Pi,
    ProviderName::KimiCode,
    ProviderName::ClaudeCode,
    ProviderName::Codex,
];

/// 选择期失败码:四家命令都不可用。
pub const REASON_CODE_NO_PROVIDER_AVAILABLE: &str =
    "aggregate_initialization_no_provider_available";
/// 选择期失败码:命令可用,但没有一家完成核验。
pub const REASON_CODE_PROVIDER_NEEDS_VERIFICATION: &str =
    "aggregate_initialization_provider_needs_verification";
/// 选择期失败码:命令可用,但候选都被真实拒绝。
pub const REASON_CODE_PROVIDER_CAPABILITY_DENIED: &str =
    "aggregate_initialization_provider_capability_denied";
/// 选择期失败码:provider health 探测降级/失败。
pub const REASON_CODE_PROVIDER_HEALTH_DEGRADED: &str =
    "aggregate_initialization_provider_health_degraded";

/// 候选表规则版本(冻结:owner 顺序 + L1/L2 两层判据)。
pub const PROVIDER_SELECTION_RULE_VERSION: &str = "aggregate-provider-owner-order-v1";

/// gateway 判定「快照引用不一致」时的既有文案(分类器按同一子串匹配)。
const CAPABILITY_SNAPSHOT_MISMATCH: &str = "capability snapshot mismatch";

/// health 快照超过该时长没有刷新过就算过期(10 分钟)。
const HEALTH_STALE_AFTER_MINUTES: i64 = 10;

/// launch 分格判定结论(判定与选择器共用的唯一词汇)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchCellState {
    /// 已核验,可放行。
    Confirmed,
    /// 未核验(记录缺失 / 分格 Unknown):需要一次真实核验。
    Unverified { detail: String },
    /// 真实拒绝(带真实原因)。
    Denied { reason: String },
    /// 证据过期(快照引用与 provider 不一致)。
    EvidenceStale { detail: String },
}

impl LaunchCellState {
    /// 是否放行(只有 `Confirmed` 放行)。
    pub fn is_confirmed(&self) -> bool {
        matches!(self, LaunchCellState::Confirmed)
    }
}

/// 判定 record 的 launch 分格:**唯一**的判定实现。
///
/// gateway 的 `require_supported` 与选择器都调它,故两处结论必然一致。
pub fn launch_cell_state_from_record(
    record: &ProviderCapabilityRecord,
    action: SessionPolicyAction,
) -> LaunchCellState {
    let row = record.action_matrix.row(&action);
    match &row.launch {
        ProviderCapabilityEvidence::Confirmed => LaunchCellState::Confirmed,
        ProviderCapabilityEvidence::Denied { reason } => LaunchCellState::Denied {
            reason: reason.clone(),
        },
        ProviderCapabilityEvidence::Unknown => LaunchCellState::Unverified {
            detail: format!("{action:?} launch cell is unknown"),
        },
    }
}

/// 载入 record 后判定(选择器用)。
///
/// 映射规则固定:
/// - 记录缺失 → `Unverified`(需要一次真实核验,不是拒绝)
/// - 快照引用不一致 → `EvidenceStale`
/// - 其它能力门失败 → `Unverified`(fail-closed:只认 `Confirmed`)
/// - store IO 错误 → `Err`(**不得**吞成放行)
pub fn launch_cell_state(
    source: &StoreBackedProviderCapabilitySource,
    provider: &ProviderRef,
    action: SessionPolicyAction,
) -> Result<LaunchCellState, ProductStoreError> {
    match source.load_record(provider) {
        Ok(record) => Ok(launch_cell_state_from_record(&record, action)),
        Err(ProviderGatewayError::Policy(store_error)) => Err(store_error),
        Err(ProviderGatewayError::UnsupportedCapability(message)) => {
            if message.contains(CAPABILITY_SNAPSHOT_MISMATCH) {
                Ok(LaunchCellState::EvidenceStale { detail: message })
            } else {
                Ok(LaunchCellState::Unverified { detail: message })
            }
        }
        Err(other) => Ok(LaunchCellState::Unverified {
            detail: other.to_string(),
        }),
    }
}

/// 判定结论 → gateway 拒绝错误(稳定前缀 + 形态与既有文案一致)。
pub fn launch_rejection_error(
    action: SessionPolicyAction,
    state: &LaunchCellState,
) -> ProviderGatewayError {
    match state {
        LaunchCellState::Unverified { .. } => ProviderGatewayError::UnsupportedCapability(format!(
            "{PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED}: {action:?} launch cell is unknown"
        )),
        LaunchCellState::Denied { reason } => ProviderGatewayError::UnsupportedCapability(format!(
            "{PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED}: {action:?} launch cell is denied; reason: {reason}"
        )),
        // 快照不一致的既有文案原样保留(分类器按同一子串匹配)。
        LaunchCellState::EvidenceStale { detail } => {
            ProviderGatewayError::UnsupportedCapability(detail.clone())
        }
        // `Confirmed` 不该被拒绝:调用方只对非 `Confirmed` 结论走这里。
        LaunchCellState::Confirmed => ProviderGatewayError::UnsupportedCapability(format!(
            "{PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED}: {action:?} launch cell is confirmed"
        )),
    }
}

/// 选择结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderSelectionOutcome {
    /// 选中一家(命令可用 + 已核验)。
    Selected(AggregateProviderSelection),
    /// 四家命令都不可用。
    NoCliAvailable(AggregateProviderSelection),
    /// 命令可用但没有一家完成核验:`suggested` 是建议核验的那一家。
    NeedsVerification {
        suggested: ProviderName,
        table: AggregateProviderSelection,
    },
    /// 命令可用,但候选都被真实拒绝。
    Denied { table: AggregateProviderSelection },
    /// provider health 探测降级/失败(拿不到可信的 L1 事实)。
    HealthDegraded { detail: String },
}

impl ProviderSelectionOutcome {
    /// 失败码(成功为 `None`)。
    pub fn reason_code(&self) -> Option<&'static str> {
        match self {
            Self::Selected(_) => None,
            Self::NoCliAvailable(_) => Some(REASON_CODE_NO_PROVIDER_AVAILABLE),
            Self::NeedsVerification { .. } => Some(REASON_CODE_PROVIDER_NEEDS_VERIFICATION),
            Self::Denied { .. } => Some(REASON_CODE_PROVIDER_CAPABILITY_DENIED),
            Self::HealthDegraded { .. } => Some(REASON_CODE_PROVIDER_HEALTH_DEGRADED),
        }
    }

    /// 候选表(健康降级时没有候选表)。
    pub fn table(&self) -> Option<&AggregateProviderSelection> {
        match self {
            Self::Selected(table) | Self::NoCliAvailable(table) => Some(table),
            Self::NeedsVerification { table, .. } | Self::Denied { table } => Some(table),
            Self::HealthDegraded { .. } => None,
        }
    }
}

/// 聚合冷启动的 provider 选择器。
///
/// 见模块文档:两层判据 + 与 gateway 同源。**不创建 operation**。
///
/// **选择判定与项目/逻辑代码库无关**:L2 读**机器级** capability 记录
/// (REQ-PCR-07,一次核验整机可用)⇒ 任意上下文的选择结论相同。
/// LC 作用域仍存在于 HTTP 层(404 守卫 / 失败面核验入口路径),不在此
/// 判定内;`project_id` 参数不参与判定(capability 记录无第二作用域)。
pub async fn select_aggregate_provider(
    paths: &ProductAppPaths,
    health: &ProviderHealthService,
    _project_id: &str,
) -> Result<ProviderSelectionOutcome, ProductStoreError> {
    select_aggregate_provider_at(paths, health, _project_id, Utc::now()).await
}

/// 选择器(显式给出「当前时刻」)。
///
/// 「当前时刻」显式化有两个理由:快照过期判定(10 分钟)与候选表 `decided_at`
/// 都必须以同一时刻为准;测试要能固定这一刻。
pub async fn select_aggregate_provider_at(
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
    // L2 读**机器级** capability 记录(REQ-PCR-07):一次核验 ⇒ 整机
    // (含本 LC)L2 放行 ⇒ 聚合初始化自动开始;不存在按项目/LC 读取
    // 的第二作用域分支。
    let source = StoreBackedProviderCapabilitySource::with_store(ProviderCapabilityStore::new(
        paths.clone(),
    ));
    decide_selection(&source, snapshot.as_ref(), now)
}

/// 候选表 + 结论(判定部分全部经 `launch_cell_state`,与 gateway 同源)。
fn decide_selection(
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
                        SessionPolicyAction::PlanningReadOnly,
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

/// health 快照是否需要刷新一次。
fn health_snapshot_needs_refresh(
    snapshot: &ProviderHealthSnapshot,
    degraded: bool,
    now: DateTime<Utc>,
) -> bool {
    snapshot.generation == 0
        || degraded
        || (now - snapshot.checked_at) > chrono::Duration::minutes(HEALTH_STALE_AFTER_MINUTES)
}

/// health 原因码的 wire 串(与枚举的 serde 形态同源,不另维护映射表)。
fn health_reason_code(code: ProviderHealthReasonCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

/// provider 展示名。
fn provider_display_name(provider: &ProviderName) -> &'static str {
    match provider {
        ProviderName::ClaudeCode => "Claude Code",
        ProviderName::Codex => "Codex",
        ProviderName::Pi => "Pi",
        ProviderName::KimiCode => "Kimi Code",
        ProviderName::Fake => "Fake",
    }
}

/// wire 名 → provider(未知串与 `Fake` 一律 `None`,fail-closed)。
pub fn provider_from_wire(value: &str) -> Option<ProviderName> {
    match value {
        "pi" => Some(ProviderName::Pi),
        "kimi_code" => Some(ProviderName::KimiCode),
        "claude_code" => Some(ProviderName::ClaudeCode),
        "codex" => Some(ProviderName::Codex),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;
    use crate::cross_cutting::aria_state_paths::AriaStatePaths;
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };
    use crate::cross_cutting::provider_boundary::PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN;
    use crate::cross_cutting::provider_health::ProviderHealthClock;
    use crate::product::logical_codebase::ProviderCapabilitySource;
    use crate::product::logical_codebase::policy::{
        ProviderDialect, ProviderWireDialect, SessionPolicyAction,
    };
    use crate::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
        ProviderActionMatrix,
    };
    use crate::product::logical_codebase::provider_gateway::{
        ProviderRefType, ResumeEvidenceState,
    };

    /// 假命令执行器:按可执行名决定「命令缺失」,并记录每次调用(用于计数刷新)。
    struct FakeRunner {
        missing: Vec<String>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeRunner {
        fn new(missing: Vec<String>) -> Arc<Self> {
            Arc::new(Self {
                missing,
                calls: Mutex::new(Vec::new()),
            })
        }

        fn call_count(&self) -> usize {
            self.calls.lock().expect("runner calls").len()
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for FakeRunner {
        async fn run(
            &self,
            request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            self.calls
                .lock()
                .expect("runner calls")
                .push(request.executable.clone());
            if self.missing.iter().any(|name| name == &request.executable) {
                return Err(BoundedCommandError::CommandMissing {
                    executable: request.executable.clone(),
                    details: "command not found".to_string(),
                });
            }
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
    struct FakeClock {
        now: DateTime<Utc>,
    }

    impl ProviderHealthClock for FakeClock {
        fn now(&self) -> DateTime<Utc> {
            self.now
        }
    }

    fn health_service(
        workspace_root: &Path,
        missing: Vec<String>,
        now: DateTime<Utc>,
    ) -> (ProviderHealthService, Arc<FakeRunner>) {
        let runner = FakeRunner::new(missing);
        let service = ProviderHealthService::with_dependencies(
            AriaStatePaths::from_workspace_root(workspace_root),
            runner.clone(),
            Arc::new(FakeClock { now }),
            Duration::from_secs(5),
            4096,
        );
        (service, runner)
    }

    fn adapter_dialect(provider: ProviderRefType) -> ProviderDialect {
        match provider {
            ProviderRefType::ClaudeCode => ProviderDialect::ClaudeCodeCliV1,
            ProviderRefType::Codex => ProviderDialect::CodexCliV1,
            ProviderRefType::Pi => ProviderDialect::PiRpcV1,
            ProviderRefType::KimiCode => ProviderDialect::KimiAcpV1,
        }
    }

    fn wire_dialect(provider: ProviderRefType) -> ProviderWireDialect {
        match provider {
            ProviderRefType::ClaudeCode => ProviderWireDialect::ClaudeCodeStreamJson,
            ProviderRefType::Codex => ProviderWireDialect::CodexAppServerRpc,
            ProviderRefType::Pi => ProviderWireDialect::PiRpc,
            ProviderRefType::KimiCode => ProviderWireDialect::KimiAcp,
        }
    }

    /// 构造指定 launch 分格的能力记录。
    fn capability_record(
        provider: ProviderRefType,
        launch: ProviderCapabilityEvidence,
    ) -> ProviderCapabilityRecord {
        ProviderCapabilityRecord {
            provider_type: provider,
            schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
            version: "1.0.0".to_string(),
            adapter_dialect: adapter_dialect(provider),
            wire_dialect: wire_dialect(provider),
            capability_snapshot_ref: MANAGED_CAPABILITY_SNAPSHOT_REF.to_string(),
            evidence: CapabilityEvidence::ProductionVerified,
            resume_evidence: ResumeEvidenceState::Confirmed,
            supported_actions: Vec::new(),
            action_matrix: ProviderActionMatrix::from_rows(vec![ProviderActionCapability {
                action: SessionPolicyAction::PlanningReadOnly,
                launch,
                resume: ProviderCapabilityEvidence::Unknown,
                write_boundary: ProviderCapabilityEvidence::Unknown,
                projection_digest: "projection-digest-planning".to_string(),
                evidence_ref: "probe://planning".to_string(),
            }])
            .expect("planning row"),
            trust: ProviderCapabilityEvidence::Confirmed,
            probed_at: Some("2026-11-03T00:00:00Z".to_string()),
            probe_artifact_ref: Some("probe://artifact".to_string()),
            execution_semantics: PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN.to_string(),
        }
    }

    /// 四场景用例：(场景名, 该机器根下要写的记录, 期望判定)。
    type LaunchCellCase = (
        &'static str,
        Option<Vec<(ProviderRefType, ProviderCapabilityEvidence)>>,
        LaunchCellState,
    );

    /// 把能力记录写进**机器级** capability store（REQ-PCR-07：唯一作用域 =
    /// 机器级 ⇒ 播种不再按项目/LC 分支）。
    ///
    /// 场景区分方式：不同场景用**不同的机器根**（各自 tempdir）或**不同记录
    /// 内容**区分，不再靠「不同 project_id / lc_id」区分。
    fn write_records(
        paths: &ProductAppPaths,
        rows: &[(ProviderRefType, ProviderCapabilityEvidence)],
    ) {
        let store = ProviderCapabilityStore::new(paths.clone());
        for (provider, launch) in rows {
            store
                .upsert(&capability_record(*provider, launch.clone()))
                .expect("upsert capability record");
        }
    }

    fn confirmed() -> ProviderCapabilityEvidence {
        ProviderCapabilityEvidence::Confirmed
    }

    fn unknown() -> ProviderCapabilityEvidence {
        ProviderCapabilityEvidence::Unknown
    }

    fn denied(reason: &str) -> ProviderCapabilityEvidence {
        ProviderCapabilityEvidence::denied(reason.to_string())
    }

    /// 断言「没有创建 operation」:聚合初始化根下不得出现任何 operation 文件。
    fn assert_no_operation_created(paths: &ProductAppPaths) {
        let root = paths.aggregate_initializations_root("project_0001");
        let created = match std::fs::read_dir(&root) {
            Ok(entries) => entries.count() > 0,
            Err(_) => false,
        };
        assert!(
            !created,
            "no operation must be created when no candidate is usable"
        );
    }

    #[tokio::test]
    async fn selection_picks_first_confirmed_candidate_in_owner_order() {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let now = Utc::now();
        let (health, _runner) = health_service(root.path(), Vec::new(), now);
        write_records(
            &paths,
            &[
                (ProviderRefType::Pi, confirmed()),
                (ProviderRefType::KimiCode, confirmed()),
                (ProviderRefType::ClaudeCode, confirmed()),
                (ProviderRefType::Codex, confirmed()),
            ],
        );

        let outcome = select_aggregate_provider_at(&paths, &health, "project_0001", now)
            .await
            .expect("selection");
        let ProviderSelectionOutcome::Selected(table) = outcome else {
            panic!("expected a selected provider, got {outcome:?}");
        };
        assert_eq!(table.selected, "pi");
        assert_eq!(
            table
                .candidates
                .iter()
                .map(|row| row.provider.clone())
                .collect::<Vec<_>>(),
            vec!["pi", "kimi_code", "claude_code", "codex"]
        );
        assert_eq!(table.candidates[0].capability_state, "confirmed");
        assert!(table.candidates[0].skipped_reason.is_none());
    }

    /// Task 3(读取点迁移):capability 读取只认**机器级**记录(REQ-PCR-07
    /// 「读取只读机器级」)。机器级路径种一条 Confirmed(全程不创建任何
    /// 项目/LC)⇒ 选择器读到 Confirmed;项目级/LC 级路径(子树真实存在)
    /// 下没有任何记录 ⇒ 判未核验——不存在「机器级无记录时回退扫描其它
    /// 作用域」的读取分支。构造接口不再携带 project_id 作用域参数
    /// (capability 记录作用域唯一 = 机器级)。
    #[tokio::test]
    async fn selection_reads_machine_level_capability() {
        // —— 场景一:机器级种 Confirmed;不创建任何项目/LC ——
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let now = Utc::now();
        let (health, _runner) = health_service(root.path(), Vec::new(), now);
        write_records(&paths, &[(ProviderRefType::Pi, confirmed())]);

        // 读取点(新接口):机器级 store 直接装配,无 project_id 参数;
        // 判定仍走选择器与 gateway 唯一共用的只读判定函数 launch_cell_state。
        let source = StoreBackedProviderCapabilitySource::with_store(ProviderCapabilityStore::new(
            paths.clone(),
        ));
        let provider_ref =
            ProviderRef::from_provider_name(&ProviderName::Pi, MANAGED_CAPABILITY_SNAPSHOT_REF)
                .expect("pi provider ref");
        assert!(
            matches!(
                launch_cell_state(
                    &source,
                    &provider_ref,
                    SessionPolicyAction::PlanningReadOnly
                ),
                Ok(LaunchCellState::Confirmed)
            ),
            "machine-level Confirmed record must read Confirmed without any project/lc"
        );

        // 选择器端到端:同一机器级事实 ⇒ Selected(Pi);项目/LC 是否存在
        // 与结论无关(本场景根本没有建)。
        let outcome = select_aggregate_provider_at(&paths, &health, "project_0001", now)
            .await
            .expect("selection");
        assert!(
            matches!(outcome, ProviderSelectionOutcome::Selected(_)),
            "selector must read the machine-level Confirmed record"
        );

        // —— 场景二:项目级/LC 级路径下没有记录 ⇒ 判未核验 ——
        let empty = tempfile::tempdir().expect("empty product root");
        let empty_paths = ProductAppPaths::new(empty.path().join(".aria"));
        // 项目级/LC 级子树真实存在(否则「不存在」是空断言),但机器级
        // capability 文件不存在。
        std::fs::create_dir_all(
            empty_paths
                .logical_codebase_root("project_0001")
                .join("lc_0001"),
        )
        .expect("project/lc subtree dirs");
        let empty_source = StoreBackedProviderCapabilitySource::new(empty_paths);
        match launch_cell_state(
            &empty_source,
            &provider_ref,
            SessionPolicyAction::PlanningReadOnly,
        )
        .expect("launch cell state")
        {
            LaunchCellState::Unverified { .. } => {}
            other => panic!(
                "no machine-level record must read unverified (no fallback scope), got {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn selection_skips_provider_with_missing_command_and_picks_next() {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let now = Utc::now();
        let (health, _runner) = health_service(root.path(), vec!["pi".to_string()], now);
        write_records(
            &paths,
            &[
                (ProviderRefType::Pi, confirmed()),
                (ProviderRefType::KimiCode, confirmed()),
                (ProviderRefType::ClaudeCode, confirmed()),
                (ProviderRefType::Codex, confirmed()),
            ],
        );

        let outcome = select_aggregate_provider_at(&paths, &health, "project_0001", now)
            .await
            .expect("selection");
        let ProviderSelectionOutcome::Selected(table) = outcome else {
            panic!("expected a selected provider, got {outcome:?}");
        };
        assert_eq!(table.selected, "kimi_code");
        assert!(!table.candidates[0].cli_available);
        assert_eq!(
            table.candidates[0].cli_reason_code.as_deref(),
            Some("command_missing")
        );
        assert_eq!(
            table.candidates[0].skipped_reason.as_deref(),
            Some("cli_unavailable")
        );
        assert_ne!(
            table.candidates[0].skipped_reason.as_deref(),
            Some("needs_verification")
        );
        assert_eq!(table.candidates[0].capability_state, "not_installed");
        assert!(!table.candidates[0].action_hint.is_empty());
    }

    #[tokio::test]
    async fn selection_reports_no_cli_available_when_every_command_is_missing() {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let now = Utc::now();
        let (health, _runner) = health_service(
            root.path(),
            vec![
                "pi".to_string(),
                "kimi".to_string(),
                "claude".to_string(),
                "codex".to_string(),
            ],
            now,
        );
        write_records(
            &paths,
            &[
                (ProviderRefType::Pi, confirmed()),
                (ProviderRefType::KimiCode, confirmed()),
                (ProviderRefType::ClaudeCode, confirmed()),
                (ProviderRefType::Codex, confirmed()),
            ],
        );

        let outcome = select_aggregate_provider_at(&paths, &health, "project_0001", now)
            .await
            .expect("selection");
        assert_eq!(
            outcome.reason_code(),
            Some(REASON_CODE_NO_PROVIDER_AVAILABLE)
        );
        let ProviderSelectionOutcome::NoCliAvailable(table) = outcome else {
            panic!("expected no cli available, got {outcome:?}");
        };
        assert_eq!(table.selected, "");
        assert!(
            table
                .candidates
                .iter()
                .all(|row| row.capability_state == "not_installed")
        );
        assert_no_operation_created(&paths);
    }

    #[tokio::test]
    async fn selection_needs_verification_without_creating_operation() {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let now = Utc::now();
        let (health, _runner) = health_service(root.path(), Vec::new(), now);
        write_records(
            &paths,
            &[
                (ProviderRefType::Pi, unknown()),
                (ProviderRefType::KimiCode, unknown()),
                (ProviderRefType::ClaudeCode, unknown()),
                (ProviderRefType::Codex, unknown()),
            ],
        );

        let outcome = select_aggregate_provider_at(&paths, &health, "project_0001", now)
            .await
            .expect("selection");
        assert_eq!(
            outcome.reason_code(),
            Some(REASON_CODE_PROVIDER_NEEDS_VERIFICATION)
        );
        let ProviderSelectionOutcome::NeedsVerification { suggested, table } = outcome else {
            panic!("expected needs verification, got {outcome:?}");
        };
        assert_eq!(suggested, ProviderName::Pi);
        assert_eq!(table.selected, "");
        assert!(
            table
                .candidates
                .iter()
                .all(|row| row.capability_state == "unverified")
        );
        assert_no_operation_created(&paths);
    }

    #[tokio::test]
    async fn selection_distinguishes_denied_from_unverified_candidates() {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let now = Utc::now();
        let (health, _runner) = health_service(root.path(), Vec::new(), now);
        write_records(
            &paths,
            &[
                (ProviderRefType::Pi, denied("pi refuses this action")),
                (ProviderRefType::KimiCode, unknown()),
                (ProviderRefType::ClaudeCode, unknown()),
                (ProviderRefType::Codex, unknown()),
            ],
        );

        let outcome = select_aggregate_provider_at(&paths, &health, "project_0001", now)
            .await
            .expect("selection");
        let ProviderSelectionOutcome::NeedsVerification { suggested, table } = outcome else {
            panic!("expected needs verification, got {outcome:?}");
        };
        assert_eq!(suggested, ProviderName::KimiCode);
        assert_eq!(table.candidates[0].capability_state, "denied");
        assert_eq!(
            table.candidates[0].warning.as_deref(),
            Some("pi refuses this action")
        );
        assert_eq!(table.candidates[1].capability_state, "unverified");
    }

    #[tokio::test]
    async fn selection_reports_health_degraded_without_creating_operation() {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let now = Utc::now();
        // 工作区根是一个文件:health 快照无法落盘 → 刷新失败 → 降级。
        let blocked = root.path().join("blocked");
        std::fs::write(&blocked, "not a directory").expect("write blocking file");
        let (health, _runner) = health_service(&blocked, Vec::new(), now);

        let outcome = select_aggregate_provider_at(&paths, &health, "project_0001", now)
            .await
            .expect("selection");
        assert_eq!(
            outcome.reason_code(),
            Some(REASON_CODE_PROVIDER_HEALTH_DEGRADED)
        );
        let ProviderSelectionOutcome::HealthDegraded { detail } = outcome else {
            panic!("expected health degraded, got {outcome:?}");
        };
        assert!(!detail.is_empty());
        assert_no_operation_created(&paths);
    }

    #[tokio::test]
    async fn selection_refreshes_health_only_when_snapshot_is_stale_or_missing() {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let now = Utc::now();
        let (health, runner) = health_service(root.path(), Vec::new(), now);
        write_records(
            &paths,
            &[
                (ProviderRefType::Pi, confirmed()),
                (ProviderRefType::KimiCode, confirmed()),
                (ProviderRefType::ClaudeCode, confirmed()),
                (ProviderRefType::Codex, confirmed()),
            ],
        );

        // 第一次:generation == 0 → 刷新一次(四家各一次版本探测)。
        select_aggregate_provider_at(&paths, &health, "project_0001", now)
            .await
            .expect("first selection");
        let after_first = runner.call_count();
        assert_eq!(after_first, 4);

        // 第二次:快照新鲜 → 不刷新。
        select_aggregate_provider_at(&paths, &health, "project_0001", now)
            .await
            .expect("second selection");
        assert_eq!(runner.call_count(), after_first);

        // 第三次:快照超过 10 分钟 → 刷新一次。
        let stale = now + chrono::Duration::minutes(11);
        select_aggregate_provider_at(&paths, &health, "project_0001", stale)
            .await
            .expect("third selection");
        assert_eq!(runner.call_count(), after_first * 2);
    }

    /// 同源锁:选择器的判定与 gateway 的判定结论必须一致,且只有 `Confirmed` 放行。
    #[test]
    fn launch_cell_state_and_require_supported_agree_on_every_input() {
        // 每种输入一个独立**机器根**（各自 tempdir），互不干扰。
        //
        // capability 记录的唯一作用域是机器级(REQ-PCR-07) ⇒ 场景区分方式
        // 既不能靠「不同 lc_id」也不能靠「不同 project_id」（记录不再按项目
        // 分叉），只能靠**不同机器根 + 不同记录内容**。
        //
        // 「record missing」场景：不写任何记录 → 记录缺失。
        let cases: Vec<LaunchCellCase> = vec![
            (
                "confirmed",
                Some(vec![(ProviderRefType::Pi, confirmed())]),
                LaunchCellState::Confirmed,
            ),
            (
                "denied",
                Some(vec![(
                    ProviderRefType::Pi,
                    denied("pi refuses this action"),
                )]),
                LaunchCellState::Denied {
                    reason: "pi refuses this action".to_string(),
                },
            ),
            (
                "unknown",
                Some(vec![(ProviderRefType::Pi, unknown())]),
                LaunchCellState::Unverified {
                    detail: "PlanningReadOnly launch cell is unknown".to_string(),
                },
            ),
            (
                "record missing",
                None,
                LaunchCellState::Unverified {
                    detail:
                        "provider_capability_launch_not_confirmed: Pi capability record missing"
                            .to_string(),
                },
            ),
        ];

        for (name, rows, expected) in cases {
            let root = tempfile::tempdir().expect("temporary product root");
            let paths = ProductAppPaths::new(root.path().join(".aria"));
            if let Some(rows) = rows {
                write_records(&paths, &rows);
            }
            let source = StoreBackedProviderCapabilitySource::with_store(
                ProviderCapabilityStore::new(paths.clone()),
            );
            let provider_ref = ProviderRef::pi(MANAGED_CAPABILITY_SNAPSHOT_REF);
            let state = launch_cell_state(
                &source,
                &provider_ref,
                SessionPolicyAction::PlanningReadOnly,
            )
            .expect("launch cell state");
            assert_eq!(state, expected, "state mismatch for {name}");
            let gateway =
                source.require_supported(&provider_ref, SessionPolicyAction::PlanningReadOnly);
            assert_eq!(
                gateway.is_ok(),
                expected.is_confirmed(),
                "gateway conclusion mismatch for {name}: {gateway:?}"
            );
            if let Err(error) = gateway {
                assert!(
                    matches!(&error, ProviderGatewayError::UnsupportedCapability(_)),
                    "unexpected gateway error for {name}: {error:?}"
                );
            }
        }

        // 快照引用不一致:选择器判 `EvidenceStale`,gateway 拒绝。
        //
        // 区分方式:读**同一个机器根**的记录,但用**不同的快照引用**去问它。
        let snapshot_root = tempfile::tempdir().expect("temporary product root");
        let snapshot_paths = ProductAppPaths::new(snapshot_root.path().join(".aria"));
        write_records(&snapshot_paths, &[(ProviderRefType::Pi, confirmed())]);
        let source = StoreBackedProviderCapabilitySource::with_store(ProviderCapabilityStore::new(
            snapshot_paths.clone(),
        ));
        let mismatched_ref = ProviderRef::pi("cap_other_snapshot");
        let state = launch_cell_state(
            &source,
            &mismatched_ref,
            SessionPolicyAction::PlanningReadOnly,
        )
        .expect("launch cell state");
        assert_eq!(
            state,
            LaunchCellState::EvidenceStale {
                detail: "capability snapshot mismatch".to_string()
            }
        );
        assert!(
            source
                .require_supported(&mismatched_ref, SessionPolicyAction::PlanningReadOnly)
                .is_err()
        );
    }

    #[test]
    fn provider_from_wire_rejects_unknown_and_fake() {
        assert_eq!(provider_from_wire("pi"), Some(ProviderName::Pi));
        assert_eq!(
            provider_from_wire("kimi_code"),
            Some(ProviderName::KimiCode)
        );
        assert_eq!(
            provider_from_wire("claude_code"),
            Some(ProviderName::ClaudeCode)
        );
        assert_eq!(provider_from_wire("codex"), Some(ProviderName::Codex));
        assert_eq!(provider_from_wire("fake"), None);
        assert_eq!(provider_from_wire("unknown-provider"), None);
        assert_eq!(provider_from_wire(""), None);
    }

    #[test]
    fn launch_rejection_error_keeps_prefix_and_real_reason() {
        let unknown = launch_rejection_error(
            SessionPolicyAction::PlanningReadOnly,
            &LaunchCellState::Unverified {
                detail: "detail".to_string(),
            },
        );
        assert!(matches!(
            &unknown,
            ProviderGatewayError::UnsupportedCapability(reason)
                if reason.starts_with(PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED)
                    && reason.contains("launch cell is unknown")
        ));

        let denied = launch_rejection_error(
            SessionPolicyAction::PlanningReadOnly,
            &LaunchCellState::Denied {
                reason: "pi refuses this action".to_string(),
            },
        );
        assert!(matches!(
            &denied,
            ProviderGatewayError::UnsupportedCapability(reason)
                if reason.starts_with(PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED)
                    && reason.contains("launch cell is denied; reason: pi refuses this action")
        ));
    }
}
