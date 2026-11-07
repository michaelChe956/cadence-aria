# 提案：自动化授权换代一步完成 + 人工门文案可行动化

> 状态：草案 —— owner 决策已全部拍板（见文末）。规格 delta 已就位，待独立审查。
> 依据：`cadence/designs/2026-11-02_技术方案_块1_自动化授权换代一步完成_v1.0.md`（owner 2026-11-02 审阅通过，425 行，源码级复核）+ `cadence/notes/2026-11-03_审查_块1change接地审查_v3.0.md`（接地审查 v3，587 行）+ `cadence/notes/2026-11-03_审查_块1change一致性裁决_v1.0.md`（一致性裁决 v1）+ owner 2026-11-02 的业务语义澄清
>
> **G-2 覆盖声明**：G-2（额度呈现）部分按 `cadence/notes/2026-11-03_会审_额度池语义_最终裁决_v1.0.md`（三方会审最终裁决）修订 —— 覆盖技术方案 §3.5 与本提案原「文案承认共池 + 同一块门卡至多一个余量数字」的写法；**目标与非目标的其余部分不动**。

## 背景

owner 2026-11-01~11-02 真机实测（issue_0001，3 成员仓 LC）撞到一组问题：在「Work Item Plan 配置」对话框把 Author 从 `claude_code` 改成 `kimi_code`、Reviewer 从 `kimi_code` 改成 `claude_code` 后点「启用自动化」，**稳定报** `automation enrollment revision conflict`；报错后自动化仍在跑，**provider 仍是旧值**。

同期撞到人工门相关的两个体验问题：过期的门卡文案一律指向「终止此门」（而真实入口是底部输入框 +「继续自动化」），以及同一张门卡上「自动修复次数已用完」与「还可自动修复 3 次」两个数字自相矛盾。

## owner 给出的业务语义（本提案的判断基准）

> 删了就是全部都删了。重来一遍。想换 provider 也一样，删了重来一遍，**不要做接续**。
>
> - 当前 WIG 在跑 → 不能删、不能换 provider；要换只能重新生成 WIG。
> - 当前 WIG 已终止且无法继续 → **必须先删掉它**，才能开新的 WIG 并切 provider。

**因此「删除旧 WIG」是换 provider 的前提条件，不是失败副作用。** 本提案据此把「换 provider」定义为**一次操作**：删光旧 WIG → 从零建新 WIG（用新 provider），不做接续。

## 根因

### 一、terminated 记录上禁止改 payload，导致「换 provider」被拆成三步

`supersede_terminated_enrollment`（`src/product/issue_automation_store_parts/cas_internals.inc.rs:671-681`，冲突返回在 `:677-681`）对 terminated 记录有一条硬门：

```rust
let options_or_target_drifted = &saved.options != options
    || saved.target.as_ref().is_some_and(|declared| *declared != *target);
if options_or_target_drifted || (source_drifted && !original_source_dangling) {
    return Ok(CasResolution::Conflict { ... });
}
```

注释写明「授权不改写：options/target 与终止记录逐字段相等」。**后果**：terminated 记录上直接带新 provider 的 Enable 被拒。

前端因此把「换 provider」拆成三步（`web/src/components/lifecycle/useIssueLifecycleGeneration.ts` 的 `runReenableSequence`）：① terminated 上用**旧** options 重新开启 → ② Disable → ③ 用**新** options 重新开启。

**这个三步序列本身就是缺陷来源**：
- 步骤①重新开启会触发 autopilot 生成，autopilot 建 plan 后 `bind_plan_ids_locked` 把 `policy_revision` 顶高一格（`cas_internals.inc.rs:55`，该处上方注释却写着「版本不递增——绑定不是换代」，实现与注释自相矛盾）→ 步骤②带旧版本号提交 → CAS 失败 → **步骤③永不执行** → provider 仍是旧值。
- 步骤①还会让**旧 provider** 真的开始生成，产出与 owner 意图不符的产物。

### 二、删除与换代的副作用已经到位，不需要重做

