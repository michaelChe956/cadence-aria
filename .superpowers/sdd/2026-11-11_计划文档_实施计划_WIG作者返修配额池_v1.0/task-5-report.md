# Task 5 报告：编译成功清零与诊断计数定性（工作包 4.1 / 4.2 / 4.3 / 4.4 / 5.5）

- 分支：`feat-wig-optimize-11-08`（worktree `.worktrees/feat-wig-optimize-11-08`）
- 计划书：`cadence/plans/2026-11-11_计划文档_实施计划_WIG作者返修配额池_v1.0.md` Task 5 段（:391-436）
- 基线：Task 1 `c37e9914` / Task 2 `770bf927` / Task 3 `183f5abc` / Task 4 `d6e767da` / 建域修正 `d892d9ba`

## ① 红态证据（Step 1）

- 测试：`src/product/workspace_engine/tests/author_rewrite_budget_pool.rs:618-725`
  `#[tokio::test] async fn compile_success_clears_auto_segment_only_not_feedback`。
- 运行：`flock /tmp/aria-cargo-wig.lock cargo test --locked --lib compile_success_clears_auto_segment`
- 结果（红）：**E0599 ×2** —— `no method named 'clear_author_rewrite_auto_segment' found for struct 'lifecycle_store::LifecycleStore'`
  （`src/product/workspace_engine/tests/author_rewrite_budget_pool.rs:624` 与 `:706` 两处调用点），
  `error: could not compile cadence-aria (lib test) due to 2 previous errors`，68.40s。
  与计划「Expected: FAIL（clear 方法不存在）」逐字一致。

## ② 绿态证据（Step 3）

| 命令 | 结果 |
| --- | --- |
| `flock /tmp/aria-cargo-wig.lock cargo test --locked --lib compile_success_clears_auto_segment` | **1 passed; 0 failed**（6363 filtered out），`compile_success_clears_auto_segment_only_not_feedback ... ok` |
| `flock /tmp/aria-cargo-wig.lock cargo test --locked --lib candidate_compile` | **25 passed; 0 failed**（6339 filtered out，既有编译成功 / 重驱动用例不回归） |
| `flock /tmp/aria-cargo-wig.lock cargo test --locked --lib author_rewrite_budget_pool` | **6 passed; 0 failed**（6358 filtered out，Task 3/4 家族含新用例全绿） |
| `flock /tmp/aria-cargo-wig.lock cargo test --locked --lib compile_failure` | **40 passed; 0 failed**（6324 filtered out） |
| `cargo fmt --all` + 行数复核 | **行数零变化**（1199/1190/262/725，未被顶破）；`git status` 仅本 Task 4 文件 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | **exit 0 / 0 error**（基线 0 error 维持） |
| `cargo test --locked --test it_core`（large_file_guard 显式跑） | **221 passed; 0 failed; 1 ignored**，exit 0（行数门禁绿） |
| `rustfmt --check --config-path rustfmt.toml --edition 2024`（4 个改动文件冗余复核） | **4/4 OK**（均为普通 `mod` 文件，`cargo fmt --all` 已覆盖） |

## ③ 逐条对照 Task 5 四个 Step 的落实清单

- **Step 1（先红）**：✅ 新增测试 `compile_success_clears_auto_segment_only_not_feedback`
  （`tests/author_rewrite_budget_pool.rs:618`），场景逐字对齐计划：
  自动 2（预算域两条未退扣减）+ 反馈 1（快照 `manual_repairs_remaining=2`、`accepted_feedback_turns=Some(1)`）
  ⇒ 真实编译成功（`engine.run_work_item_plan_compile()`，走 `record_candidate_compile_success` 真分支）
  ⇒ 权威余额 `Some(2)`；同门再反馈一次（镜像 `human_gate.rs` 同 CAS：remaining −1、turns +1）
  ⇒ `Some(1)`；`total_auto_rewrite_charges_ever == 2` 保留。
  另钉两条幂等边界：预算域 None ⇒ 幂等 Ok 且不建域（`:623-636`）；已清段重放 ⇒ Ok 零状态变化（`:704-725`）。
- **Step 2①（clear CAS 实现）**：✅ `src/product/lifecycle_store/workspace_policy_route.rs:164-198`
  `pub fn clear_author_rewrite_auto_segment(&self, session_id: &str) -> Result<(), ProductStoreError>`
  —— 与 Produces 签名逐字一致；同族 exclusive-lock CAS（`resolve_session_scope` 外层准入 + `with_exclusive_lock`
  + `with_optional_entity_live_write`，与 `refund_author_rewrite_charge` 同型）；
  None ⇒ 幂等 Ok（`:183-185`）；已清段（charges 空且 baseline==0）⇒ 幂等 Ok 不动 `updated_at`（`:186-191`）。
  锁内只取 `author_rewrite_budget.as_mut()`（`:182`）。
