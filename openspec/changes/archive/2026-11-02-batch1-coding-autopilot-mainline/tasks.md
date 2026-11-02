# Tasks

本文件按总览 §3 拆分 17 个主流程包（owner 已确认；P2-f 暂缓、各边界包第二档不做，见 proposal Non-Goals）。每包含：目标、独占文件（只此包写）、禁碰、TDD 验收（先红后绿）、规模（S≤1h/M≤2h/L≤4h）、波次（同波无共享文件才并行；C 区共享文件按总览 §4 串行）。全部包默认 glm53-task，anthropic-task 提名 0。worker 不跑构建/测试/格式化；测试产物只在 `/tmp`；统一验收由 controller 在全部包合入后执行（命令与红测展开见 `cadence/plans/2026-10-15_实施计划_批量1主流程_v1.0.md`）。

## 波 1

### P2-a 验证处理按钮 bug（S，波 1；REQ-CVT-06）

- **目标**：coder 输出门 `evidence_refs` 为空时前端回退 `gate_id#0`；`coding_operational_blocker` 加入前后端验证处理资格白名单。
- **独占文件**：`web/src/pages/CodingWorkspacePage.tsx`（仅 `handleEnterVerificationTriage`）、`web/src/pages/CodingWorkspacePage.gates.test.tsx`（加用例）、`src/product/coding_workspace_engine/provider_failure.rs`（仅白名单）、`src/product/coding_workspace_engine/gates_parts/verification_triage.inc.rs`（仅 coder 门判断）、`src/product/coding_workspace_engine/tests/verification_triage_gates.rs`（加用例）。
- **禁碰**：`CodingWorkspaceControls.tsx`（前端白名单归 P2-d）、其他 gate 创建点。
- **TDD 验收**：①夹具 `coding_output_human_triage`+`evidence_refs:[]`，点击后 `enterVerificationTriage` 被调用且 `finding_id==="<gate_id>#0"`（红→绿）；②CR 门用例（`CodingWorkspacePage.gates.test.tsx:527-631` 同构）保持绿；③后端 `coding_operational_blocker` 门可进验证处理（先用字符串字面量，P2-b 合入后改常量）。

### P2-b 后端新门＋校验放宽＋新动作（L，波 1；REQ-COD-08、REQ-CRO-09/10）

- **目标**：`CODING_OPERATIONAL_BLOCKER_REASON_CODE`、`open_coding_operational_blocker_gate`、coder 路径 `OpenOperationalGate` 落门、coder-only 校验放宽闭包、`RetryCodingWithContext` 动作与分派臂、`should_resume_runner_after_gate_response` 白名单、`extra_context` 长度校验。
- **独占文件**：`src/product/coding_workspace_engine/gates.rs`、`gates_parts/blocked_gate.inc.rs`、`coding.rs`（仅落门段与调用点）、`rework.rs`（仅落门段）、`plan_defect_routing.rs`（仅 `execution_plan_defect_flow_decision`）、`src/product/coding_models/gate.rs`（仅 `CodingGateActionType`）、`src/web/coding_ws_handler/runner.rs`（仅白名单）、`src/web/coding_ws_handler/tests_parts/manual_recovery.inc.rs`（矩阵）、新 `src/product/coding_workspace_engine/tests/coding_operational_blocker.rs`。
- **禁碰**：`plan_defect.rs`（`validate_plan_defect_finding` 本体不改）、`plan_repair/**`、`work_item_*/**`、`provider_failure.rs`/`group.rs` 既有门。
- **TDD 验收**：①WI-002 同构 finding（operational_blocker+operational_gate+未知 reason_code+空 blocker_routing）→ 决策 `OpenOperationalGate`（现为 `StopForHumanTriage`）；②该决策落 `coding_operational_blocker` 门，actions 恰为 `[retry_coding, retry_coding_with_context, abort]`，attempt `Blocked`；③`retry_coding_with_context` 带 note → 下一轮 coder prompt 含「本轮补充上下文」；空 context → `coding_gate_extra_context_required`、gate 仍 Open；>2000 → `coding_gate_extra_context_too_long`；④回归：`StopForHumanTriage` 路径、`coder_resume_recovery.rs:554`、`verification_triage_gates.rs:325` 的 `["retry_coding","abort"]` 断言保持绿（X2：旧门两动作）；⑤`should_resume_runner_after_gate_response("retry_coding_with_context", Blocked)==true`；⑥`rework.rs` 分支顺序：`OpenOperationalGate` 先于 `else if decision != RunCoderFix` 被匹配（A-9 红测）。

