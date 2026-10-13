# N7 报告：新 plan-run 删除闭包接线（REQ-CS-07/02）

- Change：`openspec/changes/fix-multi-repo-group-coding-start/`（OpenSpec Task 3.2，REQ-CS-07/02）
- Plan：`cadence/plans/2026-10-13_实施计划_多仓分组coding启动_v1.0.md` §8（含修订：锁 `.{plan_id}.json.lock` 前导点；修正后绿窗四命令、0 命中不计绿窗）
- Worktree：`.worktrees/feat-b-0808-add-monorepo`（分支 feat-b-0808-add-monorepo）
- 状态：DONE（红→绿→回归全绿；独立 commit 见下）

## 交付（两处窄改 + 两测试文件）

- **Modify `src/web/handlers/support_parts/coding_cleanup.inc.rs`**：
  - 新增 `cleanup_plan_run_shared_worktrees(app_paths, run) -> ApiResult<()>`（REQ-CS-07
    target-scoped 包装，D2.2 后行统一消费）：按 run 冻结 `target_attempts` 逐仓清理
    repo shared worktree——每仓先取 `try_acquire_shared_resource_cleanup` 短 lease
    （首个消费者；Busy fail-closed 上抛，绝不静默跳过），锁内复核记录真实 owner
    （(project, issue, target) 身份全等，漂移 IdentityMismatch 上抛；活动锁
    owner 属他人=不属本 run 冻结 attempt 集/selection 时保留不删），再
    `delete_repo_shared_worktree`。缺失 None=OK；IO/JSON 上抛不吞；**不走旧
    issue-wide helper**（`cleanup_shared_worktree_by_routing` 其它 caller 语义零变化）。
  - 新增 `shared_resource_cleanup_api_error`：Busy→`plan_coding_run_resource_busy`、
    Deleting/Deleted→`plan_coding_run_resource_unavailable`、Store→既有映射。
  - `finalize_coding_attempt_deletion` 在 `delete_attempt` 后接
    `invalidate_for_deleted_attempt`（N2 API 只读消费）：冻结集合含该 attempt
    （或 selection 指向它）→ run Invalidated；无匹配 run OK 不补建；单 scope
    attempt 零动作。
- **Modify `src/web/handlers/lifecycle/deletion.rs`**：legacy 与 schema-v2 两条
  plan 清理闭包统一接线——门禁放行后、闭包前 `CodingPlanRunStore::get` 捕获 run
  （冻结 target 集合；**损坏 run JSON 在任何变更前阻断删除且零落盘**）；有 run 时
  资源清理走上述 target-scoped 包装（schema-v2 步骤 4 由包装替换 issue-wide 枚举；
  无 run 保持原 `cleanup_shared_worktree_by_routing` 路径不动）；闭包尾部
  `purge_plan`（缺失幂等 OK；早拒/门禁路径不 purge、不动 run）。
- **Test `lifecycle_tests.inc.rs`**（+夹具：最小 project/issue/plan 种子、group 绑定
  attempt、冻结地址 run 直写、仓维 worktree 他人持锁改写、DELETE 路由请求器）：
  - `plan_coding_n7_workspace_gate_zero_mutation`：有 coding workspace 拒删
    （409 `coding_workspace_exists`）时 run 字节级零改动、plan/attempt 保留
    ——门禁基线保护（既有能力，非伪造红）。
  - `plan_coding_n7_plan_purge_removes_run_and_preserves_other_owner`：schema-v2
    闭包，先删本 workspace（member 失效）→ 持 lease `EntityDeletionStore::begin`
    立墓碑（真实删除闭包形态）→ DELETE plan：本 plan run 文件+前导点锁清
    （墓碑在位的删除窗口）；他 plan run/记录、非冻结集 target、冻结集内他人
    持锁 owner 全保留。
  - `plan_coding_n7_missing_resource_reentry_and_late_start`：无 run 删除 OK+
    重入不复活；损坏 run JSON 删除失败上抛且 plan 记录保持（IO/JSON 不吞）；
    legacy 删除后旧 command 迟到 `mark_start_requested` 拒绝且不复活 run 文件，
    无墓碑时 `purge_plan` 只清 run 文件保留锁（N2 修复轮语义边界断言）。
- **Test `support_task11_tests.rs`**：`plan_coding_n7_member_delete_invalidates_authorization`
  ——member attempt 经 `finalize_coding_attempt_deletion` 删除后 run Invalidated
  （冻结集合/selection 保留不抹除）；旧 command 的迟到首启标记
  `coding_plan_run_invalidated` Conflict 拒绝、不翻回可启相位。

## 证据（cargo --locked，红→绿→回归）

红窗方法：实现与测试文件天然分离（实现=coding_cleanup.inc.rs+deletion.rs；
测试=两个测试文件），红窗以 `git diff` 补丁 `git apply -R` 反转实现后运行，
再 `git apply` 恢复（等效 stash 语义，未用 stash；期间 N2 修复轮 77ab13d7 落地，
红判据不受影响——失败均为行为缺失断言而非编译）。

| 命令 | 红（实现反转） | 绿 |
|---|---|---|
| `--lib plan_coding_n7_` | 3 failed / 1 passed（基线门禁绿）：member 删后 run 未失效；purge 缺失＋损坏 run JSON 被吞返回 `{"status":"deleted"}`；issue-wide 扫掉他 target/run 残留可重放 | 4 passed |
| `delete_work_item_plan`（无 --lib，it_web 8 实测命中） | — | 8 passed |
| `--lib finalize_coding_attempt_deletion` | — | 3 passed |
| `--lib cleanup_shared_worktree_by_routing` | — | 2 passed |
| 回归 `--lib plan_coding_n2_` / `plan_coding_n1_` | — | 8 / 3 passed |
| `cargo check --locked`（lib） | — | 通过 |

rustfmt（仅本包四文件，edition 2024）后复验 `--lib plan_coding_n7_` 仍 4 passed。

## 边界与移交

- run 磁盘地址/锁名（`{issue_root}/coding-plan-runs/{plan_id}.json` +
  `.{plan_id}.json.lock`）已在 N2 报告登记；本包未改 app_paths.rs、未增 manifest
  字段、未建第二闭包扫描器；N2 run store 仅消费（get/invalidate/purge），零修改。
- **N2 修复轮（77ab13d7）适配**：①墓碑在位（真实删除闭包）才清前导点锁——
  测试 3 以 `try_acquire_delete`+`begin` 立墓碑形态断言锁清；无墓碑只清 run
  文件保留锁（测试 4C 断言）；②run 写准入含墓碑拒——测试顺序为先 invalidate
  再立墓碑；③未断言暂停相位行为（group.rs 新语义不涉及本包）。
- **D2.2 后行消费点**：plan 级资源清理统一走 `cleanup_plan_run_shared_worktrees`
  （不重复实现）；持 Plan lease 删 run 用 `purge_plan_with_lease`（租约内恒清锁）；
  两条 plan 清理闭包尾部 `purge_plan` 调用点即同 owner 接口。
- `EntityOperationError::Busy` 映射 `plan_coding_run_resource_busy` 落默认 500
  （fail-closed 可重试）：error.rs 状态映射不在本包独占清单，未扩；如需 409
  稳定码由 D2.2/终审统一登记。
- 共享树红绿窗受 N3（plan_start.rs）/N5（runner.rs）在途编译扰动，均已恢复后
  取证；绿窗后兄弟文件仍有未提交改动，本包 commit 仅列本包文件。
- S/P 包（跨 attempt resume、plan 终结、并行）不在本片。