- **Step 2②（成功分支接线 + 崩溃窗注释）**：✅ `src/product/workspace_engine/compile/single_candidate.rs:278-284`
  —— `record_candidate_compile_success` 内**先**调 `clear_author_rewrite_auto_segment`（授权事实先落）、
  **再**走既有 `save_candidate_compile_status`（诊断计数面）；注释写明两文件非原子、崩溃窗只留诊断计数陈旧、
  读侧以预算域为准。
- **Step 2③（注释定性 + 未采用口径登记）**：✅ `src/product/workspace_engine/compile_failure_surface.rs:355-359`
  函数文档改为「编译成功＝清**该候选编译链上『平台自动重写』段**（诊断计数 + 预算域已计费消费）归零；
  门反馈段按 REQ-HGC-01/REQ-CG-02 既有语义不清零、不返还——整池回满额须显式改 REQ-HGC-01/REQ-CG-02，
  不在本 change（未采用口径）」；`:371` 内联注释同步改写；`:372` 的 `auto_rewrites_used: 0` 行为保留。
  全文无「整池清零 / 账本清零」残留（grep 复核见 ⑥）。
- **Step 4（提交）**：见最终回复 commit hash（显式列 4 个文件）。

### Q1「只清自动段、不清反馈段」如何被钉住

- **代码证据（写侧只动预算域一个字段）**：`workspace_policy_route.rs:164-198` 的
  `clear_author_rewrite_auto_segment` 在 session flock 内只取 `session.author_rewrite_budget.as_mut()`
  （`:182`），状态转移唯一调用 `models::author_rewrite_budget::clear_auto_segment`
  （`models/author_rewrite_budget.rs:192-195`：`auto_rewrite_charges.clear()` +
  `legacy_auto_rewrites_charged = 0`，**不触碰任何其它字段**）；`human_gate_snapshot` /
  `manual_repairs_remaining` / `accepted_feedback_turns` 在该 CAS 中根本不可变（函数体无任何引用）。
  门反馈段的预留 / 结算 / 退款语义仍在 `lifecycle_store/human_gate.rs` 的原 CAS，未被本 Task 修改。
- **测试证据（两侧同时验证）**：测试断言**自动段侧**——charges 空（`:658-661`）、baseline 0（`:662-665`）、
  total_ever 2 保留（`:666-669`）、权威余额 `Some(2)`（`:690-695`）；同时断言**反馈段侧**——快照在场且
  `manual_repairs_remaining == 2`（`:673-677`）、`accepted_feedback_turns == Some(1)`（`:678-682`）。
  「同门再反馈一次 ⇒ Some(1)」（`:697-703`）进一步证明 carry-forward 未被重置：若整池被清 /
  快照被重建为满额，该读数会是 3−1=2 或 3，而不是 1。

### Q2 崩溃窗的语义（Step 2②）

- **排序**：`compile/single_candidate.rs:278-284`——成功分支**先** clear CAS（session 记录，授权事实），
  **再** `save_candidate_compile_status`（candidate compile status 文件，诊断计数面），两文件非原子。
- **崩溃窗（两写之间崩溃）留下的只有「诊断计数陈旧」**：预算域自动段已清 ⇒ 授权余额已恢复正确；
  `auto_rewrites_used` 未随成功归零 ⇒ 只是 REQ-WSC-25 的观测计数短暂陈旧。**不存在**反向的
  「授权已扣但应清未清」失真——因为授权事实先落。
- **读侧如何以预算域为准**：所有执法点经 `workspace_policy_route::author_rewrite_balance`
  （`workspace_policy_route.rs:234-247`）组装权威余额，其 `net_auto_charged` 只在预算域 **None** 时才归并
  legacy `auto_rewrites_used`（`models/author_rewrite_budget.rs:117-131`）；预算域一旦建立（本场景必然已建），
  陈旧的诊断计数对余额**零影响**。诊断计数随下一次编译成功落盘自然对齐（`cleared_...:372` 置 0）。
- 该语义已写入 `compile/single_candidate.rs:278-283` 注释。

### Q3 清段与幂等三层的关系（显式核对）

- **事实核对**：
  1. **L1 键去重集被清空**（Task 3 评审 Minor-4）：属实——`clear_auto_segment` 清空
     `auto_rewrite_charges` ⇒ `with_preclaim_charge` 的键去重（`models/author_rewrite_budget.rs:159-163`）
     对**旧键**失忆。但旧键形如 `single_candidate_author:{session}:{attempt}`（attempt 单调递增），
     新预领必然生成新键；旧键重放要到达 preclaim 须路由层复用旧 attempt 序号，正常路径不可达。
  2. **SC 链「实际启动观察」判据恒不可达**（Task 3 评审 Minor-1）：属实且本 Task 未改变——
     ledger 条目 `started=true` 由预领占位写入（`tests/.../author_rewrite_budget_pool.rs:46-51` 复刻的
     Task 2 产物形态），`is_actual_launch_observation()` 仍只认 reservation 状态。
  3. **`provider_start_ledger_for_action` 无在位 Reserved 检查**：属实且本 Task 未改变——新预领仍会
     无条件覆盖旧 reservation，旧键扣减无人退。
