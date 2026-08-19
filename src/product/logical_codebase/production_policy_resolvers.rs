//! Production `PolicyTargetResolver`: 三层身份复验 + TOCTOU 复验。
//!
//! 生产环境的 target resolver 不再直接信任请求中的 target,而是:
//! - checkout 目标(`logical_repository_id` 非空):经
//!   `RepositoryStore::resolve_logical_repository_strict` 复验三层身份
//!   (member/checkout/repository),比对 checkout id,重新 canonicalize worktree
//!   并确认 `.git` 存在。
//! - 聚合根目标(`logical_repository_id` 为空):canonicalize 聚合根目录。
//!
//! 任一复验失败都 fail-closed 为 `ProviderGatewayError::{Target, TargetMismatch}`。

use std::path::Path;

use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::policy::{PolicyTarget, SessionPolicyAction};
use crate::product::logical_codebase::provider_capability_store::ProviderCapabilityStore;
use crate::product::logical_codebase::provider_gateway::CODEX_DANGER_FULL_ACCESS_UNSUPPORTED;
use crate::product::logical_codebase::{
    LogicalCodebaseFeature, LogicalRepositoryId, PolicyTargetResolver, ProviderCapability,
    ProviderCapabilitySource, ProviderGatewayError, ProviderRef, ProviderRefType,
    SessionLaunchRequest,
};
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::RepositoryStore;

/// 生产 target resolver:按请求 project 构造 `RepositoryStore` 以在启动前重新解析三层身份。
///
/// v1.3（R9 fix round 1）：`lc_id = Some` 时 checkout 目标改从
/// `logical-codebases/{lc_id}/` 子树权威记录解析（与
/// `RepositoryStore::resolve_logical_repository_for_issue_codebase` 语义一致）；
/// `lc_id = None`（单仓/legacy）保持既有 project 级 `for_project` + strict 行为不变。
pub struct ProductionPolicyTargetResolver {
    paths: ProductAppPaths,
    lc_id: Option<String>,
}

impl ProductionPolicyTargetResolver {
    /// legacy/project 级行为（单仓与旧数据字节级不变）。
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths, lc_id: None }
    }

    /// v1.3：按 issue 所属 lc_id 作用域解析 checkout 目标（非 legacy 新 LC 用）。
    pub fn for_lc(paths: ProductAppPaths, lc_id: &str) -> Self {
        Self {
            paths,
            lc_id: Some(lc_id.to_string()),
        }
    }

    /// checkout 目标复验:解析 logical id → 严格解析三层身份 → 比对 checkout id
    /// → canonicalize worktree → 确认 `.git` 存在。任何一步失败都 fail-closed。
    fn resolve_checkout_target(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        let logical_id = Uuid::parse_str(&request.target.logical_repository_id)
            .map(LogicalRepositoryId)
            .map_err(|_| {
                ProviderGatewayError::Target("invalid logical repository id".to_string())
            })?;

        let (_member, checkout, _repository) = match self.lc_id.as_deref() {
            Some(lc_id) => RepositoryStore::with_logical_codebase_feature(
                self.paths.clone(),
                LogicalCodebaseFeature::enabled(),
            )
            .resolve_logical_repository_for_issue_codebase(
                &request.project_id,
                Some(lc_id),
                logical_id,
            )
            .map_err(|error| ProviderGatewayError::Target(error.to_string()))?,
            None => {
                let project = ProjectStore::new(self.paths.clone())
                    .get(&request.project_id)
                    .map_err(|error| ProviderGatewayError::Target(error.to_string()))?;
                RepositoryStore::for_project(self.paths.clone(), &project)
                    .resolve_logical_repository_strict(&request.project_id, logical_id)
                    .map_err(|error| ProviderGatewayError::Target(error.to_string()))?
            }
        };

        if checkout.checkout_id.0.to_string() != request.target.checkout_id {
            return Err(ProviderGatewayError::TargetMismatch {
                field: "checkout_id".to_string(),
            });
        }

        let canonical_worktree = std::fs::canonicalize(&request.target.worktree)
            .map_err(|_| ProviderGatewayError::Target("worktree missing".to_string()))?;

        revalidate_git_dir_identity(&canonical_worktree, &checkout.canonical_path)?;

        Ok(PolicyTarget::checkout(
            logical_id.0.to_string(),
            checkout.checkout_id.0.to_string(),
            canonical_worktree,
        ))
    }

    /// 聚合根目标复验:仅 canonicalize 目录,失败 fail-closed。
    fn resolve_aggregate_target(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        let canonical = std::fs::canonicalize(&request.target.worktree)
            .map_err(|_| ProviderGatewayError::Target("aggregate root missing".to_string()))?;
        Ok(PolicyTarget::aggregate_root(canonical))
    }
}

