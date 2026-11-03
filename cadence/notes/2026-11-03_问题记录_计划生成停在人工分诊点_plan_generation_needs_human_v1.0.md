# 问题记录：计划生成停在人工分诊点（`plan_generation_needs_human`）

日期：2026-11-03
状态：**owner 裁定「先记录，不解决」** —— 本文件只做问题留痕 + 认定修法，不开 OpenSpec change、不实施、不向 owner 索取现场取证数据
触发来源：owner 真机跑自动化，红卡「自动化执行失败，需要处理」+ 机器码 `plan_generation_needs_human`，同时页面内出现黄卡「计划生成停在人工分诊点」

> **元信息 1**：本记录的行号均基于 **main 分支**（commit 827a990，`.worktrees/main/`）；bugfix-b-11-02 分支同域代码行号可能偏移，引用时请按符号名定位。
>
> **元信息 2**：素材来源：2026-11-03 对 main 分支的只读根因诊断（scout 勘察 + 诊断档只读分析）；诊断过程未改任何代码。

## 现象（现场截图事实）

- 会话：`workspace_session_auto_34310029-dac5-40d8-9c34-86c8441fac7f`
- plan：`issue_work_item_plan_auto_34310029-dac5-40d8-9c34-86c8441fac7f`
- 逻辑代码库：`59851588-0612-41eb-8363-8e91473657a9` / `57007c13-3846-4109-8486-21ec60eac8f9`
- 右侧**红卡**：「自动化执行失败，需要处理」+ `plan_generation_needs_human` + 「去处理」按钮
- 右侧**绿卡**：「存在旧代绑定（只读可查）」+ `1 previous binding generation(s) kept read-only; current binding active: ...`
- 页面内**黄卡**：「计划生成停在人工分诊点」/「派发前出现需要人工处理的情况（如副作用不明）→ 可补充说明（可选）后『继续自动化』继续，保留自动化不转手动。」
- 左侧「自动执行流」：「待发起 · 选择 Provider 后开始生成」→「开始生成」

## 根因（代码级，已验证事实 F1~F7）

### F1 键空间：每 (project, issue) 一个 intent 文件，记录内身份不含 binding_version

- intent 文件 = enrollment 文件同目录同名替换 `automation-generation-intent.json`（`src/product/issue_automation_store.rs:463,499`）；每分区只有一条记录，不是 map。
- `PlanGenerationIntent` 字段：`enrollment_id / plan_id / session_id / action_key / source / options / target / phase`（`src/product/models/automation.rs:456-467`）。
- `action_key = "{enrollment_id}:{plan_id}:start_generation"`（`src/product/models/automation.rs:490-492`）；`same_identity` 逐字段比对上述身份字段，**不含 binding_version**（`src/product/models/automation.rs:480-488`）。
- `PlanGenerationPhase { Claimed, EngineStarted, ProviderDispatched, NeedsHuman }`（`src/product/models/automation.rs:469-476`）。

### F2 claim 的 fail-closed 语义：身份漂移 → Conflict（不是 NeedsHuman）

`claim_plan_generation`（`src/product/issue_automation_store.rs:457-523`）：

1. `!saved.enabled || saved.enrollment_id != enrollment_id` → Conflict（`:468-472`）
2. 需 plan_id+session_id（`:473-479`）、需 target（`:482-488`）
3. 派生 `derived`（身份取当前 enrollment 投影，phase=Claimed）（`:489-498`）
4. 既有 intent：`LegacyUnbound` → Conflict（`:504-508`）；`Current(existing)` 且 `!same_identity` → **Conflict**（`:510-514`）；否则 `Ready(existing)` **原样返回既有 phase**（`:515`）
5. 无文件 → 写 derived（phase=Claimed）（`:519`）

换代入口与身份改动：

