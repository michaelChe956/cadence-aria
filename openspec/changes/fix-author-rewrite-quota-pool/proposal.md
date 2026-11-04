# 提案：author 重写额度共池（选项 A：契约不动、执法改）

> 状态：草案 —— **owner 已拍板选项 A**（见文末「owner 决策记录」）。**本 change 只立项，不实施**：`tasks.md` 是待办清单，不是已完成记录。
> 依据：`cadence/notes/2026-11-03_会审_额度池语义_最终裁决_v1.0.md`（§5 独立缺陷、§6 选项 A）、`cadence/notes/2026-11-03_独立缺陷_自动重写额度共池未实现_v1.0.md`、`cadence/notes/2026-11-03_会审_额度池语义_Oracle方_v1.0.md`、`cadence/notes/2026-11-03_会审_额度池语义裁决_v1.0.md`。
> 前置：块1 change（`fix-automation-enrollment-provider-swap`）的 G-2 已收敛为「只改呈现」，并把本缺陷登记为**独立缺陷**（其 `design.md` 决策 6 明文「共池执法与预算隔离不在本 change 范围」）→ 本 change 承接该项。

## 背景

**契约语义（owner 2026-10-31 裁定 + REQ-WSC-21）**：author 重写额度 **3 次共用** —— 平台自动重写与 owner 反馈触发的重写**扣同一个池**，每轮消耗 1。

`openspec/specs/work-item-plan-single-candidate/spec.md:820`（REQ-WSC-21 正文）逐字：

> 预算 MUST 是**author 重写次数预算**（初值 `max_manual_repairs`，门卡上 owner 看到的数字同一口径），**平台自动重写与 owner 反馈触发的重写扣同一个池**，每轮消耗 1；编译成功 MUST 清零。`max_repairs`（机械返修池）MUST NOT 被当作自动重写预算。

`:837-840`（场景「两个预算池互不挪用」）逐字：

> **WHEN** 平台自动重写触发一轮 author 重跑
> **THEN** MUST 消耗 author 重写次数预算（`max_manual_repairs` 口径，与 owner 反馈 turn 同一池）；MUST NOT 消耗 `max_repairs`（机械返修池）

**代码现状（三层事实）**：

| 记号 | 名称 | 上限 | 账本 | 谁扣 | 执法点 |
|---|---|---|---|---|---|
**A** | 机械返修池 `max_repairs` | **1** | `ReviewCycleState.repairs_used`（轮内）+ `RunHistory.repairs_used`（会话观测） | `TriggerAggregateRepair` | `review/routing.rs:623-637`（`repairs_used_delta: 1`） |
**B1** | author 重写预算 · 平台自动重写 | **3**（`max_manual_repairs`） | `CandidateCompileStatus.auto_rewrites_used`（编译面独立文件） | 平台自动重写 | `compile_failure_surface.rs:301-314`（`used+1`） |
**B2** | author 重写预算 · owner 反馈重写 | **3**（同一池） | `HumanGateSnapshot.manual_repairs_remaining`（门面） | owner 反馈 turn 预留 | `lifecycle_store/human_gate.rs:195-201`（`−1`，同 CAS `accepted_feedback_turns += 1`） |

**缺陷有两层，比「两处账本未合并」更严重**：

1. **扣错池**：平台自动重写的执行入口是 `TriggerAggregateRepair` → `request_provider_run(WorkItemPlanSingleCandidateAuthor)`（`review/routing.rs:717-721` 逐字），它扣的是 **`max_repairs`（A 池，上限 1）** —— 正是契约 `:820` 明令「MUST NOT 被当作自动重写预算」的那一个池。既有测试把这个偏差钉死了：`src/product/workspace_engine/tests/compile_failure_auto_rewrite.rs:254-260` 断言 `cycle.repairs_used == 1`，注释逐字「**自动重写必须消耗（可观测的）预算**」→ 改成契约要求的 B 池时**必须同步改这条测试**。
2. **两处账本各记各的**：B1（编译面独立 JSON）与 B2（session CAS）之间**没有任何同步点** → 实际可用额度可能达 **6 次**（A 池 1 + B1 3 + B2 3），而契约语义是 3 次共用。

