# plan-compile-gate-visibility 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Work Item Plan 的批次确认门与 Final Compile recovery 门在 Cockpit 可见可操作（含 compile 失败转批次确认路径），并把 F-34 provider 创建竞态消除（创建请求快照 provider + 共享默认 hook + 生成前可见）。

**Architecture:** 三个工作包。WP-B（后端）：SC HumanConfirm 协议臂放行既有 recovery action（引擎守卫已 fail-closed，只动矩阵）+ HTTP confirm 端点补 WorkItemPlan AuthorConfirm 引擎分支（修掉 store-only 假 Confirmed 半落状态）。WP-A（前端）：`selectGateProjection` 增两类门识别分支（key=`node:${node_id}`）+ inbox 动作面 + facade。WP-C（前端）：plan 选项弹窗 provider 选择器 + 三创建请求带 provider + 默认应用 hook 化覆盖 legacy + 开始生成前 provider 可见。WP-B 与 WP-A 文件零交集可并行；WP-C 与 WP-A 共享 `ChatCockpitPage.tsx`（区间不重叠），WP-A 先落 WP-C 后接。

**Tech Stack:** Rust（axum WS handler + workspace_engine）、React + TypeScript + zustand + vitest、OpenSpec。

**Spec:** `openspec/changes/plan-compile-gate-visibility/`（proposal/design/tasks + 5 delta）。实施中契约疑问一律以 spec 为准，不得在实施计划外重定义。

## Global Constraints（每个任务隐含遵守）

- 不改 REQ-RET-02 三命令边界：HumanConfirm SC 仍只 `HumanGateFeedback|Confirm|AbandonHumanGate`（recovery action 是**独立**放行项，不是第四命令）；AuthorConfirm SC 仍 `Abort|AbandonHumanGate`。
- 不放宽 F-21 相位纪律与 F-30 终态守卫；迟到动作零副作用。
- 不改 compile/batch/recovery 业务状态机、action 枚举含义、错误码语义。
- 不连接/污染 4317 生产服务器。
- `cargo` 禁 `-j 1`（并行度由 `.cargo/config.toml` jobs=8 托管）；定向单测用 `cargo test --locked --lib <filter>`。
- 前端资源经 rust-embed 编译期嵌入：**改前端后部署必须先 `cd web && pnpm build` 再 `cargo build --release --locked`**（v33 事故铁律）。
- 前端测试：`cd web && npm test`（vitest --run）；后端：`cargo test --test it_web <filter>` / `--test it_core <filter>` / `--lib <filter>`。
- 每任务一次独立 commit；commit message 用约定式前缀（feat/fix/test/refactor）。
- worker 报告落 `.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/<任务名>-report.md`，只写盘不 `git add`。

## 勘察依据（行号为 2026-09-22 实测，见 scout 报告）

- 前端：`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/scout-fe-pcg.md`
- 后端：`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/scout-be-pcg.md`

---

## Task 1（WP-B）: SC HumanConfirm 协议臂放行 recovery action

**Files:**
- Modify: `src/web/workspace_ws_handler/protocol.rs:172-183`（SC HumanConfirm 臂）
- Test: `src/web/workspace_ws_handler/tests/conversational_gate_stage.rs`

**Interfaces:**
- Consumes: 既有 `WsInMessage::WorkItemPlanCompileRecoveryAction{action, reason}`（`src/web/workspace_ws_types/in_.rs:64-67`）、矩阵函数 `is_message_valid_for_stage_with_flow(flow_kind, &msg, &stage)`
- Produces: 矩阵行为变更——`(SingleCandidate, WorkItemPlanCompileRecoveryAction{..}, HumanConfirm)` → `true`；其余 stage 全部保持 `false`。引擎侧 durable 事实守卫**已存在**（`src/product/workspace_engine/compile.rs:866-876` workspace_type+active recovery node；`:874-876` RecoveryRequired 事务），本任务不改引擎。

**背景（防误改）:** 非 SC HumanConfirm 臂已在 `protocol.rs:188` 放行本消息；本任务只补 SC 臂。durable 事实校验不在纯矩阵层做（矩阵无 session 状态访问），由引擎守卫 fail-closed 承担——这与非 SC 臂现状同构。`human_gate_message_boundary_error`（`protocol.rs:207-224`）以矩阵判定为准，矩阵放行后自动不再拦截；需在测试中钉住这一点。

