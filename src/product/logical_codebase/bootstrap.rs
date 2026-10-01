//! C4 Task 3：LC 冷启动五步 durable projection 与 action contract。
//!
//! `LogicalCodebaseBootstrapProjection` 不落独立的 `bootstrap.json` 状态机；
//! 它只组合既有 durable facts——LC record、identity migration journal、
//! registration batch、manifest/checkout、聚合 policy artifact、aggregate
//! initialization 的 checkpoint/member projections 与 aggregate index 记录。
//! GET 投影零写入；显式 action（`LogicalCodebaseBootstrapService::apply`）才
//! 推进或修复，并把 command replay 映射回既有 batch/initialization/index 的
//! idempotency 身份。
//!
//! 注意：现有 `AggregateInitializationStepKind::V1` 的五步
//! （machine_skills → aggregate_preflight → pre_check → rule_and_mcp_config →
//! openspec_and_examples）是独立的 durable 协议，与本模块的 C4 冷启动五步
//! （identity → manifest/checkout → rules/policy → member index → aggregate
//! index active）互不冒充、不共享名称。

use std::path::PathBuf;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexError, AggregateIndexStatus, AggregateIndexStore,
};

fn map_index_error(error: AggregateIndexError) -> ProductStoreError {
    ProductStoreError::InvalidRecord {
        kind: "aggregate_index",
        reason: error.to_string(),
    }
}
use crate::product::logical_codebase::provider_admission_preflight::BootstrapActionKind;
use crate::product::logical_codebase::repository_routing::{
    AuthorityPolicyReference, RepositoryAuthorityResolver, RepositoryRoutingRequest,
    RepositoryTargetKind,
};

// 本文件按仓库惯例拆入 `.inc.rs`（同模块命名空间，符号路径与可见性不变），
// 保持每个文件低于 large_file_guard 的 1200 行上限：
// - bootstrap_types.inc.rs：投影/动作契约类型（五步枚举、checkpoint、notice、action 结果）。
// - bootstrap_projector.inc.rs：readiness 只读投影状态机（LogicalCodebaseBootstrapProjector）。
// - bootstrap_service.inc.rs：bootstrap action 状态机（LogicalCodebaseBootstrapService）。
// - bootstrap_tests.inc.rs：tests 模块。
include!("bootstrap_types.inc.rs");
include!("bootstrap_projector.inc.rs");
include!("bootstrap_service.inc.rs");
include!("bootstrap_tests.inc.rs");