**现场症状（owner 2026-11-02 真机）**：同一张门卡上「自动修复次数已用完，需要你来定」与「还可自动修复 3 次」必然同时出现 —— 因为平台自动重写完全不扣 `manual_repairs_used`，人工余额公式 `max_manual_repairs − manual_repairs_used` 在无人反馈时恒为 3。交付说明 `cadence/notes/2026-10-31_交付说明_候选编译失败呈现与自动重写_v1.0.md:58` 早已白纸黑字登记「两处预算账本未合并」——**这是已知的落地差距，不是设计意图**。

## 目标

| # | 目标 |
|---|---|
1 | **平台自动重写改扣 `max_manual_repairs`（B 池）**：该轮 MUST NOT 再消耗机械返修池 A（`repairs_used` 不递增、不得因 A 耗尽而阻断自动重写） |
2 | **B1 / B2 合并为一个真实余额**：author 重写次数预算只有**一个权威余额**，平台自动重写与 owner 反馈互扣、每轮 1 次、如实递减；MUST NOT 出现两个互不相干的余额 |
3 | **定义「编译成功时两个字段谁清」**：编译成功 MUST 清零的是**该候选编译链上「平台自动重写」这一段的已计费消费**（B1 及其对应的余额消费）；**MUST NOT** 返还同一 logical gate 内已由 owner 反馈消费或已预留的门反馈额度（B2 段，按 REQ-HGC-01 / REQ-CG-02 既有接续与退款语义处理） |
4 | **余额只有一个可观测真值**：门卡上 owner 看到的「还可重写几次」与执法判定 MUST 是同一个数；MUST NOT 由展示端对两个计数取 max / min / 相加推算 |
5 | **额度语义回到 3 次共用**：实际可用额度从「最多 6 次」收回「3 次共用」，并在真机上验明 |

## 非目标

- **不改 REQ-WSC-21 的「扣同一个池」表述** —— owner 2026-10-31 裁定的语义不动；本 change 只把语义做进执法层
- **不改 REQ-HGC-01 / REQ-CG-02 的门反馈接续与退款语义** —— 同门 carry-forward、结构失败退款、hold / receipt 机制一律保持既有语义
- **不改机械返修池 A 的用途** —— A 继续服务「评审发现驱动的机械聚合返修」，只是不再被平台自动重写挪用
- **不改 REQ-WSC-22 的结构指纹判据** —— 「同一依赖环重复即停」与预算各自成立，互不覆盖
- **不改 REQ-WSC-25 的尝试可序列观测** —— 自动重写次数仍以诊断计数留痕
- **不做呈现层改造** —— 门卡文案 / 命名 / 余量呈现归块1 G-2；本 change 只保证**后端给出一个权威余额**，块1 由此可以（且必须）升级呈现口径
- **不做额度配置项** —— `max_manual_repairs` 初值仍是 3，不加用户可调项

## 范围

| 能力 | 变更 |
|---|---|
`work-item-plan-single-candidate` | MODIFIED REQ-WSC-21（补「编译成功清零的对象」= 谁清；补「互扣同一余额」「余额只有一个权威事实」「平台自动重写不得消耗机械返修池」三个可验收场景；**「扣同一个池」表述逐字保留**） |

## 契约缺口判定（delta 怎么写）

逐条核对 REQ-WSC-21 的现有 5 个场景：

| 现有场景 | 行 | 能否承载「谁清」规则 |
|---|---|---|
第一次失败就出提示 | `:822-826` | 否 —— 只管提示时机 |
提示不妨碍重写 | `:827-831` | 否 —— 只管「不得因提示而停」 |
预算用尽后停并交人工门 | `:832-836` | 否 —— 只管耗尽时的呈现与交门 |
两个预算池互不挪用 | `:837-841` | 否 —— 讲「扣哪个池」，**不涉及编译成功后清哪个字段** |
非候选无效类失败不宣称自动重写 | `:842-846` | 否 —— 只管失败族判定 |

**结论：有缺口。** requirement 正文 `:820` 只写了「编译成功 MUST 清零」，**没有说清「清的是哪个字段 / 哪一段消费」** —— 而「谁清」恰恰是 owner 选项 A 明文要求本 change 定义的那一项（最终裁决 §6 选项 A：「B1/B2 合并为一个真实余额；**定义「编译成功时两个字段谁清」**」）。

