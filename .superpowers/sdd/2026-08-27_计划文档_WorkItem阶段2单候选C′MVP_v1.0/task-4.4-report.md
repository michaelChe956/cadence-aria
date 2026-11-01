# Task 4.4 验收报告：复评 parser/classifier 失败关闭

## Status

- 已完成实现，已按精确路径创建一个原子提交。
- 实现依赖 Task 2.5（`48f9c79d`）提供的 `WorkItemPlanSourceStore`、`PlanCandidateMechanicalReport` 与 typed get API；并接入 Task 3.4（`60879f15`）新增的 durable IR/report refs。

## 实现摘要

- SingleCandidate Verification 现在要求 invocation scope 的 `repaired_revision_id` 与 `mechanical_report_ref` 分别逐字匹配 durable `plan_candidate_ir_ref` 与 `mechanical_report_ref`。
- 以 `SourceStoreScope(project, issue, plan)` 和 canonical refs 调用 `get_plan_candidate_ir`、`get_mechanical_report`；缺报告、ref/record identity 错位、source hash 或 compiler version 不一致均收敛为 `Fatal(ProtocolViolation)` / `verification_scope_violation`，并 durable `Failed`。
- scope digest 在 reload durable scope 前检查，防止坏的本 invocation scope 被旧有效 scope 静默覆盖。
- `classify_review` 仅接收已验证的 mechanical report ref 并要求与 Verification scope 精确相等；初始 scope 禁止携带 verified report。finding 分类本身未改变。
- Verification parser/schema/structured-output 失败转换为不可 repair 的 `verification_scope_violation`，经既有 classifier-fatal 路径 durable failed，不再 fallback 为 NeedsHuman；UnknownCategory/UnknownClassHint 原有 fatal 链路保持。
- 复评的 original fingerprint 仍由阶段 1 evaluator 路由为 human/stop，不会第二次自动 repair；新 fingerprint 仍为 `HumanRequired(VerificationNewFindings)`；未引入 changed-path 或 region 归因。
- 为保持 `routing.rs` 低于 1200 行，将 scope-local report 验证、坏 digest 收口与 route outcome 转换移动到既有 `routing_scope.rs`。

## 变更文件

- `src/product/work_item_plan_policy/classify.rs`
- `src/product/work_item_plan_policy/tests_classify.rs`
- `src/product/workspace_engine/review/routing.rs`
- `src/product/workspace_engine/review/routing_scope.rs`
- `src/product/workspace_engine/review/structured_output.rs`
- `src/product/workspace_engine/tests/single_candidate_prompt.rs`
- `.superpowers/sdd/2026-08-27_计划文档_WorkItem阶段2单候选C′MVP_v1.0/task-4.4-report.md`

未触碰并行 Task 3.4 的 `src/product/lifecycle_store/workspace.rs`、`src/product/workspace_engine/compile.rs` 或其报告草稿。

## TDD 与新增/更新测试

`single_candidate_prompt.rs` 新增 8 个 `verification_scope_` 测试：

1. report 缺失 → protocol fatal、durable failed、diagnostic。
2. canonical report ref 指向 record identity 错位 → protocol fatal、durable failed、diagnostic。
3. report compiler version 与 repaired IR 不同 → protocol fatal、durable failed、diagnostic。
4. report source hash 与 repaired IR 不同 → protocol fatal、durable failed、diagnostic。
5. 本 invocation scope digest 错 → protocol fatal、durable failed、diagnostic。
6. Verification parser 未收到 structured output → 不 fallback 到 NeedsHuman，protocol fatal、durable failed。
7. original fingerprint 重现 → `RepeatedFingerprint` human gate，`provider_start_ledger` 保持单个既有 repair start。
8. 新 fingerprint → `VerificationNewFindings` human gate，未创建 repair start。

`tests_classify.rs` 补充 verified mechanical report ref 不匹配时的 classifier 拒绝断言；14 条 golden finding 的分类契约仍由原测试覆盖。

## 验证记录

| 命令 | 结果 | 摘要 |
| --- | --- | --- |
| `cargo fmt --check` | 通过 | 最终格式检查通过。 |
| `cargo check --locked` | 通过 | 最终树编译通过。 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 通过 | 无 warning。 |
| `cargo test --locked --lib workspace_engine::tests::single_candidate_prompt::verification_scope -- --list` | 通过 | 已验证匹配 **8 项**。 |
| `cargo test --locked --lib workspace_engine::tests::single_candidate_prompt::verification_scope` | 通过 | 8 passed。 |
| `cargo test --locked --lib work_item_plan_policy::tests_classify -- --list` | 通过 | 已验证匹配 **6 项**（其中 golden fixture 固定 14 条）。 |
| `cargo test --locked --lib work_item_plan_policy::tests_classify` | 通过 | 6 passed。 |
| `cargo test --locked --lib workspace_engine::tests::severity_three_tier -- --list` | 失败关闭 | brief 过滤名在扁平 `include!` 测试结构中匹配 **0 项**；未将 0 项视为成功。 |
| `cargo test --locked --lib historical_review_ -- --list` | 通过 | 按 controller 裁决替代为 `severity_three_tier.rs` 的真实函数前缀；已验证匹配 **3 项**。 |
| `cargo test --locked --lib historical_review_` | 通过 | 3 passed。 |
| `cargo test --locked` | 已执行；非本任务红 | 首次：仅 `it_core::large_file_guard` 失败，归因 Task 3.4 的 `lifecycle_store/workspace.rs`（1453 行）与 `workspace_engine/compile.rs`（1521 行）；我方 `routing.rs` 已拆至 1176 行。第二次：仅已知 kimi terminal flaky `concurrent_streams_share_budget_without_overdraw` 失败。 |
| `cargo test --locked --lib kimi_code_provider::client_services::terminal -- --list` | 通过 | 已验证匹配 **12 项**。 |
| `cargo test --locked --lib kimi_code_provider::client_services::terminal` | 通过 | flaky 家族单跑 12 passed。 |

## 全量门禁残余风险

- 当前 HEAD 的 `large_file_guard` 仍会因 Task 3.4 已提交的 `src/product/lifecycle_store/workspace.rs`（1453 行）和 `src/product/workspace_engine/compile.rs`（1521 行）失败；controller 已裁决其 100% 归属 3.4，Task 4.4 未修改这两个文件，也未将其纳入提交。controller 在 3.4 修复后统一重跑全量门禁。
- 最终全量 `cargo test --locked` 另一次只命中已知 kimi terminal 并发 flaky；随后按规则单跑同一族 12/12 通过。
- 除上述并行 3.4 guard 红色与已复核 flaky 外，无已知 Task 4.4 残余风险。

## 暂存与提交

- 仅以精确路径暂存本任务 7 个文件；未使用 `git add -A`、`git add .` 或 `-j`。
- 提交后已检查无暂存文件。
