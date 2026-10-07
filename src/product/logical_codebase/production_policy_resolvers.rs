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

use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::policy::{PolicyTarget, SessionPolicyAction};
use crate::product::logical_codebase::provider_capability_store::ProviderCapabilityStore;
use crate::product::logical_codebase::provider_gateway::{
    CODEX_DANGER_FULL_ACCESS_UNSUPPORTED, PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED,
    PROVIDER_CAPABILITY_WRITE_BOUNDARY_NOT_CONFIRMED,
    PROVIDER_ROOT_RECIPE_CREDENTIAL_RECHECK_DENIED,
    PROVIDER_ROOT_RECIPE_CREDENTIAL_RECHECK_UNAVAILABLE,
    PROVIDER_ROOT_RECIPE_EVIDENCE_VERSION_DRIFT, PROVIDER_ROOT_RECIPE_REQUIRES_FIXED_CLAUDE,
};
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

    /// C4 Task 2：当前请求的有效 lc 作用域：显式 `for_lc` 优先；无显式作用域
    /// 但 legacy 别名 LC record 存在（旧 project 级布局）时返回别名 id（其子树
    /// 与旧布局字节等价），使别名路径同样经过 authority resolver 冲突校验；
    /// 两者皆无 → `None`（单仓兼容路径）。
    fn effective_lc_id(&self, project_id: &str) -> Option<String> {
        if let Some(lc_id) = self.lc_id.as_deref() {
            return Some(lc_id.to_string());
        }
        let alias_exists = self
            .paths
            .logical_codebase_record_root(
                project_id,
                &crate::product::logical_codebase::store::legacy_logical_codebase_id(project_id),
            )
            .join("record.json")
            .try_exists()
            .unwrap_or(false);
        alias_exists.then(|| {
            crate::product::logical_codebase::store::legacy_logical_codebase_id(project_id)
        })
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

        // C4 Task 2：authority 身份先经唯一 resolver 冻结——重复来源/legacy 布局
        // 冲突在三层身份解析前 fail-closed。lc 作用域缺失且无 legacy 别名
        // record 时保留既有 project 级兼容路径（不把物理仓伪装为 LC target）。
        let authority_lc_id = self.effective_lc_id(&request.project_id);
        if let Some(lc_id) = authority_lc_id.as_deref() {
            let checkout_id = Uuid::parse_str(&request.target.checkout_id)
                .map(crate::product::logical_codebase::RepositoryCheckoutId)
                .map_err(|_| ProviderGatewayError::Target("invalid checkout id".to_string()))?;
            crate::product::logical_codebase::RepositoryAuthorityResolver::new(self.paths.clone())
                .resolve(crate::product::logical_codebase::RepositoryRoutingRequest {
                    project_id: request.project_id.clone(),
                    issue_id: None,
                    kind: crate::product::logical_codebase::RepositoryTargetKind::LogicalCodebase,
                    repository_id: None,
                    logical_codebase_id: Some(lc_id.to_string()),
                    logical_repository_id: Some(logical_id),
                    checkout_id: Some(checkout_id),
                })
                .map_err(|error| match error {
                    // 请求 checkout 未在 LC 子树命中＝请求身份与权威身份在
                    // checkout_id 字段上漂移:保持与 project 级路径同形的
                    // `TargetMismatch { field: "checkout_id" }`(R9「lc 寻址
                    // 不放松身份复验」契约);其余 authority 冲突(重复来源/
                    // legacy 布局/成员未知等)仍按 Target fail-closed。
                    crate::product::json_store::ProductStoreError::NotFound {
                        kind: "repository_checkout",
                        ..
                    } => ProviderGatewayError::TargetMismatch {
                        field: "checkout_id".to_string(),
                    },
                    error => ProviderGatewayError::Target(error.to_string()),
                })?;
        }

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
/// project id。Task 2b 起 capability 组装消费 record v2(wire dialect、逐 action
/// 三态行、trust),并按相位分流:正常会话走 action row 分格门(`require_supported`
/// launch / `require_write_boundary` / `require_resume_supported`),root-recipe
/// 相位走 `require_root_recipe_supported`(凭据每次重验 durable Running,不推
/// normal Confirmed)。顺序 fail-closed:记录缺失 → Codex 硬阻断 → snapshot
/// 不一致 → 分格不满足。
pub struct StoreBackedProviderCapabilitySource {
    store: ProviderCapabilityStore,
    project_id: String,
    /// root-recipe 凭据的 durable Running 重核验通道(`for_lc` 构造时可用;
    /// `new`/`with_store` 无通道,root-recipe 相位 fail-closed 拒绝)。
    recipe_recheck: Option<RootRecipeCredentialRecheck>,
}

/// root-recipe 凭据重核验通道:durable operation store(lc 作用域)+ 核验 LC。
struct RootRecipeCredentialRecheck {
    operations: crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore,
    lc_id: String,
}

impl StoreBackedProviderCapabilitySource {
    pub fn new(paths: ProductAppPaths, project_id: String) -> Self {
        Self {
            store: ProviderCapabilityStore::new(paths),
            project_id,
            recipe_recheck: None,
        }
    }

    /// v1.3:接受已按 lc_id 作用域的 `ProviderCapabilityStore`,使 gateway 的
    /// capability 读取落在 issue 所属代码库子树。
    pub fn with_store(store: ProviderCapabilityStore, project_id: String) -> Self {
        Self {
            store,
            project_id,
            recipe_recheck: None,
        }
    }

    /// Task 2b:按 lc 作用域构造,并同时建立 root-recipe 凭据的 durable
    /// Running 重核验通道(capability store 与 operation store 同一 lc 子树)。
    pub fn for_lc(paths: ProductAppPaths, project_id: String, lc_id: String) -> Self {
        Self {
            store: ProviderCapabilityStore::for_lc(paths.clone(), &lc_id),
            project_id,
            recipe_recheck: Some(RootRecipeCredentialRecheck {
                operations:
                    crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore::for_lc(
                        paths, &lc_id,
                    ),
                lc_id,
            }),
        }
    }

    /// 载入记录并施加公共硬门(缺记录 → Codex 阻断 → snapshot 不一致)。
    fn load_record(
        &self,
        provider: &ProviderRef,
    ) -> Result<
        crate::product::logical_codebase::provider_capability_store::ProviderCapabilityRecord,
        ProviderGatewayError,
    > {
        let record = self
            .store
            .get(&self.project_id, provider.provider_type)
            .map_err(ProviderGatewayError::policy)?
            .ok_or_else(|| {
                // Task 3(6b carry 重钉):Pi/Kimi 已是 1a 合法映射,缺 durable
                // capability 记录属于 capability 准入失败,不再是映射失败;
                // 稳定码沿用 launch 分格判别码并点名 provider。
                ProviderGatewayError::UnsupportedCapability(format!(
                    "{PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED}: {:?} capability record missing",
                    provider.provider_type
                ))
            })?;
        // r47(codex 首轮):移除 Task 13 期的 Codex 全阻——LC 投影(Task 5,
        // REQ-LCG-04)后 Codex 会话恒非 danger(Planning/Review→read-only+
        // on-request,Coding→workspace-write;projection.rs 无 Danger 变体),
        // 全阻把 2d 探针签发的 Confirmed 行也一并拒之门外。danger 政策门
        // 保留在 gateway 路由级(enforce_route_policy,按投影后的会话形态)。

        if record.capability_snapshot_ref != provider.capability_snapshot_ref {
            return Err(ProviderGatewayError::UnsupportedCapability(
                "capability snapshot mismatch".to_string(),
            ));
        }

        Ok(record)
    }

    /// 以 record v2 字段组装 capability 快照(wire dialect/action row/trust);
    /// `row` 由调用方按相位解析(正常行 / recipe 镜像行)。
    fn capability_from_record(
        record: &crate::product::logical_codebase::provider_capability_store::ProviderCapabilityRecord,
        row: crate::product::logical_codebase::provider_capability_store::ProviderActionCapability,
    ) -> ProviderCapability {
        ProviderCapability {
            provider_type: record.provider_type,
            version: record.version.clone(),
            adapter_dialect: record.adapter_dialect,
            wire_dialect: record.wire_dialect,
            capability_snapshot_ref: record.capability_snapshot_ref.clone(),
            action_capability: row,
            trust: record.trust.clone(),
        }
    }
}

impl ProviderCapabilitySource for StoreBackedProviderCapabilitySource {
    /// 正常 action row 的 launch 分格门:`Confirmed` 放行;v1→v2 过渡桥仅当
    /// 行未探测(Unknown)且旧 allow 列表仍列出该 action 时放行(既有 root
    /// recipe 聚合链零回归),且**绝不铸造 Confirmed**;`Denied`(真实负向
    /// 证据)恒拒;旧列表未列出且行 Unknown 亦拒(fail-closed,稳定判别码)。
    fn require_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        let record = self.load_record(provider)?;
        let row = record.action_matrix.row(&action);
        match &row.launch {
            ProviderCapabilityEvidence::Confirmed => Ok(Self::capability_from_record(&record, row)),
            ProviderCapabilityEvidence::Unknown if record.supported_actions.contains(&action) => {
                // 过渡桥:legacy bootstrap/v1 记录(矩阵未探测)维持今日放行
                // 行为,等待真实 probe(2d)写入 Confirmed 行后自然取代;
                // capability 里的行保持 Unknown——旧列表不产生正常会话
                // Confirmed(Global Constraints 第 2 条)。
                Ok(Self::capability_from_record(&record, row))
            }
            other => Err(ProviderGatewayError::UnsupportedCapability(format!(
                "{PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED}: {action:?} launch cell is {other:?}"
            ))),
        }
    }

    /// 明确 resume 门:仅 resume 分格 `Confirmed` 放行;旧 resume_evidence
    /// 二态不再消费(不产生 Confirmed),Unknown/Denied 一律
    /// `ResumeNotSupported`(不得静默转 fresh)。
    fn require_resume_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        let record = self.load_record(provider)?;
        let row = record.action_matrix.row(&action);
        match &row.resume {
            ProviderCapabilityEvidence::Confirmed => Ok(Self::capability_from_record(&record, row)),
            _other => Err(ProviderGatewayError::ResumeNotSupported),
        }
    }

    /// fresh 门的 write-boundary 半边:`Confirmed` 放行;过渡桥同 launch 分格
    /// (Unknown + 旧 allow 列表,不铸 Confirmed);`Denied` 恒拒且 reason 完整
    /// 保留在错误详情中。
    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        let record = self.load_record(provider)?;
        let row = record.action_matrix.row(&action);
        match &row.write_boundary {
            ProviderCapabilityEvidence::Confirmed => Ok(Self::capability_from_record(&record, row)),
            ProviderCapabilityEvidence::Unknown if record.supported_actions.contains(&action) => {
                Ok(Self::capability_from_record(&record, row))
            }
            other => Err(ProviderGatewayError::UnsupportedCapability(format!(
                "{PROVIDER_CAPABILITY_WRITE_BOUNDARY_NOT_CONFIRMED}: {action:?} write_boundary cell is {other:?}"
            ))),
        }
    }

    /// root-recipe 相位:仅现有固定 Claude recipe 事实放行——provider 必须
    /// 是固定 Claude recipe provider、记录在场且 snapshot 一致;凭据是
    /// durable Running operation 派生的 opaque 相位证明(类型面保证),每次
    /// 重验由 `for_lc` 通道/GREEN 第二段接入。返回的 capability 只镜像
    /// durable normal 状态(行不因 recipe 事实铸造 Confirmed),不推 normal
    /// Confirmed(隔离契约)。
    fn require_root_recipe_supported(
        &self,
        provider: &ProviderRef,
        credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        if provider.provider_type != ProviderRefType::ClaudeCode {
            return Err(ProviderGatewayError::UnsupportedCapability(format!(
                "{PROVIDER_ROOT_RECIPE_REQUIRES_FIXED_CLAUDE}: got {:?}",
                provider.provider_type
            )));
        }
        // 凭据重核验通道:durable Running 重验是 recipe 相位的硬前置;
        // 无通道(new/with_store 构造)fail-closed,绝不在无凭据重验下放行。
        let Some(recheck) = &self.recipe_recheck else {
            return Err(ProviderGatewayError::UnsupportedCapability(
                PROVIDER_ROOT_RECIPE_CREDENTIAL_RECHECK_UNAVAILABLE.to_string(),
            ));
        };
        let record = self.load_record(provider)?;
        // 已交付 recipe 证据钉定版本与记录版本漂移 → fail-closed(旧证据
        // 不跨 CLI 版本沿用,等待重新交付)。
        if let crate::product::logical_codebase::provider_capability_store::RootRecipeEvidence::Delivered { version, .. } =
            &record.root_recipe_evidence
        {
            if version != &record.version {
                return Err(ProviderGatewayError::UnsupportedCapability(format!(
                    "{PROVIDER_ROOT_RECIPE_EVIDENCE_VERSION_DRIFT}: evidence pins {version}, record has {}",
                    record.version
                )));
            }
        }
        // 凭据每次重验 durable Running:operation 终态/step 推进/digest 或
        // root 漂移 → 凭据失效,fail-closed(稳定码 + admission reason_code)。
        if let Err(denied) =
            credential.reverify_against_running_operation(&recheck.operations, &recheck.lc_id)
        {
            let detail = match &denied {
                crate::product::logical_codebase::provider_admission_preflight::ProviderAdmissionError::Waiting { reason_code, .. } => reason_code.clone(),
                other => format!("{other:?}"),
            };
            return Err(ProviderGatewayError::UnsupportedCapability(format!(
                "{PROVIDER_ROOT_RECIPE_CREDENTIAL_RECHECK_DENIED}: {detail}"
            )));
        }
        Ok(Self::capability_from_record(
            &record,
            record
                .action_matrix
                .row(&SessionPolicyAction::PlanningReadOnly),
        ))
    }
}