### P3-b Kimi 读面（M，波 1；REQ-KACS-IDLE-RO）

- **目标**：`TerminalIsolation::Bubblewrap` 新增 `readable_root: Option<PathBuf>`；bwrap argv 顺序 host ro→proc/dev/tmpfs→authority ro→target bind→git binds→cwd；LC 取可信 boundary authority，direct 传 None。
- **独占文件**：`src/cross_cutting/kimi_code_provider/client_services/{terminal_handlers.rs,terminal.rs,sandbox.rs,mod.rs}`（mod 仅测试构造适配）。
- **禁碰**：引擎/gateway/projection/CLI。
- **TDD 验收**：①argv 顺序纯单测（host ro→proc/dev/tmpfs→authority ro→target，无需真跑）；②`/tmp` 聚合 fixture 真跑（bwrap 缺失即跳过，沿用 `probe_bwrap` 先例）：terminal/native Read 读真实上游、target 写成功、上游写失败；③direct 冒烟（readable_root=None、原 rw/ro 判定不变）。

### P3-c 完成登记（S，波 1；REQ-GCE-07）

- **目标**：`group_handoff_contract_facts` 在 refs 为空时从冻结 output_contracts 登记 contracts/capabilities，接既有 hash/build/publish。
- **独占文件**：`src/product/coding_workspace_engine/group_completion.rs`（测试加本文件内）。
- **禁碰**：P3-a/b 文件、块 2、plan 生成。
- **TDD 验收**：①refs 空 + output 声明 CT-001 → 新 handoff refs/能力/hash 登记齐；②非空 refs 保持原选择；③无 output 允许为空。

## 波 2

### P2-c triage_summary 投影（M，波 2，P2-b 后同文件串行；REQ-CRO-09）

- **目标**：`CodingTriageSummary` DTO、`CodingGateRequired.triage_summary`、`build_coder_triage_summary` 宽容解析。**C1 修正（已裁决采纳）**：不给 `CreateBlockedGateInput` 加字段（其全仓 34 处字面量无 `..` 展开，加字段波及约 20 个清单外文件、含其他包独占文件）；`triage_summary` 改经 `coding_attempt_store` 独立入口（新建门后写入 summary 的 store 函数）落盘，改动收回本包独占文件；`inputs.rs` 不再修改。
- **独占文件**：`src/product/coding_models/gate.rs`（`CodingGateRequired`＋新 DTO）、`src/product/coding_attempt_store/gate.rs`（summary 独立入口）、`src/product/coding_workspace_engine/gates.rs`（仅 `build_coder_triage_summary`）、字面量补丁点 4 处：`coding_models/tests.rs:70`、`tests/it_product/product_coding_models.rs:346`、`tests/it_web/web_coding_ws_handler/part_01.rs:320`、`part_05.rs:387` 附近（后两处以实测字面量为准，函数返回类型处无需改）。
- **禁碰**：引擎调度逻辑、其他包独占文件（C1 收敛后不触 `provider_failure.rs`/`rework.rs`/`group_review_orchestrator.rs`）。
- **TDD 验收**：①WI-002 同构 raw 输出 → `problem` 为 finding.message、`did.changed_files` 4 项、`unverified` 取自 evidence；②`delivery_report` 缺失/字段类型错 → `did=None` 且门照常创建（摘要构造绝不阻塞开门）；③旧 gate JSON 反序列化 `triage_summary=None`；④全仓 `cargo check --locked` 无字面量遗漏（controller 统一验收）。

