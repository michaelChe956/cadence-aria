# WorkItemPlan 配置弹窗技术方案

## 背景

Workbench 生成 Work Item Plan 时，后端已经支持以下拆分选项：

- `include_integration_tests`
- `include_e2e_tests`
- `force_frontend_backend_split`
- `require_execution_plan_confirm`

当前前端在 `IssueLifecycleWorkbench` 内直接传固定默认值，用户无法在生成前调整。需要在创建 Work Item Plan Workspace 前补一个配置入口。

## 目标

在 Workbench 从 Design Spec 生成 Work Item Plan 时，先展示一个配置弹窗，让用户确认拆分选项后再创建 Workspace。

默认行为保持不变：

- 包含贯通/集成测试 Work Item：开启
- 包含 E2E 测试 Work Item：关闭
- 强制前后端拆分：开启
- 子 Work Item 执行前需要确认 Plan：关闭

## 非目标

- 不修改后端 API。
- 不做用户级偏好保存。
- 不调整 Work Item Plan 生成后的 Review / Revision 流程。
- 不改变 Story Spec、Design Spec 生成入口。

## 交互设计

点击 `生成 Work Item` 时，不立即调用 `prepareWorkItemPlan`，而是打开 `Work Item Plan 配置`弹窗。

弹窗包含 4 个 checkbox：

1. 包含贯通/集成测试 Work Item
2. 包含 E2E 测试 Work Item
3. 强制前后端拆分
4. 子 Work Item 执行前需要确认 Plan

底部操作：

- `取消`：关闭弹窗，不发请求。
- `创建并打开 Workspace`：使用当前配置调用 `prepareWorkItemPlan`，刷新 Workbench，然后打开返回的 Workspace。

两个现有入口共用同一套弹窗与提交逻辑：

- Drawer 中 confirmed Design Spec 的 `生成 Work Item`
- Design Spec 卡片/生命周期区域触发 Work Item Workspace 的入口

## 数据流

新增一个待启动状态，记录被点击的 Design Spec card 与启动目标。

确认弹窗后调用：

```ts
prepareWorkItemPlan(projectId, issueId, {
  title,
  story_spec_ids,
  design_spec_ids,
  include_integration_tests,
  include_e2e_tests,
  force_frontend_backend_split,
  require_execution_plan_confirm,
})
```

请求成功后：

1. `refresh(selectedProjectId)`
2. `onOpenWorkspace(response.workspace_session.workspace_session_id)`
3. 清理待启动状态并关闭弹窗

请求失败时保持弹窗打开并展示错误，避免用户丢失已选配置。

## 组件方案

新增 `WorkItemPlanOptionsDialog`：

- props 接收 `defaultOptions`、`onConfirm`、`onClose`
- 内部维护 4 个 checkbox 状态
- submit 时把选项交给父组件
- loading 时禁用按钮，避免重复提交

`IssueLifecycleWorkbench` 负责：

- 打开弹窗
- 组装 title / story_spec_ids / design_spec_ids
- 调用 `prepareWorkItemPlan`
- 刷新和打开 Workspace

## 测试方案

先补前端测试再实现：

1. 点击 `生成 Work Item` 会打开配置弹窗，且不会立即调用 `work-item-plans:prepare`。
2. 默认确认时，请求体保持当前默认值。
3. 修改 E2E 与执行前确认等选项后，请求体带用户选择的值。
4. 点击取消不会调用 `work-item-plans:prepare`。

验证命令：

```bash
pnpm exec vitest --run src/components/lifecycle/IssueLifecycleWorkbench.test.tsx
pnpm build
```

## 自检

- 无残留待办标记或需求空洞。
- 入口、默认值、请求体字段与后端已有 API 一致。
- 范围限定在 Work Item Plan 创建前配置 UI，不牵涉后端和后续 Review 流程。