- **本清段让边界变更轻还是更重**：**净变更轻，无加重**。
  - 编译成功清段使「悬空扣减」在成功边界**消失**（charges 清空 ⇒ `assert_no_dangling_charge`
    类不变量对旧键**恒真**、旧键不再占余额）——Task 3 评审 Minor-1/Minor-4 指出的
    「旧键扣减无人退」在编译成功后**不再是余额问题**（唯一残留是 total_ever 留痕，本就不参与余额）。
  - L1 键失忆的净效应 = 清段后旧键重放**若发生**会被记为一次新扣减（+1）——与任何一次真新重写的扣减
    同价，且因 attempt 键规不可达；对照不清段的世界，旧键重放被去重（0）但旧扣减**永久占额度**（−1
    直到退款/门结转）。两害相权，清段不更糟。
  - 与 Task 4 门快照结转清段（`carried_author_rewrite_budget`）**同一机制、同一边界**，未引入新面。
- **给 Task 6 的明确结论**：本 Task 不需要对幂等三层做补偿性修改；遗留缺口（ledger 无在位 Reserved
  检查、实际启动观察判据不可达）与清段**正交**，若要修属 Task 6/后续 change 的 L2 面决策，不因清段变 urgent。

## ⑤ 行数核验（含 compile_failure_surface.rs 净零证据）

| 文件 | 改前 | 改后 | 上限 | 判定 |
| --- | --- | --- | --- | --- |
| `workspace_engine/compile_failure_surface.rs` | 1199 | **1199** | 1200 | ✅ 净零：+5 行注释（`:355-359` 函数文档扩写）−3 纯空行（原 `:1118/:1127/:1137`，均位于 `mod tests` 内的测试函数间 / 断言间空行）；内联注释 `:371` 一换一 |
| `workspace_engine/compile/single_candidate.rs` | 1183 | **1190** | 1200 | ✅ +7（崩溃窗注释 6 + 清段调用 1） |
| `lifecycle_store/workspace_policy_route.rs` | 226 | **262** | 1200 | ✅ +36（方法 + 文档） |
| `workspace_engine/tests/author_rewrite_budget_pool.rs` | 540 | **725** | 1200（无登记上限） | ✅ +185（两个夹具 + 一个测试） |
| 零余量登记文件（tests.rs / workspace_single_candidate_tests.rs / compile.rs / recovery_task7.rs / routing_scope.rs / models/tests.rs / workspace.rs） | — | **未触碰** | 各自登记值 | ✅ `git status` 只有上述 4 文件 |

## ⑥ 范围自审

- 只做 Task 5 三落点 + 测试文件，**未触碰** Task 6 落点（`compile_failure_auto_rewrite.rs` 断言改写、
  `author_rewrite_budget_pool.rs` 的 Task 6 集成用例四件、验收记录）与任何 `openspec/specs/**`。
- 删除行仅 3 行纯空行（`compile_failure_surface.rs` 原 `:1118/:1127/:1137`），**零语句 / 零 `#[test]` /
  零断言 / 零 use / 零闭合括号**被删；`cleared_candidate_compile_status` 的 `auto_rewrites_used: 0`
  行为原样保留；既有断言零弱化（新测试全为新增断言）。
- 「整池清零」暗示全文复核：`grep -n "账本清零" compile_failure_surface.rs` ⇒ 0 命中（原 `:368` 已改写）。
- 预算常量未动：`RunBudgets::default()` 逐字不变；`large_file_guard` 登记表未动；
  无新增依赖；`Cargo.toml` / `Cargo.lock` / `web/package.json` 未改。

## ⑦ 遗留疑虑

1. **（Q3 结论复述）** L2 预领覆盖缺口（`provider_start_ledger_for_action` 无在位 Reserved 检查）
   与「实际启动观察」判据不可达：与清段正交，未加重、未修复；编译成功边界上旧键悬空扣减已随清段消失。
2. **崩溃窗内 `updated_at` 已动、诊断未对齐**的中间态窗口极短（同一同步函数内两次写），
   只影响观测面（REQ-WSC-25 流水的 success 行在崩溃窗外才追加），无授权语义影响。
3. 测试用直接 durable 写预置门快照 / 反馈轮（`preset_feedback_gate_and_auto_charges` /
   `accept_one_more_feedback_turn`），镜像 `human_gate.rs` 同 CAS 语义但未走真实 CAS——
   Task 6 集成验收如需端到端反馈路径覆盖，可补真实 feedback 流用例（本 Task 按计划书范围不做）。
