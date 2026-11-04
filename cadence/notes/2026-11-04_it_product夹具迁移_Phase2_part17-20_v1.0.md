# it_product 夹具迁移 · Phase 2（part_17 / part_18 / part_19 / part_20）· v1.0

> 日期：2026-11-04
> 执行：MigrateItProductP2C（worker）
> 范围：`tests/it_product/product_coding_workspace_engine/part_17.rs` / `part_18.rs` /
> `part_19.rs` / `part_20.rs`。
> **未改任何 `src/**` / `web/**` / 其他 `tests/it_*` / `openspec/**` / `context_builder.rs`**。

---

## 0. 一句话结论

本组 4 个文件里共 **9 个失败用例**：**7 个属 P3（LC 权威 + 冻结快照即可解）→ 已全部迁绿**；
**2 个属 P2（issue 维 shared worktree 族）→ 断言的是块3 已下线的产品路径，本阶段不动**。

| 文件 | 用例数 | 迁绿 | 仍红 | 仍红归属 |
|---|---|---|---|---|
| `part_17.rs` | 4（2 红 2 绿） | **0** | **2** | P2 ×2 |
| `part_18.rs` | 1（红） | **1** | 0 | — |
| `part_19.rs` | 3（红） | **3** | 0 | — |
| `part_20.rs` | 3（红） | **3** | 0 | — |
| **合计** | **11** | **7** | **2** | P2 ×2 |

**P1（provider-run 族）在我负责的 4 个文件里为 0 个**——这 4 个文件不跑 provider，
因此不需要 `LogicalCodebaseProviderGateway` / `GatewayBridgedAdapter`。

---

## 1. 逐用例清单

### 1.1 `part_18.rs`（1 例，全绿）

| 用例 | 原失败签名 | 迁移手法 | 状态 |
|---|---|---|---|
| `handle_abort_marks_attempt_aborted_and_closes_active_timeline_node` | `handle_abort` 在 `route_issue_shared_worktree` 上 fail-closed：`Store(Io("target_snapshot_identity_drifted"))`（attempt 无 `target_snapshot`、issue 无 LC 归属 ⇒ `RepositoryRouting::FailClosed`） | 建 `paths` → `seed_logical_authority(&paths, "project_0001", "issue_0001", root.path())` → 复用共享 `create_input()` 挂 `authority.snapshot(&paths)` 后建 attempt | ✅ 绿 |

补充：`seed_coding_attempt_running` 走 `get_attempt` + `write_json` 整体回写，
**冻结快照随 attempt record 一并保留**（实测 part_18 的 `handle_abort` 走到终态收口，
说明快照确实在场且与权威一致）。

### 1.2 `part_19.rs`（3 例，全绿）

| 用例 | 原失败签名 | 迁移手法 | 状态 |
|---|---|---|---|
| `rejected_push_with_verified_missing_remote_ref_records_failed_review_request` | `start attempt: Store(Io("target_snapshot_identity_drifted"))`（admission fail-closed） | 同上（LC 权威 + `create_input()` 挂快照） | ✅ 绿 |
| `nonzero_push_with_authoritatively_updated_remote_records_pushed_review_request` | 同上 | 同上 | ✅ 绿 |
| `ambiguous_push_with_initial_old_ref_converges_after_remote_updates_later` | 同上 | 同上 | ✅ 绿 |

补充：push / review-request 链**不跑 provider**，故**无需** gateway / registry /
`GatewayBridgedAdapter`；`execute_review_request` 首步的
`detect_cross_target_violation_for_delivery` 因「无 role run」按无越界放行。

### 1.3 `part_20.rs`（3 例，全绿）

| 用例 | 原失败签名 | 迁移手法 | 状态 |
|---|---|---|---|
| `group_completion_gate_rejects_changed_files_in_forbidden_scope_from_git` | `handle_final_confirm` 在 `validate_attempt_issue_shared_worktree_owner_if_present` 上 fail-closed（同 `target_snapshot_identity_drifted`） | 夹具 `group_attempt_with_committed_unit_changes` 内补 LC 权威 + **全部夹具写盘完成后** `attach_target_snapshot` | ✅ 绿 |
| `group_completion_gate_allows_changed_files_within_exclusive_scope_from_git` | 同上 | 同上 | ✅ 绿 |
| `group_completion_gate_fails_closed_when_worktree_is_missing` | 同上 | 同上 | ✅ 绿 |

