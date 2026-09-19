use axum::http::{Method, StatusCode};
use serde_json::json;

use crate::web_work_item_generation::{
    app_with_confirmed_story_and_design, request_json, valid_split_output,
};

// 退役留档（T5/REQ-RET-02）：`confirm_creates_child_work_item_sessions` 直接驱动已删除的 legacy 决策面
// （原 #[ignore]：legacy full-candidate confirm 流已被 WP2 outline 生成取代；本测含 author_decision/human_confirm 退役 wire 字面量），
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`confirm_uses_session_entity_plan_id` 直接驱动已删除的 legacy 决策面（同上），
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`confirm_is_idempotent_on_retry` 直接驱动已删除的 legacy 决策面（同上），
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：confirm 模块 ws 连接/准备夹具（prepare_and_start_generation 等）为上者专用，随消息族一并退役。

#[tokio::test]
async fn delete_legacy_rest_routes_returns_404() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "x",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/some_plan/confirm",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/some_plan/change-request",
        json!({"feedback": "x"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
