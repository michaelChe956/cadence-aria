# Task 2.6 报告：Coder/retry root input 与 launch rebind + split sync root cwd

状态：DONE　|　worktree: feat-b-0808-add-monorepo　|　基线 HEAD: 954c43f7

## 交付（两块）

### A. LC Coder/retry root cwd（brief 主体，映射 tasks.md 2.4 的 Coder/retry 行）

- `resolve_coder_root_launch_policy`（新，`provider_retry_parts/coder_root_launch.inc.rs`，include! 挂载）：cwd=**gateway 冻结的 canonical authority root**（manifest `provider_context_root`，与 2.1 author/2.3 revision launch 同源取值）；target=`PolicyTarget::checkout(snapshot logical_repository_id/checkout_id, attempt worktree)`；`writable_roots=[worktree]`（REQ-ENV-03/11：不随 cwd 扩大为聚合根）；`readable_roots=[root]`（随 cwd root 化，2.1/2.3 同口径）。分流语义与 `resolve_launch_policy_for_role` 一致：无 gateway / 无 `target_snapshot` → `Ok(None)`；逻辑 attempt 校验失败错误透传，绝不回落 member cwd 直连。
- `run_coder_with_retry_cycle`：`target_snapshot.is_some()` → root launch；否则保持既有 `resolve_launch_policy_for_role` 分流（单仓 None → 直连，零行为变化）。policy Some 时 input 显式回填 envelope 冻结 cwd（设计决策 1「launch 层按 validated envelope 重绑 cwd」；Task 2.5 冻结字段由 `revalidate_before_spawn` 在 spawn 前消费）。
- `run_code_reviewer_with_retry_cycle`：对称加 envelope cwd 重绑——现状 envelope cwd==worktree ⇒ effective 零行为变化；2.7 把 reviewer envelope 切到 root 后自动跟随（provider_retry.rs 为 2.6 独占文件，2.7 无法改，此处预防 spawn 前 cwd 复验因字段分离误伤）。
- `launch_provider_session`（launch.rs）：分流机制不变，仅文档化 Task 2.6 契约（root cwd 经 validated input 流入，canonical/authority 复验留在 gateway，不重建机制）。`provider_stream.rs` 零改动（brief 列名但无功能需求点）。
- Legacy Coder Executor 直连、None fallback、Executor 无 tool policy（D2）均不变。

### B. split sync 引擎 root cwd（controller 契约约束 #1：work_item_split_engine/engine.rs）

- `invoke_provider_via_gateway`：`AdapterInput.working_directory=Some(gateway.authority_root())`（LC 分支 cwd=canonical root），`worktree_path` 仍是 target 成员路径（2.5 字段合同两字段分离）。
- `prepare_sync_launch`：`SessionLaunchRequest` 显式构造（弃 `planning()` 构造器的 cwd==target 默认派生）——`working_directory=root`、`readable_roots=[root]`、`writable_roots=[]`（planning 只读）；envelope 冻结后由 `run_sync` spawn 前复验消费。
- 单仓 `invoke_provider` 直连回填（None → worktree_path）不变。

## TDD（brief 测试名逐字）

红→绿实证：
1. `logical_coder_rebinds_root_cwd_without_expanding_writable_root`：先红（E0599 方法缺失；行为红=cwd 探针断言 left=worktree ≠ right=root）→ 绿。断言=生产入口 `run_coder_with_retry_cycle` 全链驱动：spawn 恰 1 次、spawn 时点 effective cwd==canonical root、envelope cwd==root 且 `writable_roots==[worktree]`（不扩大）。
2. `logical_coder_without_validated_gateway_zero_spawns`：回归锁（改动前即绿——Task 12 fail-closed 门在 cycle 内保持关闭）：零 spawn + `logical_provider_gateway_required` 稳定码。
3. `split_sync_gateway_launch_rebinds_cwd_to_canonical_root`（engine_gateway_guard.rs）：先红（sync 探针 cwd=member ≠ root）→ 绿：恰 1 次 sync spawn、cwd==canonical root、worktree_path==成员路径。

未复述 provider_gateway_validated_input.rs:438-543 既有 None/target/write-root 断言（新断言只观察 cwd=root+spawn 计数+“cwd 分离而写根不扩”组合）。

## 验证（实际执行）

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib logical_coder_rebinds_root_cwd_without_expanding_writable_root` | 先红（E0599 指向新合同）→ 后绿 |
| `cargo test --locked --lib provider_gateway_validated_input` | **12/12 PASS**（文件全部：4 个 validated_streaming_input_for_role + internal review + targets/codex + routing + 2 D4 + 2 新增） |
| `cargo test --locked --lib work_item_split_engine::tests::` | **129/129 PASS**（含 split sync 新测试与既有 fail-closed/2.5 字段合同测试） |
| `cargo test --locked --lib -- logical_coding_never_falls_back... logical_provider_gateway_required_applies_to_fake_provider` | 2/2 PASS（Task 12 门回归） |
| `cargo test --locked --lib` | **3971 passed / 0 failed**（3 ignored；跑 3 轮，第 1 轮 1 例偶发失败、后 2 轮全绿复验） |
| `cargo test --locked --test it_core large_file_guard` | **PASS**（provider_retry.rs 拆分后 1183 行 < 1200，未新增超限文件） |

## 物理拆分（行数守卫）

provider_retry.rs 1230 行将破 large_file_guard 1200 红线（该守卫当前全绿），按仓库 `gates_parts/*.inc.rs` 惯例把 `resolve_coder_root_launch_policy` 拆到 `provider_retry_parts/coder_root_launch.inc.rs`（55 行，include! 同模块域，零语义变化）。

## 偏差说明

1. brief Step 5 的 `git add` 清单未含拆分新文件 `provider_retry_parts/coder_root_launch.inc.rs`（brief 撰写时不存在）——随 coder 提交一并纳入（编译必需）。`provider_stream.rs` 无 diff，add 为 no-op。
2. controller 契约约束 #1（engine.rs split sync）不在 brief Files 清单内，但为主 agent 显式逐字契约且 engine.rs 不属 2.7/2.8 独占——独立提交交付（`feat: rebind work item split sync cwd to lc root`）。
3. 2.7 预留：reviewer cycle 的 envelope cwd 重绑使 2.7 只需改 lifecycle.rs 的 envelope 构造（cwd→root），provider 侧自动跟随；无需碰 2.6 独占文件。

## Commits（显式文件清单）

1. `feat: rebind logical coder cwd to lc root`：`src/product/coding_workspace_engine/provider_retry.rs`、`src/product/coding_workspace_engine/provider_retry_parts/coder_root_launch.inc.rs`、`src/product/coding_workspace_engine/provider_stream/launch.rs`、`src/product/coding_workspace_engine/tests/provider_gateway_validated_input.rs`
2. `feat: rebind work item split sync cwd to lc root`：`src/product/work_item_split_engine/engine.rs`、`src/product/work_item_split_engine/tests/engine_gateway_guard.rs`、本报告
