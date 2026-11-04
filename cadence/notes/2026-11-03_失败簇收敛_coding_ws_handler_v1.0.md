# 失败簇收敛报告 — `web::coding_ws_handler`（v1.0）

> 日期：2026-11-03
> 工作树：`.worktrees/bugfix-b-11-02`
> 范围：**只改夹具**（`src/web/coding_ws_handler/tests.rs` + `src/web/coding_ws_handler/tests/**` + `src/web/coding_ws_handler/context.rs` 的 `#[cfg(test)]` 夹具）。
> 断言 / 实现 / spec / Plan / 技术方案 **一字未改**；未恢复 `RepositoryRouting::Legacy`；未自造 reason_code；未删断言。

---

## 0. 根因（与调度简报一致）

本轮「单仓入口删除（REQ-ROUTE-01 两态化）」把 coding attempt 的准入凭证从「无归属也放行」改成
「**LC 权威 + 冻结 `target_snapshot` + 仓维锁**」三件套。web 侧旧夹具三件全缺：

| 缺什么 | 触发的失败面 |
|---|---|
| 缺 issue 记录 | `Store(NotFound { kind: "issue", id: "issue_0001" })` / `repository routing load failed: product_store_not_found: issue issue_0001` |
| 缺 LC 权威（manifest/member/checkout/selection/policy/repos.json） | 路由 `FailClosed` ⇒ `classify_worktree_lease_for_attempt` 判 `UnknownNeedsHuman` |
| attempt 缺 `target_snapshot` | 租约证据 `target_snapshot_missing_for_logical` / `target_snapshot_identity_drifted` ⇒ `LeaseUnknown` ⇒ provider run 停等 `coding_run_lease_unknown` |
| 缺仓维锁 | 带快照的 attempt 只按**仓维**分类租约；issue 维记录会被判 `legacy_shared_worktree_present` fail-closed |

⇒ 夹具必须补齐：完整 LC 权威 + 与权威逐字一致的冻结快照 + 仓维 worktree 记录与锁（绑到本 attempt）。

---

## 1. 逐簇表

| 簇 | 用例数 | 改了哪些夹具 | 窄跑后剩余 |
|---|---|---|---|
| `tests::seed_compiled_work_item_fixture`（`failed_review_recovery` 26 + `runner_cleanup` 5 + `runner_recovery` 3 + `tests.rs` 若干） | 34+ | `src/web/coding_ws_handler/tests.rs::seed_compiled_work_item_fixture`：`seed_complete` 播种 project+issue+完整 LC 权威；attempt 带 `target_snapshot: Some(logical.snapshot(...))`；补 `admission_ticket_consumed_at`；`upsert_repo_worktree` + `bind_repo_lock_to_attempt`（work_item_id=`work_item_compile_20260702063721302_001`，attempt_id=`coding_attempt_0001`）；attempt 落盘（租约分类要按 owner 读回 attempt 判活跃） | 0 |
| `plan_repair::*` | 12 | `src/web/coding_ws_handler/tests/plan_repair/support.rs::plan_repair_fixture_with_dependency`：改用 `seed_complete`（去掉自建物理仓 + 自建 issue），group attempt 带 `target_snapshot: Some(...)`，补 `upsert_repo_worktree` + `bind_repo_lock_to_attempt`（work_item_id=`wi_current`）；夹具 engine 注入 LC gateway | 3（见 §4） |
| `runner_cleanup::*`（http 族） | 5 | `src/web/coding_ws_handler/tests/runner_cleanup.rs::seed_http_attempt_with_lease`：① 共享夹具已在 `repos.json` 播种 `repository_0001` 投影，原夹具再建一个仓会拿到 `repository_0002` ⇒ 改为**复用**已播种记录（断言 `repository_0001` 未改）；② 夹具不再建 **issue 维** `issue-shared-worktree.json`（快照 attempt 的 worktree 路由是**仓维**，issue 维记录在场 ⇒ `legacy_shared_worktree_present` ⇒ HTTP 409），仓维记录与锁沿用共享夹具已按 attempt 播种的那份 | 5 → **预期 0**（② 是统一跑之后才改，未在统一跑里验证） |
| `runner_recovery::*` | 3 | `src/web/coding_ws_handler/tests/runner_recovery.rs`：`recover_coding_channel_re_admits_and_restarts_runner` / `recover_coding_supersedes_residual_running_role_runs` 补 `seed_complete` + `target_snapshot`（恢复通道的 admission CAS 重验「路由/快照/policy」，缺任一即 `target_snapshot_identity_drifted`） | 1（见 §4） |
| `context::tests` | 2 | `src/web/coding_ws_handler/context.rs` 两条自建夹具：补 issue 记录（归属解析是路由唯一入口，缺 issue 直接 `NotFound{issue}`）+ 与权威一致的冻结快照 | 0 |
| `runner::tests` | 4 | **未改**（`src/web/coding_ws_handler/runner.rs` 的 `#[cfg(test)]` 夹具不在本簇授权路径内；且其 LC 形态断言要求「非 legacy 别名 lc_id 的真多仓 LC」，共享夹具只播种 legacy 别名 LC，无法就地满足） | 4（见 §4） |
| `tests::coding_pi_start_failure_does_not_start_registered_alternate_provider` | 1 | `src/web/coding_ws_handler/tests.rs`：补 `seed_complete` + `target_snapshot` + 仓维锁；引擎注入 LC gateway（fake adapter 经可换挡替身装入 gateway registry） | 0 |

