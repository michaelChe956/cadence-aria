//! Provider capability 用户显式重验证通道（add-provider-revalidate-probe）。
//!
//! 职责（§0 契约「只加一条」）：用户点击 Revalidate → 对该 provider 执行
//! 一次真实现场边界探针（复用 6c `run_cli_boundary_probe`，覆盖
//! Coding/Planning/Review 三个 action 含 resume 面）→ 经 2d
//! `record_verified_probe` 原子导入 durable Confirmed（与 gateway 消费的
//! capability store 同一 LC 作用域）。一次性语义：action 行已在当前 CLI
//! exact version 上全 Confirmed 时秒回不重跑；不点击不运行任何探针；探针
//! 失败如实上报，绝不伪造 Confirmed。门本身（admission fail-closed）不在
//! 本模块改动范围。

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::cross_cutting::provider_boundary::ProviderBoundaryError;
use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::policy::SessionPolicyAction;
use crate::product::logical_codebase::provider_boundary_probe::{
    CliBoundaryProbeOutcome, ResumeChannelKind, ResumeProbeSpec, probe_cli_version,
    run_cli_boundary_probe,
};
use crate::product::logical_codebase::provider_capability_probe::ProviderCapabilityProbeService;
use crate::product::logical_codebase::provider_capability_store::{
    ProviderActionCapability, ProviderCapabilityRecord, ProviderCapabilityStore,
};
use crate::product::logical_codebase::provider_gateway::ProviderRefType;
use crate::product::models::ProviderName;

/// 重验证覆盖的 action 全集（与矩阵 harness 同覆盖：Coding/Planning/
/// Review；含 resume 面——story/design 门上的 require_resume_supported 由
/// 探针工件签发 resume=Confirmed 后才可达）。
pub const REVALIDATE_ACTIONS: [SessionPolicyAction; 3] = [
    SessionPolicyAction::CodingTargetWrite,
    SessionPolicyAction::PlanningReadOnly,
    SessionPolicyAction::ReviewReadOnly,
];

/// resume 探针的单轮提示词（真机 LLM 轮次，期望极短应答）。
const REVALIDATE_RESUME_PROMPT: &str = "Reply with exactly: aria-revalidate-probe";

/// 四家真实 provider 的探针通道冻结映射（CLI 程序名 + resume 通道形态；
/// Fake 无通道——不冒充探测）。
pub fn provider_probe_channel(provider: ProviderName) -> Option<(&'static str, ResumeChannelKind)> {
    match provider {
        ProviderName::ClaudeCode => Some(("claude", ResumeChannelKind::ClaudePrintJson)),
        ProviderName::Codex => Some(("codex", ResumeChannelKind::CodexExecJson)),
        ProviderName::Pi => Some(("pi", ResumeChannelKind::PiSessionId)),
        ProviderName::KimiCode => Some(("kimi", ResumeChannelKind::KimiStreamJson)),
        ProviderName::Fake => None,
    }
}

/// ProviderRefType → ProviderName（capability 面的四家冻结映射）。
pub fn provider_name_for_type(provider_type: ProviderRefType) -> ProviderName {
    match provider_type {
        ProviderRefType::ClaudeCode => ProviderName::ClaudeCode,
        ProviderRefType::Codex => ProviderName::Codex,
        ProviderRefType::Pi => ProviderName::Pi,
        ProviderRefType::KimiCode => ProviderName::KimiCode,
    }
}

/// `SessionPolicyAction` 的稳定文本（与探针/前端 action 名同口径）。
pub fn revalidate_action_text(action: SessionPolicyAction) -> &'static str {
    match action {
        SessionPolicyAction::PlanningReadOnly => "planning_read_only",
        SessionPolicyAction::CodingTargetWrite => "coding_target_write",
        SessionPolicyAction::ReviewReadOnly => "review_read_only",
    }
}

/// 真实探针调用的完整材料包（seam 注入面：生产实现=run_cli_boundary_probe）。
#[derive(Debug, Clone)]
pub struct ProbeInvocation {
    pub provider: ProviderName,
    pub cli_program: String,
    pub action: SessionPolicyAction,
    /// 受控 fixture 基目录（生产=每次点击一个 /tmp 族 TempDir；
    /// root/member/home/target 全在其内、按调用隔离）。
    pub base: PathBuf,
    /// 证据落盘根（生产=LC 子树 `capability-evidence/boundary`，durable）。
    pub evidence_root: PathBuf,
    pub session_label: String,
    pub resume: Option<ResumeProbeSpec>,
}