impl PolicyTargetResolver for ProductionPolicyTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        if request.target.logical_repository_id.is_empty() {
            self.resolve_aggregate_target(request)
        } else {
            self.resolve_checkout_target(request)
        }
    }
}

/// 生产 capability source:store-backed,持有 `ProviderCapabilityStore` 与目标
/// project id。`require_supported` 按记录缺失 → Codex 阻断 → snapshot 不一致 →
/// action 不受支持 → 通过的顺序 fail-closed。
pub struct StoreBackedProviderCapabilitySource {
    store: ProviderCapabilityStore,
    project_id: String,
}

impl StoreBackedProviderCapabilitySource {
    pub fn new(paths: ProductAppPaths, project_id: String) -> Self {
        Self {
            store: ProviderCapabilityStore::new(paths),
            project_id,
        }
    }

    /// v1.3：接受已按 lc_id 作用域的 `ProviderCapabilityStore`，使 gateway 的
    /// capability 读取落在 issue 所属代码库子树。
    pub fn with_store(store: ProviderCapabilityStore, project_id: String) -> Self {
        Self { store, project_id }
    }
}

impl ProviderCapabilitySource for StoreBackedProviderCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        let record = self
            .store
            .get(&self.project_id, provider.provider_type)
            .map_err(ProviderGatewayError::policy)?
            .ok_or_else(|| {
                ProviderGatewayError::UnsupportedCapability("capability record missing".to_string())
            })?;

        if record.provider_type == ProviderRefType::Codex {
            return Err(ProviderGatewayError::UnsupportedCapability(
                CODEX_DANGER_FULL_ACCESS_UNSUPPORTED.to_string(),
            ));
        }

        if record.capability_snapshot_ref != provider.capability_snapshot_ref {
            return Err(ProviderGatewayError::UnsupportedCapability(
                "capability snapshot mismatch".to_string(),
            ));
        }

        if !record.supported_actions.contains(&action) {
            return Err(ProviderGatewayError::UnsupportedCapability(format!(
                "{action:?} not supported"
            )));
        }

        Ok(ProviderCapability {
            provider_type: record.provider_type,
            version: record.version,
            adapter_dialect: record.adapter_dialect,
            capability_snapshot_ref: record.capability_snapshot_ref,
            resume_evidence: record.resume_evidence,
        })
    }
}

