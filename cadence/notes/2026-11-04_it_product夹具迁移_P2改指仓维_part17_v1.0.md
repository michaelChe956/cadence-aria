# it_product 夹具迁移 · P2 改指仓维（part_17）v1.0

- 日期：2026-11-04
- 工作树：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- 唯一改动文件：`tests/it_product/product_coding_workspace_engine/part_17.rs`（177 行 → 461 行）
- 改动性质：**只改断言的读取维度**（issue 维 → 仓维）与**冲突 kind**
  （`issue_worktree_lock_owner` → `repo_worktree_lock_owner`）。断言语义强度未削减、
  用例未删、未加 `ignore`、未改 spec / delta / tasks.md、未改 `src/**`、`web/**`、
  `tests/it_product/context_builder.rs`、其他 `tests/it_product/**` 目录、`openspec/**`。

---

## 1. 背景：为什么这 4 条必须改维度而不是补权威

块3 删掉了 `IssueSharedWorktreeRoute::Legacy` 臂，引擎的 shared worktree 事实
**只剩仓维**（三元键 `(project, issue, logical repository)`）：

- `route_issue_shared_worktree`（`src/product/coding_workspace_engine/gates.rs:125`）
  - 有 `attempt.target_snapshot` → `IssueSharedWorktreeRoute::Repository { repository_id }`
    → 引擎读 `shared-worktrees/{repository_id}.json`；
  - 无快照 → `RepositoryRouting::load_for_issue` 命中 `FailClosed` → fail-closed
    `target_snapshot_identity_drifted`（**注意**：旧现场里 issue 记录都没落盘，
    实际报的是 `Store(NotFound { kind: "issue", id: "issue_0001" })`）。
- 同 issue 下若残留 legacy `issue-shared-worktree.json`，多仓路径先撞迁移 preflight
  → `LegacySharedWorktreePresent("{project}/{issue}")`。
- 仓维锁归属校验 `validate_repo_worktree_lock_owner`（`src/product/lifecycle_store/
  worktree/repo_locks.rs:283`）在 owner 不匹配时返回
  `ProductStoreError::Conflict { kind: "repo_worktree_lock_owner", id: "<repo>/<wi>" }`。

因此：

| 旧形态 | 引擎实际行为 | 旧断言读到的 |
|---|---|---|
| issue 维 `upsert_issue_shared_worktree` + `try_acquire_issue_worktree_lock` | 引擎根本不读 issue 维记录 | 冲突从未发生（断言恒不成立） |
| 无 LC 权威（无 `target_snapshot`） | 路由 fail-closed | 报错但原因是**归属漂移**，不是锁归属冲突 |

⇒ P2-1 / P2-2 属于前者（断言读错了维度）；P2-3 / P2-4 属于后者
（断言「绿在错误的原因上」：`expect_err` + 「状态未变」恰好被 fail-closed 满足）。
本组四条**统一**：补 LC 权威 + 冻结 `target_snapshot` ⇒ 引擎走仓维 ⇒ 把断言钉到
**真正的仓维冲突**（`Conflict{kind:"repo_worktree_lock_owner"}`），并追加「冲突后
仓维锁仍由他人持有」的断言（谁持锁语义由 issue 维换到仓维继续被断言住）。

## 2. 逐条维度对照