pub type ProbeFuture =
    Pin<Box<dyn Future<Output = Result<CliBoundaryProbeOutcome, ProviderBoundaryError>> + Send>>;
pub type ProbeRunner = Arc<dyn Fn(ProbeInvocation) -> ProbeFuture + Send + Sync>;
pub type VersionSource = Arc<dyn Fn(&str) -> Result<String, ProviderBoundaryError> + Send + Sync>;

/// 重验证稳定错误面：
/// - `Unsupported`：Fake/无探针通道 provider（HTTP 稳定码
///   `provider_capability_probe_unsupported`）；
/// - `ProbeFailed`：版本探测/边界探针/导入任一环节失败关闭（HTTP 稳定码
///   `provider_capability_probe_failed`，detail 携带 action 与原始错误）；
/// - `Store`：底层存储错误（走既有 product_store_api_error）。
#[derive(Debug)]
pub enum RevalidateError {
    Unsupported {
        provider: String,
    },
    ProbeFailed {
        action: Option<SessionPolicyAction>,
        detail: String,
    },
    Store(ProductStoreError),
}

impl From<ProductStoreError> for RevalidateError {
    fn from(error: ProductStoreError) -> Self {
        Self::Store(error)
    }
}

/// 重验证结果：全行已验证秒回（durable 零变化）或本次完成探测+导入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevalidateOutcome {
    AlreadyConfirmed {
        version: String,
    },
    Revalidated {
        version: String,
        imported_actions: Vec<SessionPolicyAction>,
    },
}

/// 只读 capability 状态投影（GET 面）：逐 provider 的 durable 事实。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilityActionRowSnapshot {
    pub action: SessionPolicyAction,
    pub launch: ProviderCapabilityEvidence,
    pub resume: ProviderCapabilityEvidence,
    pub write_boundary: ProviderCapabilityEvidence,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilityProviderSnapshot {
    pub provider_type: ProviderRefType,
    pub cli_program: &'static str,
    pub version: Option<String>,
    pub rows: Vec<ProviderCapabilityActionRowSnapshot>,
    pub probed_at: Option<String>,
    pub probe_artifact_ref: Option<String>,
}

/// capability 重验证服务（产品层 owner；web 层只做 HTTP 面）。
#[derive(Clone)]
pub struct ProviderCapabilityRevalidateService {
    paths: ProductAppPaths,
    version_source: VersionSource,
    probe_runner: ProbeRunner,
}

/// probe_and_import 的作用域参数包(clippy too_many_arguments 收敛;
/// 生命周期跟 revalidate 调用栈)。
struct ProbeScope<'a> {
    project_id: &'a str,
    lc_id: &'a str,
    provider: ProviderName,
    cli_program: &'a str,
    resume_kind: ResumeChannelKind,
    base: &'a std::path::Path,
}

