# Task 6 报告：既有测试改写核对 + 集成验收 + 门禁与留痕（工作包 1.6 / 2.3 / 2.5 / 3.5 / 4.5 / 5.2 / 7.1-7.7 / 8.1-8.4 / 6.1 / 6.2）

- 分支：`feat-wig-optimize-11-08`（worktree `.worktrees/feat-wig-optimize-11-08`）
- 计划书：`cadence/plans/2026-11-11_计划文档_实施计划_WIG作者返修配额池_v1.0.md` Task 6 段（:440-496）
- 基线：`e14e738c`（Task 1-5 全部落地后）

## ① 红态证据（四个新用例先红的原因）

计划书 Task 6 Step 2 对四个新用例的期望即「全部 PASS」（实现已在 T1-T5 落地，本 Task 是收口钉住），故红态分两类如实登记：

1. **开发期真实红（本 Task 实跑观察到）**：三个用例（`auto_rewrites_exhausted_stops_and_gates_with_zero` / `same_dependency_cycle_repeat_still_escalates` / `old_session_without_gate_local_facts_is_conservative_unknown`）曾红于 `persist escalated status: PathEscape("")`——测试夹具用 `PlanningCompileFailure::operational` 构造的失败缺 durable 身份（seal 前空 `failure_id` 与空 candidate refs，落盘路径为空串被拒）。补 `failure_id` + `source_revision_ref` / `plan_candidate_ir_ref`（取自 durable `previous` 状态）后转绿。属**测试夹具缺陷**，非产品缺陷。
2. **缺陷钉住红（对「建域多计 1」的实证，本 Task 实跑）**：把 `models/author_rewrite_budget.rs:154` 临时还原为 `legacy_auto_rewrites_used` 原值（去掉 `saturating_sub(1)`，即 `d892d9ba` 修正前形态）后，`auto_rewrites_exhausted_stops_and_gates_with_zero` 立即红：`建域基线扣掉本次（不多计 1）: left: 1, right: 0`（`tests/author_rewrite_budget_pool.rs:900`）；其逐次余额断言 `Some(2)`（第 1 次后）在多计 1 的世界会读到 `Some(1)`（净扣 = baseline 1 + 1 charge = 2），第 2 次后即 0 ⇒「3 次共用只跑得出 2 次」。实证毕即 `git checkout --` 还原（status 干净）。
3. **若在 T1-T5 之前写这四个用例会红的原因（分析）**：T1 前无预算域模型 / `available_author_rewrites`（编译错误）；T2 前无预领同 CAS 扣减（用例 1/2 的 B 池扣减事实不存在）；T4 前执法面读 A 池 / 诊断计数（用例 1 的 A 池 0 断言、用例 2 的升级交门快照 0 会红）；T5 评审 Minor-1 修复前反馈镜像在 None 分支漂移（用例 1 的真实 CAS 路径不受影响——本就用真实 CAS）。

## ② 绿态证据（匹配数 / 通过数）

| 命令 | 结果 |
| --- | --- |
| `flock /tmp/aria-cargo-wig.lock cargo test --locked --lib author_rewrite_budget_pool` | **10 passed / 0 failed**（6358 filtered out；含 4 个新用例 + 既有 6 个家族用例全绿） |
| 红态实证（还原缺陷后单跑 `auto_rewrites_exhausted_stops_and_gates_with_zero`） | **0 passed / 1 failed**（红因即「建域基线」断言）⇒ 还原文件后该用例随家族 10/0 绿 |
| `flock /tmp/aria-cargo-wig.lock cargo test --locked --lib tests_types::` | **4 passed / 0 failed**（6364 filtered out；`RunBudgets` `{1,12,3}` 钉住用例 PASS） |
| `git diff --stat -- src/product/work_item_plan_policy/types.rs` | **零输出**（types.rs 全程未改） |
| `cargo fmt --all` | **exit 0**（多次运行，含每次编辑后） |
| `flock /tmp/aria-cargo-wig.lock cargo test --locked --lib`（全量） | **6361 passed / 0 failed / 7 ignored**（0 filtered；基线 6357 + 4 新用例，142.30s） |
| `flock /tmp/aria-cargo-wig.lock cargo test --locked --test it_core`（含 large_file_guard） | **221 passed / 0 failed / 1 ignored**（21.71s，行数门禁绿） |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | **exit 0 / 0 error**（基线 0 error 维持，2m09s） |

