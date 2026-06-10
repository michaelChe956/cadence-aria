# CodingWorkspace Provider QA P3 前端展示与交互实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Coding Workspace 前端展示 TestPlan、step evidence、missing required steps、blocked gate metadata，并能发送恢复动作。

**Architecture:** TypeScript 类型跟随后端 v2 DTO；Zustand store 接收 session state 和 gate updates；hook 发送 gate response 后本地移除 pending gate；页面在 Testing 和 Gate 面板展示证据。

**Tech Stack:** React 19、TypeScript、Zustand、Vitest、Testing Library、Vite。

---

## 依赖与边界

- 必须先完成 P1 和 P2。
- 本阶段不改后端协议字段名。
- 前端展示保持工作台风格，不引入新的营销式页面或大重构。

## 文件结构

- Modify: `web/src/api/types.ts`
- Modify: `web/src/api/types.test.ts`
- Modify: `web/src/state/coding-workspace-store.ts`
- Modify: `web/src/state/coding-workspace-store.test.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.test.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`

## Task 1: TypeScript DTO 对齐后端 v2

**Files:**
- Modify: `web/src/api/types.ts`
- Modify: `web/src/api/types.test.ts`

- [ ] **Step 1: 写失败测试**

在 `types.test.ts` 新增 `accepts plan based testing reports and blocked gate metadata`。

构造 `TestingReport`，包含：

- `plan_id`
- `plan_summary`
- `steps`
- `unplanned_commands`
- `missing_required_steps`
- `skipped_required_steps`
- `context_warnings`
- `raw_provider_output_ref`
- `overall_status: "passed_with_warnings"`

构造 `CodingGateRequired`，包含：

- `reason_code`
- `evidence_refs`
- `raw_provider_output_ref`
- action type `rerun_missing_steps`

Run:

```bash
pnpm -C web test -- types.test.ts
```

Expected: FAIL，类型缺字段。

- [ ] **Step 2: 更新类型**

新增：

- `TestPlanTool`
- `TestPlanRiskLevel`
- `TestPlanStep`
- `TestingStepResult`

扩展：

- `TestingOverallStatus` 增加 `passed_with_warnings`
- `TestingReport` 增加 v2 字段
- `ReviewFinding` 增加追踪字段
- `CodeReviewReport.raw_provider_output_ref`
- `InternalPrReview.raw_provider_output_ref`
- `CodingGateActionType` 增加 P2 新动作
- `CodingGateRequired` 增加 metadata

- [ ] **Step 3: 运行测试**

```bash
pnpm -C web test -- types.test.ts
```

Expected: PASS。

## Task 2: Store 与 WebSocket hook

**Files:**
- Modify: `web/src/state/coding-workspace-store.ts`
- Modify: `web/src/state/coding-workspace-store.test.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.test.tsx`

- [ ] **Step 1: 写 store 测试**

新增或扩展测试，断言：

- `setSessionState` 能保存 v2 `testing_report`。
- `pending_gates` 中 blocked gate metadata 不丢失。
- `addPendingGate` 可 upsert blocked gate。
- `resolvePendingGate` 可移除 gate。

Run:

```bash
pnpm -C web test -- coding-workspace-store.test.ts
```

Expected: FAIL 或现有类型不通过。

- [ ] **Step 2: 更新 store**

如果类型更新后 store 逻辑已兼容，只补测试即可。若 v2 report 字段触发 undefined 风险，确保：

- session state 直接保存 `snapshot.testing_report`。
- pending gates 直接保存 `snapshot.pending_gates`。
- `addPendingGate` 用 `gate_id` upsert。

- [ ] **Step 3: hook 响应 gate 后移除 pending gate**

在 `respondGate` 中：

- `sendJson` 失败时不改本地状态。
- `sendJson` 成功后调用 `resolvePendingGate(gateId)`。

新增 hook 测试，断言发送 `gate_response` 后本地 gate 被移除。

Run:

```bash
pnpm -C web test -- useCodingWorkspaceWs.test.tsx
```

Expected: PASS。

## Task 3: Testing 面板展示 plan-based report

**Files:**
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`

- [ ] **Step 1: 写失败测试**

新增测试 `renders plan based testing report details`。

store state 包含 blocked `testingReport`：

- `plan_summary: "API smoke and security review"`
- step `API smoke`
- `missing_required_steps: ["security"]`
- `context_warnings: ["missing_design_spec"]`

断言页面显示：

- `API smoke and security review`
- `API smoke`
- `missing required: security`
- `missing_design_spec`

Run:

```bash
pnpm -C web test -- CodingWorkspacePage.test.tsx
```

Expected: FAIL，页面未展示 v2 字段。

- [ ] **Step 2: 实现 Testing UI**

在 Testing 面板中展示：

- Test Plan summary。
- 每个 step 的 title、required/optional、status、step_id。
- evidence refs。
- provider analysis。
- missing required steps。
- context warnings。
- raw provider output ref。

UI 约束：

- 使用现有卡片/徽章风格。
- 文本过长时 truncate 或 wrap，不允许溢出。
- 不嵌套复杂卡片。

## Task 4: Blocked Gate metadata 与恢复动作

**Files:**
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`

- [ ] **Step 1: 写失败测试**

新增测试 `renders blocked gate metadata and sends recovery action`。

store state 包含 blocked gate：

- `reason_code: "review_payload_parse_error"`
- `evidence_refs: ["code_review_0001.json"]`
- `raw_provider_output_ref: "provider-raw/code_review/code_review_0001.txt"`
- action `retry_review`

断言：

- 页面显示 reason code。
- 页面显示 raw output ref。
- 点击“重试审查”调用 `respondGate(gate_id, "retry_review", undefined)`。

Run:

```bash
pnpm -C web test -- CodingWorkspacePage.test.tsx
```

Expected: FAIL，metadata 未展示。

- [ ] **Step 2: 实现 GatePanel metadata**

在非 stage gate 的 GatePanel 中展示：

- `reason_code`
- `raw_provider_output_ref`
- `evidence_refs`

保留现有 `confirm_stage` 特殊处理逻辑。

- [ ] **Step 3: 运行页面测试**

```bash
pnpm -C web test -- CodingWorkspacePage.test.tsx
```

Expected: PASS。

## 阶段验证

Run:

```bash
pnpm -C web test -- types.test.ts coding-workspace-store.test.ts useCodingWorkspaceWs.test.tsx CodingWorkspacePage.test.tsx
pnpm -C web test
pnpm -C web build
```

Expected: 全部 PASS。

## 提交

```bash
git add web/src/api/types.ts web/src/api/types.test.ts web/src/state/coding-workspace-store.ts web/src/state/coding-workspace-store.test.ts web/src/hooks/useCodingWorkspaceWs.ts web/src/hooks/useCodingWorkspaceWs.test.tsx web/src/pages/CodingWorkspacePage.tsx web/src/pages/CodingWorkspacePage.test.tsx
git commit -m "feat: show coding QA plans and recovery gates"
```