- [ ] **Step 1: 写失败测试**（`conversational_gate_stage.rs`，沿用文件内既有断言风格）：

```rust
// SC HumanConfirm 臂放行 compile recovery action（change plan-compile-gate-visibility 缺口 B）
#[test]
fn sc_human_confirm_accepts_compile_recovery_action() {
    for action in [CompileRecoveryAction::Continue, CompileRecoveryAction::AbortAndRollback, CompileRecoveryAction::HumanTriage] {
        let msg = WsInMessage::WorkItemPlanCompileRecoveryAction { action, reason: None };
        assert!(is_message_valid_for_stage_with_flow(FlowKind::SingleCandidate, &msg, &WorkspaceStage::HumanConfirm));
    }
}

#[test]
fn sc_recovery_action_rejected_outside_human_confirm() {
    let msg = WsInMessage::WorkItemPlanCompileRecoveryAction { action: CompileRecoveryAction::Continue, reason: None };
    for stage in [WorkspaceStage::Running, WorkspaceStage::AuthorConfirm, WorkspaceStage::ReviewDecision, WorkspaceStage::Completed, WorkspaceStage::PrepareContext] {
        assert!(!is_message_valid_for_stage_with_flow(FlowKind::SingleCandidate, &msg, &stage));
    }
}
```
（枚举/函数导入名以文件内既有用例为准，形态不变。）

- [ ] **Step 2: 跑测试确认失败**：`cargo test --locked --lib sc_human_confirm_accepts_compile_recovery`（预期 FAIL/断言红）
- [ ] **Step 3: 实现**——`protocol.rs:172-183` SC 臂的消息匹配集加入 `WsInMessage::WorkItemPlanCompileRecoveryAction{..}`（放在三命令之后，注释标明「独立 compile recovery 操作，durable 事实由引擎守卫校验，见 compile.rs handle_work_item_plan_compile_recovery_action」）。不改动三命令匹配、不触 `human_gate_message_boundary_error` 逻辑。
- [ ] **Step 4: 跑测试确认通过**：`cargo test --locked --lib conversational_gate`（全部绿）；`cargo clippy --all-targets --all-features --locked -- -D warnings` 0
- [ ] **Step 5: Commit**：`git add src/web/workspace_ws_handler/protocol.rs src/web/workspace_ws_handler/tests/conversational_gate_stage.rs && git commit -m "feat(ws): SC HumanConfirm 放行 compile recovery action（缺口 B 协议面）"`

## Task 2（WP-B）: recovery action 的 it_web 集成矩阵

**Files:**
- Test: `tests/it_core/workspace_ws_integration/part_02.rs`（WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID / recovery 集成用例现址）

**Interfaces:**
- Consumes: Task 1 矩阵放行；既有分发 `src/web/workspace_ws_handler/decisions/inbound.rs:354-372`（失败回 `ProtocolError{code:"INVALID_COMPILE_RECOVERY_ACTION"}`）
- Produces: 集成证据（REQ-PCG-02 / REQ-CG-02 场景「非 recovery 状态拒绝」）。

- [ ] **Step 1: 写失败测试**（沿用 part_02.rs 既有 WS 集成 harness 写法）覆盖四个场景：
  1. SC 会话构造至 compile recovery 态（active `WorkItemPlanCompileRecovery` node + RecoveryRequired 事务，fixture 参照 `src/product/workspace_engine/tests/single_candidate_recovery.rs:176` 的构造路径）→ 发 `work_item_plan_compile_recovery_action{action:"continue"}` → 引擎按既有语义继续（不创建第二个 compile），无 ProtocolError；
  2. 同一 SC 会话在 **generate/Running** 阶段发 recovery action → 收 `WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID`（矩阵拒绝，零副作用）；
  3. SC HumanConfirm 但**无 recovery 事实**（普通人工门）→ 收 `INVALID_COMPILE_RECOVERY_ACTION`（引擎守卫拒绝，零副作用：turn/预算不变）；
  4. 终态（Confirmed/Terminated）发 recovery action → stage 错误，零副作用。
