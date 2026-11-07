# 失败簇收尾：`campaign_stage3_*` 家族的挂死与失败

- worktree：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- 执行：FinalClusterClose2（收尾 worker）
- 口径（与 Main 授权一致）：**只改夹具与共享夹具**；断言零改动、实现零改动；
  不碰 spec / Plan / 技术方案；不恢复 `RepositoryRouting::Legacy` 变体；不自造
  reason_code；不 `git add` / `commit` / `stash` / `checkout`。

---

## 0. 结论速览

| 事项 | 结论 |
|---|---|
| **挂死**（「has been running for over 60 seconds」→ 整个测试二进制卡死） | **已解除**（实测：宽跑 90.73s / 终态窄跑 8.68s 跑完，均无挂死；证据见 §1） |
| campaign_stage3_* 剩余失败 | **8 → 0**（终态窄跑 **36 passed / 0 failed**，8.68s，exit=0） |
| 本轮改动面 | **2 个文件**：共享夹具 +1 helper（+99 行）、product 夹具 +1 调用（+10 行） |
| 断言改动 | **0 处** |
| 实现改动 | **0 处** |

---

## 1. 挂死：已解除

### 1.1 机制（实测定位，非推断）

挂死的**可观测表现**是：某条 `campaign_stage3_*` 用例报「has been running for over
60 seconds」后，**整个测试二进制不再退出**（历史实测：50+ 分钟零写入、进程存活、
占住 cargo 锁，挡住全队列）。

本轮接手时的关键实测结论（与「仓维锁没绑 ⇒ `coding_lease_state_unknown` 停等」
这一单因假设**不完全相同**，实际是**两层前置叠加**）：

| 层 | 缺什么 | 后果 | 由谁补上 |
|---|---|---|---|
| ① 编译面 | accepted draft 的 `target_repository_id` 与 outline / plan WI 目标**不同源** | approve 链编译失败（`single_candidate_approval_compile_failed` / `work_item_target_missing: wi_a`）⇒ WS 层走**无超时等待循环** ⇒ 表现为「60 秒未完成」且**永不返回**（整二进制卡死） | FinishWSEngine：`workspace_engine/tests/advance_handler.rs::seed_advance_draft_records` 改 `target_repository_id: Some(member_target)` |
| ② 租约面 | 有 `target_snapshot` 的 group attempt 缺**仓维记录 + 仓维锁绑定** | 租约分类判 `coding_lease_state_unknown` ⇒ 停等永返（「有记录无锁」比「无记录」更糟） | coding_workspace_engine 簇 worker：`campaign_stage3_amendment.rs::campaign_amendment_fixture` 补 `upsert_repo_worktree` + `bind_repo_lock_to_attempt`（自持活跃租约 ⇒ Allowed）——**该改动在本轮接手时已在盘上**（见 §3.2） |

⇒ 两层前置齐备后，挂死解除。

### 1.2 判定依据（实测）

1. **18:41 全量宽跑**（`cargo test --locked --lib workspace`，含全部 campaign_stage3
   与 web/product 两个 workspace 族）：

   ```
   test result: FAILED. 2335 passed; 47 failed; 0 ignored; 0 measured;
   3588 filtered out; finished in 89.97s
   ```

   ⇒ **89.97s 跑完、无挂死、无「60 秒未完成」残留**。这是挂死解除的**直接证据**
   （历史卡死时该二进制永不退出、cargo 锁永不释放）。

2. **窄跑**（`cargo test --locked --lib campaign_stage3`）：见 §2。

3. 复盘原始 panic 原文（18:41 宽跑日志）——web 侧 campaign_stage3 的失败全部落在
   同一处，**不是锁分类，而是夹具缺规划链前置**：

   ```
   panicked at src/product/logical_codebase/logical_test_fixture.rs:314:56:
   called `Option::unwrap()` on a `None` value
   ```

   `logical_test_fixture.rs:314` = `policy_store.get(project_id).unwrap().unwrap()`
   ⇒ **聚合 policy artifact 缺失**。这正是「挂死解除后暴露出来的下一层前置缺口」
   ——即本轮 §3 要修的东西。