| 入口 | 改动 | 位置 |
|---|---|---|
| `rebind` | plan_id/session_id/source/options.provider 全换新代；enrollment_id 不变；policy_revision+1；binding_version+1；旧代 current 进 previous | `src/product/issue_automation_store_parts/command_writes_impl.inc.rs:152-186` |
| `supersede_terminated_enrollment` | plan_id/session_id=None；prepare_intent_id 轮换；attention_facts 清空；**删旧代 plan-intent 与 generation-intent**（注释明写「避免身份比对 fail-closed 卡死」） | `src/product/issue_automation_store_parts/cas_internals.inc.rs:619-621`（注释）、`:698-731`、`:733-748`（删除实现） |

⇒ 换代后 identity 必变 → claim 返 `Conflict` → `start_plan_generation_once` 把它转 `Err`（`src/web/plan_generation.rs:243-249`）→ **不写红卡**。
⇒ 唯一能「换代 + 红卡 reason=plan_generation_needs_human」同时成立的路径：换代后身份字段仍全等（`same_identity` 不含 binding_version 的后果）→ 新一代继承旧 phase。

测试已固化 Conflict 语义：`src/product/issue_automation_store/tests.rs:825-853`、`src/product/issue_automation_store/target_union_rebind_tests.inc.rs:504-510`。

### F3 `phase=NeedsHuman` 的全部产出点只有两处

| 位置 | 触发 | 是否写红卡事实 |
|---|---|---|
| `src/web/plan_generation.rs:265-270` | claim 读到 `EngineStarted`/`ProviderDispatched` 且无活 run | ✅ 写 `plan_generation_dispatch_unproven`（`:271-276`） |
| `src/web/plan_generation.rs:283-288` | claim 读到 `Claimed` 且 `session.single_candidate_phase == Generate` | ❌ 只改 phase |

`continue_plan` 只把 phase 写回 `Claimed`（`src/web/handlers/workspace_sc_recovery.rs:597-604`、`:714-721`）。

### F4 病因码被同 key upsert 覆盖（诊断信息丢失机制）

- upsert 同 key **整条替换**，只冻结 `occurred_at`：`src/product/issue_automation_store_parts/command_writes_impl.inc.rs:320-329`

  ```rust
  Some(existing) => { let frozen = existing.occurred_at.clone(); *existing = fact; existing.occurred_at = frozen; }
  ```

- 两个 reason 共用同一把 key（`plan_generation_failed_fact_key` = `action_key + ":needs_human"`，`src/web/plan_generation.rs:26-33`）。
- 编排器**每 tick** 重入 `start_plan_generation_once`（`src/web/autopilot_orchestrator.rs:172`）；NeedsHuman 走 `Ok` → `clear_reconcile_backoff`、**不退避**（`src/web/autopilot_orchestrator.rs:230-238`）。
- ⇒ 下一 tick 必然把 `plan_generation_dispatch_unproven` 覆写成 `plan_generation_needs_human`；**红卡显示的码不携带原始病因**。

### F5 红卡与黄卡：同一 durable 停点、同一把 key

| | 红卡 | 黄卡 |
|---|---|---|
| 写者 | durable 事实 `AutomationAttentionFact{kind: AutomationFailed, stage: Plan}`（`src/web/plan_generation.rs:140-149`、`:125-159`、`:94`），落 `automation-enrollment.json` 的 `attention_facts`（`src/product/models/automation.rs:122-126`） | GET control 的只读投影 `stage="generation_needs_human"` + `allowed_actions=["continue_plan"]`（`src/web/handlers/workspace_sc_recovery.rs:816-822`） |
| key | `plan_generation_failed_fact_key(enrollment_id, binding.plan_id)`（`src/web/plan_generation.rs:141`） | 同一把 key（`src/web/handlers/workspace_sc_recovery.rs:298-304`） |
| 文案 | 标题常量「自动化执行失败，需要处理」`AUTOMATION_ATTENTION_AUTOMATION_FAILED_TITLE`（`src/web/automation_attention_info.rs:30`），summary 直出机器码（`:123-126`，注释「reason_code 只读不翻译」） | 「计划生成停在人工分诊点…」（`web/src/components/chat-workspace/ChatInputBar.tsx:509-517`） |
| 动作 | 只有「去处理」= 导航进 session（`web/src/components/chat-workspace/cockpit/CockpitInbox.tsx:268-304`；路由映射 `web/src/router.tsx:325-356`） | 「继续自动化」= `POST .../automation-enrollment/continue-plan`（`web/src/api/client.ts:728-734`，`web/src/components/chat-workspace/ChatInputBar.tsx:424`） |

