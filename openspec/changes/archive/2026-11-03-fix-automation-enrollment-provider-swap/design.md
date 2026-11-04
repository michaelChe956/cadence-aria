# 设计：自动化授权换代一步完成 + 人工门文案可行动化

> 完整技术方案见 `cadence/designs/2026-11-02_技术方案_块1_自动化授权换代一步完成_v1.0.md`（owner 2026-11-02 审阅通过）。
> 本文件是 OpenSpec 层的**决策记录**：背景 / 决定 / 理由 / 代价。落点与验收标准见技术方案与 `tasks.md`。
>
> **G-2 覆盖声明**：G-2（额度呈现）部分按 `cadence/notes/2026-11-03_会审_额度池语义_最终裁决_v1.0.md`（三方会审最终裁决）修订 —— **该裁决覆盖技术方案 §3.5 与本文件原「决策 5」的写法**，技术方案本身按约定不改，作为历史版本留痕。修订点见本文件「决策 5」「决策 6」与 `tasks.md` 第 6、7 组。

## 决策 1：换 provider 一次完成，放宽 terminated 路径的授权内容检查

**背景**：`supersede_terminated_enrollment`（`src/product/issue_automation_store_parts/cas_internals.inc.rs:671-681`）对 terminated 记录有一条硬门 —— 「options/target 与终止记录逐字段相等」。这导致 terminated 记录上直接带新 provider 的 Enable 被拒。

**后果**：前端把「换 provider」拆成三步（用旧 options 重开 → Disable → 用新 options 重开）。第一步触发 autopilot 生成，autopilot 建 plan 后 `bind_plan_ids_locked` 把 `policy_revision` 顶高一格（`cas_internals.inc.rs:55`，该处注释却写着「版本不递增——绑定不是换代」），第二步带旧版本号提交必然 CAS 失败，第三步永不执行 → provider 永远换不掉。且第一步会让**旧 provider 真的开始生成**。

**决定**：放宽该检查 —— `options` 允许不同（provider 可换）；`target` 与 `source` 的漂移管控保持不变。terminated 记录上的 Enable **一律走完整换代流程**（删光旧 WIG → 轮换 `prepare_intent_id` → 解绑 plan/session → binding 换代 → 旧代进 `previous` → 账本留痕 → 删两个 intent 文件），前端从三步塌缩为一次 Enable。

**理由**：
- owner 2026-11-02 给出的业务语义是「删了就是全部都删了。想换 provider 也一样，删了重来一遍，**不要做接续**」→ 换 provider 的本质是「删旧建新」，不是「在旧授权上原地改」
- 「删光旧 WIG 全闭包」与「换代留痕」代码**已实现**（`src/web/handlers/automation_enrollment_reenable.rs:23-73`、`cas_internals.inc.rs:688-763`），缺的只有那条内容检查
- 三步序列本身就是缺陷来源：它既制造了版本号竞态，又让旧 provider 产出。塌缩成一步后，竞态从根上消失

**代价**：
- 「授权不可原地改写」这条保护被移除 → 换 provider 现在能在一次操作内完成换代。**留痕未削弱**（换代动作全部保留），但「改写」与「换代」的边界由服务端从「内容比较」改为「路径判定」
- 既有契约测试需改写：terminated 路径 **2 条**（`attention_termination_tests.inc.rs:805-819`、`:1098-1112`）；另有 **3 条不得动**（`tests.rs:634-678`、`autopilot_orchestrator_parts/tests.inc.rs:232-265`、`automation_enrollment_tests_termination.inc.rs:135-170` —— 属 disabled→enable 路径，与本 change 无关）

**明确不做**：不引入「options 不同才换代」的判据 —— 那是为绕过该硬门而设计的旧路线残留，照做会复活它并导致每次临时开关都删 plan。「换代」的判据是 **`terminated_at` 是否存在**。

## 决策 2：删除旧 WIG 是换 provider 的前提，失败面如实告知

**背景**：owner 现场看到「换 provider 失败 → 旧 plan 产物没了、新 provider 没设上」。此前记录把它定性为「CAS 失败不回滚导致的意外损失」。

**决定**：**不承诺零副作用**。删旧 WIG 是「删了重来」的固有语义，失败面如实告知「上一次的工作已经被清空，但新的没建起来」+「点重试」。

**理由**：
- owner 裁定「删了就是全部都删了」→ 删除是**前提条件**，不是意外损失
- 「先删后建」是单 plan 不变量（同一 issue 同时只有一个存活 plan 代际）的实现方式；改为「先建后删」会短暂有两个 WIG 存活，与「全部删掉重建」直接矛盾
- 失败面不说清会让 owner 以为「什么都没发生」，在空态里反复找原因

