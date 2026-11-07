# it_product 夹具迁移 · P2 改指仓维 · part_13（v1.0）

- 日期：2026-11-04
- 执行者：RepointP2ToRepoDimPart13（Worker A）
- 工作树：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- 唯一被改文件：`tests/it_product/product_coding_workspace_engine/part_13.rs`

---

## 1. 背景与根因

块3 删掉 `IssueSharedWorktreeRoute::Legacy` 臂后，coding workspace engine 只读写**仓维**
shared worktree 记录（三元键 `(project, issue, logical repository)`，锁 kind 为
`repo_worktree_lock_owner`），**不再读写 issue 维记录**。

part_13 里 2 条 P2 用例的断言读的是 **issue 维** `get_issue_shared_worktree(...)`
⇒ 读到的永远是引擎不写的那个维度 ⇒ 断言永远不成立。

改法：**只改断言的读取维度**（issue 维 → 仓维），**不改断言的语义强度与期望值**。

---

## 2. 逐条维度对照

### 2.1 `completing_group_unit_moves_issue_shared_lock_to_next_unit`

| 项 | 改前 | 改后 |
|---|---|---|
| 夹具 | `group_engine_with_two_units()`（part_12，**无 LC 权威**、无冻结快照） | `group_engine_with_two_units_under_lc_authority()`（part_13 自带，LC 权威 + 冻结 `target_snapshot`） |
| 锁现场 | `upsert_issue_shared_worktree` + `try_acquire_issue_worktree_lock`（issue 维） | `seed_repo_shared_worktree_lock(..., "work_item_0001")`（**仓维**，owner = 本 attempt，活跃项 = work_item_0001） |
| 断言读取 | `get_issue_shared_worktree("project_0001","issue_0001")` | `get_repo_shared_worktree("project_0001","issue_0001", authority.member_id)` |
| 断言期望 | `current_active_work_item_id == Some("work_item_0002")` | **一字未改**：`current_active_work_item_id == Some("work_item_0002")` |

**语义保持不变的依据**

1. 期望值逐字未改：仍是「unit 完成后锁转移到**下一个** unit（work_item_0002）」。
2. 前置条件等价：改前夹具在 **work_item_0001** 上持锁（owner = attempt id），改后夹具仍在
   **work_item_0001** 上持锁（owner = attempt id，仓维三元键）—— 变的只是**记录落在哪个维度**。
3. 转移仍由引擎执行：断言读的是引擎写出的仓维记录，不是夹具自说自话（见 §4 负控）。
4. 「谁持锁」语义未被削弱：`transfer_repo_worktree_lock` 要求 owner 匹配（否则
   `Conflict{kind:"repo_worktree_lock_owner"}`），夹具把 owner 绑到本 attempt ⇒
   「锁由本 attempt 持有并随之转移」这一约束仍然成立。

### 2.2 `group_final_confirm_completes_attempt_after_all_units_completed`

| 项 | 改前 | 改后 |
|---|---|---|
| 夹具 | `group_attempt_waiting_for_final_confirm()`（part_12，**issue 维**锁 + 无 LC 权威） | `group_attempt_waiting_for_final_confirm_under_lc_authority()`（part_13 内新建，LC 权威 + 冻结快照 + **仓维**锁） |
| 锁现场 | `upsert_issue_shared_worktree` + `try_acquire_issue_worktree_lock`（issue 维，活跃项 work_item_0002） | `seed_repo_shared_worktree_lock(..., "work_item_0002")`（仓维，owner = 本 attempt） |
| 断言读取 | `get_issue_shared_worktree(&updated.project_id, &updated.issue_id)` | `get_repo_shared_worktree(&updated.project_id, &updated.issue_id, authority.member_id)` |
| 断言期望 | `current_active_work_item_id == None`；`last_completed_work_item_id == Some("work_item_0002")` | **一字未改**（同上两行） |

**语义保持不变的依据**

