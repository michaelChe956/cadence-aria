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
/// 以 (manifest, selection) 成对状态为唯一权威信号，返回显式两态；
/// 单仓 `Legacy` 变体已随去单仓入口整体移除——无归属/不成对一律
/// `FailClosed`，绝不静默回退物理仓库。
pub enum RepositoryRouting {
    /// (Some, Some)：有 manifest 且有有效 selection → 逻辑解析（由调用方按 target/snapshot 定具体成员）。
    Logical {
        manifest: LogicalCodebaseManifest,
        selection: Box<IssueCodebaseSelection>,
    },
    /// 其余一切状态（含 (None, None) 的无归属旧数据）→ 明确错误，
    /// 稳定错误码 + 可诊断 reason，绝不静默回退物理仓库。
    FailClosed {
        code: RepositoryRoutingErrorCode,
        reason: String,
    },
}

/// 解析 issue 唯一归属的代码库（REQ-ROUTE-01 两态中枢）：返回 issue 记录里
/// 持久化的 `logical_codebase_id`（必填 String）；issue 不存在时返回
/// `NotFound` 错误（不再以 `Ok(None)` 表达「无归属」——归属缺失是数据
/// 错误，由读取层显式报 `issue_not_bound_to_logical_codebase`）。
pub fn resolve_issue_logical_codebase_id(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
) -> Result<String, ProductStoreError> {
    crate::product::issue_store::IssueStore::new(app_paths.clone())
        .get(project_id, issue_id)
        .map(|issue| issue.logical_codebase_id)
}

/// 据 lc_id 构造 manifest/selection 的权威 store 对（只读 `for_lc` 子树；
/// project 级 store 的 None 回退已随 Legacy 变体删除）。
fn issue_codebase_stores(
    app_paths: &ProductAppPaths,
    lc_id: &str,
) -> (LogicalCodebaseStore, IssueCodebaseSelectionStore) {
    (
        LogicalCodebaseStore::for_lc(app_paths.clone(), lc_id),
        IssueCodebaseSelectionStore::for_lc(app_paths.clone(), lc_id),
    )
}

impl RepositoryRouting {
    /// 无归属（issue 无 LC 归属且无 legacy 别名 record）的统一两态
    /// fail-closed 形态：`FailClosed(TargetMissing)`，reason 语义与
    /// `issue_not_bound_to_logical_codebase` 同源。
    pub fn unbound_issue() -> Self {
        RepositoryRouting::FailClosed {
            code: RepositoryRoutingErrorCode::TargetMissing,
            reason:
                "issue_not_bound_to_logical_codebase: work item has no logical codebase attribution"
                    .to_string(),
        }
    }

    /// 纯判定，不加载 store（便于单测）；加载在 `load_for_issue` 中完成。
    pub fn classify(
        manifest: Option<LogicalCodebaseManifest>,
        selection: Option<IssueCodebaseSelection>,
    ) -> Self {
        match (manifest, selection) {
            (None, None) => Self::unbound_issue(),
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

    /// 加载辅助（B6）：按 issue 所属 codebase 经 `for_lc` 子树加载
    /// manifest + selection 后交 `classify`（REQ-ROUTE-01：只读 lc_id
    /// 权威子树，无 project 级回退）。
    pub fn load_for_issue(
        app_paths: &ProductAppPaths,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Self, ProductStoreError> {
        let lc_id = resolve_issue_logical_codebase_id(app_paths, project_id, issue_id)?;
        let (logical, selections) = issue_codebase_stores(app_paths, &lc_id);
        let manifest = logical.load_manifest(project_id)?;
        let selection = selections.load(project_id, issue_id)?;
        Ok(Self::classify(manifest, selection))
    }
}

// ---- Task 1：唯一 authority resolver（显式 kind + fail-closed 冲突）----

use std::path::{Path, PathBuf};

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