/// 校验 worktree 的 `.git` 归属(REQ-ENV-03 的 git-dir identity 复验)。
///
/// 真实 git worktree 的 `.git` 是指向 `<主仓>/.git/worktrees/<name>` 的文件,
/// 非 worktree checkout 的 `.git` 是目录。两种形态解析出的实际 git dir 都必须在
/// canonicalize 后以主仓 `.git` 目录为前缀,否则视为 git-dir identity 漂移
/// (validate→spawn 之间 `.git` 指针被调包),fail-closed。
fn revalidate_git_dir_identity(
    canonical_worktree: &Path,
    main_checkout_path: &Path,
) -> Result<(), ProviderGatewayError> {
    let git_entry = canonical_worktree.join(".git");
    if !git_entry.exists() {
        return Err(ProviderGatewayError::TargetMismatch {
            field: "git_dir".to_string(),
        });
    }

    let actual_git_dir = if git_entry.is_dir() {
        std::fs::canonicalize(&git_entry).map_err(|_| ProviderGatewayError::TargetMismatch {
            field: "git_dir".to_string(),
        })?
    } else if git_entry.is_file() {
        let content = std::fs::read_to_string(&git_entry).map_err(|_| {
            ProviderGatewayError::TargetMismatch {
                field: "git_dir".to_string(),
            }
        })?;
        let pointer = content
            .lines()
            .next()
            .and_then(|line| line.trim().strip_prefix("gitdir:"))
            .map(str::trim)
            .ok_or_else(|| ProviderGatewayError::TargetMismatch {
                field: "git_dir".to_string(),
            })?;
        let pointer_path = Path::new(pointer);
        let resolved = if pointer_path.is_absolute() {
            pointer_path.to_path_buf()
        } else {
            canonical_worktree.join(pointer_path)
        };
        std::fs::canonicalize(&resolved).map_err(|_| ProviderGatewayError::TargetMismatch {
            field: "git_dir".to_string(),
        })?
    } else {
        return Err(ProviderGatewayError::TargetMismatch {
            field: "git_dir".to_string(),
        });
    };

    let main_git_dir = std::fs::canonicalize(main_checkout_path.join(".git"))
        .map_err(|_| ProviderGatewayError::Target("main git dir missing".to_string()))?;

    if !actual_git_dir.starts_with(&main_git_dir) {
        return Err(ProviderGatewayError::TargetMismatch {
            field: "git_dir".to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::logical_codebase::policy::ProviderDialect;
    use crate::product::logical_codebase::policy::SessionPolicyAction;
    use crate::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, ProviderCapabilityRecord,
    };
    use crate::product::logical_codebase::provider_gateway::ResumeEvidenceState;
    use crate::product::logical_codebase::{
        LogicalCodebaseFeature, ProviderRef, RepositoryCheckoutId,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    /// 注册一个真实逻辑代码库(manifest + member + checkout + repository),并创建
    /// 一个真实的 git worktree 目录(含 `.git`),供 target resolver 复验。
    struct ResolverFixture {
        _root: TempDir,
        paths: ProductAppPaths,
        project_id: String,
        logical_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
        worktree: PathBuf,
    }

    fn resolver_fixture() -> ResolverFixture {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let project = ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "resolver project".to_string(),
                description: None,
            })
            .expect("create project");

        let worktree = root.path().join("api");
        fs::create_dir_all(&worktree).expect("create repository root");
        run_git(&worktree, &["init", "--quiet"]);
        run_git(
            &worktree,
            &["config", "user.email", "resolver@example.test"],
        );
        run_git(&worktree, &["config", "user.name", "Resolver Fixture"]);
        fs::write(worktree.join("README.md"), "# api\n").expect("write initial file");
        run_git(&worktree, &["add", "README.md"]);
        run_git(&worktree, &["commit", "--quiet", "-m", "initial commit"]);

        let repository = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        )
        .create(CreateRepositoryInput {
            project_id: project.id.clone(),
            name: "api".to_string(),
            path: worktree,
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: "resolver-fixture".to_string(),
        })
        .expect("register logical repository");

        ResolverFixture {
            _root: root,
            paths,
            project_id: project.id,
            logical_id: repository.logical_repository_id.expect("logical id"),
            checkout_id: repository.primary_checkout_id.expect("checkout id"),
            worktree: repository.path,
        }
    }

    fn run_git(cwd: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("start git");
        assert!(status.success(), "git {} failed", args.join(" "));
    }

    impl ResolverFixture {
        fn resolver(&self) -> ProductionPolicyTargetResolver {
            ProductionPolicyTargetResolver::new(self.paths.clone())
        }

        fn coding_request(&self, worktree: PathBuf) -> SessionLaunchRequest {
            SessionLaunchRequest {
                project_id: self.project_id.clone(),
                provider: ProviderRef::claude_code("cap_claude_code_1_4_0"),
                action: SessionPolicyAction::CodingTargetWrite,
                target: PolicyTarget::checkout(
                    self.logical_id.0.to_string(),
                    self.checkout_id.0.to_string(),
                    worktree.clone(),
                ),
                readable_roots: vec![self.paths.root().to_path_buf()],
                writable_roots: vec![worktree],
                config_artifact_ref: "sha256:managed-config-artifact".to_string(),
            }
        }
    }

    #[test]
    fn coding_target_resolves_and_canonicalizes_worktree() {
        let fixture = resolver_fixture();
        let request = fixture.coding_request(fixture.worktree.clone());

        let resolved = fixture.resolver().resolve_and_revalidate(&request).unwrap();

        assert_eq!(
            resolved.worktree,
            fs::canonicalize(&fixture.worktree).unwrap()
        );
        assert_eq!(
            resolved.logical_repository_id,
            fixture.logical_id.0.to_string()
        );
        assert_eq!(resolved.checkout_id, fixture.checkout_id.0.to_string());
    }

    #[test]
    fn coding_target_rejects_missing_worktree() {
        let fixture = resolver_fixture();
        let missing = fixture._root.path().join("missing-worktree");
        let request = fixture.coding_request(missing);

        let error = fixture
            .resolver()
            .resolve_and_revalidate(&request)
            .unwrap_err();

        assert!(matches!(error, ProviderGatewayError::Target(_)));
    }

    #[test]
    fn coding_target_rejects_checkout_id_mismatch() {
        let fixture = resolver_fixture();
        let wrong_checkout = Uuid::new_v4().to_string();
        let request = SessionLaunchRequest {
            project_id: fixture.project_id.clone(),
            provider: ProviderRef::claude_code("cap_claude_code_1_4_0"),
            action: SessionPolicyAction::CodingTargetWrite,
            target: PolicyTarget::checkout(
                fixture.logical_id.0.to_string(),
                wrong_checkout,
                fixture.worktree.clone(),
            ),
            readable_roots: vec![fixture.paths.root().to_path_buf()],
            writable_roots: vec![fixture.worktree.clone()],
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        };

        let error = fixture
            .resolver()
            .resolve_and_revalidate(&request)
            .unwrap_err();

        assert!(
            matches!(error, ProviderGatewayError::TargetMismatch { ref field } if field == "checkout_id")
        );
    }

    #[test]
    fn aggregate_target_resolves_and_canonicalizes() {
        let fixture = resolver_fixture();
        let aggregate_root = fixture._root.path().join("aggregate");
        fs::create_dir_all(&aggregate_root).unwrap();
        let request = SessionLaunchRequest {
            project_id: fixture.project_id.clone(),
            provider: ProviderRef::claude_code("cap_claude_code_1_4_0"),
            action: SessionPolicyAction::PlanningReadOnly,
            target: PolicyTarget::aggregate_root(aggregate_root.clone()),
            readable_roots: vec![fixture.paths.root().to_path_buf()],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        };

        let resolved = fixture.resolver().resolve_and_revalidate(&request).unwrap();

        assert_eq!(
            resolved.worktree,
            fs::canonicalize(&aggregate_root).unwrap()
        );
        assert!(resolved.logical_repository_id.is_empty());
        assert!(resolved.checkout_id.is_empty());
    }

    /// 非 legacy LC fixture：project + 新建 LC（logical-codebases/{lc_id}/ 子树权威）
    /// + 真实 git 仓 member/checkout + identity registry。
    ///
    /// 不写 project 级 legacy manifest/repos.json（R9 新 LC 登记语义）。
    struct NewLcResolverFixture {
        _root: TempDir,
        paths: ProductAppPaths,
        project_id: String,
        lc_id: String,
        logical_id: LogicalRepositoryId,
        checkout_id: crate::product::logical_codebase::RepositoryCheckoutId,
        worktree: PathBuf,
    }

    fn new_lc_resolver_fixture() -> NewLcResolverFixture {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let project = ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "new-lc resolver project".to_string(),
                description: None,
            })
            .expect("create project");
        let aggregate_root = root.path().join("aggregate-root");
        fs::create_dir_all(&aggregate_root).expect("create aggregate root");
        let record = crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone())
            .create(
                &project.id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "new-lc".to_string(),
                    aggregate_root,
                },
            )
            .expect("create logical codebase record");
        let lc_id = record.id;

        let worktree = root.path().join("api");
        fs::create_dir_all(&worktree).expect("create repository root");
        run_git(&worktree, &["init", "--quiet"]);
        run_git(
            &worktree,
            &["config", "user.email", "lc-resolver@example.test"],
        );
        run_git(&worktree, &["config", "user.name", "New LC Resolver"]);
        fs::write(worktree.join("README.md"), "# api\n").expect("write file");
        run_git(&worktree, &["add", "README.md"]);
        run_git(&worktree, &["commit", "--quiet", "-m", "initial commit"]);

        let authority = crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
            paths.clone(),
            lc_id.clone(),
        );
        let logical_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = crate::product::logical_codebase::RepositoryCheckoutId(Uuid::new_v4());
        let physical_repository_id = format!("repository_{}", Uuid::new_v4().simple());
        let manifest = crate::product::logical_codebase::LogicalCodebaseManifest::new(
            &project.id,
            root.path().join("aggregate-root"),
            vec![logical_id],
        );
        authority
            .save_manifest(&project.id, &manifest)
            .expect("save lc manifest");
        let now = "2026-08-18T00:00:00Z".to_string();
        let source_identity =
            crate::product::logical_codebase::RepositorySourceIdentity::from_git_parts(
                &worktree,
                worktree.join(".git"),
                None,
            );
        authority
            .save_member(
                &project.id,
                &crate::product::logical_codebase::CodebaseMemberRecord {
                    logical_repository_id: logical_id,
                    physical_repository_id: physical_repository_id.clone(),
                    alias: "api".to_string(),
                    role: "repository".to_string(),
                    ordinal: 0,
                    source_identity: source_identity.clone(),
                    repo_type: crate::product::logical_codebase::RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: crate::product::logical_codebase::MemberStatus::Active,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .expect("save lc member");
        authority
            .save_checkout(
                &project.id,
                &crate::product::logical_codebase::RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: logical_id,
                    physical_repository_id: physical_repository_id.clone(),
                    kind: crate::product::logical_codebase::CheckoutKind::Main,
                    canonical_path: worktree.clone(),
                    checkout_path_hash: "sha256:checkout".to_string(),
                    git_dir_identity: source_identity.git_dir_identity().to_string(),
                    revision: None,
                    availability: crate::product::logical_codebase::CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now,
                },
            )
            .expect("save lc checkout");
        crate::product::logical_codebase::IdentityRegistryStore::new(paths.clone())
            .upsert_active(
                &project.id,
                crate::product::logical_codebase::IdentityRegistryEntry::active(
                    source_identity,
                    logical_id,
                    physical_repository_id,
                    checkout_id,
                    "new-lc-resolver-fixture".to_string(),
                ),
            )
            .expect("register identity");

        NewLcResolverFixture {
            _root: root,
            paths,
            project_id: project.id,
            lc_id,
            logical_id,
            checkout_id,
            worktree,
        }
    }

    impl NewLcResolverFixture {
        fn coding_request(&self, worktree: PathBuf) -> SessionLaunchRequest {
            SessionLaunchRequest {
                project_id: self.project_id.clone(),
                provider: ProviderRef::claude_code("cap_claude_code_1_4_0"),
                action: SessionPolicyAction::CodingTargetWrite,
                target: PolicyTarget::checkout(
                    self.logical_id.0.to_string(),
                    self.checkout_id.0.to_string(),
                    worktree.clone(),
                ),
                readable_roots: vec![self.paths.root().to_path_buf()],
                writable_roots: vec![worktree],
                config_artifact_ref: "sha256:managed-config-artifact".to_string(),
            }
        }
    }

    /// 红→绿基线：legacy（project 级）resolver 对新 LC 子树 checkout fail-closed
    /// （新 LC 不写 legacy manifest，strict 解析必失败）。修复前即红，用于钉住
    /// 「不能用 for_project 解析新 LC」这一约束。
    #[test]
    fn legacy_project_resolver_fails_closed_for_new_lc_checkout() {
        let fixture = new_lc_resolver_fixture();
        let request = fixture.coding_request(fixture.worktree.clone());

        let error = ProductionPolicyTargetResolver::new(fixture.paths.clone())
            .resolve_and_revalidate(&request)
            .unwrap_err();

        assert!(matches!(error, ProviderGatewayError::Target(_)));
    }

    /// R9 fix round 1【Important-1】：非 legacy LC 的 coding 启动 target 复验必须
    /// 按 lc_id 子树权威解析通过。
    #[test]
    fn for_lc_resolver_validates_new_lc_checkout_target() {
        let fixture = new_lc_resolver_fixture();
        let request = fixture.coding_request(fixture.worktree.clone());

        let resolved =
            ProductionPolicyTargetResolver::for_lc(fixture.paths.clone(), &fixture.lc_id)
                .resolve_and_revalidate(&request)
                .expect("resolve new lc checkout target");

        assert_eq!(
            resolved.worktree,
            fs::canonicalize(&fixture.worktree).unwrap()
        );
        assert_eq!(
            resolved.logical_repository_id,
            fixture.logical_id.0.to_string()
        );
        assert_eq!(resolved.checkout_id, fixture.checkout_id.0.to_string());
    }

    /// 非 legacy LC 下 checkout_id 不匹配仍 fail-closed（lc 寻址不放松身份复验）。
    #[test]
    fn for_lc_resolver_rejects_checkout_id_mismatch() {
        let fixture = new_lc_resolver_fixture();
        let request = SessionLaunchRequest {
            project_id: fixture.project_id.clone(),
            provider: ProviderRef::claude_code("cap_claude_code_1_4_0"),
            action: SessionPolicyAction::CodingTargetWrite,
            target: PolicyTarget::checkout(
                fixture.logical_id.0.to_string(),
                Uuid::new_v4().to_string(),
                fixture.worktree.clone(),
            ),
            readable_roots: vec![fixture.paths.root().to_path_buf()],
            writable_roots: vec![fixture.worktree.clone()],
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        };

        let error = ProductionPolicyTargetResolver::for_lc(fixture.paths.clone(), &fixture.lc_id)
            .resolve_and_revalidate(&request)
            .unwrap_err();

        assert!(
            matches!(error, ProviderGatewayError::TargetMismatch { ref field } if field == "checkout_id")
        );
    }

    fn write_gitdir_file(worktree_dir: &std::path::Path, git_dir: &std::path::Path) {
        fs::write(
            worktree_dir.join(".git"),
            format!("gitdir: {}\n", git_dir.display()),
        )
        .expect("write gitdir file");
    }

    /// 正例:真实 git worktree 的 `.git` 是指向 `<主仓>/.git/worktrees/<name>` 的
    /// 文件;解析出的 git dir 在主仓 `.git` 目录之内,应通过 identity 复验并返回
    /// canonical target。
    #[test]
    fn coding_target_accepts_worktree_gitfile_pointing_into_main_git_dir() {
        let fixture = resolver_fixture();
        let linked = fixture._root.path().join("linked-worktree");
        fs::create_dir_all(&linked).unwrap();
        let worktree_git_dir = fixture.worktree.join(".git").join("worktrees").join("test");
        fs::create_dir_all(&worktree_git_dir).unwrap();
        write_gitdir_file(&linked, &worktree_git_dir);

        let request = fixture.coding_request(linked.clone());

        let resolved = fixture.resolver().resolve_and_revalidate(&request).unwrap();

        assert_eq!(resolved.worktree, fs::canonicalize(&linked).unwrap());
        assert_eq!(
            resolved.logical_repository_id,
            fixture.logical_id.0.to_string()
        );
        assert_eq!(resolved.checkout_id, fixture.checkout_id.0.to_string());
    }

    /// 负例:worktree 的 `.git` 文件指向主仓 `.git` **之外**的路径,git-dir identity
    /// 漂移,应 fail-closed 为 `TargetMismatch { field: "git_dir" }`。
    #[test]
    fn coding_target_rejects_worktree_gitfile_pointing_outside_main_git_dir() {
        let fixture = resolver_fixture();
        let linked = fixture._root.path().join("linked-worktree");
        fs::create_dir_all(&linked).unwrap();
        let outside = fixture._root.path().join("stolen-git-dir");
        fs::create_dir_all(&outside).unwrap();
        write_gitdir_file(&linked, &outside);

        let request = fixture.coding_request(linked);

        let error = fixture
            .resolver()
            .resolve_and_revalidate(&request)
            .unwrap_err();

        assert!(
            matches!(error, ProviderGatewayError::TargetMismatch { ref field } if field == "git_dir")
        );
    }

    /// 构造一个已 bootstrap 的 store-backed capability source。TempDir 由调用方
    /// 保持存活,确保 capability 文件在测试期间存在。
    fn store_backed_source(project_id: &str) -> (TempDir, StoreBackedProviderCapabilitySource) {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        ProviderCapabilityStore::new(paths.clone())
            .ensure_bootstrap(project_id)
            .expect("bootstrap capabilities");
        let source = StoreBackedProviderCapabilitySource::new(paths, project_id.to_string());
        (root, source)
    }

    #[test]
    fn store_backed_capability_claude_code_passes_for_supported_action() {
        let (_root, source) = store_backed_source("project_0001");

        let capability = source
            .require_supported(
                &ProviderRef::claude_code("cap_managed_snapshot"),
                SessionPolicyAction::CodingTargetWrite,
            )
            .unwrap();

        assert_eq!(capability.provider_type, ProviderRefType::ClaudeCode);
        assert_eq!(capability.version, "0.0.0-managed");
        assert_eq!(capability.adapter_dialect, ProviderDialect::ClaudeCodeCliV1);
        assert_eq!(capability.capability_snapshot_ref, "cap_managed_snapshot");
        assert_eq!(capability.resume_evidence, ResumeEvidenceState::Confirmed);
    }

    #[test]
    fn store_backed_capability_codex_is_blocked_even_with_matching_snapshot() {
        let (_root, source) = store_backed_source("project_0001");

        let error = source
            .require_supported(
                &ProviderRef::codex("cap_managed_snapshot"),
                SessionPolicyAction::CodingTargetWrite,
            )
            .unwrap_err();

        assert!(
            matches!(&error, ProviderGatewayError::UnsupportedCapability(reason) if reason == CODEX_DANGER_FULL_ACCESS_UNSUPPORTED)
        );
    }

    #[test]
    fn store_backed_capability_missing_record_is_unsupported() {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let source = StoreBackedProviderCapabilitySource::new(paths, "project_0001".to_string());

        let error = source
            .require_supported(
                &ProviderRef::claude_code("cap_managed_snapshot"),
                SessionPolicyAction::CodingTargetWrite,
            )
            .unwrap_err();

        assert!(
            matches!(&error, ProviderGatewayError::UnsupportedCapability(reason) if reason == "capability record missing")
        );
    }

    #[test]
    fn store_backed_capability_snapshot_mismatch_is_unsupported() {
        let (_root, source) = store_backed_source("project_0001");

        let error = source
            .require_supported(
                &ProviderRef::claude_code("cap_other_snapshot"),
                SessionPolicyAction::CodingTargetWrite,
            )
            .unwrap_err();

        assert!(
            matches!(&error, ProviderGatewayError::UnsupportedCapability(reason) if reason == "capability snapshot mismatch")
        );
    }

    #[test]
    fn store_backed_capability_action_not_in_supported_actions_is_unsupported() {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let store = ProviderCapabilityStore::new(paths.clone());
        store.ensure_bootstrap("project_0001").unwrap();
        store
            .upsert(
                "project_0001",
                &ProviderCapabilityRecord {
                    provider_type: ProviderRefType::ClaudeCode,
                    version: "0.0.0-managed".to_string(),
                    adapter_dialect: ProviderDialect::ClaudeCodeCliV1,
                    capability_snapshot_ref: "cap_managed_snapshot".to_string(),
                    evidence: CapabilityEvidence::FixtureVerified,
                    resume_evidence: ResumeEvidenceState::Confirmed,
                    supported_actions: vec![SessionPolicyAction::ReviewReadOnly],
                },
            )
            .unwrap();
        let source = StoreBackedProviderCapabilitySource::new(paths, "project_0001".to_string());

        let error = source
            .require_supported(
                &ProviderRef::claude_code("cap_managed_snapshot"),
                SessionPolicyAction::CodingTargetWrite,
            )
            .unwrap_err();

        assert!(
            matches!(&error, ProviderGatewayError::UnsupportedCapability(reason) if reason == "CodingTargetWrite not supported")
        );
    }
}
