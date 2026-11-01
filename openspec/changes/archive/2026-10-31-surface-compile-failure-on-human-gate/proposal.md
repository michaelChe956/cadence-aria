# Proposal: surface-compile-failure-on-human-gate

## Why

真机会话 `workspace_session_auto_3b4c7476-f300-40ff-abb7-81d0956f78fd` 停在人工门，owner 原话是
「到这又卡住了，点了之后会报错，但是我看不到报错内容」。核到的事实：

1. **编译失败的原因没有到达门卡**。同一条 `dependency_cycle` 连续失败三次（timeline node
   `_013` / `_015` / `_017`），但会话 `policy_diagnostics` 为空、门快照 `findings` 为空——
   `final_compile_prepare_failed` 走的是 plain `?` 站点，既不落 `policy_diagnostics`，也没有
   任何 durable 诊断；门卡上读不到一个字。原因只活在 timeline node 的 `summary` 与本次 close 的
   内存 stash 里。
2. **摘要本身是机器话**。原文形如
   `Final Compile 失败：[final_compile_prepare_failed] dependency_cycle: change_order 与既有
   WorkItem 依赖构成环: work_item_1284b6…_003, work_item_1284b6…_004`——用哈希 id 表达，
   既没说是谁把谁写成 provider，也没说为什么成环。
3. **「确认当前版本」是死路**。`human_confirm` 之后的 Final Compile 是**必经**步骤：approve
   只把会话推到 `Running` 并立刻重跑同一次编译，compile 未落 Confirmed 就回到门。候选自身无效时
   这一循环必然重复——owner 点第 1 次、第 2 次、第 3 次，看到的都是同一个环（`_014`/`_016`
   `human_confirm completed 已确认通过`，紧接着 `_015`/`_017` 同样失败）。
4. **owner 的裁定**（2026-10-31）：
   - 「不应该出现这个成环的情况，应该拒绝」→ 编译不过的候选不应出现在 owner 可确认的集合里；
   - 「就算成环了要有醒目的提示，让用户重新生成，并提醒用户可能 design 都需要重写，重写的时候
     着重注意哪些」→ 门卡必须醒目声明候选无效、给出「让 author 重新生成」的出路与重写要点，
     且要点要同时进入给 author 的修订提示词。

## What Changes

- **完整诊断 durable 落盘**：SC initial compile 的唯一返回边界收口——失败落
  `workspace-sessions/{session}/candidate-compile-status.json`（含稳定码、完整人话、完整诊断原文，
  不截断），成功落「无失败」覆盖。与门卡其它 durable 事实（`human-dispositions.json`、
  `human-gate-rejections/{turn}.json`）同族同目录、同一 live-write 准入。
- **依赖环诊断人话化**：`apply_change_order_cross_repo_depends_on` 的环错误改为
  「真实环 witness → 人话段 + 机器 id 段」（复用 SG1 既有的确定性 witness 提取）：点名哪个
  Work Item 把哪个写成 provider、两仓在 `change_order` 的位次、为什么与仓序自动边合成环、
  以及该怎么改。找不到 witness 时给通用可行动文案，不虚构环形状。
- **门卡投影**：`session_state` 新增顶层 `candidate_compile_failure{code,message,detail?,
  owner_hint?,rewrite_guidance?}`，与 `human_gate_turn_rejection` 同层同族；只投影**当前候选**
  （source revision + candidate IR 双侧与会话 durable 值逐字相等）的失败，旧候选与缺 refs 一律不投影。
  刷新/重连后仍在（durable 读取，不依赖内存 stash）。
- **确认动作 fail-closed**：两条确认入口（`close_human_gate` Approve 臂、`handle_confirm` 的
  SC Approval 臂）共用同一平台判据；当前候选已确定编译不过时以稳定码
  `CANDIDATE_COMPILE_FAILED` 拒绝，零 durable 副作用、门保持打开，文案说明「会原样再失败一次」
  并给下一步。判据无法证明同属一个候选时 fail-open（绝不把门永久挡死）。
- **给 owner 与给 author 双份指引**：`owner_hint`（候选无效 + 让 author 重新生成 + design 可能
  需一起重写）与 `rewrite_guidance`（provider 只能指向前仓、同仓顺序只能靠 `depends_on`、依赖图
  必须无环、design 可能需同步改）。同一组要点经 `[candidate_compile_failure]` 段进入门内修订
  提示词（author 教学面），使「重新生成」有具体可执行指令。

## Capabilities

### Modified Capabilities

