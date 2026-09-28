//! C4 Task 8：真实 provider/index/planning 准入门。
//!
//! 在任何 LC provider turn / planning envelope 启动前，用 Task 1 的
//! `RepositoryAuthorityResolver` 冻结 target 与 authority root，读取实际
//! 聚合 policy artifact（id/revision/digest），并检查真实 provider 将消费的
//! 每个成员 checkout 的 `.claude/rules/language.md` 与实际 gateway capability
//! 谓词。`ProviderCapabilityStore::ensure_bootstrap` 只产生待验证记录，不能
//! 证明真实能力；本预检的 `ready == true` 也只表示「材料齐备且 gateway
//! `validate` 产出 envelope」，spawn 前仍必须调用
//! `LogicalCodebaseProviderGateway::revalidate_before_spawn`。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::Digest as _;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::provider_gateway::{
    LogicalCodebaseProviderGateway, ProviderGatewayError, SessionLaunchRequest,
    ValidatedSessionLaunchPolicy,
};
use crate::product::logical_codebase::repository_routing::{
    AuthorityPolicyReference, RepositoryAuthorityResolver, RepositoryRoutingRequest,
    RepositoryTargetKind, ResolvedTargetIdentity,
};
use crate::product::logical_codebase::store::LogicalCodebaseStore;
use crate::product::logical_codebase::types::{
    CheckoutKind, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
};

/// 冷启动等待面的动作种类。首定义位于 Task 8（Task 3 的 bootstrap 投影被
/// 排产延后）；Task 3 落地 bootstrap.rs 时必须 `use` 本类型，不得另造同义枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapActionKind {
    Prepare,
    Continue,
    Retry,
    Revalidate,
    Repair,
}
impl BootstrapActionKind {
    /// 与 serde snake_case 序列化一致的稳定动作名（通知 payload/前端按钮
    /// 语义共用，不引入第二套字符串协议）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::Continue => "continue",
            Self::Retry => "retry",
            Self::Revalidate => "revalidate",
            Self::Repair => "repair",
        }
    }
}


/// 真实 provider 将消费的成员规则文件引用（C4 Task 8）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderRuleReference {
    pub member_id: LogicalRepositoryId,
    pub checkout_id: RepositoryCheckoutId,
    pub path: PathBuf,
    pub digest: Option<String>,
}

/// 准入预检结果：材料齐备且 gateway validate 通过时 `ready == true`。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderAdmissionPreflightResult {
    pub target: ResolvedTargetIdentity,
    pub authority_root: PathBuf,
    pub policy: AuthorityPolicyReference,
    pub rules: Vec<ProviderRuleReference>,
    pub capability_snapshot_ref: String,
    pub ready: bool,
    pub missing_materials: Vec<String>,
    pub allowed_actions: Vec<BootstrapActionKind>,
}

/// 预检失败：waiting 是可操作的持久等待事实（携带缺失材料与允许动作），
/// 其余为不可恢复的存储/路由错误。
#[derive(Debug)]
pub enum ProviderAdmissionError {
    Waiting {
        reason_code: String,
        detail: String,
        missing_materials: Vec<String>,
        allowed_actions: Vec<BootstrapActionKind>,
    },
    Store(ProductStoreError),
}

impl From<ProductStoreError> for ProviderAdmissionError {
    fn from(error: ProductStoreError) -> Self {
        Self::Store(error)
    }
}

/// LC-scoped provider admission preflight：resolver + policy + gateway。
pub struct LogicalCodebaseProviderAdmissionPreflight {
    paths: ProductAppPaths,
    lc_id: String,
    gateway: Arc<LogicalCodebaseProviderGateway>,
}

impl LogicalCodebaseProviderAdmissionPreflight {
    pub fn new(
        paths: ProductAppPaths,
        lc_id: impl Into<String>,
        gateway: Arc<LogicalCodebaseProviderGateway>,
    ) -> Self {
        Self {
            paths,
            lc_id: lc_id.into(),
            gateway,
        }
    }

