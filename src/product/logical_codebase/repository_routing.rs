use sha2::Digest as _;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::logical_codebase::issue_selection::{
    IssueCodebaseSelection, IssueCodebaseSelectionStore,
};
use crate::product::logical_codebase::store::{LogicalCodebaseManifest, LogicalCodebaseStore};

/// 稳定错误码（B3）：fail-closed 的机器可读分类，HTTP 映射见 Task 3（error.rs/support.rs）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryRoutingErrorCode {
    /// (Some, None)：manifest 存在但 selection 缺失 → 数据不完整
    TargetMissing,
    /// (None, Some)：孤立 selection，无 manifest → 数据损坏
    OrphanedSelection,
    /// target 指向不存在/非 active 成员
    TargetUnknown,
    /// 多目标 group 无唯一 target / 无唯一 resolve
    TargetAmbiguous,
    /// manifest/member/checkout/snapshot 权威不一致
    Inconsistent,
    /// 成员删除/停用（tombstone）
    MemberRemoved,
    /// selection 已失效（invalidation）
    SelectionInvalidated,
}

impl RepositoryRoutingErrorCode {
    /// 稳定错误码字符串（B3）：各 Web 入口 fail-closed 的机器可读分类，
    /// 用于 HTTP 响应 `code` 字段。所有入口必须经此方法，避免字面量重复导致漂移。
    pub fn stable_code(&self) -> &'static str {
        match self {
            RepositoryRoutingErrorCode::TargetMissing => "repository_routing_target_missing",
            RepositoryRoutingErrorCode::OrphanedSelection
            | RepositoryRoutingErrorCode::Inconsistent
            | RepositoryRoutingErrorCode::MemberRemoved
            | RepositoryRoutingErrorCode::SelectionInvalidated => "repository_routing_inconsistent",
            RepositoryRoutingErrorCode::TargetUnknown => "repository_routing_target_unknown",
            RepositoryRoutingErrorCode::TargetAmbiguous => "repository_routing_ambiguous",
        }
    }
}

/// Web 运行时统一 repository 分流判定（REQ-ROUTE-01）。
/// 以 (manifest, selection) 成对状态为唯一权威信号，返回显式三态。
pub enum RepositoryRouting {
    /// (None, None)：无 manifest 且无 selection → 物理 RepositoryRecord.id 解析（改动前行为）。
    ///
    /// C5（single-repository-automation-entry）口径澄清：`Legacy` 即
    /// **单仓（single-repository）** 语义——issue 未持久归属逻辑代码库时，
    /// 权威载体是其 `repo_id` 指向的真实物理仓。命名保留 `Legacy`
    /// （不改名，见 Non-Goals）：它描述的是先于逻辑代码库分流存在的
    /// 物理仓解析路径，而非 deprecated 分支。
    Legacy { repository_id: String },
    /// (Some, Some)：有 manifest 且有有效 selection → 逻辑解析（由调用方按 target/snapshot 定具体成员）。
    Logical {
        manifest: LogicalCodebaseManifest,
        selection: Box<IssueCodebaseSelection>,
    },
    /// 其余一切不一致状态 → 明确错误，稳定错误码 + 可诊断 reason，绝不静默回退物理仓库。
    FailClosed {
        code: RepositoryRoutingErrorCode,
        reason: String,
    },
}

/// 解析 issue 唯一归属的代码库（v1.3）：返回 issue 记录里持久化的
/// `logical_codebase_id`（Some=逻辑代码库，None=单仓或旧数据）。issue 不存在时
/// 返回 None（路由交由 manifest/selection 存在性继续判定，不静默伪造）。
pub fn resolve_issue_logical_codebase_id(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
) -> Result<Option<String>, ProductStoreError> {
    match crate::product::issue_store::IssueStore::new(app_paths.clone()).get(project_id, issue_id)
    {
        Ok(issue) => Ok(issue.logical_codebase_id),
        Err(ProductStoreError::NotFound { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

/// 据 lc_id 构造 manifest/selection 的权威 store 对；None 保持旧 project 级路径。
fn issue_codebase_stores(
    app_paths: &ProductAppPaths,
    lc_id: Option<&str>,
) -> (LogicalCodebaseStore, IssueCodebaseSelectionStore) {
    match lc_id {
        Some(lc_id) => (
            LogicalCodebaseStore::for_lc(app_paths.clone(), lc_id),
            IssueCodebaseSelectionStore::for_lc(app_paths.clone(), lc_id),
        ),
        None => (
            LogicalCodebaseStore::new(app_paths.clone()),
            IssueCodebaseSelectionStore::new(app_paths.clone()),
        ),
    }
}

impl RepositoryRouting {
    /// 纯判定，不加载 store（便于单测）；加载在 `load_for_issue` 中完成。
    pub fn classify(
        manifest: Option<LogicalCodebaseManifest>,
        selection: Option<IssueCodebaseSelection>,
    ) -> Self {
        match (manifest, selection) {
            (None, None) => RepositoryRouting::Legacy {
                repository_id: String::new(),
            }, // repository_id 由调用方从 entity 取
            (Some(manifest), Some(selection)) => RepositoryRouting::Logical {
                manifest,
                selection: Box::new(selection),
            },
            (Some(_), None) => RepositoryRouting::FailClosed {
                code: RepositoryRoutingErrorCode::TargetMissing,
                reason: "work_item_target_missing: logical codebase manifest and issue selection must both exist".to_string(),
            },
            (None, Some(_)) => RepositoryRouting::FailClosed {
                code: RepositoryRoutingErrorCode::OrphanedSelection,
                reason: "orphaned_issue_selection: issue selection exists without logical codebase manifest".to_string(),
            },
        }
    }

    /// 加载辅助（B6）：按 issue 所属 codebase（v1.3）经 store 加载 manifest + selection
    /// 后交 `classify`。逻辑 issue 的 manifest/selection 均取 lc_id 子树；单仓 issue
    /// 与旧数据回退 project 级路径。
    pub fn load_for_issue(
        app_paths: &ProductAppPaths,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Self, ProductStoreError> {
        let lc_id = resolve_issue_logical_codebase_id(app_paths, project_id, issue_id)?;
        let (logical, selections) = issue_codebase_stores(app_paths, lc_id.as_deref());
        let manifest = logical.load_manifest(project_id)?;
        let selection = selections.load(project_id, issue_id)?;
        Ok(Self::classify(manifest, selection))
    }
}

// ---- Task 1：唯一 authority resolver（显式 kind + fail-closed 冲突）----

use std::path::PathBuf;

use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexError, AggregateIndexStatus, AggregateIndexStore,
};
use crate::product::logical_codebase::policy::{
    AggregatePolicyArtifact, AggregatePolicyArtifactStore,
};
use crate::product::logical_codebase::types::{LogicalRepositoryId, RepositoryCheckoutId};

/// 代码库 target kind（C4）：`single_repo` 与 `logical` 同级且互斥。
/// kind 缺失、与 issue/enrollment 不符、target 身份不一致或跨 kind 访问
/// 一律 fail-closed，返回结构化冲突与重新选择/核验动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryTargetKind {
    SingleRepo,
    LogicalCodebase,
}

/// 所有选择/登记/checkout/policy/index/provider 读取的唯一入口请求。
/// 调用方必须显式给出 kind 与目标身份；resolver 不做路径猜测或
/// “最新可用记录”回退。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRoutingRequest {
    pub project_id: String,
    pub issue_id: Option<String>,
    pub kind: RepositoryTargetKind,
    pub repository_id: Option<String>,
    pub logical_codebase_id: Option<String>,
    pub logical_repository_id: Option<LogicalRepositoryId>,
    pub checkout_id: Option<RepositoryCheckoutId>,
}

/// resolver 冻结后的 target 身份：kind、ids、canonical path 与
/// source identity digest。后续 policy/envelope 漂移断言以此为基线。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedTargetIdentity {
    pub kind: RepositoryTargetKind,
    pub repository_id: Option<String>,
    pub logical_codebase_id: Option<String>,
    pub logical_repository_id: Option<LogicalRepositoryId>,
    pub checkout_id: Option<RepositoryCheckoutId>,
    pub canonical_path: PathBuf,
    pub source_identity_digest: String,
}

/// authority 子树内聚合 policy artifact 的冻结引用。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthorityPolicyReference {
    pub policy_id: String,
    pub policy_revision: u64,
    pub policy_digest: String,
    pub artifact_root: PathBuf,
}

