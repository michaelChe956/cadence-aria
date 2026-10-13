# N4 报告：真实 Git / Logical envelope 接线回归证据（消费现能力，不重复实现）

- Change：`openspec/changes/fix-multi-repo-group-coding-start/`（Task 2.2，REQ-CS-04/08）
- Plan：`cadence/plans/2026-10-13_实施计划_多仓分组coding启动_v1.0.md` §5
- Worktree：`.worktrees/feat-b-0808-add-monorepo`（分支 feat-b-0808-add-monorepo）
- 状态：DONE（**新回归直接绿——如实标记为基线/回归证据**，非伪造红；生产代码零修改，
  `lifecycle.rs` 条件写面未触发——未发现接线断点）

## 结论

`start_attempt` 缺目录回落 WorktreePrepare、`execute_worktree_prepare` 真实物化、
`prepare_resumed_attempt_for_runner` 半启动 demote、
`coding_session_launch_request_for_role` 的 root cwd/写根 envelope 均已在生产路径
正确接线。本包只补三个多仓/边界维度的回归证据，未重复实现任何能力。

## 新增测试（3 个，全部命中 `plan_coding_n4_`）

1. `plan_coding_n4_ready_target_real_git_materialization`
   （`src/product/workspace_engine/tests/advance_split_recovery_matrix.rs`，复用
   `split_advance_fixture` 真实双 Git 仓 + 既有 `sc_advance_unmaterialized_worktree_
   falls_back_to_worktree_prepare` 语义的多仓版）
   - split 建组后 Ready 首项 worktree 仅元数据（目录不存在）；repo 维 shared worktree
     三件套（target/worktree_path/branch/base_branch）与冻结 attempt 全等；
     `current_lock_owner_id`==冻结 attempt id，`validate_repo_worktree_lock_owner`
     以 `current_work_item_id`+attempt id 通过（owner/lease 冻结匹配，REQ-CS-04）。
   - `start_attempt`：未物化 Ready 路径 → Running+WorktreePrepare 回落，不物化目录。
   - `execute_worktree_prepare`（冻结 physical repo）：真实派生 branch+worktree——
     `git worktree list --porcelain` 含 `worktree <path>`+`branch refs/heads/<冻结branch>`
     条目；`git rev-parse --verify refs/heads/<branch>` 成功。
   - 物化后再入 `start_attempt`：group 短路进 Coding，head_commit==worktree 真实 HEAD。
2. `plan_coding_n4_logical_launch_keeps_root_cwd_and_only_target_writable`
   （`src/product/coding_workspace_engine/tests/provider_start_persistence.rs`）
   - Coder：envelope `working_directory == authority_root`（REQ-ENV-10 root cwd）、
     `readable_roots == [authority_root]`、`writable_roots == [所选派生 worktree]` 唯一
     （REQ-ENV-11），root 不在可写根；`PolicyTarget` worktree=派生 worktree 且
     logical_repository_id/checkout_id 与冻结 `target_snapshot` 全等。
   - CodeReviewer/InternalReviewer：`ReviewReadOnly`、可写根恒空、cwd 仍为 root、
     root 不可写。
   - LC root cwd ≠ target worktree（不误判串仓）。
3. `plan_coding_n4_resume_demotes_only_unmaterialized_selected_target`
   （`advance_split_recovery_matrix.rs`，扩展既有
   `split_semi_started_unmaterialized_demotes_per_attempt_only` 的副作用维度）
   - selected 未物化半启动 → demote WorktreePrepare（Running 保持）；未 selected
     attempt resume 恒等（Created/PrepareContext）。
   - **未 selected 零真实 Git 副作用**：两 physical repo 的
     `git worktree list --porcelain` 无 aria-issues 条目、`git branch --list aria/issues/*`
     为空。
   - 资源占用按真实 lease 展示：未 selected 的 repo 维锁 owner 仍冻结为其 attempt id
     （split 登记意图），非 provider 已运行。
   - 零 provider 副作用：两 attempt `list_role_runs` 均空。

## 红绿证据（如实）

- 首跑（基线窗）：`cargo test --locked --lib plan_coding_n4_` →
  **3 passed; 0 failed**（现能力已满足，按 Plan §5/§9.1 允许直接绿，标记为基线/回归，
  未制造改动或假红）。过程中唯一修正是测试自身编译错误
  （`validate_repo_worktree_lock_owner` 返回 `Result<IssueSharedWorktree>`，去掉双重解包），
  非生产代码变化。
- 锚定回归：`sc_advance_unmaterialized_worktree_falls_back_to_worktree_prepare` →
  1 passed；`split_semi_started_unmaterialized_demotes_per_attempt_only` → 1 passed。
- 模块面：`advance_split_recovery_matrix` 10 passed；`provider_start_persistence`
  14 passed；`advance_ready_only` 9 passed（零修改，锚能力完好）。

## 写面与边界

- 修改文件：仅上列两个测试文件；`advance_ready_only.rs` 零修改（单靶回落/demote 断言
  已完整，多仓版不重复）；`lifecycle.rs` 条件独占写面未触发（envelope/root cwd/写根
  契约现实现即 REQ-CS-04 要求，无接口缺口）。
- 真实 provider 进入验收（REQ-CS-04 Scenario 2 的「不把首启回执当实跑证据」终验）归
  controller/Main 统一真人验收，本包不声称已通过。
- 提交：`fix: N4 已有能力接线回归证据（多仓真实Git物化/root cwd envelope/恢复demote零副作用）`
  （仅含两测试文件与本报告）。
