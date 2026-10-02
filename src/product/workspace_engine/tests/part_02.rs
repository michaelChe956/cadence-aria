use crate::product::lifecycle_store::spec::ExistingSpecRecord;
use crate::product::lifecycle_store::{
    AggregateDesignSpecScope, AggregateStorySpecScope,
};
use crate::product::logical_codebase::{PlanningContextSnapshot, PlanningContextSnapshotStore};

// part_02 拆分（large_file_guard 1200 行上限）：三个用例族经 include! 挂载，与本文档同一模块作用域，测试路径与行为零变化（纯移动）。
include!("part_02/artifact_constraints_and_review_input.rs");
include!("part_02/engine_run_persistence.rs");
include!("part_02/provider_drive_writeback.rs");