**中间实测（改完 `seed_compiled_work_item_fixture` / `plan_repair_fixture` / `context` 三处后）：`155 passed / 21 failed`（基线 58 failed）⇒ 已收 37 条。**
最终数字见 §5。

---

## 2. gateway 替身关系说明（Main 已批准，原样贴出）

```
产品层 build_gateway_with_registry 位于 src/product/coding_workspace_engine/tests/（模块私有）
  ⇒ web 测试树不可见
  ⇒ web 侧按同一形状建本地替身 src/web/coding_ws_handler/tests/gateway.rs
  ⇒ 非第二套共享夹具；logical_test_fixture.rs 仍只读复用
```

补充说明（为何 web 侧必须自建替身）：

- REQ-ROUTE-01 两态化后，携带冻结 `target_snapshot` 的 attempt **只能**经
  `LogicalCodebaseProviderGateway` 启动 provider（`provider_stream.rs` 的
  `is_logical_target = attempt.target_snapshot.is_some()` 门要求 `validated_input` 非空，
  legacy 直连被强制关闭；`launch_provider_session` 在「validated 在场」臂**忽略**调用方传入的
  裸 adapter，改由 gateway 从**自己的 registry** 解析 provider）。
- 生产 gateway 的能力源读 durable capability 记录（launch/resume/write_boundary 三格须
  `Confirmed`，需真实 probe 产出）、可用性门读真实 provider 健康快照 —— 二者都不是 web 测试面
  能就地满足的（`ProviderActionMatrix` 的 `rows` 私有，web 层无法构造 Confirmed 矩阵）。
- 故 web 侧按产品层同型双件套建替身：能力源恒 `Confirmed`、可用性门恒健康、target 直通、
  同步槽打桩；provider 由用例经**可换挡替身**（`SwappableProvider`）装入 registry
  —— `ProviderRegistry::register` 需 `&mut self`，gateway 一旦持有 `Arc` registry 就无法再注册，
  而 gateway 必须在夹具构造期就绪、用例的 fake adapter 却在其后才有的。

---

## 3. 共享夹具（`src/product/logical_codebase/logical_test_fixture.rs`）能力评估

**结论：现有能力已覆盖本簇全部所需，本轮未扩能力、未改该文件（只读复用）。**
用到：`seed_complete` / `snapshot_for` / `LogicalFixture::snapshot` / `upsert_repo_worktree` /
`bind_repo_lock_to_attempt`。

已发现的**缺口（仅报告，未自造）**：