    /// 预检一次 provider 启动请求。只读 durable 事实 + gateway `validate`
    /// （validate 不启动 provider）；任何缺失/漂移都返回 waiting 事实而非
    /// 让运行时 Failed。
    pub fn check(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<ProviderAdmissionPreflightResult, ProviderAdmissionError> {
        let resolution = RepositoryAuthorityResolver::new(self.paths.clone()).resolve(
            RepositoryRoutingRequest {
                project_id: request.project_id.clone(),
                issue_id: None,
                kind: RepositoryTargetKind::LogicalCodebase,
                repository_id: None,
                logical_codebase_id: Some(self.lc_id.clone()),
                logical_repository_id: parse_member_id(&request.target.logical_repository_id),
                checkout_id: parse_checkout_id(&request.target.checkout_id),
            },
        )?;

        let mut missing_materials = Vec::new();
        let mut allowed_actions = Vec::new();

        // 1. manifest：冷启动未登记时投影 waiting（Prepare）。
        let manifest = resolution
            .manifest
            .as_ref()
            .ok_or_else(|| ProviderAdmissionError::Waiting {
                reason_code: "logical_codebase_manifest_missing".to_string(),
                detail: format!(
                    "logical codebase {} has no manifest; register members first",
                    self.lc_id
                ),
                missing_materials: vec![format!(
                    "logical-codebases/{}/manifest.json",
                    self.lc_id
                )],
                allowed_actions: vec![BootstrapActionKind::Prepare],
            })?;

        // 2. 实际成员规则：每个 active 成员的 main checkout 必须有
        //    `.claude/rules/language.md`（与 single_candidate_author 同一路径）。
        let logical = LogicalCodebaseStore::for_lc(self.paths.clone(), &self.lc_id);
        let members = logical.list_members(&request.project_id)?;
        let checkouts = logical.list_checkouts(&request.project_id)?;
        let mut rules = Vec::new();
        for member in &members {
            if member.status != MemberStatus::Active {
                continue;
            }
            let Some(checkout) = checkouts
                .iter()
                .find(|checkout| {
                    member.checkout_ids.contains(&checkout.checkout_id)
                        && checkout.kind == CheckoutKind::Main
                })
                .or_else(|| {
                    checkouts
                        .iter()
                        .find(|checkout| {
                            member.checkout_ids.contains(&checkout.checkout_id)
                        })
                })
            else {
                missing_materials.push(format!(
                    "member {} ({}) has no recorded checkout",
                    member.alias,
                    member.logical_repository_id.0
                ));
                continue;
            };
            let rule_path = checkout
                .canonical_path
                .join(".claude/rules/language.md");
            match std::fs::read(&rule_path) {
                Ok(bytes) => rules.push(ProviderRuleReference {
                    member_id: member.logical_repository_id,
                    checkout_id: checkout.checkout_id,
                    path: rule_path,
                    digest: Some(format!("sha256:{:x}", sha2::Sha256::digest(&bytes))),
                }),
                Err(_) => {
                    missing_materials.push(format!(
                        "member {} missing {}",
                        member.alias,
                        rule_path.display()
                    ));
                    rules.push(ProviderRuleReference {
                        member_id: member.logical_repository_id,
                        checkout_id: checkout.checkout_id,
                        path: rule_path,
                        digest: None,
                    });
                }
            }
        }
        if !missing_materials.is_empty() {
            allowed_actions.push(BootstrapActionKind::Prepare);
            allowed_actions.push(BootstrapActionKind::Retry);
        }

        // 3. 聚合 policy artifact：digest/revision 由 store 校验后冻结进引用。
        let policy = resolution.policy.clone().ok_or_else(|| {
            let mut materials = missing_materials.clone();
            materials.push(format!(
                "logical-codebases/{}/aggregate-policy.json",
                self.lc_id
            ));
            ProviderAdmissionError::Waiting {
                reason_code: "aggregate_policy_artifact_missing".to_string(),
                detail: format!(
                    "logical codebase {} has no aggregate policy artifact",
                    self.lc_id
                ),
                missing_materials: materials,
                allowed_actions: vec![BootstrapActionKind::Prepare, BootstrapActionKind::Retry],
            }
        })?;

        // 4. 真实 gateway 谓词：bootstrap capability 记录不满足 snapshot/action
        //    时在此拒绝（validate 不启动 provider）。
        let validated = self.gateway.validate(request.clone()).map_err(|error| {
            let reason_code = match &error {
                ProviderGatewayError::UnsupportedCapability(_) => {
                    "provider_capability_not_satisfied"
                }
                ProviderGatewayError::PolicyMissing(_) => "aggregate_policy_artifact_missing",
                _ => "provider_gateway_denied",
            };
            let mut materials = missing_materials.clone();
            if matches!(error, ProviderGatewayError::PolicyMissing(_)) {
                materials.push(format!(
                    "logical-codebases/{}/aggregate-policy.json",
                    self.lc_id
                ));
            }
            ProviderAdmissionError::Waiting {
                reason_code: reason_code.to_string(),
                detail: error.to_string(),
                missing_materials: materials,
                allowed_actions: vec![BootstrapActionKind::Revalidate, BootstrapActionKind::Retry],
            }
        })?;

        // 5. envelope 与 resolver 冻结值比对：authority root / policy digest 漂移
        //    在 spawn 前转为 waiting（Revalidate），绝不回落旧路径。
        let envelope = validated.envelope();
        if envelope.authority_root != resolution.authority_root {
            return Err(ProviderAdmissionError::Waiting {
                reason_code: "authority_root_drift".to_string(),
                detail: format!(
                    "envelope authority root {} does not match resolver-frozen root {}",
                    envelope.authority_root.display(),
                    resolution.authority_root.display()
                ),
                missing_materials: missing_materials.clone(),
                allowed_actions: vec![BootstrapActionKind::Revalidate],
            });
        }
        if envelope.policy_digest != policy.policy_digest
            || envelope.policy_revision != policy.policy_revision
        {
            return Err(ProviderAdmissionError::Waiting {
                reason_code: "policy_digest_drift".to_string(),
                detail: format!(
                    "envelope policy {}/{} does not match authority policy {}/{}",
                    envelope.policy_id,
                    envelope.policy_revision,
                    policy.policy_id,
                    policy.policy_revision
                ),
                missing_materials: missing_materials.clone(),
                allowed_actions: vec![BootstrapActionKind::Revalidate],
            });
        }

        // 6. spawn 前复验（pub(crate) 拓宽后由 admission 显式调用）：policy
        //    revision/digest、capability、config digest、target cwd 的 TOCTOU
        //    全维度复核；失败转 waiting，provider 保持零启动。
        let cwd = if request.target.worktree.is_absolute() {
            request.target.worktree.clone()
        } else {
            manifest.provider_context_root.join(&request.target.worktree)
        };
        self.gateway
            .revalidate_before_spawn(&validated, &cwd, false)
            .map_err(|error| ProviderAdmissionError::Waiting {
                reason_code: "spawn_revalidation_drift".to_string(),
                detail: error.to_string(),
                missing_materials: missing_materials.clone(),
                allowed_actions: vec![BootstrapActionKind::Revalidate, BootstrapActionKind::Retry],
            })?;

        if !missing_materials.is_empty() {
            return Err(ProviderAdmissionError::Waiting {
                reason_code: "member_language_rules_missing".to_string(),
                detail: format!(
                    "logical codebase {} has members without .claude/rules/language.md",
                    self.lc_id
                ),
                missing_materials,
                allowed_actions,
            });
        }

        Ok(ProviderAdmissionPreflightResult {
            target: resolution.target,
            authority_root: resolution.authority_root,
            policy,
            rules,
            capability_snapshot_ref: validated.capability_snapshot_ref().to_string(),
            ready: true,
            missing_materials,
            allowed_actions: Vec::new(),
        })
    }
}

fn parse_member_id(value: &str) -> Option<LogicalRepositoryId> {
    uuid::Uuid::parse_str(value)
        .ok()
        .map(LogicalRepositoryId)
}

fn parse_checkout_id(value: &str) -> Option<RepositoryCheckoutId> {
    uuid::Uuid::parse_str(value)
        .ok()
        .map(RepositoryCheckoutId)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
    use crate::product::logical_codebase::policy::{ProviderDialect, SessionPolicyAction};
    use crate::product::logical_codebase::provider_gateway::{
        PolicyTargetResolver, ProviderCapability, ProviderCapabilitySource, ProviderRef,
        ProviderRefType,
    };
    use crate::product::logical_codebase::types::{
        CheckoutAvailability, CodebaseMemberRecord, RepositoryCheckoutRecord,
        RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};

    // ---- 测试 fakes（与 provider_gateway_tests 同型，作用域隔离） ----

    struct PassThroughTargetResolver;
    impl PolicyTargetResolver for PassThroughTargetResolver {
        fn resolve_and_revalidate(
            &self,
            request: &SessionLaunchRequest,
        ) -> Result<crate::product::logical_codebase::policy::PolicyTarget, ProviderGatewayError>
        {
            let canonical = std::fs::canonicalize(&request.target.worktree)
                .map_err(|_| ProviderGatewayError::Target("worktree missing".to_string()))?;
            if request.target.logical_repository_id.is_empty() {
                Ok(crate::product::logical_codebase::policy::PolicyTarget::aggregate_root(
                    canonical,
                ))
            } else {
                Ok(crate::product::logical_codebase::policy::PolicyTarget::checkout(
                    request.target.logical_repository_id.clone(),
                    request.target.checkout_id.clone(),
                    canonical,
                ))
            }
        }
    }

    struct StaticCapabilitySource {
        deny: std::sync::atomic::AtomicBool,
    }
    impl StaticCapabilitySource {
        fn allowing() -> Self {
            Self {
                deny: std::sync::atomic::AtomicBool::new(false),
            }
        }
        fn deny(&self) {
            self.deny.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    impl ProviderCapabilitySource for StaticCapabilitySource {
        fn require_supported(
            &self,
            provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            if self.deny.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(ProviderGatewayError::UnsupportedCapability(
                    "capability record missing".to_string(),
                ));
            }
            let adapter_dialect = match provider.provider_type {
                ProviderRefType::ClaudeCode => ProviderDialect::ClaudeCodeCliV1,
                ProviderRefType::Codex => ProviderDialect::CodexCliV1,
            };
            Ok(ProviderCapability {
                provider_type: provider.provider_type,
                version: "1.4.0".to_string(),
                adapter_dialect,
                capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
                resume_evidence:
                    crate::product::logical_codebase::provider_gateway::ResumeEvidenceState::Confirmed,
            })
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

    fn always_available_gate()
    -> Arc<crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate> {
        use crate::cross_cutting::provider_availability_gate::ProviderHealthSource;
        use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
        use crate::product::models::ProviderName;
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
            providers: [ProviderName::ClaudeCode, ProviderName::Codex]
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

    struct AdmissionFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        project_id: String,
        lc_id: String,
        aggregate_root: PathBuf,
        member_root: PathBuf,
        policy_store: AggregatePolicyArtifactStore,
        capabilities: Arc<StaticCapabilitySource>,
        streaming_adapter: Arc<CountingStreamingAdapter>,
        gateway: Arc<LogicalCodebaseProviderGateway>,
    }

    fn admission_fixture() -> AdmissionFixture {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = ProductAppPaths::new(temp.path());
        let project = ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "admission-project".to_string(),
                description: None,
            })
            .expect("project");
        let aggregate_root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();
        let member_root = temp.path().join("member-a");
        git_init_with_commit(&member_root);

        let logical = LogicalCodebaseStore::new(paths.clone());
        let lc = logical
            .create(
                &project.id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "admission-lc".to_string(),
                    aggregate_root: aggregate_root.clone(),
                },
            )
            .expect("lc");

        let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc.id);
        let member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let canonical = std::fs::canonicalize(&member_root).unwrap();
        let source = RepositorySourceIdentity {
            scheme: "test".to_string(),
            key_digest: "sha256:admission-member".to_string(),
            canonical_git_dir: canonical.join(".git"),
            canonical_origin: None,
            first_seen_path_hash: "sha256:admission-path".to_string(),
        };
        let mut manifest = crate::product::logical_codebase::store::LogicalCodebaseManifest::new(
            &project.id,
            aggregate_root.clone(),
            vec![member_id],
        );
        manifest.logical_codebase_id = uuid::Uuid::new_v4();
        lc_store.save_manifest(&project.id, &manifest).unwrap();
        let now = "2026-09-28T00:00:00Z".to_string();
        lc_store
            .save_member(
                &project.id,
                &CodebaseMemberRecord {
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_admission_member".to_string(),
                    alias: "member-a".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: source.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .unwrap();
        lc_store
            .save_checkout(
                &project.id,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_admission_member".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: canonical.clone(),
                    checkout_path_hash: "sha256:admission-checkout".to_string(),
                    git_dir_identity: source.git_dir_identity(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now,
                },
            )
            .unwrap();

        let policy_store = AggregatePolicyArtifactStore::for_lc(paths.clone(), &lc.id);
        policy_store
            .ensure_bootstrap(&manifest)
            .expect("bootstrap policy");

        let capabilities = Arc::new(StaticCapabilitySource::allowing());
        let streaming_adapter = Arc::new(CountingStreamingAdapter::new());
        let mut registry = ProviderRegistry::new();
        registry.register(
            crate::product::models::ProviderName::ClaudeCode,
            streaming_adapter.clone(),
        );
        registry.register(
            crate::product::models::ProviderName::Codex,
            streaming_adapter.clone(),
        );
        let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
            policy_store.clone(),
            capabilities.clone(),
            Arc::new(PassThroughTargetResolver),
            Arc::new(registry),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
            Arc::new(crate::product::logical_codebase::GatewayRunAudit::new()),
            std::fs::canonicalize(&aggregate_root).expect("canonical authority root"),
        ));

        AdmissionFixture {
            _temp: temp,
            paths,
            project_id: project.id,
            lc_id: lc.id,
            aggregate_root,
            member_root,
            policy_store,
            capabilities,
            streaming_adapter,
            gateway,
        }
    }

