# task 对证｜`fix-automation-enrollment-provider-swap`（块1）v1.0

- 日期：2026-11-03
- 工作树：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`（所有路径均相对此绝对路径）
- 对证对象：`openspec/changes/fix-automation-enrollment-provider-swap/tasks.md`（61 条 `- [ ]`）
- 证据来源：块1 实施报告 5 份（片1/片3/片4/片5/片6；**片2 尚未交付**）+ 实际代码 + 两份 spec delta + 块1 实施 Plan（`cadence/plans/2026-11-03_计划文档_实施_块1_自动化授权换代与人工门文案_v1.0.md`）
- 判定规则：① 报告有明确证据 **且** ② 代码中亲眼确认产物存在 → 勾选；②缺失 → 未证实；切片在途 → 待切片落地；只完成一部分 → 部分完成
- 只改 `tasks.md` 的勾选框（`- [ ]` ⇄ `- [x]`），**未改任何任务文字/编号/顺序/其它段落**（归一化 diff 自证：`NONE (only checkbox marks changed)`）
- ⚠️ 查证期间树在活跃变动（片2 正在落地：`src/product/issue_automation_store.rs` mtime 由 08:49 变为 09:32）；本文结论是**查证时刻快照**

---

## 1. 统计

| 项 | 数量 |
|---|---|
| 总条数 | **61** |
| 已勾选 `- [x]` | **36** |
| 未勾选 `- [ ]` | **25** |
| ├ 待切片落地 | **20**（1.1–1.6 片2 在途 6 条；3.4 跨块半支 1 条；8.1–8.4 片7 4 条；9.1–9.8 片7 真机 8 条；10.4 片7 1 条） |
| ├ 部分完成 | **4**（3.8、6.4、7.8、10.2） |
| └ 未证实 | **1**（3.3） |

已勾选清单：2.1 2.2 2.3 2.4 2.5 / 3.1 3.2 3.5 3.6 3.7 / 4.1 4.2 / 5.1–5.7 / 6.1 6.2 6.3 6.5 / 7.1–7.7 7.9 7.10 7.11 7.12 / 10.1 10.3。

---

## 2. 逐条对照表（61 条）

> 报告简称：R1=`2026-11-03_实施_块1_片1_v1.0.md`、R3=`…片3…`、R4=`…片4…`、R5=`…片5…`、R6=`…片6…`（均在 `cadence/notes/`）。

| 任务 | 结论 | 证据出处（报告:行 → 代码:行） |
|---|---|---|
| 1.1 | 待片2 | 无片2 报告；代码未落地：`cas_internals.inc.rs:716-727` 仍是合并判定 `options_or_target_drifted \|\| (source_drifted && !original_source_dangling)`，**未按 `original_source_dangling` 分路径放行 options 漂移** |
| 1.2 | 待片2 | 同上（判据仍未按路径写；亦未见「options 不同才换代」判据，故 C-2 未被违反） |
| 1.3 | 待片2 | R3 未涉；代码：`issue_automation_store.rs:91-92` 与 `:163` **已改为按路径表述（片2 在途已落一半）**；`:106`（悬空源专用 CAS）按要求保持原样 ✓；但 `cas_internals.inc.rs:656`、`:720` 仍是「与终止记录逐字段相等」旧表述 ⇒ 未完成 |
| 1.4 | 待片2 | 代码面**已见修正**：`cas_internals.inc.rs:35-36`（「(None,None) 才写入并 revision+1」）与 `:55-58`（对齐 `policy_revision += 1`、「绑定不是换代」指不新建 binding 代）自洽；`:63` 的 `next.policy_revision += 1;` **保留未删** ✓。片2 未交付报告 ⇒ 不打勾 |
| 1.5 | 待片2 | 代码：`attention_termination_tests.inc.rs:828-830` 起 options 漂移断言**已翻转为「按换代放行」**（在途 RED 态，实现未落地）；悬空源路径 `:1077-1105` 仍**拒绝** options 漂移 ✓（REQ-DRG-18 未被推翻）；`automation_reenable_supersede_tests.inc.rs:533` 悬空注释保持 ✓。片2 未交付报告 ⇒ 不打勾 |
| 1.6 | 待片2 | ① ② ③ 已建：`attention_termination_supersede_tests.inc.rs:13` / `:115` / `:169`（经 `tests.rs:915` include 挂载）；④ 已建：`automation_reenable_supersede_tests.inc.rs:629`（`reenable_with_new_provider_is_accepted_in_one_step`）。**均为 RED 态**（1.1 未落地）⇒ 不打勾 |
| 2.1 | 已完成 | R4:49-70（② 三步塌缩）→ `useIssueLifecycleGeneration.ts:506-599` `reenableTerminatedEnrollment` 只剩**一条 Enable**（`:573-587`），无 Disable、无步骤①；`runReenableSequence` 全仓 0 命中 |
| 2.2 | 已完成 | R4:49-60 → `reenableSequenceRef` / `resumingSequence` 全仓 0 命中；`reenableTerminatedEnrollment` 签名为 5 参（`:506-512`，无 `sequence`）；「目标 provider 已生效 → return」早退已不存在 |
| 2.3 | 已完成 | R4:72-87（§1.2 表）→ `WorkItemPlanOptionsDialog.tsx:444`「换 provider 会先删除现有工作项组」；全仓非测试源码 `grep 重新开启后关闭再开启` **0 命中**（仅测试负向断言 `WorkItemPlanOptionsDialog.reenable.test.tsx:239`） |
| 2.4 | 已完成 | R4:72-87 + R4:107-118 → `WorkItemPlanOptionsDialog.tsx:530-538`（`SubmitErrorState.code: string \| null`）、`:224`（写入 `reason.code`）、`:462` 主文案 `submit-error-message`、`:463-472` 次行 `submit-error-code`（`text-xs font-normal` muted，无码不渲染） |
| 2.5 | 已完成 | R4:88-106（① 改写）+ R4:202-215（定向绿）→ 用例 `useIssueLifecycleGeneration.reenable.test.ts:635`（一次提交、无 disable/enable churn）、`:674`（provider 未变也只一条 enable）、`:689`（失败不留半提交：`puts=["enable"]`、仍 terminated）；实现见 `:573-587` |
| 3.1 | 已完成 | R3:10 + R3:30（9 类表）→ `models/automation.rs:556-575`（9 变体）、`:577-592`（`as_str`）、`:599-604`（`Conflict{current_revision, kind}`）；构造点计数：`issue_automation_store.rs` 17 + `cas_internals.inc.rs` 9 + `command_writes_impl.inc.rs` 7 = **33** |
| 3.2 | 已完成 | R3:10（判别位贯通）→ `issue_automation_store.rs:36-52`（`CasResolution::Conflict{kind}`）；6 个 `resolve*` 均带 `kind` 透传（`cas_internals.inc.rs:25-32`、`:108-111`、`:137-140`、`:163-166`、`:391` 等）；`current_revision:` 全仓 46 行（≥45） |
| 3.3 | **未证实** | R3:22 + R3:87-93 明确称「**拆 if** ×2（terminate/rebind）+ revision 过期 → 2」；**代码中不存在**：`command_writes_impl.inc.rs:43-50`（`enrollment_id … \|\| expected_policy_revision …` 仍合并 → `BindingIdentityMismatch`）、`:127-131`（`!saved.enabled \|\| expected_policy_revision …` 仍合并 → `BindingIdentityMismatch`）；全文件 **无 `ConflictKind::RevisionStale`**；文件 mtime 08:50 后未再变动。⚠️ 配套测试 `automation_enrollment_conflict.rs:274` 起 `command_write_split_ifs_produce_distinguishable_conflict_kinds` 断言 `stale_kind == RevisionStale`、`rebind` 亦为 `RevisionStale` ⇒ **当前必然失败**。详见 §4.3 |
| 3.4 | 待切片落地 | R3:120-126（§5.2：明确**未落地**，等块3 片1/B3S1）→ `automation_enrollment.rs:582-591` 仍是 `Conflict { current_revision, kind: _ }` → `enrollment_conflict(...)` 旧码单一路由；`src/web/error.rs` 对两个新码 **0 命中**（文件 mtime 20:08 未改动）；新码目前只出现在测试断言里（`automation_enrollment_tests_reenable.inc.rs:630/648/750/754`） |
| 3.5 | 已完成 | R4:14-37（统一包装）+ R4:66（调用点）→ `useIssueLifecycleGeneration.ts:104-126` 唯一包装 `automationEnrollmentFailure(reason, reenable)`；调用点 `:401-414`（重新开启，`reenable=true`）与自动提交 catch（`reenable` 缺省）；全仓无第二套包装实现 |
| 3.6 | 已完成 | R4:14-31 → `useIssueLifecycleGeneration.ts:79-86`：「这个自动化正在运行中，不能直接换 provider。先到 驾驶舱 → 待处理 → 终止整个自动化…」（未声称全局可见）+「平台的授权记录刚发生变化，本次操作没有生效。等当前生成停下来再点一次（会自动重试）。」 |
| 3.7 | 已完成 | R4:37 + R4:107-118 → 机器码进次行（`WorkItemPlanOptionsDialog.tsx:463-472`，`text-xs` 弱化）；包装**保留** `code`/`details`（`useIssueLifecycleGeneration.ts:106-111`）；主文案行不含机器码（测试 `WorkItemPlanOptionsDialog.reenable.test.tsx:242` 断言） |
| 3.8 | 部分完成 | 用例齐备：R3:1-9（用例①-⑥）→ `automation_enrollment_tests_reenable.inc.rs:618`（①payload_immutable）/`:637`（②revision_stale）/`:655`（③两码不同）/`:684`（④其余落旧码）/`:709`（⑤终止自愈仍旧码）/`:734`（⑥判别位过 store→HTTP 不掉）；⑦ 见 `automation_enrollment_conflict.rs` 测试 `termination_api_error_keeps_legacy_code_message_and_details`。**但**：①③ 依赖 3.4（未落地）⇒ 当前必红；「老前端拿到新码不崩」无专门用例（仅 ④⑤ 的旧码回归锁）。缺：3.4 落地后复跑 |
| 4.1 | 已完成 | R4:31 + R4:64（③ 4-C 失败面）→ `useIssueLifecycleGeneration.ts:87-91`（`AUTOMATION_SWAP_INTERRUPTED_MESSAGE`「上一次的工作已经被清空…但新的没建起来。下一步：点「重试」重新开始。」）、`:588-597`（重读后 `terminated_at == null` ⇒ 抛该文案；**未**表述为「未发生任何变化」） |
| 4.2 | 已完成 | R4:88-106（⑦ 新增）+ R4:216-231（绿）→ 用例 `useIssueLifecycleGeneration.reenable.test.ts:828`（`tells the user the old work items are gone but the new ones were not built`） |
| 5.1 | 已完成 | R5:53-66（前端文件 19）+ R5:87-99 → `gate-stale-guidance.ts`（127 行，六条文案常量 `:27-50` + 分流函数 `:76-125`）；后端单一事实源 `src/web/plan_stop.rs:146` `plan_control_facts` |
| 5.2 | 已完成 | R5:87-98（分流表末行「非过期卡 … `showTerminate: true`」）→ `gate-stale-guidance.ts:76-80`（第 2 参数 `actionBlockReason: GateActionBlockReason` 必带）、`:82-87`（`actionBlockReason === null` ⇒ `showTerminate: true`） |
| 5.3 | 已完成 | R5:90 → `gate-stale-guidance.ts:35-37`（`GATE_STALE_STOPPED_CONTINUE_COPY` 指向底部输入框 +『继续自动化』）、`:39-41`（无入口版本）、`:105-112`（`showTerminate: false`）；消费点 `GatePromptEntry.tsx:801-812`（`staleGuidance.showTerminate` 才渲染 `终止此门`） |
| 5.4 | 已完成 | R5:87-98 → `gate-stale-guidance.ts:98-100`（terminated → `none`/false）、`:116-118`（failed → `continue`）、`:122-124`（推进到别的门 → `terminate_optional` + `showTerminate: true`） |
| 5.5 | 已完成 | R5:87-98 → `gate-stale-guidance.ts:27-28`（`GATE_STALE_COMPLETED_COPY` 逐字沿用「这一版已确认完成，此卡是历史记录，无需处理」）；回归锁用例 `GatePromptEntry.stale-completed.test.tsx:66` |
| 5.6 | 已完成 | R5:13-51（12 处注入 + 4 处字面量补齐）+ R5:53-66（前端 4 处 ingestion）→ wire 字段 `workspace_ws_types/out.rs:756`（`plan_continue_available: Option<bool>`）/+ `:906`；注入点实测 11 处 `project_plan_continue_available`（`manager/mod.rs:357` 定义；`attachment.rs:111`、`router.rs:381/629/647/677/697`、`decisions.rs:131/172`、`decisions/inbound.rs:267/314/570`）+ durable/observer 重建路径 `durable_projection.rs:179`（`plan_continue_available_for_record`）= **12 处**；前端 4 处：`api/types/workspace.ts:1149`、`state/workspace-ws-store-types.ts:625`+`:682`、`state/workspace-ws-store.ts:179`+`:363`、`state/workspace-observer-store.ts:418`（observer 镜像） |
| 5.7 | 已完成 | R5:67-77（独占清单外 3 个文件：`GatePromptEntry.stale-completed.test.tsx` / `workspace-observer-store.test.ts` / `workspace-ws-store.snapshot.test.ts`）→ 既有测试已同步（`GatePromptEntry.guidance.test.tsx:449-561` 新增 4 条 REQ-HGC-04 渲染面用例，含 `:515`「非过期卡仍渲染「终止此门」按钮」）；编译失败态合法终止按钮**未删**：`GatePromptEntry.compile-failure.test.tsx:280` 仍断言 `getByRole("button",{name:"终止此门"})` 可见 |
| 6.1 | 已完成 | R1:243（跨块顺序/H-2）+ Plan 片1 §157-176（理由）；代码：`tests/it_core/large_file_guard.rs:4-6`（`MAX_PRODUCT_CODE_LINES = 1200`、`SCAN_ROOTS` 含 `web/src`），`grep GatePromptEntry tests/it_core/large_file_guard.rs` **0 命中**（未入祖父表） |
| 6.2 | 已完成 | R1:13 + R1:80-86（`ALL_VERBATIM: True`，逐字搬移）→ 两份拆出物无状态/无副作用：`gate-card-rejection.tsx:1-9`（仅 `ChatEntry` 类型 + `humanizeGateProse`）、`gate-card-compile-failure.tsx:1-14`（lucide 图标 + 类型 + 视觉 token + 文案函数）；两文件**不反向依赖** `GatePromptEntry.tsx`（0 命中） |
| 6.3 | 已完成 | R1:13（新建两文件）+ R6:22（G-2 落点）→ 拆出物在 `web/src/components/chat-workspace/entries/` 下、命名沿用 `gate-*.tsx`；G-2 的分流函数/文案常量落在**同目录新文件** `gate-quota-labels.ts`（不是塞回 `GatePromptEntry.tsx`：该文件仅 `import { gateQuotaPresentation }`，`:80`）。⚠️ 字面差异：G-2 代码落在第三个新文件而非两份拆出物 —— 该选择由 Plan §6-I（`:1188-1196`）钉死，故按「已满足」判定 |
| 6.4 | 部分完成 | R1:13 + R1:127-135（行数目标）+ R1:196-207（Rust 门禁**无法运行**）→ ① 行数达标 ✓（`wc -l`：921 → 现 950，目标 ≤1000）；② 前端 vitest 全绿 ✓（R6 全量 259 files / 2633 tests passed）；③ **`large_file_guard` 从未实测绿**（R1:196-207 与 R6:194-206 均报 crate 编译被兄弟切片挡住，改用等价复现）⇒ 差 ③，留待 8.1 / 片7 |
| 6.5 | 已完成 | R1:243（MUST 先于片5/片6）+ R5（片5 于 921 行基础上施工 → 950）+ R6:24 → 顺序事实上成立：片1 拆出物已被 `GatePromptEntry.tsx:93-97` import、该文件 950 行（≤1200） |
| 7.1 | 已完成 | R6:7-14（G-2 口径）+ R6:39-52（改名落点表）→ `gate-quota-labels.ts:19-23`（平台自动重写/人工返修/你已说明）；meta 行消费处 `GatePromptEntry.tsx:595-601` 只用 b1/b2/acceptedTurns，**无 `max_repairs`**（A 池不上额度名）；非测试源码 `grep 剩余修复轮次\|还可自动修复\|你已回复过` **0 命中** |
| 7.2 | 已完成 | R6:7-13 + R6:22 → `gate-quota-labels.ts:106-115`（B1 行 + 「编译面自动重写账本」、B2 行 + 「门预算账本」）；来源常量 `:26`、`:28` |
| 7.3 | 已完成 | R6:12（不做共享余额的理由）→ `gate-quota-labels.ts:86-90`（返回类型 `sharedBalance: null`）、`:123`；`grep 共享余额` 非测试源码仅 `:42` 的「无法确认」免责常量与注释 |
| 7.4 | 已完成 | R6:12 + R6:15 → 未合并说明 `gate-quota-labels.ts:31` + `:119-122`；界面无共享余额数值（返回类型即 `null`） |
| 7.5 | 已完成 | R6:14（6-D 分流）→ `gate-quota-labels.ts:59-66`（`repair_budget_exhausted`：`single_candidate` → SC 固定标签，否则中性文案）、`:34`/`:36` 常量；两套表值已改：`gate-next-step.tsx:107`、`workspace-cockpit-gate-projection.ts:20`（「次数已用完」0 命中） |
| 7.6 | 已完成 | R6:12 + R6:22 → 共池说明常量 `gate-quota-labels.ts:39`（「共用同一份额度上限」），**无任何算术合并值**；spec 侧 `human-gate-convergence/spec.md:15` 同为 MUST NOT 表述 |
| 7.7 | 已完成 | R6:15（6-F 保守呈现）→ `gate-quota-labels.ts:42`（`GATE_QUOTA_HISTORY_UNAVAILABLE = "预算历史不可用／共享余额无法确认"`）、`:102-105`（`ledgerHistoryUnavailable` 判定）、`:116-118`（只在缺事实时输出）；不补默认值（`:106-115` 均以 `!== null` 为条件） |
| 7.8 | 部分完成 | R6:16 + R6:162（concern 5：**6-G 未修**，「`remaining_budget` 仍是卡片创建时快照、非实时；本片只做命名与来源标注」）→ 已实现「旧卡只读历史、不读成当前授权」（`gate-quota-labels.ts:102-118` = 6-F；投影层重建取新快照由既有 F-49 用例 `workspace-cockpit-projection.gate.test.ts:838-855` 覆盖）；**未实现**「当前卡重建/重连 SHALL 刷新同门当前事实」的实时化 —— Plan §6-G（`:1166-1170`）明确降级为另立项 ⇒ 差实时刷新 |
| 7.9 | 已完成 | R6:39-52（改名落点表 8 处逐条 ✅）+ R6:24 → `GatePromptEntry.tsx:595-601`（meta 行改为消费 `gateQuotaPresentation`）、`gate-next-step.tsx:107`、`workspace-cockpit-gate-projection.ts:20`（第二套表）均落地；另三处连带落点亦实测：`PlanRepairReadOnlyPanel.tsx:84`、`CockpitEscalation.tsx:180`、`workspace-cockpit-projection-selectors.ts:35`，`ChatInputBar.tsx:510-513` |
| 7.10 | 已完成 | R6:52（「明确未改」）→ `git status --short` 对 `StructuredOutputDiagnostic.tsx`、`pages/CodingWorkspacePage.tsx`、`coding-workspace/CodingProviderConfigPanel.tsx` **输出为空**（未触碰） |
| 7.11 | 已完成 | R6:18-37（16 文件清单）+ R6:86-105 → 新文案在测试中的出现计数：`GatePromptEntry.test.tsx` 9 处、`GatePromptEntry.guidance.test.tsx` 8 处、`p1-entries.test.tsx` 2 处、`PlanRepairReadOnlyPanel.test.tsx` 1 处、`ChatCockpitPage.inbox.test.tsx` 1 处、`ChatCockpitPage.plan-continue.test.tsx:247`（「自动化停在等你说明」）= **21 处 ≥ 14** |
| 7.12 | 已完成 | R6:107-148（六场景 TDD）+ R6:152-160（定向全绿）→ `gate-quota-labels.test.ts:14`（场景1 两来源+未合并）/`:37`（场景2 结构阻断「次数用完」不出现）/`:56`（场景2b SC 分流）/`:71`（场景3 停止原因与正余额并存）/`:88`（场景4 M:3→2、C:0→1）/`:112`（场景5 旧数据缺事实）/`:127`（场景6 不产生合并余额）；「三笔原始计数仍可见」见 `GatePromptEntry.guidance.test.tsx:606`。⚠️ 场景4 的「重建/重连不回填」按 R6:109 注释**只覆盖呈现层**（快照层问题归 7.8） |
| 8.1 | 待片7 | 无片7 报告；`large_file_guard` 至今未实测绿（R1:196-207、R6:194-206 均因 crate 编译被兄弟切片挡住） |
| 8.2 | 待片7 | 无片7 报告（`assert_aggregate_scope_discipline` 属隔离/纪律测试组，本轮无执行证据） |
| 8.3 | 待片7 | 无片7 报告；片级证据存在（R6:152-160 全量 259 files / 2633 passed）但那是片6 收口态，非 8.3 的「统一全量门禁」 |
| 8.4 | 待片7 | 无片7 报告（controller 重发前统一跑，尚未发生） |
| 9.1–9.8 | 待片7（真机） | 无片7 报告；R4:340 明示「**未做真机联调**」（新稳定码需片3 后端起效，而 3.4 仍未落地）⇒ 8 条真机验收全部不可能已执行 |
| 10.1 | 已完成 | 产物在位并已核：`proposal.md` / `design.md` / `tasks.md` + `specs/work-item-group-autopilot/spec.md:3`（MODIFIED REQ-WIGA-02）`:101`（MODIFIED REQ-WIGA-09）`:187`（ADDED REQ-WIGA-SWAP-01）`:220`（ADDED REQ-WIGA-SWAP-02）+ `specs/human-gate-convergence/spec.md:3`（MODIFIED REQ-HGC-01）`:~99-103`（ADDED REQ-HGC-04） |
| 10.2 | 部分完成 | 缺陷单已落盘：`cadence/notes/2026-11-03_独立缺陷_自动重写额度共池未实现_v1.0.md:1-3`（「状态：**待立项**」），且明确「不属块1 范围」✓；但**独立 OpenSpec change 尚未开**（`openspec/changes/` 仅 archive + 4 个既有 change，无该缺陷的 change 目录），且该项是片7 Step 3 的交付物（无片7 报告）⇒ 差立项 |
| 10.3 | 已完成 | `design.md:107`（「REQ-WSC-21 的预算隔离与共池执法不在本 change 范围，不得因 G-2 完成而把 REQ-WSC-21 标记为已修复」）+ `specs/human-gate-convergence/spec.md:96`（场景「共池权威事实缺席时只验收呈现修复」）+ `cadence/notes/2026-11-03_块1G2按最终裁决修订_实施报告_v1.0.md:153`/`:192` |
| 10.4 | 待片7 | 无片7 报告；现有 `2026-11-03_审查_块1…` 系列均为**实施前**审查（接地/一致性/双审），非「归档前独立审查结论」 |

---

## 3. 未勾选 25 条的具体原因

### 3.1 待切片落地（20 条）

| 任务 | 归属切片 | 具体原因（当前代码/报告状态） |
|---|---|---|
| 1.1 | 片2（在途） | `cas_internals.inc.rs:716-727` 仍为合并判定：`options` 漂移在**两条路径**都触发 `SourceTargetDrift` Conflict。片2 的 RED 测试已落（`attention_termination_tests.inc.rs:828`、「正常路径放行 options 漂移」），实现未落 |
| 1.2 | 片2（在途） | 同上；判据仍由 `supersede_terminated_enrollment` 入口（terminated 记录上的 Enable）决定，未引入「options 不同才换代」，也尚未体现「按路径」 |
| 1.3 | 片2（在途） | 5 处注释中 2 处已改（`issue_automation_store.rs:91-92`、`:163`）、1 处按要求保持（`:106`）、2 处未改（`cas_internals.inc.rs:656`、`:720`） |
| 1.4 | 片2（在途） | 注释已自洽（`cas_internals.inc.rs:35-36`、`:55-58`），版本号递增保留（`:63`）；**缺片2 交付报告**，故不勾（按规则不凭代码自判为「已完成」，因归属切片仍在途、可能再被改写） |
| 1.5 | 片2（在途） | options 漂移断言已翻转（`:828` 起），悬空源仍拒（`:1077-1105`）；实现未落地 ⇒ 翻转后的断言必红；且 `automation_reenable_supersede_tests.inc.rs:617` 附近的注释更新未见（仅 `:533` 保持原样） |
| 1.6 | 片2（在途） | 用例 ①②③④ 已全部建好但为 RED（1.1 未落地） |
| 3.4 | 片3 跨块半支（等块3 片1） | `automation_enrollment.rs:582-591` 仍是 `kind: _` → 单一旧码；`src/web/error.rs` **无** `automation_enrollment_payload_immutable` / `automation_enrollment_revision_stale` 条目（0 命中）。片3 报告 §5.2（R3:120-126）自述「等 B3S1 落地后补这两处」 |
| 8.1 | 片7（controller） | `large_file_guard` 未实测绿；Rust 侧单测全绿亦无统一运行证据 |
| 8.2 | 片7 | 隔离与纪律测试（`assert_aggregate_scope_discipline` 等）无执行证据 |
| 8.3 | 片7 | 前端 vitest 全绿的「收口态」证据只有片级复跑（R6），非 8.3 要求的门禁运行 |
| 8.4 | 片7 | 全量回归由 controller 在重发前统一跑，尚未发生 |
| 9.1 | 片7（真机） | 需真实后端；且 3.4（新稳定码）未落地 ⇒ 「不再出现英文机器码」的实测前提不成立 |
| 9.2 | 片7（真机） | 同上，需真机 + 驾驶舱操作 |
| 9.3 | 片7（真机） | 需真机验证「一次操作换成功」；后端 1.1 放宽未落地 ⇒ 当前真机必失败 |
| 9.4 | 片7（真机） | 需后台日志观测 |
| 9.5 | 片7（真机） | 需真机复现「换代后失败」 |
| 9.6 | 片7（真机） | 需跑一轮完整生成验证产物 provider |
| 9.7 | 片7（真机） | 门卡分流已实现（5.x），但「停在待人工」的真机场景需实测 |
| 9.8 | 片7（真机） | 额度呈现三要点已实现（7.x），但真机双来源事实需实测 |
| 10.4 | 片7 | 「归档前独立审查结论」= 片7 Step 5，未发生（现有审查均为实施前） |

### 3.2 部分完成（4 条）

| 任务 | 已完成什么 | 还差什么 |
|---|---|---|
| 3.8 | 用例 ①–⑦ 已写：`automation_enrollment_tests_reenable.inc.rs:618/637/655/684/709/734` + `automation_enrollment_conflict.rs` 的 ⑦（终止路径码/文案/details 逐字不变）；「终止自愈仍生效」有 ⑤⑦ 锁 | ①③ 依赖 3.4（HTTP 分流未落地）⇒ 现在必红；「老前端拿到新码不崩」无专门用例；需 3.3/3.4 落地后复跑 |
| 6.4 | ① 行数达标（921 → 现 950 ≤ 1000 目标，硬线 1200 未入祖父表）；② 前端 vitest 全绿（R6 全量 259 files / 2633 passed） | ③ `large_file_guard` **从未实测绿**（R1:196-207、R6:194-206 均因 crate 编译被兄弟切片挡住，仅等价复现）⇒ 待 8.1 / 片7 复跑 |
| 7.8 | 「旧卡只读历史、不读成当前授权」：`gate-quota-labels.ts:102-118`（6-F 保守呈现）；重建取新快照的投影层行为已有既有 F-49 用例（`workspace-cockpit-projection.gate.test.ts:838-855`） | 「当前卡重建 / 重连 SHALL 刷新同门当前事实」的**实时化未做**：`remaining_budget` 仍是卡片创建时快照（R6:16、R6:162；Plan §6-G `:1166-1170` 明确降级「另立项」） |
| 10.2 | 独立缺陷单已落盘（`cadence/notes/2026-11-03_独立缺陷_自动重写额度共池未实现_v1.0.md`，含契约条数、代码实际做法、后果），并明确不在本 change 修 | 未开**独立 OpenSpec change**（`openspec/changes/` 无对应目录；缺陷单状态仍为「待立项」）；该项属片7 Step 3，无交付报告 |

### 3.3 未证实（1 条）—— ⚠️ 需 controller 复核

| 任务 | 报告怎么说 | 代码查证结果 |
|---|---|---|
| 3.3 | R3:22「**拆 if** ×2（终止路径 / rebind 路径）」；R3:87-93 明确列「terminate_plan 身份不符（拆 if 后）→4 / revision 过期（拆 if 后）→**2**」「rebind 未启用（拆 if 后）→4 / revision 过期（拆 if 后）→**2**」 | `src/product/issue_automation_store_parts/command_writes_impl.inc.rs`**仍是合并 if**：`:43-50` `if saved.enrollment_id != expected_enrollment_id \|\| expected_policy_revision != saved.policy_revision` → 单一 `kind: ConflictKind::BindingIdentityMismatch`；`:127-131` `if !saved.enabled \|\| expected_policy_revision != …` → 同样单一 `BindingIdentityMismatch`。**全文件无 `ConflictKind::RevisionStale`**（全仓 `RevisionStale` 仅 2 个构造点：`issue_automation_store.rs:279`、`cas_internals.inc.rs:707`）。文件 mtime **08:50:02**，查证时（09:4x）未变 |

**后果**：`automation_enrollment_conflict.rs:274-400` 的测试 `command_write_split_ifs_produce_distinguishable_conflict_kinds` 断言 `stale_kind == ConflictKind::RevisionStale`（终止路径）与 rebind 的 `RevisionStale` ⇒ **当前代码必红**。该类差异对用户无感（都落回旧码），但测试红意味着「片3 已完成」这一状态不成立，需片3 复跑补齐（可能是在途/回退）。

---

## 4. 亲自查证的代码位置清单（文件:行）

**后端（Rust）**
- `src/product/models/automation.rs:556-575`（`ConflictKind` 9 变体）、`:577-592`（`as_str`）、`:599-604`（`Conflict{current_revision, kind}`）、`:618`（`conflict_revision` 访问器）
- `src/product/issue_automation_store.rs:36-52`（`CasResolution::Conflict{kind}`）、`:88-92`、`:102-106`、`:163`（注释）、`:279`（`RevisionStale` 构造点）
- `src/product/issue_automation_store_parts/cas_internals.inc.rs:25-32`（resolve 透传 `kind`）、`:35-36`、`:55-58`、`:63`（`policy_revision += 1` 保留）、`:108-111`、`:137-140`、`:163-166`、`:391`（其余 resolve 透传）、`:656`、`:663-727`（`supersede_terminated_enrollment` 判定未分路径）、`:707`（`RevisionStale` 构造点）
- `src/product/issue_automation_store_parts/command_writes_impl.inc.rs:43-50`、`:127-131`（合并 if 未拆）、`:31/:48/:56/:124/:130/:158`（`kind` 构造点 7 处）
- `src/web/handlers/automation_enrollment.rs:582-591`（`kind: _` + 旧码单一路由）
- `src/web/handlers/automation_enrollment_conflict.rs:63` 区（`{ current_revision, .. }`）、`:227-246`（⑦ 回归锁测试）、`:274-400`（⑧ 拆 if 判别位测试）
- `src/web/error.rs`（新稳定码 0 命中）
- `src/web/plan_stop.rs:40/80/135/146/214/251/269/303`（单一事实源全函数面）
- `src/web/workspace_ws_types/out.rs:756`（wire 字段）、`:906`（字面量补齐）
- 12 处注入点：`manager/mod.rs:357`、`manager/attachment.rs:111`、`manager/durable_projection.rs:179`、`router.rs:381/629/647/677/697`、`workspace_ws_handler/decisions.rs:131/172`、`workspace_ws_handler/decisions/inbound.rs:267/314/570`
- 测试：`src/product/issue_automation_store/attention_termination_tests.inc.rs:746`、`:828-830`、`:1002-1105`；`attention_termination_supersede_tests.inc.rs:13/115/169`；`issue_automation_store/tests.rs:915`（include）；`src/web/handlers/automation_reenable_supersede_tests.inc.rs:533/629`；`src/web/handlers/automation_enrollment_tests_reenable.inc.rs:618/637/655/684/709/734`
- `tests/it_core/large_file_guard.rs:4-6`（1200 硬线 + SCAN_ROOTS；`GatePromptEntry` 未登记）

**前端（TS/TSX）**
- `web/src/components/lifecycle/useIssueLifecycleGeneration.ts:79-91`（三条中文成品文案常量）、`:104-126`（唯一包装 + 保留 code/details）、`:399-415`（terminated 分支）、`:506-599`（`reenableTerminatedEnrollment` 唯一 Enable）、`:573-587`（单条 enable 命令）、`:588-597`（4-C 失败面）
- `web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx:224`（写入 `code`）、`:444`（「换 provider 会先删除现有工作项组」）、`:462-472`（主文案 + 次行机器码）、`:530-538`（`SubmitErrorState.code`）
- `web/src/components/chat-workspace/entries/GatePromptEntry.tsx:80`（import quota）、`:213-221`（`sessionStatus`/`planContinueAvailable`）、`:241-247`（quota 派生）、`:595-601`（meta 行）、`:801-812`（终止按钮分条件）；`wc -l` = 950
- `web/src/components/chat-workspace/entries/gate-stale-guidance.ts:27-50`（六条文案）、`:76-125`（分流表）
- `web/src/components/chat-workspace/entries/gate-quota-labels.ts:19-42`（名字/来源/未合并/中性/共池/历史不可用常量）、`:59-66`（6-D 分流）、`:86-124`（`gateQuotaPresentation`，`sharedBalance: null`）
- `web/src/components/chat-workspace/entries/gate-card-rejection.tsx:1-9`（纯移动 + 无反向依赖）、`gate-card-compile-failure.tsx:1-14`（同上）；`wc -l` = 95 / 167
- `web/src/components/chat-workspace/entries/gate-next-step.tsx:107`、`:252-255`、`:321`
- `web/src/state/workspace-cockpit-gate-projection.ts:20`；`state/workspace-cockpit-projection-selectors.ts:35`；`state/workspace-ws-store-types.ts:625/682`；`state/workspace-ws-store.ts:179/363`；`state/workspace-observer-store.ts:418`；`state/workspace-cockpit-projection.gate.test.ts:838-855`
- `web/src/api/types/workspace.ts:1149`
- `web/src/components/chat-workspace/ChatInputBar.tsx:510-513`；`cockpit/PlanRepairReadOnlyPanel.tsx:84`；`components/cockpit/CockpitEscalation.tsx:180`
- 测试：`useIssueLifecycleGeneration.reenable.test.ts:511/532/635/674/689/773/795/814/828/881`；`WorkItemPlanOptionsDialog.reenable.test.tsx:195/212/237-239/242`；`GatePromptEntry.guidance.test.tsx:449-561`（452/482/515/537）、`:606`；`GatePromptEntry.stale-completed.test.tsx:66/99/127`；`GatePromptEntry.compile-failure.test.tsx:280`；`p1-entries.test.tsx`；`gate-quota-labels.test.ts:14/37/56/71/88/112/127`；`ChatCockpitPage.plan-continue.test.tsx:247`

**契约 / 文档**
- `openspec/changes/.../specs/work-item-group-autopilot/spec.md:3/101/187/220`
- `openspec/changes/.../specs/human-gate-convergence/spec.md:3/15/19/28-31/83-86/96`、REQ-HGC-04 段 `:~99-151`
- `openspec/changes/.../design.md:107`
- `cadence/notes/2026-11-03_独立缺陷_自动重写额度共池未实现_v1.0.md:1-3`
- `cadence/plans/2026-11-03_计划文档_实施_块1_…v1.0.md:1494-1509`（组→切片映射）、`:1166-1170`（6-G）、`:1188-1196`（6-I）

---

## 5. 收尾自证（真实输出）

```
$ ls -la openspec/changes/fix-automation-enrollment-provider-swap/tasks.md
-rw-r--r-- 1 michaelche michaelche 16882 Nov  3 09:34 openspec/changes/fix-automation-enrollment-provider-swap/tasks.md

$ ls -la "cadence/notes/2026-11-03_task对证_fix-automation-enrollment-provider-swap_v1.0.md"
-rw-r--r-- 1 michaelche michaelche 30762 Nov  3 09:35 cadence/notes/2026-11-03_task对证_fix-automation-enrollment-provider-swap_v1.0.md

$ grep -c '^- \[x\]' openspec/changes/fix-automation-enrollment-provider-swap/tasks.md
36

$ grep -c '^- \[ \]' openspec/changes/fix-automation-enrollment-provider-swap/tasks.md
25

$ diff <(sed -E 's/^- \[[ x]\] /- [ ] /' 改前副本) <(sed -E 's/^- \[[ x]\] /- [ ] /' tasks.md)
NONE (only checkbox marks changed)

$ git status --short openspec/changes/fix-automation-enrollment-provider-swap/
?? openspec/changes/fix-automation-enrollment-provider-swap/
```

（该 change 目录整体为未跟踪状态，故 `??`；未执行任何 `git add` / `git commit`。`tasks.md` 只改勾选框：36 行 `- [ ]` → `- [x]`，其余 25 行保持，任务文字/编号/顺序/其它段落逐字未动。）