⇒ **写 `## MODIFIED Requirements`**：逐字复制主 specs 的 REQ-WSC-21 全文（含 5 个既有场景），只改两处必要措辞（①「复用既有机械返修通道」→「复用既有**返修路由**通道」，与「不得消耗 `max_repairs`」不再自相矛盾；②「编译成功 MUST 清零」→「编译成功 MUST 清零**该候选编译链上「平台自动重写」这一段的已计费消费**」），并新增 4 个可验收场景。

**「扣同一个池」表述逐字不动**（owner 裁定；`:820` 原句保留）。
**主 specs 不改** —— `openspec/specs/**` 由归档时 delta 自动同步，本 change 只提供 delta。

## 风险与取舍

| 风险 | 处置 |
|---|---|
改扣减池 = 行为变更，会让「自动重写一次就吃掉 1/3 人工额度」变得可见 | **这正是选项 A 的目的**（语义即 3 次共用）；owner 已拍板。真机验收要覆盖「自动重写 1 次 + owner 反馈 1 次 → 余额 1」 |
既有测试锁死了错误行为（`compile_failure_auto_rewrite.rs:254-260` 断言 `repairs_used == 1`） | **必须同批改写**为「扣 B 池、A 池不递增」；MUST NOT 削弱「自动重写必须消耗（可观测的）预算」这一断言的强度 —— 改为断言扣的是 B 池余额 |
「编译成功清零」若整池清零 → 静默推翻 REQ-HGC-01（同一 logical gate 内 carry-forward、不得每轮重置回默认）与 REQ-CG-02 | **本 change 取交集口径**：只清编译链上的自动重写段，不动门反馈段。整池清零口径需要显式修改 REQ-HGC-01/REQ-CG-02，**不在本 change 内暗改**（见 `design.md` 决策 3） |
B1 在编译面独立 JSON、B2 在 session CAS → 跨文件非原子 | 自动重写的扣减 MUST 迁到与门反馈**同一个 session CAS 预算域**；B1 降级为诊断计数（REQ-WSC-25 可观测），MUST NOT 继续当授权余额来源 |
旧记录缺新事实（老会话没有新的余额字段） | 一律 `#[serde(default)]`；旧记录按「余额未知」保守处理，MUST NOT 补默认满额（与块1「旧数据缺事实」的呈现口径一致） |
本 change 落地后，块1 已归档的 REQ-HGC-01「MUST NOT 展示共享余额（当前后端尚未提供该事实）」会过期 | **跨 change 顺序登记**：本 change 落地后须同步块1 的门卡呈现口径（两处账面事实 → 单一权威余额）。该同步**不在本 change 范围**（块1 change 独立归档），登记在 `tasks.md` 第 6 组与本文件「连锁要求」 |
「实际额度从 6 收回 3」会让正在跑的会话突然额度不够 | 会话内既有消费事实保留；余额按 durable 事实重算，MUST NOT 因合并而凭空多给或抹掉已消费的额度 |

## owner 决策记录（2026-10-31 / 2026-11-03）

| # | 决策 | 结论 |
|---|---|---|
1 | author 重写总额度语义 | **3 次共用**（owner 2026-10-31 裁定；契约 REQ-WSC-21:820/840） |
2 | 走选项 A 还是 B | **选项 A：把语义做进执法**（owner 2026-11-03 拍板）—— 契约不动、代码改；行为变更单独走本 change + 真机验证额度语义 |
3 | 「编译成功时两个字段谁清」 | **清编译链上的自动重写段（B1 + 其对应的余额消费），不清门反馈段（B2）** —— 见 `design.md` 决策 3；整池清零口径需改 REQ-HGC-01/REQ-CG-02，不在本 change |
4 | 契约表述是否要改 | **不改**。REQ-WSC-21 的「扣同一个池」表述逐字保留；「谁清」由本 change 的 delta 补场景承载 |
5 | 本 change 范围 | **只立项 + 执法层改造**；呈现层归块1 G-2；本 change 不实施（待 owner 批准后由实施链推进） |