1. 期望值逐字未改：确认后**锁被释放**（`current_active_work_item_id` 为空）且**最后一个 unit 记为
   已完成**（`last_completed_work_item_id == work_item_0002`）。
2. 其余断言（attempt 变 `Completed`、scope 为 `WorkItemGroup`、全部 unit 为 `Completed`）全部保留。
3. 夹具形态与 part_12 同款：work item + plan + 权威终局夹具（handoff revision + unit run）+
   review request + work item 执行状态 + unit 完成 + running 播种 + `FinalConfirm`/`WaitingForHuman` +
   head commit + 终局就绪快照；**唯一差异**是锁维度与 `target_snapshot`。
4. 释放路径等价：引擎在 group 收口时先 `mark_repo_worktree_completed_item`（写
   `last_completed_work_item_id` 并清活跃项）再 `release_repo_worktree_lock_by_owner`
   （清 owner）⇒ 仓维记录终态与 issue 维记录终态同形。

---

## 3. 改动清单

### 3.1 新增夹具（part_13 内部，不动 part_12 共享夹具）

`group_attempt_waiting_for_final_confirm_under_lc_authority()`

- **形态纪律**：LC 权威**先于** attempt 建立（`ProjectStore::create` 的 project id 按
  `projects/` 目录数分配；attempt 落盘本身就会建 `projects/project_0001/issues/issue_0001/`
  ⇒ 先建 attempt 再建权威会把 project 分配成 `project_0002`，权威与 attempt 的 project 身份
  错位 ⇒ routing 恒 fail-closed）。
- 其余形态与 part_12 夹具逐项对齐（见 §2.2 第 3 点）。

### 3.2 复用夹具（上一轮已在 part_13 建好，本次直接复用）

- `group_engine_with_two_units_under_lc_authority()`
- `group_engine_with_last_running_unit_under_lc_authority()`
- `group_engine_under_lc_authority(...)`
- `seed_repo_shared_worktree_lock(paths, authority, attempt, work_item_id)`（建**仓维**记录 +
  以 attempt id 为 owner 持锁）

### 3.3 改动行数

| 文件 | 改前行数 | 改后行数 | 增/删 |
|---|---|---|---|
| `tests/it_product/product_coding_workspace_engine/part_13.rs` | 606 | **730** | +148 / −24 |

---

## 4. 「绿在错误原因上」的排查（负控证据）

Main 要求确认：这两条**不是**因为 fail-closed 恰好满足才绿的。

**负控 A**（临时注释 test1 的仓维 seeding，其余不动）：

```
complete unit: Store(Io("open /tmp/.tmpXXXX/.aria/projects/project_0001/issues/issue_0001/
                         shared-worktrees/<uuid>.json: No such file or directory (os error 2)"))
⇒ FAILED
```

即：缺仓维 shared worktree 记录时，引擎的 `transfer_repo_worktree_lock` **直接 IO 失败**
（缺文件），**不是** fail-closed；断言锚在真实仓维锁记录上。

**负控 B**（临时注释终局夹具里的仓维 seeding）：

```
panicked at part_13.rs: expectation failed: existing shared worktree
⇒ FAILED
```

即：缺仓维记录时 `get_repo_shared_worktree` 返回 `None` ⇒ 断言读不到记录 ⇒ 红。

**推论**：两条用例的绿都要求引擎**真的写下了仓维锁事实**（转移 / 标记完成 / 释放）。
若引擎走 fail-closed，`.expect("complete unit")` / `.expect("final confirm")` 会先 panic ⇒ 必红。
⇒ **无「绿在错误原因上」问题**。

**一次无效负控的作废说明（留痕）**：第一次负控（同时注释两处 seeding）报「2 passed」，
经 diff/md5 复核发现：编译到的实际是并发 worker（P2B）在同一时刻写回的另一版
part_13.rs（两处 seeding 均为可用态）⇒ 该结论作废；重跑（带版本校验）后得到上述
负控 A / B 的真实红因。