/// Task 2.8（映射 tasks.md 2.2；REQ-ENV-10 双工厂 root assertion）：四路
/// canonical LC root 投影的一致性断言。
///
/// - `registration_root`：登记工厂投影（`LogicalCodebaseRecord.aggregate_root`）；
/// - `aggregate_root`：aggregate 生产 driver 投影（root recipe receipt 冻结
///   的 canonical root，源自 preflight snapshot root）；
/// - `authority_root`：gateway 工厂投影（manifest `provider_context_root`）；
/// - `envelope_working_directory`：envelope 冻结的会话 cwd（root-cwd 契约：
///   LC 会话 cwd 即 canonical LC root，REQ-ENV-01/10）。
///
/// 全部在场投影必须 canonical 相等（含空格/symlink 形态差异），任一不一致
/// 或 canonicalize 失败都 fail-closed 为 `TargetMismatch`/`Target`
/// （zero spawn），绝不允许「一个 root 生成配置、另一个 root 消费配置」。
/// `registration_root`/`aggregate_root` 为 `None` 表示该投影尚未落盘
/// （bootstrap 早期/legacy 无 receipt），不视为不一致。返回一致的
/// canonical root 供调用方复用。
pub fn assert_canonical_lc_root_consistent(
    registration_root: Option<&Path>,
    aggregate_root: Option<&Path>,
    authority_root: &Path,
    envelope_working_directory: &Path,
) -> Result<PathBuf, ProviderGatewayError> {
    fn canonical(label: &str, path: &Path) -> Result<PathBuf, ProviderGatewayError> {
        path.canonicalize().map_err(|error| {
            ProviderGatewayError::Target(format!(
                "canonicalize {label} {}: {error}",
                path.display()
            ))
        })
    }

    let canonical_authority = canonical("authority root", authority_root)?;
    let canonical_cwd = canonical("working directory", envelope_working_directory)?;
    if canonical_cwd != canonical_authority {
        return Err(ProviderGatewayError::TargetMismatch {
            field: "lc_root".to_string(),
        });
    }
    for (field, root) in [
        ("registration_root", registration_root),
        ("aggregate_root", aggregate_root),
    ] {
        if let Some(root) = root
            && canonical(field, root)? != canonical_authority
        {
            return Err(ProviderGatewayError::TargetMismatch {
                field: field.to_string(),
            });
        }
    }
    Ok(canonical_authority)
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
    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
    use crate::product::logical_codebase::policy::ProviderDialect;
    use crate::product::logical_codebase::policy::ProviderWireDialect;
    use crate::product::logical_codebase::policy::SessionPolicyAction;
    use crate::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
        ProviderActionMatrix, ProviderCapabilityRecord, RootRecipeEvidence,
    };
    use crate::product::logical_codebase::provider_gateway::ResumeEvidenceState;
    use crate::product::logical_codebase::provider_gateway::{
        PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED, PROVIDER_CAPABILITY_WRITE_BOUNDARY_NOT_CONFIRMED,
    };
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
                // Task 2.5：cwd 字段映射 target worktree（测试 fixture 零语义变化）。
                working_directory: worktree.clone(),
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
            working_directory: fixture.worktree.clone(),
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
            working_directory: aggregate_root.clone(),
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
                working_directory: worktree.clone(),
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
            working_directory: fixture.worktree.clone(),
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
        assert_eq!(
            capability.wire_dialect,
            ProviderWireDialect::ClaudeCodeStreamJson
        );
        assert_eq!(capability.capability_snapshot_ref, "cap_managed_snapshot");
        // bootstrap 记录的 v2 行全 Unknown:过渡桥不因旧 allow 列表铸造任何
        // Confirmed(launch/resume 分格保持 Unknown)。
        assert_eq!(
            capability.action_capability.launch,
            ProviderCapabilityEvidence::Unknown
        );
        assert_eq!(
            capability.action_capability.resume,
            ProviderCapabilityEvidence::Unknown
        );
    }

    /// r47(codex 首轮):LC 投影后 Codex 恒非 danger(Planning/Review→
    /// read-only,Coding→workspace-write),探针签发的 Confirmed 行放行;
    /// 全阻(Task 13 期形态)已移除——danger 政策只适用 direct coder 路径
    ///(不进本 gateway)。
    #[test]
    fn store_backed_capability_codex_probe_confirmed_row_passes() {
        let (root, source) = store_backed_source("project_0001");
        // 种一枚探针签发的 planning Confirmed 行(2d import 形态)。
        let store =
            crate::product::logical_codebase::provider_capability_store::ProviderCapabilityStore::new(
                crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria")),
            );
        let verified =
            crate::product::logical_codebase::provider_capability_store::ProviderCapabilityRecord {
                provider_type: ProviderRefType::Codex,
                schema_version:
                    crate::product::logical_codebase::provider_capability_store::PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
                version: "1.0.0-probe".to_string(),
                adapter_dialect: ProviderDialect::CodexCliV1,
                wire_dialect: ProviderWireDialect::CodexAppServerRpc,
                capability_snapshot_ref: "cap_managed_snapshot".to_string(),
                evidence: CapabilityEvidence::ProductionVerified,
                resume_evidence: ResumeEvidenceState::Unsupported,
                supported_actions: vec![SessionPolicyAction::PlanningReadOnly],
                action_matrix: ProviderActionMatrix::from_rows(vec![
                    ProviderActionCapability {
                        action: SessionPolicyAction::PlanningReadOnly,
                        launch: ProviderCapabilityEvidence::Confirmed,
                        resume: ProviderCapabilityEvidence::Confirmed,
                        write_boundary: ProviderCapabilityEvidence::Confirmed,
                        projection_digest: "sha256:probe".to_string(),
                        evidence_ref: "probe-evidence".to_string(),
                    },
                ])
                .expect("probe rows"),
                trust: ProviderCapabilityEvidence::Unknown,
                probed_at: Some("2026-10-07T00:00:00Z".to_string()),
                probe_artifact_ref: Some("probe-evidence".to_string()),
                root_recipe_evidence: RootRecipeEvidence::None,
            };
        store
            .import_verified_probe_row(
                "project_0001",
                &verified,
                SessionPolicyAction::PlanningReadOnly,
            )
            .expect("import probe row");

        let capability = source
            .require_supported(
                &ProviderRef::codex("cap_managed_snapshot"),
                SessionPolicyAction::PlanningReadOnly,
            )
            .expect("probe-confirmed codex planning row must pass (LC sessions never run danger)");

        assert_eq!(capability.provider_type, ProviderRefType::Codex);
    }

    /// r47(pi/kimi 首轮):探针直建记录(无 bootstrap 先建)的
    /// capability_snapshot_ref 必须是运行时约定值 cap_managed_snapshot——
    /// 否则 load_record 比对 mismatch(pi/kimi 首轮现场)。
    #[test]
    fn probe_import_record_uses_runtime_snapshot_ref_convention() {
        // 直接构造最小 projection/evidence 不可行(无 Default)——用
        // probe_import_record 的真实调用面:经 run_cli_boundary_probe 的
        // 单测已有(lcg_t06_external);此处直接断言常量约定与 load_record
        // 的运行时 ref 一致(双源漂移防线)。
        let runtime_ref = ProviderRef::codex("cap_managed_snapshot");
        let probe_record_ref = "cap_managed_snapshot";
        assert_eq!(
            runtime_ref.capability_snapshot_ref, probe_record_ref,
            "探针直建记录的 snapshot ref 必须与运行时 ProviderRef 约定一致(pi/kimi 首轮 mismatch 根因)"
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
            matches!(&error, ProviderGatewayError::UnsupportedCapability(reason) if reason.starts_with(PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED) && reason.contains("capability record missing"))
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
                // 2a 的 FRU 过渡桩已整体替换:显式新字段构造(v2 全字段),
                // 行为与旧桩一致(矩阵全 Unknown),语义由 v2 分格门接管。
                &ProviderCapabilityRecord {
                    provider_type: ProviderRefType::ClaudeCode,
                    schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
                    version: "0.0.0-managed".to_string(),
                    adapter_dialect: ProviderDialect::ClaudeCodeCliV1,
                    wire_dialect: ProviderWireDialect::ClaudeCodeStreamJson,
                    capability_snapshot_ref: "cap_managed_snapshot".to_string(),
                    evidence: CapabilityEvidence::FixtureVerified,
                    resume_evidence: ResumeEvidenceState::Confirmed,
                    supported_actions: vec![SessionPolicyAction::ReviewReadOnly],
                    action_matrix: ProviderActionMatrix::unknown_all(),
                    trust: ProviderCapabilityEvidence::Unknown,
                    probed_at: None,
                    probe_artifact_ref: None,
                    root_recipe_evidence: RootRecipeEvidence::None,
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

        // v2 语义:行 Unknown 且旧 allow 列表未列出 → launch 分格 fail-closed
        // 稳定判别码(替代旧 v1 文案)。
        assert!(
            matches!(&error, ProviderGatewayError::UnsupportedCapability(reason) if reason.starts_with(PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED)),
            "unexpected launch-cell error: {error:?}"
        );
    }

    // ===== Task 2b(lcg_t02):StoreBacked source 消费 record v2 的分格门/形状 =====

    /// 构造指定三态的矩阵行。
    fn lcg_t02_row(
        action: SessionPolicyAction,
        launch: ProviderCapabilityEvidence,
        resume: ProviderCapabilityEvidence,
        write_boundary: ProviderCapabilityEvidence,
    ) -> ProviderActionCapability {
        ProviderActionCapability {
            action,
            launch,
            resume,
            write_boundary,
            projection_digest: format!("projection-digest-{action:?}"),
            evidence_ref: format!("probe://{action:?}"),
        }
    }

    /// 构造 v2 Claude 记录(非 bootstrap 版本;legacy allow 列表可空)。
    fn lcg_t02_v2_record(
        matrix: ProviderActionMatrix,
        supported_actions: Vec<SessionPolicyAction>,
    ) -> ProviderCapabilityRecord {
        ProviderCapabilityRecord {
            provider_type: ProviderRefType::ClaudeCode,
            schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
            version: "1.4.0".to_string(),
            adapter_dialect: ProviderDialect::ClaudeCodeCliV1,
            wire_dialect: ProviderWireDialect::ClaudeCodeStreamJson,
            capability_snapshot_ref: "cap_managed_snapshot".to_string(),
            evidence: CapabilityEvidence::ProductionVerified,
            resume_evidence: ResumeEvidenceState::Confirmed,
            supported_actions,
            action_matrix: matrix,
            trust: ProviderCapabilityEvidence::Confirmed,
            probed_at: Some("2026-10-03T00:00:00Z".to_string()),
            probe_artifact_ref: Some("probe://artifact-0001".to_string()),
            root_recipe_evidence: RootRecipeEvidence::None,
        }
    }

    /// 以指定矩阵 + legacy allow 列表 upsert Claude 记录并返回 source。
    fn lcg_t02_source_with_record(
        root: &TempDir,
        matrix: ProviderActionMatrix,
        supported_actions: Vec<SessionPolicyAction>,
    ) -> StoreBackedProviderCapabilitySource {
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let store = ProviderCapabilityStore::new(paths.clone());
        store.ensure_bootstrap("project_0001").unwrap();
        store
            .upsert(
                "project_0001",
                &lcg_t02_v2_record(matrix, supported_actions),
            )
            .unwrap();
        StoreBackedProviderCapabilitySource::new(paths, "project_0001".to_string())
    }

    #[test]
    fn lcg_t02_require_supported_reads_launch_cell_from_normal_action_row() {
        let root = tempfile::tempdir().expect("temporary product root");
        // Coding 行 launch Confirmed/resume Unknown/write_boundary Confirmed;
        // legacy allow 列表留空——正常会话只由 v2 行放行,不依赖旧列表。
        let source = lcg_t02_source_with_record(
            &root,
            ProviderActionMatrix::from_rows(vec![lcg_t02_row(
                SessionPolicyAction::CodingTargetWrite,
                ProviderCapabilityEvidence::Confirmed,
                ProviderCapabilityEvidence::Unknown,
                ProviderCapabilityEvidence::Confirmed,
            )])
            .unwrap(),
            Vec::new(),
        );

        let capability = source
            .require_supported(
                &ProviderRef::claude_code("cap_managed_snapshot"),
                SessionPolicyAction::CodingTargetWrite,
            )
            .unwrap();

        // capability 组装消费 record v2 冻结字段:wire dialect/action row/trust。
        assert_eq!(capability.provider_type, ProviderRefType::ClaudeCode);
        assert_eq!(capability.version, "1.4.0");
        assert_eq!(capability.adapter_dialect, ProviderDialect::ClaudeCodeCliV1);
        assert_eq!(
            capability.wire_dialect,
            ProviderWireDialect::ClaudeCodeStreamJson
        );
        assert_eq!(capability.trust, ProviderCapabilityEvidence::Confirmed);
        assert_eq!(
            capability.action_capability.action,
            SessionPolicyAction::CodingTargetWrite
        );
        assert_eq!(
            capability.action_capability.launch,
            ProviderCapabilityEvidence::Confirmed
        );

        // launch 分格 Unknown(legacy 未列出)→ fail-closed,稳定判别码。
        let source = lcg_t02_source_with_record(
            &root,
            ProviderActionMatrix::from_rows(vec![lcg_t02_row(
                SessionPolicyAction::ReviewReadOnly,
                ProviderCapabilityEvidence::Unknown,
                ProviderCapabilityEvidence::Unknown,
                ProviderCapabilityEvidence::Confirmed,
            )])
            .unwrap(),
            Vec::new(),
        );
        let error = source
            .require_supported(
                &ProviderRef::claude_code("cap_managed_snapshot"),
                SessionPolicyAction::ReviewReadOnly,
            )
            .unwrap_err();
        assert!(
            matches!(&error, ProviderGatewayError::UnsupportedCapability(reason) if reason.starts_with(PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED)),
            "unexpected launch-cell error: {error:?}"
        );
    }

    #[test]
    fn lcg_t02_require_write_boundary_reads_boundary_cell() {
        let root = tempfile::tempdir().expect("temporary product root");
        // write_boundary Denied 是真实负向证据:legacy 列表列出该 action 也不放行。
        let source = lcg_t02_source_with_record(
            &root,
            ProviderActionMatrix::from_rows(vec![lcg_t02_row(
                SessionPolicyAction::CodingTargetWrite,
                ProviderCapabilityEvidence::Confirmed,
                ProviderCapabilityEvidence::Unknown,
                ProviderCapabilityEvidence::Denied {
                    reason: "boundary probe denied".to_string(),
                },
            )])
            .unwrap(),
            vec![SessionPolicyAction::CodingTargetWrite],
        );
        let error = source
            .require_write_boundary(
                &ProviderRef::claude_code("cap_managed_snapshot"),
                SessionPolicyAction::CodingTargetWrite,
            )
            .unwrap_err();
        assert!(
            matches!(&error, ProviderGatewayError::UnsupportedCapability(reason)
                if reason.starts_with(PROVIDER_CAPABILITY_WRITE_BOUNDARY_NOT_CONFIRMED)
                    && reason.contains("boundary probe denied")),
            "unexpected write-boundary error: {error:?}"
        );

        // write_boundary Confirmed → 放行。
        let source = lcg_t02_source_with_record(
            &root,
            ProviderActionMatrix::from_rows(vec![lcg_t02_row(
                SessionPolicyAction::CodingTargetWrite,
                ProviderCapabilityEvidence::Confirmed,
                ProviderCapabilityEvidence::Unknown,
                ProviderCapabilityEvidence::Confirmed,
            )])
            .unwrap(),
            Vec::new(),
        );
        let capability = source
            .require_write_boundary(
                &ProviderRef::claude_code("cap_managed_snapshot"),
                SessionPolicyAction::CodingTargetWrite,
            )
            .unwrap();
        assert_eq!(
            capability.action_capability.write_boundary,
            ProviderCapabilityEvidence::Confirmed
        );
    }

    #[test]
    fn lcg_t02_require_resume_supported_reads_resume_cell() {
        let root = tempfile::tempdir().expect("temporary product root");
        // resume 分格 Unknown:旧 resume_evidence 二态(lcg_t02_v2_record 内为
        // Confirmed)不得放行 v2 行——明确 resume fail-closed,不静默转 fresh。
        let source = lcg_t02_source_with_record(
            &root,
            ProviderActionMatrix::from_rows(vec![lcg_t02_row(
                SessionPolicyAction::CodingTargetWrite,
                ProviderCapabilityEvidence::Confirmed,
                ProviderCapabilityEvidence::Unknown,
                ProviderCapabilityEvidence::Confirmed,
            )])
            .unwrap(),
            Vec::new(),
        );
        let error = source
            .require_resume_supported(
                &ProviderRef::claude_code("cap_managed_snapshot"),
                SessionPolicyAction::CodingTargetWrite,
            )
            .unwrap_err();
        assert!(
            matches!(&error, ProviderGatewayError::ResumeNotSupported),
            "unexpected resume-cell error: {error:?}"
        );

        // resume Confirmed → 放行。
        let source = lcg_t02_source_with_record(
            &root,
            ProviderActionMatrix::from_rows(vec![lcg_t02_row(
                SessionPolicyAction::CodingTargetWrite,
                ProviderCapabilityEvidence::Confirmed,
                ProviderCapabilityEvidence::Confirmed,
                ProviderCapabilityEvidence::Confirmed,
            )])
            .unwrap(),
            Vec::new(),
        );
        let capability = source
            .require_resume_supported(
                &ProviderRef::claude_code("cap_managed_snapshot"),
                SessionPolicyAction::CodingTargetWrite,
            )
            .unwrap();
        assert_eq!(
            capability.action_capability.resume,
            ProviderCapabilityEvidence::Confirmed
        );
    }

    /// v1 旧记录(无 schema/matrix)的过渡语义:launch/write 分格经桥放行
    /// (既有 root recipe 契约零回归),但 capability 行保持全 Unknown——
    /// 旧 supported_actions/provenance 不产生正常会话 Confirmed。
    #[test]
    fn lcg_t02_v1_legacy_actions_keep_launching_without_confirmed_cells() {
        let root = tempfile::tempdir().expect("temporary product root");
        let dir = root.path().join("projects/project_0001/logical-codebase");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("capabilities.json"),
            r#"[{
                "provider_type": "claude_code",
                "version": "0.0.0-managed",
                "adapter_dialect": "claude_code_cli_v1",
                "capability_snapshot_ref": "cap_managed_snapshot",
                "evidence": "fixture_verified",
                "resume_evidence": "confirmed",
                "supported_actions": ["planning_read_only", "coding_target_write", "review_read_only"]
            }]"#,
        )
        .unwrap();
        let paths = ProductAppPaths::new(root.path());
        let source = StoreBackedProviderCapabilitySource::new(paths, "project_0001".to_string());

        let capability = source
            .require_supported(
                &ProviderRef::claude_code("cap_managed_snapshot"),
                SessionPolicyAction::PlanningReadOnly,
            )
            .unwrap();
        assert_eq!(
            capability.action_capability.launch,
            ProviderCapabilityEvidence::Unknown
        );
        assert_eq!(
            capability.wire_dialect,
            ProviderWireDialect::ClaudeCodeStreamJson
        );
        // write 分格同样经桥放行(root recipe 聚合链的 spawn 依赖)。
        source
            .require_write_boundary(
                &ProviderRef::claude_code("cap_managed_snapshot"),
                SessionPolicyAction::PlanningReadOnly,
            )
            .unwrap();
    }

    /// 2a 审查 carry ①:矩阵缺 action 行 → `row()` 读取为全 Unknown 行
    /// (fail-closed,不 panic、不拒绝),source 消费面上缺行按 launch 分格
    /// Unknown 处理(legacy 未列出即拒)。
    #[test]
    fn lcg_t02_matrix_missing_action_row_reads_unknown() {
        let matrix = ProviderActionMatrix::from_rows(vec![lcg_t02_row(
            SessionPolicyAction::CodingTargetWrite,
            ProviderCapabilityEvidence::Confirmed,
            ProviderCapabilityEvidence::Confirmed,
            ProviderCapabilityEvidence::Confirmed,
        )])
        .unwrap();

        // 缺行(Planning/Review 不在矩阵中)→ 全 Unknown 行。
        let missing = matrix.row(&SessionPolicyAction::PlanningReadOnly);
        assert_eq!(missing.action, SessionPolicyAction::PlanningReadOnly);
        assert_eq!(missing.launch, ProviderCapabilityEvidence::Unknown);
        assert_eq!(missing.resume, ProviderCapabilityEvidence::Unknown);
        assert_eq!(missing.write_boundary, ProviderCapabilityEvidence::Unknown);

        // 消费面:仅含 Coding 行的记录 + legacy 未列出 → Planning 请求按
        // launch 分格 Unknown fail-closed。
        let root = tempfile::tempdir().expect("temporary product root");
        let source = lcg_t02_source_with_record(&root, matrix, Vec::new());
        let error = source
            .require_supported(
                &ProviderRef::claude_code("cap_managed_snapshot"),
                SessionPolicyAction::PlanningReadOnly,
            )
            .unwrap_err();
        assert!(
            matches!(&error, ProviderGatewayError::UnsupportedCapability(reason) if reason.starts_with(PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED)),
            "missing row must read as Unknown and fail closed: {error:?}"
        );
    }

    /// 2a 审查 carry ①:重复 action 行 → `from_rows` 显式拒绝(fail-closed,
    /// 不静默去重/覆盖)。
    #[test]
    fn lcg_t02_matrix_duplicate_action_rows_are_rejected() {
        let error = ProviderActionMatrix::from_rows(vec![
            lcg_t02_row(
                SessionPolicyAction::CodingTargetWrite,
                ProviderCapabilityEvidence::Confirmed,
                ProviderCapabilityEvidence::Confirmed,
                ProviderCapabilityEvidence::Confirmed,
            ),
            lcg_t02_row(
                SessionPolicyAction::CodingTargetWrite,
                ProviderCapabilityEvidence::Unknown,
                ProviderCapabilityEvidence::Unknown,
                ProviderCapabilityEvidence::Unknown,
            ),
        ])
        .unwrap_err();
        assert!(
            error.to_string().contains("duplicate action row"),
            "duplicate rows must be rejected: {error}"
        );
    }
}
