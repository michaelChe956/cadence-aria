//! 集成测试入口：product 域。
#[path = "it_product/product_coding_attempt_store.rs"]
mod product_coding_attempt_store;
#[path = "it_product/product_coding_models.rs"]
mod product_coding_models;
#[path = "it_product/product_coding_workspace_engine.rs"]
mod product_coding_workspace_engine;
#[path = "it_product/product_coding_workspace_runner.rs"]
mod product_coding_workspace_runner;
#[path = "it_product/product_git_workspace_service.rs"]
mod product_git_workspace_service;
#[path = "it_product/product_index.rs"]
mod product_index;
#[path = "it_product/product_lifecycle_store.rs"]
mod product_lifecycle_store;
#[path = "it_product/product_runtime_compat.rs"]
mod product_runtime_compat;
#[path = "it_product/product_test_executor.rs"]
mod product_test_executor;
#[path = "it_product/product_work_item_models.rs"]
mod product_work_item_models;
#[path = "it_product/product_work_item_plan_store.rs"]
mod product_work_item_plan_store;
#[path = "it_product/product_work_item_split_engine.rs"]
mod product_work_item_split_engine;
#[path = "it_product/product_work_item_split_validator.rs"]
mod product_work_item_split_validator;

/// Fixture 初始状态播种：将 attempt 置为 Running 可执行态并写入 admission 会话
/// 标记后直接落盘——模拟「已经 admission CAS 进入」的合法可执行状态（状态机已
/// 封死直达 Running 通道；播种不经 `update_attempt_status`，等价于 lib 测试的
/// `seed_running_attempt_for_test`，仅用于集成测试初始状态，不代表生产写路径）。
pub(crate) fn seed_coding_attempt_running(
    store: &cadence_aria::product::coding_attempt_store::CodingAttemptStore,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> cadence_aria::product::coding_models::CodingExecutionAttempt {
    let mut attempt = store
        .get_attempt(project_id, issue_id, attempt_id)
        .expect("load coding attempt for Running seeding");
    attempt.status = cadence_aria::product::coding_models::CodingAttemptStatus::Running;
    attempt.admission_ticket_consumed_at = Some(chrono::Utc::now().to_rfc3339());
    cadence_aria::product::json_store::write_json(
        &store
            .paths()
            .issue_root(&attempt.project_id, &attempt.issue_id)
            .join("coding-attempts")
            .join(format!("{}.json", attempt.id)),
        &attempt,
    )
    .expect("seed running coding attempt record");
    attempt
}

/// Fixture 初始状态播种：将任意 attempt record 直接落盘（不经过受控状态转换，
/// 也不经过 `update_attempt_non_status_fields` 的冻结字段读改写），用于模拟磁盘上
/// 已存在的 legacy record（固定 id / 重复 legacy id 的 scoped 读写场景）。
/// 仅用于集成测试初始状态，不代表生产写路径。
pub(crate) fn write_coding_attempt_record_for_test(
    store: &cadence_aria::product::coding_attempt_store::CodingAttemptStore,
    attempt: &cadence_aria::product::coding_models::CodingExecutionAttempt,
) {
    cadence_aria::product::json_store::write_json(
        &store
            .paths()
            .issue_root(&attempt.project_id, &attempt.issue_id)
            .join("coding-attempts")
            .join(format!("{}.json", attempt.id)),
        attempt,
    )
    .expect("seed legacy coding attempt record");
}