**为什么快照挂在夹具最末尾**（形态纪律）：group attempt 一旦带 `target_snapshot`，
`validate_group_attempt_integrity` 会走 Logical 路由分支
（`validate_group_single_target` 要求权威 unit 带 `target_repository_id`）。
本夹具**故意不绑计划**（用例断言的正是「因夹具未绑定计划而停在
`coding_attempt_plan_binding`」），故快照必须在所有中间写盘完成后挂载，
否则夹具自身会先死在 group 归属校验上。

**scope 前缀不影响断言**：`scope_member_prefix` 返回 `repository_0001`
（成员 checkout 相对聚合根），而 `scope_allows_member_path` 同时接受
「仓内相对」与「根相对带成员前缀」两种写法 ⇒ 用例断言的仓内相对路径
（`web/src/app.tsx`）仍原样成立。

### 1.4 `part_17.rs`（4 例：2 红 → P2；2 绿 → 保持不动）

| 用例 | 状态 | 说明 |
|---|---|---|
| `group_unit_owner_conflict_does_not_advance_unit_or_attempt` | ❌ 仍红（**P2**） | 见 §2 |
| `final_confirm_owner_conflict_does_not_complete_attempt` | ❌ 仍红（**P2**） | 见 §2 |
| `abort_owner_conflict_does_not_abort_attempt` | ✅ 绿（**本阶段不动**） | 见 §2.2 |
| `failure_owner_conflict_does_not_fail_attempt` | ✅ 绿（**本阶段不动**） | 见 §2.2 |

---

## 2. P2 判定（issue 维 shared worktree 族）

### 2.1 两个仍红用例的阻塞签名

两例的夹具都建 **issue 维** shared worktree 记录
（`LifecycleStore::upsert_issue_shared_worktree` + `try_acquire_issue_worktree_lock`，
owner = `coding_attempt_other`），断言引擎因「他人持锁」而拒绝：

```rust
assert!(matches!(
    error,
    CodingWorkspaceEngineError::Store(ProductStoreError::Conflict {
        kind: "issue_worktree_lock_owner", ..
    })
));
```

块3 删掉 `IssueSharedWorktreeRoute::Legacy` 臂后，引擎**只读仓维**记录，
issue 维锁**再也不会**产生 `issue_worktree_lock_owner` 冲突
（仓维对应 kind 是 `repo_worktree_lock_owner`）。⇒ 两条路都红：

| 路径 | 实际错误签名 | 证据 |
|---|---|---|
| **现状**（无快照、无 LC 权威） | `Store(Io("target_snapshot_identity_drifted"))` | `route_issue_shared_worktree` 的 `FailClosed` 臂（`src/product/coding_workspace_engine/gates.rs:141-146`）；同一稳定码在本阶段的**早前一轮**全量跑里由 `review_request_blocks_when_only_runtime_artifacts_changed` 直接观测到（`start attempt: Store(Io("target_snapshot_identity_drifted"))`，该例后被并行 worker 迁绿） |
| **补 LC 权威 + 快照后** | `LegacySharedWorktreePresent("project_0001/issue_0001")` | `preflight_repo_shared_worktree_absent` 见到同 issue 下存在旧 `issue-shared-worktree.json` ⇒ fail-closed；同一签名在**收尾全量跑**里由 `final_confirm_releases_issue_shared_worktree_lock` / `failed_group_attempt_releases_transferred_shared_worktree_lock_by_owner` / `failed_attempt_releases_issue_shared_worktree_lock` 直接观测到（这三条正是「带快照 + 留有 issue 维旧记录」的同构现场） |

两条路都不是断言要的 `Conflict { kind: "issue_worktree_lock_owner" }`
⇒ **P2：夹具不是原因，断言指向已下线的产品路径**（改断言被本阶段铁律禁止）。

### 2.2 同文件另两例「是绿的，但绿在错误的原因上」——本阶段**不动**

`abort_owner_conflict_does_not_abort_attempt` / `failure_owner_conflict_does_not_fail_attempt`
的断言只要求「**报错** + attempt 状态未变」，而 fail-closed
（`target_snapshot_identity_drifted`）恰好满足 ⇒ 它们现在是绿的。

**若给它们补 LC 权威 + 快照，反而会变红**（代码路径推导，**未实测**）：仓维 shared worktree
记录不存在 ⇒ `validate_attempt_issue_shared_worktree_lock_if_present` 无冲突可查 ⇒
`handle_abort` / `handle_attempt_failed` 会**成功执行** ⇒ attempt 状态不再停留在
`Created` ⇒ 断言 `status == Created` 失败。