/// authority 子树内 aggregate index 的只读引用；冷启动允许全空。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthorityAggregateIndexReference {
    pub aggregate_index_id: Option<String>,
    pub membership_revision: Option<u64>,
    pub status: Option<AggregateIndexStatus>,
}

/// resolver 输出：authority root + 冻结身份 + manifest/selection/policy/index
/// 事实。所有事实都来自 `logical-codebases/{lc_id}/` 权威子树（legacy 别名
/// LC 除外，它保持 legacy root 字节等价）；单仓只从物理仓事实解析。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RepositoryAuthorityResolution {
    pub authority_root: PathBuf,
    pub target: ResolvedTargetIdentity,
    pub manifest: Option<LogicalCodebaseManifest>,
    pub selection: Option<IssueCodebaseSelection>,
    pub policy: Option<AuthorityPolicyReference>,
    pub aggregate_index: AuthorityAggregateIndexReference,
}

/// 唯一显式 kind/authority resolver。只读：任何冲突只返回结构化
/// `ProductStoreError`，不写任何 store，也不调用 `default_logical_codebase_id`
/// 之类的 fallback。
pub struct RepositoryAuthorityResolver {
    paths: ProductAppPaths,
}

impl RepositoryAuthorityResolver {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    /// C4 Task 2：issue 归属驱动的显式 authority 解析（各读取入口迁移的唯一助手）。
    ///
    /// - issue 持久化归属 LC（`IssueRecord.logical_codebase_id = Some`）→ 显式
    ///   `LogicalCodebase` 请求，kind/身份/重复来源/legacy 布局冲突全部由
    ///   `resolve` fail-closed 校验。
    /// - issue 无归属但 legacy 别名 LC record 存在（`ProjectStore::get/list`
    ///   幂等 `migrate_legacy` 的产物）→ 显式解析 legacy 别名 LC，其子树与旧
    ///   project 级布局字节等价；不猜“最新可用记录”。
    /// - 两者皆无（单仓 issue、未迁移旧数据）→ `Ok(None)`：调用方保留
    ///   single-repo/legacy 兼容路径，但不得把物理仓伪装为 LC target。
    pub fn resolve_for_issue(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Option<RepositoryAuthorityResolution>, ProductStoreError> {
        // 与 `resolve_issue_logical_codebase_id` 同语义的宽容读取：issue 记录
        // 不存在时不伪造归属（legacy 兼容：旧数据/裸 fixture 只带 issue 字符串
        // id），其余读取错误原样传播。
        let issue_record = crate::product::issue_store::IssueStore::new(self.paths.clone())
            .get(project_id, issue_id);
        let (issue_exists, attributed) = match issue_record {
            Ok(issue) => (true, issue.logical_codebase_id),
            Err(ProductStoreError::NotFound { .. }) => (false, None),
            Err(error) => return Err(error),
        };
        // 项目记录存在时先幂等触发 `migrate_legacy`（`ProjectStore::get` 内
        // 置，与所有既有入口一致）：legacy 布局存在则别名 LC record 必在。
        // 无项目记录的裸 fixture/旧数据保持 `None` 兼容分支，不伪造 LC。
        match crate::product::project_store::ProjectStore::new(self.paths.clone())
            .get(project_id)
        {
            Ok(_) => {}
            Err(ProductStoreError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(error),
        }
        let alias_record_exists = self
            .paths
            .logical_codebase_record_root(
                project_id,
                &crate::product::logical_codebase::store::legacy_logical_codebase_id(
                    project_id,
                ),
            )
            .join("record.json")
            .try_exists()
            .map_err(|error| {
                ProductStoreError::Io(format!("try_exists alias record: {error}"))
            })?;
        let lc_id = attributed.or_else(|| {
            alias_record_exists.then(|| {
                crate::product::logical_codebase::store::legacy_logical_codebase_id(project_id)
            })
        });
        let Some(lc_id) = lc_id else {
            return Ok(None);
        };
        // `resolve` 要求携带的 issue 必须存在（Task 1 语义）；issue 记录缺失的
        // legacy 数据不带 issue_id 解析，随后从同一 LC 子树宽容补读 selection，
        // 保持 `load_for_issue` 时代 (manifest, selection) 成对判定的兼容性。
        let mut resolution = self.resolve(RepositoryRoutingRequest {
            project_id: project_id.to_string(),
            issue_id: issue_exists.then(|| issue_id.to_string()),
            kind: RepositoryTargetKind::LogicalCodebase,
            repository_id: None,
            logical_codebase_id: Some(lc_id.clone()),
            logical_repository_id: None,
            checkout_id: None,
        })?;
        if !issue_exists && resolution.selection.is_none() {
            resolution.selection = crate::product::logical_codebase::IssueCodebaseSelectionStore::for_lc(
                self.paths.clone(),
                &lc_id,
            )
            .load(project_id, issue_id)?;
        }
        Ok(Some(resolution))
    }

    pub fn resolve(
        &self,
        request: RepositoryRoutingRequest,
    ) -> Result<RepositoryAuthorityResolution, ProductStoreError> {
        validate_relative_id(&request.project_id)?;
        crate::product::project_store::ProjectStore::new(self.paths.clone())
            .get(&request.project_id)?;

        // issue attribution：请求携带 issue 时必须存在，且其持久化的
        // codebase 归属与请求 kind 一致；不符一律 kind_mismatch。
        let mut attributed_lc: Option<String> = None;
        if let Some(issue_id) = request.issue_id.as_deref() {
            let issue =
                crate::product::issue_store::IssueStore::new(self.paths.clone())
                    .get(&request.project_id, issue_id)?;
            attributed_lc = issue.logical_codebase_id.clone();
        }

        match request.kind {
            RepositoryTargetKind::LogicalCodebase => {
                self.resolve_logical(&request, attributed_lc.as_deref())
            }
            RepositoryTargetKind::SingleRepo => {
                self.resolve_single_repo(&request, attributed_lc.as_deref())
            }
        }
    }

    fn resolve_logical(
        &self,
        request: &RepositoryRoutingRequest,
        attributed_lc: Option<&str>,
    ) -> Result<RepositoryAuthorityResolution, ProductStoreError> {
        let Some(lc_id) = request.logical_codebase_id.as_deref() else {
            return Err(ProductStoreError::Conflict {
                kind: "repository_routing_kind_mismatch",
                id: format!("logical_codebase_id:missing:issue:{:?}", request.issue_id),
            });
        };
        validate_relative_id(lc_id)?;
        if request.repository_id.is_some() {
            return Err(ProductStoreError::Conflict {
                kind: "repository_routing_kind_mismatch",
                id: format!(
                    "repository_id:{}:logical_codebase:{lc_id}",
                    request.repository_id.clone().unwrap_or_default()
                ),
            });
        }
        if let Some(attributed) = attributed_lc
            && attributed != lc_id
        {
            return Err(ProductStoreError::Conflict {
                kind: "repository_routing_kind_mismatch",
                id: format!(
                    "issue:{}:attributed_logical_codebase:{attributed}:requested:{lc_id}",
                    request.issue_id.clone().unwrap_or_default()
                ),
            });
        }

        // LC record 直读（不经 `migrate_legacy`，resolver 保持零写入）。
        let record_root = self
            .paths
            .logical_codebase_record_root(&request.project_id, lc_id);
        let record_path = record_root.join("record.json");
        let record_exists = record_path
            .try_exists()
            .map_err(|error| {
                ProductStoreError::Io(format!("try_exists {}: {error}", record_path.display()))
            })?
            && !record_root.join("tombstone.json").exists();
        if !record_exists {
            return Err(ProductStoreError::NotFound {
                kind: "logical_codebase",
                id: lc_id.to_string(),
            });
        }
        let record: crate::product::logical_codebase::store::LogicalCodebaseRecord =
            crate::product::json_store::read_json(&record_path)?;

        let logical = LogicalCodebaseStore::for_lc(self.paths.clone(), lc_id);
        let manifest = logical.load_manifest(&request.project_id)?;
        // authority root 与 `SessionPolicyEnvelope.authority_root` 同义：LC 的
        // 聚合政策权威根 = manifest.provider_context_root（冷启动无 manifest 时
        // 取 LC record 的 aggregate_root）。manifest/record 均来自请求 LC 子树，
        // 误读他 LC 立即产生不同 root。目录尚不存在（冷启动 LC）时保留原始
        // 路径——与 `LogicalCodebaseGatewayFactory` 的构造语义一致；spawn 前
        // 复验由 provider admission 负责，不在此 fail-closed。
        let authority_root_path = manifest
            .as_ref()
            .map(|manifest| manifest.provider_context_root.clone())
            .unwrap_or_else(|| record.aggregate_root.clone());
        let authority_root = std::fs::canonicalize(&authority_root_path)
            .unwrap_or(authority_root_path);
        let members = logical.list_members(&request.project_id)?;
        let checkouts = logical.list_checkouts(&request.project_id)?;

        // 同一 git-dir/来源两个别名成员：重复候选 fail-closed。
        let mut seen_sources = std::collections::BTreeMap::new();
        for member in &members {
            if member.status != crate::product::logical_codebase::types::MemberStatus::Active {
                continue;
            }
            let digest = member.source_identity.key_digest.clone();
            if seen_sources
                .insert(digest.clone(), member.logical_repository_id)
                .is_some()
            {
                return Err(ProductStoreError::Conflict {
                    kind: "repository_routing_source_identity_mismatch",
                    id: digest,
                });
            }
        }

        // 旧 project-level 布局与新 LC 子树来源身份冲突：不猜、不回退。
        let legacy_root = self.paths.logical_codebase_root(&request.project_id);
        let legacy_manifest_exists = legacy_root.join("manifest.json").exists();
        if legacy_manifest_exists
            && lc_id
                != crate::product::logical_codebase::store::legacy_logical_codebase_id(
                    &request.project_id,
                )
        {
            let legacy_members =
                LogicalCodebaseStore::new(self.paths.clone()).list_members(&request.project_id)?;
            let requested_sources: std::collections::BTreeSet<&str> = members
                .iter()
                .filter(|member| {
                    member.status
                        == crate::product::logical_codebase::types::MemberStatus::Active
                })
                .map(|member| member.source_identity.key_digest.as_str())
                .collect();
            if let Some(conflict) = legacy_members.iter().find(|member| {
                member.status
                    == crate::product::logical_codebase::types::MemberStatus::Active
                    && requested_sources.contains(member.source_identity.key_digest.as_str())
            }) {
                return Err(ProductStoreError::Conflict {
                    kind: "repository_routing_legacy_conflict",
                    id: conflict.source_identity.key_digest.clone(),
                });
            }
        }

        let (canonical_path, source_identity_digest, member_id, checkout_id) =
            resolve_logical_target(request, &manifest, &members, &checkouts, &record.aggregate_root)?;

        let selection = match request.issue_id.as_deref() {
            Some(issue_id) => IssueCodebaseSelectionStore::for_lc(
                self.paths.clone(),
                lc_id,
            )
            .load(&request.project_id, issue_id)?,
            None => None,
        };

        let policy = AggregatePolicyArtifactStore::for_lc(self.paths.clone(), lc_id)
            .get(&request.project_id)?
            .map(|artifact| {
                if let Some(manifest) = manifest.as_ref()
                    && artifact.logical_codebase_id != manifest.logical_codebase_id.to_string()
                {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "repository_routing",
                        reason: format!(
                            "repository_routing_inconsistent: policy artifact {} does not belong to manifest {}",
                            artifact.logical_codebase_id, manifest.logical_codebase_id
                        ),
                    });
                }
                Ok(AuthorityPolicyReference {
                    policy_id: artifact.policy_id,
                    policy_revision: artifact.revision,
                    policy_digest: artifact.digest,
                    artifact_root: authority_root.clone(),
                })
            })
            .transpose()?;

        let aggregate_index = read_authority_aggregate_index(&self.paths, &request.project_id, lc_id)?;

        Ok(RepositoryAuthorityResolution {
            authority_root,
            target: ResolvedTargetIdentity {
                kind: RepositoryTargetKind::LogicalCodebase,
                repository_id: None,
                logical_codebase_id: Some(lc_id.to_string()),
                logical_repository_id: member_id,
                checkout_id,
                canonical_path,
                source_identity_digest,
            },
            manifest,
            selection,
            policy,
            aggregate_index,
        })
    }

