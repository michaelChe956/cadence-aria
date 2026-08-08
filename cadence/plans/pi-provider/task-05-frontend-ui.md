# Task 5: 前端 Provider 目录 + 权限控制 + 失败状态可见性（tasks 5.1, 5.2）

> 自包含任务文件。执行前请先读 `00-overview.md` 的 Global Constraints 与跨任务接口契约。依赖 Task 1（前端 catalog 含 Pi）、Task 3（`ProviderConfigSnapshot.permission_modes`）、Task 4（`CodingRolePermissionModes` 默认 Auto）。

**Goal:** 在普通 Workspace 与 Coding Workspace 的 Provider 配置中展示 Pi；Claude Code 与 Codex 提供一致的 `Auto`/`Supervised` 控制，Pi 仅显示 `Auto`；运行失败状态可见。

**对应 spec requirement:**
- 「Provider 权限模式默认为 Auto，Pi 仅支持 Auto」（前端权限控制，Pi 仅 Auto）
- 「Pi 在活跃 Provider 工作流中可发现且可选择」（前端选择器含 Pi）

**背景（类型链，关键）：** 普通 Workspace 前端有两个相关类型：
- `WsProviderConfig`（`web/src/api/types/workspace.ts:278`）：`{author, reviewer?}`，**无** `review_rounds`，用于 SessionState 回显。
- `ProviderConfigSnapshot`（`web/src/api/types/common.ts:226`）：`{author, reviewer?, review_rounds}`，**真正发送**给后端的 wire 类型。
- `providerConfigFor()`（`web/src/pages/ChatWorkspacePageParts.tsx:335`）：从面板状态构造 `ProviderConfigSnapshot` 发给后端。

权限模式必须打通完整链路：面板选择 → workspace store state → `providerConfigFor()` → `ProviderConfigSnapshot.permission_modes` → 后端。`ProviderConfigPanel` 的真实 props 是 `providers: WsProviderConfig | null`（**不是** `healthSnapshot` prop），健康快照从 `useProviderAvailabilityStore` 读。

**Files:**
- Modify: `web/src/api/types/common.ts:226`（`ProviderConfigSnapshot` 加 `permission_modes`）
- Modify: `web/src/api/types/workspace.ts:278`（`WsProviderConfig` 加 `permission_modes`，SessionState 回显）
- Modify: `web/src/components/workspace/ProviderConfigPanel.tsx`（加 Author/Reviewer 权限模式选择 + `onPermissionModeSelect` callback；Pi 仅 Auto）
- Modify: `web/src/pages/ChatWorkspacePage.tsx`（绑定权限 mode state + setter）
- Modify: `web/src/pages/ChatWorkspacePageParts.tsx:335`（`providerConfigFor()` 接受并序列化 permission_modes）
- Modify: workspace store（权限 mode state + 更新 action）
- Modify: `web/src/components/coding-workspace/CodingProviderConfigPanel.tsx`（确认 Pi 出现在已有权限控件，Pi 仅 Auto）
- Test: `web/src/components/workspace/ProviderConfigPanel.test.tsx`、`web/src/components/coding-workspace/CodingProviderConfigPanel.test.tsx`、`web/src/pages/ChatWorkspacePageParts.test.tsx`

**Interfaces:**
- Consumes: `getProviderOptions(snapshot)`（Task 1 含 Pi）、`ProviderConfigSnapshot.permission_modes`（Task 3 wire）、`CodingRolePermissionModes`（Task 4）、`useProviderAvailabilityStore`。
- Produces: 普通 Workspace 面板含每角色权限控制（Pi 仅 Auto）；权限模式经 `providerConfigFor` 发到后端；不可用 Pi 禁用 + 原因；失败状态可见。

---

## Step 1: 写失败测试 -- 普通 Workspace 面板展示 Pi + 权限模式选择

**测试模板依据：** 照 `web/src/components/workspace/ProviderConfigPanel.test.tsx` 现有写法——用 `useProviderAvailabilityStore.setState()` 设健康快照，`providerEntry()`/`setProviderHealth()` 是该文件已有 helper。`providers` prop 用真实 `WsProviderConfig`（`{author, reviewer?}`，**无** `review_rounds`）。