### P3-a 上游定位/注入（L，波 2，P2-b 后同文件区段串行；REQ-COD-07）

- **目标**：`ResolvedUpstreamDelivery` 与 `resolved_upstream_deliveries_for_attempt`（cross_attempt_handoff.rs）；`prompts.rs` 新增 `resolved_upstream_deliveries_section` 并在 fresh/delta/rework/teaching 实际 prompt 追加；ID API 从同一结果取值。
- **独占文件**：`src/product/coding_workspace_engine/{cross_attempt_handoff.rs,prompts.rs,coding.rs,rework.rs,tests/cross_attempt_handoff.rs}`（coding.rs/rework.rs 与 P2-b 不同区段，P2-b 先）。
- **禁碰**：Kimi（P3-b）、`group_completion.rs`（P3-c）、projection/plan、块 2 门分支。
- **TDD 验收**：①上游 unit 已完成而 attempt 待确认 → 下游 prompt 有 owner path/SHA/CT-001；②fresh/delta/rework/teaching/retry 均带段；③单仓无段、renderer/hash 不变。

## 波 3

### P2-d 前端分诊卡三按钮（M，波 3，P2-b/c 形状定后合入；REQ-CRO-09）

- **目标**：新 `CoderTriageCard`、`buildTriageNarrative` 纯函数、验证处理折叠、前端白名单、`gateActionRequiresContext`、Cockpit 动作标签。**C5**：`isTriageCardGate` 谓词按 reason_code 判定，`coder_provider_stalled` 的纳入由 P4-e 扩（C 区 #1：P2-d→P4-e）。
- **独占文件**：`web/src/pages/CoderTriageCard.tsx`（新）、`web/src/state/coding-triage-narrative.ts`（新＋test）、`web/src/pages/CodingWorkspaceControls.tsx`（GatePanel 分流＋前端白名单）、`web/src/api/types/coding.ts`（类型）、`web/src/hooks/useCodingWorkspaceWs.ts`（仅 `gateActionRequiresContext`）、`web/src/components/chat-workspace/cockpit/CockpitInboxWaitingCards.tsx`（标签表）、`web/src/pages/CodingWorkspaceControls.verification-triage.test.tsx`（调整折叠断言）。
- **禁碰**：`CodingWorkspacePage.tsx`（P2-a/P2-e 占用）、非 triage 门渲染。
- **TDD 验收**：①`coding_operational_blocker` 门渲染三按钮（固定文案与顺序），点第二按钮出输入框，空输入提交被拦；②老 gate（只有 retry_coding/abort）渲染两按钮（X2）；③`coding_output_human_triage` 同卡两按钮；④CR 三门/`code_review_blocked` 渲染与现有快照一致；⑤「转入验证处理」默认折叠不可见，展开后可见且回调仍传 gate 身份；⑥`buildTriageNarrative` 纯函数用例（有/无 summary、N>1 finding、无 delivery）。

### P4-a idle 重试主流程（L，波 3，块 2 A4/summary 后；REQ-PTR-IDLE）