⇒ 同一件事的两个语义面：通知面说「执行失败」，恢复面说「人工分诊点·可继续」。

### F6 契约口径（REQ-WIGA-12）

`openspec/specs/work-item-group-autopilot/spec.md:800-812`：

> ### Requirement: plan 阶段自动化通知事实与接入（REQ-WIGA-12）
> 系统 SHALL 复用批 1 durable 事实投影 DTO 与 target（不建第二模型…），追加 kind＝plan_question（choice/人工门/trust/compile 恢复门）、**automation_failed（provider/链停/清理失败）**、review_limit（plan 评审超限）；plan 侧用 workspace_session 目标。… **写者固定：trust/compile 门、其余 plan 停点（含派发前失败）**、advance/coding_start、终止清写各归其包；事实有稳定 key 与 durable 时间，**重复不增项**，写失败上抛。
>
> #### Scenario: 派发前失败无活窗口仍通知
> - **WHEN** plan 生成在 provider 派发前失败转 NeedsHuman（无活 run/对话窗口）
> - **THEN** 写 automation_failed 事实并通知；稳定 key 下重复停点不增项、零重复 provider

另有 `spec.md:491-493`（停点失败同源通知）、`spec.md:299-301`（「编排器 tick 停在该点，不自动 retry；用户显式链继续成功后才恢复」）、`spec.md:795-798`。

口径结论：红卡**是契约强制要求**（不是缺陷）；但契约把「provider 失败/链停/清理失败」合并进一个 kind，前端只能给固定标题「执行失败」⇒ **标题窄于契约**；契约只规定「重复停点不增项」，**未规定 reason_code 覆盖/保留语义** ⇒ 病因被覆盖属规范缺口。

### F7 两个死/漏分支

- `src/web/plan_generation.rs:282-290` **不可达**：`admission_outcome` 返回 `None`（允许继续）的唯一条件是 `status ∈ {Open, Running}` 且 `single_candidate_phase == Prepare`（`:355-361`），而 `:282` 判 `== Generate` ⇒ 永假。注释描述的「崩溃于预留之后、推进之前」安全网实际不存在（该情形被归为 WaitingForHuman，不写事实）。
- **成功路径不清红卡**：生成事实的 resolve 只挂在 `continue_plan`（`src/web/handlers/workspace_sc_recovery.rs:558`、`:728-733`）；`workspace_session/router.rs:95-116` 解的是 plan_gate 事实（`:341-345`），`manager/choices.rs:571` 解 choice 事实。

## 推断（基于代码 + 截图）

