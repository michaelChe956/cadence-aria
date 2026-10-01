# Task 2.7 报告：Group/Internal Reviewer root cwd 迁移

状态：DONE　|　worktree: feat-b-0808-add-monorepo　|　基线 HEAD: eaacf753（2.6 交付后）

## 交付

| 面 | 变更 | 语义 |
|---|---|---|
| `lifecycle.rs` `resolve_launch_policy_for_role`（envelope 构造点，LC 分支） | `working_directory=canonical root`（`gateway.authority_root()`，与 2.1/2.3/2.6 同源取值）；`readable_roots=[root]`（随 cwd root 化） | cwd 与 target 分离；`writable_roots` 不随 cwd 扩大（Coder=target worktree（REQ-ENV-11），CodeReviewer/InternalReviewer 恒空（REQ-ENV-03））；target/git identity 仍钉 attempt snapshot。单仓/legacy 早门前返回 `Ok(None)`，零变化 |
| `group_review_orchestrator.rs` `RealGroupReviewExecutor::execute` | policy 在场时 input 显式回填 envelope 冻结 cwd（`provider_input.working_directory=Some(envelope cwd)`），再捆绑 `ValidatedStreamingProviderInput` | 与 2.6 coder/reviewer cycle 同款 launch 层重绑（设计决策 1）；spawn 前 `revalidate_before_spawn` 消费 Task 2.5 冻结字段。无政策路径保持 None（回填 working_dir，单仓零变化） |
| `internal_pr_review.rs` `execute_internal_pr_review_with_commands` | 同款 envelope cwd 回填 | 同上；CodeReviewer cycle（provider_retry.rs，2.6 预埋）经 lifecycle envelope 切换自动跟随，无需触碰 2.6 独占文件 |
| 两工厂 doc | 补一行「LC root cwd 由 execute 侧 envelope 重绑注入（Task 2.7），工厂本身保持 D2 锚点」 | 工厂签名/工具策略零变化（Reviewer → DenyFileWriteBuiltins 不放宽） |

## TDD（brief 测试名逐字）

1. `group_review_and_internal_reviewer_separate_root_cwd_from_attempt_target`：先红（spawn cwd=worktree ≠ canonical root，left=/tmp/…/worktree）→ 绿。双面驱动生产入口：group review=`RealGroupReviewExecutor::execute`（经 `group_review_streaming_input` 工厂），InternalReviewer=`execute_internal_pr_review`（经 `internal_pr_review_streaming_input` 工厂）；断言各恰 1 次 spawn、spawn 时点 effective cwd==canonical root、≠attempt worktree（分离）；resolver 层 envelope cwd==root 且 target.worktree/logical_repository_id/checkout_id 仍==snapshot 身份（target/git identity 由 resolver 校验，cwd 分离不放松 target 维度）。
2. `logical_reviewer_keeps_empty_writable_roots_and_d4_baseline`：先红（envelope cwd=worktree ≠ root）→ 绿。断言 writable_roots 恒空（不随 cwd 扩大）、readable_roots=[root]、D4 baseline 先于 reviewer spawn 落盘（spawn 时点 baselines 目录恰 1 文件）、会话期间写非 target 成员主 checkout 后 `execute_review_request` 以 `cross_target_violation_detected` 阻断交付。

新断言只观察 root cwd + spawn 计数 + 「cwd 分离而写根不扩」组合，未复述 provider_gateway_validated_input.rs:438-543 的 None/target/write-root 断言（G-28 口径）。

## 验证（实际执行）

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib group_review_and_internal_reviewer_separate_root_cwd_from_attempt_target` | 先红（行为红：cwd 探针 left=worktree ≠ right=root）→ 后绿 |
| `cargo test --locked --lib logical_reviewer_keeps_empty_writable_roots_and_d4_baseline` | 先红（envelope cwd 断言）→ 后绿 |
| `cargo test --locked --lib group_review_identity_snapshot` | **7/7 PASS**（5 既有 + 2 新增） |
| `cargo test --locked --lib provider_gateway_validated_input` | **12/12 PASS**（含 2.6 两个 root-cwd 测试与 D4 双测试） |
| `cargo test --locked --lib` | **3973 passed / 0 failed**（3 ignored；基线 3971+2 新增，零回归） |
| `cargo test --locked --test it_core large_file_guard` | **PASS**（group_review_orchestrator.rs 1193、provider_gateway_validated_input.rs 1191、internal_pr_review.rs 906、lifecycle.rs 567、group_review_identity_snapshot.rs 976，均 <1200） |
| `cargo test --locked --test it_web provider_gateway_envelope` | **6/6 PASS**（web 层 LC internal review 经 gateway 启动 + envelope 断言在 root cwd 下保持绿） |

## 偏差说明

1. `src/product/coding_workspace_engine/tests/provider_gateway_validated_input.rs` 不在 brief Files 清单，但 `drive_logical_coding_run`（D4 seam harness）手动捆绑 policy 时未做生产侧的 envelope cwd 回填——lifecycle envelope 切 root 后 spawn 前 cwd 复验正确地 fail-closed（`provider_gateway_target_mismatch: cwd`），两例 D4 测试先红。按 2.8 的消费面 fixture re-pin 先例，harness 对齐生产接线（`run_coder_with_retry_cycle` 的 envelope cwd 回填，+5 行），随本任务一并提交（编译/测试必需）。D4 断言本身（baseline 内容/漂移阻断）未改语义。
2. CodeReviewer 角色随 lifecycle envelope 切换自动跟随 root cwd（2.6 预埋回填点消费）——契约「reviewer input 自动跟随」即含此面，无额外改动。

## Commits（显式文件清单）

1. `ca8dc509` `feat: rebind logical reviewer cwd to lc root`：`src/product/coding_workspace_engine/group_review_orchestrator.rs`、`src/product/coding_workspace_engine/internal_pr_review.rs`、`src/product/coding_workspace_engine/lifecycle.rs`、`src/product/coding_workspace_engine/tests/group_review_identity_snapshot.rs`、`src/product/coding_workspace_engine/tests/provider_gateway_validated_input.rs`（偏差 #1）、本报告