- **目标**：新 `provider_idle.rs`（阈值＋运行期 last activity，`mod.rs` 注册）、`provider_stream.rs` idle timer、`ProviderInvocationOutcome`/`CodingWorkspaceEngineError` idle 变体、`resolve_provider_retry_cycle_outcome` idle 前置、`provider_failure.rs::open_provider_stalled_gate`、README 三项 env。**C3 修正**：不用假时钟（tokio 未启用 `test-util`）；配置经 `ARIA_CODING_*_IDLE_SECS` 在调用启动时一次读取，红测用 env 覆写短值（先例 `coding_workspace_engine/tests/group_review_budget.rs:159`）或 `#[cfg(test)]` 短常量（先例 `workspace_engine/provider_drive/watchdog.rs:22-58`）。**C4 修正**：复用 `src/product/workspace_engine/provider_drive/watchdog.rs` 与 `tests/provider_idle_watchdog.rs` 范式（真实活动重置＋choice/permission 暂停＋耗尽收口），不重造。
- **独占文件**：`src/product/coding_workspace_engine/{provider_idle.rs(新),provider_stream.rs,provider_retry.rs,types.rs,provider_failure.rs,mod.rs,tests/provider_failure_recovery.rs}`、README（env 三项）。
- **禁碰**：group/前端、块 2 动作文件。
- **TDD 验收**：①env 覆写短阈值（≈1s）触发挂起（替代「假时钟 300/600」措辞）、真实事件重置计时、choice/permission 等待暂停、`pi_empty_output_retry` 算新事件；②首次挂起后重试成功继续原流程；③两次挂起只调 2 次并转 `Blocked`＋stalled 门，不切 provider、不改 transport 3 次；④工具 call 未返回用 `max(角色,1800)`。

### P1-a 聚合确认后端（M，波 3*，与 P4-c 分段串行；REQ-MTG-11）

- **目标**：新 `plan_final_confirm.rs::{build_plan_final_confirm,post_plan_coding_final_confirm}`（共享纯函数 pub(crate) 导出供 A1 调用）、`build_plan_coding_progress` 接 `final_confirm/attention_items`、POST 路由。3* 分段：P4-c DTO → P1-a `build_plan_final_confirm` → P4-c `issue_coding_attention_items` → P1-a GET；`src/web/types.rs` 串行。
- **独占文件**：新 `src/web/handlers/coding/plan_final_confirm.rs`、新 `plan_final_confirm_tests.rs`、`src/web/handlers/coding.rs`、`src/web/handlers/mod.rs`、`src/web/handlers/coding/plan_start.rs`、`plan_start_tests.rs`、`src/web/app.rs`、`src/web/types.rs`（引用段）。
- **禁碰**：`handoffs.rs`（`handle_final_confirm` 不改）、plan-run 持久模型、provider/分诊、前端。
- **TDD 验收**：①多仓全部 FinalConfirm＋readiness 完整 → GET ready；②一次 POST 全确认（逐仓 confirmed）；③第 2 项失败 → 第 3 项未调用（not_attempted）。

### P4-c 事实投影后端（M，波 3*，与 P1-a 分段串行；REQ-CRO-12）

- **目标**：新 `src/web/coding_attention_info.rs::issue_coding_attention_items`（直接读 attempt facts，调块 1 纯 `build_plan_final_confirm`，不反调 GET/lifecycle）；`coding_attempt_store/gate.rs` 只读 `list_open_blocked_gate_facts`（gate+created_at）；`IssueLifecycleResponse.coding_attention_items`（DTO 先于 P1-a 引用合入）；`web/mod.rs` 注册。
- **独占文件**：`src/product/coding_attempt_store/gate.rs`、新 `src/web/coding_attention_info.rs`、`src/web/mod.rs`、`src/web/types.rs`（DTO 段）、`src/web/handlers/lifecycle.rs`。
- **禁碰**：引擎、前端、P1-a 的 handler 文件（3* 分段外）。
- **TDD 验收**：①choice/triage/blocked/stalled/manual_recovery/final_confirm 各类事项有稳定 key；②事实关闭后投影消失（无 ack）；③plan ready/partial 聚一条 `plan_final_confirm` 并抑制逐仓；④occurred_at 取事实时间非 now。

## 波 4

### P2-e choice 可处理（M，波 4，P2-d 后同 hook 区段；REQ-CRO-11）

