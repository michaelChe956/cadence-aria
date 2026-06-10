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

- [x] **Step 1: 写失败测试**

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

- [x] **Step 2: 更新类型**

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
- 所有新增字段必须兼容旧 snapshot 缺字段：
  - `TestingReport.plan_id` 可为 `null` 或缺失。
  - `TestingReport.steps` 缺失时按空数组展示。
  - `CodingGateRequired.reason_code` 缺失时不展示 metadata 行。
  - `raw_provider_output_ref` 缺失时不渲染路径。

- [x] **Step 3: 运行测试**

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

- [x] **Step 1: 写 store 测试**

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

- [x] **Step 2: 更新 store**

如果类型更新后 store 逻辑已兼容，只补测试即可。若 v2 report 字段触发 undefined 风险，确保：

- session state 直接保存 `snapshot.testing_report`。
- pending gates 直接保存 `snapshot.pending_gates`。
- `addPendingGate` 用 `gate_id` upsert。

- [x] **Step 3: hook 响应 gate 后等待后端确认**

在 `respondGate` 中：

- `sendJson` 失败时不改本地状态。
- `sendJson` 成功后不要立即调用 `resolvePendingGate(gateId)`。
- 先把对应 gate 标记为 `submitting`，禁用按钮并显示处理中。
- 收到后端 `coding_session_state` 且该 `gate_id` 不再存在时，才调用 `resolvePendingGate(gateId)` 或直接由 `setSessionState` 替换。
- 收到 `coding_protocol_error` 时恢复 gate 可点击状态，并显示错误码。

新增 hook 测试，断言发送 `gate_response` 后本地 gate 进入 submitting，但不会立即移除；收到后端 snapshot 后才移除。

Run:

```bash
pnpm -C web test -- useCodingWorkspaceWs.test.tsx
```

Expected: PASS。

## Task 3: Testing 面板展示 plan-based report

**Files:**
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`

- [x] **Step 1: 写失败测试**

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

- [x] **Step 2: 实现 Testing UI**

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
- 旧 report 缺少 v2 字段时展示旧 `commands` 列表，不显示空的 TestPlan 区块。
- `raw_provider_output_ref` 只作为证据路径文本展示；除非后端已有安全读取 API，不直接构造可点击本地文件链接。

## Task 4: Blocked Gate metadata 与恢复动作

**Files:**
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`

- [x] **Step 1: 写失败测试**

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
- 对 `manual_continue` 或 `accept_risk` action，页面要求用户填写原因；空原因时不发送 `gate_response`。
- 填写原因后调用 `respondGate(gate_id, "manual_continue", reason)`。

Run:

```bash
pnpm -C web test -- CodingWorkspacePage.test.tsx
```

Expected: FAIL，metadata 未展示。

- [x] **Step 2: 实现 GatePanel metadata**

在非 stage gate 的 GatePanel 中展示：

- `reason_code`
- `raw_provider_output_ref`
- `evidence_refs`
- gate submitting 状态。
- `coding_protocol_error` 的错误码和简短说明。
- 对 `manual_continue` / `accept_risk` 展示原因输入框，placeholder 为“说明跳过该门禁的原因和后续风险处理”。
- 原因输入框最多 2000 字，超长时前端禁用提交。

保留现有 `confirm_stage` 特殊处理逻辑。

- [x] **Step 3: 运行页面测试**

```bash
pnpm -C web test -- CodingWorkspacePage.test.tsx
```

Expected: PASS。

## Task 5: Gate 响应确认流、错误态与旧数据兼容

**Files:**
- Modify: `web/src/state/coding-workspace-store.ts`
- Modify: `web/src/state/coding-workspace-store.test.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.test.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`

- [x] **Step 1: 写 gate submitting store 测试**

新增测试 `tracks_gate_submission_without_removing_gate_until_snapshot_confirms`。

断言：

- `markGateSubmitting(gateId)` 后 gate 仍在 `pendingGates` 中。
- gate metadata 包含 `submitting: true`。
- `setSessionState` 收到不含该 gate 的 snapshot 后，gate 被移除。
- `setGateError(gateId, "coding_gate_response_failed")` 后 gate 仍在，`submitting: false`，错误码可读。

Run:

```bash
pnpm -C web test -- coding-workspace-store.test.ts
```

Expected: FAIL，store 还没有 gate submitting/error 状态。

- [x] **Step 2: 写 hook 确认流测试**

新增测试 `respond_gate_waits_for_server_snapshot_before_resolving_gate`。

断言：

- 调用 `respondGate("gate_1", "retry_review", undefined)` 后发送 `gate_response`。
- 发送成功后本地仍存在 gate，但按钮态为 submitting。
- 模拟收到 `coding_protocol_error` 时 gate 恢复可点击并显示错误。
- 模拟收到不含 `gate_1` 的 `coding_session_state` 后 gate 被移除。
- 对 `manual_continue` action，空原因不发送消息；填写原因后发送 `extra_context`。

Run:

```bash
pnpm -C web test -- useCodingWorkspaceWs.test.tsx
```

Expected: FAIL，当前发送成功后可能直接移除 gate 或没有错误态。

- [x] **Step 3: 写旧 report 页面兼容测试**

新增测试 `renders_legacy_testing_report_without_plan_fields`。

构造只包含旧字段的 `testingReport`：

```ts
{
  id: "testing_report_0001",
  attempt_id: "coding_attempt_0001",
  commands: [],
  overall_status: "passed",
  provider_claim: null,
  backend_verified: true,
  started_at: "2026-06-10T00:00:00Z",
  completed_at: "2026-06-10T00:00:01Z"
}
```

断言：

- 页面不崩溃。
- 显示测试通过状态。
- 不显示空的 Test Plan summary。

Run:

```bash
pnpm -C web test -- CodingWorkspacePage.test.tsx
```

Expected: FAIL 或类型不通过。

- [x] **Step 4: 实现前端确认流与兼容展示**

实现要求：

- store 增加 `submittingGateIds` 和 `gateErrors`，或在 `pendingGates` 外维护同等状态。
- `respondGate` 只负责发送和标记 submitting，不直接删除 gate。
- `coding_session_state` 是 gate 是否移除的唯一确认来源。
- `coding_protocol_error` 将错误绑定到当前 gate。
- GatePanel 的 action button 在 submitting 时禁用并显示“处理中”。
- GatePanel 对 `manual_continue` / `accept_risk` action 发送前要求非空原因，原因作为 `extra_context`。
- GatePanel 限制原因输入最多 2000 字，超长时显示错误并禁用提交。
- 旧 report 缺 v2 字段时走 legacy commands 展示路径。

- [x] **Step 5: 运行补充前端测试**

```bash
pnpm -C web test -- coding-workspace-store.test.ts useCodingWorkspaceWs.test.tsx CodingWorkspacePage.test.tsx
```

Expected: 全部 PASS。

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