`supersede_terminated_plan_generation`（`src/web/handlers/automation_enrollment_reenable.rs:23-73`）已按「删了就是全部都删了」实现：先收口未终结 attempt，再按同一闭包**物理删除** attempts / worktree / shared-worktree / reviewer 资源与 plan 根。

`supersede_terminated_enrollment`（`cas_internals.inc.rs:688-763`）已完成换代动作：`prepare_intent_id` 轮换、plan/session 解绑、两个 intent 文件删除、`attention_facts` 清空、binding 换代（旧代 current 进 `previous`）、账本留痕。

**缺的只有一件事**：那条「options 必须逐字段相等」的检查，把「换 provider」挡在门外。

### 三、两条失败路径共用同一个 409 码与同一句 message

`src/product/issue_automation_store.rs:246-268`（enabled 下异 payload）与 `:269-273`（revision 不一致）都返回 `CasResolution::Conflict { current_revision }`，映射成同一 `EnrollmentError::Conflict` 与同一句 `automation enrollment revision conflict`。owner 读到后无法分辨该「等一下重试」还是「该先终止自动化」。

**接地审查补充**：该错误码今天承载 **6 类语义**（全仓 45 行 `current_revision:` —— 33 个冲突构造点、7 处字段声明、5 处错误文案/函数签名/测试夹具），且旧码被终止自愈逻辑钉住（`web/src/components/chat-workspace/cockpit/automationTerminationConflict.tsx:17`）→ **不能直接改名**，须引入原因判别位并保留旧码。

### 四、能力矩阵三态在错误出口被压平

`ProviderCapabilityEvidence` 有 `Confirmed` / `Denied { reason }` / `Unknown` 三态，准入口只放行 `Confirmed`，其余两种走同一格式串 `{action:?} launch cell is {other:?}`，真实拒绝原因被塞进 Debug 文本，而这句话原样进入用户可见 message。

### 五、三个额度被混称为「自动修复」

`manual_repairs_remaining`（人工返修）、`trigger=repair_budget_exhausted`（平台自动重写）、人工说明回合是**三类不同的额度维度**。但仓库已有 owner 2026-10-31 的裁定（`src/product/workspace_engine/compile_failure_surface.rs:126-138`）写明：**平台自动重写与人工返修是同一个预算池**、记在两处账本上。

→ 本提案**不推翻该裁定的语义**（语义仍是「3 次共用」），但**呈现写法按 2026-11-03 会审最终裁决改为**：门卡**分别呈现两个账面事实**（平台自动重写侧的 `auto_rewrite.remaining`、人工返修侧的 `manual_repairs_remaining`）并各自标注来源，**不宣称已共用一个池**、**不承诺算术合并值**；两处账本尚未合并时界面**不显示「共享余额」**，只显示两个来源各自的账面事实 + 「两处账本尚未合并」说明。共池执法与预算隔离属独立缺陷（见非目标）。

### 六、过期门卡文案与 durable 停点脱钩

过期门卡统一挂「此时只能终止此门后重新发起」+「终止此门」按钮（`web/src/components/chat-workspace/entries/GatePromptEntry.tsx:669-680`、`:773-780`；`gate-next-step.tsx:299-302`），而 `stopped_needs_human` 的真实入口是**页面底部输入框 +「继续自动化」**。

**已修的只有「已确认完成」一类**（`gate-next-step.tsx:87-88` `GATE_STALE_COMPLETED_COPY`，判据 `sessionStatus === "confirmed"`）；`stopped_needs_human` / `failed` / 「推进到别的门」三类仍按同一段文案渲染。

**接地审查补充**：文案实际住在 `gate-next-step.tsx`（不是 `gate-prompt-copy.ts`，后者存的是动作/错误条文案）；且「无真实续跑入口时不得假指引」所需的 `planContinueControl` **不在 store 里**（页级局部，真实符号见 `web/src/pages/ChatCockpitPage.tsx:756-769`；仓库中**不存在** `canContinuePlan` 这个符号），而该页 1471 行、在 `large_file_guard` 祖父表内**不得增长**。