- **目标**：WS 帧/快照带 `questions`；Coding 页提交改 REST（恒带逐题 answers/command_id/expected_run_id）；新 `useChoiceRespondCommand.ts`；无 runner 人话提示（X1：无恢复按钮）。
- **独占文件**：`src/web/coding_ws_handler/protocol.rs`（仅 `CodingChoiceRequest` 变体与帧映射）、`tests/it_product/product_coding_workspace_engine/part_04.rs`（**C2 修正**：538 附近穷尽 struct pattern 补 `..`，仅此一处）、`web/src/hooks/useCodingWorkspaceWs.ts`（仅 choice 处理器段）、`web/src/state/coding-workspace-store.ts`（`pendingChoiceEntry`）、`web/src/pages/CodingWorkspacePage.tsx`（仅 `handleChoiceResponse` 段）、`web/src/components/chat-workspace/entries/ChoiceRequestEntry.tsx`、新 `web/src/hooks/useChoiceRespondCommand.ts`、对应测试。
- **禁碰**：引擎 `provider_stream.rs`、`socket.rs` ChoiceResponse 分支（保留）。
- **TDD 验收**：①帧输出带 `questions`；②entry metadata 含 `questions`；③多题提交 → REST body 含逐题 `answers[question_id]`、`command_id`、`expected_run_id`；④无 `expected_run_id` → 不发请求且显示「AI 当前没有在运行，这个问题暂时无法回答」（无恢复按钮）；⑤`expired` → 失效提示；⑥单题回落 `default`；⑦既有 `CodingWorkspacePage.pending-choice.test.tsx`、`ChoiceRequestEntry.test.tsx` 保持绿。

### P4-b internal 接线（M，波 4，P4-a 后；REQ-PTR-IDLE）

- **目标**：`group_review_orchestrator.rs::{map_group_review_engine_error,execute_with_retry_result,create_failure_gate}` 与 `group_review_errors.rs` 保留 idle 类别，internal reviewer 同样一次恢复。
- **独占文件**：`src/product/coding_workspace_engine/{group_review_orchestrator.rs,group_review_errors.rs}` 含单测。
- **禁碰**：P4-a 文件、adapter。
- **TDD 验收**：①internal reviewer idle 一次恢复后继续；②耗尽落 stalled 门而非 3 次 Transport。

### P4-d 全局消费前端（M，波 4，P4-c 后；REQ-UI37-19）

- **目标**：`api/types/lifecycle.ts` DTO、`workspace-cockpit-projection.ts` mapper、`useWorkspaceSessionObservers.ts` 全量纳入 display/count（无 watched/TTL/上限过滤）。
- **独占文件**：`web/src/api/types/lifecycle.ts`、`web/src/state/workspace-cockpit-projection.ts`、`web/src/hooks/useWorkspaceSessionObservers.ts` 及 test。
- **禁碰**：Shell/router、引擎、后端。
- **TDD 验收**：①当前 plan B、A 不在 watched 内仍进 display/count；②同 key 不重复提醒；③处理消失后条目移除。

## 波 5

### P1-b 总览页与聚合按钮（M，波 5；REQ-MTG-11/12）

- **目标**：新 `CodingPlanOverviewPage.tsx`（列表+单详情切换，owner 19:45）与 `PlanCodingFinalConfirm.tsx`；`usePlanCodingProgress` 统一 `{data,loading,error,refetch}`（可选 5s 轮询）并迁移两处调用；`toPlanCodingProgressView` 补齐；`confirmPlanCoding` client；GroupProgress 两入口。
- **独占文件**：新 `web/src/pages/{CodingPlanOverviewPage,PlanCodingFinalConfirm}.tsx` 及各自新 `.test.tsx`、`web/src/api/types/coding.ts`、`web/src/api/client.ts`、`web/src/api/coding-attempts.test.ts`、`web/src/hooks/usePlanCodingProgress.ts`（＋新同名测试）、`web/src/pages/plan-coding-progress.ts`、`web/src/pages/CodingWorkspaceGroupProgress.tsx`（＋`.test.tsx`）、`web/src/components/coding-workspace/dashboard/CodingDashboard.tsx`（仅迁 hook 返回）。
- **禁碰**：后端、`CodingWorkspaceControls.tsx`/WS 动作、全局通知（P4-d/e）。
- **TDD 验收**：①列全 WI/仓库/进度/依赖/待办；②一次点击仅一个 POST（聚合确认）；③失败仓可见；④5s 轮询见非当前仓更新。

