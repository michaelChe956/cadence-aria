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
use crate::product::logical_codebase::aggregate_initialization::{
    AggregateInitializationOperation, AggregateInitializationOperationStatus,
    AggregateInitializationStepKind, AggregateInitializationStepRecord,
    AggregateInitializationStepStatus,
};
use crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
use crate::product::logical_codebase::policy::SessionPolicyAction;
use crate::product::logical_codebase::provider_gateway::{
    LogicalCodebaseProviderGateway, ProviderGatewayError, SessionLaunchRequest,
};
use crate::product::logical_codebase::repository_routing::{
    AuthorityPolicyReference, RepositoryAuthorityResolver, RepositoryRoutingRequest,
    RepositoryTargetKind, ResolvedTargetIdentity,
};
use crate::product::logical_codebase::store::LogicalCodebaseStore;
use crate::product::logical_codebase::types::{
    CheckoutKind, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
};

// 本文件按仓库惯例拆入 `.inc.rs`（同模块命名空间，符号路径与可见性不变），
// 保持每个文件低于 large_file_guard 的 1200 行上限：
// - provider_admission_credential.inc.rs：credential/phase/marker 块
//   （BootstrapActionKind、BootstrapPhaseCredential、BootstrapExecutorMarker）。
// - provider_admission_check.inc.rs：check 面与共享等待判定 helpers
//   （LogicalCodebaseProviderAdmissionPreflight::check、ensure_bootstrap_* 等）。
// - provider_admission_preflight_tests.inc.rs：tests 模块。
include!("provider_admission_credential.inc.rs");
include!("provider_admission_check.inc.rs");
include!("provider_admission_preflight_tests.inc.rs");