**代价**：换 provider 失败时用户回到原点（状态清楚，但产物没了）。

## 决策 3：失败面按语义分诊，不按来源分诊

**背景**：两个完全不同的失败共用同一个 409 码与同一句 `automation enrollment revision conflict` —— 版本号过期（该等一下重试）与运行中不许改 payload（该先终止）。owner 读到后无法分辨。实测该错误还承载另外 7 类语义（共 9 类、33 个构造点）。

**决定**：引入**原因判别位**（`ConflictKind`），按语义分流两个新稳定码（`automation_enrollment_payload_immutable` / `automation_enrollment_revision_stale`），其余 7 类**落回旧码**。机器码与原始 detail 降级到次要行。

**理由**：
- 「等一下重试」与「该先终止」是两类完全不同的用户动作，必须让 owner 能分辨
- **旧码不能改名**：它被终止自愈逻辑钉住（`web/src/components/chat-workspace/cockpit/automationTerminationConflict.tsx:17`）→ 改名会静默失效。改用判别位 + 新码，终止路径**一行不改**

**代价**：≥45 处判别位穿透（机械改动，`current_revision:` 全仓 45 行）；前端 5 处测试夹具写死了英文 message，需同步。

**文案准确性约束**：终止入口按钮只在「当前会话 + server 托管 + `workspace_type === "work_item_plan"` + 未终止」时出现 → 失败文案**不得**声称终止入口在所有落屏可见。

## 决策 4：过期门卡按 durable 停点分流

**背景**：过期门卡统一挂「此时只能终止此门后重新发起」+「终止此门」按钮，而 `stopped_needs_human` 的真实入口是**页面底部输入框 +「继续自动化」**。已修的只有「已确认完成」一类。

**决定**：新增单一事实源（`gate-stale-guidance.ts`），按 durable `session_status` 分六类给文案与真实入口；「终止此门」只在真的没有别的出路时（推进到别的门）保留为可选出口。分流主键是 `session_status`（能说出「这一轮归谁管」），不是 `action_block_reason`（只说明「这张卡过期了」）。

**理由**：
- 门卡指错路比没有路更糟 —— owner 照做会终止掉还能续的这一轮
- 「已确认完成」已修对，其余三类仍错 → 说明分流表要覆盖全部停点，不是补一句文案

**代价**：需要「本轮是否可继续」这个事实 —— 现有 `planContinueControl`（真实符号，`web/src/pages/ChatCockpitPage.tsx:756-769`；仓库中**不存在** `canContinuePlan` 这个符号）是页级局部变量，门卡组件拿不到。**决定新增 WS 会话状态字段**（`plan_continue_available`），真实来源是 HTTP control 投影。

**明确不做**：门卡余量数字实时化（快照语义，另立项）。

## 决策 5：三个额度分别命名，两个账面事实分别呈现（2026-11-03 会审最终裁决 §4 修订）

**背景**：同一张门卡上「自动修复次数已用完」+「还可自动修复 3 次」自相矛盾。2026-11-03 三方会审（controller + oracle + design-task-max）核出的真实结构是：**一个语义池（author 重写次数预算，上限 3 = `max_manual_repairs`）、两处未合并账本、两套执法**：

| 记号 | 名称 | 上限 | 账本 | 谁扣 |
|---|---|---|---|---|
| A | 机械返修池 `max_repairs` | 1 | `ReviewCycleState.repairs_used` / `RunHistory.repairs_used` | `TriggerAggregateRepair` |
| B1 | author 重写预算 · 平台自动重写 | 3（`max_manual_repairs`） | `CandidateCompileStatus.auto_rewrites_used` | 平台自动重写 |
| B2 | author 重写预算 · owner 反馈重写 | 3（同一池） | `HumanGateSnapshot.manual_repairs_remaining` | owner 反馈 turn 预留 |

**决定**（取代本 change 原「至多一个余量数字 + 文案承认共池」的写法）：
- **命名**：三个额度各用各的名字（「平台自动重写」/「人工返修」/「你已说明 N 轮」）；**A 池（机械返修）不上界面额度名**
- **余量**：**分别呈现两个账面事实**（B1 的 `auto_rewrite.remaining`、B2 的 `manual_repairs_remaining`），各自标注来源与用途；MUST NOT 任选其一当「共享余额」、MUST NOT 合成猜测值、MUST NOT 隐藏另一事实后宣称共享余额已确定
- **共享余额**：**仅在后端提供同域权威事实时才展示**；当前未提供 → 界面**不显示**「共享余额」，只显示两个来源各自的账面事实 + 「两处账本尚未合并」的说明
- **「已用完」**：MUST NOT 由 `repair_budget_exhausted` 直接断言（该 trigger 可由 A 池耗尽、cycle initial/verification 的结构封闭等路径产生）→ 中性文案「本轮评审自动返修已停止，需要你决定下一步。」，并按 `flowKind` / 触发路径分流
- **共池表述**：只用说明性文字（「平台自动重写与你的反馈共用同一份额度上限」），MUST NOT 承诺算术合并值；requirement 与界面文案 **MUST NOT 宣称「已共用一个池」**

