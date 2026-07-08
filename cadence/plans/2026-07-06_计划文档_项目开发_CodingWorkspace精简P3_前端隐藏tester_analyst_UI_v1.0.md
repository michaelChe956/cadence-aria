# Coding Workspace 精简 Plan 3：前端隐藏 tester/analyst UI

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 前端隐藏 tester/analyst 相关的 UI 展示和配置项，使界面与新的 coder+reviewer 双角色链路保持一致。类型字段保留不删（向后兼容）。

**Architecture:** 改动集中在三处：① `CodingWorkspaceControls.tsx` 中 `lockedProviderRole`/`providerRoleForStage` 的 testing/rework→analyst 映射，以及 gate 区域的 `testingResultReview`/`testingBlocked`/`analystGate` 显示；② `CodingWorkspacePage.tsx` 中 header 的 provider 信息展示（去掉 Tester 字样）；③ provider 选择面板（如有）隐藏 tester/analyst 选项。类型定义（`CodingProviderRole`、`CodingRoleProviderConfigSnapshot` 等）保留不删。

**Tech Stack:** TypeScript + React + Tailwind CSS，Vitest 测试，`cd web && pnpm test`。

## Global Constraints

- 禁止使用 npm/yarn，只用 pnpm
- 前端类型字段（`tester`、`analyst`、`tester_plan`、`tester_execute`）保留不删，仅隐藏 UI
- 测试命令：`cd web && pnpm test`；类型检查：`cd web && pnpm tsc -b`

---

### Task 1：清理 CodingWorkspaceControls.tsx 中的 testing/analyst 显示

**Files:**
- Modify: `web/src/pages/CodingWorkspaceControls.tsx`

**Interfaces:**
- Consumes: `CodingGateRequired`（含 `role`、`stage`、`reason_code`）、`CodingExecutionStage`、`CodingProviderRole`
- Produces: `lockedProviderRole` 不再返回 `tester`/`analyst`；gate 区域不再显示 testing/analyst 专属文案

- [x] **Step 1: 修改 providerRoleForStage，去掉 testing 和 rework 映射**

在 `CodingWorkspaceControls.tsx` 中找到 `providerRoleForStage` 函数（约 421 行），修改为：

```typescript
function providerRoleForStage(stage: CodingExecutionStage): CodingProviderRole | null {
  switch (stage) {
    case "coding":
      return "coder";
    case "code_review":
      return "code_reviewer";
    case "internal_pr_review":
      return "internal_reviewer";
    // testing 和 rework 阶段不再有独立的角色 UI
    default:
      return null;
  }
}
```

- [x] **Step 2: 清理 gate 区域的 testingBlocked / testingResultReview / analystGate 显示**

在约 278 行处，删除以下三个变量及其 JSX 使用：

```typescript
// 删除这三个变量
const testingResultReview = activeGate.reason_code === TESTING_RESULT_REVIEW_REASON_CODE;
const testingBlocked = activeGate.stage === "testing" && !testingResultReview;
const analystGate = activeGate.role === "analyst";
```

同时删除 JSX 中对应的三个条件渲染块：

```tsx
{/* 删除 */}
{testingBlocked ? (
  <div className="mt-0.5 text-xs font-semibold text-amber-900">测试被阻塞</div>
) : null}
{/* 删除 */}
{testingResultReview ? (
  <div className="mt-0.5 text-xs font-semibold text-amber-900">
    等待确认 Tester 结果
  </div>
) : null}
{/* 删除 */}
{analystGate ? (
  <div className="mt-0.5 text-xs font-semibold text-amber-900">
    Analyst 建议人工决策
  </div>
) : null}
```

- [x] **Step 3: 检查 TESTING_RESULT_REVIEW_REASON_CODE 是否还有其他使用方**

```bash
grep -rn "TESTING_RESULT_REVIEW_REASON_CODE" web/src/
```

若只在 Step 2 删掉的代码中使用，同时删除该常量的 import 或定义。

- [x] **Step 4: 类型检查**

```bash
cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0630/web
pnpm tsc -b 2>&1 | head -30
```

预期：0 errors。

- [x] **Step 5: 运行前端测试**

```bash
pnpm test 2>&1 | tail -30
```

若有测试因删除 testing/analyst 文案而失败，更新断言以匹配新 UI（不再断言 testing/analyst 文案存在）。

- [x] **Step 6: 提交**

```bash
cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0630
git add web/src/pages/CodingWorkspaceControls.tsx
git commit -m "feat(web): hide testing/analyst UI in coding workspace controls"
```

---

### Task 2：清理 CodingWorkspacePage.tsx 的 provider header 展示

