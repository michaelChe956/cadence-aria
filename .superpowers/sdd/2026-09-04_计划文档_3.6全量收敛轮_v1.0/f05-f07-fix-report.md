# F-05 / F-07 修复报告

## 范围

- F-07：SingleCandidate 用户显式重开在前次失败后必须申请新的 durable provider-start ledger 键并实际启动 provider；Completed 会话保持拒绝且不进入僵尸 Running。
- F-05：重开不会把仍被 `active_node_id` 指向的 Failed author node 改写为 Completed，也不会覆盖其 `completed_at` 或 `summary`。

## 根因与修复

1. provider-start 键以 ledger 长度推导，同时对已经处于 Generate 的恢复重放复用最后一把键，保留 one-shot 恢复语义。
2. `StartGeneration` 在 SingleCandidate 且 durable phase 为 Failed 时，先用存储层 CAS 重臂为 Prepare/Open；Completed 返回冲突。该重臂只位于用户显式 `start_generation`，恢复链不调用它。
3. SingleCandidate 路径不再在 reservation 之前预烧 durable Running；成功 reservation 是唯一写入 Generate/Running 和 ledger 的原子核。
4. `complete_active_node` 仅对 Active 节点转换 Completed，终态节点完全保持历史字段。
5. `AlreadyFinished` 分支补齐 manager `finish_run`，释放 active-run 注册。
6. 已消费且无可启动实例的 reservation 现在经过 Message 失败路径，生成失败节点、恢复 PrepareContext 并输出可见错误。

## TDD 证据

红阶段：

- `single_candidate_failed_reopen_claims_next_ledger_key_and_starts_provider`：修复前第二次重开不启动 provider，停留 Running。
- `single_candidate_completed_reopen_is_rejected_without_starting_or_running`：修复前 Completed 重开未输出拒绝且会预烧 Running。
- `single_candidate_reopen_preserves_failed_author_node_terminal_fields`：修复前 Failed author node 被覆盖为 Completed。

绿阶段（新增锚）：

- `single_candidate_failed_reopen_claims_next_ledger_key_and_starts_provider` 通过。
- `single_candidate_completed_reopen_is_rejected_without_starting_or_running` 通过。
- `single_candidate_reopen_preserves_failed_author_node_terminal_fields` 通过。
- `explicit_start_generation_rearms_failed_single_candidate_but_rejects_completed` 通过。

## 定向验证

已通过：

- `cargo test --locked --lib single_candidate`：108 passed。
- `cargo test --locked --lib provider_run`：29 passed。
- `cargo test --locked --lib timeline`：21 passed。
- `cargo test --locked --lib single_candidate_provider_start_reservation_is_one_shot_and_durable`：1 passed。
- `cargo test --locked --lib single_candidate_revise_route_does_not_misfire_a_second_followup_review`：1 passed。
- `cargo test --locked --lib single_candidate_author_relay_still_supersedes_in_flight_run`：1 passed。
- `rustfmt --edition 2024 --check`（本组 Rust 文件）通过。
- `git diff --check` 通过。

本组完成时，`cargo fmt --check` 与 `git diff --check` 已通过。全局 `cargo clippy --all-targets --all-features -- -D warnings` 由集成阶段在所有并行批次落地后统一执行。

## 影响文件

- `src/product/lifecycle_store/workspace_single_candidate.rs`
- `src/product/workspace_engine/single_candidate.rs`
- `src/product/workspace_engine/lifecycle.rs`
- `src/product/workspace_engine/session_state/timeline.rs`
- `src/product/workspace_engine/tests/single_candidate_recovery.rs`
- `src/web/workspace_ws_handler/run/provider_run.rs`
- `src/web/workspace_ws_handler/run/single_candidate.rs`
- `src/web/workspace_ws_handler/tests/single_candidate_provider_run.rs`
