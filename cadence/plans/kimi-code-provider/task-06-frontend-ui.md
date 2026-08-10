# Task 6: 前端 UI 与运行可见性（tasks 6.1, 6.2）

> 自包含任务文件。执行前请先读 `00-overview.md`。依赖 Task 1（catalog/order）、Task 3（Supervised+AskUserQuestion 能力）。

**Goal:** 前端 Provider 配置展示 Kimi；Kimi 提供 Auto/Supervised 控制（与 Claude/Codex 一致，区别 Pi 的 Auto-only）；穷尽 union/match 补 Kimi；运行事件与界面呈现不可用原因（版本过低/未登录）与失败状态。

**对应 spec requirement:**
- 「Kimi 在活跃 Provider 工作流中可发现且可选择」（前端展示）
- 「Kimi 权限模式默认 Auto 且支持 Supervised」（UI 控件）
- 「Kimi 复用既有授权与失败边界」（不可用/失败可见性）

**Files:**
- Modify: `web/src/components/workspace/ProviderConfigPanel.tsx`（权限控件：Kimi 显示 Auto+Supervised，**非** Pi 的 Auto-only）
- Modify: `web/src/components/coding-workspace/CodingProviderConfigPanel.tsx`（同上）
- Modify: `web/src/pages/ChatWorkspacePageParts.tsx`（默认 fallback 顺序含 kimi_code）
- Modify: `web/src/hooks/workspace-ws-message-handler.ts`（WebSocket parser 含 kimi_code）
- Modify: `web/src/components/lifecycle/CreateRepositoryDialog.tsx`（Task 1 已过滤 Kimi，此 task 验证 + 测试）
- Modify: 相关 `.test.tsx` 的 fixture union（加 kimi_code）
- Verify: `web/src/api/types/provider.ts`（Task 1 已加 RealProviderName）、`web/src/state/provider-options.ts`（Task 1 已加 catalog/order）

**Interfaces:**
- Consumes: 后端状态 API（health DTO 含 Kimi）、Task 1 catalog。
- Produces: 前端完整 Kimi 体验。**Task 7 依赖。**

**参照：** `ProviderConfigPanel.tsx` 中 Pi 的 Auto-only 处理（Kimi 此处**不**套用，Kimi 显示 Auto+Supervised，与 Claude/Codex 一致）。

---

## Step 1: 权限控件 Kimi 显示 Auto+Supervised（失败测试先行）

`web/src/components/workspace/ProviderConfigPanel.test.tsx` 加：
```tsx
// Kimi 应同时显示 Auto 与 Supervised（区别于 Pi 仅 Auto）
// 选中 Kimi 时权限下拉含两个选项
```
`web/src/components/coding-workspace/CodingProviderConfigPanel.test.tsx` 同。

- [ ] Run: `cd web && pnpm test -- ProviderConfigPanel` 与 `CodingProviderConfigPanel`
- Expected: FAIL（若 Kimi 被误套用 Pi 的 Auto-only 过滤）

实现：确认 ProviderConfigPanel 的"仅 Auto"过滤只对 `"pi"` 生效，**不含** `"kimi_code"`。Kimi 显示完整 Auto/Supervised。

- [ ] Run: 同上
- Expected: PASS

## Step 2: WebSocket parser + 默认 fallback 含 kimi_code

`web/src/hooks/workspace-ws-message-handler.ts`：provider name 解析的穷尽 union/match 加 `"kimi_code"`。
`web/src/pages/ChatWorkspacePageParts.tsx`：默认 fallback provider 顺序含 `"kimi_code"`（与 PROVIDER_ORDER 一致）。

- [ ] Run: `cd web && pnpm test -- workspace-ws-message-handler` 与 `ChatWorkspace`
- Expected: 含 kimi_code；先 FAIL 后 PASS。

## Step 3: fixture union + 仓库初始化过滤测试

更新相关 `.test.tsx` 的 fixture/request union 含 `kimi_code`。
`web/src/components/lifecycle/CreateRepositoryDialog.test.tsx`：断言仓库初始化选项**不含** kimi_code（Task 1 已实现过滤，此 task 补测试）。

- [ ] Run: `cd web && pnpm test -- CreateRepositoryDialog` 及受影响 fixture 测试
- Expected: PASS

## Step 4: 错误文案可见性

确认 health snapshot 的 reason（版本过低 / 未登录运行错误）经 `provider-options.ts` 的 `blockedReason` 正确显示为禁用原因（既有逻辑，Kimi 复用；若 Kimi 的 reason 字段格式不同则适配）。

- [ ] Run: `cd web && pnpm test -- provider-options`
- Expected: Kimi 不可用时显示原因。

## Step 5: 质量检查与提交

- [ ] Run: `cd web && pnpm tsc -b && pnpm test`
- Expected: 全绿
- [ ] Commit:
```bash
git add -A
git commit -m "feat(kimi): Task 6 前端 UI（权限控件 Auto+Supervised/WebSocket parser/fallback 顺序/仓库初始化过滤测试/错误文案）"
```