**理由**：
- 现有代码里**没有能算出真实共享余额的公式**：正确算法需「同预算域下已计费的唯一授权数 + 活动预留数」（互斥集合），展示端补不出来（`StructuralGovernanceState::available_user_budget` 的既有用户账模式未被独立的编译重写账覆盖）
- 会审否掉了「两态取源」（`auto_rewrite` 在场则取它、否则取 `manual_repairs_remaining`）：自动重写消费 1 + owner 反馈消费 1 时正确剩余是 1，两态取源会给出 2 —— **错**
- B1 在结构失败时**不撤销计数**（`compile_failure_surface.rs:682-714` 只把 phase 改为 stopped、不减计数）→ B1 的 R 可能 > 0 却已停止，**R>0 不保证允许继续重写**
- 门反馈有 hold / receipt / 退款机制（REQ-CG-02:37-75）→ `3 − run_history.manual_repairs_used` **不等于**真实余额
- 同一块门卡上两个数字打架的正解是「各自标清来源 + 说明账本未合并」，而不是任挑一个冒充总额

**代价**：门卡上会出现两个来源各自的数字 —— 这是**如实呈现**，不是新的矛盾；代价是「到底还能再写几次」这个问题**暂时无法在界面上给出答案**（真实共享余额需要后端新事实）。

**顺带（不变）**：底部停机卡现在断言「人工说明次数已用尽」—— 与 durable 事实相反（实测当时额度还剩 3 份）→ 改为指向「写说明 + 点继续自动化」。

## 决策 6：G-2 收敛为「只改呈现」，共池执法与预算隔离不在本 change 范围

**背景**：契约（`work-item-plan-single-candidate/spec.md:820`/`:840`）明文要求 author 重写「扣同一个池、每轮消耗 1」，而代码把它拆成了两处账本 + 两套执法；且平台自动重写的执行入口 `TriggerAggregateRepair` → `request_provider_run(WorkItemPlanSingleCandidateAuthor)`（`review/routing.rs:717-721`）扣的是 **`max_repairs`（A 池，上限 1）**，而不是契约要求的 `max_manual_repairs`（B 池，上限 3）—— 这条偏差已被既有测试 `compile_failure_auto_rewrite.rs:234-260` 锁定（注释逐字：「自动重写必须消耗（可观测的）预算」）。因此实际额度可能达 **6 次**（A 池 1 + B1 3 + B2 3），而契约语义是 3 次共用。

**决定**：G-2 的范围**收敛为「只改呈现层」** —— 不动账本、不动执法点、不动 CAS。共池执法与预算隔离**不在本 change 范围**，作为**独立缺陷**登记（`cadence/notes/2026-11-03_独立缺陷_自动重写额度共池未实现_v1.0.md`；本次会审新增两条：不只是「未合并」，还有「扣错池」；以及「实际额度可达 6 次」）。

**理由**：
- 把语义做进执法是**行为变更**（额度从最多 6 次收回 3 次），需要单独的 OpenSpec change + 真机验证额度语义，还要同步改写锁定了当前实现的既有测试 —— 不是呈现层能承载的
- 会审 §6 的唯一待拍板项（「3 次共用 vs 最多 6 次」）**尚未由 owner 拍板**；拍板前擅自改执法等于替 owner 决策
- 呈现层在两种裁定下都要改（选项 A 做进执法、选项 B 接受现状改契约）→ **G-2 本身不需要重拍**

**契约边界**：G-2 只修**呈现语义**；**REQ-WSC-21 的预算隔离与共池执法不在本 change 范围**，**不得因 G-2 完成而把 REQ-WSC-21 标记为已修复**（验收口径见裁决 §4.1 场景 6：共享权威尚不存在时**仅验收呈现修复**）。

**明确不做**（不在本 change）：把平台自动重写改扣 `max_manual_repairs`、合并 B1/B2 两处账本、定义「编译成功时两个字段谁清」、改写 `compile_failure_auto_rewrite.rs` 的既有断言。

