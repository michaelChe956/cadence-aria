# Task 4.1 实施报告

## 状态
已完成：聚合索引 Building-first 状态机、双快照审计证据和统一 single-writer 锁已实现。

## 变更
- `build`、`rebuild`、`sync_and_verify` 共用 project-scoped writer lock；`rebuild` 使用锁内私有 apply body，不在锁内递归调用 `build`。
- `apply_index` 在 CodeGraph 命令前持久化 `Building`，并发读取可观察到 rebuilding；成功通过 `replace_active` 发布 `Active`。
- 捕获 before/after member snapshots；首建漂移标记新记录 `Failed`，重建/同步期间漂移标记新记录 `Stale`，active 仍指向 before generation；after 快照保留审计证据。
- `AggregateIndexRecord.observed_after_member_snapshots` 增加 serde default；store 增加已创建 record 的更新入口。
- 现有 operation 测试移至 `operation_tests.inc.rs`，新增 Building 可见、首建漂移失败、重建漂移 stale、双快照断言。

## 验证
- `cargo fmt --check`：通过
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过
- `cargo check --locked`：通过
- `cargo test --locked --lib aggregate_index`：40 passed
- `cargo test --locked`：381 passed, 12 ignored

## 风险
- 无已知阻塞风险；重建命令继续使用 `init`，同步入口使用 `sync`，均在统一 writer lock 下执行。

## Fix round 1
- 补充 `sync_drift_marks_new_record_stale_and_keeps_before_active` 回归测试，使用 `sync` 命令期间成员 revision 漂移，断言新 record 为 `Stale`、before generation 仍为 active，并保留 after snapshot 审计证据。
- Fake CodeGraph runner 增加 sync 漂移脚本与 `sync .` 响应；同步入口注释改为说明 `_prior` 是 freshness evidence，实际 active 指针由 operation 重新读取。
- 验证：`cargo test --locked --lib sync_drift_marks_new_record_stale_and_keeps_before_active` 通过。
