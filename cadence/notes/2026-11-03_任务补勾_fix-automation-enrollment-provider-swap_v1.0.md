# 任务补勾｜`fix-automation-enrollment-provider-swap`（块1）v1.0

- 日期：2026-11-03（本轮查证时刻约 17:0x–17:3x）
- 工作树：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- 对象：`openspec/changes/fix-automation-enrollment-provider-swap/tasks.md`（61 条）
- 判据：**当前代码实况**（逐条亲查），不按 09:34 对证快照（`2026-11-03_task对证_fix-automation-enrollment-provider-swap_v1.0.md` 已过时）；「待统一绿测」类按 controller 判据补充处理
- 本轮改动：**只改勾选框 9 处**（1.1–1.6、3.3、3.4、6.4 由 `- [ ]` → `- [x]`），任务文字/编号/顺序/行数零改动（归一化 diff 自证：`NONE (only checkbox marks changed)`）
- 材料：`2026-11-03_实施_1_2_v1.0.md`（片2 报告）、`…_实施_块1_片1/片3/片4/片5/片6…`、`…_交付说明_块1…`、`…_决策与偏离台账_v1.0.md`、`…_失败归因与断言更新_v1.0.md`、`…_基线对照证据_v1.0.md`

---

## 1. 统计

| 项 | 数量 |
|---|---|
| 总条数 | **61** |
| 已勾 `- [x]`（本轮后） | **45**（前值 36，本轮 +9） |
| 未勾 `- [ ]`（本轮后） | **16** |

**未勾 16 条逐类分布**：

| 类 | 条数 | 条目 |
|---|---|---|
| A. 部分完成（缺专门用例/缺立项） | 3 | 3.8（缺「老前端拿新码不崩」专门用例）、7.8（实时刷新因裁决另立项；后半已实现）、10.2（缺陷单已落盘、未开独立 change） |
| B. 统一绿测未执行（门禁类任务本体=跑测） | 4 | 8.1、8.2、8.3、8.4 |
| C. 真机验收未执行 | 8 | 9.1–9.8 |
| D. 归档前流程未发生 | 1 | 10.4 |

---

## 2. 已知 8 条「勾选滞后」重判结果（逐条）