---

## 5. 测试数字

均以 `flock /tmp/aria-cargo.lock` 串行执行。

| 项 | 命令 | 结果 |
|---|---|---|
| 目标 2 条 | `cargo test --locked --test it_product -- completing_group_unit_moves_issue_shared_lock_to_next_unit group_final_confirm_completes_attempt_after_all_units_completed` | **2 passed / 0 failed** |
| 引擎模块 | `cargo test --locked --test it_product -- product_coding_workspace_engine` | **73 passed / 0 failed**（138 filtered out） |
| 全量 it_product | `cargo test --locked --test it_product` | **211 passed / 0 failed**（0 ignored） |
| 格式 | `cargo fmt --check` | **clean**（exit 0，无输出） |

基线对照：本组开工时全量为 202 passed / 9 failed（9 条失败全在 part_01，
由 part_01 owner 的 P2 改指仓维收口）⇒ 收口后全量 211 / 0。

---

## 6. 声明

- **未改任何断言的语义强度**：2 条用例的期望值逐字未改（`Some("work_item_0002")`、
  `None`、`Some("work_item_0002")`），其余断言（attempt 状态 / scope / 全部 unit 完成）全部保留。
- **未删用例、未加 `#[ignore]` / `should_panic`**。
- **未改 spec / delta / tasks.md**，未改 `openspec/**`。
- **未改 `src/**`、`web/**`、`tests/it_product/context_builder.rs`**，未改其他
  `tests/it_product/**` 文件（part_12 的共享夹具只读不改）。
- **未做任何 git 操作**（无 add / commit / stash / checkout / reset）。
- `cargo` 全部经 `flock /tmp/aria-cargo.lock` 串行执行，未起第二个 cargo 任务。

---

## 7. Concerns（需 Main 知悉）

1. **并发编辑撞车**：MigrateItProductP2B（P2B）在 20:26~20:28 也写过 `part_13.rs`
   （夹具内的仓维 seeding + 恢复 test1 的 seeding）。已通过「md5 + diff 双校验」确认最终文件
   （`md5 = 746c932b62f1aac4977693b9306a5e2b`，730 行）与 P2B 的净改动**共存无丢失**；
   P2B 已确认其 3 条 P3 及其夹具未被我的改动触碰。⚠️ 若后续还有 worker 要动 part_13，
   建议先与 P2B / 我对齐，避免再次出现「编译到别人版本」。
2. **死代码已由他人处置**：我不再引用 part_12 的 `group_engine_with_two_units()` 后，该函数一度
   全仓零引用 ⇒ `dead_code` warning ⇒ CI 的 `cargo clippy --all-targets -- -D warnings` 会阻断。
   已由 P2B 删除（commit `e63a9fd`）解决，我未动 part_12。
3. **part_12 仍是 issue 维**：`group_attempt_waiting_for_final_confirm()` 仍走 issue 维
   `upsert_issue_shared_worktree` / `try_acquire_issue_worktree_lock`，若 part_01 的
   `group_final_confirm_releases_transferred_shared_worktree_lock_by_owner` 等要改仓维，
   需要把 `LogicalRepositoryId` 带出夹具签名（会影响 part_01 调用点）⇒ 口径归
   UnblockIntoPartsAndRepointP2 / Main，我未越界处理。
4. **用例名保留 "issue_shared_lock" 字样**：`completing_group_unit_moves_issue_shared_lock_to_next_unit`
   的断言已改读仓维，但**函数名未改**（改名会波及 spec / 任务清单里的用例引用，且超出
   「只改读取维度」的范围）。如需一并改名为 `..._moves_repo_shared_lock_to_next_unit`，请示下后由
   归属者统一执行。
5. **未改 issue 维 API 的其他调用点**：part_12 / part_01 里仍有 issue 维 shared worktree 用法，
   不在本组范围。
