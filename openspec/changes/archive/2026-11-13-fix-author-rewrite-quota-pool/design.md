# 设计：author 重写额度共池（选项 A）

> 依据：`cadence/notes/2026-11-03_会审_额度池语义_最终裁决_v1.0.md`（§5 独立缺陷、§6 选项 A）、`cadence/notes/2026-11-03_独立缺陷_自动重写额度共池未实现_v1.0.md`、`cadence/notes/2026-11-03_会审_额度池语义_Oracle方_v1.0.md`、`cadence/notes/2026-11-03_会审_额度池语义裁决_v1.0.md`。
> 本文件是 OpenSpec 层的**决策记录**：背景 / 决定 / 理由 / 代价。落点与验收标准见 `tasks.md`；契约文本见 `specs/work-item-plan-single-candidate/spec.md` 的 MODIFIED REQ-WSC-21。
> **本 change 只立项，不实施** —— 本文件记录的是**待批准的设计决策**，不是已完成实现。

## 决策 1：把 B1 / B2 归一为**一个权威余额**（共池的执法形态）

**背景**：owner 2026-10-31 裁定的代码记录就在源码里（`src/product/workspace_engine/compile_failure_surface.rs:123-132` 逐字）：

> 这是同一个预算池，两类重写都扣它：
> - **平台自动重写**（本 change 的编译类失败自动返修）；
> - **owner 反馈触发的重写**（门内反馈 turn 预留时 `manual_repairs_remaining` 原子减 1，见 `lifecycle_store/human_gate.rs`）。
>
> 另一个池 `RunBudgets::max_repairs`（默认 1）是**机械返修**池……**不得**拿来当自动重写预算。

但代码上 B1 记在**编译面独立文件**（`CandidateCompileStatus.auto_rewrites_used`，`src/product/models/human_gate.rs:265`），B2 记在**门快照**（`HumanGateSnapshot.manual_repairs_remaining`），两者之间没有任何同步点。

**决定**：author 重写次数预算改为**单一预算域、单一权威余额**：

| 项 | 内容 |
|---|---|
总额 | `max_manual_repairs`（初值 3，来源不变） |
已计费 · 自动重写段 | 平台自动重写已扣的次数（**编译成功时清零**，见决策 3） |
已计费 · 门反馈段 | owner 反馈 turn 已扣的次数（沿用既有 hold / receipt / 退款语义） |
活动预留 | 既有 `UserBudgetHold`（REQ-CG-02） |
可用余额 | `总额 − 已计费（两段之和）− 活动预留` —— 沿用 `StructuralGovernanceState::available_user_budget`（`src/product/models/workspace.rs:513`）的既有模式，不新造第二套预算模型 |

两段消费**同属一个预算域、同一扣减序列**：自动重写扣 1 后，门反馈的可用余额同步少 1；反之亦然。MUST NOT 出现两个互不相干的余额，MUST NOT 由展示端对两个计数取 max / min / 相加推算余额。

**理由**：
- 会审三方一致的事实：**owner 裁定指 B1+B2**（A 被 `:131-132` 明令排除）；「两处账本」是实现落点的概括，不是「两份额度」的设计意图
- 现有代码里**没有能算出真实共享余额的公式** —— 正确算法需要「同预算域下已计费的唯一授权数 + 活动预留数」（互斥集合），展示端补不出来（`available_user_budget` 的既有用户账模式未覆盖独立的编译重写账）→ 只能由后端归一
- 「两态取源」（`auto_rewrite` 在场取它、否则取 `manual_repairs_remaining`）已被会审否掉：自动消费 1 + 人工消费 1 时正确剩余是 1，两态取源会给出 2 —— **错**

**代价**：
- 需要一个**跨文件非原子**的迁移：现有 B1 在编译面 JSON、B2 在 session CAS → 扣减必须迁到同一个 session CAS 预算域（决策 4）
- B1 `auto_rewrites_used` 降级为**诊断计数**（REQ-WSC-25 的「模型重写次数」仍要能算）→ 它不再是授权余额来源，任何读它当余额的地方都要改
- 旧记录缺新事实 → 一律 `#[serde(default)]`，按「余额未知」保守处理

## 决策 2：平台自动重写改扣 B 池；A 池只服务「评审发现驱动的机械聚合返修」

