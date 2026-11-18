// 聚合索引失败/降级记录的**诊断码口径**用例。
//
// 由 `provider_diagnostics.rs` 的既有 `mod tests` 以 `include!` 引入
// （large_file_guard 1200 行上限外挪，纯外挪，无行为变化），因此用例全路径
// 仍是 `web::handlers::provider_diagnostics::tests::<fn>`，过滤器仍是
// `cargo test --locked --lib provider_diagnostics`。
//
// 本文件只断言「码」这一层事实：诊断结论、字段形状都不动。

use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
};

/// 造一条失败/降级世代（身份固定，便于断言）。
fn failure_record(aggregate_index_id: &str, status: AggregateIndexStatus) -> AggregateIndexRecord {
    let mut record = AggregateIndexRecord::building(
        aggregate_index_id.to_string(),
        "project_0001".to_string(),
        1,
        Vec::new(),
        "2026-11-18T00:00:00Z".to_string(),
    );
    record.status = status;
    record
}

/// 落一条带 warning 的失败/降级世代，返回诊断响应里 `last_failure.code`。
fn last_failure_code(root: &Path, status: AggregateIndexStatus, warning: &str) -> String {
    let paths = ProductAppPaths::new(root.join(".aria"));
    let store = AggregateIndexStore::new(paths);
    let aggregate_index_id = "aggregate_index_gen";
    store
        .create("project_0001", failure_record(aggregate_index_id, status))
        .expect("create aggregate index record");
    store
        .mark_status(
            "project_0001",
            aggregate_index_id,
            status,
            Some(warning.to_string()),
        )
        .expect("mark aggregate index status");
    let body = build_diagnostics(&state(root));
    body["aggregate_index"]["last_failure"]["code"]
        .as_str()
        .expect("last_failure code")
        .to_string()
}

/// 封套可解析 ⇒ 诊断项的码是**内层稳定码**（与冷启动卡片、HTTP 重建错误
/// 出口同源），不再是通用状态码。
#[test]
fn diagnostics_failed_last_failure_exposes_inner_stable_code() {
    let root = tempdir().expect("root");
    let code = last_failure_code(
        root.path(),
        AggregateIndexStatus::Failed,
        "aggregate_index_failed:codegraph_missing: codegraph cli not found",
    );
    assert_eq!(code, "codegraph_missing");
}

/// Degraded 世代同理：封套可解析 ⇒ 内层稳定码。
#[test]
fn diagnostics_degraded_last_failure_exposes_inner_stable_code() {
    let root = tempdir().expect("root");
    let code = last_failure_code(
        root.path(),
        AggregateIndexStatus::Degraded,
        "aggregate_index_degraded:aggregate_index_member_source_excluded: \
         included members [b]; move it into the member main checkout",
    );
    assert_eq!(code, "aggregate_index_member_source_excluded");
}

/// 封套不可解析 ⇒ 回落到状态级码（不猜码）。
#[test]
fn diagnostics_last_failure_falls_back_to_status_code_when_envelope_unparsable() {
    let root = tempdir().expect("root");
    let code = last_failure_code(
        root.path(),
        AggregateIndexStatus::Failed,
        "aggregate index build aborted before member scan",
    );
    assert_eq!(code, "aggregate_index_failed");
}