impl ProviderCapabilityRevalidateService {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self {
            paths,
            version_source: Arc::new(probe_cli_version),
            probe_runner: Arc::new(|invocation: ProbeInvocation| {
                let ProbeInvocation {
                    provider,
                    cli_program,
                    action,
                    base,
                    evidence_root,
                    session_label,
                    resume,
                } = invocation;
                Box::pin(async move {
                    run_cli_boundary_probe(
                        provider,
                        &cli_program,
                        action,
                        &base,
                        &evidence_root,
                        &session_label,
                        resume,
                    )
                    .await
                })
            }),
        }
    }

    /// 测试 seam：注入 CLI 版本源（默认 `cli --version` 真实探测）。
    pub fn with_version_source(mut self, source: VersionSource) -> Self {
        self.version_source = source;
        self
    }

    /// 测试 seam：注入探针 runner（默认真实 `run_cli_boundary_probe`）。
    pub fn with_probe_runner(mut self, runner: ProbeRunner) -> Self {
        self.probe_runner = runner;
        self
    }

    /// 用户显式重验证：对该 provider 执行真实现场探针并原子导入 durable
    /// Confirmed。语义见模块注释（版本钉定幂等/失败如实/不点不跑）。
    ///
    /// 幂等与失败语义：全行 Confirmed@当前版本 → `AlreadyConfirmed`（零
    /// 探针）；否则仅对缺失/漂移 action 逐个探测+导入，任一失败立即返回
    /// `ProbeFailed`（同一请求中先成功的导入保留——durable 如实反映已验
    /// 证事实，不回滚、不伪造）。
    pub async fn revalidate(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: ProviderName,
    ) -> Result<RevalidateOutcome, RevalidateError> {
        let Some((cli_program, resume_kind)) = provider_probe_channel(provider.clone()) else {
            return Err(RevalidateError::Unsupported {
                provider: format!("{provider:?}"),
            });
        };
        // 版本探测（一次 `cli --version`）：CLI 缺失即如实失败，先于任何
        // 探针会话。
        let current_version =
            (self.version_source)(cli_program).map_err(|error| RevalidateError::ProbeFailed {
                action: None,
                detail: error.to_string(),
            })?;
        let provider_type = provider_type_for_name(&provider);
        let store = ProviderCapabilityStore::for_lc(self.paths.clone(), lc_id);
        let record = store.get(project_id, provider_type)?;
        let missing: Vec<SessionPolicyAction> = REVALIDATE_ACTIONS
            .iter()
            .copied()
            .filter(|action| !row_confirmed_at_version(record.as_ref(), *action, &current_version))
            .collect();
        if missing.is_empty() {
            return Ok(RevalidateOutcome::AlreadyConfirmed {
                version: current_version,
            });
        }
        // 受控 fixture：每次点击一个 /tmp 族独立目录（按调用隔离，uuid
        // 命名；与 repository_store 登记先例同构，不引运行时新依赖），
        // 探测结束 best-effort 清理；证据 durable 落 LC 子树。
        let base = probe_base_dir()?;
        let result = self
            .probe_and_import(
                ProbeScope {
                    project_id,
                    lc_id,
                    provider,
                    cli_program,
                    resume_kind,
                    base: &base,
                },
                &missing,
            )
            .await;
        let _ = std::fs::remove_dir_all(&base);
        result.map(|imported_actions| RevalidateOutcome::Revalidated {
            version: current_version,
            imported_actions,
        })
    }

    /// 逐缺失/漂移 action 执行真探针并导入(失败即返回,先成功导入保留)。
    async fn probe_and_import(
        &self,
        scope: ProbeScope<'_>,
        missing: &[SessionPolicyAction],
    ) -> Result<Vec<SessionPolicyAction>, RevalidateError> {
        let ProbeScope {
            project_id,
            lc_id,
            provider,
            cli_program,
            resume_kind,
            base,
        } = scope;
        let evidence_root = capability_evidence_root(&self.paths, project_id, lc_id)?;
        let mut imported_actions = Vec::new();
        for action in missing {
            let session_label = format!(
                "revalidate-{cli_program}-{}-{}",
                revalidate_action_text(*action),
                chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
            );
            let invocation = ProbeInvocation {
                provider: provider.clone(),
                cli_program: cli_program.to_string(),
                action: *action,
                base: base.to_path_buf(),
                evidence_root: evidence_root.clone(),
                session_label,
                resume: Some(ResumeProbeSpec::new(resume_kind, REVALIDATE_RESUME_PROMPT)),
            };
            let outcome = (self.probe_runner)(invocation).await.map_err(|error| {
                RevalidateError::ProbeFailed {
                    action: Some(*action),
                    detail: error.to_string(),
                }
            })?;
            self.import_outcome(project_id, lc_id, &outcome)?;
            imported_actions.push(*action);
        }
        Ok(imported_actions)
    }

    /// 只读 capability 状态投影（零写入、零探针触发；bootstrap 默认记录
    /// 不产生真实 Confirmed 显示）。
    pub fn capability_snapshot(
        &self,
        project_id: &str,
        lc_id: &str,
    ) -> Result<Vec<ProviderCapabilityProviderSnapshot>, ProductStoreError> {
        let store = ProviderCapabilityStore::for_lc(self.paths.clone(), lc_id);
        let mut snapshots = Vec::new();
        for provider_type in [
            ProviderRefType::ClaudeCode,
            ProviderRefType::Codex,
            ProviderRefType::Pi,
            ProviderRefType::KimiCode,
        ] {
            let provider = provider_name_for_type(provider_type);
            let cli_program = provider_probe_channel(provider)
                .map(|(program, _)| program)
                .unwrap_or("");
            let record = store.get(project_id, provider_type)?;
            let rows = REVALIDATE_ACTIONS
                .iter()
                .map(|action| {
                    let row = record
                        .as_ref()
                        .map(|record| record.action_matrix.row(action))
                        .unwrap_or_else(|| ProviderActionCapability::unknown(*action));
                    ProviderCapabilityActionRowSnapshot {
                        action: *action,
                        launch: row.launch,
                        resume: row.resume,
                        write_boundary: row.write_boundary,
                        evidence_ref: row.evidence_ref,
                    }
                })
                .collect();
            snapshots.push(ProviderCapabilityProviderSnapshot {
                provider_type,
                cli_program,
                version: record.as_ref().map(|record| record.version.clone()),
                rows,
                probed_at: record.as_ref().and_then(|record| record.probed_at.clone()),
                probe_artifact_ref: record
                    .as_ref()
                    .and_then(|record| record.probe_artifact_ref.clone()),
            });
        }
        Ok(snapshots)
    }

    /// 把三方一致的探针结果导入 durable（writer 装配=与 gateway 读子树
    /// 同一 `lc_scope_root` 派生的 capability store，构造保证同源）。
    fn import_outcome(
        &self,
        project_id: &str,
        lc_id: &str,
        outcome: &CliBoundaryProbeOutcome,
    ) -> Result<(), ProductStoreError> {
        let durable = ProviderCapabilityProbeService::with_durable_writer(
            ProviderCapabilityStore::for_lc(self.paths.clone(), lc_id),
        );
        durable.record_verified_probe(
            project_id,
            &outcome.record,
            &outcome.evidence,
            &outcome.projection,
        )
    }
}