**背景**：平台自动重写的执行入口是 `TriggerAggregateRepair` → `request_provider_run(WorkItemPlanSingleCandidateAuthor)`（`src/product/workspace_engine/review/routing.rs:717-721`），而该 action 在 `routing.rs:623-637` 会生成 `repairs_used_delta: 1`（A 池扣次）并递增轮内 `repairs_used`。同时 `routing_platform_rescue.rs:124` 的守卫 `cycle.repairs_used >= RunBudgets::default().max_repairs` 决定「编译失败轮还能不能被平台救回」—— 也就是说**平台自动重写受 A 池限制、且消耗 A 池**。

既有测试把这个偏差钉死：`src/product/workspace_engine/tests/compile_failure_auto_rewrite.rs:254-260` 断言 `cycle.repairs_used == 1`，注释逐字「**自动重写必须消耗（可观测的）预算**」。

**决定**：
1. **平台自动重写路径不再产生 A 池扣次** —— 该路径的 `repairs_used_delta` / 轮内 `repairs_used` MUST NOT 递增；改由决策 1 的预算域扣 B（author 重写次数预算）
2. **「平台自动重写能否继续」的判据只看 B 余额** —— `routing_platform_rescue.rs:124` 的 A 池守卫 MUST 改为读 B 的可用余额（与 `decide_route` 的 `remaining` 同一事实源）
3. **A 池的用途不变**：评审 finding 驱动的机械聚合返修仍扣 A（`max_repairs`，上限 1）—— 本 change 只把「编译失败驱动的 author 重写」从 A 池里摘出来

**理由**：
- REQ-WSC-21:840 明文「MUST NOT 消耗 `max_repairs`（机械返修池）」；当前实现是**被测试锁定的契约违背**，不是注释过期
- 会审分歧 1 已裁定：`TriggerAggregateRepair` 就是「平台自动重写（重跑 author）」的执行入口（证据 A/B/C/D 四条齐备）
- A 池判「机械 repair 不重跑 author」在 SC 路径上本来就不准确 —— 本 change 摘掉挪用后，A 池回到「评审发现驱动的返修」这一真实用途上，契约表述与实现才对齐

**代价**：
- **必须同批改写既有测试**：`compile_failure_auto_rewrite.rs:254-260` 的断言口径从「`repairs_used == 1`」改为「**B 池余额减 1**、且 A 池 `repairs_used` 不递增」。MUST NOT 削弱「自动重写必须消耗（可观测的）预算」这一断言的**强度** —— 只换池，不去掉「必须消耗预算」这层断言
- A 池在 SC 路径上不再被自动重写消耗 → 评审驱动的返修轮次在同门内可能比现在多一次（此前被自动重写「蹭」掉了 A 的额度）。这是把两池职责还原的必然结果，须在真机验收中观察

## 决策 3：「编译成功时两个字段谁清」= **清编译链上的自动重写段，不清门反馈段**

**背景**：REQ-WSC-21:820 逐字只说「编译成功 MUST 清零」，**没说清清的是哪个字段**。当前代码清的是 B1：`compile_failure_surface.rs:353-369` 的 `cleared_candidate_compile_status` 写死 `auto_rewrites_used: 0`，注释逐字「编译成功＝账本清零：新事实从零计」；B2（`manual_repairs_remaining`）**不清**。合并成一个余额后，「两个字段谁清」必须显式定义 —— 否则实现者只能各写各的。

同时存在一条硬约束：REQ-HGC-01（`openspec/specs/human-gate-convergence/spec.md:11`）明文「同一 logical gate 内的修订轮次 SHALL carry-forward 预算（**不得每轮重置回默认**）」，REQ-CG-02（`openspec/specs/work-item-plan-conversational-gate/spec.md:23` 起）对同门预留 / 结算 / 退款另有规定。

**决定**（owner 选项 A 明文要求本 change 定义的那一项）：

| 字段 / 段 | 编译成功时 | 理由 |
|---|---|---|
**B1** `auto_rewrites_used`（编译链上的自动重写诊断计数） | **清零** | REQ-WSC-21:820「编译成功 MUST 清零」的对象是**候选编译链上的重写预算账**；新事实从零计 |
**B1 对应的余额消费**（预算域里的「自动重写段」已计费） | **清零**（该段消费解除） | 与 B1 同源；编译链结束即不再占用该段的额度 |
**B2** `manual_repairs_remaining` / 预算域里的「门反馈段」已计费与活动 hold | **不清零、不返还** | 同一 logical gate 内已由 owner 反馈消费或已预留的额度，继续按 REQ-HGC-01（carry-forward）与 REQ-CG-02（预留 / 结算 / 退款）的既有语义处理 |

