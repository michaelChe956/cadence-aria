# Task 3.4 实施报告

## 状态
已完成实现并完成 Task 3.4 实施提交 `60879f15`，以及按 large-file 门禁要求的收尾拆分提交 `ac212292`。

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
- `cargo test --locked --lib ir_adapter`：8/8 通过。
- `cargo test --locked --lib work_item_plan_compiler::tests::publish_freshness`：4/4 通过。
- `cargo test --locked --lib work_item_plan_initial_compile`：13/13 通过。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过。
- 拆分后隔离导出 `cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo check --locked`：均通过。
- `cargo test --locked --test it_core large_file_guard`：通过；拆分后 `workspace.rs` 1181 行、`compile.rs` 989 行，两个拆分文件分别 287/536 行，均低于 1200 行。
- 隔离导出 `work_item_plan_initial_compile -- --list`：18 项；`work_item_revision_store::tests::initial_publication -- --list`：11 项。
- 隔离导出定向 initial compile 与 initial publication 测试：通过。
- 拆分后隔离导出 `cargo test --locked` 在 2822 项通过后仅被 Kimi flaky `kill_while_wait_pending_returns_killed` 阻断；按要求单跑该测试、`timeout_terminates_process_group` 与 `output_is_capped_and_truncation_flagged_once` 均通过。全量证据来自隔离导出（`git archive HEAD` + `web/dist` 软链），避免并行 worktree 状态污染。

## Acceptance evidence
- changed-files：见最终 `git show --stat`，不含 policy 并行文件。
- tests-added：`compile/ir_adapter_tests.rs`、CAS/hash/source-store 回归测试。
- residual-risks：SingleCandidate 端到端 provider/四 crash 边界仍需集成 fixture 进一步覆盖；全量命令仍有已知 Kimi terminal timing flaky，已单跑复核。
- no-staged-files：拆分提交后 index 复核为空；本报告在收尾提交后补入最终验证结果，故工作树仅保留该报告的未暂存文字更新。

## 收尾
- 实施提交：`60879f15 feat(workitem): adapt validated ir to shared compile core`。
- large-file 收尾提交：`ac212292 refactor(workitem): 拆分超线的 workspace/compile 文件`。
- 需独立 reviewer 对原 Task 3.4 的完整 crash-boundary、wrapper journal parity 与遗留 `expect` 硬条件作最终验收。

## Fix round 1
- 按 `review-3.4-verdict.md` Issues 1-9 补齐 oracle 测试：wrapper↔pure publication journal 逐字段等价、SingleCandidate canonical-ref-only 运行与 provenance ref/hash 在 revision/journal/transaction 三方一致、PlanArtifactsWritten 中断后的同 transaction 重启复用、normal/recovery artifact `created_at` 一致、execute IO 错误、stale/mechanical-error 首个 transaction put 前零写入，以及缺失 `flow_kind` 的旧 transaction Legacy 语义。
- 新增测试态 legacy compile store panic spy，SingleCandidate normal/recovery 路径在删除 draft/index/outline 后仍通过，证明未读取 legacy active-index/outline/draft。
- resume/validate 错误统一经过 `single_candidate_recovery_failed` durable Failed diagnostics 记录；保留原错误文本并与 `failure()` fail-closed 轨迹一致。
- Fix round 定向门禁：`task_3_4` 8/8、`ir_adapter` 8/8、`work_item_plan_initial_compile` 18/18、`initial_publication` 11/11；fmt/clippy 通过。