| 任务 | 重判结论 | 代码实证（文件:行，当前实况） |
|---|---|---|
| 1.1 | **改勾 [x]** | `src/product/issue_automation_store_parts/cas_internals.inc.rs:724`（`options_drifted`）、`:731`（`options_drifted && original_source_dangling` → 拒 `DefensiveStateMismatch`，正常路径放行）、`:739`（`target_drifted \|\| (source_drifted && !original_source_dangling)` → `SourceTargetDrift`）。与 controller 给的证据行 `:724/731/733` 一致（`:733` 为 current_revision 行，现位于 `:732-735` 块内）。片2 报告 `实施_1_2` §一/§二 有据 |
| 1.2 | **改勾 [x]** | 入口判据=终止态而非 options：`src/product/issue_automation_store.rs:175`（`if saved.terminated_at.is_some() => supersede_terminated_enrollment`）、`:191`（terminated 对其余写命令 Conflict）；disabled（`terminated_at.is_none()`）落入普通 CAS 分支（`:201` 起账本重放/幂等分支）。无「options 不同才换代」判据 |
| 1.3 | **改勾 [x]** | ① `cas_internals.inc.rs` 原 `:665-670` 内联注释（HEAD 定位核实）已被「授权漂移按路径分治」新注释块替换（现 `:712-723`）；② `issue_automation_store.rs:88-92`（`compare_and_set` 文档：正常重开放行 options 漂移）；③ `:103-109`（悬空源专用 CAS 文档，**「options/target 仍逐字段相等」按要求保留**于 `:106`）；④ `:159-165`（terminated 分支分派注释更新） |
| 1.4 | **改勾 [x]** | `cas_internals.inc.rs:53-57`（原 `:49-50` 自相矛盾注释已改为「绑定补全会把 policy_revision += 1——『绑定不是换代』指不新建 binding 代」）；`:61` `next.policy_revision += 1;` **保留未删**；原 `:31` 文档头（revision+1 表述）本就自洽、未动 |
| 1.5 | **改勾 [x]** | 正常路径翻转：`src/product/issue_automation_store/attention_termination_tests.inc.rs:828-890`（原 `:805-819` 用例翻转为「options 漂移按换代放行」＋换代留痕断言 `:870-890`）；悬空路径保持拒绝：`:1077-1106`（原 `:1098-1112`，`assert_conflict` 于 `:1095-1106`）——REQ-DRG-18 未被推翻；`src/web/handlers/automation_reenable_supersede_tests.inc.rs:533-535`（原 `:530` 注释）与 `:627-630`（原 `:617` 注释，④ 用例）按新契约更新，`:609-624` 断言 options/target 不变 |
| 1.6 | **改勾 [x]** | ① 正常路径新 provider 一步接受+换代：`attention_termination_supersede_tests.inc.rs:13`；② 悬空路径 options 不同仍拒：`:112`；③ disabled 不换代不删 plan：`:166`（binding_version 不变、prepare_intent_id 不轮换、plan 绑定保留、意图文件不删，断言 `:221-235` 区）；④ binding_version+1 且旧代进 previous：`:874-884` + `automation_reenable_supersede_tests.inc.rs:703-716`（HTTP 层 200 一步换代）。挂载：`issue_automation_store/tests.rs` 末尾 include |
| 3.3 | **改勾 [x]** | `src/product/issue_automation_store_parts/command_writes_impl.inc.rs:43-48`（terminate：身份不符 `BindingIdentityMismatch`）**与** `:49-54`（revision 过期 → `:52` `ConflictKind::RevisionStale`）已拆；rebind：`:131-136`（未启用）**与** `:137-142`（`:140` `RevisionStale`）已拆；全文件 `RevisionStale` 2 处构造点（`:52`/`:140`）。配套测试 `src/web/handlers/automation_enrollment_conflict.rs:301`（`command_write_split_ifs_produce_distinguishable_conflict_kinds`）。文件 mtime 09:37（09:34 快照后已补落）。**09:34 快照的「报告说做了、代码里找不到」疑点本轮已消解** |
| 3.4 | **改勾 [x]** | `src/web/handlers/automation_enrollment.rs:574-580`（`enrollment_api_error` 按 kind 分流：`:576` `PayloadImmutable`→`enrollment_payload_immutable`、`:578` `RevisionStale`→`enrollment_revision_stale`、`:580` `_`→`enrollment_conflict` 旧码逐字不变）；`src/web/error.rs:287-288` 两新稳定码 → `StatusCode::CONFLICT`（409）；终止路径不改：`automation_enrollment_conflict.rs:261`（⑦ 回归锁：码/message/details 逐字不变，即使判别位为 `RevisionStale`）。两文件 mtime 09:56/10:02（块3 片1 先落、块1 片3 后落，串行合规） |

**8/8 全部改勾**，且每条均为「代码实况 + 片2/片3 报告双证据」。

---

## 3. 逐条对照表（61 条全量）

> 结论列：`勾`=本轮保持或改勾；`未勾`=保持未勾。代码位置为本轮亲自查证的当前行号（文件仍在收尾的已标 mtime）。

