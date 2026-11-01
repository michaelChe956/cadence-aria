> Task 5.0 wire 摸底产出(2026-09-03,scout 只读实测;计划文档 §Task 5 已按此修正)。

# Code Context

## Files Retrieved
1. `src/product/coding_workspace_engine/tests/campaign_stage3_amendment.rs:760-1010` — 阶段 3 amendment 权威 E2E：暂停、原 plan session typed feedback、断线重连、出版、回原 attempt。
2. `src/web/coding_ws_handler/protocol.rs:135-148` — coding 出站两种 amendment 消息及 DTO 别名。
3. `src/web/coding_ws_handler/outbound.rs:8-98` — coding 出站写入成功/失败的 delivery ack 结算。
4. `src/product/coding_workspace_engine/plan_repair_start.rs:195-287` — 创建/校验 link 并发送 `plan_repair_required`。
5. `src/product/coding_attempt_store/amendment_delivery.rs:30-115,177-214` — durable delivery marker、幂等 mark、身份校验。
6. `src/product/coding_workspace_engine/amendment.rs:304-440,644-676` — application journal 重放、delivery 投递、resume。
7. `src/product/workspace_engine/conversational_gate.rs:86-142,188-214,248-328,518-698` — 原 plan session amendment 门与 turn 状态机。
8. `src/web/workspace_ws_handler/decisions.rs:4-59`、`decisions/inbound.rs:839-847` — child WS `confirm_plan_amendment` 入口。
9. `src/product/models/plan_repair.rs:74-95,129-159`、`src/product/models/workspace_link.rs:13-45` — 完整 wire 嵌套类型。
10. `cadence/reports/workitem-coding-campaign/coding_run_campaign.mjs:409-416,417-498` — 当前 coding driver 连接/处理及 fail-closed 分支。
11. `src/web/app.rs:397-412` — workspace/coding WS 实际路由。

## Key Code

### ① `plan_repair_required` wire
- 外层（`protocol.rs:135-138`，serde tag `type` snake_case）：
  `{"type":"plan_repair_required","request":<PlanRepairRequest>,"session_link":<WorkspaceSessionLink|null>}`。
- `request` 完整字段（`models/plan_repair.rs:76-95`）：
  `id, plan_id, base_plan_revision_id, trigger_attempt_id, trigger_unit_run_id, trigger_review_id|null, trigger_finding_id, amendment_id|null, defect_class, reason_code, repair_target, contract_refs[], capability_refs[], evidence[], fingerprint, status, created_at, updated_at`。
  `repair_target={kind,logical_work_item_ids[],work_item_revision_ids[]}`；`evidence[]={kind,source_ref,message}`（同文件 `42-62`）。
- `session_link` 不是 URL 字符串，而是完整对象（`protocol.rs:145-148`; `models/workspace_link.rs:13-45`）：
  `{id, relation, parent_session_id, child_session_id, trigger:{attempt_id,unit_run_id,review_id|null,finding_id,repair_request_id,amendment_id,fingerprint,base_plan_revision_id}, return_context:{original_attempt_id,original_unit_run_id,timeline_anchor_id,original_route}, created_at}`。
- 发送点保证 `Some(session_link)`（`plan_repair_start.rs:279-285`），并在发送前校验 request/link/attempt 全链路身份（`plan_repair_start.rs:339-366`）。
- `session_link` 没有 `ws_url`/`url` 字段；`original_route` 是 HTTP 前端相对路径，不是 WS 路径。PlanRepair 生产构造为 `/workbench/projects/{project}/issues/{issue}/coding/{attempt}`（`product/workspace_engine/plan_repair.rs:230-238`）。实际 coding WS 必须由客户端以 `ARIA_WS_BASE_URL`（默认 `BASE` http→ws）为基址拼 `/ws/projects/{project}/issues/{issue}/coding-attempts/{attempt}`（driver `coding_run_campaign.mjs:13-15,410-416`）。child workspace WS 则是 `/api/workspace-sessions/{session_id}/ws` 或 `/api/ws/workspace/{session_id}`（`web/app.rs:397-404`）。