| # | 用例 | 原来读什么维度 | 改读什么维度 | 断言语义保持不变的依据 |
|---|---|---|---|---|
| P2-1 | `group_unit_owner_conflict_does_not_advance_unit_or_attempt` | issue 维：`upsert_issue_shared_worktree` + `try_acquire_issue_worktree_lock`；断言 `Conflict{kind:"issue_worktree_lock_owner"}` | 仓维：`upsert_repo_shared_worktree` + `try_acquire_repo_worktree_lock`；断言 `Conflict{kind:"repo_worktree_lock_owner"}` | 冲突前置于状态变更（`complete_current_group_unit` 首句即 `validate_attempt_issue_shared_worktree_lock_if_present`）；unit 状态（Running/Pending）、attempt 指针（`current_work_item_id`/`active_unit_id`）全部保留原断言并新增 attempt `status == Created` 与「锁仍属 `coding_attempt_other`、active WI 仍 `work_item_0001`」 |
| P2-2 | `final_confirm_owner_conflict_does_not_complete_attempt` | issue 维（同上） | 仓维（同上） | 冲突仍在写终态前（`handle_final_confirm` 的 `validate_attempt_issue_shared_worktree_lock_if_present` 在 `update_attempt_status(Completed)` 之前）；保留 `status == WaitingForHuman` 断言并新增「锁仍属 `coding_attempt_other`」 |
| P2-3 | `abort_owner_conflict_does_not_abort_attempt` | issue 维（同上）；断言只要求「报错 + 状态未变」 | 仓维（同上）；断言**收紧**为 `Conflict{kind:"repo_worktree_lock_owner"}` + 保留 `status == Created` + 新增「锁仍属 `coding_attempt_other`、active WI 仍 `work_item_0001`」 | 「冲突时不得中止」语义由错误 kind 精确化（原断言被 fail-closed 顺带满足，现钉到真冲突）；状态断言原样保留 |
| P2-4 | `failure_owner_conflict_does_not_fail_attempt` | issue 维（同上）；断言只要求「报错 + 状态未变」 | 仓维（同上）；断言**收紧**为 `Conflict{kind:"repo_worktree_lock_owner"}` + 保留 `status == Created` + 新增「锁仍属 `coding_attempt_other`、active WI 仍 `work_item_0001`」 | 「冲突时不得失败收尾」语义同上 |

### 冲突现场（4 条统一）

- LC 权威：`crate::context_builder::seed_logical_authority`（`project_0001` + 真多仓 LC
  + 归属 issue + 成员/checkout/selection/policy/聚合索引），返回 `LogicalAuthority`。
- 冻结快照：`authority.snapshot(&paths)` → 复用生产构造器
  `build_attempt_target_snapshot`，故 `logical_repository_id` 与权威一致。
- 仓维锁：`part17_seed_repo_worktree_lock` → `upsert_repo_shared_worktree`
  （键 `shared-worktrees/{member_id}.json`）+ `try_acquire_repo_worktree_lock`
  （`work_item_id="work_item_0001"`、`lease_id="coding_attempt_other"`）。

### 形态纪律（沿用 Phase 2 已踩坑结论）

- **LC 权威必须先于 coding attempt 建立**：`ProjectStore::create` 的 project id 按
  `projects/` 目录数分配，而 attempt 落盘本身就会建
  `projects/project_0001/issues/issue_0001/` ⇒ 先建 attempt 再建权威会把 project
  分配成 `project_0002`，权威与 attempt 的 project 身份错位 ⇒ 路由恒 fail-closed。
  本组 3 个夹具全部「权威 → attempt」顺序。
- 成员 checkout 必须是聚合根的直接子目录且是真实 git 仓（由 `context_builder` 保证）。
- issue 记录按固定 id 直接落盘（由 `context_builder` 保证）。
- 仓维 shared worktree 记录缺失 ⇒ group 推进会 `NotFound{repo_shared_worktree}`；
  本组 4 条都显式建了仓维记录。

## 3. 新增（本组私有）夹具

均在 `part_17.rs` 内，`part17_` 前缀避免与其它 part 的同名符号冲突，且**不依赖
正在被并发修改的 part_13**：

| 夹具 | 作用 |
|---|---|
| `part17_group_engine_with_two_units_under_lc_authority` | group attempt（双 unit：Running/Pending）+ LC 权威 + 冻结快照 |
| `part17_final_confirm_attempt_under_lc_authority` | WorkItem 范围 attempt + 冻结快照，播种到 `FinalConfirm` / `WaitingForHuman` |
| `part17_coding_store_with_attempt_under_lc_authority` | WorkItem 范围 attempt + 冻结快照（`Created` 现场） |
| `part17_seed_repo_worktree_lock` | 建仓维 shared worktree 记录并把锁交给指定 owner |
| `part17_read_repo_worktree_lock` | 读回仓维锁现场（断言「谁持锁」） |