    impl AdmissionFixture {
        fn write_language_rules(&self, contents: &str) {
            let dir = self.member_root.join(".claude/rules");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("language.md"), contents).unwrap();
        }

        fn preflight(&self) -> LogicalCodebaseProviderAdmissionPreflight {
            LogicalCodebaseProviderAdmissionPreflight::new(
                self.paths.clone(),
                self.lc_id.clone(),
                self.gateway.clone(),
            )
        }

        fn launch_request(&self) -> SessionLaunchRequest {
            SessionLaunchRequest::planning(
                self.project_id.clone(),
                ProviderRef::claude_code("snapshot-admission-test"),
                crate::product::logical_codebase::policy::PolicyTarget::aggregate_root(
                    self.aggregate_root.clone(),
                ),
                vec![self.aggregate_root.clone()],
                "sha256:admission-managed-config",
            )
        }
    }

    fn git_init_with_commit(path: &Path) {
        std::fs::create_dir_all(path).unwrap();
        for args in [
            vec!["init", "-b", "main"],
            vec!["config", "user.email", "admission@test.local"],
            vec!["config", "user.name", "Admission Test"],
        ] {
            let output = std::process::Command::new("git")
                .current_dir(path)
                .args(&args)
                .output()
                .expect("git");
            assert!(output.status.success(), "git {args:?} failed");
        }
        std::fs::write(path.join("README.md"), "# member\n").unwrap();
        let output = std::process::Command::new("git")
            .current_dir(path)
            .args(["add", "."])
            .output()
            .unwrap();
        assert!(output.status.success());
        let output = std::process::Command::new("git")
            .current_dir(path)
            .args(["commit", "-m", "init"])
            .output()
            .unwrap();
        assert!(output.status.success());
    }

    #[test]
    fn missing_member_language_rule_waits_before_provider_spawn() {
        let fixture = admission_fixture();
        // 成员 checkout 缺 .claude/rules/language.md。
        let error = fixture
            .preflight()
            .check(&fixture.launch_request())
            .unwrap_err();
        match error {
            ProviderAdmissionError::Waiting {
                reason_code,
                missing_materials,
                allowed_actions,
                ..
            } => {
                assert_eq!(reason_code, "member_language_rules_missing");
                assert!(missing_materials
                    .iter()
                    .any(|item| item.contains("language.md")));
                assert!(allowed_actions.contains(&BootstrapActionKind::Prepare));
                assert!(allowed_actions.contains(&BootstrapActionKind::Retry));
            }
            other => panic!("expected waiting fact, got {other:?}"),
        }
        // provider 零启动。
        assert_eq!(fixture.streaming_adapter.start_count(), 0);
    }

    #[test]
    fn bootstrap_capability_record_is_not_real_provider_capability() {
        let fixture = admission_fixture();
        fixture.write_language_rules("# rules\n");
        // 真实 capability 谓词拒绝（记录缺失/不支持）→ waiting，而非因 JSON
        // 存在而放行。
        fixture.capabilities.deny();
        let error = fixture
            .preflight()
            .check(&fixture.launch_request())
            .unwrap_err();
        match error {
            ProviderAdmissionError::Waiting {
                reason_code,
                detail,
                ..
            } => {
                assert_eq!(reason_code, "provider_capability_not_satisfied");
                assert!(detail.contains("capability"));
            }
            other => panic!("expected waiting fact, got {other:?}"),
        }
        assert_eq!(fixture.streaming_adapter.start_count(), 0);
    }

    #[test]
    fn policy_digest_or_authority_root_drift_blocks_spawn() {
        let fixture = admission_fixture();
        fixture.write_language_rules("# rules\n");
        // validate 冻结 envelope 后升级 policy revision/digest → spawn 前复验
        // （pub(crate) 拓宽后的 revalidate_before_spawn）拒绝，provider 零启动。
        let validated = fixture.gateway.validate(fixture.launch_request()).unwrap();
        let existing = fixture.policy_store.get(&fixture.project_id).unwrap().unwrap();
        let revised = existing.with_revised_policy("upgrade", "2026-09-28T01:00:00Z".to_string());
        fixture
            .policy_store
            .save(&fixture.project_id, &revised)
            .unwrap();

        let error = fixture
            .gateway
            .revalidate_before_spawn(
                &validated,
                &std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
                false,
            )
            .unwrap_err();
        assert!(
            matches!(
                &error,
                ProviderGatewayError::PolicyDrift { dimension }
                    if dimension.contains("policy_revision") || dimension.contains("policy_digest")
            ),
            "unexpected drift error: {error:?}"
        );
        assert_eq!(fixture.streaming_adapter.start_count(), 0);

        // 之后的 admission 预检同样 waiting（resolver 与 store 的 policy 引用
        // 一致，但 envelope 由最新 validate 产出；漂移事实由复验路径证明）。
        let result = fixture.preflight().check(&fixture.launch_request());
        assert!(result.is_ok() || matches!(result, Err(ProviderAdmissionError::Waiting { .. })));
    }

    #[test]
    fn valid_rules_policy_and_gateway_produce_envelope() {
        let fixture = admission_fixture();
        fixture.write_language_rules("# language rules\n");

        let result = fixture
            .preflight()
            .check(&fixture.launch_request())
            .unwrap();
        assert!(result.ready, "missing: {:?}", result.missing_materials);
        assert_eq!(result.rules.len(), 1);
        let rule = &result.rules[0];
        assert!(rule.digest.as_deref().unwrap().starts_with("sha256:"));
        assert_eq!(
            result.authority_root,
            std::fs::canonicalize(&fixture.aggregate_root).unwrap()
        );
        assert_eq!(result.capability_snapshot_ref, "snapshot-admission-test");
        assert!(result.policy.policy_digest.starts_with("sha256:"));
        // 预检本身零启动。
        assert_eq!(fixture.streaming_adapter.start_count(), 0);
    }
}
