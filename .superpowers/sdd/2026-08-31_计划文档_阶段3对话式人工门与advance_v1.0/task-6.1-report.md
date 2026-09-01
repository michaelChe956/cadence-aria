# Task 6.1 报告

- 任务：SC-linked dependency gate、等待态与 fail-closed 诊断
- BASE：`9e4d4810`
- Commit：待提交
- Commit message：`feat(coding): gate SC group units on dependency handoffs`

## 变更

- 新增 `GroupDependencyGateStatus` 与 `GroupDependencyGateSnapshot` durable 模型。
- 新增 `CodingAttemptStore` 的 dependency gate snapshot 读写和 attempt 子目录路径。
- 新增 SC selector：校验 attempt binding、active plan revision、dependency graph、同 attempt logical work item 唯一性、未知/自依赖/环，并按拓扑层、`order_index`、logical work item、unit id 稳定选择。
- 依赖 readiness 要求 dependency `Completed`、handoff 指针存在且可读、handoff logical WI/revision/run/commit identity 匹配当前 unit/run；binding 通过 active plan revision 与 plan work-item binding 校验。
- `advance_to_next_group_unit` 仅对 `admission_kind=sc_advance` 走新 selector；Waiting/FailedClosed 不进入 `ReviewRequest`，不启动 provider；Legacy 保留原 `order_index` 分支。
- 新增 4 项 gate classifier 回归测试，覆盖拓扑阻塞、Waiting、handoff binding mismatch、unknown/self/cycle reason code。

## 验证

- RED 端：`cargo test --locked --lib sc_group_dependency_gate -- --list` 首次实现前为 `0 tests`；实现后 list 为 `4 tests`。
- GREEN：`cargo test --locked --lib sc_group_dependency_gate`：4 passed。
- `cargo check --locked --lib`：passed。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：passed。
- `cargo test --locked --lib runtime_handoff_replay -- --list`：1 test；运行：1 passed。
- `cargo test --locked --lib group_final_readiness -- --list`：29 tests；运行：29 passed。
- `cargo test --locked --lib`：3017 passed，0 failed，2 ignored。
- `cargo fmt --check`：passed（格式化后复核）。
- `git diff --cached --check`：passed。

## 风险

- `HandoffRevision` 当前模型没有独立 `plan_revision_id` 字段，因此 binding 匹配依据为 attempt binding 对应的 active plan revision、plan work-item binding、unit revision，以及 handoff 的 unit/run identity；未扩展 handoff 公共模型。
- 新增单测主要验证 selector 的拓扑/分类函数；完整 provider ledger 零写入证据仍依赖现有调度路径与后续 6.2 隔离回归。