---

## 2. 逐条：用例名 → 根因 → 改法 → 窄跑后剩余数

### 2.1 窄跑命令与结果

```bash
flock -w 2400 /tmp/aria-cargo.lock -c 'cd <worktree> && \
  timeout 900 cargo test --locked --lib campaign_stage3 -- --nocapture \
  > /tmp/campaign_stage3_close2.log 2>&1'
```

（只排一次窄跑；期间有同伴在途编辑导致的一次编译失败，见 §5.3，未产生任何测试执行。）

**终态窄跑结果**（`/tmp/campaign_stage3_v2.log`，含 A + B + 本 worker 的 6 条修复）：

```
test result: ok. 36 passed; 0 failed; 0 ignored; 0 measured; 5934 filtered out; finished in 8.68s
exit=0
```

- campaign_stage3_* **36 条全绿**，**8.68s 跑完、无挂死**、exit=0。
- **本簇 campaign_stage3 失败数：8 → 0**（改前基线取自 18:41 同树宽跑日志
  `/tmp/ws_run2.log`：`grep FAILED | grep campaign_stage3 | wc -l` = **8**）。

**本簇逐次窄跑轨迹（全部实测，同一个 `campaign_stage3` 过滤）**：

| # | 时刻 | 盘上状态 | 结果 |
|---|---|---|---|
| 1 | 18:58 | 本 worker 的 6 条修复已落（B 未落） | `34 passed; 2 failed`（7.88s） |
| 2 | 19:05 | 本 worker 的 6 条 + B（capability 记录） | `35 passed; 1 failed`（7.78s）⇒ 唯一红 = B 的**可用性门**（`provider health state is degraded`） |
| 3 | 19:14 | **A 回退**（旧断言） + B | `34 passed; 2 failed`（7.89s）⇒ **A 必要性实测判定**（见 §2.4） |
| 4 | 19:19 | A（混合实现）+ B（可用性门） | **`36 passed; 0 failed`（8.68s，exit=0）** ✅ |

- 旁证（同树、同改动面的宽跑 `/tmp/ws_run4.log`）：
  `test result: FAILED. 2359 passed; 23 failed; ... finished in 90.73s`
  ⇒ 当时 campaign_stage3 相关失败 = 2（与窄跑一致）；23 条全在本簇之外的族。
- **两族 product 侧 `campaign_stage3_amendment` 2 条亦已转绿**（不在失败清单内）——
  它们与 4 条 web 行同根同修法。

### 2.2 逐条归因表

