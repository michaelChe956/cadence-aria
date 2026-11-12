# Task 4 报告：执法面改读统一余额 + A 池摘除

## ① 红态证据

- `cargo test --locked --lib author_rewrite_budget_pool`：5 用例 **2 passed / 3 failed**（Task 3 两个绿，Task 4 三个新用例红，判据未变）：
  - `platform_round_consumes_no_a_pool_delta`：`cycle.repairs_used` left 1 / right 0（平台轮仍扣 A 池）。
  - `a_pool_exhausted_still_allows_auto_rewrite`：A 池耗尽仍阻断委托（守卫仍读 `cycle.repairs_used`）。
  - `escalation_gate_snapshot_carries_unified_balance_and_settles_segment`：无门快照（守卫未读 B 池，仍委托）。
- `cargo test --locked --lib decide_route_reads_unified`：编译红 `E0061 this function takes 3 arguments but 4 arguments were supplied`（签名未变）。

## ② 绿态证据

均为实测（flock /tmp/aria-cargo-wig.lock，per-worktree 锁）：

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib author_rewrite_budget_pool` | 5 匹配 / 5 passed / 0 failed（3 新 + Task 3 两个无回归） |
| `cargo test --locked --lib decide_route_reads_unified` | 1 / 1 passed |
| `cargo test --locked --lib compile_failure` | 红点修复后全绿（首轮 39 passed / 1 failed：`conversational_gate` 钉旧公式的断言，已按 Step 3③ 改口径） |
| `cargo test --locked --lib`（最终） | **6355 passed / 0 failed / 7 ignored** |
| `cargo test --locked --test it_core` | **221 passed / 0 failed / 1 ignored**（含 `large_file_guard`） |
| `cargo test --locked --test it_web` | **631 passed / 0 failed / 17 ignored**（中途一轮并发下 `lcg_t13_workspace_streaming_legacy_four_provider_path_unchanged` 偶发 1 失败，单跑 3/3 通过、重跑全量 631/0，与本改动无关——Pi raw start capture 比较） |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | **exit 0 / 0 error**（首轮抓到 `max_auto_rewrites` 死代码 → 改为总额由调用方传入，见下） |
| `cargo fmt --all --check` | exit 0 |
| `rustfmt --check --config-path rustfmt.toml --edition 2024 review/tests_policy_routing.rs`（include! 目标） | exit 1，但差异全在我未碰的既有行（1/625/638/653/694/701/790/948/968），我的 316-317 行无差异——仓库既有状态 |

## ③ 七个 Step 落实清单

| Step | 落点 |
|---|---|
| 1 先红 | `compile_failure_surface.rs:938` 内联 `decide_route_reads_unified_balance_only`；`tests/author_rewrite_budget_pool.rs` 三个新用例 + 夹具 `preset_*` / `durable_balance` |
| 2 换源 | `compile_failure_surface.rs:145-` `decide_route(.., author_budget_available: Option<u32>)`（`unwrap_or(0)`，`0⇒Escalate{remaining:0}`，`>0⇒AutoRewrite{available-1}`）；`:294-` `candidate_compile_status(.., author_budget_available)`；`auto_rewrites_used` 仍 `AutoRewrite` 时 +1 作诊断计数；`max_auto_rewrites()` 保留为总额来源（`workspace_policy_route::author_rewrite_balance` 的 `RunBudgets::default().max_manual_repairs`） |
| 3① SC 改道臂 | `review/routing.rs` 改道臂 `manual_repairs_remaining: gate_budget`；结转清零经 `platform_gate_carried_budget`（`routing_scope.rs` `provider_start_ledger_for_action` 早返回分支）同 CAS 落 |
| 3② route_outcome | `review/policy_routing.rs` `human_gate_snapshot`：`is_platform_compile_failure_round(&context.findings)` ⇒ `context.author_budget_available.unwrap_or(0)`；否则保留「新 logical gate 初值」公式（注释写明评审驱动门是全新 logical gate，重置合法） |
| 3③ compile.rs | `ensure_human_gate_snapshot_after_compile_failure`：公式 3 行换 `self.author_rewrite_balance().unwrap_or(0)`；`author_rewrite_budget: carried_author_rewrite_budget(&store, &record)`（同 CAS 结转）；镜像回写。**净 −1 行（1651→1650）** |
| 3④ 已有快照 | 不动 `single_candidate_approval_gate` / `carry_forward_open_gate_budget`；`platform_gate_carried_budget` 在「快照值 ≠ 重算权威余额」（已被 carry-forward 接续）时不清账 |
| 4 A 池 delta | `review/routing.rs` delta 块：`platform_round` ⇒ `(repairs_used_delta, cycle.repairs_used) = (0,0)`，否则 `(1,1)`；`verification_count` 在 decision 侧合并，未动 |
| 5 rescue 守卫 | `routing_platform_rescue.rs` `rescue_platform_compile_failure_repair(.., author_budget_available)`：守卫 `author_budget_available.unwrap_or(0) == 0`；doc 改 B 池口径；`cycle` 参数改名 `_cycle`（不再使用）；唯一调用点 `routing.rs:515` 传 `self.author_rewrite_balance()` |
| 6 | 见门禁 + 行数 |
| 7 | 提交（显式列文件） |

新增命名谓词：`is_platform_compile_failure_round(&[ClassifiedFinding])`（判据 = 分类后非 Advisory 的 finding 全部带 `PLATFORM_COMPILE_FAILURE_FINDING_PREFIX` 且至少一条），`routing.rs` 与 `policy_routing.rs` 共用。

### 与计划书的偏离（均有理由）

1. **谓词可见性**：计划写 `pub(super)`；`policy_routing.rs` 是 `review::routing` 的兄弟模块，`pub(super)` 在 `routing_platform_rescue` 内仅到 `routing` 子树，调用不到。改为 `pub(in crate::product::workspace_engine::review)`，并把 `routing.rs` 的 `mod routing_platform_rescue` 改 `pub(super) mod`。
2. **新增 `models::author_rewrite_budget::carried_forward_budget_state`**（Task 1 文件，纯函数 + 单测）：老会话预算域未建时先冻 legacy 计数再清零，否则门快照建立后读侧再次归并 legacy 计数造成双计；compile.rs 零余量，不能内联。
3. **新增 `lifecycle_store::workspace_policy_route::{author_rewrite_balance(store, record, total), carried_author_rewrite_budget}`**：durable 读侧组装，所有执法点共用；`total` 一律由调用方传 `compile_failure_surface::max_auto_rewrites()`（总额唯一来源，计划 Step 2 要求保留），store 层不依赖引擎常量。
4. **未决预领（Reserved 未消费）保护**（计划书未覆盖，实测暴露）：见 ⑨-1。
5. **`rescue` 的 `cycle` 参数**：守卫换 B 池后不再使用，改名 `_cycle` 保持签名位置（调用点不变）。
6. **`compile_failure_auto_rewrite.rs:254` 断言改写**（原属 Task 6 Step 1，但 Task 4 Step 6 要求 `compile_failure` 全绿，改前此断言必红）：按 Task 6 Step 1 原文三连断言——`cycle.repairs_used == 0`、`run_history.repairs_used == 0`、B 池恰 1 条未退扣减，`verification_count == 1` 原样保留。强度不降（新增 B 池可观测断言）。

## ④ 余额事实源单一性逐点表

事实源统一函数：`lifecycle_store/workspace_policy_route.rs` `author_rewrite_balance(store, record)` = `available_author_rewrites({ total: RunBudgets::default().max_manual_repairs, human_gate_snapshot: record.human_gate_snapshot, author_rewrite_budget: record.author_rewrite_budget, legacy_auto_rewrites_used: legacy_b1_used(store, record.id) })`。**输入全部来自 durable**：`store.get_workspace_session()` 的记录 + `store.load_candidate_compile_status()`；**严禁**读 `self.session.author_rewrite_budget` 镜像（Task 3 评审 Minor-3）。

| 执法点 | 改前读 | 改后读 | 收敛？ |
|---|---|---|---|
| `decide_route` 余额 | `max_auto_rewrites − previous.auto_rewrites_used`（诊断计数） | 第四参 `author_budget_available`（调用侧 `engine.author_rewrite_balance()`，`compile/single_candidate.rs:237-242`） | ✅ |
| `candidate_compile_status` 调用侧（`record_candidate_compile_failure`） | 同上 | `self.author_rewrite_balance()` → `get_workspace_session`（durable record：门快照 + 预算域）+ `load_candidate_compile_status`（legacy） | ✅ |
| rescue 守卫（`routing.rs:515` → `rescue_platform_compile_failure_repair`） | `cycle.repairs_used >= max_repairs`（A 池） | `self.author_rewrite_balance()`（同一函数，同一 durable 行），`unwrap_or(0)==0` | ✅ **与 decide_route 同一函数同一事实源** |
| SC 改道臂快照（`routing.rs` ①） | `max_manual_repairs − history.manual_repairs_used` | `gate_snapshot_author_budget(store, session_id)` = 无未决预领 ⇒ `author_rewrite_balance`（同一函数）；有未决预领 ⇒ 既有初值公式（见 ⑨-1） | ✅（未决预领例外） |
| `route_outcome` 快照（`policy_routing.rs` ②） | 同上公式 | 平台轮 ⇒ `context.author_budget_available`（= 上一行同值）；非平台轮 ⇒ 公式不变 | ✅（非平台轮刻意不变） |
| 失败门补建（`compile.rs` ③） | 同上公式 | `self.author_rewrite_balance().unwrap_or(0)` | ✅ |
| 持久化钩子（`platform_gate_carried_budget`） | 无 | 以 `expected` 重算 `available_author_rewrites`，仅当「快照净余额 == 权威余额」才结转清零 | ✅ 同一公式 |
| A 池 delta 判别 | 无 | `is_platform_compile_failure_round(&findings)` | — |

`auto_rewrites_used` 现仅出现在：诊断计数 +1（`candidate_compile_status`）、`legacy_b1_used` 的 legacy 读侧归并（预算域未建时）、Task 5 清零。**没有任何执法判定直接读它当余额**。

## ⑤ A 池摘除完整性

- 轮内 `ReviewCycleState.repairs_used`：`routing.rs` delta 块 `platform_round ⇒ 0`（用例 `platform_round_consumes_no_a_pool_delta` 断言 `cycle.repairs_used == 0`）。
- 会话观测 `RunHistory.repairs_used`：`repairs_used_delta` 同为 0（同用例断言 `persisted.run_history.repairs_used == 0`）。
- `verification_count`：delta 块不碰；同用例断言 `cycle.verification_count == 1` 保留（`compile_failure_auto_rewrite.rs:276` 同断言原样保留）。
- 非平台轮：`(1,1)` 原样；`tests/single_candidate.rs:580/648/866`、`contract_prerevision.rs`、`contract_autorepair.rs` 的 `repairs_used == 1` 断言**一字未动且仍绿**。
- 守卫侧：`a_pool_exhausted_still_allows_auto_rewrite`（cycle.repairs_used=1 + B 余额 1 ⇒ 仍委托；B 余额 0 ⇒ 维持人工门）。

## ⑥ 既有调用点补参清单

| 位置 | 第四参 | 断言是否动过 |
|---|---|---|
| `compile/single_candidate.rs:237` | `self.author_rewrite_balance()` | — |
| `compile_failure_surface.rs:304` | `author_budget_available` 透传 | — |
| 内联 `binds_failure_to_candidate:840` | `Some(3)` | 否（remaining 2 仍成立） |
| `same_structural_cycle:902` / `:915` | `Some(3)` / `Some(2)` | 否（Escalate{repeated:2,remaining:2} 仍成立） |
| `different_cycle_structure:973/979` | `Some(3)` / `Some(2)` | 否 |
| `exhausted_budget:997/1011` | `Some(3)` / `Some(0)`（previous.used 仍设为 max，仅作诊断） | 否 |
| `structure_gate_rejection:1030` / `non_candidate_invalid:1046` | `None`（Gate 判定在余额之前） | 否 |
| `single_candidate_structure_failure.rs:161`（纯投影，仅取 `.0.failure`） | `None` | — |
| `tests/single_candidate/structure_parity.rs:431/557`（同上投影） | `None` | 否 |
| `GateSnapshotContext` 构造点：`routing_scope.rs`、`tests_policy_routing.rs:316`、`conversational_gate_revision_fingerprint.rs:48` | 新字段 `author_budget_available`（测试侧 `None`，其 findings 非平台前缀 ⇒ 公式不变） | 否 |

### 工作包 2.4 逐条判定（既有钉旧池断言）

- `compile_failure_auto_rewrite.rs:254-264`：**改口径**（A 池 0 + B 池扣 1 + verification_count 1）。
- 同文件 `model_finding_in_verification_still_lands_human_gate` / `mixed_platform_and_model_findings_still_land_human_gate` / `misrouted_auto_rewrite_opens_single_gate_and_reconciles_status`：**不动**（模型 finding / 混合轮判据为非平台轮；misrouted 不断言快照余额）。
- `conversational_gate.rs` `compile_failure_human_confirm_rebuilds_gate_snapshot_and_accepts_feedback`：快照余额断言 `max−manual_repairs_used(=2)` **改口径为统一余额 3**，下游「turn 预留后 remaining」两处随起点 2→3 由 1 改 2（`:620`、`:632`，仍断言「恰扣 1」）（该夹具无门快照、无自动段、无 legacy ⇒ 总额 3；`manual_repairs_used` 不再是补建公式来源），并新增「自动段已结转清零」断言；`attempts_used == 3` 断言原样。属计划 Step 3③ 明示的公式替换。
- `single_candidate.rs` / `contract_*` 的 `repairs_used == 1`：评审 finding 驱动，**不动**。

## ⑦ 行数核验

| 文件 | 改前 | 改后 | 上限 |
|---|---|---|---|
| `workspace_engine/compile.rs` | 1651 | **1650**（净 −1） | 1651 登记 |
| `review/routing.rs` | 1338 | 1354 | 1391 登记 |
| `review/routing_scope.rs` | 1200 | 1200（+2/−2：删 2 个纯空行；fmt 将 `use std::collections::BTreeSet;` 归位，行数不变） | 1200 |
| `compile_failure_surface.rs` | 1179（HEAD，含并发 970a1c2f 的 +25） | 1199 | 1200 |
| `compile/single_candidate.rs` | 1165 | 1183 | 1200 |
| `review/routing_platform_rescue.rs` | 326 | ≈430 | 1200 |
| `review/policy_routing.rs` | 162 | 176 | 1200 |
| `models/author_rewrite_budget.rs` | 362 | 398 | 1200 |
| `lifecycle_store/workspace_policy_route.rs` | 191 | 225 | 1200 |
| `tests/author_rewrite_budget_pool.rs` | 332 | 540 | 1200 |
| `lifecycle_store/workspace.rs`、`lifecycle_store/tests.rs`、`workspace_single_candidate_tests.rs`、`tests/recovery_task7.rs`、`models/tests.rs` | — | **本 Task 未触碰** | — |

净零手法删除的行：`routing_scope.rs` 删 2 个纯空行；`compile_failure_surface.rs` 删 7 个纯空行（顶层 item 间 3、测试 fn 间 4，另含并发提交后的重新压缩）。**未删任何语句 / `#[test]` / 断言 / `use` / 闭合括号**。
`cargo fmt --all --check` exit 0；`tests_policy_routing.rs` 是 `include!` 目标（`tests.rs:51`），我只加了 1 行字段；单跑 `rustfmt --check` 的差异全在我未碰的既有行（第 1/625/638/… 行），属仓库既有状态，我的第 316-317 行无差异。

