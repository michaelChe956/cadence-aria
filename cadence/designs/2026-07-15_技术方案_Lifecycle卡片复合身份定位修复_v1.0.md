# Lifecycle 卡片复合身份定位修复技术方案

## 文档信息

- 日期：2026-07-15
- 版本：v1.0
- 适用范围：Issue 生命周期工作台中的 Story Spec、Design Spec、Work Item Group
- 目标分支：`feat-b-0715`

## 问题现象

同一 Project 下，不同 Issue 会分别从 `0001` 开始生成 Story Spec、Design Spec 和 Work Item Group ID。例如 `issue_0001` 与 `issue_0002` 都可以拥有 `story_spec_0001`。

当前前端打开右侧抽屉时只保存实体 ID，并在所有 Issue 的卡片中按实体 ID 查找第一个匹配项。因此点击 `issue_0002` 的 `story_spec_0001` 时，可能错误展示 `issue_0001` 的 Story Spec；后续打开 Workspace、生成 Design Spec 等操作也会继续使用错误卡片。

## 根因

生命周期实体的实际身份是以下三元组：

```text
卡片类型 + Issue ID + 实体 ID
```

现有实现只在部分选中状态中使用 `卡片类型 + 实体 ID`，抽屉状态和 URL `focus` 参数仍只使用实体 ID，缺少 Issue 维度。

## 方案

### 复合身份键

统一使用以下格式标识生命周期卡片：

```text
<kind>:<issue_id>:<entity_id>
```

示例：

```text
story_spec:issue_0002:story_spec_0001
design_spec:issue_0002:design_spec_0001
work_item_group:issue_0002:issue_work_item_plan_0001
```

复合键仅用于前端状态、卡片选中和 `/workbench?focus=...` 路由定位，不修改后端模型、持久化目录或 `.aria` 中的已有 ID。

### 查找规则

- 抽屉查找必须精确匹配复合身份键。
- 不保留只按实体 ID 查找的回退逻辑。
- 无效或无法匹配的 `focus` 参数不打开抽屉，不猜测目标 Issue。
- 所有打开抽屉的入口，包括卡片点击、查看完整 Issue、生成下一阶段后聚焦新卡片，都必须生成同一格式的复合键。

### 数据兼容性

- `issue_0001` 现有 Story Spec、Design Spec、Work Item Group 和 Workspace Session 不迁移、不重命名。
- 生命周期接口已经返回每张卡片的 `issueId`，前端加载后可直接生成复合键。
- Workspace Session 查找继续按 `issueId + entityId + workspaceType` 执行，不改变后端契约。

## 影响范围

- `lifecycleCardKey`：加入 `issueId`，成为统一复合键生成入口。
- 抽屉 Zustand store：保存复合身份键，而不是单实体 ID。
- `findCardInColumns`：只按复合身份键精确查找。
- `IssueLifecycleWorkbench`：所有 `openDrawer`、选中、删除和生成后聚焦路径统一使用复合键。
- Router：`focus` 参数继续使用单个字符串字段，但内容改为复合身份键。

## 测试设计

采用表驱动回归测试，至少覆盖：

1. 两个 Issue 均存在 `story_spec_0001`，点击 issue2 卡片时抽屉展示 issue2 内容并打开 issue2 Workspace。
2. 两个 Issue 均存在 `design_spec_0001`，点击 issue2 卡片时抽屉展示 issue2 内容并打开 issue2 Workspace。
3. 两个 Issue 均存在 `issue_work_item_plan_0001`，点击 issue2 Work Item Group 时抽屉展示 issue2 子项并打开 issue2 Workspace。
4. 三种卡片的选中状态互不串联。
5. 无效的单实体 ID `focus` 不触发模糊匹配。
6. 现有单 Issue 抽屉、生成下一阶段、删除和 Workspace 跳转测试继续通过。

## 验证命令

```bash
cd web
pnpm test -- IssueLifecycleWorkbench
pnpm test
pnpm tsc -b
pnpm build
```

## 非目标

- 不修改 Story Spec、Design Spec、Work Item Group 的后端 ID 生成策略。
- 不迁移或重写 `.aria` 数据。
- 不改变 Workspace Session API。
- 不引入新的路由查询参数。