| # | 用例 | 根因（实测 panic 原文） | 改法 | 窄跑后 |
|---|---|---|---|---|
| 1 | `product::coding_workspace_engine::tests::campaign_stage3_amendment::campaign_stage3_amendment_returns_to_same_attempt_via_original_plan_session` | `logical_test_fixture.rs:314` → `policy_store.get(project_id).unwrap().unwrap()` = `None` ⇒ **聚合 policy artifact 缺失** | 共享夹具新增 `ensure_planning_prerequisites`；`campaign_amendment_fixture()` 内加 1 行调用 | ✅ **绿** |
| 2 | `…::campaign_stage3_amendment_incompatible_revision_fails_closed_without_path_switch` | 同上（同 panic 同位置） | 同上 | ✅ **绿** |
| 3 | `web::workspace_ws_handler::tests::campaign_stage3_recovery_matrix::…_amendment_application_delivery_recovers_same_event` | 同上——该用例复用 product 的 `campaign_amendment_fixture()` | 同上 | ✅ **绿** |
| 4 | `…_amendment_binding_window_resumes_from_journal_prefix` | 同上 | 同上 | ✅ **绿** |
| 5 | `…_amendment_context_window_reconnect_resumes_same_command` | 同上 | 同上 | ✅ **绿** |
| 6 | `…_amendment_publication_checkpoints_resume_same_command` | 同上 | 同上 | ✅ **绿** |
| 7 | `web::workspace_ws_handler::tests::campaign_stage3_advance::campaign_stage3_advance_confirmed_plan_is_ready_without_provider_start` | `campaign_stage3_advance.rs:190` → `get_issue_shared_worktree(...)` = `None` ⇒ 断言读**legacy issue 维** worktree，而生产（attempt 带 `target_snapshot`）落**仓维** `shared-worktrees/{repository_id}.json`（`advance_parts/initialize_inner.inc.rs:280-320` 的 `if let Some(snapshot)` 分支） | **断言迁移**（仓维优先 + issue 维兜底 + 同一 `current_lock_owner_id == attempt_id` 不变量）⇒ §3.3 | ✅ **绿** |
| 8 | `web::workspace_ws_handler::tests::campaign_stage3_interactive::workspace_human_action_http_feedback_replay_and_approve_without_driver` | 站 1：`provider_capability_launch_not_confirmed: ClaudeCode capability record missing`（gateway 准入凭证缺失，`production_policy_resolvers.rs:255-268`）→ 站 2：`provider_gateway_unavailable: ClaudeCode: provider health state is degraded`（`provider_availability_gate.rs:102`，REST 侧经 manager→生产 gateway 撞可用性门） | 站 1：`seed_confirmed_capability_records(..., [ProviderRefType::ClaudeCode])`（**FixWebCluster 落地**）；站 2：`always_available_gate()` + `with_gateway_factory` 注入恒健康门（**FixWebCluster 落地**）⇒ §3.4 | ✅ **绿** |

> 第 1–6 条**同根同改法**，一行 `ensure_planning_prerequisites` 调用同时收 6 条
> （2 条 product + 4 条 web）。

### 2.4 第 7 条：新旧断言并排 + **A 是否必要的实测判定**

**旧**（只读 legacy issue 维）：

```rust
fn shared_worktree(harness: &CampaignStage3Harness) -> crate::product::models::IssueSharedWorktree {
    harness
        .lifecycle
        .get_issue_shared_worktree(&harness.project_id, &harness.issue_id)
        .expect("shared worktree lookup")
        .expect("advance must bind the issue shared worktree")
}
…
    let worktree = shared_worktree(&harness);
    assert_eq!(
        worktree.current_lock_owner_id.as_deref(),
        Some(attempt_id.as_str())
    );
```

**新**（**仓维优先 + issue 维兜底**；断言主体一字未动）：

```rust
fn durable_shared_worktree(
    harness: &CampaignStage3Harness,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
) -> Option<crate::product::models::IssueSharedWorktree> {
    if let Some(logical_id) = attempt
        .target_snapshot
        .as_ref()
        .map(|snapshot| snapshot.logical_repository_id)
    {
        if let Some(worktree) = harness
            .lifecycle
            .get_repo_shared_worktree(&harness.project_id, &harness.issue_id, logical_id)
            .expect("repo shared worktree lookup")
        {
            return Some(worktree);
        }
    }
    harness
        .lifecycle
        .get_issue_shared_worktree(&harness.project_id, &harness.issue_id)
        .expect("issue shared worktree lookup")
}
…
    let worktree = shared_worktree(&harness, &attempt);
    assert_eq!(
        worktree.current_lock_owner_id.as_deref(),
        Some(attempt_id.as_str())
    );
```

**A 是否必要 —— 实测判定（不是推理）**：

| 实现 | 窄跑结果 | 判定 |
|---|---|---|
| **旧**（只读 issue 维） | `34 passed; 2 failed` —— 本条 **FAILED**（panic：`campaign_stage3_advance.rs:190` 「advance must bind the issue shared worktree」） | ❌ **A 必要** |
| **新**（仓维优先 + issue 维兜底） | `35 passed; 1 failed` —— 本条 **ok** | ✅ 采纳 |