- `work-item-plan-single-candidate`：ADDED REQ-WSC-18（候选编译失败必须以完整可读原因呈现在人工门）、
  REQ-WSC-19（候选编译不过时确认动作不得形成死路）、REQ-WSC-20（候选编译失败必须给 owner 出路与
  author 重写要点）、REQ-WSC-21（首次失败即提示 owner 并**真正驱动**自动重写）、REQ-WSC-22（同一依赖环
  结构重复必须提前停并升级提示）。不改 REQ-WSC-01..17。

### owner 2026-10-31 追加裁定（本 change 采纳）

- **「用户确认不是必经的」**：final compile 没问题就不需要 owner 确认；因此编译类失败**不**应把候选推进
  owner 可确认的集合，而应走自动重写。
- **「能重写尽量尝试重写」**：编译类失败（`final_compile_prepare_failed` / `dependency_cycle`）必须
  **真正驱动** author 重写（复用既有机械返修通道 `route_single_candidate_contract_prerevision` →
  `apply_policy_route(TriggerAggregateRepair)` → `request_provider_run(WorkItemPlanSingleCandidateAuthor)`）。
- **「还可自动修复 3 次」＝还能让 author 重写几次**：预算按 **author 重写次数**计（初值
  `max_manual_repairs`=3），**平台自动重写与 owner 反馈重写扣同一个池**；`max_repairs`=1 是机械返修池，
  与本条无关。
- **提示时机**：第 1 次 final compile 失败就出提示，**与自动重写并行**（不是等重写失败才提示，也不是
  提示了就停）。
- **同一环重复 → 提前停**：判据是**结构**（成员集合 + 边集合的规范化摘要），与文案/行号/编译运行 id
  无关；即使预算仍有余也提前停，升级提示并要求 owner 修改 design。
- **design 根因只告知 owner**：气泡/门卡说明「由于 design 的什么原因导致成环并无法推进，需要修改
  design」；平台**不会**自动改 design；给 author 的教学提示**不得**要求它去改 design。

## 规格来源与时序

owner 2026-10-31 真机反馈与同日两条裁定；现场证据与该会话的 durable 记录（timeline nodes
`_013`..`_018`、`human_gate_snapshot.findings=[]`、`policy_diagnostics=[]`）。前端门卡呈现面由并行
改动实现，wire 形状 `candidate_compile_failure{code,message,detail?,owner_hint?,rewrite_guidance?}`
在本 change 内成文并已与前端实现对齐。

## Non-Goals

- **不新造第二条「自动重生成」通道**。既有教学重驱（REQ-WSC-09）明确只覆盖 generate 段
  parse 语法类失败与 IR 校验失败，且每 candidate 恰一次；Final Compile 段没有任何平台发起的
  author 重驱入口（只有 owner 反馈 turn 与 parse/IR 教学重驱）。因此本 change 按 owner 的
  fallback 落地：门卡醒目声明候选无效 + 确认不可用 + 给出「让 author 重新生成」的出路。
- **不改 SG1 候选结构门的自动边口径**。核到现场候选逃过候选门的原因：该候选 IR 的
  `intent_contract.target` 是 `single_repository`（不是 `LogicalCodebase`），而
  `project_effective_planning_graph` 的 change_order 自动边只从 `logical_target` 出发，于是候选门
  看不到这条环；Final Compile 的 `apply_change_order_cross_repo_depends_on` 按 projected draft 的
  `target_repository_id` 建边才炸。收窄这个口径等于改变候选门的边计算与冻结 digest，属独立
  高风险改动——另开票据，不在本 change 内偷改（详见 design.md「已知缺口」）。
- 不改 reviewer 契约、不改 workflow 状态机、不改 Work Item Plan 发布路径。
- 不改 `web/src/**`（并行前端改动的写面）。

## Impact

产品：SC compile 失败诊断（`draft_batch::compile_support` 环渲染、新模块 `compile_failure_surface`）、
durable 候选编译事实（`lifecycle_store::human_dispositions`）、门卡投影（`session_state` / `out.rs`）、
两条确认入口的 fail-closed 判据、门内修订提示词（`human_gate_revision`）。协议：
`session_state.candidate_compile_failure`（可选、缺席省略，旧客户端字节行为不变）。前端：门卡按该
字段呈现「候选无效」并禁用确认（并行改动）。验收：TDD 钉住完整落盘、当前候选绑定、刷新可读、
确认 fail-closed 零副作用、旧候选不拦、author 提示词含具体要点、无失败时既有提示词逐字不变。