/// ProviderName → ProviderRefType（有探针通道的四家；Fake 不可达——
/// `revalidate` 入口已按通道拒绝）。
fn provider_type_for_name(provider: &ProviderName) -> ProviderRefType {
    match provider {
        ProviderName::ClaudeCode => ProviderRefType::ClaudeCode,
        ProviderName::Codex => ProviderRefType::Codex,
        ProviderName::Pi => ProviderRefType::Pi,
        ProviderName::KimiCode => ProviderRefType::KimiCode,
        ProviderName::Fake => unreachable!("fake has no probe channel"),
    }
}

/// 行级 fast-path 判定：记录版本与当前 CLI 一致且 launch/resume/
/// write_boundary 全 Confirmed 才视为已验证（旧版本 Confirmed 不跨版本
/// 沿用）。
fn row_confirmed_at_version(
    record: Option<&ProviderCapabilityRecord>,
    action: SessionPolicyAction,
    current_version: &str,
) -> bool {
    match record {
        Some(record) if record.version == current_version => {
            let row = record.action_matrix.row(&action);
            row.launch == ProviderCapabilityEvidence::Confirmed
                && row.resume == ProviderCapabilityEvidence::Confirmed
                && row.write_boundary == ProviderCapabilityEvidence::Confirmed
        }
        _ => false,
    }
}

/// 探针证据的 durable 落盘根：LC 子树 `capability-evidence/boundary`
/// （与 capabilities.json 同一 `lc_scope_root`，可审计）。
fn capability_evidence_root(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: &str,
) -> Result<PathBuf, ProductStoreError> {
    Ok(crate::product::logical_codebase::store::lc_scope_root(
        paths,
        project_id,
        &Some(lc_id.to_string()),
    )?
    .join("capability-evidence")
    .join("boundary"))
}