1. **缺「真多仓（非 legacy 别名 lc_id）LC 权威」的播种能力**。
   `seed_complete` / `seed_logical_authority` 固定走 legacy 别名 id（`legacy_logical_codebase_id`），
   `LogicalCodebaseStore::new`（project 级根）。而 `runner::tests` 的
   `internal_pr_review_form_fixture` 以 `logical_codebase_id = "logical_codebase_0001"`
   断言「真多仓形态 ⇒ 跳过 InternalReviewer provider」，需要 per-LC 子树
   （`logical-codebases/{lc_id}/`）的 manifest/member/checkout/selection。
   ⇒ 建议共享夹具增一个 `seed_logical_authority_for_lc(paths, project_id, issue_id, root, lc_id)`
   （显式 lc_id + 子树根），否则该簇只能在 web 侧手搓 per-LC 权威。
2. **`seed_complete` 内部会调两次 `ProjectStore::create`**（`seed_complete` 一次 +
   `seed_logical_authority` 一次）⇒ 临时根里多出一个 `project_0002`。
   对本簇无害（无人列举 projects），但对「按目录列举取首个 project」的夹具是隐患
   （`startup_window_compensation` 正是这种夹具）。
   ⇒ 建议共享夹具去掉 `seed_logical_authority` 里那次重复 create，或暴露
   `seed_logical_authority_without_project`。
3. **缺「web 层测试 gateway」能力**（见 §2）——本轮按 Main 批准在 web 侧建了本地替身，
   未污染共享夹具。

---

## 4. 仍未解项（逐条归因）

| # | 用例 | 归因 | 说明 |
|---|---|---|---|
| 1 | `runner::tests::logical_codebase_attempt_skips_internal_pr_review_provider` | **夹具需 per-LC 权威（共享夹具缺能力）** | 夹具以 `logical_codebase_id="logical_codebase_0001"` 断言真多仓形态；共享夹具只播种 legacy 别名 LC |
| 2 | `runner::tests::logical_codebase_legacy_internal_pr_review_attempt_skips_provider` | 同上 | 同上 |
| 3 | `runner::tests::plan_coding_s2_runner_flows_to_final_confirm_with_plan_run` | **夹具未修** | `n5_runner_fixture` 自建 project/issue/attempt（无 LC 权威、无快照、无仓维锁） |
| 4 | `runner::tests::sc_advance_completion_parks_cleanly_when_next_unit_waits_on_out_of_bucket_work_item` | **夹具未修** | `n5_sc_advance_parked_fixture` 同上 |
| 5 | `plan_repair::provider_start_failure_recovery::coding_plan_repair_provider_start_failure_recovery_modern_can_retry_again` | **需要生产 gateway 通路** | 走 **runner** 路径 ⇒ gateway 由 `state.gateway_factory()`（生产工厂）组装：能力源读 durable capability 记录（web 层无法构造 `Confirmed` 矩阵）、可用性门读真实健康快照、trust 源读真实 trust store —— 三者都不是 web 测试面可就地满足的 |
| 6 | `plan_repair::provider_start_failure_recovery::..._legacy_can_retry_again` | 同上 | 同上 |
| 7 | `plan_repair::failed_review_recovery::coding_plan_repair_recovered_reviewer_retry_can_start_plan_repair` | 同上 | 同上（同样走 runner 路径） |
| 8 | `runner_recovery::startup_window_compensation::bad_tree_fail_closed_while_conflict_free_tree_resumes` | **夹具未修（影响面大，未在本轮动）** | `WindowStartupFixture` 被同文件 20+ 条用例共用；改它需同时播种 LC 权威 + 快照 + selection（多树同根共享成员），一旦有误会连带打红同文件 20+ 条已绿用例；本轮按「只排一次窄跑」的节奏未冒险改 |
| 9-13 | `runner_cleanup::http_*`（5 条） | **已修，但改在统一跑之后（未验证）** | 统一跑里报 HTTP 409（`legacy_shared_worktree_present`）：夹具建的 **issue 维** `issue-shared-worktree.json` 与快照 attempt 的**仓维**路由互斥。已改为不建 issue 维记录（仓维记录与锁由共享夹具按 attempt 播种）。**该改动未在统一跑里验证**，待收口 `cargo test --all-targets` 确认 |

