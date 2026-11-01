# S1 跨 attempt handoff 解析 交付报告

状态：DONE

## 根因

`authoritative_resolved_handoff_revision_ids` 只用 `list_coding_units(attempt)` 查依赖 unit，而 attempt 是 per-target 桶。依赖落在另一个桶（另一个 attempt）时 `matches.next()` 为 None，返回 `WorkItemHandoffMissing`。三处调用同一根因：

- `plan_defect.rs` `active_unit_run_projection`（经 `render_coder_unit_run_context` / `render_reviewer_unit_run_context`）
- `group_completion.rs` `preflight_group_unit_completion`
- `plan_defect_routing.rs` `authoritative_group_reviewer_bindings`

## 改动

- 新增 `coding_workspace_engine/cross_attempt_handoff.rs`（注册于 `mod.rs`）
  - `resolve_dependency_handoff_id`：本桶命中走原逻辑；未命中才跨 attempt。
  - `cross_attempt_dependency`：`list_attempts_for_work_item_group` 取全 plan attempt，排除自身；候选过滤 = unit `Completed` + 有 `latest_handoff_revision_id` + 有 `completion_commit` + `work_item_revision_id` 等于当前 attempt 绑定 plan revision 中该 WI 的 revision；多个候选取 `attempt_no` 最大。
  - `validate_dependency_handoff`：原 6 项绑定校验原样提取，同桶与跨桶共用，`list_coding_unit_runs` 用 owner attempt。
  - fail-closed：无合格候选 → `WorkItemHandoffMissing("<dep>: ...")`，消息列出每个 owner attempt 的 id/attempt_no/unit id/状态/revision/handoff/commit；全 plan 无 attempt 承载则明说。
- `plan_defect.rs`：`authoritative_resolved_handoff_revision_ids` 改为委托新模块；删除已无用的 import。三个调用点签名不变，零改动。
- 不动：`group_dependency_gate.rs`（桶外依赖仍 Waiting，`sc_group_dependency_gate` 语义锁定）、`runtime_impact.rs:478`（amendment 路径，设计标为后续风险）。

## 测试（TDD，先红后绿）

新增 `tests/cross_attempt_handoff.rs`，8 例；红：7 失败 1 通过（"全 plan 无此 unit"在旧代码下本就 Err）；绿：8 通过。

- 跨桶依赖完成 → 返回 owner handoff id
- 多 owner 取 attempt_no 最大
- 反例：owner 未 Completed（消息含 WI、owner attempt、状态）
- 反例：Completed 但未发布 handoff
- 反例：work_item_revision_id 与 plan 绑定不一致（stale）
- 反例：unit completion_commit 与 handoff 不一致 → `unit_run_handoff_binding_mismatch`
- 反例：全 plan 无 attempt 承载该 WI
- 贯通：`render_coder_unit_run_context` 创建 run 且 prompt 含 handoff id → `complete_group_unit_after_code_review` preflight 通过 → `authoritative_group_reviewer_bindings` 解析 → `build_group_final_readiness_snapshot` 为 `Complete`

## 验证（均已实跑）

- `cargo test --locked --lib cross_attempt_handoff`：8 passed
- `cargo test --locked --lib product::coding_workspace_engine`：601 passed，含 `sc_group_dependency_gate`、`group_completion_authority`、`group_start_context`、`multi_target_delivery`、`retry_push`
- `cargo test --locked --lib`：4730 passed，0 failed，7 ignored
- `cargo clippy --all-targets --all-features --locked`：我的文件无告警
- `rustfmt --check`：仅我的三个文件，通过

## 备注

- 全量 lib 首跑曾见 1 个失败（`runner_death_and_recovery_sweeps_exempt_incomplete_journal_retry_run`），属并行 S4 在途的测试文件（`web/coding_ws_handler/tests/failed_review_recovery/runner.rs`，非本片），S4 落盘后重跑全绿。
- 只改动 `*.rs` 中本片文件；未触碰 `git add -A` 范围外文件。
