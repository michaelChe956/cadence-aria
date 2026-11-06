# Tasks: fix-automation-enrollment-provider-swap

> **收口注记（2026-11-03）**：本 change 已于 2026-11-03 归档，delta 已同步进主 specs
> （`work-item-group-autopilot` 与 `human-gate-convergence`；MODIFIED 逐字核对通过，ADDED 全场景在位）。
> 勾选口径与逐条文件:行级证据见 `cadence/notes/2026-11-03_收口_行数登记表改动清单_v1.0.md` §M.1。
>
> **剩余未勾 9 条及原因**（2026-11-04 补记：8.1–8.4 已核验补勾，「待统一绿测」类现为 **0** 条；出处 `cadence/notes/2026-11-04_待绿测5条核验补勾_v1.0.md` §1.1–§1.4）：**2026-11-07 补记：7.8 / 9.1 / 9.2 / 9.7 / 9.8 已实测通过并补勾（本文件 §7 / §9 证据行）；剩余未勾 4 条 = 9.3 / 9.4 / 9.5 / 9.6（被 bug1 阻塞，2026-11-07 仍未勾）**
> · **8.1–8.4**（4 条）—— ✅ 2026-11-04 收口统一绿测核验后补勾完毕，不再计入未勾（同上 §1.1–§1.4）
> · **9.1–9.8**（8 条）真机验收 —— **⏳ 待实做（Main 真机验收批次：受控环境操控 + 浏览器实测；逐条做、逐条勾、逐条留证据）**（**2026-11-07 补记：9.1 / 9.2 / 9.7 / 9.8 已实测通过并补勾；9.3 / 9.4 / 9.5 / 9.6 仍未勾**）
>   ⇒ 本机四家 provider（`pi` / `codex` / `kimi` / `claude`）+ `codegraph` **均已安装**（`command -v` 实测：前四家在 `/home/michaelche/.local/share/fnm/aliases/default/bin/` 与 `/home/michaelche/.kimi-code/bin/`，`codegraph` 同 fnm 路径、全局包 `@colbymchenry/codegraph@1.6.2`）
>   ⇒ **不是环境不具备，是尚未执行**；下列 8 条**全部可做**
>   逐条怎么做：
>   · **9.1** 运行中换 provider → 中文文案说清「先终止」+ 去哪儿点；不再出现英文机器码 ⇒ **浏览器实测**（fake provider 驱动）（✅ 2026-11-07 已勾）
>   · **9.2** 按指引到驾驶舱点「终止整个自动化」→ 回弹窗重新启用 → 走通 ⇒ **浏览器实测**（纯 UI 流程）（✅ 2026-11-07 已勾）
>   · **9.3** 终止态换 provider → 一次操作换成功；不出现旧 provider 的产出 ⇒ **浏览器实测 + 产出归属核对**
>   · **9.4** 换完 provider 后后台日志无 reconcile 持续冲突 ⇒ **跑一遍看日志**
>   · **9.5** 换失败 → 旧 WIG 确实已删、新 WIG 未建、失败面文案正确 ⇒ **构造失败**（fake provider 失败注入）
>   · **9.6** 换完跑一轮完整生成 → 产物由新 provider 产出 ⇒ **浏览器实测 + 产出归属核对**
>   · **9.7** 自动化停在待人工 → 门卡指向底部输入框 +「继续自动化」 ⇒ **浏览器实测**（✅ 2026-11-07 已勾）
>   · **9.8** 门卡额度呈现 → 两来源账面事实分别显示 + 不出现共享余额数字 + 出现「两处账本尚未合并」 ⇒ **浏览器实测**（纯呈现）（✅ 2026-11-07 已勾）
> · **7.8** 当前卡重建/重连的实时刷新未做（Plan §6-G 已降级另立项；B2 余量仍是卡片创建时快照）（✅ 2026-11-07 已勾）
>   ⇒ 旧卡只读历史（`gate-quota-labels.ts:42`/`:102-118`）与门快照重建取新预算
>     （`workspace-cockpit-gate-projection.ts:343-366`）**已落地**，缺的是实时刷新那一段