1. 红卡存在 ⇒ 停点在 claim 阶段：分支 1（无绑定 session `:180-182`）、2（run policy 漂移 `:207-212`）、3（durable 准入终态 `:238-240`+`:338-367`）、6（`:282-290`）**都不写红卡** ⇒ 必为分支 4/5 ⇒ durable `intent.phase == "needs_human"`。
2. `reason_code=plan_generation_needs_human` ⇒ phase 早已 needs_human（该码只在分支 4 写，而分支 4 前提就是 phase 已是 needs_human；phase 变 needs_human 只可能由分支 5 或 6 造成）⇒ **原始病因已被 `mark_plan_generation_phase(NeedsHuman)`（`:265`）覆盖，不可从当前 durable 复原**。
3. 左栏「待发起 · 选择 Provider 后开始生成」对应 `providerStatus=="not_started"`，判定为 `stage=="prepare_context" && timelineNodes.length==0`（`web/src/pages/ChatCockpitPage.tsx:1099-1100`、`:335-337`；文案 `web/src/pages/ChatCockpitPageParts.tsx:80-81`）⇒ **该 session 一个 timeline 节点都没产出**；结合 SC 侧「provider start 预留与 `single_candidate_phase=Generate` 同一次 CAS 落盘」（`src/product/lifecycle_store/workspace_single_candidate.rs:194-206`）⇒ **外部 provider 极可能从未被真正调用**。
4. 绿卡 ⇒ `binding_history.previous.len() == 1` 且当前绑定的 plan/session 都还在（live 分支 `src/web/plan_confirmed_info.rs:551-563`）⇒ **确实发生过恰好一次换代**。
5. 换代入口判定（推断）：`supersede_terminated_enrollment` 会轮换 `prepare_intent_id`（`cas_internals.inc.rs:694`），而 `plan_id = issue_work_item_plan_auto_{prepare_intent_id}`（`src/product/models/automation.rs:432-447`）；现场 plan id 后缀 = `34310029-dac5-…` ⇒ **prepare_intent_id 未被轮换 ⇒ 换代走 `rebind`**，而非 terminated 取代。

## 假设（未验证，按可能性排序）

| # | 假设 | 支持 | 反证/风险 |
|---|---|---|---|
| H1 | 分支 5（越过派发无活 run）先发生，`plan_generation_dispatch_unproven` 被下一 tick 覆写 | 唯一能产生 needs_human 的路径；「一个节点都没产出」与「派发后中断」吻合；`:265` 覆盖原 phase | provider_start_ledger 为空 + single_candidate_phase 非 Generate ⇒ 可确认「外部 provider 从未被触达」 |
| H2 | 分支 6（SC ledger 已 Generate 而 checkpoint 仍 Claimed） | 该分支存在且不写事实，首次停点不留红卡、红卡是下一 tick 才写 | **代码证明该分支不可达**（F7） |
| H3 | 换代（rebind）后新一代继承旧代 needs_human 检查点 | same_identity 不含 binding_version；rebind 不删 intent；绿卡证明换代过 | 换代若改了 plan_id/session_id ⇒ same_identity 假 ⇒ Conflict ⇒ **换代后新链根本起不来**（更隐蔽的失败形态，无红卡） |
| H4 | 事实跨代残留：红卡属于旧代，新一代只是继承了「旧代事实仍挂在 enrollment 上」 | rebind **不清 attention_facts**（`command_writes_impl.inc.rs:84-186`），对比 `supersede_terminated_enrollment` 明确 `attention_facts = Vec::new()`（`cas_internals.inc.rs:700`） | 现场 plan/session 是同一 uuid 派生对，倾向同代；事实里的 plan_id/session_id/key 实值可佐证是否同代 |

H1 与 H3 可并存：旧代在分支 5 停下 → 换代 → 新一代继承 needs_human → tick 覆写 reason → 红卡 + 黄卡同时出现。

## 分支对照与可判读字段（备查，非本次交付前置）

> 本节只记录「哪些 durable 字段能区分哪条分支」，供后续追查时对照；**本次不要求现场取证**。

路径形状：`<workspace_root>/.aria/projects/<project_id>/issues/<issue_id>/…`（`src/product/app_paths.rs:46-56`；`src/product/lifecycle_store/paths.rs:60-64`）。

**A. `automation-enrollment.json`**（`src/product/models/automation.rs:91-127`）
可判读字段：`binding_history.previous` 长度；`attention_facts[]` 中 `kind=="automation_failed"` 那条的 `key`/`plan_id`/`session_id`/`reason_code`/`occurred_at`；`prepare_intent_id`；`policy_revision`；`options.plan_approval`；`session_id`/`plan_id`/`source`。