> 推导依据：`route_issue_shared_worktree` 在 `target_snapshot` 在场时走仓维分支
> （`gates.rs:129-134`），仓维记录缺失 ⇒ `get_repo_shared_worktree` 返回 `None` ⇒
> `validate_*_if_present` 直接 `Ok(())`（`gates_parts/worktree_lock.inc.rs:294-330`）。
>
> ⚠️ 本条的实测验证（临时探针用例）**因并行 worker 在途编辑导致的编译失败而未完成**，
> 探针已删除（`part_17.rs` 保持 177 行零改动）。

⇒ **建议 Main 注意**：任何「无差别给 part_17 全量补 LC 权威」的批量迁移都会把这两例
从绿变红。本阶段**未动**这两例（零改动）。

---

## 3. 验证输出

### 3.1 模块级（`product_coding_workspace_engine`）

```bash
flock /tmp/aria-cargo.lock cargo test --locked --test it_product -- product_coding_workspace_engine
```

| | 迁移前（Phase 1 基线） | 迁移后（本阶段完成 + 并行 worker 同期在途成果） |
|---|---|---|
| passed | 15 | **55**（收尾时全量跑里引擎模块为 56 绿 / 18 红） |
| failed | 58 | **19**（收尾时全量跑里引擎模块为 18 红） |

本阶段独立贡献：**+7 绿 / −7 红**——迁移前后本组 4 文件的 7 个 P3 用例全部由红转绿，
且**最近两次**模块级与全量级失败清单里**都不再出现**这 7 个用例名。

### 3.2 全量 `it_product`（收尾快照）

```bash
flock /tmp/aria-cargo.lock cargo test --locked --test it_product
```

```
test result: FAILED. 193 passed; 18 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.82s
```

> 该数字是**并行 worker 在途状态下的快照**（Phase 1 基线为 `153 passed / 58 failed`
> ⇒ 累计 `+40 绿 / −40 红`，其中 **+7 绿属本阶段**）。
> 18 条失败**全部**落在 `product_coding_workspace_engine::` 模块内，其中 **2 条属本组文件**。

### 3.3 本组 4 文件的最终失败清单（**仅 2 条，均为 P2**）

```
product_coding_workspace_engine::final_confirm_owner_conflict_does_not_complete_attempt      ← part_17（P2）
product_coding_workspace_engine::group_unit_owner_conflict_does_not_advance_unit_or_attempt    ← part_17（P2）
```

其余 16 条失败全部落在 `part_01` / `part_04` / `part_13` 等**其他 worker 的文件**
（`LegacySharedWorktreePresent` / `Store(NotFound{kind:"issue"})` /
`coding_run_lease_unknown` / `logical_provider_gateway_required` 四类签名）。

### 3.4 编译零警告

```bash
flock /tmp/aria-cargo.lock cargo build --locked --tests
```

⇒ **0 warning / 0 error**。

### 3.5 cargo fmt

`part_XX.rs` 是 `include!` 片段，`cargo fmt` **不覆盖**（rustfmt 不展开 `include!`），
与 Phase 1 结论一致；本阶段未对片段单独跑 rustfmt（避免整文件去缩进的噪声）。

---

## 4. 被改文件行数

| 文件 | 变化 | 说明 |
|---|---|---|
| `tests/it_product/product_coding_workspace_engine/part_18.rs` | **+17 / −6**（净 +11） | 1 例：LC 权威 + 快照 |
| `tests/it_product/product_coding_workspace_engine/part_19.rs` | **+42 / −12**（净 +30） | 3 例：各 +10 |
| `tests/it_product/product_coding_workspace_engine/part_20.rs` | **+17 / −4**（净 +13） | 夹具补权威 + 末尾挂快照 |
| `tests/it_product/product_coding_workspace_engine/part_17.rs` | **0 / 0** | P2，本阶段不动 |
| **合计** | **+76 / −22（净 +54）** | |

**断言零改动**：全程零 `assert_*` 改动、零期望值改动、零测试名改动、零 `#[ignore]`、
零 `assert!`/`expect` 删除。改动全部落在 **setup**（建权威 / 挂快照）。

`src/**`、`web/**`、其他 `tests/it_*`、`openspec/**`、`tests/it_product/context_builder.rs`：
**零改动**。

---

## 5. helper 需求

**无新增需求。** Phase 1 的 `context_builder.rs` 三个导出项即足够：

