# Task 3.3：五 checkpoint failpoint/recovery parity 报告

## 结论

已完成抽取前后 Work Item Plan initial compile 的 failpoint/recovery parity 证明，并修复了一个真实恢复分流差异：initial publication 在 `Prepared` journal 阶段（包括 `PlanActivated` failpoint 在 active-lineage 写入后触发的窗口）恢复时，Continue 现在统一先经过 `resume_initial_plan_compile_transaction`，写入 `publication_resumed`，再进入 finalizer；不会把 publication checkpoint case 误当作已完成 active outcome 而跳过恢复 cursor。

## 覆盖矩阵

### 五个 finalizer checkpoint

`compile_recovery_continue_replays_each_partial_finalizer_checkpoint_after_restart` 逐项覆盖：

1. `PlanSummaryPrepared`
2. `FirstChildSessionEnsured`
3. `FirstChildBindingEnsured`
4. `FirstChildContextPrepared`
5. `CompileReportPersisted`

每项均注入 failpoint，捕获 `RecoveryRequired` transaction，销毁并用 `WorkspaceEngine::new_persistent` 重建，再执行 Continue。断言包括：原 compile ID、唯一 transaction、终态 committed、child session/runtime binding/context、唯一 compile report、Plan 状态、provider ledger 字节前后不变、新增 started 为 0、无 `ProviderRunRequested`，以及与 3.1 正常编译 baseline 的 finalizer observation parity。

### 五个 initial publication checkpoint

`compile_recovery_continue_replays_each_pre_active_publication_checkpoint_after_restart` 逐项覆盖：

1. `LineageWritten`
2. `FirstWorkItemArtifactsWritten`
3. `PlanArtifactsWritten`
4. `FirstWorkItemActivated`
5. `PlanActivated`

每项均验证第一次中断 journal 为 `Prepared`、保留 compile/publication identity、`artifact_fingerprint` 和完整 publication artifacts；重启 Continue 必须先出现 `publication_resumed`，随后才出现首个 finalizer cursor。恢复后验证 journal 进入 `PlanActivated`、fingerprint/IDs/artifacts 完整一致、PlanRevision/WorkItemRevision/VerificationPlanRevision/ProjectionBundle 内容无重复且字节语义一致、同一 transaction committed、provider ledger 不变且新增 started 为 0、无 provider request，并再次 replay publication 验证幂等。

## 关键实现变更

`handle_work_item_plan_compile_recovery_action(Continue)` 在读取 initial publication journal 后，对 `Prepared` journal 强制走 `resume_initial_plan_compile_transaction`；缺 journal 仍沿用既有 active-outcome reload/transaction resume 分支，避免扩大正常路径范围。该修复保留原 publication journal put、finalizer cursor 和 recovery 语义，不创建新 transaction、不吞掉 `publication_resumed`。

另在 `part_15.rs` 增加：

- initial publication identity/fingerprint 的确定性与字段完整性断言；
- recovered outcome 与 interrupted outcome 的完整结构 parity 断言；
- 正常编译 baseline 与 failpoint recovery finalizer observation 的对照辅助断言。

## 测试与门禁证据

先列举（匹配数）：

- `compile_recovery_continue_replays_each_partial_finalizer_checkpoint_after_restart -- --list`：已验证匹配 1 项。
- `compile_recovery_continue_replays_pre_active_publication_with_same_tx_after_restart -- --list`：已验证匹配 1 项。
- `initial_plan_publication_resumes_each_store_write_failure_after_restart -- --list`：已验证匹配 1 项。
- `work_item_plan_initial_compile -- --list`：已验证匹配 13 项（达到 brief 要求的至少 13 项）。
- `compile_recovery_continue -- --list`：已验证匹配 4 项。
- `initial_plan_publication -- --list`：已验证匹配 5 项。
- `workspace_engine::tests -- --list`：已验证匹配 890 项。

执行结果：

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过。
- `cargo test --locked --lib compile_recovery_continue`：4 passed。
- `cargo test --locked --lib initial_plan_publication`：5 passed。
- `cargo test --locked --lib work_item_plan_initial_compile`：13 passed。
- `cargo test --locked --lib workspace_engine::tests`：890 passed，1 ignored（既有已知 gap）。

## Acceptance Contract 证据

- changed-files：`src/product/workspace_engine/compile.rs`、`src/product/workspace_engine/tests/part_03/part_09.rs`、`src/product/workspace_engine/tests/part_03/part_15.rs`、本报告。
- tests-added：五个 finalizer checkpoint 对照 baseline；五个 initial publication checkpoint engine restart/recovery；publication identity/fingerprint deterministic tests；完整 outcome parity assertions。
- residual-risks：无新增已知风险；全量 `workspace_engine::tests` 仍保留 1 个既有 ignored test（legacy review budget gap），与本任务无关。
- no-staged-files：提交后核验 `git status --short` 为空。
- review gate：等待独立 reviewer 核对 3.1 observer baseline、publication cursor 顺序、failpoint 矩阵、ledger 断言口径及恢复分流范围。