## 目标

| # | 目标 |
|---|---|
1 | **换 provider 一次操作完成**：terminated 记录上直接带新 provider 的 Enable 被接受，仍走完整换代流程（轮换 `prepare_intent_id`、解绑、binding 换代、旧代进 `previous`、账本留痕、删光旧 WIG 全闭包） |
2 | **不做接续**：换 provider 后不残留旧 provider 的绑定身份、生成意图或半成品产物 |
3 | **换 provider 的失败面可行动**：区分「运行中不许改（先终止）」与其它语义，给中文文案与具体下一步，机器码降级到次要行 |
4 | **过期门卡指向真实入口**：按 durable 停点状态分流，过期卡不再把 owner 引向「终止此门」；无真实续跑入口时不得假指引 |
| 5 | **额度显示不自相矛盾且不谎报**：三个额度各用各的名字（平台自动重写 / 人工返修 / 你已说明 N 轮），**两个来源的账面事实分别呈现并各自标注来源**，界面**不显示未经证实的「共享余额」**（改为「两处账本尚未合并」说明）；「已用完」不由触发原因直接断言；owner 2026-10-31 的**共池语义不被推翻**，但呈现层**不得宣称已共用一个池**、**不得承诺算术合并值** |

## 非目标

- 不改 provider 适配层、网关信封、流式输入结构、工具策略
- 不改「运行中不许改 payload」这条契约本身（REQ-WIGA-02 保持 fail-closed）—— 本提案只补**出路指引**
- 不做 provider 选择的用户偏好配置
- 不做自动重写与人工返修的**账本合并**（属独立事项；本期只改文案与呈现）
- 不改其它人工门的行为（只改文案与呈现）
- 不做 provider CLI 的自动安装
- 不动 `binding_history.previous` 的留痕语义（它是审计链，不是接续）

## 范围

| 能力 | 变更 |
|---|---|
`work-item-group-autopilot` | MODIFIED REQ-WIGA-02（terminated 记录上的人工重开：`options` 允许与终止记录不同、`source`/`target` 仍逐字段相等）与 MODIFIED REQ-WIGA-09（terminated 记录的人工 Enable 换代；`source`/`target` 漂移仍 409）；ADDED「换 provider 一次完成且不做接续」「失败面按语义分诊」 |
`human-gate-convergence` | MODIFIED REQ-HGC-01（门卡预算显示的唯一授权事实：三类额度各自命名、**两个来源的账面事实分别呈现并各自标注来源**、共享余额须有后端同域权威事实才展示、「已用完」不得由触发原因直接断言、共池只作说明性表述）；ADDED REQ-HGC-04「过期人工门卡按 durable 停点分流指引」 |

## 风险与取舍

