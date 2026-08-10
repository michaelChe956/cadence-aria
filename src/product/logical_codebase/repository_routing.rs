use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
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

/// Web 运行时统一 repository 分流判定（REQ-ROUTE-01）。
/// 以 (manifest, selection) 成对状态为唯一权威信号，返回显式三态。
pub enum RepositoryRouting {
    /// (None, None)：无 manifest 且无 selection → 物理 RepositoryRecord.id 解析（改动前行为）。
    Legacy { repository_id: String },
    /// (Some, Some)：有 manifest 且有有效 selection → 逻辑解析（由调用方按 target/snapshot 定具体成员）。
    Logical {
        manifest: LogicalCodebaseManifest,
        selection: IssueCodebaseSelection,
    },
    /// 其余一切不一致状态 → 明确错误，稳定错误码 + 可诊断 reason，绝不静默回退物理仓库。
    FailClosed {
        code: RepositoryRoutingErrorCode,
        reason: String,
    },
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
            (Some(manifest), Some(selection)) => RepositoryRouting::Logical { manifest, selection },
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

    /// 加载辅助（B6）：经 store 加载 manifest + selection 后交 `classify`。
    pub fn load_for_issue(
        app_paths: &ProductAppPaths,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Self, ProductStoreError> {
        let manifest = LogicalCodebaseStore::new(app_paths.clone()).load_manifest(project_id)?;
        let selection =
            IssueCodebaseSelectionStore::new(app_paths.clone()).load(project_id, issue_id)?;
        Ok(Self::classify(manifest, selection))
    }
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
}