## ⑧ 范围自审

- 未做 Task 5（编译成功清段）/ Task 6（集成验收）的落点；`cleared_candidate_compile_status` 未动。
- 未改 `RunBudgets::default()`、`large_file_guard` 登记表、`Cargo.*`、`openspec/**`。
- 既有断言零弱化：改动的两处（`compile_failure_auto_rewrite.rs:254`、`conversational_gate.rs:587`）均为计划点名的公式/池口径替换，且新增断言数 ≥ 删除数。
- 越出计划 Files 清单但为必需：`models/author_rewrite_budget.rs`、`lifecycle_store/workspace_policy_route.rs`、`single_candidate_structure_failure.rs`、`routing_scope.rs`（`GateSnapshotContext` 新字段与钩子，净零）、`tests_policy_routing.rs` / `conversational_gate_revision_fingerprint.rs`（结构体字面量补字段）、`structure_parity.rs`（调用补参）、`conversational_gate.rs`。

## ⑨ 遗留疑虑

1. **未决预领与门快照的冲突（本 Task 已规避，但值得 Main 知晓）**：若平台轮改道人工门时，持久化记录里存在 Reserved 未消费的预领扣减（Task 3 的崩溃窗形态），门快照以「含该扣减的余额」建立并同 CAS 清账，会吞掉 Task 3 对账要退款的那条扣减（`undelegated_reconcile_refunds_charge...` 实测红）。处置：`gate_snapshot_author_budget` 与 `platform_gate_carried_budget` 均用 `unconsumed_preclaim_key`（已改 `pub(crate)`）判未决——未决时快照走既有初值公式、账本不清，退款与接力消费照旧。代价：该形态下门快照为初值（3 − manual_used），读侧余额由 `快照 − 账本净扣` 给出，自洽。
2. **建域冻结基线疑似多计 1（Task 2 落地代码，未改，请 Task 6 复核）**：`with_preclaim_charge(None, key, legacy_used, ..)` 把 `legacy_used` 冻进 baseline，而生产链路里 `candidate_compile_status` 已先把本次重写的 +1 写进 `auto_rewrites_used`（`legacy_b1_used` 读到的是含本次的值）。全新会话第 1 次重写：baseline=1 + 1 条扣减 ⇒ net 2，余额 3−2=1，而 `decide_route` 报的 `remaining` 是 2。按此口径「纯自动 3 次」只能跑 2 次，Task 6 `auto_rewrites_exhausted_stops_and_gates_with_zero` 会暴露。本 Task 的断言刻意不依赖该算术（只断言「1 条未退扣减」「域已清」「余额 == 快照」）。修法倾向：建域时 baseline = `legacy_used.saturating_sub(1)`（本次扣减对应的 +1 不冻进基线）。
3. **Task 3 评审提示的条件性边缘仍在**：退款仲裁只看当前 `repair_reservation` 键；若 Reserved 预领被下一次 TAR 预领无条件覆盖（`routing_scope.rs` `provider_start_ledger_for_action` 无在位 Reserved 检查），旧键扣减无人退。本 Task 未触碰预领/覆盖语义。
4. 持久化钩子仅在 durable 路径（`persist_policy_route`）结转清账；无 store 的内存回退路径（`apply_policy_route_state`）不清，但执法面读 durable，不受影响。
5. `gate_snapshot_author_budget` / `author_rewrite_balance` 每次调用各做一次 durable 读（session + compile status），仅发生在路由/落门这类低频点。