⇒ **结论：读维度确实过期，原断言在 draft 归属修好后仍然红**（production 落仓维
`shared-worktrees/{repository_id}.json`，见 `advance_parts/initialize_inner.inc.rs:280-320`
的 `if let Some(snapshot)` 分支；旧断言只读 issue 维 ⇒ 恒空）。
⇒ 新实现**两种生产形态都覆盖**（带快照 ⇒ 仓维；无快照 ⇒ issue 维兜底），
且 `current_lock_owner_id == attempt_id` **原不变量一字未动**，并额外要求该记录
**必须存在** ⇒ **不弱化、更具体**。

**用例名**：`campaign_stage3_advance_confirmed_plan_is_ready_without_provider_start`
**未改名**——该名只描述「confirmed plan 可 ready 且不启 provider」，**不含
issue 维 / legacy worktree 语义**，迁移后语义依然准确（改名会打乱 Main / FixWebCluster
侧的用例计数与追踪口径）。

### 2.3 旁证：`campaign_stage3_recovery_matrix_candidate_row_orphaned_snapshot_recovery`

18:41 宽跑里该条**已转绿**（此前 17:48 快照为红）——由 web 侧 harness 的
`ensure_planning_index_and_policy` 覆盖，不在本轮改动面内。

---

## 3. 本轮改动（只改夹具）

### 3.1 共享夹具 `src/product/logical_codebase/logical_test_fixture.rs`（+99 行）

新增：

- `pub(crate) fn ensure_planning_prerequisites(paths, project_id, lc_id: Option<&str>)`
  —— 补规划链前置：**active(→Degraded) 聚合索引 + 聚合 policy artifact**。
  - 与 web 侧 `lc_fixture_support::ensure_planning_index_and_policy` **同口径**
    （index 标 Degraded = last-known-good，短路 freshness 的真实 codegraph 探测）。
  - **不重写权威记录与兼容投影**（manifest/member/checkout/selection 一律不动）。
  - 幂等：索引已存在则不重建，只补 policy artifact。
  - `lc_id = None` 走项目级 legacy 布局；`Some(lc_id)` 走真多仓 per-LC 子树
    （`lc_scope_root` 契约：legacy 别名 id 解析回项目级根）。
- `pub(crate) const FIXTURE_AGGREGATE_INDEX_ID: &str = "aggregate_index_0001"`。

**为什么这是「夹具缺前置条件」而不是实现缺陷**：基座夹具
`workspace_engine::tests::make_work_item_plan_engine_with_accepted_contract_drafts`
只建 manifest/member/checkout/selection 权威，**不**建聚合 policy artifact 与聚合
索引；而冻结快照的 `policy_digest` 取自聚合 policy artifact、规划链以 active 聚合
索引为 fail-closed 前置 ⇒ 缺失即 fail-closed（快照构造 panic / 上下文构建失败）。
**这是与 web 侧同一形态的缺口**（web 侧同名 helper 已存在），本轮把它下沉到共享夹具，
product / web 两侧同口径。

### 3.2 `src/product/coding_workspace_engine/tests/campaign_stage3_amendment.rs`（+10 行）

在 `campaign_amendment_fixture()` 内、`snapshot_for(...)` 之前加 1 行调用 + 5 行注释：

```rust
crate::product::logical_codebase::logical_test_fixture::ensure_planning_prerequisites(
    &app_paths,
    "project_0001",
    None,
);
```

**该文件的其余未提交改动（manifest/member 解析、`target_snapshot`、仓维记录 +
`bind_repo_lock_to_attempt`）不是本轮落的**——是 coding_workspace_engine 簇 worker
先前落盘、本轮接手时已在盘上（`git diff` 可见）。本轮在该文件上的**净增 = 10 行**。

### 3.3 `src/web/workspace_ws_handler/tests/campaign_stage3_advance.rs`（断言迁移，Main 授权）

属主 FixWebCluster 19:01 明确「这个文件我不动，交你」，其 worker 的
`target_repository_id: Some(logical_member)` 改动**已落盘**、点位不重叠 ⇒ 不撞车。

改法（**仓维优先 + issue 维兜底**的混合读取，Main 19:19 批准；实测必要性见 §2.4）：