**没有一条是「断言语义已作废」**：本簇的断言全部仍成立，缺的是夹具前置条件。
唯一的语义位移在 `context::tests::repository_path_legacy_single_repo_without_selection_uses_physical`
——该用例名/注释描述的「无 manifest、无 selection ⇒ 走物理仓路径」在 REQ-ROUTE-01 后**已不是可解析形态**
（路由恒 `FailClosed`）。按「新旧并排」留痕：**断言 `path.ends_with("repository_0001")` 一字未改**，
夹具改为播种完整 LC 权威 + 与权威一致的冻结快照（解析结果仍是成员的物理 checkout 路径），
并在用例内加了注释说明语义位移。**用例名保留**（未改名，以免破坏 CI 用例名过滤）；
建议后续由 Main 决定是否重命名为「logical 路由解析成员 checkout 物理路径」。

---

## 5. 最终窄跑结果

命令（单次，`flock /tmp/aria-cargo.lock`）：

```
flock /tmp/aria-cargo.lock -c 'cargo test --locked --lib web::coding_ws_handler -- --nocapture 2>&1 | tail -45'
```

结果：**见 §5.1**。

### 5.1 最终数字

**统一 lib 全量跑（Main 起的 `cargo test --locked --lib -- --nocapture`，日志 `/tmp/aria-full-lib-test.log`）中本簇取数：**

```
$ grep -cE "^test web::coding_ws_handler.* ok$"     /tmp/aria-full-lib-test.log
163
$ grep -cE "^test web::coding_ws_handler.* FAILED$" /tmp/aria-full-lib-test.log
13
```

⇒ **`web::coding_ws_handler` 簇：163 passed / 13 failed**（本轮基线 58 failed ⇒ 已收 45 条）。

13 条剩余逐条（见 §4）：`runner::tests` 4 + `plan_repair` runner 路径 3 + `runner_recovery::startup_window_compensation` 1 +
`runner_cleanup` http 族 5。

⚠️ **其中 `runner_cleanup` http 族 5 条是在统一跑启动之后才改的**（见 §1 该行注），
改动内容：夹具不再建 issue 维 `issue-shared-worktree.json`（快照 attempt 的路由是仓维，
issue 维记录在场会被判 `legacy_shared_worktree_present` ⇒ HTTP 409），
仓维记录与锁沿用共享夹具已按 attempt 播种的那份。
⇒ **这 5 条的修复未在统一跑里验证**，预期剩余 **8 条**，待收口的 `cargo test --all-targets` 确认。

**本簇窄跑（我自己的、单独一次）中间实测：`155 passed / 21 failed`**（当时只改了
`seed_compiled_work_item_fixture` / `plan_repair_fixture` / `context` 三处）。

---

## 6. 改动清单（`git diff --stat`，只含夹具）

```
$ git diff --stat -- src/web/coding_ws_handler/context.rs \
    src/web/coding_ws_handler/tests.rs \
    src/web/coding_ws_handler/tests/plan_repair.rs \
    src/web/coding_ws_handler/tests/plan_repair/ \
    src/web/coding_ws_handler/tests/runner_cleanup.rs \
    src/web/coding_ws_handler/tests/runner_recovery.rs
 src/web/coding_ws_handler/context.rs               |  46 ++++++++-
 src/web/coding_ws_handler/tests.rs                 | 105 ++++++++++++++++++---
 src/web/coding_ws_handler/tests/plan_repair.rs     |   6 +-
 .../coding_ws_handler/tests/plan_repair/support.rs |  78 +++++++++------
 .../tests/plan_repair/typed_sources.rs             |  16 ++++
 src/web/coding_ws_handler/tests/runner_cleanup.rs  |  66 +++++--------
 src/web/coding_ws_handler/tests/runner_recovery.rs |  35 +++++--
 7 files changed, 255 insertions(+), 97 deletions(-)

新增：src/web/coding_ws_handler/tests/gateway.rs（303 行）
```