| API | 本阶段用途 |
|---|---|
| `seed_logical_authority(&paths, project_id, issue_id, root) -> LogicalAuthority` | part_18 / part_19 / part_20 建权威现场 |
| `LogicalAuthority::snapshot(&paths) -> AttemptTargetSnapshot` | 建 attempt 时冻结快照（part_18 / part_19）、挂快照（part_20） |
| `attach_target_snapshot(&store, &attempt, snapshot)` | part_20 给**已落盘**的 group attempt 补挂快照 |

（`build_gateway` / `GatewayBridgedAdapter` 本阶段未用到——这 4 个文件不跑 provider。）

**给 Main 的一条可选建议**（非阻塞）：`attach_target_snapshot` 目前只能「夹具全部写盘后」
使用；若后续有 fixture 需要在**中间**写盘阶段就带上快照（如 part_12 的 group 权威夹具
`group_engine_with_two_units_under_lc_authority` 那样建组即带快照），helper 作者可能需要
再补一个「建 attempt 时直接带 `target_snapshot`」的入口——但 `create_group_attempt` 本身
就接受 `target_snapshot`，**不需要** helper 改动。

---

## 6. Concerns

1. **P2 同族用例仍需裁定**：收尾全量跑里 18 条失败中，**10 条属 issue 维 shared worktree
   族**（本组 2 个 + `part_01` 的 7 个 + `part_13` 的 1 个；签名统一为
   `LegacySharedWorktreePresent("project_0001/issue_0001")` 或
   `Conflict{kind:"issue_worktree_lock_owner"}` 断言失败），其余 8 条属 P1 provider-run 族
   （`coding_run_lease_unknown` / `logical_provider_gateway_required`）。
   P2 的可选路径（需 Main 裁定）：
   (a) 把断言的 kind 改指仓维 `repo_worktree_lock_owner` + 夹具改建仓维记录
   （**改断言，超出本阶段铁律**）；(b) 登记为已知红；(c) 产品侧恢复兼容臂。
   本组 peer `UnblockIntoPartsAndRepointP2` 正在做 (a) 方向的 part_01 / part_12 部分。
2. **「绿在错误的原因上」的用例**（§2.2）：`abort_owner_conflict_does_not_abort_attempt` /
   `failure_owner_conflict_does_not_fail_attempt` 现在靠 fail-closed 侥幸通过，
   **一旦补 LC 权威就会翻红**。建议在裁定 P2 时一并处理，别让后续 worker 误补。
3. **并发编译抖动**：本阶段期间有并行 worker 在改 `src/cross_cutting/session_launch.rs`
   （A1：`into_parts` 放宽为 `pub`，19:16 落盘）与
   `src/web/types/logical_codebase_bootstrap.rs`（19:05）。中途出现过两次
   **他人 in-flight 编辑导致的编译失败**（`E0507: cannot move out of __self.provider_type`、
   `mismatched closing delimiter`），与本阶段改动无关，且随后自行消失。
   本阶段**未触碰任何 `src/**`**，也未代改他人文件。
4. **全量数字是并发快照**：§3.2 的 `193 passed / 18 failed` 含其他 worker 的在途成果，
   重跑可能变化；本组 4 文件的权威数字是 §3.3（**仅 2 条失败，均 P2**）。
5. **helper 侧 API 变更对本组无影响**：helper 作者在本阶段收尾时把
   `GatewayBridgedAdapter` 改成 `GatewayBridgedAdapter::new(inner, ProviderType::X)`
   并新增 `GatewayValidatedStartAdapter` / `gateway_bridged_output_schema`。
   **本组 4 个文件不跑 provider、零处调用 gateway**，且我用到的三个导出项
   （`seed_logical_authority` / `LogicalAuthority::snapshot` / `attach_target_snapshot`）
   **签名未变** ⇒ 无需跟改；收尾的模块级 + 全量级跑均在**新 helper** 之上完成。
6. **未跑 `cargo fmt`**：见 §3.5（`include!` 片段不在 `cargo fmt` 覆盖范围内）。
7. **临时探针已清除**：为取 P2 用例的精确错误签名曾在 `part_17.rs` 追加一个临时用例；
   因并行 worker 在途编辑导致编译失败（`mismatched closing delimiter`、
   `borrow of moved value: store`，均**不在**我的文件里），探针未跑成并已删除，
   `part_17.rs` 恢复为 **177 行零改动**（已用 `wc -l` + 尾字节比对核验）。
   §2.1 的两条签名因此改用「源码路径 + 同轮 sibling 用例的实测签名」作为证据。