    fn resolve_single_repo(
        &self,
        request: &RepositoryRoutingRequest,
        attributed_lc: Option<&str>,
    ) -> Result<RepositoryAuthorityResolution, ProductStoreError> {
        let Some(repository_id) = request.repository_id.as_deref() else {
            return Err(ProductStoreError::Conflict {
                kind: "repository_routing_kind_mismatch",
                id: format!(
                    "repository_id:missing:issue:{:?}",
                    request.issue_id
                ),
            });
        };
        if request.logical_codebase_id.is_some()
            || request.logical_repository_id.is_some()
            || request.checkout_id.is_some()
        {
            return Err(ProductStoreError::Conflict {
                kind: "repository_routing_kind_mismatch",
                id: format!("repository_id:{repository_id}:logical_target_present"),
            });
        }
        if let Some(attributed) = attributed_lc {
            return Err(ProductStoreError::Conflict {
                kind: "repository_routing_kind_mismatch",
                id: format!(
                    "issue:{}:attributed_logical_codebase:{attributed}:repository:{repository_id}",
                    request.issue_id.clone().unwrap_or_default()
                ),
            });
        }

        let repository_store = crate::product::repository_store::RepositoryStore::new(
            self.paths.clone(),
        );
        let records = repository_store.list(&request.project_id)?;
        let record = records
            .iter()
            .find(|record| record.id == repository_id)
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "repository",
                id: repository_id.to_string(),
            })?;
        let canonical_path =
            crate::product::repository_store::canonicalize_repo_path(&record.path)?;

        // 同一 git-dir 两个别名：其余物理记录解析到同一 canonical path 即冲突。
        for other in &records {
            if other.id == repository_id {
                continue;
            }
            if let Ok(other_canonical) =
                crate::product::repository_store::canonicalize_repo_path(&other.path)
                && other_canonical == canonical_path
            {
                return Err(ProductStoreError::Conflict {
                    kind: "repository_routing_source_identity_mismatch",
                    id: format!("{repository_id}:{}", other.id),
                });
            }
        }

        let source = crate::product::repository_store::resolve_repository_source(&canonical_path)?;

        // 旧 project-level 布局与物理仓来源身份冲突。
        let legacy_root = self.paths.logical_codebase_root(&request.project_id);
        if legacy_root.join("manifest.json").exists() {
            let legacy_members =
                LogicalCodebaseStore::new(self.paths.clone()).list_members(&request.project_id)?;
            if let Some(conflict) = legacy_members.iter().find(|member| {
                member.status
                    == crate::product::logical_codebase::types::MemberStatus::Active
                    && member.source_identity.key_digest == source.key_digest
            }) {
                return Err(ProductStoreError::Conflict {
                    kind: "repository_routing_legacy_conflict",
                    id: conflict.source_identity.key_digest.clone(),
                });
            }
        }

        Ok(RepositoryAuthorityResolution {
            authority_root: canonical_path.clone(),
            target: ResolvedTargetIdentity {
                kind: RepositoryTargetKind::SingleRepo,
                repository_id: Some(repository_id.to_string()),
                logical_codebase_id: None,
                logical_repository_id: None,
                checkout_id: None,
                canonical_path,
                source_identity_digest: source.key_digest,
            },
            manifest: None,
            selection: None,
            policy: None,
            aggregate_index: AuthorityAggregateIndexReference {
                aggregate_index_id: None,
                membership_revision: None,
                status: None,
            },
        })
    }
}