- [ ] **Step 2: 跑确认**：`cargo test --test it_core --test it_web compile_recovery`（场景 1 在 Task 1 落地后应绿；若场景 1 需要额外接线则红——红了先查分发链再改，不允许改引擎语义）
- [ ] **Step 3: 全量回归**：`cargo test --test it_core workspace_ws` + `cargo test --test it_web`
- [ ] **Step 4: Commit**：`git add tests/it_core/workspace_ws_integration/part_02.rs && git commit -m "test(it_core): SC compile recovery action 集成矩阵（合法/非法阶段/缺事实/终态）"`

## Task 3（WP-B）: HTTP confirm 端点补 WorkItemPlan 批次确认引擎分支

**Files:**
- Modify: `src/product/workspace_engine/lifecycle.rs:1027-1067`（`http_confirm_disposition`，现只收 Story|Design，WorkItemPlan 一律 NotHandled）
- Modify: `src/web/handlers/workspace_session.rs:147-206`（端点仲裁调用处；`:178-190` NotHandled 兜底=store-only 假 Confirmed，本任务消灭该半落状态对 WorkItemPlan 的可达性）
- Test: `tests/it_web/web_lifecycle_api/`（新 part 或就近 part）+ 引擎单测就近放置

**Interfaces:**
- Consumes: 既有引擎 API `confirm_work_item_plan`（`src/product/workspace_engine/controls.rs:121-208`：校验 compiled WorkItems 非空 → confirm_issue_work_item_plan → ensure_work_item_sessions_for_plan 建 子会话 → 落 Confirmed，SC 走 CAS 保留门快照 `:178-193`）；既有 stage 推进/Completed 节点逻辑参照 `controls.rs:49-68`（legacy handle_confirm 臂）与 `compile.rs:526-527`（SC 自动链）。
- Produces: `http_confirm_disposition` 新增 arm——`workspace_type==WorkItemPlan && stage==AuthorConfirm && active_node_type==Some(WorkItemBatchConfirm)` 时执行与 `controls.rs:49-68` plan 路径等价的确认（confirm_work_item_plan + stage→Completed + 节点收口）；`with_review=true` 对 WorkItemPlan 返回 422（镜像 F-31 ReviewUnavailable 模式）；其余 WorkItemPlan 形态维持 NotHandled（fail-closed 不变）。

**背景:** 现状 store-only 兜底置 Confirmed 后 stage 停在 AuthorConfirm，矩阵不放行 Advance（`protocol.rs:193-196` 仅 Completed+SC）→ 既不能 advance 也没建子会话=半落状态（用户事故「界面无操作可行」的根子之一）。本任务把既有 typed HTTP confirm 通路接到既有引擎执行面，不新增状态机。

- [ ] **Step 1: 写失败测试**（it_web，参照 part_05 `json!({"author_provider":"fake",…})` 与既有 confirm 端点用例）：
  1. 构造 WorkItemPlan 会话至 batch-confirm（AuthorConfirm + active `WorkItemBatchConfirm` node；fixture 参照 `src/product/workspace_engine/tests/part_03/` draft batch 路径）→ `POST /api/workspace-sessions/{id}/confirm` body `{"confirmed_by":"user"}` → 200；断言：session_status=Confirmed、stage 投影 Completed、子 work item 会话已建（`ensure_work_item_sessions_for_plan` 效果）、随后 `advance` 可达（矩阵放行）；
  2. 同会话 `{"with_review":true}` → 422（WorkItemPlan 不支持送审）；
  3. WorkItemPlan 但 stage=HumanConfirm（对话门态）→ NotHandled 原语义不变（409/既有形态，以现网 Story|Design 之外的既有响应为准断言）；
  4. 重复 confirm（幂等重放）→ 第二次按既有终态守卫拒绝，零副作用。
- [ ] **Step 2: 跑确认失败**（红）
- [ ] **Step 3: 实现**：`lifecycle.rs` `http_confirm_disposition` 增加 WorkItemPlan 分支——判据三连（type/stage/active node），执行体复用 `confirm_work_item_plan` 并补 stage→Completed/节点收口（优先把 `controls.rs:49-68` 的 plan 推进段抽成私有函数两处共用，避免复制）；`workspace_session.rs` 侧不需新路由（NotHandled 分支自然消失）。SC CAS 快照语义保留。
- [ ] **Step 4: 跑确认通过 + 回归**：定向 it_web 用例绿；`cargo test --test it_web` 全绿；`cargo clippy --all-targets --all-features --locked -- -D warnings` 0；`cargo fmt`（只 fmt 本任务文件，收口注明）
- [ ] **Step 5: Commit**：`git add src/product/workspace_engine/lifecycle.rs src/product/workspace_engine/controls.rs tests/it_web/web_lifecycle_api/ && git commit -m "feat(engine): HTTP confirm 端点接 WorkItemPlan 批次确认引擎分支（消灭 store-only 半落状态）"`