| 风险 | 处置 |
|---|---|
放宽「options 逐字段相等」可能绕过授权留痕 | 换代留痕**已由 `supersede_terminated_enrollment` 实现**（轮换、解绑、binding 换代、旧代进 previous、账本 Accepted）；本提案只移除那条**内容相等检查**，不触碰留痕动作。`source`/`target` 的漂移管控**保持不变**（仍只在悬空源重授权时放行） |
「绑定递增版本号」是否可移除 | 接地审查已穷举：并发绑定互斥靠 enrollment 独占锁 + 内容判定（`cas_internals.inc.rs:39-44`）、同键幂等靠内容判定（`:38-40`）、换代路径另有 increment（rebind `command_writes_impl.inc.rs:176-178`、terminated 取代 `cas_internals.inc.rs:694`、Enable/Disable `cas_internals.inc.rs:500/506`）→ **移除绑定处递增不削弱任何既有保护**。但本提案**不在本 change 移除它**（三步塌缩后该路径不再被走到），仅同步修正该处与 `:31` 的自相矛盾注释 |
「换 provider 时旧 WIG 已删、新的没建起来」 | owner 已裁定**接受**（选 1）：这是「删了重来」的固有语义。失败面须明确告知「旧的已删、新的没建起来」并给重试指引，**不得**表述为「什么都没发生」 |
错误码拆分影响终止自愈 | 旧码 `enrollment_conflict` 被 `automationTerminationConflict.tsx:17` 与 `CockpitInboxWaitingCards.termination.test.tsx:307` 钉住 → 引入**原因判别位**（`EnrollmentError::Conflict` 带 reason），旧码保留给 termination 路径，PUT 路径按 reason 分流新码；**不改** `automationTerminationConflict.tsx` 的自愈分支 |
| 额度共池与「分别命名」的表面冲突 | 共池语义（3 次共用）由 owner 2026-10-31 裁定，但**代码上是两处未合并账本** → 呈现层**分别呈现两个账面事实并各自标注来源**，加「两处账本尚未合并」说明；**不宣称已共用一个池**、**不承诺算术合并值**；「共享余额」只在后端给出同域权威事实后才展示 |
| 门卡出现两个来源的数字是否变成新的自相矛盾 | 不是：两个数字**来源不同、用途不同、分别标注**，且明示账本未合并；原矛盾来自**同一个「还能做几次」被两个池混称**（门卡 `:561-573` 与两套 trigger 标签表），本轮从命名与来源标注上消除该混称 |
| 「已用完」不得由 `repair_budget_exhausted` 直接断言后，owner 会失去停止原因 | 改为按 `flowKind`/触发路径分流呈现；判定不出路径时用中性文案「本轮评审自动返修已停止，需要你决定下一步。」，不再把结构封闭误报成额度耗尽 |
| `GatePromptEntry.tsx` 逼近行数红线 | 实测 1162 行 / 硬线 1200 且不在祖父表 → **G-2 触碰前先拆该文件**（`tasks.md` 第 6 组，独立任务组，先于第 7 组） |
「无真实续跑入口」需要新事实源 | 新增 WS 会话状态字段承载「本轮是否可继续」，由 `GatePromptEntry` 读取；**不得**把状态挂在 `ChatCockpitPage.tsx`（1471 行，祖父表锁定不得增长） |
门卡文案改动会撞既有测试 | 已核实 `GatePromptEntry.stale-completed.test.tsx` 等处钉住旧文案 → 文案与测试同批改；其中编译失败态的合法「终止此门」**不得**一并删 |

## owner 决策（2026-11-02 已全部拍板）

| # | 决策 | 结论 |
|---|---|---|
1 | A-1 走哪条路线 | **一步完成**（terminated 上直接带新 provider 的 Enable，仍走完整换代）。原设计的「甲-1/甲-2/甲-3」版本号 workaround 作废——它们是为绕过那道硬门而存在的，门拆了就不需要 |
2 | 配置弹窗是否新增「终止并换用新 provider」按钮 | **不新增**，仅在失败文案指路到既有「终止整个自动化」确认面。终止不可逆；且该确认面的可见性缺陷（F-C）已修复（确认面已改为经 `DialogShell` portal 到 `document.body` 的 fixed 浮层，`z-[80]`） |
3 | 换 provider 时旧 WIG 保留还是作废 | **全部删除**（owner 原话「删了就是全部都删了」）。代码已实现物理删除，无需改动 |
4 | 「重来失败保留旧的吗」 | **1 接受**：先删后建；失败即空态，失败面明确告知「旧的已删、新的没建起来」 |
5 | G-2 额度口径 | ~~**A 文案承认共池**（与 owner 2026-10-31 裁定一致），不推翻该裁定~~ → **2026-11-03 三方会审最终裁决覆盖**：改为「两个账面事实分别呈现 + 说明两处账本尚未合并」，**不得宣称已共用一个池**、**不得承诺算术合并值**。owner 2026-10-31 的**语义裁定（3 次共用）未被推翻**，被覆盖的是呈现写法；共池执法与预算隔离登记为独立缺陷 |
| 6 | G-1 的 `canContinuePlan` 落点（owner 原话；源码真实符号 `planContinueControl`） | **A 新增 WS 状态字段** |