## ③ 四个集成用例逐个：断言意图与可测缺陷

1. **`auto_rewrite_one_plus_feedback_one_leaves_single_balance_of_one`**（delta 场景「自动重写与反馈互扣同一余额」/ 工作包 1.6 / Review Focus ②）：真实 TAR 路由落 B 池扣减（键 `single_candidate_author:{id}:0`，baseline 0）→ 接力消费收敛「已扣已跑」→ 生产单一装配口读余额 `Some(2)` → 预置同 logical gate 门快照（`remaining 3, turns Some(0)`，复用既有 preset 夹具、不动自动段）→ **真实 human-gate CAS** `compare_and_reserve_human_gate_turn` 扣反馈 1（快照 3→2、turns Some(0)→Some(1) 同 CAS 原子）→ 权威余额 `Some(1)`；另断言自动段恰 1 条未退（分账可观测）+ `assert_no_dangling_charge`。**可测缺陷**：两套账本各自为政会给出「自动侧 2 / 反馈侧 2」两个数字（永远拼不出 1）；反馈 CAS 漏扣 / 双扣 / 非原子（remaining 与 turns 不同落）；预领扣减未落或被反馈路径误退。
2. **`auto_rewrites_exhausted_stops_and_gates_with_zero`**（REQ-WSC-21 既有场景「预算用尽后停并交人工门」/ 工作包 1.6 / Review Focus ③）：第 1 次走**真实路由**（决策时诊断计数已含本次 ⇒ 建域 baseline 必为 `used−1 = 0`，直断言 `legacy_auto_rewrites_charged == 0`）⇒ 余额 `Some(2)`；第 2/3 次按生产预领序列落键 `:1`/`:2`（legacy 参数按「决策先 +1」口径传 2、3，域已建走键追加分支），逐次断言 `Some(1)`、`Some(0)`；第 4 次（**新环指纹** ⇒ 排除同环重复分支）经 `candidate_compile_status` ⇒ `Escalate{repeated:1, remaining:0}`、phase Stopped、`remaining=Some(0)`、诊断计数停 3；路由面（第二干净会话 + 同口径 3 扣减）B 池 0 ⇒ 升级交门：快照 `manual_repairs_remaining == 0`（不补满额 3）、自动段同 CAS 结转清零、`total_ever` 恰 3、不委托（`assert_no_author_rerun_requested`）。**可测缺陷**：正是暴露「建域冻结基线多计 1」的那条（见 ①-2 实证）；也钉住「快照重置公式误回满额 3」「第 4 次虚增 total_ever」「耗尽仍委托」。**它现在为何能绿**：`d892d9ba` 把建域基线改为 `saturating_sub(1)` ⇒ 第 1 次后净扣恰 1（余额 2）、第 2 次后恰 2（余额 1）、第 3 次后恰 3（余额 0）——三条逐次断言与终态断言同时成立；修正前第 1 次后净扣即 2，`Some(2)` 断言先红。
3. **`same_dependency_cycle_repeat_still_escalates`**（REQ-WSC-22 判据不动 / 工作包 7.6 单测面）：cause 携带与 durable 相同的结构指纹再次失败 ⇒ `candidate_compile_status` 给 `Escalate{repeated:2, remaining:2}`——**余额尚有 2 也不得多跑**（指纹判据先于预算判据）；诊断计数停 1（不白耗）；落盘后通道拒绝（`route_compile_failure_to_rewrite` 返回 false，phase Stopped ⇒ 升级交门由调用方落）；零 author 委托事件；预算域保持 None（零预领扣减）。**可测缺陷**：共池改造若误把「继续重写」当默认（余额驱动盖过指纹判据）、或同环重复仍预领扣减白占额度。
4. **`old_session_without_gate_local_facts_is_conservative_unknown`**（工作包 5.2 / Review Focus ①的保守面）：预置旧会话形态（门快照 `accepted_feedback_turns == None`、无预算域）⇒ 生产装配口读余额 **None**（不是 Some(3)、不按 remaining 2 臆算）⇒ `candidate_compile_status(..., None)` 给 `Escalate{repeated:1, remaining:0}`（保守交门，不臆造满额）；诊断计数不 +1；落盘后通道拒绝 + 老会话零预算写入（不建域、快照 None 事实不被改写）。**可测缺陷**：老会话被补满额 3 / 被臆造成余额 0 之外的第三种口径 / None 被静默翻成 Some。