**B. `automation-generation-intent.json`**（`src/product/models/automation.rs:456-467`）
可判读字段：`phase`（**决定性**）、`enrollment_id`/`plan_id`/`session_id`/`action_key`/`source`/`options`/`target`（用于与 A 的 current 逐字段比对是否跨代；`action_key` 是否等于 `"{enrollment_id}:{current.plan_id}:start_generation"`）。

**C. `automation-plan-intent.json`**（`src/product/models/automation.rs:426-447`）
可判读字段：`prepare_intent_id`/`plan_id`/`session_id`（换代后是否残留；rebind 不清它）。

**D. `workspace-sessions/<session_id>.json`**（`src/product/models/workspace.rs:807-895`）
可判读字段：`status`、`single_candidate_phase`、`provider_start_ledger`（长度与条目）、`repair_reservation.state`、`human_gate_snapshot.resumable`、`run_policy`、`workspace_type`。

**E. 进程日志**：`wiga autopilot reconcile failed for <project>/<issue>: plan generation claim failed: automation_enrollment_conflict: current_revision=Some(N)`（`src/web/autopilot_orchestrator.rs:232-236` + `src/web/plan_generation.rs:243-249`）⇒ **换代后新链被 Conflict 卡死**（H3 强证据）；无此日志 ⇒ 未走到 Conflict。

**F. `GET /automation/enrollment/control`** 现场响应：`stage`/`allowed_actions`/`stop_key` ⇒ `stop_key` 与红卡事实 key 应完全相等（`src/web/handlers/workspace_sc_recovery.rs:298-304`）；不等 ⇒ 红卡是跨代残留。

判读结论表：`phase=needs_human` ⇒ 认领停点确认；`engine_started`/`provider_dispatched` ⇒ 「越过派发」残留（tick 尚未覆写）；`provider_start_ledger` 空 + `single_candidate_phase=prepare` ⇒ 可证明外部 provider 从未被调用。

## 缺口清单

| # | 缺口 | 证据 |
|---|---|---|
| G1 | 病因不可恢复：phase 被覆写成 needs_human、事实 reason_code 被同 key 覆盖 ⇒ 事后无法判定「越过派发」还是「派发前」 | `src/web/plan_generation.rs:265`；`src/product/issue_automation_store_parts/command_writes_impl.inc.rs:320-329`；`src/web/autopilot_orchestrator.rs:172` |
| G2 | rebind 换代不清 intent / 不清事实：新一代被 Conflict 卡死、旧代 automation_failed 事实残留并继续投影 | `command_writes_impl.inc.rs:84-186` vs `cas_internals.inc.rs:700`/`:733-748`；`src/product/issue_automation_store.rs:509-517` |
| G3 | 黄卡跨代泄漏：`generation_intent()` 读 intent 文件**不做 same_identity 校验** ⇒ 旧代 checkpoint 给新代投影「人工分诊点」 | `src/web/plan_generation.rs:394-406`；`src/web/handlers/workspace_sc_recovery.rs:100-126` |
| G4 | 死分支：`:282-290` 不可达，「预留之后、推进之前」的崩溃安全网实际不存在 | `src/web/plan_generation.rs:238-240`、`:338-367`、`:282` |
| G5 | 成功不清红卡：生成事实的 resolve 只挂在 continue_plan | `src/web/handlers/workspace_sc_recovery.rs:558`、`:728-733`；`src/web/workspace_session/router.rs:95-116`、`:341-345` |
| G6 | 契约缺口：REQ-WIGA-12 未规定 reason_code 覆盖/保留语义与「换代后 checkpoint 归属」 | `openspec/specs/work-item-group-autopilot/spec.md:800-812` |

## 七、认定的修法（一条主线）

### 主线

把「plan 生成停在人工分诊点」从「泛化失败码 + 两张语义冲突的卡」变成**说得清、跨代干净、失败面可行动**的状态。四步按序做，前两步是治本，后两步是可行动化与配套。

