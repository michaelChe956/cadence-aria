//! Durable store for `AggregateInitializationOperation`.
//!
//! Records live at
//! `.aria/projects/{project}/logical-codebase/aggregate-initializations/{operation_id}.json`
//! and are byte-stable. The store enforces the five-step state machine, rejects
//! jumps/reorders, and requires an output checkpoint before a step can be marked
//! completed.

use std::path::PathBuf;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

use super::aggregate_initialization::{
    AGGREGATE_INITIALIZATION_OPERATION_KIND, AggregateCancellationRecord,
    AggregateInitializationErrorRecord, AggregateInitializationOperation,
    AggregateInitializationOperationStatus, AggregateInitializationStepKind,
    AggregateInitializationStepRecord, AggregateInitializationStepStatus,
};

// 本文件按仓库惯例拆入 `.inc.rs`（同模块命名空间，符号路径与可见性不变），
// 保持每个文件低于 large_file_guard 的 1200 行上限：
// - aggregate_initialization_store_operations.inc.rs：durable store 的记录 IO 与
//   状态机写入（create/list/get/mark/checkpoint/finish/cancel/recover/reopen）。
// - aggregate_initialization_store_validation.inc.rs：root recipe 命令索引与
//   operation/step 记录形状校验（五步布局、合法状态与错误构造）。
// - aggregate_initialization_store_tests.inc.rs：tests 模块。
include!("aggregate_initialization_store_operations.inc.rs");
include!("aggregate_initialization_store_validation.inc.rs");
include!("aggregate_initialization_store_tests.inc.rs");