/// 解析 LC kind 的 canonical path/source digest/member/checkout 身份。
/// 显式 member/checkout 必须在请求 LC 子树内命中，否则 target unknown。
fn resolve_logical_target(
    request: &RepositoryRoutingRequest,
    manifest: &Option<LogicalCodebaseManifest>,
    members: &[crate::product::logical_codebase::types::CodebaseMemberRecord],
    checkouts: &[crate::product::logical_codebase::types::RepositoryCheckoutRecord],
    record_root: &PathBuf,
) -> Result<(PathBuf, String, Option<LogicalRepositoryId>, Option<RepositoryCheckoutId>), ProductStoreError>
{
    match request.logical_repository_id {
        Some(member_id) => {
            let member = members
                .iter()
                .find(|member| member.logical_repository_id == member_id)
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "logical_repository",
                    id: member_id.0.to_string(),
                })?;
            let checkout = match request.checkout_id {
                Some(checkout_id) => {
                    if !member.checkout_ids.contains(&checkout_id) {
                        return Err(ProductStoreError::NotFound {
                            kind: "repository_checkout",
                            id: checkout_id.0.to_string(),
                        });
                    }
                    checkouts
                        .iter()
                        .find(|checkout| checkout.checkout_id == checkout_id)
                        .ok_or_else(|| ProductStoreError::NotFound {
                            kind: "repository_checkout",
                            id: checkout_id.0.to_string(),
                        })?
                }
                None => checkouts
                    .iter()
                    .find(|checkout| {
                        member.checkout_ids.contains(&checkout.checkout_id)
                            && checkout.kind
                                == crate::product::logical_codebase::types::CheckoutKind::Main
                    })
                    .ok_or_else(|| ProductStoreError::NotFound {
                        kind: "repository_checkout",
                        id: member.checkout_ids.first().map(|id| id.0.to_string()).unwrap_or_default(),
                    })?,
            };
            Ok((
                checkout.canonical_path.clone(),
                member.source_identity.key_digest.clone(),
                Some(member_id),
                Some(checkout.checkout_id),
            ))
        }
        None => {
            // 冷启动（无 manifest）LC：聚合根回退到 record.aggregate_root，
            // 使 bootstrap/成员等 GET 投影在 manifest 尚未生成时仍可解析身份。
            let root = manifest
                .as_ref()
                .map(|manifest| manifest.provider_context_root.clone())
                .unwrap_or_else(|| record_root.clone());
            // 聚合 source digest：root + 排序后的成员 source digests。
            let mut digests: Vec<&str> = members
                .iter()
                .filter(|member| {
                    member.status
                        == crate::product::logical_codebase::types::MemberStatus::Active
                })
                .map(|member| member.source_identity.key_digest.as_str())
                .collect();
            digests.sort_unstable();
            let mut payload = root.to_string_lossy().into_owned();
            for digest in digests {
                payload.push('\0');
                payload.push_str(digest);
            }
            let source_identity_digest =
                format!("sha256:{:x}", sha2::Sha256::digest(payload.as_bytes()));
            Ok((root, source_identity_digest, None, None))
        }
    }
}

/// 只读读取 LC 子树的 aggregate index 引用：优先 active；无 active 时
/// 保留最新一代非 superseded 记录的状态事实（如 Failed）；全空返回 None 组合。
fn read_authority_aggregate_index(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: &str,
) -> Result<AuthorityAggregateIndexReference, ProductStoreError> {
    let store = AggregateIndexStore::for_lc(paths.clone(), lc_id);
    let map_error = |error: AggregateIndexError| -> ProductStoreError {
        ProductStoreError::InvalidRecord {
            kind: "aggregate_index",
            reason: error.to_string(),
        }
    };
    if let Some(active) = store.active(project_id).map_err(map_error)? {
        return Ok(AuthorityAggregateIndexReference {
            aggregate_index_id: Some(active.aggregate_index_id),
            membership_revision: Some(active.membership_revision),
            status: Some(AggregateIndexStatus::Active),
        });
    }
    let mut latest: Option<crate::product::logical_codebase::aggregate_index::AggregateIndexRecord> =
        None;
    for record in store.records(project_id).map_err(map_error)? {
        if record.status == AggregateIndexStatus::Superseded {
            continue;
        }
        if latest
            .as_ref()
            .is_none_or(|current| record.updated_at > current.updated_at)
        {
            latest = Some(record);
        }
    }
    Ok(match latest {
        None => AuthorityAggregateIndexReference {
            aggregate_index_id: None,
            membership_revision: None,
            status: None,
        },
        Some(record) => AuthorityAggregateIndexReference {
            aggregate_index_id: Some(record.aggregate_index_id),
            membership_revision: Some(record.membership_revision),
            status: Some(record.status),
        },
    })
}