## 明确不改的部分

| 项 | 原因 |
|---|---|
`bind_plan_ids_locked` 的 `policy_revision += 1` | 已不在用户路径上，但仍是并发语义的一部分。**只修它那两处自相矛盾的注释** |
| `source` / `target` 的漂移管控 | 不变（terminated 路径同样要求 `source`/`target` 与终止记录逐字段相等） |
REQ-WIGA-02（**enabled 记录**上的异 payload fail-closed） | 不变（本次只在 **terminated** 路径放宽 `options`，见 MODIFIED REQ-WIGA-02） |
REQ-WIGA-09（terminated **阻断全部自动路径**） | 不变（本次只放宽**人工** Enable 的 `options`，见 MODIFIED REQ-WIGA-09） |
终止自愈（`automationTerminationConflict.tsx`） | 不变（旧码保留） |
`binding_history.previous` | 不变（它是审计链，不是接续） |
自动重写 / 人工返修账本合并 | 独立事项（2026-11-03 会审：共池**未实现** + 平台自动重写**扣错池**，均登记为独立缺陷，见决策 6） |
平台自动重写改扣 `max_manual_repairs`（契约要求的 B 池） | 独立事项（行为变更，需单独 change + 真机验证额度语义；且 owner 尚未拍板「3 次共用 vs 最多 6 次」，见决策 6） |
门卡余量数字实时化 | 快照语义，另立项 |
「已初始化」判定文件 | owner 裁定：用户自己决定要不要重新初始化 |

## owner 决策记录（2026-11-02 已全部拍板）

| # | 决策 | 结论 |
|---|---|---|
1 | A-1 路线 | **一步完成**。原设计的「甲-1/甲-2/甲-3 版本号 workaround」**作废** —— 它们是为绕过那道硬门而存在的，门拆了就不需要 |
2 | 配置弹窗是否加终止按钮 | **不加**，仅文案指路。F-C（确认面不可见）已修复（确认面已改为经 `DialogShell` portal 到 `document.body` 的 fixed 浮层，`z-[80]`） |
3 | 换 provider 时旧 WIG 保留还是作废 | **全部删除**（owner：「删了就是全部都删了」）。代码已实现物理删除 |
4 | 「重来失败保留旧的吗」 | **接受**（先删后建，失败即空态） |
5 | G-2 额度口径 | ~~**文案承认共池**（与 owner 2026-10-31 裁定一致），不推翻该裁定~~ → **已被 2026-11-03 三方会审最终裁决覆盖**：改为「分别呈现两个账面事实 + 说明两处账本尚未合并」，**MUST NOT 宣称已共用一个池**、**MUST NOT 承诺算术合并值**（见决策 5、决策 6）。owner 2026-10-31 的**语义裁定本身未被推翻**（语义仍是 3 次共用），被覆盖的是**呈现写法** |
| 6 | G-1 的 `canContinuePlan` 落点（owner 原话；源码真实符号 `planContinueControl`） | **新增 WS 状态字段** |
7 | A-2「零副作用」要求 | **撤回**（删除是前提条件，不是意外损失） |
8 | 「已初始化」判定文件 | **不改** |

## 决策带来的连锁要求

| 要求 | 原因 | 落地位置 |
|---|---|---|
判据按**路径**而非**内容**写 | 「换代」由 `terminated_at` 决定，不是由 options 差异决定 | 技术方案 §3.1 |
既有测试只改 2 条，3 条不得动 | 那 3 条属 disabled→enable 路径，与本 change 无关 | 技术方案 §3.1 |
失败文案不得声称终止按钮处处可见 | 入口有前置条件 | 技术方案 §3.3 |
新分流函数必须放新文件 | `GatePromptEntry.tsx` 只剩 38 行余量 | 技术方案 §3.4 |
| `planContinueAvailable` 必须走 WS 状态而非页面 prop | `ChatCockpitPage.tsx` 不得增长 | 技术方案 §3.4 |
**G-2 触碰前必须先拆 `GatePromptEntry.tsx`** | 该文件实测 1162 行 / 硬线 1200 且不在祖父表；G-2 净增行（两个账面事实 + 未对齐说明 + 分流文案）必然顶破红线 | 会审裁决 §4「实施前置」→ `tasks.md` 第 6 组（独立任务组，先于第 7 组） |
**G-2 只改呈现，不改账本/执法点/CAS** | 共池执法与预算隔离是独立缺陷，且 owner 尚未拍板「3 次共用 vs 最多 6 次」 | 会审裁决 §5/§6 → 本文件决策 6 → `tasks.md` 第 7 组范围红线 + 第 10 组 |