复用（未改动）的既有夹具：`part_01::seed_lc_authority`、
`part_08::seed_group_work_items_and_plan`、`part_12::init_group_worktree`。

## 4. 测试数字

全部在 `/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`，
窄跑一律 `flock /tmp/aria-cargo.lock`。

### 4.1 本组 4 条（逐条点名）

```
flock /tmp/aria-cargo.lock cargo test --locked --test it_product -- \
  product_coding_workspace_engine::group_unit_owner_conflict_does_not_advance_unit_or_attempt \
  product_coding_workspace_engine::final_confirm_owner_conflict_does_not_complete_attempt \
  product_coding_workspace_engine::abort_owner_conflict_does_not_abort_attempt \
  product_coding_workspace_engine::failure_owner_conflict_does_not_fail_attempt

running 4 tests
test ...::abort_owner_conflict_does_not_abort_attempt ... ok
test ...::failure_owner_conflict_does_not_fail_attempt ... ok
test ...::final_confirm_owner_conflict_does_not_complete_attempt ... ok
test ...::group_unit_owner_conflict_does_not_advance_unit_or_attempt ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 207 filtered out
```

⇒ **part_17 本组 4 条全绿，0 红。**

### 4.2 引擎模块测

```
flock /tmp/aria-cargo.lock cargo test --locked --test it_product -- product_coding_workspace_engine
=> test result: FAILED. 64 passed; 9 failed; 0 ignored; 0 measured; 138 filtered out
```

9 条失败**全部不在 part_17**（part_01 / part_04，他人归属，见 4.4）：

```
aborted_group_attempt_releases_transferred_shared_worktree_lock_by_owner
dirty_group_delete_keeps_transferred_shared_worktree_lock
dirty_shared_worktree_blocks_lock_release_and_next_work_item
execute_coding_persists_coder_role_run_and_full_output_chat_entry
failed_attempt_releases_issue_shared_worktree_lock
failed_group_attempt_releases_transferred_shared_worktree_lock_by_owner
final_confirm_releases_issue_shared_worktree_lock
group_final_confirm_releases_transferred_shared_worktree_lock_by_owner
group_next_work_item_coder_run_does_not_resume_previous_unit_session
```

### 4.3 全量

```
flock /tmp/aria-cargo.lock cargo test --locked --test it_product
=> test result: FAILED. 200 passed; 11 failed; 0 ignored; 0 measured; 0 filtered out
```

11 条失败 = 4.2 的 9 条 + 2 条 **part_13 在途**（Worker A 正在改、跑测时其文件处于
中间态；同一时刻之前的模块测里这两条是绿的）：

```
completing_group_unit_moves_issue_shared_lock_to_next_unit          (part_13)
group_final_confirm_completes_attempt_after_all_units_completed     (part_13)
```

⇒ part_17 贡献失败数 **0**。Main 给的开工基线为 202 passed / 9 failed；本次 200/11
的差额来自 part_13 的在途改动，非本组引入。

### 4.4 剩余失败用例的归属（供 Main 分派）

| 用例 | 文件 | 归属 |
|---|---|---|
| `failed_attempt_releases_issue_shared_worktree_lock` | part_01 | UnblockIntoPartsAndRepointP2 |
| `final_confirm_releases_issue_shared_worktree_lock` | part_01 | UnblockIntoPartsAndRepointP2 |
| `failed_group_attempt_releases_transferred_shared_worktree_lock_by_owner` | part_01 | UnblockIntoPartsAndRepointP2 |
| `group_final_confirm_releases_transferred_shared_worktree_lock_by_owner` | part_01 | UnblockIntoPartsAndRepointP2 |
| `aborted_group_attempt_releases_transferred_shared_worktree_lock_by_owner` | part_01 | UnblockIntoPartsAndRepointP2 |
| `dirty_group_delete_keeps_transferred_shared_worktree_lock` | part_01 | UnblockIntoPartsAndRepointP2 |
| `dirty_shared_worktree_blocks_lock_release_and_next_work_item` | part_01 | UnblockIntoPartsAndRepointP2 |
| `execute_coding_persists_coder_role_run_and_full_output_chat_entry` | part_04 | MigrateItProductP2A |
| `group_next_work_item_coder_run_does_not_resume_previous_unit_session` | part_01 | MigrateItProductFixtures（Fake → 真实 provider，Main 已授权） |

