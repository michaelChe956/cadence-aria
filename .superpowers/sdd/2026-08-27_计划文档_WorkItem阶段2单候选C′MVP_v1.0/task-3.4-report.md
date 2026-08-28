# Task 3.4 实施报告

## 状态
已完成实现并准备原子提交。提交 SHA 将在提交后补入本报告。

## 变更
- 新增 `compile/ir_adapter.rs`：将已验证 `PlanCandidateIr` 确定性组装为契约 13 字段 `InitialPlanCompileInput`，并组装 durable context。
- 新增 `compile/ir_adapter_tests.rs`：覆盖 IR adapter 确定性、mechanical Error fail-closed、provenance tuple 校验。
- `WorkspaceSessionRecord` 增加向后兼容的 SingleCandidate phase、source/IR/report refs、Approval tuple 与 compile reservation。
- `LifecycleStore` 增加 Approval SHA-256/NUL tuple CAS、compile reservation CAS、canonical ref / scope / phase / tuple 校验及稳定错误域。
- SingleCandidate compile 通过 durable refs 重载 source/IR/report/provenance，执行 freshness 校验后汇入共享 prepare→execute；Continue 校验 transaction refs 并复用 compile/provenance/publication IDs。
- execute 的 prepared work item / verification plan 两处 `expect` 改为可恢复 `Err`；initial finalizer artifact version 使用注入的 transaction/revision 时间，不再直接取时钟。
- source-store put 侧非法 scope/object ID 归类为 `SOURCE_STORE_MALFORMED_REF`，补充回归测试；现有 typed get API 保持 canonical ref 约束。

## TDD 与验证
- RED：新增 CAS 测试初次编译失败（缺失 phase、字段和 CAS API）。
- GREEN：实现 session schema、hash vector、Approval CAS、reservation CAS 后通过。
- `cargo test --locked --lib single_candidate_compile_id_uses_the_published_test_vector`：通过。
- `cargo test --locked --lib single_candidate_approval_and_reservation_are_cas_bound_to_durable_refs`：通过。
- `cargo test --locked --lib ir_adapter`：3/3 通过。
- `cargo test --locked --lib work_item_plan_compiler::tests::publish_freshness`：4/4 通过。
- `cargo test --locked --lib work_item_plan_initial_compile`：13/13 通过。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过。
- 共享 worktree 的 `cargo test --locked --lib` 被 4.4 未暂存 policy 中间态的既有失败阻断；该失败与本任务路径无关。全量证据使用提交后隔离导出采集。

## Acceptance evidence
- changed-files：见最终 `git show --stat`，不含 policy 并行文件。
- tests-added：`compile/ir_adapter_tests.rs`、CAS/hash/source-store 回归测试。
- residual-risks：SingleCandidate 端到端 provider/四 crash 边界仍需集成 fixture 进一步覆盖；共享树 policy 中间态需 controller 合并后复跑。
- no-staged-files：提交后复核。