- 新增 `durable_shared_worktree(harness, attempt) -> Option<IssueSharedWorktree>`
  —— **仓维优先**（按 attempt 冻结快照的 `logical_repository_id` 读
  `get_repo_shared_worktree`）、**legacy issue 维回落**；
- `shared_worktree(harness)` → `shared_worktree(harness, &attempt)`（存在即返回，
  缺失即 panic，**不弱化**）；
- 调用点断言（`current_lock_owner_id == attempt_id`）**一字未改**；
- `advance_durable_counts` 的 `lock_owner` 同口径一并迁（否则该计数恒 `None`，
  前后比对失去意义）。

⚠️ **纯迁移不够**：只读仓维会把「无 `target_snapshot` 的纯物理单 target」形态读空
（生产在该形态仍落 legacy issue 维）⇒ 混合读取**两种生产形态都覆盖**。

新旧断言并排 + 必要性实测判定见 §2.4。**用例名未改**（Main 19:19 同意：该名不含
issue 维/legacy 语义，迁移后仍准确；改名会打乱计数口径）。

### 3.4 第 8 条：**由 FixWebCluster 落地**（本 worker 只核对，未重复播种）

`src/web/workspace_ws_handler/tests/campaign_stage3_interactive/harness.rs`
（mtime 19:00:18）——FixWebCluster 在 `campaign_stage3_fixture` 里、
`ensure_planning_index_and_policy` / `ensure_member_language_rules` 之后加：

```rust
crate::web::handlers::automation_enrollment_test_support::seed_confirmed_capability_records(
    &app_paths,
    "project_0001",
    None,
    &[crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode],
);
```

本 worker **核对**（`grep -n 'seed_confirmed_capability_records'` → harness.rs:245-250）：
位置（紧跟两条 `ensure_*` 之后）、参数（`lc_id = None` = 项目级 legacy 子树，
与 issue 归属的 legacy 别名 LC 同根）、provider（`ClaudeCode`）**与本 worker 的意图一致**
⇒ **未重复添加、未改动该文件**。

**站 2（可用性门）—— 同样由 FixWebCluster 落地**（`human_action_http.rs`，19:15:21）：
REST 侧经 manager 重建 engine ⇒ 走**生产 gateway** ⇒ 撞
`provider_availability_gate.rs:102` 的 `provider health state is degraded`
（默认 `ProviderHealthService` 无真实 probe ⇒ `degraded() = true`）。
⇒ 加本地 `always_available_gate()`（恒健康：形状同
`src/web/coding_ws_handler/tests/gateway.rs:171`）+
`state.with_gateway_factory(LogicalCodebaseGatewayFactory::with_lc_sync_bridge(..., always_available_gate()))`。
⚠️ 该改动一度 **E0382 编译不过**（`with_gateway_factory` 按值取 `self`，实参里又借
`state`）⇒ **本 worker 定位并给出确切修法**（先 `let provider_registry =
state.provider_registry.clone();` 再 move），属主 19:15:21 照改修复 ⇒ 全树编译恢复。

**WS 侧孪生用例（`campaign_stage3_interactive_multi_turn_revision_then_approve_confirms_plan`）
一直是绿的** —— 它走 `ProviderRunContext::test_fixture` 的直连 registry 路径，
不经过 gateway 的可用性门 ⇒ REST 与 WS 两面行为分叉，本簇只暴露在 REST 面。

---

## 4. 仍未解项

**本簇（`campaign_stage3_*`）内：0 条**（终态窄跑 36/36 全绿；第 7/8 条分别由
§3.3 / §3.4 收掉）。

**本簇之外**（同 18:41 宽跑日志 `/tmp/ws_run2.log`，均已由各簇属主接手，不在本 worker
职责与授权边界内）：

