use axum::http::{Method, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::{
    CreateIssueWorkItemPlanInput, CreateWorkItemInput, LifecycleStore,
};
use cadence_aria::product::models::{
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, WorkItemKind, WorkItemPlanStatus,
};
use serde_json::json;

use crate::web_work_item_generation::{
    app_with_confirmed_story_and_design, request_json, valid_split_output,
};

// 退役留档（T5/REQ-RET-02）：`work_item_plan_start_generation_returns_outline_artifact` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`work_item_plan_author_streams_provider_output_before_outline_artifact` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`work_item_plan_author_completes_provider_node_before_author_confirm` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`outline_review_revise_requires_decision_before_next_outline_provider_run` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`work_item_plan_outline_review_revision_reenters_review_after_revised_outline_confirm` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：author 模块 ws 连接辅助（connect_ws/recv_ws_until/enable_test_controls）为 staged author 族测试专用，随消息族一并退役。