## Task 4（WP-A）: selectGateProjection 增 batch/recovery 两识别分支

**Files:**
- Modify: `web/src/state/workspace-cockpit-projection.ts`（`GateProjection` :123-146、`selectGateProjection` :171-267、`CockpitInboxItem`/派生 :269-411）
- Test: `web/src/state/workspace-cockpit-projection.test.ts`（717 行，describe gate projection :93）

**Interfaces:**
- Consumes: timeline node_type 枚举已含两 kind（`web/src/state/workspace-ws-store-types.ts:94-101`）；stage 值（`web/src/api/types/workspace.ts:273-280`）。
- Produces:
  - `GateProjection` 新字段 `kind: "human_gate" | "batch_confirm" | "compile_recovery"`（既有四路全部默认 `"human_gate"`，消费方不破坏）；
  - batch 分支存在条件：`flow_kind==="single_candidate" && stage==="author_confirm" && 存在 active node type==="work_item_batch_confirm"`，key=`node:${node_id}`；
  - recovery 分支存在条件：`存在 active node type==="work_item_plan_compile_recovery"`（其 stage 恒 HumanConfirm），key=`node:${node_id}`；
  - inbox item id 沿用 `${sessionId}:gate:${key}` 形态（`cockpitInboxItemSessionId` :293-303 自动兼容）；
  - action_block_reason：两新门无 turn/snapshot 载体，阻断判据只取 terminal_stage/closed（`gateActionBlockReason` :60-83 对新 kind 走窄分支，不误报 phase_mismatch）。

- [ ] **Step 1: 写失败测试**（纯函数风格，手工构造 `WorkspaceWsState`）：
  1. batch 门投影：构造含 active `work_item_batch_confirm` node + stage author_confirm 的 single_candidate state → `selectGateProjection` 返回 `{kind:"batch_confirm", key:"node:<id>", …}`；`selectCockpitInbox` 含一条 gate item；
  2. recovery 门投影：active `work_item_plan_compile_recovery` node → `{kind:"compile_recovery", key:"node:<id>"}`，摘要含 recovery 原因（node payload 字段，缺失时只读诊断文案）；
  3. 稳定 key 去重：同 node 重复帧/重连后重投影 → 同 key 单条 inbox item（不重复追加）；
  4. 终态收口：session_status=Confirmed/Terminated 后即便残留 active node 也不投影门（F-30 权威）；
  5. 既有四路回归：无两新 node 时行为与改前一致（kind="human_gate"）。
- [ ] **Step 2: 跑确认失败**：`cd web && npm test -- workspace-cockpit-projection`
- [ ] **Step 3: 实现**（按 Interfaces 的条件与 key 构造；title 文案：batch=「确认整组 Work Item Draft」，recovery=「Final Compile 恢复」；无凭据时 `readOnlyDiagnostic`，不猜动作——D1/REQ-PCG-03）
- [ ] **Step 4: 跑确认通过 + tsc**：`npm test -- workspace-cockpit-projection` 绿 + `pnpm exec tsc --noEmit` 0
- [ ] **Step 5: Commit**：`git add web/src/state/workspace-cockpit-projection.ts web/src/state/workspace-cockpit-projection.test.ts && git commit -m "feat(cockpit): 批次确认与 compile recovery 门投影（稳定 node key 去重）"`

## Task 5（WP-A）: 动作面——facade 两新动作 + CockpitInbox 分流 + 页面接线

**Files:**
- Modify: `web/src/state/cockpit-action-routing.ts`（facade :3-105、门身份 :158-190）
- Modify: `web/src/components/chat-workspace/cockpit/CockpitInbox.tsx`（`CockpitInboxRow` :154-312、`GateInboxActions` :314-474）
- Modify: `web/src/pages/ChatCockpitPage.tsx:322-345`（confirm 谓词 `isStoryDesignAuthorConfirm` 放宽到 work_item_plan batch AuthorConfirm）、`:1130-1140`（CockpitInbox props 接线）
- Test: `web/src/components/chat-workspace/cockpit/CockpitInbox.test.tsx`、`web/src/pages/ChatCockpitPage.plan.test.tsx`