## 波 6

### P1-c 导航与焦点（S，波 6；REQ-MTG-12）

- **目标**：router 增 plan 总览路由、`CodingWorkspaceRouteComponent` 解析 search（`planId/workItemId/focus/gateId/choiceId`）；CodingWorkspacePage 接定位/返回总览；统一 A1 target 导航（块 4 只供回调不写此两文件）。
- **独占文件**：`web/src/router.tsx`、`web/src/pages/CodingWorkspacePage.tsx`、`CodingWorkspacePage.test.tsx`、`CodingWorkspacePage.pending-choice.test.tsx`。
- **禁碰**：WS 协议、块 2 动作 handler、块 4 通知投影。
- **TDD 验收**：①正确 attempt 地址（复用现有 WS 重置/重连）；②真实详情；③徽标 `focus.gateId/choiceId` 定位卡片；④返回总览。

### P1-d attempt 状态覆盖（S，波 6，P1-b 后同 Dashboard 串行；REQ-MTG-13）

- **目标**：`coding-dashboard-projection.ts` 新增 `selectCodingDashboardAttempts`；CodingDashboard 独立状态条（复用既有标签表，不改 unit 拓扑判据）。
- **独占文件**：`web/src/state/coding-dashboard-projection.ts`（＋同名 `.test.ts`）、`web/src/components/coding-workspace/dashboard/CodingDashboard.tsx`（＋同名 `.test.tsx`）。
- **禁碰**：unit 拓扑判据、API、后端。
- **TDD 验收**：①11 态参数化；②WaitingForHuman＋FinalConfirm 显示「等待最终确认」不叫分诊；③单仓显示当前 attempt。

## 波 7

### P4-e 提醒、去处理与挂起卡（L，波 7；REQ-UI37-19、REQ-CRO-09/10）

- **目标**：`CockpitShell.tsx` 按 key 提醒＋系统通知 onclick；`CockpitInbox.tsx`「去处理」＋`onOpenAttentionTarget`；`CodingWorkspaceControls.tsx` 挂起门接入三按钮卡（**C5**：`isTriageCardGate` 纳入 `coder_provider_stalled`，复用块 2 CoderTriageCard/summary/A4，说明提交即重新开跑）。
- **独占文件**：`web/src/components/cockpit/CockpitShell.tsx`（＋test）、`web/src/components/chat-workspace/cockpit/CockpitInbox.tsx`（＋test）、`web/src/pages/CodingWorkspaceControls.tsx`（＋`verification-triage.test.tsx`；C 区 #1：P2-d→P4-e→批 2）。
- **禁碰**：router/Page（P1-c owner）、API/引擎。
- **TDD 验收**：①plan B 收到 plan A 新 key 提醒，onclick 导航正确；②通知权限拒绝仍有平台消息；③stalled 门三按钮、说明情况提交即跑。

## 收口（controller 执行，不派工）

- [ ] 统一验收：四条 cargo 标准命令＋web 三绿（禁 `-j`）；定向红/绿证据留存。
- [ ] 单仓冒烟（P2-g）：单仓 manual attempt coding→review→原确认；构造分诊门新卡可处理；单题 choice 作答到达 provider。
- [ ] 主流程 7 点验收（总览 §6 ①~⑦）与真实浏览器/provider 验收。
- [ ] `openspec validate batch1-coding-autopilot-mainline --strict` 通过。