**理由**：
- 两者**都成立**的交集只有这一条：REQ-WSC-21 的「编译成功清零」讲的是**编译链**；REQ-HGC-01 的「不得每轮重置回默认」讲的是**同一门内的接续**。整池清零会静默推翻 REQ-HGC-01 / REQ-CG-02 —— 而这两个 requirement **不在本 change 的修改范围**，不得暗改
- 会审 design-task-max 方的独立结论同向：「只清编译诊断段计数，不补回同门反馈额」；并明确「若 owner 选择成功即重置整个 B，则必须显式改 HGC / CG 合同，不能静默覆盖」
- 只清编译链段还保住了 REQ-WSC-21 自身的语义：**每次编译链拿到一份干净的自动重写额度**

**明确不采用的口径**：编译成功即整个 author 重写额度池回到 `max_manual_repairs`（满额）。**若 owner 要这个口径**，必须**显式修改** REQ-HGC-01 / REQ-CG-02 的接续与退款语义 —— 不在本 change 内暗改，本 change 的 delta 也不按该口径写。

**代价**：
- 「编译成功清零」从此**不是整池清零** → 契约正文必须把这层意思写清（delta 里逐字改这一句），否则实现者会按字面理解做错
- 编译链段可跨编译链重复使用（成功即归零）—— 这正是 REQ-WSC-21 原文的语义，但验收要显式钉住，避免被误读为「额度无限」

## 决策 4：自动重写的扣减必须与「委托 author 重跑」在同一 CAS 内完成

**背景**：B1 现状是「**先记消耗、后确认委托**」：`compile_failure_surface.rs:301-314` 在派生失败事实时 `used+1`，而真正是否委托 author 由后续路由决定；未委托时 `reconcile_auto_rewrite_not_delegated`（`compile_failure_surface.rs:682-714`）**只把 phase 改成 stopped，不减计数** → B1 可能 `remaining > 0` 却已停止（会审已登记为「R>0 不保证允许继续重写」）。B2 侧则是「同 CAS 扣减 + 建 hold + 建 grant」（`lifecycle_store/human_gate.rs:195-201` 及其后），与 provider-start 幂等账本一起落盘。

**决定**：
1. 平台自动重写的预算扣减 MUST 与「author 重跑预领/开始」在**同一个 session CAS** 内完成（同 REQ-CG-02 的 hold + provider-start ledger 模式）→ 崩溃恢复时不会出现「扣了但没跑」或「跑了没扣」
2. 「未委托」的对账 MUST 同时**回退该次扣减**（不是只改 phase）—— 否则会出现「没跑也占额度」
3. 扣减 MUST 幂等：同一 command / 同一 provider-start 键重放 MUST NOT 重复扣
4. `decide_route` 的 `remaining` 与执法判定 MUST 取**同一个**余额事实源（不得一个读 B1 诊断计数、另一个读预算域）

**理由**：
- 共池之后额度是**稀缺共享资源**，记账不准会直接变成「owner 少了一次重写」或「多给一次」→ 必须在 CAS 层做，不靠事后对账
- 会审 design-task-max 方的风险清单第 1、4 条：「候选 JSON 写入与反馈 session CAS 分离」「B1 在真实委托前计数；未委托仅改 phase，不回退计数」

**代价**：改动会触及 provider 预领 / 启动的幂等链路（`review/routing_scope.rs` 的 reservation 预领与 CAS 持久化）→ 回归面比「只改一个计数器」大，这是共池的固有成本

## 决策 5：呈现层必须跟着变成「一个数」，但呈现改造不在本 change

**背景**：块1 change 的 G-2 已把门卡呈现收敛为「**分别呈现两个账面事实**（B1 的 `auto_rewrite.remaining`、B2 的 `manual_repairs_remaining`）+「两处账本尚未合并」说明」，并明文「MUST NOT 展示『共享余额』」—— 这在**共池未实现**时是如实的。本 change 让共池**成为事实**，块1 的该条 requirement 随即过期。

**决定**：
1. 本 change 只保证**后端给出一个权威余额**（决策 1）→ 呈现层**有资格**展示单一余额
2. **呈现改造不在本 change** —— 归块1 G-2 的后续迭代；本 change MUST NOT 顺手改门卡文案 / 命名（会与块1 撞文件、撞契约）
3. **跨 change 顺序登记**：块1 已归档的 REQ-HGC-01（「当前后端尚未提供该事实 → MUST NOT 展示共享余额」）在本 change 落地后必须同步；本 change 的 delta **不**去改块1 的 delta 文件（不同 change，独立归档）

**理由**：呈现与执法分离是块1 已确立的边界（其 `design.md` 决策 6）；把呈现一起改会让两个 change 的契约互相覆盖，回归面失控。