**Interfaces:**
- Consumes: recovery 发送函数**已存在**——`useWorkspaceWs.ts:605-613` `sendWorkItemPlanCompileRecoveryAction(action, reason?)`；batch confirm 复用既有 HTTP confirm 发送（`ChatCockpitPage.tsx:322-345`）；terminate 复用既有 `sendAbandonGate`/Abort（SC AuthorConfirm 矩阵已放行 Abort）。
- Produces:
  - `CockpitActionFacade` 增 `confirmBatch(): Promise<void>`（走 HTTP confirm，仅 kind=batch_confirm 可用）与 `recoverCompile(action: WorkItemPlanCompileRecoveryAction, reason?: string): Promise<void>`（走 WS，仅 kind=compile_recovery 可用）；两者发送前查新 kind 的 action_block_reason，不与三命令互转（REQ-RET-02/REQ-CG-02）；
  - `GateInboxActions` 分流：kind=batch_confirm → [确认整组][终止]；kind=compile_recovery → [继续][放弃并回滚][转人工]（+ human_triage 可选 reason 输入，对齐 legacy `WorkItemPlanStagedPanel.tsx:27-46` 的面板能力）；不可操作时禁用+原因（driver 非 actionable 沿用 `actionableSessionId` 门控）；
  - WS 出站不需要新消息类型（`web/src/api/types/workspace.ts:227-231` 已有 recovery 帧；batch 走 HTTP）。

- [ ] **Step 1: 写失败测试**（fixture 风格沿用）：
  1. batch 门行渲染两按钮，点确认调 facade.confirmBatch（mock 断言），点终止走既有 terminate；
  2. recovery 门行渲染三按钮，continue → `recoverCompile("continue")`；human_triage 带 reason；
  3. 终态/门关闭 fixture → 按钮禁用且显示原因；driver 非 actionable → 禁用；
  4. 协议拒绝（INVALID_COMPILE_RECOVERY_ACTION）→ 收件箱出现可诊断错误条目（复用 `classifyProtocolError` :107-156 分流）；
  5. ChatCockpitPage 谓词放宽：work_item_plan+author_confirm+batch node 的会话点确认走 HTTP confirm（带 provider 无关 body `{"confirmed_by":"user"}`）。
- [ ] **Step 2: 跑确认失败**：`cd web && npm test -- CockpitInbox` 与 `npm test -- ChatCockpitPage.plan`
- [ ] **Step 3: 实现**（按 Interfaces；不碰 `useWorkspaceWs.ts` 导出面——复用现有函数）
- [ ] **Step 4: 跑确认通过 + 全前端口径**：`npm test`（vitest 全量）+ `pnpm exec tsc --noEmit` 0
- [ ] **Step 5: Commit**：`git add web/src/state/cockpit-action-routing.ts web/src/components/chat-workspace/cockpit/CockpitInbox.tsx web/src/pages/ChatCockpitPage.tsx web/src/components/chat-workspace/cockpit/CockpitInbox.test.tsx web/src/pages/ChatCockpitPage.plan.test.tsx && git commit -m "feat(cockpit): 批次确认/compile recovery 门动作面与页面接线"`

## Task 6（WP-C）: plan 选项弹窗 provider 选择器 + 三创建请求带 provider