### 第 1 步：停点原因不再被覆盖（治本，收益最大、改动最小）

**现状**：事实 upsert 同 key 整条替换、只冻结 `occurred_at`（`src/product/issue_automation_store_parts/command_writes_impl.inc.rs:320-329`），两个 reason 共用一把 key（`src/web/plan_generation.rs:26-33`）；编排器每 tick 重入（`src/web/autopilot_orchestrator.rs:172`）且 NeedsHuman 走 `Ok` 不退避（`:230-238`）⇒ 更具体的 `plan_generation_dispatch_unproven` 必被下一 tick 覆盖成 `plan_generation_needs_human`；intent 的 `phase` 也被 `mark_plan_generation_phase(NeedsHuman)` 抹平（`src/web/plan_generation.rs:265`）⇒ 病因事后不可复原（缺口 G1）。

**改法**：

1. 事实 upsert 同 key 时**一并冻结首次 `reason_code`**（与 `occurred_at` 同待遇），落 `command_writes_impl.inc.rs:320-329`；
2. `PlanGenerationIntent` 增 `stop_reason` 字段承载**原始停点原因**，落 `src/product/models/automation.rs:456-467` 与 `src/web/plan_generation.rs:250-277`；`phase` 归一为 needs_human 时不再抹掉原因。

**效果**：红卡上显示的就是病因码；事后能判「越过派发」还是「派发前」，不再只剩一句泛化英文。

### 第 2 步：换代必须清旧代检查点与事实（治本）

**现状**：`rebind` 不删 `automation-generation-intent.json` / `automation-plan-intent.json`、也不清旧代 `attention_facts`（`src/product/issue_automation_store_parts/command_writes_impl.inc.rs:84-186`）⇒ 新一代被 `Conflict` 卡死（`src/product/issue_automation_store.rs:509-517`，无红卡、更隐蔽）或在身份恰好全等时**继承旧代停点**；旧代事实残留并继续投影。对照组：「终止后取代」已做清理（`src/product/issue_automation_store_parts/cas_internals.inc.rs:700`、`:733-748`，注释明写「避免身份比对 fail-closed 卡死」）。

**改法**：

1. rebind 换代时按「终止后取代」同款清理（删两个 intent 文件 + 清旧代 facts）；或在 `claim_plan_generation` 里把「当前 binding 已换代且 binding_version 更大」的 identity 漂移从 `Conflict` 降级为重置为 `Claimed`。
2. **约束**：必须只对「确已换代」生效，不得打开「旧代 checkpoint 被新代继承」的口子；存量 issue 的 intent 文件处置口径需在方案里定死。

**效果**：换代后新一代干净起跑；红卡不再跨代泄漏（缺口 G2）。

### 第 3 步：失败面按契约分层 + 补齐可执行动作（可行动化）

**现状**：契约 REQ-WIGA-12（`openspec/specs/work-item-group-autopilot/spec.md:800-812`）**要求**写 `automation_failed` 事实并通知 ⇒ 红卡存在是对的、不是缺陷；但契约把「provider 失败/链停/清理失败」合并进一个 kind，前端只能给固定标题「自动化执行失败，需要处理」（`src/web/automation_attention_info.rs:30`、`:71-79`）且 summary 直出机器码（`:123-126`，注释「reason_code 只读不翻译」）；红卡只有「去处理」导航，没有「继续自动化」；黄卡文案固定写「派发前…副作用不明」，而 `dispatch_unproven` 恰恰是「**已越过派发**、外部 provider 是否被触达不可证明」。

**改法**：