```ts
function piEntry(available: boolean): ProviderHealthEntry {
  return {
    provider: "pi",
    display_name: "Pi",
    available,
    version: available ? "0.83.0" : null,
    reason_code: available ? null : "command_missing",
    reason: available ? null : "pi 未安装",
    checked_at: "2026-07-31T00:00:00Z",
    install_hint: "安装 pi",
  };
}

function setHealthWithPi(piAvailable: boolean) {
  useProviderAvailabilityStore.setState({
    snapshot: {
      schema_version: 1,
      generation: 1,
      checked_at: "2026-07-31T00:00:00Z",
      state_status: "ready",
      state_error: null,
      real_workflow_blocked: false,
      test_provider_enabled: false,
      providers: [providerEntry("claude_code", true), providerEntry("codex", true), piEntry(piAvailable)],
    },
  });
}

it("author 可选 Pi，且 Pi 角色只提供 Auto", () => {
  setHealthWithPi(true);
  const onPermissionModeSelect = vi.fn();
  render(
    <ProviderConfigPanel
      providers={{ author: "pi", reviewer: "codex" }}
      editable
      onSelectProvider={() => {}}
      reviewerEnabled
      onToggleReviewer={() => {}}
      permissionModes={{ author: "auto", reviewer: "auto" }}
      onPermissionModeSelect={onPermissionModeSelect}
    />,
  );

  // author provider 选择器含 Pi
  const authorSelect = screen.getByLabelText("Author Provider");
  expect(within(authorSelect).getByRole("option", { name: /Pi/ })).toBeTruthy();

  // author 是 Pi 时，权限控件只有 Auto，没有 Supervised
  const authorModes = screen.getByTestId("author-permission-mode");
  expect(within(authorModes).getByRole("button", { name: "Auto" })).toBeTruthy();
  expect(within(authorModes).queryByRole("button", { name: "Supervised" })).toBeNull();

  // reviewer 是 Codex，两种模式都在
  const reviewerModes = screen.getByTestId("reviewer-permission-mode");
  expect(within(reviewerModes).getByRole("button", { name: "Supervised" })).toBeTruthy();
});

it("Pi 不可用时选项禁用且显示原因", () => {
  setHealthWithPi(false);
  render(
    <ProviderConfigPanel
      providers={{ author: "pi", reviewer: "codex" }}
      editable
      onSelectProvider={() => {}}
      reviewerEnabled
      onToggleReviewer={() => {}}
      permissionModes={{ author: "auto", reviewer: "auto" }}
      onPermissionModeSelect={() => {}}
    />,
  );
  expect(screen.getByText(/pi 未安装/)).toBeTruthy();
  // Pi option 应禁用
  const authorSelect = screen.getByLabelText("Author Provider");
  const piOption = within(authorSelect).getByRole("option", { name: /Pi/ });
  expect(piOption).toBeDisabled();
});
```

注：`providerEntry` 是该测试文件已有 helper（当前只处理 claude_code/codex，需扩展或用上面的 `piEntry`）。`getByLabelText("Author Provider")` / `getByTestId("author-permission-mode")` 的实际查询串以 Step 2 实现的 DOM 结构为准，实现时同步这两处。

- [ ] Run: `cd web && npm test ProviderConfigPanel`
- Expected: FAIL -- 面板无权限模式控件 / 无 `permissionModes`、`onPermissionModeSelect` prop


## Step 2: 面板加权限控件 + Pi 仅 Auto

`web/src/components/workspace/ProviderConfigPanel.tsx`：
- 加 props：`permissionModes: { author: "auto"|"supervised"; reviewer: "auto"|"supervised" }` 与 `onPermissionModeSelect(role, mode)`。
- 参照 `CodingProviderConfigPanel.tsx` 的权限选择 UI（`permissionMode` 选择 + `auto`/`supervised` 文案），为 Author/Reviewer 各加权限模式选择。
- **Pi 仅 Auto**：当某角色 provider 为 `"pi"` 时，该角色权限控件只显示/可选 `Auto`（Supervised 选项禁用或隐藏），文案说明「Pi 仅支持 Auto」。

- [ ] Run: `cd web && npm test ProviderConfigPanel`
- Expected: PASS

## Step 3: 类型链打通 —— `ProviderConfigSnapshot` 与 `WsProviderConfig` 加 `permission_modes`

`web/src/api/types/common.ts:226`：

```ts
export type ProviderPermissionMode = "auto" | "supervised";

export type ProviderConfigSnapshot = {
  author: WorkspaceProviderName;
  reviewer?: WorkspaceProviderName | null;
  review_rounds: number;
  permission_modes?: { author: ProviderPermissionMode; reviewer: ProviderPermissionMode };
};
```

`web/src/api/types/workspace.ts:278` `WsProviderConfig` 加同样的 `permission_modes?` 字段（SessionState 回显）。