/// 受控 fixture 基目录：`$TMPDIR/aria-capability-revalidate-<uuid>`（按
/// 调用隔离；创建失败如实上抛，绝不落到非临时位置）。
fn probe_base_dir() -> Result<PathBuf, RevalidateError> {
    let base = std::env::temp_dir().join(format!(
        "aria-capability-revalidate-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&base).map_err(|error| RevalidateError::ProbeFailed {
        action: None,
        detail: format!("probe base dir {}: {error}", base.display()),
    })?;
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::cross_cutting::provider_boundary::{ProviderBoundaryEvidence, ProviderBoundaryMode};
    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
    use crate::product::logical_codebase::policy::{
        PolicyTarget, ProviderDialect, ProviderWireDialect,
    };
    use crate::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
        ProviderActionMatrix, RootRecipeEvidence,
    };
    use crate::product::logical_codebase::provider_gateway::ResumeEvidenceState;
    use crate::product::logical_codebase::provider_projection::ProviderPolicyProjection;
    use crate::protocol::contracts::AdapterRole;
    use tempfile::tempdir;

    const PROJECT_ID: &str = "project_0001";
    const PROBE_VERSION: &str = "1.42.0";

    fn paths_fixture() -> (ProductAppPaths, tempfile::TempDir) {
        let root = tempdir().expect("root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        (paths, root)
    }

    fn lc_id_fixture() -> String {
        format!("lc_{}", uuid::Uuid::new_v4().simple())
    }

    fn digest64(seed: u8) -> String {
        let hex: String = (0..64)
            .map(|i| char::from_digit((u32::from(seed) + i) % 16, 16).expect("hex digit"))
            .collect();
        format!("sha256:{hex}")
    }

    /// 三方一致、launch/resume/write_boundary 全 Confirmed 的 Codex 材料
    /// 包（真实探针 outcome 的同构替身；boundary 模式按 action 语义）。
    fn codex_confirmed_outcome(
        action: SessionPolicyAction,
        digest_seed: u8,
    ) -> CliBoundaryProbeOutcome {
        let digest = digest64(digest_seed);
        let artifact_ref = format!("probe://boundary/codex/{digest_seed:04}");
        let probed_at = "2026-10-10T00:00:00Z".to_string();
        let mode = match action {
            SessionPolicyAction::CodingTargetWrite => ProviderBoundaryMode::TargetWriteOnly,
            SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => {
                ProviderBoundaryMode::ReadOnly
            }
        };
        let evidence = ProviderBoundaryEvidence::new(
            ProviderName::Codex,
            PROBE_VERSION.to_string(),
            mode,
            digest.clone(),
            artifact_ref.clone(),
            probed_at.clone(),
        );
        let projection = ProviderPolicyProjection::new(
            ProviderRefType::Codex,
            ProviderDialect::CodexCliV1,
            ProviderWireDialect::CodexAppServerRpc,
            PROBE_VERSION.to_string(),
            action,
            AdapterRole::Executor,
            ProviderPermissionMode::Auto,
            None,
            "never".to_string(),
            "workspace-write".to_string(),
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
            digest.clone(),
        );
        let row = ProviderActionCapability {
            action,
            launch: ProviderCapabilityEvidence::Confirmed,
            resume: ProviderCapabilityEvidence::Confirmed,
            write_boundary: ProviderCapabilityEvidence::Confirmed,
            projection_digest: digest,
            evidence_ref: artifact_ref.clone(),
        };
        let record = ProviderCapabilityRecord {
            provider_type: ProviderRefType::Codex,
            schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
            version: PROBE_VERSION.to_string(),
            adapter_dialect: ProviderDialect::CodexCliV1,
            wire_dialect: ProviderWireDialect::CodexAppServerRpc,
            capability_snapshot_ref: "cap_managed_snapshot".to_string(),
            evidence: CapabilityEvidence::ProductionVerified,
            resume_evidence: ResumeEvidenceState::Confirmed,
            supported_actions: Vec::new(),
            action_matrix: ProviderActionMatrix::from_rows(vec![row]).expect("唯一 action 行"),
            trust: ProviderCapabilityEvidence::Unknown,
            probed_at: Some(probed_at),
            probe_artifact_ref: Some(artifact_ref),
            root_recipe_evidence: RootRecipeEvidence::None,
        };
        CliBoundaryProbeOutcome {
            evidence,
            projection,
            record,
        }
    }

    /// 已全 Confirmed（指定版本）的 durable 记录（fast-path 种子）。
    fn fully_confirmed_record(
        version: &str,
        actions: &[SessionPolicyAction],
    ) -> ProviderCapabilityRecord {
        let rows = actions
            .iter()
            .map(|action| ProviderActionCapability {
                action: *action,
                launch: ProviderCapabilityEvidence::Confirmed,
                resume: ProviderCapabilityEvidence::Confirmed,
                write_boundary: ProviderCapabilityEvidence::Confirmed,
                projection_digest: digest64(3),
                evidence_ref: "probe://boundary/codex/seed".to_string(),
            })
            .collect();
        ProviderCapabilityRecord {
            provider_type: ProviderRefType::Codex,
            schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
            version: version.to_string(),
            adapter_dialect: ProviderDialect::CodexCliV1,
            wire_dialect: ProviderWireDialect::CodexAppServerRpc,
            capability_snapshot_ref: "cap_managed_snapshot".to_string(),
            evidence: CapabilityEvidence::ProductionVerified,
            resume_evidence: ResumeEvidenceState::Confirmed,
            supported_actions: Vec::new(),
            action_matrix: ProviderActionMatrix::from_rows(rows).expect("action 行"),
            trust: ProviderCapabilityEvidence::Unknown,
            probed_at: Some("2026-10-09T00:00:00Z".to_string()),
            probe_artifact_ref: Some("probe://boundary/codex/seed".to_string()),
            root_recipe_evidence: RootRecipeEvidence::None,
        }
    }

    fn version_source(version: &'static str) -> VersionSource {
        Arc::new(move |_program: &str| Ok(version.to_string()))
    }

    /// 计数并按 action 派发一致材料包（或注入失败）的 fake runner；同时
    /// 断言 seam 契约：base/evidence_root 在场、resume 规格随行。
    fn counting_runner(
        calls: Arc<AtomicUsize>,
        fail_action: Option<SessionPolicyAction>,
    ) -> ProbeRunner {
        Arc::new(move |invocation: ProbeInvocation| {
            let calls = calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                assert!(invocation.base.is_absolute(), "fixture base 必须绝对路径");
                assert!(
                    invocation
                        .evidence_root
                        .ends_with("capability-evidence/boundary"),
                    "证据根必须落 LC 子树: {}",
                    invocation.evidence_root.display()
                );
                assert!(invocation.resume.is_some(), "产品通道必须携带 resume 面");
                assert!(
                    invocation
                        .session_label
                        .contains(revalidate_action_text(invocation.action)),
                    "label 携带 action 名: {}",
                    invocation.session_label
                );
                if Some(invocation.action) == fail_action {
                    return Err(ProviderBoundaryError::ProbeFailed(
                        "boundary_unobserved: negative probe landed".to_string(),
                    ));
                }
                let seed = match invocation.action {
                    SessionPolicyAction::CodingTargetWrite => 21,
                    SessionPolicyAction::PlanningReadOnly => 22,
                    SessionPolicyAction::ReviewReadOnly => 23,
                };
                Ok(codex_confirmed_outcome(invocation.action, seed))
            })
        })
    }

    #[tokio::test]
    async fn pcr_unsupported_provider_returns_stable_error_without_probe() {
        let (paths, _root) = paths_fixture();
        let calls = Arc::new(AtomicUsize::new(0));
        let service = ProviderCapabilityRevalidateService::new(paths)
            .with_version_source(version_source(PROBE_VERSION))
            .with_probe_runner(counting_runner(calls.clone(), None));
        let error = service
            .revalidate(PROJECT_ID, &lc_id_fixture(), ProviderName::Fake)
            .await
            .expect_err("Fake 无通道必须拒绝");
        match error {
            RevalidateError::Unsupported { provider } => {
                assert!(provider.contains("Fake"), "provider 名如实: {provider}");
            }
            other => panic!("期望 Unsupported，实际 {other:?}"),
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0, "不点不跑：零探针");
    }

    #[tokio::test]
    async fn pcr_already_confirmed_same_version_returns_without_probing() {
        let (paths, _root) = paths_fixture();
        let lc_id = lc_id_fixture();
        let store = ProviderCapabilityStore::for_lc(paths.clone(), &lc_id);
        store
            .upsert(
                PROJECT_ID,
                &fully_confirmed_record(PROBE_VERSION, &REVALIDATE_ACTIONS),
            )
            .expect("seed");
        let calls = Arc::new(AtomicUsize::new(0));
        let service = ProviderCapabilityRevalidateService::new(paths)
            .with_version_source(version_source(PROBE_VERSION))
            .with_probe_runner(counting_runner(calls.clone(), None));
        let outcome = service
            .revalidate(PROJECT_ID, &lc_id, ProviderName::Codex)
            .await
            .expect("全行已验证应秒回");
        assert_eq!(
            outcome,
            RevalidateOutcome::AlreadyConfirmed {
                version: PROBE_VERSION.to_string()
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0, "秒回不重跑：零探针");
    }

    #[tokio::test]
    async fn pcr_unknown_rows_probe_all_actions_and_import_confirmed() {
        let (paths, _root) = paths_fixture();
        let lc_id = lc_id_fixture();
        let calls = Arc::new(AtomicUsize::new(0));
        let service = ProviderCapabilityRevalidateService::new(paths.clone())
            .with_version_source(version_source(PROBE_VERSION))
            .with_probe_runner(counting_runner(calls.clone(), None));
        let outcome = service
            .revalidate(PROJECT_ID, &lc_id, ProviderName::Codex)
            .await
            .expect("Unknown 应逐 action 探测导入");
        assert_eq!(
            outcome,
            RevalidateOutcome::Revalidated {
                version: PROBE_VERSION.to_string(),
                imported_actions: REVALIDATE_ACTIONS.to_vec(),
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 3, "三 action 各一次真探针");
        let record = ProviderCapabilityStore::for_lc(paths, &lc_id)
            .get(PROJECT_ID, ProviderRefType::Codex)
            .expect("读回")
            .expect("导入后记录在场");
        assert_eq!(record.version, PROBE_VERSION, "版本钉定");
        assert_eq!(record.evidence, CapabilityEvidence::ProductionVerified);
        for action in REVALIDATE_ACTIONS {
            let row = record.action_matrix.row(&action);
            assert_eq!(
                row.launch,
                ProviderCapabilityEvidence::Confirmed,
                "{action:?} launch"
            );
            assert_eq!(
                row.resume,
                ProviderCapabilityEvidence::Confirmed,
                "{action:?} resume"
            );
            assert_eq!(
                row.write_boundary,
                ProviderCapabilityEvidence::Confirmed,
                "{action:?} write_boundary"
            );
        }
    }

    #[tokio::test]
    async fn pcr_partial_rows_probe_only_missing_actions() {
        let (paths, _root) = paths_fixture();
        let lc_id = lc_id_fixture();
        let confirmed = [
            SessionPolicyAction::PlanningReadOnly,
            SessionPolicyAction::ReviewReadOnly,
        ];
        ProviderCapabilityStore::for_lc(paths.clone(), &lc_id)
            .upsert(
                PROJECT_ID,
                &fully_confirmed_record(PROBE_VERSION, &confirmed),
            )
            .expect("seed");
        let calls = Arc::new(AtomicUsize::new(0));
        let service = ProviderCapabilityRevalidateService::new(paths)
            .with_version_source(version_source(PROBE_VERSION))
            .with_probe_runner(counting_runner(calls.clone(), None));
        let outcome = service
            .revalidate(PROJECT_ID, &lc_id, ProviderName::Codex)
            .await
            .expect("仅缺失行需补探");
        assert_eq!(
            outcome,
            RevalidateOutcome::Revalidated {
                version: PROBE_VERSION.to_string(),
                imported_actions: vec![SessionPolicyAction::CodingTargetWrite],
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1, "只探缺失的 Coding 行");
    }

    #[tokio::test]
    async fn pcr_version_drift_reprobes_all_actions() {
        let (paths, _root) = paths_fixture();
        let lc_id = lc_id_fixture();
        ProviderCapabilityStore::for_lc(paths.clone(), &lc_id)
            .upsert(
                PROJECT_ID,
                &fully_confirmed_record("0.9.0-old", &REVALIDATE_ACTIONS),
            )
            .expect("seed");
        let calls = Arc::new(AtomicUsize::new(0));
        let service = ProviderCapabilityRevalidateService::new(paths)
            .with_version_source(version_source(PROBE_VERSION))
            .with_probe_runner(counting_runner(calls.clone(), None));
        let outcome = service
            .revalidate(PROJECT_ID, &lc_id, ProviderName::Codex)
            .await
            .expect("版本漂移应整体重验");
        assert_eq!(
            outcome,
            RevalidateOutcome::Revalidated {
                version: PROBE_VERSION.to_string(),
                imported_actions: REVALIDATE_ACTIONS.to_vec(),
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 3, "旧 Confirmed 不跨版本沿用");
    }

    #[tokio::test]
    async fn pcr_probe_failure_reports_honest_error_and_keeps_prior_imports() {
        let (paths, _root) = paths_fixture();
        let lc_id = lc_id_fixture();
        let calls = Arc::new(AtomicUsize::new(0));
        // Coding(首个)成功、Planning 失败 → 如实点名失败 action，Coding 的
        // 已导入 Confirmed 保留，失败行不落盘。
        let service = ProviderCapabilityRevalidateService::new(paths.clone())
            .with_version_source(version_source(PROBE_VERSION))
            .with_probe_runner(counting_runner(
                calls.clone(),
                Some(SessionPolicyAction::PlanningReadOnly),
            ));
        let error = service
            .revalidate(PROJECT_ID, &lc_id, ProviderName::Codex)
            .await
            .expect_err("探针失败必须如实失败");
        match error {
            RevalidateError::ProbeFailed { action, detail } => {
                assert_eq!(action, Some(SessionPolicyAction::PlanningReadOnly));
                assert!(
                    detail.contains("boundary_unobserved"),
                    "detail 携带原始错误: {detail}"
                );
            }
            other => panic!("期望 ProbeFailed，实际 {other:?}"),
        }
        let record = ProviderCapabilityStore::for_lc(paths, &lc_id)
            .get(PROJECT_ID, ProviderRefType::Codex)
            .expect("读回")
            .expect("先成功的导入保留");
        assert_eq!(
            record
                .action_matrix
                .row(&SessionPolicyAction::CodingTargetWrite)
                .launch,
            ProviderCapabilityEvidence::Confirmed,
            "先成功的 Coding 行保留"
        );
        assert_ne!(
            record
                .action_matrix
                .row(&SessionPolicyAction::PlanningReadOnly)
                .launch,
            ProviderCapabilityEvidence::Confirmed,
            "失败行绝不伪造 Confirmed"
        );
    }

    #[tokio::test]
    async fn pcr_cli_missing_fails_before_any_probe() {
        let (paths, _root) = paths_fixture();
        let calls = Arc::new(AtomicUsize::new(0));
        let service = ProviderCapabilityRevalidateService::new(paths)
            .with_version_source(Arc::new(|_program: &str| {
                Err(ProviderBoundaryError::ProbeFailed(
                    "command missing: codex --version".to_string(),
                ))
            }))
            .with_probe_runner(counting_runner(calls.clone(), None));
        let error = service
            .revalidate(PROJECT_ID, &lc_id_fixture(), ProviderName::Codex)
            .await
            .expect_err("CLI 缺失必须如实失败");
        match error {
            RevalidateError::ProbeFailed { action, detail } => {
                assert_eq!(action, None, "版本探测失败先于任何 action");
                assert!(detail.contains("command missing"), "detail: {detail}");
            }
            other => panic!("期望 ProbeFailed，实际 {other:?}"),
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn pcr_snapshot_projects_durable_rows_and_import_visible() {
        let (paths, _root) = paths_fixture();
        let lc_id = lc_id_fixture();
        let calls = Arc::new(AtomicUsize::new(0));
        let service = ProviderCapabilityRevalidateService::new(paths.clone())
            .with_version_source(version_source(PROBE_VERSION))
            .with_probe_runner(counting_runner(calls.clone(), None));

        // 未探测：bootstrap 默认记录不得显示为真实 Confirmed。
        let before = service
            .capability_snapshot(PROJECT_ID, &lc_id)
            .expect("快照");
        assert_eq!(before.len(), 4, "四家真实 provider");
        for snapshot in &before {
            assert!(snapshot.probed_at.is_none(), "未探测无 probed_at");
            for row in &snapshot.rows {
                assert_ne!(
                    row.launch,
                    ProviderCapabilityEvidence::Confirmed,
                    "未探测全 Unknown"
                );
            }
        }

        service
            .revalidate(PROJECT_ID, &lc_id, ProviderName::Codex)
            .await
            .expect("探测导入");
        let after = service
            .capability_snapshot(PROJECT_ID, &lc_id)
            .expect("快照");
        let codex = after
            .iter()
            .find(|snapshot| snapshot.provider_type == ProviderRefType::Codex)
            .expect("codex 行");
        assert_eq!(codex.version.as_deref(), Some(PROBE_VERSION));
        assert!(codex.probed_at.is_some(), "probed_at 投影");
        assert!(codex.probe_artifact_ref.is_some(), "证据引用投影");
        for row in &codex.rows {
            assert_eq!(row.launch, ProviderCapabilityEvidence::Confirmed);
        }
    }
}