## ④ 工作包 2.4 逐条判定（既有断言改口径 vs 不动；已写入提交说明）

1. `compile_failure_auto_rewrite.rs:261-268`（`cycle.repairs_used == 0` + `run_history.repairs_used == 0`）：**Task 4 已改口径**（原 `cycle.repairs_used == 1` 钉旧池挪用行为）——1 条断言拆 2 条、锁 A 池两个计数点，**强度不降**；本 Task 核对确认，不再改动。
2. `:269-281`（B 池恰 1 条未退扣减）：Task 4 新增，把原「自动重写必须消耗（可观测的）预算」的观测落点从 A 池换到真实预算池并精确到条数，**强度升**；核对确认。
3. `:282-285`（`verification_count == 1`）：原样保留（REQ-WSC-25 可序列观测不回归）。
4. 同文件其余预算相关行仅 `:157` 夹具预置 `auto_rewrites_used: 1`（事实预置，非断言，**不动**）；`model_finding_in_verification` / `mixed_platform_and_model_findings` / `misrouted_auto_rewrite` 三条用例无池断言（**不动**）——grep 复核全文件 `repairs_used`/`manual_repairs_remaining`/`auto_rewrites_used` 断言仅上述各处。
5. 其它 SC 面：`conversational_gate.rs` 失败门补建断言 2→3 已由 Task 4 按「无快照 ⇒ 总额」契约改（两侧仍钉 turn 预留恰扣 1 与 settle-clear，非弱化）；`tests/candidate_compile_failure_surface.rs` 等调用点仅补传第四参（断言未动）；`conversational_gate.rs:530` 呈现读点未动（呈现层归块 1 G-2）。

## ⑤ 验收记录六块落位（`cadence/notes/2026-11-11_验收记录_WIG作者返修配额池_v1.0.md`）

| 块 | 落位（file:line） |
| --- | --- |
| ① 读写点清单（工作包 1.1） | `:10-59`（/tmp 清单 36 行原文 `:18-56` + 模块文档结论 `:57-59`） |
| ② 各 Task 定向与全量真实输出 | `:61-74`（含 T6 红态实证 `:72`） |
| ③ 基线对照数字（含待真机复测标注） | `:76-81`（`repairs_used 1→0`+B 池扣 1 `:78`；「最多 6 次 → 3 次共用」**待真机复测** `:80`；`RunBudgets` 逐字不变 `:81`） |
| ④ 真机验收清单（7.2-7.6，owner 回填） | `:83-93`（五项均「实际观测待回填」） |
| ⑤ 明确不改清单（7.7） | `:95-103` |
| ⑥ 归档前复核项（6.1/6.2/8.1-8.3） | `:105-113`（6.1 块 1 已归档 2026-11-03 + REQ-HGC-01 措辞确认 + 衔接建议 `:107`；delta 逐字核对 `:111`；specs 零改动 `:112`） |
| 已发生事实登记（契约级更正 / 评审修正 / 三条边界 / it_web 抖动） | `:115-140` |

## ⑥ T5 评审 2 Minor 的处置

- **Minor-1（夹具漂移）**：`accept_one_more_feedback_turn`（`tests/author_rewrite_budget_pool.rs:595-620`）与真实 CAS 逐分支对齐——`remaining == 0` ⇒ 断言红拒绝（原为盲减下溢风险），`accepted_feedback_turns == None` ⇒ 保持 None（原被翻成 `Some(1)`）；既有 T5 用例行为不变（Some(1)→Some(2)、2→1>0）。同时按评审要求补**真实反馈路径**端到端用例（用例 1 走 `compare_and_reserve_human_gate_turn` 真实 CAS）。
- **Minor-2（崩溃窗注释）**：`compile/single_candidate.rs:284` 补句「预算域 None 的老会话本 CAS 零写入、无新增中间态（幂等 Ok 不建域）」（+1 行，1191/1200）。

## ⑦ 三条既有边界核对结论（与台账一致）