1. 红卡标题按 `stage`/`reason_code` 分层（生成停点 ≠ provider 失败），summary 给中文原因、机器码移入 details（落 `src/web/automation_attention_info.rs:30`、`:71-79`、`:123-126`）；
2. 红卡补「继续自动化」动作，复用现成 `continue_plan` REST（前端 `web/src/components/chat-workspace/cockpit/CockpitInbox.tsx:268-304` + A1 DTO 增 `allowed_actions`）；
3. 黄卡文案按码分场景：reason_code 透到 control 投影，前端按码区分（落 `src/web/handlers/workspace_sc_recovery.rs:816-822` + `web/src/components/chat-workspace/ChatInputBar.tsx:509-517`）。

**效果**：owner 看到的失败面说清「是什么问题 + 下一步点哪里」，且卡片本身可解（不必先跳转）。

### 第 4 步：编排器真停住 + 诊断出口与清理（配套）

**现状**：NeedsHuman 走 `Ok` → 清退避 → 每 tick 重入并重写同 key（`src/web/autopilot_orchestrator.rs:172`、`:230-238`），与契约「编排器 tick 停在该点，不自动 retry；用户显式链继续成功后才恢复」（`openspec/specs/work-item-group-autopilot/spec.md:299-301`）不符；黄卡读 intent **不做身份校验**（`src/web/plan_generation.rs:394-406`、`src/web/handlers/workspace_sc_recovery.rs:100-126`，缺口 G3）；生成成功的 resolve 只挂在 `continue_plan`（`src/web/handlers/workspace_sc_recovery.rs:558`、`:728-733`，缺口 G5）；`:282-290` 是不可达死分支（缺口 G4）。

**改法**：

1. 给 NeedsHuman 增设独立退避通道（**不要**改成 `Err`，避免污染错误诊断与日志），落 `src/web/autopilot_orchestrator.rs:172-186`、`:230-238`；
2. control GET 增 `stop_reason` 与 `intent_generation`（是否同代），并给 `generation_intent()` 加 `same_identity` 校验，落 `src/web/handlers/workspace_sc_recovery.rs:780-850`、`src/web/types.rs`、`src/web/plan_generation.rs:394-406`；
3. 生成成功（`PlanGenerationOutcome::Running`）时 resolve 同 key 事实，落 `src/web/plan_generation.rs:293-333`；
4. 删除死分支 `:282-290` 或并入 `admission_outcome`（`:338-367`），落 `src/web/plan_generation.rs`。

**效果**：停点真的停住、不反复重写；owner 能一眼看出是否同代；恢复后红卡自动消失。

### 落地顺序与门禁

| 批次 | 内容 | 门禁 |
|---|---|---|
| 批 1 | 第 1 步 + 第 2 步 | 触及 durable 语义、跨代数据归属与存量数据处置 → **先走 OpenSpec**（建议由 design-task-sol 出方案，owner 拍板存量处置口径） |
| 批 2 | 第 3 步 | owner 可见行为变更（标题/文案/动作）→ 走 OpenSpec |
| 批 3 | 第 4 步 | 除「独立退避通道」需同批进 OpenSpec 外，其余为只读投影与幂等清理，可随后批 |

**不在本次范围**：任何代码实施、任何现场取证、存量数据处置的具体执行（只定口径，不动数据）。

## 关联

- 契约：`openspec/specs/work-item-group-autopilot/spec.md:800-812`（REQ-WIGA-12）、`:491-493`、`:299-301`、`:795-798`
- 计划：`cadence/plans/2026-09-26_实施计划_WIGAutopilot_P1后台plan链_v1.0.md:140-146,505,523`、`cadence/plans/2026-10-16_实施计划_批量2自动化入口_v1.0.md:77`
- 同族票：`cadence/notes/2026-11-02_独立票据_失败面可行动化_说清问题与给出操作_v1.0.md`（P1-17 失败面总纲；红卡 summary 直出机器码、标题窄于契约属该族，但**本条 role-chain/分诊面未被逐条覆盖**）
- 引入 role-chain/自动化通知面的提交：`49a3940`
- 同会话同分支的另一问题记录：`cadence/notes/2026-11-03_问题记录_自动化目标预检忽略所选provider_多仓自动授权受阻_v1.0.md`
