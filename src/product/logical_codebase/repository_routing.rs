use sha2::Digest as _;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::logical_codebase::issue_selection::{
    IssueCodebaseSelection, IssueCodebaseSelectionStore,
};
use crate::product::logical_codebase::store::{LogicalCodebaseManifest, LogicalCodebaseStore};

// 本文件按仓库惯例拆入 `.inc.rs`（同模块命名空间，符号路径与可见性不变），
// 保持每个文件低于 large_file_guard 的 1200 行上限：
// - repository_routing_types.inc.rs：稳定错误码、路由判定（classify/load_for_issue）
//   与目标/authority 引用契约类型。
// - repository_routing_resolver.inc.rs：RepositoryAuthorityResolver 只读解析
//   （仅从请求的 LC 子树读 manifest/selection/index）。
// - repository_routing_policy_reader.inc.rs：受限政策文本读取路径
//   （read_policy_text_for_reference，fail-closed）。
// - repository_routing_tests.inc.rs：tests 模块。
include!("repository_routing_types.inc.rs");
include!("repository_routing_resolver.inc.rs");
include!("repository_routing_policy_reader.inc.rs");
include!("repository_routing_tests.inc.rs");