**代价**：**本 change 落地后，块1 的门卡呈现会短暂地「有真实余额却仍显示两个数」** → 必须在归档顺序上登记（见 `tasks.md` 第 6 组），否则 owner 会看到「后端已共池、界面还说两处账本未合并」

## 决策 6：旧记录兼容与迁移

**背景**：既有 durable 记录里 B1 在 `CandidateCompileStatus`、B2 在门快照；新预算域是新增事实。

**决定**：
1. 新增预算字段一律 `#[serde(default)]` + `skip_serializing_if` → 老记录仍可反序列化
2. 老记录**缺新事实**时按「余额未知」保守处理：MUST NOT 补默认满额、不得让旧会话看起来还有 3 次；呈现口径与块1「旧数据缺事实」的既有规则一致（显示「预算历史不可用／共享余额无法确认」）
3. 已发生的消费事实**不得**因合并而凭空多给或抹掉：老会话的 B1 / B2 计数按「已计费」并入同一预算域
4. 迁移 MUST NOT 改写历史记录文件（只在新事实时按新口径计算）→ 迁移只发生在**读侧派生**，不产生写侧批量改写

**理由**：批量改写 durable 记录的风险远高于读侧派生；且额度语义只对「之后的消费」有意义。

**代价**：老会话的余额在首次参与新预算域时需要一次归并计算（幂等、按 turn / provider-start 键去重）

## 明确不改的部分

| 项 | 原因 |
|---|---|
REQ-WSC-21 的「扣同一个池」表述 | owner 2026-10-31 裁定；契约语义不动，本 change 只把语义做进执法 |
REQ-HGC-01 的同门 carry-forward / REQ-CG-02 的预留·结算·退款 | 决策 3 的「谁清」口径必须与它们共存；改它们是另一件事 |
机械返修池 A 的用途与上限 | A 继续服务「评审发现驱动的机械聚合返修」，只是不再被平台自动重写挪用（决策 2） |
REQ-WSC-22 的结构指纹判据（「同一环重复即停」） | 与预算各自成立，互不覆盖；本 change 不动 |
REQ-WSC-25 的尝试可序列观测 | 自动重写次数以诊断计数继续留痕（决策 1） |
门卡文案 / 额度命名 / 余量呈现 | 归块1 G-2（决策 5） |
`max_manual_repairs` 初值与可配置性 | 初值仍是 3；不加用户可调项 |
`manual_repairs_used_delta` 写入 `run_history` 的禁令（F-52） | 保持；反馈消费仍只记门内预算域 |

## owner 决策记录（2026-10-31 / 2026-11-03）

| # | 决策 | 结论 |
|---|---|---|
1 | author 重写总额度语义 | **3 次共用**（owner 2026-10-31） |
|2 | 走选项 A 还是 B | **选项 A：把语义做进执法**（owner 2026-11-03 拍板）—— 契约不动、代码改 |
| 3 | 「编译成功时两个字段谁清」 | **清编译链上的自动重写段（B1 + 其余额消费），不清门反馈段（B2）** —— 决策 3；整池清零需改 REQ-HGC-01/REQ-CG-02，不在本 change |
| 4 | 契约是否要改 | 只补「谁清」与可验收场景；「扣同一个池」逐字保留；主 specs 由归档时 delta 自动同步 |

## 决策带来的连锁要求

| 要求 | 原因 | 落地位置 |
|---|---|---|
既有测试 `compile_failure_auto_rewrite.rs:254-260` 必须同批改写，且**不得削弱**「自动重写必须消耗预算」的断言强度 | 它正在把「扣错池」钉死 | `tasks.md` 第 2 组 |
`routing_platform_rescue.rs:124` 的 A 池守卫必须改读 B 余额 | 否则自动重写仍受 A 池限制 | `tasks.md` 第 2 组 |
扣减必须与「委托 author 重跑」同 CAS，幂等 | 「扣了没跑 / 跑了没扣」= owner 直接少一次额度 | `tasks.md` 第 3 组 |
未委托时必须回退扣减 | 现状只改 phase 不减计数（`compile_failure_surface.rs:682-714`） | `tasks.md` 第 3 组 |
`decide_route` 的 `remaining` 与执法读同一余额事实源 | 避免「一处读诊断计数、一处读预算域」 | `tasks.md` 第 3 组 |
块1 的门卡呈现口径需在本 change 落地后同步 | 「两处账本尚未合并」的说明会过期 | `tasks.md` 第 6 组（跨 change 登记） |
MUST NOT 改块1 的 delta / 主 specs | 不同 change，独立归档；本 change 只在自己的 delta 里写 | `tasks.md` 第 6 组 |