/// C2 Task 10（REQ-ENV-C2-POLICY）：受限政策读取错误。resolver 无法唯一
/// 解析／身份或 digest 不一致一律 fail-closed——调用方（evidence mediator
/// 应用服务）落"政策核验"等待事实，MUST NOT 回落到成员仓路径、项目级
/// 历史布局或绝对路径猜测。
#[derive(Debug, thiserror::Error)]
pub enum PolicyReadError {
    /// policy_id 结构不可解析，或引用指向的 LC 子树无 policy artifact
    /// （resolver 不可用口径）。
    #[error("policy_reference_unavailable:{detail}")]
    Unavailable { detail: String },
    /// policy_id／revision／authority root 与 artifact 身份不一致（引用被
    /// 串改或指向错误子树）。
    #[error("policy_identity_mismatch:{detail}")]
    IdentityMismatch { detail: String },
    /// 正文 canonical SHA-256 与引用 digest 不一致（政策已升级，冻结引用
    /// 过期）。
    #[error("policy_digest_mismatch:expected:{expected}:actual:{actual}")]
    DigestMismatch { expected: String, actual: String },
    /// 底层 durable store 读失败。
    #[error("policy_read_store_error:{0}")]
    Store(#[from] ProductStoreError),
}

/// C2 Task 10：受限政策读取结果——与引用同 digest 的政策正文＋三元引用；
/// 不携带宿主绝对路径。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PolicyTextResult {
    pub policy_id: String,
    pub policy_revision: u64,
    pub policy_digest: String,
    pub text: String,
}