**Files:**
- Modify: `web/src/pages/CodingWorkspacePage.tsx`

**Interfaces:**
- Consumes: `store.roleProviderConfigSnapshot`（含 `coder`、`tester_plan`、`tester_execute`、`analyst`、`code_reviewer`）
- Produces: header 信息只展示 coder / code_reviewer / internal_reviewer

- [x] **Step 1: 定位并修改 provider header 字符串**

在 `CodingWorkspacePage.tsx:50` 附近找到：

```typescript
? `Coder ${store.roleProviderConfigSnapshot.coder} · Tester ${store.roleProviderConfigSnapshot.tester_plan}/${store.roleProviderConfigSnapshot.tester_execute}`
```

改为：

```typescript
? `Coder ${store.roleProviderConfigSnapshot.coder} · Reviewer ${store.roleProviderConfigSnapshot.code_reviewer}`
```

- [x] **Step 2: 检查是否还有其他地方展示 tester/analyst provider 名称**

```bash
grep -rn "tester_plan\|tester_execute\|\.analyst\b" web/src/pages/CodingWorkspacePage.tsx
grep -rn "tester_plan\|tester_execute\|\.analyst\b" web/src/pages/CodingWorkspaceReports.tsx
```

对每处出现：若是 UI 展示，改为显示 coder/reviewer；若是类型操作（传参、存储），保留不动。

- [x] **Step 3: 类型检查 + 测试**

```bash
cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0630/web
pnpm tsc -b 2>&1 | head -20
pnpm test 2>&1 | tail -20
```

- [x] **Step 4: 提交**

```bash
cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0630
git add web/src/pages/CodingWorkspacePage.tsx web/src/pages/CodingWorkspaceReports.tsx
git commit -m "feat(web): update provider header to show coder+reviewer only"
```

---

### Task 3：隐藏 provider 选择面板中的 tester/analyst 角色

**Files:**
- Modify: 包含 provider 选择 UI 的组件（执行时先用下方命令定位）

**Interfaces:**
- Consumes: `CodingProviderSelectRole` 类型、provider 选择下拉/按钮组件
- Produces: UI 中不再展示 tester/analyst 的 provider 选择项

- [x] **Step 1: 定位 provider 选择相关组件**

```bash
cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0630
grep -rn "onSelect\|ProviderSelect\|provider.*select\|selectRole\|tester.*provider\|analyst.*provider" \
    web/src/pages/ web/src/components/ 2>/dev/null | grep -v "test\." | head -20
```

找到渲染 provider 选择 UI 的组件文件（可能是 `CodingWorkspaceControls.tsx` 内部或单独组件）。

- [x] **Step 2: 隐藏 tester/analyst 角色的选择项**

根据 Step 1 找到的组件，将 provider 选择的角色列表从：

```typescript
// 原来可能包含 tester_plan / tester_execute / analyst 的角色
const selectableRoles = ["coder", "tester_plan", "tester_execute", "analyst", "code_reviewer", "internal_reviewer"];
```

改为只展示：

```typescript
const selectableRoles = ["coder", "code_reviewer", "internal_reviewer"];
```

若组件使用 `CodingProviderSelectRole` 类型动态枚举，则在渲染时过滤：

```typescript
// 过滤掉已不在新流程中使用的角色
const visibleRoles = roles.filter(
  (role) => !["tester_plan", "tester_execute", "tester", "analyst"].includes(role)
);
```

- [x] **Step 3: 类型检查 + 测试**

```bash
cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0630/web
pnpm tsc -b 2>&1 | head -20
pnpm test 2>&1 | tail -20
```

- [x] **Step 4: 提交**

```bash
cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0630
git add web/src/
git commit -m "feat(web): hide tester/analyst provider selection from UI"
```

---

### Task 4：e2e 冒烟验证（可选，有 Playwright 环境时执行）

**Files:**
- 无新增修改；只运行验证

- [x] **Step 1: 运行前端完整测试**

```bash
cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0630/web
pnpm test 2>&1 | grep -E "Tests|FAIL|PASS" | tail -20
```

预期：全绿。

- [ ] **Step 2: 若有 Playwright 环境，运行 e2e 冒烟**

```bash
pnpm test:e2e 2>&1 | tail -30
```

预期：无与 tester/analyst UI 相关的 e2e 失败。

> 本项为可选验证，当前按用户要求不由 Agent 执行，保留给用户手动验证。

- [x] **Step 3: 最终提交确认**

确认三个 plan 的所有 commit 均在 `feat-b-0630` 分支上，推送到远端：

```bash
cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0630
git log --oneline -15
git push origin feat-b-0630
```