### ② `plan_amendment_updated` wire
- 外层（`protocol.rs:139-148`）：`{"type":"plan_amendment_updated","event_id":string,"amendment":<PlanAmendmentManifest>}`。
- `amendment` 完整字段（`models/plan_repair.rs:144-159`）：
  `id, repair_request_id, previous_plan_revision_id, new_plan_revision_id, revised_work_items{}, superseded_revisions[], dependency_graph_changes[], contract_deltas[], unaffected_units[], revalidation_required_units[], stale_units[], replacement_units{}, resume_target:{logical_work_item_id,mode}, created_at`。
  `revised_work_items[id]={previous_revision_id,next_revision_id,delta_kind}`；`resume_target.mode` 为 `reexecute|revalidate|await_handoff`。
- `event_id` durable 固定为 `coding_plan_amendment_updated_{attempt_id}_{amendment_id}`（`amendment_delivery.rs:204-210`），不是随机 WS message id；round-trip 断言见 `web/coding_ws_handler/tests/plan_repair.rs:330-365`。

### ③ 原 plan session amendment 门时序
1. Coding 先进入暂停：attempt=`awaiting_plan_amendment`、触发 unit run=`blocked_by_plan_defect`、context=`open`，且 context.plan_session_id 指向原 plan session（stage3 `:768-784`）。coding WS 初始快照也会把 reconciliation 后的 `linked_plan_repair` 放入 `coding_session_state`（`coding_ws_handler/state.rs:20-26,83-89`）。
2. 用户实际连接原 plan session workspace WS，发送 `{"type":"human_gate_feedback","command_id":C,"feedback":F}`（输入字段 `workspace_ws_types/in_.rs:113-116`）。`command_id` 完全来自 driver/client；服务端验证非空且 ≤256 bytes（`conversational_gate.rs:69-79`），并以 `(session_id,command_id)` durable 查重（`:522-540`）。
3. amendment 门探测要求：WorkItemPlan + SingleCandidate、原 session 已 `Confirmed` 且 phase `Completed`、仍有 human_gate_snapshot、存在指向该 plan session 的 Open/Applying context、group attempt 为 `AwaitingPlanAmendment`（`:86-142`）。命中后 durable CAS `Confirmed→WaitingForHuman`，**沿用原快照和预算，不建第二 session**（`:634-665`）。
4. 服务端响应 `human_gate_turn_open`（`workspace_ws_types/out.rs:129-133`）：`turn_id, command_id:C, remaining_budget`；首次 turn 内部 `status=Reserved, attempt_no=1,budget_reserved=1`，预算扣 1（`conversational_gate.rs:611-629,691-698`）。handler 对新 turn 先发一次 open，再启动 provider（`workspace_ws_handler/decisions.rs:299-316`）。
5. provider 成功后发 `human_gate_turn_completed:{turn_id,artifact_ref}`；失败则 `human_gate_turn_failed:{turn_id,failure_class,message}`（`workspace_ws_handler/run/provider_run.rs:1059-1078`）。stage3 断言 turn 完成、原 session `WaitingForHuman`、预算从 2→1、provider ledger 仅一项（`:793-821`）。
6. **confirm 不是在 coding WS，也不是原 plan session `Confirm` 的 amendment 出版确认。** amendment 出版确认走 child plan-repair session WS：先 child 初始 `session_state.stage=human_confirm`，发送 `{"type":"confirm_plan_amendment","amendment_id":A}`（`workspace_ws_handler/tests/plan_repair_activation.rs:77-97`），入口 `decisions/inbound.rs:839-847`→`decisions.rs:4-24`。发布成功后 handler 激活 attempt 并给 child 发新的 state（`decisions.rs:25-40`）。原 plan session 的门在 coding application 完成后才关闭回 `Confirmed`（stage3 `:977-982`）。
7. 同 command 重连重发：服务端先返回 `HumanGateCommandOutcome::Replayed`，workspace handler 仍发 `human_gate_turn_open`（携同一 `turn_id,command_id`，预算从 durable snapshot读取），不再 provider start；stage3 `:843-882` 断言 `recover_human_gate_turns` 无动作、同 turn replay、预算不变。

### ④ confirm 后 coding 侧何时算 resume 成功
- child confirm→出版后，coding application 依次 journal：`Started → PlanBindingWritten → UnitRunsWritten → ResumeTargetWritten → Completed`（`amendment.rs:333-416`）；然后必须先 reconcile delivery、再 `resume_attempt_after_amendment`（`:419-430`）。
- 成功判据（权威 stage3 `:928-1010`）：`resumed.id == 原 attempt.id`；issue 下 attempt 数仍 1（不新建）；binding 指向 `manifest.new_plan_revision_id`，`applied_amendment_ids=[manifest.id]`；context **同 id** 且 `Applied`，保存 previous/new revision 与 resume_target；application journal=`Completed`；原 plan session=`Confirmed`。
- unit/attempt/stage 依据 `manifest.resume_target`：`Reexecute→unit Running, attempt Running, stage Coding`；`Revalidate→unit NeedsRevalidation, stage CodeReview`；`AwaitHandoff→unit AwaitingAmendment`（stage3 `:983-1010`）。对 coding driver 来说，不能只凭 `plan_amendment_updated` 判 resume；应等后续 `coding_session_state`/REST snapshot 满足上述同 attempt + status/stage/unit/binding 条件。

