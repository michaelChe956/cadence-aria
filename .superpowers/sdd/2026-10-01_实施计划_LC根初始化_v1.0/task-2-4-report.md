# Task 2.4 报告：ReviewOnly 与 plan review builders root cwd（映射 tasks.md 2.3 review-builder 面）

状态：DONE　|　worktree: feat-b-0808-add-monorepo　|　基线 HEAD: 3bc5ae9e

## 交付合同

| 面 | 变更 | 语义 |
|---|---|---|
| `prompts/review_context.rs` | 新增 `WorkspaceEngine::review_launch_target_and_cwd()`（33 行，返回 `(target_worktree, session_cwd)`）：LC 分支（注入 gateway）cwd=`gateway.authority_root()`（2.3 新增的 canonical authority root，经 Task 2.5 `working_directory` 字段合同下发；不从 target/worktree/进程 cwd 推导）、target=显式 `session.repository_path`（缺失 fail-closed，绝不回退进程 cwd）；单仓分支（无 gateway）原值不变（`working_dir`=repository_path、缺省进程 cwd回退，`working_directory=None`） | 全部 review builder 的唯一 cwd/target 解析点，不立第二套约定 |
| `prompts/review.rs` ×5 构造点 | `build_review_input` 泛型（ReviewOnly）+ `build_work_item_plan_review_input` legacy 分支 + `build_projection_plan_review_input`/`build_work_item_plan_outline_review_input`/`build_work_item_batch_review_input`：各自内联的 `match repository_path / current_dir` 推导替换为共享 helper，input 携带 `working_directory`（LC=Some(root)） | LC：cwd=root、target=成员 checkout 显式独立；单仓：字节级原值 |
| `review_parts/single_candidate_input.inc.rs`、`review_parts/draft_input.inc.rs` | SC/draft builder 同一替换（B5 拆分部件，brief 预告可触碰） | delegated builders 全覆盖 |
| 与 drive 层对接 | `start_review_session_via_gateway`（2.3 已完成，零改动）以 `input.working_dir` 锚 `PolicyTarget::aggregate_root`（target 现为显式成员路径）并在 validated 后以 envelope cwd 回填——builder 层 Some(root) 与 envelope 冻结值同源一致；legacy 直连 `provider.start` 在 LC 下经 `effective_working_directory`（优先新字段）也恒 root（纵深防御） | ReviewOnly 不变 Executor：`DenyFileWriteBuiltins` 七处全部保持，readable roots/writable roots 面归 drive 层（2.3 已定，未动） |

## 新测试（brief 逐字命名，位于 `tests/design_reviewer_boundary.rs`）

- `all_logical_plan_review_builders_keep_reviewer_write_guard` ✅ —— 一测七面：legacy 整组候选 / outline / draft / batch / single-candidate / projection 六个 plan review builder + ReviewOnly 泛型（Design）分支，LC 会话下断言 `working_directory==Some(canonical root)`（非成员 worktree、非进程 cwd）、`working_dir==显式成员 checkout`（target 不从 cwd 推导）、`role==Reviewer`、`tool_policy==deny_file_write_builtins`；伴随断言：LC 缺 repository_path → fail-closed（错误含 `explicit member target`，无进程 cwd 回退）；单仓零变化（无 gateway：`working_directory==None`，repository_path 缺省回退进程 cwd / 显式时 cwd=target 原值）

红→绿：先红=legacy 分支 `working_directory None vs Some(root)`（builder 现以 `session.repository_path`/进程 cwd 作 cwd——与 brief 红因一致）→ 实现后绿。

## 验证（实际执行）

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib all_logical_plan_review_builders_keep_reviewer_write_guard` | 先红（cwd 断言）→ 1 passed |
| `cargo test --locked --lib design_reviewer_boundary`（brief Step 4） | 8 passed（新测试因扁平 include 不含文件名，单独过滤见上行） |
| `cargo test --locked --lib tests_policy_routing`（brief Step 4 字面命令） | 0 matched（tests_policy_routing.rs 为扁平 include，文件名不进测试名；实际用例由下行名过滤覆盖） |
| 名过滤 `_routes_are_characterized/policy_route/human_gate_snapshot/structured_*/unknown_category_diagnostic/legacy_batch_pass_routes/auto_if_valid_batch/work_item_policy_counts/build_work_item/build_review_input/work_item_plan_reviewer_prompt/work_item_plan_initial_compile/builder_factory_applies_role_policy_matrix/workspace_builder_family_pairs_role_with_tool_policy/prompt_sliding_window` | 55 passed（tests_policy_routing 13 例 + part_08/20 builder + part_03/part_10 projection + part_32 builder 策略族） |
| `cargo test --locked --lib single_candidate_prompt::review_prompt` 等价过滤（`review_prompt`） | SC builder 3 例绿（含 reads_compiled_ir_and_mechanical_report） |
| `cargo test --locked --lib` | 3968 passed / 0 failed（3 ignored；基线 3967+1 新增） |
| `cargo test --locked --test it_core large_file_guard` | 1 passed（review.rs 818 / review_context.rs 635 / boundary 635 / parts 140+223，均 <1200） |
| rustfmt（改动文件） | 我的 3 处 hunk 已修；其余 9 处为 HEAD 既有漂移（`git show HEAD | rustfmt --check` 同为 9，未触碰） |

## 范围边界与 carry

- `review/drive.rs` 零改动（2.3 独占面；builder 与 drive 的 cwd 同源自 `authority_root()`，无冲突）。
- `build_review_repair_input`（review_repair.rs）保持 `working_directory: None`——repair input 由 drive 层 gateway 启动点统一回填 envelope cwd（via_gateway 三启动点均经 `start_review_session_via_gateway`），不在本任务文件面。
- gateway 夹具为 builder 面最小形态（不做登记/selection/bootstrap 链）——validate 面已由 2.3 part_32/root_cwd_revision 契约测试覆盖，不重复。
- 追加文件（超出 brief Files 清单，均为 brief 预告的 B5 拆分部件）：`review_parts/single_candidate_input.inc.rs`、`review_parts/draft_input.inc.rs`。