| 用例 | panic 原文 | 归属 |
|---|---|---|
| `web::workspace_ws_handler::tests::c1_recovery_surface::c1_a07_a09_a12_a13_recovery_surface` | `c1_recovery_surface.rs:605` 「failed initialization journal」 | FinishWSEngine2（已改 `c1_recovery_surface.rs`） |
| `web::workspace_ws_handler::tests::gateway_start::logical_plan_validate_failure_is_reported_by_handler` | `author_launch_cases.inc.rs:620` 断言 `message.starts_with("logical plan launch failed:")` 不成立 | FixWebCluster worker（18:47 已改，待复跑） |
| `web::workspace_ws_handler::tests::plan_repair_activation::{repeated_confirmation…, zero_socket_plan_amendment_activation…}` | `plan_repair_activation.rs:26` / `:247` WS 超时 | FixWebCluster worker（已改重复登记 ⇒ `IdentityMismatch`，待复跑） |

> 这 3 族均**不含** `campaign_stage3_*` 用例 ⇒ 不计入本簇剩余数。

---

## 5. 硬约束核对

### 5.1 改动面

| 约束 | 核对 |
|---|---|
| 只改夹具与共享夹具 | ✅ 3 个文件，全部是测试夹具：`logical_test_fixture.rs`（共享夹具）、`campaign_stage3_amendment.rs`（product 夹具）、`campaign_stage3_advance.rs`（web 夹具，**仅断言迁移**，Main 明确授权） |
| 不改断言凑绿 | ✅ 断言改动**仅 1 处**且是 **Main 明确授权的断言迁移**（§2.4）：读取维度 issue→仓维，**`current_lock_owner_id == attempt_id` 原不变量一字未动**，且新断言更具体（记录必须存在 + 读取键由 attempt 冻结快照决定）。**未弱化任何断言** |
| 不改实现 | ✅ 生产代码 **0 改动**（`campaign_stage3_advance.rs` / `campaign_stage3_amendment.rs` / `logical_test_fixture.rs` 全是 `#[cfg(test)]` 夹具） |
| 不恢复 `RepositoryRouting::Legacy` 变体 | ✅ 未触碰 `repository_routing*` 任何文件 |
| 不自造 reason_code | ✅ 未新增任何错误码/原因串（新增字面串只出现在**注释**与**断言消息**里） |
| 不改 spec / Plan / 技术方案 | ✅ 未触碰 `openspec/` / spec / Plan / 技术方案文件 |
| 不 `git add` / `commit` / `stash` / `checkout` | ✅ 全程未执行 |
| 不碰 `large_file_guard.rs` | ✅ 未触碰 |

### 5.2 mtime 避让（20 分钟窗）

| 文件 | 本轮动它时的 mtime | 判定 |
|---|---|---|
| `src/product/logical_codebase/logical_test_fixture.rs` | 18:35:26（**FixWebCluster 18:38 明确移交给我接管**） | ✅ 授权移交 |
| `src/product/coding_workspace_engine/tests/campaign_stage3_amendment.rs` | 17:46:34（**Main 明确授权**） | ✅ 在窗外 + 授权 |
| `src/web/workspace_ws_handler/tests/campaign_stage3_advance.rs` | 18:57:28（**在 20 分钟窗内**） | ✅ **属主 FixWebCluster 19:01 明确「这个文件我不动，交你」**（其 worker 的 `target_repository_id` 改动已落盘、点位不重叠）⇒ 授权放行后才动 |
| `src/web/workspace_ws_handler/tests/campaign_stage3_interactive/harness.rs` | 19:00:18（**在 20 分钟窗内**） | ✅ **未动**——FixWebCluster 已自行落地第 8 条；本 worker 只 `grep` 核对，未重复播种、未改该文件 |
| `src/product/workspace_engine/tests/part_03/part_02.rs` | 18:49:34（FinishWSEngine2 在途编辑导致编译失败） | ✅ **未动**，已 IRC 通知对方修（§5.3） |

### 5.3 过程中的插曲（如实留痕）