⚠️ `git status` 里另有 3 个同目录文件显示为已改：
`src/web/coding_ws_handler/runner.rs`、`src/web/coding_ws_handler/tests/runner_recovery/startup_window_compensation.rs`、
`src/web/coding_ws_handler/tests_parts/manual_recovery.inc.rs` —— 这三个是**本轮早于本簇开工**就已存在的改动
（mtime 10:07–10:34，本簇首笔编辑在 16:12），**不是本簇改的**，故未计入上面的 diff stat；
本簇**未改** `runner.rs` 与 `startup_window_compensation.rs` 任何一行。

**改动全部落在夹具**：`tests.rs` 的夹具函数、`tests/**` 下的夹具文件、`context.rs` 的
`#[cfg(test)] mod tests` 夹具。**断言一行未改**（含 `assert_eq!(repository.id, "repository_0001")`、
`assert!(path.ends_with("repository_0001"))`、各用例的 `assert_eq!` / `assert!` 全文）；
**实现（`src/product/**`、`src/web/coding_ws_handler/runner/**`）一行未改**；
`src/web/workspace_ws_handler/**` 一行未动（兄弟 worker 独占）。

改动后各文件实际行数（均**不在** `large_file_guard.rs` 门禁表内）：

| 文件 | 行数 |
|---|---|
| `src/web/coding_ws_handler/tests.rs` | 1101 |
| `src/web/coding_ws_handler/tests/plan_repair/support.rs` | 705 |
| `src/web/coding_ws_handler/tests/plan_repair.rs` | 808 |
| `src/web/coding_ws_handler/tests/plan_repair/typed_sources.rs` | 397 |
| `src/web/coding_ws_handler/tests/runner_cleanup.rs` | 526 |
| `src/web/coding_ws_handler/tests/runner_recovery.rs` | 1222 |
| `src/web/coding_ws_handler/context.rs` | 515 |
| `src/web/coding_ws_handler/tests/gateway.rs`（新增） | 303 |

---

## 7. mtime 避让上报

本轮开工前对 `src/web/coding_ws_handler/**` 与 `src/product/logical_codebase/logical_test_fixture.rs`
逐个 `ls -la --time-style=+%H:%M:%S` + `find -mmin -20` 检查：**开工时无 20 分钟内被改动的目标文件**
（最近一次改动为 `logical_test_fixture.rs` 14:05、`plan_repair/support.rs` 10:14、`tests.rs`/`context.rs`
前一日 20:08）。本轮**未触碰** `src/product/**` 任何文件；`src/web/workspace_ws_handler/**` 由兄弟
worker 独占，本簇**一行未动**。

## 8. 行数门禁

本簇改动的文件均**不在** `large_file_guard.rs` 门禁表内（`tests.rs` / `plan_repair/support.rs` /
`plan_repair.rs` / `runner_cleanup.rs` / `runner_recovery.rs` / `context.rs` / 新增 `tests/gateway.rs`），
可自由加行；**未触碰** `tests/failed_review_recovery/runner.rs`（1242/1242 零余量），
**未修改** `large_file_guard.rs`。实际行数见 §6。


---

## 9. 追加收敛（`runner::tests` 4 条，per-LC 子树权威）

### 9.1 共享夹具扩能力（product 级 `logical_test_fixture.rs`，Main 已批准）

新增两个 API（**`snapshot_for` 的签名与行为未变**，现有 20+ 调用方不受影响）：

- `pub(crate) fn seed_logical_authority_for_lc(paths, project_id, issue_id, root, lc_id) -> (LogicalRepositoryId, RepositoryCheckoutId)`
  —— 在**真 per-LC 子树**（`LogicalCodebaseStore::for_lc(paths, lc_id)`）播种
  manifest + member + 主 checkout + selection + policy + **该子树的 `record.json`**
  + `repos.json` 投影（`logical_repository_id` / `primary_checkout_id` /
  `identity_schema_version=1`）。
  ⚠️ **`record.json` 必须自带**：`resolve_logical` **直读**
  `logical-codebases/{lc_id}/record.json`（不经 `migrate_legacy`）—— legacy 别名
  布局由 `migrate_legacy` 懒生成，**真 per-LC 子树没有这条捷径**。
  ⚠️ 该函数**断言 `lc_id != legacy_logical_codebase_id(project_id)`**（legacy 别名
  布局请用 `seed_logical_authority`）。