| 任务 | 结论 | 代码位置（文件:行） |
|---|---|---|
| 1.1 | 勾（本轮改） | `cas_internals.inc.rs:724/731/739`（分路径判定） |
| 1.2 | 勾（本轮改） | `issue_automation_store.rs:175/191/201` |
| 1.3 | 勾（本轮改） | `cas_internals.inc.rs:712-723`；`issue_automation_store.rs:88-92/103-109/159-165`（`:106` 悬空表述保留） |
| 1.4 | 勾（本轮改） | `cas_internals.inc.rs:53-57`（注释自洽）、`:61`（递增保留） |
| 1.5 | 勾（本轮改） | `attention_termination_tests.inc.rs:828-890/1095-1106`；`automation_reenable_supersede_tests.inc.rs:533-535/609-624/627-630` |
| 1.6 | 勾（本轮改） | `attention_termination_supersede_tests.inc.rs:13/112/166`；`automation_reenable_supersede_tests.inc.rs:632-725` |
| 2.1 | 勾（保持） | `useIssueLifecycleGeneration.ts:506-599`（`reenableTerminatedEnrollment` 唯一 Enable `:573-587`；无 Disable/步骤①） |
| 2.2 | 勾（保持） | 同文件全仓 grep `reenableSequenceRef/resumingSequence/runReenableSequence` = 0 命中；签名 5 参（`:506-512`） |
| 2.3 | 勾（保持） | `WorkItemPlanOptionsDialog.tsx:433/444`（「换 provider 会先删除现有工作项组」）；源码「重新开启后关闭再开启」0 命中 |
| 2.4 | 勾（保持） | `WorkItemPlanOptionsDialog.tsx:534`（`code: string \| null`）、`:224`（写入 `reason.code`）、`:462` 主文案、`:467` `submit-error-code` 次行 |
| 2.5 | 勾（保持） | `useIssueLifecycleGeneration.reenable.test.ts:635`（单命令无 churn）、`:674`（provider 未变也一条 enable）、`:689`（失败不留半提交） |
| 3.1 | 勾（保持） | `src/product/models/automation.rs:556-575`（`ConflictKind` 9 变体）、`:577-592`（`as_str`）、`:599-604`（`Conflict{current_revision, kind}`）；构造点实测 store 17 + cas_internals 10 + command_writes 9 = 36 ≥ 33 |
| 3.2 | 勾（保持） | `CasResolution::Conflict{kind}` 穿透：`cas_internals.inc.rs:23-31/90-99/108-114/137-146/164-173/344-352/391-400` 等 resolve 族；`current_revision:` src/ 非测试命中 49 ≥ 45 |
| 3.3 | 勾（本轮改） | `command_writes_impl.inc.rs:43-54/131-142`（拆 if；`RevisionStale` 于 `:52/:140`） |
| 3.4 | 勾（本轮改） | `automation_enrollment.rs:574-580`；`web/error.rs:287-288`；终止不改 `automation_enrollment_conflict.rs:261-296` |
| 3.5 | 勾（保持） | `useIssueLifecycleGeneration.ts:104-125`（唯一包装 `automationEnrollmentFailure(reason, reenable)`）；调用点 `:412`（reenable=true）/`:629` |
| 3.6 | 勾（保持） | 同文件 `:82`（运行中换 provider → 驾驶舱→待处理→终止整个自动化，未声称全局可见）、`:86`（「平台的授权记录刚发生变化，本次操作没有生效。等当前生成停下来再点一次（会自动重试）。」） |
| 3.7 | 勾（保持） | `WorkItemPlanOptionsDialog.tsx:462-472`（机器码次行 `submit-error-code`）；包装保留 code/details（`useIssueLifecycleGeneration.ts:106-111`） |
| 3.8 | **未勾** | 用例①-⑥+⑦⑧齐备：`automation_enrollment_tests_reenable.inc.rs:614/633/651/679/704/729` + `automation_enrollment_conflict.rs:261/301`；**缺「老前端拿到新码不崩」专门用例**（见 §4） |
| 4.1 | 勾（保持） | `useIssueLifecycleGeneration.ts:90-91`（`AUTOMATION_SWAP_INTERRUPTED_MESSAGE`「上一次的工作已经被清空…但新的没建起来。下一步：点『重试』重新开始。」）、`:588-597`（重读非 terminated ⇒ 抛该文案；注释明示 MUST NOT「未发生任何变化」） |
| 4.2 | 勾（保持） | `useIssueLifecycleGeneration.reenable.test.ts:828`（old work items gone / new not built） |
| 5.1 | 勾（保持） | `gate-stale-guidance.ts`（127 行：常量 `:27-50` + 分流 `:76-125`）；后端单一事实源 `src/web/plan_stop.rs:269`（`plan_continue_available_for_record`）等 |
| 5.2 | 勾（保持） | `gate-stale-guidance.ts:79`（第 2 参 `actionBlockReason` 必带）、`:82-88`（`actionBlockReason === null` ⇒ `showTerminate: true`） |
| 5.3 | 勾（保持） | `gate-stale-guidance.ts:106-113`（`stopped_needs_human` + `planContinueAvailable===true` → continue/无终止按钮；无入口版本 `:114-120`） |
| 5.4 | 勾（保持） | `gate-stale-guidance.ts:96-104`（terminated → none）、`:121-127`（failed → continue）、`:82-88`（推进到别的门 → terminate_optional + showTerminate） |
| 5.5 | 勾（保持） | `gate-stale-guidance.ts:27-28`（`GATE_STALE_COMPLETED_COPY` 历史记录语义保持）；回归锁 `GatePromptEntry.stale-completed.test.tsx` |
| 5.6 | 勾（保持） | wire 字段 `workspace_ws_types/out.rs:756/906`；注入 11 调用点（`manager/attachment.rs:111`、`router.rs:381/629/647/677/697`、`workspace_ws_handler/decisions.rs:131/172`、`decisions/inbound.rs:267/314/570`）+ 定义 `manager/mod.rs:357` + durable 重建 `durable_projection.rs:178-181`；前端 4 处：`api/types/workspace.ts:1149`、`workspace-ws-store.ts:179/363`、`workspace-observer-store.ts:418`（observer 镜像）、store 类型 |
| 5.7 | 勾（保持） | `GatePromptEntry.guidance.test.tsx:449-482+`（REQ-HGC-04 渲染面 4 用例）；编译失败态「终止此门」保留：`GatePromptEntry.compile-failure.test.tsx:280`（文件 mtime 11-02 20:08 未动） |
| 6.1 | 勾（保持） | `tests/it_core/large_file_guard.rs:4-6`（1200 硬线）；`GatePromptEntry` 不在祖父表；拆分后 950 行 |
| 6.2 | 勾（保持） | `gate-card-rejection.tsx`（95 行）/`gate-card-compile-failure.tsx`（167 行），各仅 1 处类型引用 `GatePromptEntry`、无反向依赖 |
| 6.3 | 勾（保持） | 拆出物在 `web/src/components/chat-workspace/entries/`、`gate-*.tsx` 命名；G-2 常量/分流落 `gate-quota-labels.ts`，`GatePromptEntry.tsx:80` 仅 import |
| 6.4 | 勾（本轮改，**待统一绿测**） | ① `wc -l GatePromptEntry.tsx` = **950 ≤ 1000**（本轮实测）；③ 前端 vitest 全绿有实证（片5 §6.4 + 交付说明 §:141：260 files / 2656 tests / exit 0，拆分后运行）；② `large_file_guard` 实跑未发生 ⇒ 按「待统一绿测」判勾（若绿测证明缺陷改回，见 8.1） |
| 6.5 | 勾（保持） | 事实顺序成立：片1 拆分（11-02 深夜～11-03 晨）先于片5/片6；`GatePromptEntry.tsx` 950 ≤ 1200 |
| 7.1 | 勾（保持） | `gate-quota-labels.ts:19-23`（平台自动重写/人工返修/你已说明）；A 池不上额度名（`:14` 注释 + meta 行无 max_repairs） |
| 7.2 | 勾（保持） | `gate-quota-labels.ts:26-28`（来源常量）、`:106-115`（B1/B2 两行各自标注） |
| 7.3 | 勾（保持） | `gate-quota-labels.ts:86-90/123`（`sharedBalance: null` 类型层恒空）；无合并值 |
| 7.4 | 勾（保持） | `gate-quota-labels.ts:31`（`GATE_QUOTA_LEDGERS_UNMERGED_NOTE`「两处账本尚未合并」）+ `:119-122` |
| 7.5 | 勾（保持） | `gate-quota-labels.ts:34-36`（中性文案/SC 固定标签）、`:59-66`（6-D 分流）；两套表值已改：`gate-next-step.tsx:107`、`workspace-cockpit-gate-projection.ts:20`（「次数已用完」0 命中） |
| 7.6 | 勾（保持） | `gate-quota-labels.ts:39`（共池说明「共用同一份额度上限」，无算术合并值） |
| 7.7 | 勾（保持） | `gate-quota-labels.ts:42`（「预算历史不可用／共享余额无法确认」）、`:101-105`（缺事实判定）、`:116-118`（只在缺时输出、不补默认） |
| 7.8 | **未勾** | 后半已实现（旧卡只读历史：`gate-quota-labels.ts:101-118`）；前半（重建/重连实时刷新）**因裁决作废面**（见 §4） |
| 7.9 | 勾（保持） | `GatePromptEntry.tsx:595-601`（meta 行消费 `gateQuotaPresentation`）、`gate-next-step.tsx:100-107`、`workspace-cockpit-gate-projection.ts:16-21`（第二套表） |
| 7.10 | 勾（保持） | `git status --short` 对 `StructuredOutputDiagnostic.tsx`、`CodingWorkspacePage.tsx` 输出为空（未触碰） |
| 7.11 | 勾（保持） | 新文案在测试中出现 ≥21 处（09:34 清点 21；相关测试文件 mtime ≤09:07 未再动） |
| 7.12 | 勾（保持） | `gate-quota-labels.test.ts:14/37/56/71/88/112/127`（六场景）；三笔原始计数 `GatePromptEntry.guidance.test.tsx:606` |
| 8.1 | **未勾** | 见 §4 |
| 8.2 | **未勾** | 见 §4 |
| 8.3 | **未勾** | 见 §4 |
| 8.4 | **未勾** | 见 §4 |
| 9.1 | **未勾** | 见 §4 |
| 9.2 | **未勾** | 见 §4 |
| 9.3 | **未勾** | 见 §4 |
| 9.4 | **未勾** | 见 §4 |
| 9.5 | **未勾** | 见 §4 |
| 9.6 | **未勾** | 见 §4 |
| 9.7 | **未勾** | 见 §4 |
| 9.8 | **未勾** | 见 §4 |
| 10.1 | 勾（保持） | `proposal.md`/`design.md`/`tasks.md` + `specs/work-item-group-autopilot/spec.md:3/101/187/220`（MODIFIED REQ-WIGA-02/09 + ADDED SWAP-01/02）+ `specs/human-gate-convergence/spec.md:3/100`（MODIFIED REQ-HGC-01 + ADDED REQ-HGC-04） |
| 10.2 | **未勾** | 见 §4 |
| 10.3 | 勾（保持） | `design.md:107`（契约边界：不得因 G-2 把 REQ-WSC-21 标记已修复） |
| 10.4 | **未勾** | 见 §4 |