1. **L2 预领覆盖缺口**（`provider_start_ledger_for_action` 无在位 Reserved 检查）：**属实、未修、归 owner 决策**——正常链路不可达（attempt 序号单调 + L2 CAS 挡重放），修属后续 change 的 L2 面。
2. **SC 链「实际启动观察」恒不可达**：**T3 评审 Minor-1 裁定「防御位非缺陷」，保留**；本 change 未假设 SC ledger 有实际启动观察（`assert_no_dangling_charge` 以 reservation `ProviderStarted` 为准）。
3. **L1 键失忆**：**T5 评审第 ⑩ 项裁定成立——清段与三边界正交且净变更轻**（清段使旧键悬空扣减在成功边界消失、L2 反被缓解），无需补偿性修改。
（均已登记进验收记录 `:129-133`。）

## ⑧ 行数核验（large_file_guard 口径）

| 文件 | 实测 | 上限 | 判定 |
| --- | --- | --- | --- |
| `workspace_engine/tests/author_rewrite_budget_pool.rs` | **1198** | 1200（无登记上限） | ✅ 本 Task 净增 473（725→1198，含夹具对齐 +4 用例 + 助手）；曾两度超限（1246/1226/1205）已按净零纪律压缩（提取 `persist_record`/`assert_no_author_rerun_requested` 助手、合并注释行、删 Part C 与 T4 用例重复的再读断言），压缩只删注释/重复断言，**零语句、零 #[test]、零必要断言被删** |
| `workspace_engine/compile/single_candidate.rs` | **1191** | 1200 | ✅ +1（崩溃窗注释句） |
| `tests/compile_failure_auto_rewrite.rs` | **462** | —（未登记） | ✅ 本 Task 零改动（Step 1 为核对落档） |
| 零余量登记文件 ×7（tests.rs 2893/2893、workspace_single_candidate_tests.rs 2055/2055、compile.rs 1651/1651、recovery_task7.rs 1295/1295、routing_scope.rs 1200/1200、models/tests.rs 1523/1523、lifecycle_store/workspace.rs 1729/1730） | 实测=登记值 | 各自登记 | ✅ 本 Task 未触碰 |
| `compile_failure_surface.rs` 1199/1200、`routing.rs` 1367/1391、`workspace_policy_route.rs` 274/1200、`routing_platform_rescue.rs` 429/1200 | 实测 | 各自上限 | ✅ 均绿 |

## ⑨ 范围自审

- 只改 2 个源文件 + 2 个文档：`tests/author_rewrite_budget_pool.rs`（Step 2 + T5 Minor-1）、`compile/single_candidate.rs`（T5 Minor-2 注释句）、验收记录（Step 5 新建）、本报告。`compile_failure_auto_rewrite.rs` **零改动**（Step 1 按指示为「核对 + 落档」——Task 4 已完成改写且强度不降，判定进提交说明与本报告 ④）。
- 未触碰：`openspec/specs/**`（diff 空）、其它 change 目录、本 change delta、`Cargo.toml`/`Cargo.lock`/`web/package.json`、零余量登记文件 ×7、`compile_failure_surface.rs`、并发会话文件（`provider_installation/runner.rs` 等）。
- 预算常量未动（`tests_types::` 4/0 + types.rs 零 diff）；`large_file_guard` 登记表未动；无新增依赖；禁 Docker / 禁 `-j 1` 遵守；cargo 全程 per-worktree 锁 `flock /tmp/aria-cargo-wig.lock`；无 `git add -A`（提交显式列文件）。
- 过程说明：一次编译失败（`PathEscape`）为夹具缺陷自纠；两次撞上 `cargo fmt` 展开超限，均以净零手法回落，未抬任何上限。

## ⑩ 遗留疑虑

1. **真机验收五项未执行**（工作包 7.2-7.6 + 「3 次共用」复测）——owner 执行后回填验收记录 ④ 节。
2. **L2 预领覆盖缺口**未修（owner 决策是否立项；见 ⑦-1）。
3. **块 1 呈现面迭代**（REQ-HGC-01 措辞升级）待块 1 后续迭代（验收记录 ⑥-6.1 已给衔接建议）。
4. `it_web` provider 网关并发抖动为仓库既有（631/0/17 稳定），本轮未重跑 `it_web`（沿用 controller 口径）；若归档前需要最新 `it_web` 数字由 controller 补跑。
5. [`include!` 文件不受 `cargo fmt` 管理] 跨链提醒已随验收记录登记——序 3/4/5 触达此类文件时必须单跑 `rustfmt --check`。