/// C2 Task 10：以 `resolve_for_issue` 产出的 `AuthorityPolicyReference` 为
/// 输入读取同 digest 政策正文。
///
/// `policy_id` 形如 `policy/{project_id}/{manifest 身份}/{revision}`
/// （`AggregatePolicyArtifact` 构造契约；内嵌的是 manifest 身份而非子树
/// 目录键），故解析出 project_id 后在项目全部权威 LC 子树（legacy root＋
/// `logical-codebases/*`）中按 policy_id 恰匹配检索——恰一个匹配才继续，
/// 零个 Unavailable、多个 IdentityMismatch，不做任何路径猜测；随后
/// revision／authority root／digest 逐项复核，任一不一致 fail-closed。
/// 底层 `get` 已复核 digest 是正文 canonical SHA-256。
pub fn read_policy_text_for_reference(
    paths: &ProductAppPaths,
    reference: &AuthorityPolicyReference,
) -> Result<PolicyTextResult, PolicyReadError> {
    let segments: Vec<&str> = reference.policy_id.split('/').collect();
    if segments.len() != 4 || segments[0] != "policy" || segments.iter().any(|s| s.is_empty()) {
        return Err(PolicyReadError::Unavailable {
            detail: format!("malformed policy_id: {}", reference.policy_id),
        });
    }
    let project_id = segments[1];
    validate_relative_id(project_id)?;

    // 候选权威子树：legacy root（`None` scope）＋该项目全部
    // `logical-codebases/{id}` 子树；与 legacy 同名的目录即 legacy root 本身，
    // 跳过避免重复计数。
    let legacy_id = crate::product::logical_codebase::store::legacy_logical_codebase_id(project_id);
    let mut scopes: Vec<Option<String>> = vec![None];
    if let Ok(entries) = std::fs::read_dir(paths.logical_codebases_root(project_id)) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str()
                && name != legacy_id
            {
                scopes.push(Some(name.to_string()));
            }
        }
    }

    let mut matched: Option<(Option<String>, AggregatePolicyArtifact)> = None;
    for scope in scopes {
        let policy_store = match scope.as_deref() {
            Some(lc_id) => AggregatePolicyArtifactStore::for_lc(paths.clone(), lc_id),
            None => AggregatePolicyArtifactStore::new(paths.clone()),
        };
        let Some(artifact) = policy_store.get(project_id)? else {
            continue;
        };
        if artifact.policy_id != reference.policy_id {
            continue;
        }
        if matched.is_some() {
            return Err(PolicyReadError::IdentityMismatch {
                detail: format!(
                    "policy_id {} matches multiple authority subtrees",
                    reference.policy_id
                ),
            });
        }
        matched = Some((scope, artifact));
    }
    let Some((scope, artifact)) = matched else {
        return Err(PolicyReadError::Unavailable {
            detail: format!(
                "no authority subtree holds policy {}",
                reference.policy_id
            ),
        });
    };

    if artifact.revision != reference.policy_revision {
        return Err(PolicyReadError::IdentityMismatch {
            detail: format!(
                "reference revision {} does not match artifact revision {}",
                reference.policy_revision, artifact.revision
            ),
        });
    }
    // authority root 复核：artifact 必须来自引用冻结时的同一权威根（manifest
    // 优先，冷启动回退 LC record.aggregate_root，与 `resolve_logical` 同口径）。
    let logical = match scope.as_deref() {
        Some(lc_id) => LogicalCodebaseStore::for_lc(paths.clone(), lc_id),
        None => LogicalCodebaseStore::new(paths.clone()),
    };
    let manifest = logical.load_manifest(project_id)?;
    let record_root = paths.logical_codebase_record_root(
        project_id,
        scope
            .as_deref()
            .unwrap_or(&legacy_id),
    );
    let record: crate::product::logical_codebase::store::LogicalCodebaseRecord =
        crate::product::json_store::read_json(&record_root.join("record.json"))?;
    let expected_root = manifest
        .as_ref()
        .map(|manifest| manifest.provider_context_root.clone())
        .unwrap_or_else(|| record.aggregate_root.clone());
    let expected_root = std::fs::canonicalize(&expected_root).unwrap_or(expected_root);
    if expected_root != reference.artifact_root {
        return Err(PolicyReadError::IdentityMismatch {
            detail: format!(
                "authority root {} does not match reference {}",
                expected_root.display(),
                reference.artifact_root.display()
            ),
        });
    }

    // digest 一致性：引用冻结 digest 必须等于 artifact 正文 digest（get 已
    // 校验 digest 是正文 canonical SHA-256）。
    if artifact.digest != reference.policy_digest {
        return Err(PolicyReadError::DigestMismatch {
            expected: reference.policy_digest.clone(),
            actual: artifact.digest.clone(),
        });
    }
    Ok(PolicyTextResult {
        policy_id: artifact.policy_id,
        policy_revision: artifact.revision,
        policy_digest: artifact.digest,
        text: artifact.policy_text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::logical_codebase::issue_selection::IssueCodebaseSelection;
    use crate::product::logical_codebase::store::LogicalCodebaseManifest;

    fn manifest_fixture() -> LogicalCodebaseManifest {
        LogicalCodebaseManifest::new(
            "project_0001",
            std::path::PathBuf::from("/tmp/logical-codebase"),
            Vec::new(),
        )
    }

    #[test]
    fn none_none_routes_to_legacy() {
        // (None, None) → Legacy；不读任何文件，纯判定
        let routing = RepositoryRouting::classify(None, None);
        assert!(matches!(routing, RepositoryRouting::Legacy { .. }));
    }

    #[test]
    fn some_some_routes_to_logical() {
        let selection = IssueCodebaseSelection::all_members("project_0001", "issue_0001", None);
        let routing = RepositoryRouting::classify(Some(manifest_fixture()), Some(selection));
        assert!(matches!(routing, RepositoryRouting::Logical { .. }));
    }

    #[test]
    fn some_none_is_fail_closed_with_stable_code() {
        // 有 manifest 无 selection → 不完整逻辑状态，fail-closed，稳定错误码 TargetMissing（B3）
        let routing = RepositoryRouting::classify(Some(manifest_fixture()), None);
        match routing {
            RepositoryRouting::FailClosed { code, .. } => {
                assert_eq!(code, RepositoryRoutingErrorCode::TargetMissing)
            }
            _ => panic!("(Some, None) must fail-closed"),
        }
    }

    #[test]
    fn none_some_is_fail_closed_with_stable_code() {
        // 无 manifest 有 selection → 孤立 selection/数据损坏，fail-closed，稳定错误码 OrphanedSelection
        let selection = IssueCodebaseSelection::all_members("project_0001", "issue_0001", None);
        let routing = RepositoryRouting::classify(None, Some(selection));
        match routing {
            RepositoryRouting::FailClosed { code, .. } => {
                assert_eq!(code, RepositoryRoutingErrorCode::OrphanedSelection)
            }
            _ => panic!("(None, Some) must fail-closed"),
        }
    }

    #[test]
    fn resolve_issue_logical_codebase_id_reads_persisted_attribution() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::product::app_paths::ProductAppPaths::new(temp.path());
        let store = crate::product::issue_store::IssueStore::new(paths.clone());
        let issue = store
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some("repository_0001".to_string()),
                logical_codebase_id: Some("logical_codebase_0001".to_string()),
                title: "logical issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();
        let lc_id = resolve_issue_logical_codebase_id(&paths, "project_0001", &issue.id).unwrap();
        assert_eq!(lc_id.as_deref(), Some("logical_codebase_0001"));
    }

    #[test]
    fn load_for_issue_resolves_manifest_and_selection_from_lc_subtree() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::product::app_paths::ProductAppPaths::new(temp.path());
        let lc_id = "logical_codebase_0001";
        // 建 issue 归属 lc_id。
        let store = crate::product::issue_store::IssueStore::new(paths.clone());
        let issue = store
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some("repository_0001".to_string()),
                logical_codebase_id: Some(lc_id.to_string()),
                title: "logical issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();
        // 在 lc 子树写 manifest + selection。
        let logical = LogicalCodebaseStore::for_lc(paths.clone(), lc_id);
        logical
            .save_manifest(
                "project_0001",
                &LogicalCodebaseManifest::new(
                    "project_0001",
                    std::path::PathBuf::from("/tmp/logical-codebase"),
                    Vec::new(),
                ),
            )
            .unwrap();
        IssueCodebaseSelectionStore::for_lc(paths.clone(), lc_id)
            .save(
                &IssueCodebaseSelection::all_members("project_0001", &issue.id, None)
                    .for_logical_codebase(lc_id),
            )
            .unwrap();

        let routing = RepositoryRouting::load_for_issue(&paths, "project_0001", &issue.id).unwrap();
        match routing {
            RepositoryRouting::Logical {
                manifest,
                selection,
            } => {
                assert_eq!(manifest.project_id, "project_0001");
                assert_eq!(selection.logical_codebase_id.as_deref(), Some(lc_id));
            }
            _ => panic!("must resolve Logical from lc subtree"),
        }
    }

    // ---- Task 1：唯一 authority resolver（显式 kind + fail-closed 冲突）----

    use std::collections::BTreeMap;

    use crate::product::logical_codebase::aggregate_index::{
        AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
    };
    use crate::product::logical_codebase::policy::{
        AggregatePolicyArtifact, AggregatePolicyArtifactStore,
    };
    use crate::product::logical_codebase::types::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, MemberStatus,
        RepositoryCheckoutRecord, RepositoryType,
    };
    use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
    use crate::product::project_store::{CreateProjectInput, ProjectStore};

    fn git(cwd: &std::path::Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(cwd)
            .args(arguments)
            .output()
            .expect("git command must start");
        assert!(
            output.status.success(),
            "git {arguments:?} failed in {}: {}",
            cwd.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_git_repository_with_commit(path: &std::path::Path) {
        std::fs::create_dir_all(path).unwrap();
        git(path, &["init", "-b", "main"]);
        git(path, &["config", "user.email", "resolver@test.local"]);
        git(path, &["config", "user.name", "Resolver Test"]);
        std::fs::write(path.join("README.md"), "# member\n").unwrap();
        git(path, &["add", "."]);
        git(path, &["commit", "-m", "init"]);
    }

    /// 收集 `.aria` durable inventory（相对路径 → 文件字节），用于断言 resolver 零写入。
    fn aria_inventory(root: &std::path::Path) -> BTreeMap<String, Vec<u8>> {
        let mut inventory = BTreeMap::new();
        let mut stack = vec![root.join(".aria")];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let relative = path.strip_prefix(root).unwrap().to_path_buf();
                    inventory.insert(
                        relative.to_string_lossy().into_owned(),
                        std::fs::read(&path).unwrap_or_default(),
                    );
                }
            }
        }
        inventory
    }

    fn create_project_fixture(paths: &crate::product::app_paths::ProductAppPaths) -> String {
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "resolver-project".to_string(),
                description: None,
            })
            .unwrap()
            .id
    }

    fn active_index_record(
        aggregate_index_id: &str,
        project_id: &str,
        membership_revision: u64,
    ) -> AggregateIndexRecord {
        let mut record = AggregateIndexRecord::building(
            aggregate_index_id.to_string(),
            project_id.to_string(),
            membership_revision,
            Vec::new(),
            "2026-09-28T00:00:00Z".to_string(),
        );
        record.status = AggregateIndexStatus::Active;
        record
    }

    #[test]
    fn resolver_reads_lc_manifest_selection_policy_and_index_only_from_requested_lc_subtree() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::product::app_paths::ProductAppPaths::new(temp.path());
        let project_id = create_project_fixture(&paths);

        std::fs::create_dir_all(temp.path().join("alpha-root")).unwrap();
        std::fs::create_dir_all(temp.path().join("beta-root")).unwrap();
        let logical = LogicalCodebaseStore::new(paths.clone());
        let lc_a = logical
            .create(
                &project_id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "alpha".to_string(),
                    aggregate_root: temp.path().join("alpha-root"),
                },
            )
            .unwrap();
        let lc_b = logical
            .create(
                &project_id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "beta".to_string(),
                    aggregate_root: temp.path().join("beta-root"),
                },
            )
            .unwrap();

        let manifest_a = LogicalCodebaseManifest::new(
            &project_id,
            temp.path().join("alpha-root"),
            Vec::new(),
        );
        let manifest_b = LogicalCodebaseManifest::new(
            &project_id,
            temp.path().join("beta-root"),
            Vec::new(),
        );
        LogicalCodebaseStore::for_lc(paths.clone(), &lc_a.id)
            .save_manifest(&project_id, &manifest_a)
            .unwrap();
        LogicalCodebaseStore::for_lc(paths.clone(), &lc_b.id)
            .save_manifest(&project_id, &manifest_b)
            .unwrap();

        let issue_store = crate::product::issue_store::IssueStore::new(paths.clone());
        let issue_a = issue_store
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project_id.clone(),
                repo_id: None,
                logical_codebase_id: Some(lc_a.id.clone()),
                title: "alpha issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();
        IssueCodebaseSelectionStore::for_lc(paths.clone(), &lc_a.id)
            .save(
                &IssueCodebaseSelection::all_members(&project_id, &issue_a.id, None)
                    .for_logical_codebase(&lc_a.id),
            )
            .unwrap();

        let policy_a = AggregatePolicyArtifact::bootstrap(
            &project_id,
            &manifest_a.logical_codebase_id.to_string(),
            "2026-09-28T00:00:00Z".to_string(),
        );
        let policy_b = AggregatePolicyArtifact::bootstrap(
            &project_id,
            &manifest_b.logical_codebase_id.to_string(),
            "2026-09-28T00:00:00Z".to_string(),
        );
        AggregatePolicyArtifactStore::for_lc(paths.clone(), &lc_a.id)
            .save(&project_id, &policy_a)
            .unwrap();
        AggregatePolicyArtifactStore::for_lc(paths.clone(), &lc_b.id)
            .save(&project_id, &policy_b)
            .unwrap();

        AggregateIndexStore::for_lc(paths.clone(), &lc_a.id)
            .create(
                &project_id,
                active_index_record("aggregate_index_alpha", &project_id, manifest_a.membership_revision),
            )
            .unwrap();
        AggregateIndexStore::for_lc(paths.clone(), &lc_b.id)
            .create(
                &project_id,
                active_index_record("aggregate_index_beta", &project_id, manifest_b.membership_revision),
            )
            .unwrap();

        let resolver = RepositoryAuthorityResolver::new(paths.clone());
        let resolution = resolver
            .resolve(RepositoryRoutingRequest {
                project_id: project_id.clone(),
                issue_id: Some(issue_a.id.clone()),
                kind: RepositoryTargetKind::LogicalCodebase,
                repository_id: None,
                logical_codebase_id: Some(lc_a.id.clone()),
                logical_repository_id: None,
                checkout_id: None,
            })
            .unwrap();

        assert_eq!(
            resolution.authority_root,
            std::fs::canonicalize(temp.path().join("alpha-root")).unwrap()
        );
        assert_eq!(resolution.target.kind, RepositoryTargetKind::LogicalCodebase);
        assert_eq!(
            resolution.target.logical_codebase_id.as_deref(),
            Some(lc_a.id.as_str())
        );
        let manifest = resolution.manifest.expect("manifest from requested lc subtree");
        assert_eq!(manifest.provider_context_root, temp.path().join("alpha-root"));
        assert_eq!(manifest.logical_codebase_id, manifest_a.logical_codebase_id);
        let selection = resolution.selection.expect("selection from requested lc subtree");
        assert_eq!(selection.logical_codebase_id.as_deref(), Some(lc_a.id.as_str()));
        let policy = resolution.policy.expect("policy from requested lc subtree");
        assert_eq!(policy.policy_id, policy_a.policy_id);
        assert_eq!(policy.policy_digest, policy_a.digest);
        assert_eq!(policy.policy_revision, policy_a.revision);
        assert_eq!(
            resolution.aggregate_index.aggregate_index_id.as_deref(),
            Some("aggregate_index_alpha")
        );
        assert_eq!(
            resolution.aggregate_index.membership_revision,
            Some(manifest_a.membership_revision)
        );
        assert_eq!(
            resolution.aggregate_index.status,
            Some(AggregateIndexStatus::Active)
        );
    }

    // ---- C2 Task 10：受限政策读取（REQ-ENV-C2-POLICY，#17／BYPASS-17）----

    /// 最小 LC 政策 fixture：project + alpha/beta 两个 LC record + manifest +
    /// bootstrap policy artifact + issue 归属 alpha（成员/checkout 不参与
    /// policy 解析，policy 走 `resolve_logical` 的 None-member 分支）。
    #[allow(clippy::type_complexity)]
    fn policy_reader_fixture(
        paths: &crate::product::app_paths::ProductAppPaths,
        temp: &std::path::Path,
    ) -> (
        String,
        String,
        String,
        AggregatePolicyArtifact,
        std::path::PathBuf,
    ) {
        let project_id = create_project_fixture(paths);
        let alpha_root = temp.join("alpha-policy-root");
        let beta_root = temp.join("beta-policy-root");
        std::fs::create_dir_all(&alpha_root).unwrap();
        std::fs::create_dir_all(&beta_root).unwrap();
        let logical = LogicalCodebaseStore::new(paths.clone());
        let lc = logical
            .create(
                &project_id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "policy-alpha".to_string(),
                    aggregate_root: alpha_root.clone(),
                },
            )
            .unwrap();
        logical
            .create(
                &project_id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "policy-beta".to_string(),
                    aggregate_root: beta_root.clone(),
                },
            )
            .unwrap();
        let manifest = LogicalCodebaseManifest::new(&project_id, alpha_root, Vec::new());
        LogicalCodebaseStore::for_lc(paths.clone(), &lc.id)
            .save_manifest(&project_id, &manifest)
            .unwrap();
        let policy = AggregatePolicyArtifact::bootstrap(
            &project_id,
            &manifest.logical_codebase_id.to_string(),
            "2026-09-29T00:00:00Z".to_string(),
        );
        AggregatePolicyArtifactStore::for_lc(paths.clone(), &lc.id)
            .save(&project_id, &policy)
            .unwrap();
        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project_id.clone(),
                repo_id: None,
                logical_codebase_id: Some(lc.id.clone()),
                title: "policy read issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();
        (
            project_id,
            issue.id,
            lc.id,
            policy,
            beta_root,
        )
    }

    #[test]
    fn policy_reader_returns_same_digest_text_for_frozen_reference() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::product::app_paths::ProductAppPaths::new(temp.path());
        let (project_id, issue_id, lc_id, policy, beta_root) =
            policy_reader_fixture(&paths, temp.path());

        let frozen = RepositoryAuthorityResolver::new(paths.clone())
            .resolve_for_issue(&project_id, &issue_id)
            .unwrap()
            .expect("lc resolution")
            .policy
            .expect("policy reference");

        let result = read_policy_text_for_reference(&paths, &frozen).expect("same digest text");
        assert_eq!(result.policy_id, policy.policy_id);
        assert_eq!(result.policy_revision, policy.revision);
        assert_eq!(result.policy_digest, policy.digest);
        assert_eq!(result.text, policy.policy_text);
        // 正文 digest 必须是返回正文的 canonical SHA-256（非自报）。
        let recomputed = format!(
            "sha256:{:x}",
            sha2::Sha256::digest(result.text.as_bytes())
        );
        assert_eq!(recomputed, result.policy_digest);

        // authority root 与引用不符（引用被串改到 beta 根）→ IdentityMismatch
        // fail-closed，不按串改 root 猜测。
        let tampered = AuthorityPolicyReference {
            artifact_root: beta_root,
            ..frozen.clone()
        };
        let mismatched = read_policy_text_for_reference(&paths, &tampered).unwrap_err();
        assert!(matches!(mismatched, PolicyReadError::IdentityMismatch { .. }));

        // 引用 digest 与 artifact 正文 digest 不一致（引用被串改 digest）→
        // DigestMismatch fail-closed，不得返回正文。
        let digest_tampered = AuthorityPolicyReference {
            policy_digest: "sha256:tampered-digest".to_string(),
            ..frozen.clone()
        };
        let digest_rejected = read_policy_text_for_reference(&paths, &digest_tampered).unwrap_err();
        assert!(matches!(
            digest_rejected,
            PolicyReadError::DigestMismatch { .. }
        ));

        // 政策升级（revision 2 覆盖保存）后，旧冻结引用（revision 1）不再有
        // 权威子树持有 → Unavailable fail-closed，不得静默返回新正文。
        let revised =
            policy.with_revised_policy("升级后的政策正文", "2026-09-29T01:00:00Z".to_string());
        AggregatePolicyArtifactStore::for_lc(paths.clone(), &lc_id)
            .save(&project_id, &revised)
            .unwrap();
        let stale = read_policy_text_for_reference(&paths, &frozen).unwrap_err();
        assert!(matches!(stale, PolicyReadError::Unavailable { .. }));
    }

    #[test]
    fn policy_reader_fail_closes_on_missing_artifact_or_malformed_reference() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::product::app_paths::ProductAppPaths::new(temp.path());
        let (project_id, _issue_id, _lc_id, _policy, _beta_root) =
            policy_reader_fixture(&paths, temp.path());

        // 引用指向不存在 artifact 的 LC → Unavailable，无路径猜测。
        let foreign = AuthorityPolicyReference {
            policy_id: format!("policy/{project_id}/logical_codebase_missing/1"),
            policy_revision: 1,
            policy_digest: "sha256:deadbeef".to_string(),
            artifact_root: temp.path().join("alpha-policy-root"),
        };
        assert!(matches!(
            read_policy_text_for_reference(&paths, &foreign),
            Err(PolicyReadError::Unavailable { .. })
        ));

        // policy_id 结构不可解析 → Unavailable。
        let malformed = AuthorityPolicyReference {
            policy_id: "not-a-policy-id".to_string(),
            ..foreign
        };
        assert!(matches!(
            read_policy_text_for_reference(&paths, &malformed),
            Err(PolicyReadError::Unavailable { .. })
        ));
    }

    #[test]
    fn resolver_rejects_kind_mismatch_duplicate_source_and_legacy_conflict_without_writes() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::product::app_paths::ProductAppPaths::new(temp.path());
        let project_id = create_project_fixture(&paths);

        let workspace = temp.path().join("workspace");
        let repo_a = workspace.join("repo-a");
        init_git_repository_with_commit(&repo_a);
        let canonical = std::fs::canonicalize(&repo_a).unwrap();
        let source = crate::product::repository_store::resolve_repository_source(&canonical)
            .unwrap();

        let logical = LogicalCodebaseStore::new(paths.clone());
        let lc = logical
            .create(
                &project_id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "gamma".to_string(),
                    aggregate_root: workspace.clone(),
                },
            )
            .unwrap();
        let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc.id);

        let member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        let duplicate_member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let mut manifest = LogicalCodebaseManifest::new(&project_id, workspace.clone(), Vec::new());
        manifest.member_ids = vec![member_id];
        lc_store.save_manifest(&project_id, &manifest).unwrap();

        let now = "2026-09-28T00:00:00Z".to_string();
        let member = CodebaseMemberRecord {
            logical_repository_id: member_id,
            physical_repository_id: "repository_gamma_member".to_string(),
            alias: "repo-a".to_string(),
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
        };
        lc_store.save_member(&project_id, &member).unwrap();
        lc_store
            .save_checkout(
                &project_id,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_id,
                    physical_repository_id: member.physical_repository_id.clone(),
                    kind: CheckoutKind::Main,
                    canonical_path: canonical.clone(),
                    checkout_path_hash: crate::product::id::repo_hash_for_path(
                        canonical.to_string_lossy().as_ref(),
                    ),
                    git_dir_identity: source.git_dir_identity(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .unwrap();

        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project_id.clone(),
                repo_id: None,
                logical_codebase_id: Some(lc.id.clone()),
                title: "gamma issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();

        let resolver = RepositoryAuthorityResolver::new(paths.clone());

        // (a) single-repo 身份访问 LC 绑定 issue → kind mismatch，零写入。
        let before = aria_inventory(temp.path());
        let error = resolver
            .resolve(RepositoryRoutingRequest {
                project_id: project_id.clone(),
                issue_id: Some(issue.id.clone()),
                kind: RepositoryTargetKind::SingleRepo,
                repository_id: Some("repository_single".to_string()),
                logical_codebase_id: None,
                logical_repository_id: None,
                checkout_id: None,
            })
            .unwrap_err();
        assert!(
            matches!(
                &error,
                crate::product::json_store::ProductStoreError::Conflict { kind, id }
                    if *kind == "repository_routing_kind_mismatch" && id.contains(&issue.id)
            ),
            "single-repo request for a logical issue must fail closed with kind mismatch, got: {error:?}"
        );
        assert_eq!(aria_inventory(temp.path()), before);

        // (b) 同一 git-dir 两个别名成员 → source identity mismatch，零写入。
        let duplicate_member = CodebaseMemberRecord {
            logical_repository_id: duplicate_member_id,
            physical_repository_id: "repository_gamma_duplicate".to_string(),
            alias: "repo-a-alias".to_string(),
            role: "member".to_string(),
            ordinal: 2,
            source_identity: source.clone(),
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![RepositoryCheckoutId(uuid::Uuid::new_v4())],
            status: MemberStatus::Active,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        lc_store.save_member(&project_id, &duplicate_member).unwrap();
        let before = aria_inventory(temp.path());
        let error = resolver
            .resolve(RepositoryRoutingRequest {
                project_id: project_id.clone(),
                issue_id: Some(issue.id.clone()),
                kind: RepositoryTargetKind::LogicalCodebase,
                repository_id: None,
                logical_codebase_id: Some(lc.id.clone()),
                logical_repository_id: None,
                checkout_id: None,
            })
            .unwrap_err();
        assert!(
            matches!(
                &error,
                crate::product::json_store::ProductStoreError::Conflict { kind, id }
                    if *kind == "repository_routing_source_identity_mismatch"
                        && id.contains(&source.key_digest)
            ),
            "duplicate member source identity must fail closed, got: {error:?}"
        );
        assert_eq!(aria_inventory(temp.path()), before);
        lc_store
            .save_member(&project_id, &{
                // 移除重复成员，恢复唯一成员现场供 (c) 使用。
                let mut restored = duplicate_member;
                restored.status = MemberStatus::Removed;
                restored
            })
            .unwrap();

        // (c) 旧 project-level 布局与新 LC 子树来源身份冲突 → legacy conflict，零写入。
        let legacy_root = paths.logical_codebase_root(&project_id);
        let legacy_store = LogicalCodebaseStore::for_lc(
            paths.clone(),
            crate::product::logical_codebase::store::legacy_logical_codebase_id(&project_id),
        );
        let mut legacy_manifest =
            LogicalCodebaseManifest::new(&project_id, workspace.clone(), Vec::new());
        let legacy_member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        legacy_manifest.member_ids = vec![legacy_member_id];
        legacy_store.save_manifest(&project_id, &legacy_manifest).unwrap();
        legacy_store
            .save_member(
                &project_id,
                &CodebaseMemberRecord {
                    logical_repository_id: legacy_member_id,
                    physical_repository_id: "repository_legacy_member".to_string(),
                    alias: "repo-a-legacy".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: source.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![RepositoryCheckoutId(uuid::Uuid::new_v4())],
                    status: MemberStatus::Active,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .unwrap();
        let _ = legacy_root;

        let before = aria_inventory(temp.path());
        let error = resolver
            .resolve(RepositoryRoutingRequest {
                project_id: project_id.clone(),
                issue_id: Some(issue.id.clone()),
                kind: RepositoryTargetKind::LogicalCodebase,
                repository_id: None,
                logical_codebase_id: Some(lc.id.clone()),
                logical_repository_id: Some(member_id),
                checkout_id: None,
            })
            .unwrap_err();
        assert!(
            matches!(
                &error,
                crate::product::json_store::ProductStoreError::Conflict { kind, id }
                    if *kind == "repository_routing_legacy_conflict"
                        && id.contains(&source.key_digest)
            ),
            "legacy project-level layout conflicting with the requested lc must fail closed, got: {error:?}"
        );
        assert_eq!(aria_inventory(temp.path()), before);

        // 成员仓零 Git 写副作用：HEAD/dirty 不变。
        let dirty = std::process::Command::new("git")
            .current_dir(&repo_a)
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&dirty.stdout).trim().is_empty(),
            "member repository must stay clean"
        );
    }
}