### 4.5 格式

```
flock /tmp/aria-cargo.lock cargo fmt --check
=> 无输出，exit 0（全树无 diff；part_17.rs 干净）
```

### 4.6 改动规模

| 文件 | 改前行数 | 改后行数 |
|---|---|---|
| `tests/it_product/product_coding_workspace_engine/part_17.rs` | 177 | 461 |

## 5. concerns

1. **本组 4 条的语义在仓维下可完整表达，无「无法表达」的用例** —— 4 条的原始不变量
   （他人持锁时 group 推进不得推进 unit/attempt、final confirm 不得完成 attempt、
   abort 不得中止 attempt、failure 不得失败收尾）在仓维全部有对应表达，且断言比
   改前**更强**（错误 kind 精确化 + 追加「锁仍属他人」）。
2. **`it_product` 编译曾被他人改动打断**（非本组改动，已解决）：`part_01.rs:914/1011`
   仍读 `provider.inputs`，而 `context_builder.rs` 的 `GatewayExactInputAdapter` 已改为
   只持 `inner: Arc<dyn StreamingProviderAdapter>` ⇒ `error[E0609]`。归属 helper 作者
   MigrateItProductFixtures，20:08 修好（构造时另留 `capturing` 句柄）。修好后本组
   的 4 条与全量均已实测。
3. **跨文件副作用（已解决）**：part_12 的 `group_engine_with_two_units` 曾因 part_13 与
   part_17 双双改走仓维而全仓零引用 ⇒ `dead_code` warning（CI
   `cargo clippy --all-targets -- -D warnings` 会阻断）。归属者 MigrateItProductP2B 已
   从 part_12 删除该函数并留去向注释，`cargo build --locked --tests` 零 warning。
4. **全量数字受他人在途改动影响**：本次 200 passed / 11 failed 中有 2 条是 part_13
   在跑测时刻处于中间态（Worker A 正在改），非本组引入；本组失败数 = 0。
5. **本组夹具自包含**：4 条用的 5 个 `part17_*` 夹具全部在 part_17 内，不依赖正在被
   并发修改的 part_13；仅复用 part_01 / part_08 / part_12 的三个通用夹具
   （`seed_lc_authority` / `seed_group_work_items_and_plan` / `init_group_worktree`），
   未修改它们。

## 6. 声明

- 未改任何断言的**语义强度**：4 条的错误 kind 从「issue 维 `issue_worktree_lock_owner`」
  精确化为「仓维 `repo_worktree_lock_owner`」，并追加「冲突后仓维锁仍由
  `coding_attempt_other` 持有、active WI 仍 `work_item_0001`」断言；P2-3 / P2-4 由
  「只要求报错 + 状态未变」**收紧**为「必须是仓维锁归属冲突」。无任何一条被放宽。
- 未删用例（4 条用例名一字未改）、未加 `#[ignore]`、未改 `src/**`、`web/**`、
  `tests/it_product/context_builder.rs`、其他 `tests/it_product/**` 目录、
  `openspec/**`（spec / delta / tasks.md 一律未动）。
- 未做任何 git 操作（无 add / commit / stash / checkout / reset）。
- 未改任何期望值的**语义**（谁持锁、锁何时释放、冲突时不得推进 / 完成 / 中止 /
  失败），只把「读哪个维度」与「冲突 kind」从 issue 维换到仓维。