**Files:**
- Modify: `web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx`（FormValue :3-9、表单 :60-158、错误内联 :104-109）
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`（story/design 分支 :932-973、confirm 回调 :984-1013）
- Test: `web/src/components/lifecycle/`（对话框/Workbench 既有测试文件就近）

**Interfaces:**
- Consumes: 请求体协议**已带**可选 `author_provider/reviewer_provider`（`ProviderWorkspaceConfigInput`，`web/src/api/types/common.ts:235`）——client.ts 零改动；可用性数据 `web/src/state/provider-availability-store.ts` + `web/src/state/provider-options.ts`（不可用置灰，参照 `ProviderConfigPanel.tsx:44-52`）；默认值 `web/src/state/workspace-provider-defaults.ts`（`readWorkspaceProviderDefaults`）。
- Produces:
  - `WorkItemPlanOptionsFormValue` 增 `author_provider?: string; reviewer_provider?: string`；弹窗内两下拉，prefill=localStorage 默认，不可用项置灰禁选；
  - `handleConfirmWorkItemPlanOptions` 组装请求含两 provider 字段；
  - story/design 分支（:932-973）请求体自动补 `author_provider/reviewer_provider`=localStorage 默认值（无选择 UI——选择面仍在 cockpit provider 面板/新弹窗，创建即快照消除竞态；REQ-PPS-01 场景二）；
  - provider 不可用（显式选择走服务端 `resolve_explicit_provider_name` 已 fail-closed 回 `provider_unavailable`）→ 弹窗既有内联 alert 显示后端 message、弹窗不关（:36-40/:104-109 机制已存在，补测试钉住）。

- [ ] **Step 1: 写失败测试**：①表单含两下拉且 prefill 默认；②提交请求体断言含 provider 字段（mock client）；③不可用项禁用；④story/design 请求体断言含默认 provider；⑤`provider_unavailable` 4xx → 内联错误+弹窗保持打开。
- [ ] **Step 2: 跑确认失败**：`cd web && npm test -- WorkItemPlanOptionsDialog IssueLifecycleWorkbench`
- [ ] **Step 3: 实现**
- [ ] **Step 4: 跑确认通过 + tsc**
- [ ] **Step 5: Commit**：`git add web/src/components/lifecycle/ && git commit -m "feat(workbench): 创建请求携带 provider 快照（plan 弹窗选择器 + story/design 默认填充）"`

## Task 7（WP-C）: 默认应用 hook 化 + legacy 页覆盖 + 生成前 provider 可见

**Files:**
- Create: `web/src/hooks/useProviderDefaultsApplication.ts`
- Modify: `web/src/pages/ChatCockpitPage.tsx:749-783`（补发块整块迁移为 hook 调用；`appliedDefaultsSessionRef` :183 一并迁移）
- Modify: `web/src/pages/ChatWorkspacePageLegacy.tsx`（挂载 hook——其 `selectProvider` 即 `useWorkspaceWs.ts:646-651` 同一函数）
- Modify: `web/src/components/chat-workspace/ChatInputBar.tsx`（「开始生成」按钮 :246 旁显示实际 provider 名+可用状态；provider 不可用时按钮禁用并显示原因）
- Test: `web/src/hooks/`（新 hook 测试）、`web/src/pages/ChatWorkspacePageLegacy*.test.tsx` 就近、`web/src/components/chat-workspace/ChatInputBar.test.tsx`

**Interfaces:**
- Consumes: 现补发块语义（`ChatCockpitPage.tsx:749-783`）：非 takeover、同 session、`stageConfig.providerEditable`、connected、未应用过 → 差异时 `selectProvider("author"/"reviewer")`。
- Produces:
  - `useProviderDefaultsApplication({ ws, sessionId, connected, providerEditable, providers, isTakeover }): void`——幂等（每 session 一次），已锁定/非 PrepareContext 不发（REQ-PPS-02）；
  - legacy 页首次获得默认应用能力（同参调用）；
  - ChatInputBar 在 PrepareContext 且显示开始生成时，旁示 `author` 实际 provider（store providers + availability snapshot；不可用 → 按钮禁用+原因；未确定 → 禁用+「provider 未确定」）——REQ-PPS-03。

- [ ] **Step 1: 写失败测试**：①hook：首次挂载发差异 selectProvider、二次挂载同 session 不发、providerEditable=false 不发、takeover 不发；②legacy 页挂载后默认被应用（页面测试 mock ws 断言调用）；③ChatInputBar：PrepareContext 显示 provider 名；不可用 → 开始生成禁用+原因文案；Running 后不再禁用（锁定态正常输入）。
- [ ] **Step 2: 跑确认失败**：`cd web && npm test -- useProviderDefaultsApplication ChatWorkspacePageLegacy ChatInputBar`
- [ ] **Step 3: 实现**（hook 从 :749-783 原样抽取，行为零变化；cockpit 改调 hook）
- [ ] **Step 4: 跑确认通过 + 全量**：`npm test` 全绿 + `pnpm exec tsc --noEmit` 0
- [ ] **Step 5: Commit**：`git add web/src/hooks/useProviderDefaultsApplication.ts web/src/pages/ChatCockpitPage.tsx web/src/pages/ChatWorkspacePageLegacy.tsx web/src/components/chat-workspace/ChatInputBar.tsx && git commit -m "feat(workspace): provider 默认应用共享 hook 化覆盖 legacy + 生成前实际 provider 可见"`

## Task 8（收口）: 全量门禁 + openspec 逐 delta 核查

**Files:** 无新改动（仅核查与登记）

- [ ] **Step 1: 全量门禁八项**：`cargo fmt --check`；`cargo clippy --all-targets --all-features --locked -- -D warnings`；`cargo test --locked --lib`；`cargo test --test it_core`；`cargo test --test it_web`；`cd web && pnpm exec tsc --noEmit`；`pnpm vitest run`；large_file_guard（随 test 跑）。it_core 两 known-flaky（交接 §7）偶发红复跑确认，不误判回归。
- [ ] **Step 2: openspec 核查**：`openspec validate plan-compile-gate-visibility --strict` 过；对照 5 delta 逐 requirement/scenario 核实现与测试证据（tasks.md 4.1）；确认无 legacy 逐段协议恢复、三命令未扩权。
- [ ] **Step 3: 登记遗留**：provider 跨设备偏好=后续演进（非本 change）；将核查结论追加到 `.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/progress.md`。

## Task 9（收口）: 部署 v43 + 三对账

**Files:** 无代码改动

- [ ] **Step 1: 前端构建**：`cd web && pnpm build`（**必须先于 cargo**，v33 事故铁律），记录新资产指纹
- [ ] **Step 2: 后端构建**：`cargo build --release --locked`
- [ ] **Step 3: 停旧起新**（hub，谱系名 `aria-64-v43`；启动参数沿用交接 §3 固定行；部署窗口=5min durable 无活动）
- [ ] **Step 4: 三对账**：`curl http://127.0.0.1:4317/api/health` + `pgrep -x aria` → `md5sum /proc/<pid>/exe` 与磁盘二进制一致 + `curl -s http://127.0.0.1:4317/ | grep -o 'index-[^"]*'` = 新资产指纹
- [ ] **Step 5: 通知复验**：把「已修+复验靶」追加到 `cadence/notes/2026-09-21_验证记录_全流程E2E.md`（测试会话惯例）并汇报用户复验点（batch 门可见可确认、recovery 三动作、plan 创建带 provider、开始生成前 provider 可见）

