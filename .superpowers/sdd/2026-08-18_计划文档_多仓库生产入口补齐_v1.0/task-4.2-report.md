# Task 4.2 实施报告

## 状态
已完成：`AggregateIndexFreshnessService::assess` 保留 Degraded last-known-good 状态和 warning，避免无漂移时误报 Active。

## 变更
- freshness 通过 `AggregateIndexStore::active_required` 读取 Active、Degraded 或 Stale 的可读记录。
- membership/snapshot 漂移仍优先返回 Stale；无漂移的 Degraded 记录返回 Degraded，并保留 warning 作为可审计 reason。
- `sync_if_stale` 仅在 freshness 状态为 Stale 时调用 CodeGraph 同步；Degraded 记录原样返回，不自动重建。
- 新增 Degraded assess 回归测试及 `sync_if_stale` 不触发同步的回归测试。

## 验证
- `cargo fmt --check`：通过
- `cargo check --locked`：通过
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过
- `cargo test --locked --lib assess_preserves_degraded_last_known_good_instead_of_reporting_active`：通过（1 passed）
- `cargo test --locked --lib freshness`：通过（10 passed）
- `cargo test --locked`：通过（381 passed，12 ignored；包含 2 个 doc-tests）

## 风险
无已知阻塞风险。完整测试首次运行曾出现一次并发相关的 `provider_error_routes::parse_error_timeout_and_incompatible_output_have_stable_routes` 偶发失败，单测重跑及完整测试重跑均通过；与本次 freshness 改动无关。