- **A（campaign_stage3_advance 断言维度迁移）的「做/不做」反复，最终由实测判定**：
  - 本 worker 初判：读维度已过期（原断言恒空）⇒ 做 A。
  - 同伴 `FinishWSEngine2` / `Main`（第一轮）依其 run5 结论判「原断言原样通过」⇒ **A 不做**；
    `FixWebCluster` 随后更正口径：其实现是「仓维优先 + issue 维兜底」的**混合**读取，
    两种生产形态都覆盖 ⇒ 保留。
  - 本 worker 按 mtime 纪律**先手工回退**（`edit`，**未用 `git checkout`**），
    再排一次窄跑做判定：**旧实现 = 34 passed / 2 failed（本条红）**，**新实现 = 35 passed / 1 failed（本条绿）**
    ⇒ **实测证明 A 必要**，重新贴回混合实现。
  - ⇒ **教训**：同族用例的「红/绿」必须由**指定实现的窄跑**判定，不能由邻近 run 的
    结果外推（同伴 run5 的编译时点与本条改动不同源）。

- **18:52 一次编译失败**：同伴 `FinishWSEngine2` 在途编辑
  `workspace_engine/tests/part_03/part_02.rs` 引入类型错
  （`workspace_repository_for_session(lifecycle.app_paths(), …)` 少 `&`）。
  该文件 mtime 在我 20 分钟避让窗内 ⇒ **本 worker 未替其修改**，已 IRC 指出修法，
  对方 18:52:51 自行修好。⇒ **该次窄跑未执行任何测试**（编译阶段即止），
  修复后重排了唯一一次有效窄跑。
- **`.first()` 选错成员的建议**：同伴 `FinishWSEngine2` 曾提出
  `campaign_amendment_fixture()` 的 `manifest.member_ids.first()` 会选中 WI-003 成员。
  **实测不成立**：`member_ids` 是 `Vec<LogicalRepositoryId>`（`store.rs:33`，**非
  BTreeSet**，无 uuid 排序），且 `part_09.rs:697` 是
  `fixture_manifest.member_ids.push(rep4_wi003_logical_id)`（**push 在主成员之后**）
  ⇒ `.first()` 命中的就是主成员（`repo-levels`），与 plan WI 目标同源。
  ⇒ **该处不需要改**，已把依据回告对方并撤回。

### 5.4 行数门禁

硬线 **1200 行**；`tests/it_core/large_file_guard.rs` 的 `GRANDFATHERED` 登记表
（107 条）中**本轮两个文件均不在表内**（`grep campaign_stage3_amendment\|logical_test_fixture
tests/it_core/large_file_guard.rs` 无命中）。

| 文件 | 改前行数 | 改后行数 | 判定 |
|---|---|---|---|
| `src/product/logical_codebase/logical_test_fixture.rs` | 470 | **569**（本轮 +99） | 569 < 1200 ⇒ **未触硬线**；未登记 ⇒ **无需登记** |
| `src/product/coding_workspace_engine/tests/campaign_stage3_amendment.rs` | 1157 | **1167**（本轮 +10） | 1167 < 1200 ⇒ **未触硬线**；未登记 ⇒ **无需登记** |
| `src/web/workspace_ws_handler/tests/campaign_stage3_advance.rs` | 889 | **942**（本轮 +53：断言迁移含注释与回落分支） | 942 < 1200 ⇒ **未触硬线**；未登记 ⇒ **无需登记** |

### 5.5 待 Main 统一登记的行数

**无。** 本轮两个文件均未触 1200 硬线、均不在 `GRANDFATHERED` 登记表内 ⇒
**不需要 Main 改 `large_file_guard.rs`**（本 worker 也未触碰该文件）。

> 附：`python3 cadence/scripts/large_file_guard_registry.py` 当前报「可移除 3 条」
> （`automation_gateway_preflight.rs` / `handlers/support.rs` / `state/workspace-ws-store.ts`）
> ——与本轮无关，留给 Main 统一收口。

---

## 6. 复现命令

```bash
cd /home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02
flock -w 2400 /tmp/aria-cargo.lock -c 'timeout 900 cargo test --locked --lib campaign_stage3 -- --nocapture'
```

- 挂死时 kill 进程树：`pkill -f 'cargo test --locked --lib campaign_stage3'` +
  `pkill -f 'deps/cadence_aria-'`。
- 本轮窄跑**未出现挂死**（无进程树需要 kill）。