- [ ] Run: `cd web && npm run build`
- Expected: 类型编译通过

## Step 4: `providerConfigFor()` 序列化 permission_modes + `providerNameFor` 接受 pi + workspace store state

`web/src/pages/ChatWorkspacePageParts.tsx:335` `providerConfigFor()` 接受权限模式并写入返回的 `ProviderConfigSnapshot.permission_modes`。

**高1 前端（关键）：** `providerConfigFor()` 内部的 `providerNameFor()`（`ChatWorkspacePageParts.tsx:359`）当前只认 `claude_code`/`codex`/`fake`，未知 provider 会回退成 `claude_code`。必须让它接受 `"pi"`：

```ts
function providerNameFor(value: string | undefined | null, fallback: WorkspaceProviderName): WorkspaceProviderName {
  if (value === "claude_code" || value === "codex" || value === "pi" || value === "fake") {
    return value;
  }
  return fallback;
}
```

workspace store 加权限 mode state 与更新 action；`ChatWorkspacePage.tsx` 绑定 state + setter，传给 `ProviderConfigPanel` 的 `permissionModes`/`onPermissionModeSelect`。

`web/src/pages/ChatWorkspacePageParts.test.tsx` 加测试：

```ts
it("providerConfigFor 保留 pi 选择不回退", () => {
  const snapshot = providerConfigFor({ author: "pi", reviewer: "codex" }, true, 1, { author: "auto", reviewer: "auto" });
  expect(snapshot.author).toBe("pi");
  expect(snapshot.permission_modes?.author).toBe("auto");
});
```

- [ ] Run: `cd web && npm test ChatWorkspacePageParts`
- Expected: PASS

## Step 5: Coding 面板 Pi 仅 Auto + 服务端规范化

`web/src/components/coding-workspace/CodingProviderConfigPanel.tsx` 已有权限控件（`:160-192` 无条件渲染 `["auto","supervised"]`）。确认 Task 1 的 catalog 改动让 Pi 出现在三角色选择器；当角色选 Pi 时，权限控件 filter 为仅 `auto`：

```ts
// 角色当前 provider === "pi" 时，模式列表过滤为 ["auto"]
const modes = roleProvider === "pi" ? ["auto"] : ["auto", "supervised"];
```

文案说明「Pi 仅支持 Auto」。

**服务端规范化（防陈旧数据/API 输入）：** 在 Coding 的 provider-selection / permission-mode 更新入口（后端，定位 `rg -n "CodingRolePermissionModes" src/product/coding_workspace_engine/ src/web/coding_ws_handler/ -g '*.rs'`）加 validation：若某角色 provider 为 `Pi`，强制其 mode 为 `Auto`。

`CodingProviderConfigPanel.test.tsx` 补测试：三角色均可选 Pi，选 Pi 时权限控件仅 Auto。

后端测试（`tests/it_product/product_coding_models.rs` 或对应）补：直接构造 `Pi + Supervised` 的 snapshot，断言保存/运行输入被规范化为 `Auto`。

- [ ] Run: `cd web && npm test CodingProviderConfigPanel`；`cargo test -p cadence-aria coding_models`
- Expected: PASS

## Step 6: 不可用 Pi 禁用 + 原因；失败状态可见

确认：Pi 不可用时选择器保留已配置值但禁用，显示 `reason`/`install_hint`（复用现有 `blockedReason`/`realProviderOption`，Task 1 已覆盖）。运行失败经 `ProviderEvent` → 前端状态链路显示（fail-fast：失败即显示失败，不切换）。

补测试：Pi 不可用时选项 disabled 且显示原因。

- [ ] Run: `cd web && npm test`
- Expected: PASS

## Step 7: 前端全量测试 + Commit

- [ ] Run:

```bash
cd web && npm test && npm run build && cd ..
git add web/src/api/types/common.ts web/src/api/types/workspace.ts web/src/components/workspace/ProviderConfigPanel.tsx web/src/pages/ChatWorkspacePage.tsx web/src/pages/ChatWorkspacePageParts.tsx web/src/components/coding-workspace/CodingProviderConfigPanel.tsx
git commit -m "feat(web): show Pi (Auto-only) and per-role permission controls in workspace and coding provider config"
```

---

## 完成检查（对应 tasks 5.1/5.2）

- [ ] 5.1：普通/Coding Workspace 的 Provider 配置展示 Pi；Claude Code 与 Codex 提供一致的 `Auto`/`Supervised` 控制，Pi 仅显示 `Auto`。
- [ ] 5.2：运行事件与界面状态呈现不可用原因与失败状态。