- `pub(crate) fn snapshot_for_in_scope(paths, project_id, logical_id, checkout_id, lc_id: Option<&str>) -> AttemptTargetSnapshot`
  —— 按 `for_lc(lc_id)` 读权威事实；`snapshot_for` 现在委托它（`lc_id = None`）。

### 9.2 web 侧接线（`src/web/coding_ws_handler/runner.rs`，3 个夹具）

| 夹具 | 用例 | 根因 | 改法 |
|---|---|---|---|
| `internal_pr_review_form_fixture` | `logical_codebase_attempt_skips_internal_pr_review_provider` / `logical_codebase_legacy_internal_pr_review_attempt_skips_provider` | 夹具把 issue 绑到 `logical_codebase_0001`（**真 per-LC id**）⇒ 该子树无 `record.json` ⇒ `NotFound { kind: "logical_codebase", id: "logical_codebase_0001" }` | 播种子树权威（`seed_logical_authority_for_lc`）+ 冻结快照（`snapshot_for_in_scope`）+ **仓维** worktree/锁（`upsert_repo_worktree` + `bind_repo_lock_to_attempt`）；⚠️ 该夹具同时服务两条 legacy 用例 ⇒ legacy 路径保持原语义 |
| `n5_runner_fixture` | `plan_coding_s2_runner_flows_to_final_confirm_with_plan_run` | `Store(NotFound { kind: "issue", id: "issue_0001" })`（夹具无 issue 记录、无 LC 权威、无快照、无仓维锁） | `seed_complete` + 快照 + 仓维锁 |
| `n5_sc_advance_parked_fixture` | `sc_advance_completion_parks_cleanly_when_next_unit_waits_on_out_of_bucket_work_item` | 同上 | 同上 |

### 9.3 追加后的最终窄跑

`cargo test --locked --lib web::coding_ws_handler`：**172 passed / 4 failed**
（接手时 170/6；基线 58 ⇒ **收 54 条**）。

**剩余 4 条**（**接手前后同样红**，不在本轮授权路径内）：

| # | 用例 | 归因 |
|---|---|---|
| 1 | `plan_repair::failed_review_recovery::coding_plan_repair_recovered_reviewer_retry_can_start_plan_repair` | 走 **runner** 路径 ⇒ gateway 由**生产工厂**（`state.gateway_factory()`）组装：能力源读 durable capability 记录、可用性门读真实健康快照、trust 源读真实 trust store —— 三者都不是 web 测试面可就地满足的 |
| 2 | `plan_repair::provider_start_failure_recovery::coding_plan_repair_provider_start_failure_recovery_legacy_can_retry_again` | 同上 |
| 3 | `plan_repair::provider_start_failure_recovery::…_modern_can_retry_again` | 同上 |
| 4 | `runner_recovery::startup_window_compensation::bad_tree_fail_closed_while_conflict_free_tree_resumes` | `WindowStartupFixture` 被同文件 20+ 条用例共用；改动面大，本轮未冒险动 |

### 9.4 行数实测（**Main 统一调登记，我不碰 `large_file_guard.rs`**）

```
src/web/coding_ws_handler/runner.rs                        2924   （登记上限 2830 ⇒ 需上调）
src/web/coding_ws_handler/tests.rs                         1101
src/web/coding_ws_handler/tests/plan_repair/support.rs      705
src/web/coding_ws_handler/tests/plan_repair.rs              808
src/web/coding_ws_handler/tests/plan_repair/typed_sources.rs 397
src/web/coding_ws_handler/tests/runner_cleanup.rs           526
src/web/coding_ws_handler/tests/runner_recovery.rs         1222
src/web/coding_ws_handler/tests/gateway.rs                  303   （新增）
src/web/coding_ws_handler/context.rs                        515
src/web/coding_ws_handler/tests/failed_review_recovery/runner.rs  1242  （登记 1242，零余量，**本轮未碰** ✅）
```