---

## 4. 未勾 16 条的具体原因

| 任务 | 具体原因（缺什么） | 归属 |
|---|---|---|
| 3.8 | 用例①-⑥（`automation_enrollment_tests_reenable.inc.rs:614/633/651/679/704/729`）+⑦⑧（`automation_enrollment_conflict.rs:261/301`）已落地，「两条路径不同稳定码」③ 与「终止自愈仍生效」⑤⑦ 有直接用例；**缺「老前端拿到新码不崩」专门用例**（本轮 grep `老前端/旧客户端/不崩/未知` 于相关测试 0 命中；交付说明 §:108 同判）。另：统一绿测未跑（该批用例无绿输出） | 片3 补专门用例 + 片7/controller 统一绿测 |
| 7.8 | 后半（旧卡只读历史、不读成当前授权）已实现：`gate-quota-labels.ts:101-118`；**前半（当前卡重建/重连 SHALL 刷新同门当前事实）因裁决作废**——按块1 Plan §6-G（`:1166-1170`）降级「另立项」，台账 §:139 登记为已批准偏离（`remaining_budget` 仍是卡片创建时快照、非实时） | 独立立项（不在本 change 修） |
| 8.1 | 授权持久化相关单测无统一绿输出：10:17 统一跑（实为 `--lib`）**编译失败终止、0 用例执行**（`/tmp/aria-full-lib-test.log`，28 errors）；`large_file_guard`（在 `tests/it_core/`，`--lib` 不覆盖）**从未实跑绿**（片1 §3.4、片6 concern 2 均被 crate 编译挡住）；13:27 全量 `cargo test --tests` 为 **612 FAILED**（归因：块3「单仓入口删除」夹具缺前置，收敛进行中——`失败簇收敛_*` 3 份笔记 16:11–17:05 仍在途） | 片7 / controller 收口统一跑 |
| 8.2 | `assert_aggregate_scope_discipline` 等隔离/纪律测试无任何执行证据（本轮 notes 全量检索 0 命中运行记录） | 片7 / controller 收口统一跑 |
| 8.3 | 前端 vitest 只有「收口前基线」绿证（260 files / 2656 tests / exit 0，片5 §6.4 + 交付说明 §:141）；8.3 要求收口态全绿——收口复跑未发生，且 `web/src` 正被收尾 worker 改动中（FixWebCluster 在途）⇒ 该结论待复核 | 片7 / controller 收口复跑 |
| 8.4 | 全量回归（Rust lib + it_core + it_web + 前端 test/build）由 controller 在重发前统一跑——尚未发生（612 FAILED 收敛仍在途） | controller |
| 9.1 | 真机验收未执行（需真实后端 + 驾驶舱操作）；台账 §:144 明示 8 条全部未执行。注：其前置 3.4 新稳定码**现已落地**，真机前置成立、只差执行 | 片7 / 真机环境 |
| 9.2 | 真机验收未执行（同上） | 片7 / 真机环境 |
| 9.3 | 真机验收未执行；后端 1.1 放宽**现已落地**（本轮实证），只差真机跑 | 片7 / 真机环境 |
| 9.4 | 真机验收未执行（需后台日志观测 reconcile） | 片7 / 真机环境 |
| 9.5 | 真机验收未执行（需真机复现「换代后失败」） | 片7 / 真机环境 |
| 9.6 | 真机验收未执行（需跑一轮完整生成验证产物 provider） | 片7 / 真机环境 |
| 9.7 | 门卡分流已实现（5.3 实证），但「停在待人工」真机场景未实测 | 片7 / 真机环境 |
| 9.8 | 额度呈现已实现（7.1-7.7 实证），但真机双来源账面事实未实测 | 片7 / 真机环境 |
| 10.2 | 缺陷单已落盘：`cadence/notes/2026-11-03_独立缺陷_自动重写额度共池未实现_v1.0.md`（状态「**待立项**」）；**缺独立 OpenSpec change**——本轮实测 `openspec/changes/` 仅 archive + 3 个既有 change（fix-aggregate…/fix-automation…/mechananicalize…/remove-single…），无该缺陷目录 | owner / controller 立项（片7 Step 3） |
| 10.4 | 「归档前独立审查结论」未发生——现有 `审查_块1*` 系列均为**实施前**审查（接地/一致性/双审），非归档前独立审查 | 片7 / 收口 |