### ⑤ 断线恢复与 driver 要点
- delivery marker 是 attempt 目录下 durable JSON（具体路径由 `amendment_event_delivery_path` 生成）：首次 `load_or_prepare` 写 `{id,event_id,attempt_id,amendment_id,status:pending,delivered_at:null,created_at,updated_at}`（`amendment_delivery.rs:31-63`）；全字段、派生 id、attempt/amendment 身份严格校验（`:177-201`）。
- apply 每次从 marker 读取；已 `Delivered` 不再发事件，否则注册 event_id waiter、入队 `plan_amendment_updated`，等待 socket writer ack 后才 mark Delivered（`amendment.rs:644-676`）。写失败、receiver drop、channel close/cancellation 均保持 Pending 并释放 waiter（`delivery_ack.rs:25-113`; `outbound.rs:26-60,74-85`）。
- 断线/重启恢复应重放：相同 `event_id`/相同 manifest，不能生成新 id。`Delivered` 可跳过投递；`Pending` 必须重新发送并等待新连接真实写成功。确认 mark 是幂等的（已 Delivered 直接返回；`amendment_delivery.rs:101-115`）。并发 recovery 由 exclusive lock + identity 校验收敛到一个 marker。
- application journal 与 delivery 是两层：journal 可能 Completed 但 delivery Pending；只有 delivery Delivered 且 attempt Running 才视为“当前完整完成”之一（`amendment.rs:273-300`）。

### ⑥ 明确偏差/危险假设（红色）
- **🔴 “session_link 是唯一发现路径”不成立。** Coding `coding_session_state` 同时内联 `linked_plan_repair`，且每次 state build 先调用 `reconcile_linked_plan_repair_pause`（`coding_ws_handler/state.rs:20-26,83-89`）；REST/WS 重连都可由 durable snapshot 找到 repair。driver 不应只依赖一次 `plan_repair_required` 事件。
- **🔴 “门挂 group/coding attempt session”不成立。** amendment typed feedback 门挂在原 WorkItemPlan 单候选 session；group attempt 不开人工门、attempt 零变化、不新增 session（stage3 `:786-841`）。
- **🔴 “confirm 原 plan session 即出版 amendment”不成立。** 出版确认明确是 child plan-repair session 的 `confirm_plan_amendment`；原 plan session 仅在 coding application 完成后关回 Confirmed（`plan_repair_activation.rs:77-97`; stage3 `:977-982`）。
- **🔴 “看到 `plan_amendment_updated` 就算 resume 成功”不成立。** 该事件发送后 producer 还阻塞等待 socket-write ack；ack 前 marker Pending、异常会使 attempt `AmendmentApplyFailed`（`amendment.rs:644-676`; activation `:96-143`）。
- **🔴 `original_route` 不是 WS URL，也不保证等于 workspace child URL。** PlanRepair route 是 coding 前端 HTTP 路由；真实 WS endpoint 需按项目/issue/attempt 或 child session 另行拼接（`plan_repair.rs:230-238`; `app.rs:397-412`）。

## Architecture
Coding attempt 检出 plan 缺陷后 durable pause/context + child repair session，向 coding WS 发 control event。用户通过原 plan session workspace WS 的 typed feedback 产生并完成 amendment turn；候选在 child plan-repair session AwaitingConfirmation，由 child confirm 触发 publication。Coding application journal 重放 binding/unit/resume 三阶段，随后以同一 event_id 经 coding WS 投递 manifest；writer ack 才落 delivery Delivered，最后回原 attempt 并关闭原 plan 门。

## Start Here
先读 `src/product/coding_workspace_engine/tests/campaign_stage3_amendment.rs:760-1010`，再按 `src/product/coding_workspace_engine/amendment.rs:304-440,644-676` 实现 driver 的状态判定与重放策略；不要把 child confirm、原 plan feedback、coding delivery 三条 WS 线混为一条。