> owner 决策已全部拍板；技术方案见 `cadence/designs/2026-11-02_技术方案_块1_自动化授权换代一步完成_v1.0.md`（owner 2026-11-02 审阅通过）。
> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`；本文件只记高层工作包，每个工作包映射到相关 requirement。

## 跨块同文件有序串行登记（块1 ↔ 块3，5 个文件）

> **这不是 task 组，是跨块并行的顺序约束登记** —— 块1 与块3 的实施 Plan 有一部分**改同一个文件**；这些文件必须**按表定顺序串行**，否则两块的提交会撞 Git。
> 登记来源：2026-11-03 双审（事实错误与漏登）处置；与两份实施 Plan 一致（块1 Plan「并行安全与跨块协调」节、块3 Plan §4.1「跨块文件交集」）。

| # | 文件 | 顺序（先 → 后） | 两块各改什么 | 性质与处置 |
|---|---|---|---|---|
| 1 | `web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx` | **块1 片4 → 块3 片3** | 块1 片4：`:404-408` 文案面（「重新开启后关闭再开启生效」→「换 provider 会先删除现有工作项组」）+ `SubmitErrorState` 加 `code`；块3 片3：`enrollmentTargetLabel` 的 `case "single_repository"`（`:533-534`）渲染保留 + Enable 提交路径改用 `carrierTargetFromEnrollmentTarget`（为 `null` 时禁用提交 + 提示） | 同文件**不同位置**、语义分离 ⇒ **必须串行**；块3 片3 **不得**删 `SubmitErrorState.code` / `errorCode` 分支，块1 片4 **不得**删 `single_repository` 分支 |
| 2 | `web/src/components/lifecycle/WorkItemPlanOptionsDialog.reenable.test.tsx` | **块1 片4 → 块3 片3** | 块1 片4：改写 `:195` 用例 + 新增「主文案 + 次要行机器码」用例；块3 片3：`:36`/`:80` 夹具保留 + 新增「`single_repository` 终止记录 → 重新开启禁用 + 提示」用例 | 同文件**不同用例** ⇒ **必须串行**；后落方**只加不删**先落方的用例 |
| 3 | `web/src/components/lifecycle/useIssueLifecycleGeneration.reenable.test.ts` | **块1 片4 → 块3 片3** | 块1 片4：改写 `:603`/`:643`/`:661`（三步塌缩为一次提交）；块3 片3：`:88`/`:159`/`:523` 夹具保留（按该用例是否走提交决定保留 legacy 或改 `logical_codebase`） | 同文件**不同用例** ⇒ **必须串行**；后落方**只加不删**先落方的用例 |
| 4 | `src/web/handlers/automation_enrollment.rs` | **块3 片1 → 块1 片3** | 块3 片1：删 `:384` 的 `AutomationCarrierResolution::SingleRepository` 载体判定臂（Enable / prepare / Enrolled advance）；块1 片3：只改 `:582-591` 的 `enrollment_api_error`（按 kind 分流；如确需可改 `:20-25` 的 `enrollment_conflict` helper） | 同文件**互不相干的位置**（非语义冲突）⇒ 属 **Git 级同文件并发写**，**必须串行**；块1 片3 **MUST NOT** 动 `:384` 的载体判定 |
| 5 | `src/web/error.rs` | **块3 片1 → 块1 片3** | 块3 片1：新增稳定码 `issue_not_bound_to_logical_codebase` → 400 BAD_REQUEST（稳定码→状态表 `:160-215`）；块1 片3：新增两条稳定码（运行态限制 / 版本过期）→ 409 | 同文件**不同码表条目**（非语义冲突）⇒ 属 **Git 级同文件并发写**，**必须串行**；后落方不得删改先落方的码条目 |

**登记规则（两块一致）**：① 表中「先落方」必须**独立提交**落地后，「后落方」才可起工，且开工前须 rebase 到先落方提交之上；② 后落方**不得**删改先落方在该文件里的行，合并冲突时**手工保留双方行**；③ #4 #5 改的是同一文件里**互不相干的位置**，不是语义冲突，但同文件并发写会撞 Git ⇒ 仍按串行处理。

**明确不构成交集、不要误登记的文件**：`WorkItemPlanOptionsDialog.plan-approval.test.tsx`、`WorkItemPlanOptionsDialog.target.test.tsx`、`useIssueLifecycleGeneration.plan-approval.test.ts` 三份在**块1 片4 的禁碰清单**里（块1 侧零改动，块3 片3 单方独占）⇒ **不构成跨块交集，不需要定序**。

## 1. 放宽 terminated 路径的授权内容检查

映射 requirement：REQ-WIGA-SWAP-01、MODIFIED REQ-WIGA-02、MODIFIED REQ-WIGA-09

- [x] 1.1 `supersede_terminated_enrollment` 的判定允许 `options` 不同（provider 可换），**但必须按路径分**：`options` 漂移**仅在 `original_source_dangling == false`（正常重开）时放行**；`original_source_dangling == true`（悬空源重授权）时 `options`/`target` **仍 MUST 逐字段相等**。`target` 与 `source` 的漂移管控保持不变
- [x] 1.2 **判据按路径写**：terminated 记录上的 Enable 一律换代；disabled 记录上的 Enable 走普通路径不换代。**不得**引入「options 不同才换代」的判据
- [x] 1.3 同步修正 `cas_internals.inc.rs:665-670` 与 `issue_automation_store.rs:85-87`/`:103-107`/`:150-157` 里的「options 逐字段相等」表述 —— **注意 `issue_automation_store.rs:98-101`（悬空源重授权专用 CAS）的「options/target 仍逐字段相等」表述保持不变**
- [x] 1.4 修正 `cas_internals.inc.rs:31` 与 `:49-50` 两处自相矛盾的绑定注释；**不删除**绑定处的版本号递增
- [x] 1.5 改写 terminated 路径的 2 条既有契约测试（`attention_termination_tests.inc.rs:805-819`、`:1098-1112`）+ `automation_reenable_supersede_tests.inc.rs:530`/`:617` 的注释与断言。**⚠ `:1098-1112` 同时是 `dangling-reference-governance` 的 REQ-DRG-18「只有 source 可被放行」（`openspec/specs/dangling-reference-governance/spec.md:561-564`）的执行证据 —— 该场景的前置是「source 已悬空」，因此 **REQ-DRG-18 的契约无需修订**；但**测试翻转必须按 1.1 的路径分执行**：`:805-819`（正常路径）翻转为「允许 options 不同」；`:1098-1112`（悬空源路径）**保持拒绝**。**不得**把 options 检查整体删除 —— 那会静默推翻 REQ-DRG-18 的 MUST
- [x] 1.6 用例：terminated **正常路径**上直接带新 provider 的 Enable 被接受且完成换代；terminated **悬空源路径**上 options 不同**仍被拒**；disabled 上的 Enable 不换代不删 plan；换代后 `binding_version` 递增且旧代进 `previous`

## 2. 前端三步塌缩为一步

映射 requirement：REQ-WIGA-SWAP-01

- [x] 2.1 `runReenableSequence` 改为 terminated 记录上直接提交带新 options 的 Enable
- [x] 2.2 删 `reenableSequenceRef` / `resumingSequence` / `sequence` 形参与塌缩后无意义的早退条件
- [x] 2.3 `WorkItemPlanOptionsDialog.tsx:404-408` 删「重新开启后关闭再开启生效」，补「换 provider 会先删除现有工作项组」
- [x] 2.4 `SubmitErrorState` 加 `code`；机器码降级到次要行
- [x] 2.5 用例：一次提交即完成换 provider；旧 provider 不被触发任何生成；失败时不留半提交状态

## 3. 失败面按语义分诊

映射 requirement：REQ-WIGA-SWAP-02

- [x] 3.1 `EnrollmentError::Conflict` 增加原因判别位；归类全部 33 个构造点的语义（9 类）
- [x] 3.2 判别位穿透（≥45 处机械改动，`current_revision:` 全仓 45 行），**含 `CasResolution`（`issue_automation_store.rs:36-45`）这一层** —— 否则 `resolve()` 会丢原因
- [x] 3.3 **命令写入侧拆 if**：`command_writes_impl.inc.rs:46` 与 `:124` 把「身份不符/未启用」与「revision 过期」压进同一个 `if` → 整条映射 `revision_stale` 会指错路 → 必须拆成两个分支
- [x] 3.4 HTTP 层按原因分流新稳定码（运行态限制 / 版本过期）；**其余 7 类落回旧码**；**终止路径一行不改**
- [x] 3.5 步骤①与步骤②③的文案包装统一
- [x] 3.6 中文文案成品：「运行中不能直接换 provider」+ 指路到既有「终止整个自动化」确认面（**不得**声称终止入口在所有落屏可见）；「授权记录刚发生变化，本次操作没有生效」+「等一下重试」
- [x] 3.7 机器码与原始 detail 降级到次要行 / 折叠区
- [x] 3.8 用例：两条路径返回不同稳定码；终止自愈仍生效；老前端拿到新码不崩

## 4. 「删了重来」失败的失败面

映射 requirement：REQ-WIGA-SWAP-01

- [x] 4.1 失败面文案如实告知「上一次的工作已经被清空，但新的没建起来」+ 重试指引；不得表述为「未发生任何变化」
- [x] 4.2 用例：模拟换代后失败 → 旧 WIG 已删、新 WIG 未建、失败面文案正确

## 5. 过期门卡按 durable 停点分流

映射 requirement：REQ-HGC-04（`human-gate-convergence`）

- [x] 5.1 新增单一事实源模块（门卡过期指引表），按 durable 停点状态给出文案与真实入口
- [x] 5.2 **签名必须带 `actionBlockReason`** —— 区分「这张卡已过期」与「这张卡正在等你拍板」。**非过期卡的 `showTerminate` 默认 `true`**（否则普通门卡的「终止此门」按钮会被删掉 —— owner 正在用的按钮）
- [x] 5.3 `stopped_needs_human` 指向底部输入框 +「继续自动化」，不指向终止，且不挂「终止此门」按钮
- [x] 5.4 `failed` 指向「继续自动化」重试；`terminated` 视为历史记录；推进到别的门以最新门卡为准并保留终止作为可选出口
- [x] 5.5 已确认完成场景保持现状（回归锁）
- [x] 5.6 新增 WS 会话状态字段 `plan_continue_available`：服务端从 HTTP control 投影注入，**注入点必须对齐参照实现的全部调用点**（刷新 / 重连 / observer 重建路径都要有，否则这些路径发 `None` → 门卡变假指引）；前端 4 处 ingestion 同步（含最易漏的 observer 镜像）
- [x] 5.7 同步更新钉住旧文案的既有测试；其中编译失败态的合法「终止此门」**不得**一并删

## 6. 前置：拆分 `GatePromptEntry.tsx`（G-2 的实施前置，必须先于第 7 组）

映射 requirement：无（纯移动、零行为变化）；依据：2026-11-03 三方会审最终裁决 §4「实施前置」

- [x] 6.1 **为什么必须先拆**：`web/src/components/chat-workspace/entries/GatePromptEntry.tsx` 实测 1162 行，行数门禁硬线 1200（`tests/it_core/large_file_guard.rs:4-6`，`SCAN_ROOTS` 含 `web/src`），该文件**不在祖父表** ⇒ 净增余量极小。G-2 要在门卡上同时呈现**两个账面事实 + 来源标注 + 未对齐说明 + 按路径分流的中性文案**，净增行必然顶破红线
- [x] 6.2 **拆分纪律**：纯移动、零行为变化。沿用仓库既有先例（`gate-card-metadata.ts`、`CockpitInboxGateActions.tsx`、`workspace-cockpit-gate-projection.ts` 均为「从大文件纯移动拆出」模式）；拆出物 SHALL 只做「读事实 → 渲染/文案」，无状态、无副作用
- [x] 6.3 拆出物放在 `web/src/components/chat-workspace/entries/` 下，命名沿用既有 `gate-*.ts(x)` 风格；**G-2 后续要新增的分流函数与文案常量 MUST 落在拆出的新文件里**，不得再往 `GatePromptEntry.tsx` 里塞
- [x] 6.4 验收：`GatePromptEntry.tsx` 行数显著下降（目标 ≤ 1000 行，留出净增余量）；`large_file_guard` 绿；前端 vitest 全绿（拆分零行为变化）
- [x] 6.5 **顺序硬约束**：本组 MUST 先于第 7 组（G-2 呈现层改造）落地；否则门卡行数门禁会被 G-2 的净增行顶破

## 7. 额度呈现层改造（只改呈现层）

映射 requirement：MODIFIED REQ-HGC-01（`human-gate-convergence`）

> **范围红线**：本组**只改呈现层** —— **不动账本**（B1 `auto_rewrites_used` / B2 `manual_repairs_used` / A 池 `repairs_used` 一律不改）、**不动执法点**（`evaluate.rs`、`compile_failure_surface.rs`、`lifecycle_store/human_gate.rs` 一律不改）、**不动 CAS**（`human_gate.rs:193-201` 的同 CAS 预留/递减不动）。

- [x] 7.1 三个额度各用各的名字（**平台自动重写** / **人工返修** / **你已说明 N 轮**），各计数只标自己的名字；**A 池（机械返修 `max_repairs`）不上界面额度名**；MUST NOT 用「自动修复」统称
- [x] 7.2 **分别呈现两个账面事实**：B1 的 `auto_rewrite.remaining`（平台自动重写侧）与 B2 的 `manual_repairs_remaining`（人工返修侧），**各自标注来源与用途**
- [x] 7.3 **MUST NOT** 任选其中一个当「共享余额」；**MUST NOT** 合成或猜测跨账本的余量值；**MUST NOT** 隐藏另一事实后宣称共享余额已确定
- [x] 7.4 **共享余额仅在后端提供同域权威合并事实时才展示**；当前后端未提供 → 界面**不显示**「共享余额」，只显示两个来源各自的账面事实 + 「两处账本尚未合并」的说明
- [x] 7.5 **「已用完」MUST NOT 由 `repair_budget_exhausted` 直接断言** → 中性文案「本轮评审自动返修已停止，需要你决定下一步。」；按触发路径（`flowKind`）分流，SC 分流的固定标签 MUST NOT 渲染为「次数已用完」
- [x] 7.6 **共池只用说明性文字**（「平台自动重写与你的反馈共用同一份额度上限」），**MUST NOT** 承诺算术合并值；requirement 与界面文案 **MUST NOT** 宣称「已共用一个池」
- [x] 7.7 **数据缺失**：旧记录缺事实 → 显示「预算历史不可用／共享余额无法确认」，不补默认、不让旧卡读成当前授权
- [x] 7.8 **卡片刷新**：当前卡重建 / 重连 SHALL 刷新同门当前事实；旧卡只读历史
      ✅ 2026-11-07 owner 真机实测通过（owner 回报并授权勾选；owner 侧操作端口 5173 开发期 / 4317 后端）。★证据文件待补★ —— 本行证据来自 owner 回报，未附截图/日志文件；owner 提供后再补链接。
- [x] 7.9 落地点（按技术方案 §3.5 的改名落点表 + 遗漏项）：`GatePromptEntry.tsx` 门卡 meta 行（现 `:561-573`「你已回复过 N 轮」/「还可自动修复 N 次」）、`gate-next-step.tsx` 的 `GATE_HUMAN_TRIGGER_LABELS`（现 `:97-102`「自动修复次数已用完，需要你来定」）、`workspace-cockpit-gate-projection.ts` 的**第二套** `GATE_TRIGGER_LABELS`（现 `:16-21`「修复轮次已用尽」）—— 不改则 G-2 在该面复现
- [x] 7.10 明确不改：结构化输出修复语义的「自动修复」（`StructuredOutputDiagnostic.tsx`）、Coder 修复次数（第四个池，`CodingWorkspacePage.tsx`）
- [x] 7.11 同步更新 ≥14 处受影响的测试断言（**不得只改文案不改测试**）
- [x] 7.12 用例（对应裁决 §4.1 六条验收场景）：两来源余量不一致时分别呈现且不承诺共享余额；结构/复验阻断不出现「次数用完」；停止原因与正余额并存不互相覆盖；同门反馈受理后 M:3→2 / C:0→1 且重建重连不回填 3；旧数据缺事实显示「预算历史不可用」；技术详情仍可见三笔原始计数

## 8. 回归与门禁

- [x] 8.1 授权持久化相关单测全绿（含行数门禁 `large_file_guard`）
  - ✅ 2026-11-04 补勾（收口统一绿测实测）：`cargo test --locked --lib` → 5964 passed / 0 failed / 7 ignored / 0 measured，101.33s；`cargo test --locked --all-targets` → lib 5964/0（99.91s）、large_file_guard 2 passed / 0 failed（product_source_and_test_files_stay_under_line_limit ok）；行数门禁 109 条登记 · 超自身上限 0 · 未登记且超 1200 的 0 · 路径升序 · cap 全 >1200。lib 与门禁两档 0 红；全树仅剩 2 条红均为有意登记（it_core/it_web 各 1，详见 8.4 注记及出处）
- [x] 8.2 隔离与纪律测试（`assert_aggregate_scope_discipline` 等）全绿
  - ✅ 2026-11-04 补勾：`assert_aggregate_scope_discipline` 为 lib 单测（定义于 `src/product/logical_codebase/coordinator_provider_turn.inc.rs:678`），含于 `cargo test --locked --lib` → 5964 passed / 0 failed / 7 ignored / 0 measured，101.33s ⇒ 隔离与纪律测试全绿。全树仅剩 2 条红均为有意登记（详见 8.4 注记及出处），均不在 lib 档
- [x] 8.3 前端 vitest 全绿（含改动的门卡文案测试与 WS 状态字段相关测试）
  - ✅ 2026-11-04 补勾：`pnpm test` → 260 files / 2662 tests passed / exit 0（29.18s），改动的门卡文案测试与 WS 状态字段（`plan_continue_available`）相关测试含于其中、全绿；`pnpm build` → exit 0（✓ built in 2.90s）。前端档 0 红；全树仅剩 2 条红均为有意登记且均在 Rust it 档（详见 8.4 注记及出处）
- [x] 8.4 全量回归（Rust lib + it_core + it_web + 前端 test/build）由 controller 在重发前统一跑
  - ✅ 2026-11-04 补勾（controller 收口统一跑）：`cargo build --all-targets` → exit 0，1m59s；`cargo test --locked --lib` → 5964 passed / 0 failed / 7 ignored / 0 measured，101.33s；`cargo test --locked --all-targets` → lib 5964/0（99.91s）、large_file_guard 2 passed / 0 failed；`cargo clippy --locked --all-targets -- -D warnings` → exit 0；`pnpm build` → exit 0（✓ built in 2.90s）；`pnpm test` → 260 files / 2662 tests passed / exit 0（29.18s）。仅剩 2 条红均为【有意登记】：① it_core 197 passed / 1 failed = `workspace_ws_abort_with_pi_reaches_cancelled_state_and_stops_output`（`tests/it_core/workspace_ws_integration/part_04.rs:291`；断言钉 adapter 侧观察点 (ProviderType::Pi, ProviderPermissionMode::Auto)，而 validated 启动按设计只外借 launch_view() 的 role/working_dir/prompt ⇒ 无等价可观察点 ⇒ 已裁定暂留红、待立项；出处：`cadence/notes/2026-11-03_全量失败收尾_it_core_ws_integration_v1.0.md` §6、`cadence/notes/2026-11-03_越线拆分_it_core_part05_v1.0.md`）；② it_web 593 passed / 1 failed = `evidence_query::invalid_query_returns_422_and_uninitialized_index_returns_503`（`tests/it_web/evidence_query/part_03.rs:43`；环境态：codegraph 1.6.2 对未建索引目录恒 exit 0 ⇒「未建索引 ⇒ 503」本机不可复现，产品契约另由 `src/web/handlers/evidence_error_mapping.rs` 的单测钉住；出处：`cadence/notes/2026-11-03_全量失败收尾_it_web_派A3_v1.0.md` §3.1）

## 9. 真机验收

- [x] 9.1 运行中换 provider → 中文文案说清「先终止」+ 去哪儿点；不再出现英文机器码
      ✅ 2026-11-07 owner 真机实测通过（owner 回报并授权勾选；owner 侧操作端口 5173 开发期 / 4317 后端）。★证据文件待补★ —— 本行证据来自 owner 回报，未附截图/日志文件；owner 提供后再补链接。
- [x] 9.2 按指引到驾驶舱点「终止整个自动化」→ 回弹窗重新启用 → 走通
      ✅ 2026-11-07 owner 真机实测通过（owner 回报并授权勾选；owner 侧操作端口 5173 开发期 / 4317 后端）。★证据文件待补★ —— 本行证据来自 owner 回报，未附截图/日志文件；owner 提供后再补链接。
- [ ] 9.3 终止态换 provider → **一次操作**换成功；不出现旧 provider 的产出
- [ ] 9.4 换完 provider 后后台日志**无** reconcile 持续冲突
- [ ] 9.5 换失败 → 旧 WIG 确实已删（符合语义）、新 WIG 未建、失败面文案正确
- [ ] 9.6 换完跑一轮完整生成 → 产物由**新** provider 产出
- [x] 9.7 自动化停在待人工 → 门卡指向底部输入框 +「继续自动化」，不再是「终止此门」
      ✅ 2026-11-07 owner 真机实测通过（owner 回报并授权勾选；owner 侧操作端口 5173 开发期 / 4317 后端）。★证据文件待补★ —— 本行证据来自 owner 回报，未附截图/日志文件；owner 提供后再补链接。
- [x] 9.8 门卡额度呈现 → **两个来源的账面事实分别显示并各自标注来源**；界面**不出现**「共享余额」数字；出现「两处账本尚未合并」说明；`repair_budget_exhausted` 触发时**不出现**「次数已用完」，改显中性文案
      ✅ 2026-11-07 owner 真机实测通过（owner 回报并授权勾选；owner 侧操作端口 5173 开发期 / 4317 后端）。★证据文件待补★ —— 本行证据来自 owner 回报，未附截图/日志文件；owner 提供后再补链接。

## 10. 契约与留痕

- [x] 10.1 本 change 的 proposal / design / 两份 spec delta（`work-item-group-autopilot` 的 MODIFIED REQ-WIGA-02/09 + ADDED REQ-WIGA-SWAP-01/02；`human-gate-convergence` 的 MODIFIED REQ-HGC-01 + ADDED REQ-HGC-04）/ tasks
- [x] 10.2 **登记独立缺陷**（2026-11-03 会审裁决 §5）：自动重写额度共池未实现 + 平台自动重写执行入口扣的是机械返修池 `max_repairs`（A 池，上限 1）而非契约要求的 `max_manual_repairs`（B 池，上限 3）→ 开**独立** OpenSpec change / 缺陷单，**不在本 change 内修**
- [x] 10.3 **契约边界留痕**：本 change 只修呈现语义；**不得**因 G-2 完成而把 REQ-WSC-21 的预算隔离与共池执法标记为已修复
- [x] 10.4 归档前补独立审查结论