---

## 5. 「报告说做了、代码里找不到」排查结论

- **本轮未发现新条目**。09:34 快照的唯一疑点 **3.3**（片3 报告称「拆 if ×2」、彼时代码仍是合并 if）已随 09:37 的 `command_writes_impl.inc.rs` 落地**消解**（本轮实证 `:52`/`:140` 两个 `RevisionStale` 半支 + 配套测试 `:301`）。
- 片2（`实施_1_2_v1.0.md`）报告「验收输出」一节为待填占位（交付说明 §:61 已登记）——其代码与测试本轮已**全部亲查实证**，按「代码实况为据，报告验收输出未覆盖」处理（1.1–1.6 勾选依据为代码而非该空节）。

## 6. 稳定性说明（实现收尾中）

- 本轮新勾 9 条所涉文件 mtime：`cas_internals.inc.rs` 11:26、`issue_automation_store.rs` 09:32、`attention_termination_tests.inc.rs` 09:33、`attention_termination_supersede_tests.inc.rs`（新建）、`automation_reenable_supersede_tests.inc.rs` 09:33、`command_writes_impl.inc.rs` 09:37、`automation_enrollment.rs` 09:56、`models/automation.rs` 09:56、`web/error.rs` 10:02、`automation_enrollment_tests_reenable.inc.rs` 10:34、前端 5.x/6.x/7.x 文件均 ≤09:17 —— **均早于本轮查证且不在 3 个收尾 worker（失败簇收敛：workspace_engine / coding_workspace_engine / web cluster）的改动面内**，结论稳定。
- `web/src`（FixWebCluster 在途）与 `src/product/coding_workspace_engine|workspace_engine`（另两个 worker 在途）只影响 8.2/8.3/8.4 的绿测判断，已按「未执行」保守留白，无需复核。
- 6.4 的 `large_file_guard` 半支与 8.1 同源：若统一绿测暴露缺陷（如 `web/src/api/types/workspace.ts` 余量 2 行被顶破），6.4 应改回未勾并注明。

## 7. 收尾自证（真实输出见 IRC/交付消息）

- `ls -la openspec/changes/fix-automation-enrollment-provider-swap/tasks.md` + 本清单文件
- `wc -l` tasks.md = 128 行（与 09:34 版一致，零加删行）
- `grep -c '^- \[x\]'` = **45**；`grep -c '^- \[ \]'` = **16**
- 归一化 diff（勾选框抹平后对比改动前副本）：`NONE (only checkbox marks changed)`