---

## 派工与审查（controller 执行）

- **并行波次 1**：Task 1-3（WP-B，ds-task）∥ Task 4-5（WP-A，ds-task）——文件零交集。`ChatCockpitPage.tsx` 仅 WP-A 触碰（Task 5），WP-C 等 WP-A 落地后再起。
- **波次 2**：Task 6-7（WP-C，ds-task）。
- 每波次完成 → k3-reviewer 审查（brief 附本计划路径+对应 spec delta 路径+diff 范围；报告落 `.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/pcg-<wp>-review-k3.md`）；FAIL → fix round 再审；PASS 才进下一波次。
- Task 8-9 由 controller 收口执行。
- 同文件竞态教训（交接 §8）：worker commit 前 `git status` 核对暂存区仅含自己文件；fmt 收口只列自己文件。

## Self-Review 记录

- Spec 覆盖：REQ-PCG-01→Task 3+4+5；REQ-PCG-02→Task 1+2+4+5；REQ-PCG-03→Task 4（只读诊断/终态收口）；REQ-PPS-01→Task 6；REQ-PPS-02→Task 7；REQ-PPS-03→Task 7（ChatInputBar）+Task 6（弹窗可见可选）；REQ-CG-02 recovery 并存/拒绝→Task 1+2；REQ-RET-02 delta→Task 1 注释约束+全任务 Global Constraints；REQ-WSC-01 批次可观察→Task 3+4+5。tasks.md 1.1-1.4→Task 4/5；2.1-2.4→Task 1/2/3；3.1-3.4→Task 6/7；4.1-4.3→Task 8/9。
- provider_start_ledger.provider（SC 流为 None）评估为**不纳入**：REQ-PPS-03 的可见性由 `session_state.providers`（session_state.rs:487-489）承载已足，ledger 属 coding 启动幂等面，扩面即违反「不改状态机」边界。已在此记录决策依据。
- 类型一致性：`kind` 三值、facade 两新方法名、hook 签名在各任务间已对齐；recovery 动作枚举沿用 `WorkItemPlanCompileRecoveryAction`（前端 `workspace.ts:171-175`）。
