# Workbench 四项 Bug 修复与 Monaco Editor 集成设计

## 概述

本方案解决 Workbench 中四个已知问题，核心改动是引入 Monaco Editor 作为统一的代码/文档展示引擎，同时修复人工确认流程的状态管理缺陷。

## 技术选型

**Monaco Editor**（`@monaco-editor/react` + `monaco-editor`）

选型理由：
- 内置 DiffEditor，支持 inline / side-by-side 切换
- Markdown 语法高亮开箱即用
- 只读/编辑模式一个 option 切换
- VS Code 同款引擎，用户体验熟悉
- 虚拟化渲染，长文档无性能问题

加载策略：
- 使用 `@monaco-editor/react` 内置 loader 从 jsDelivr CDN 加载核心
- 组件级 lazy loading（`React.lazy`）
- 加载中显示骨架屏占位

## 新增共享组件

### `web/src/components/shared/MonacoDiffViewer.tsx`

```typescript
interface MonacoDiffViewerProps {
  original: string;
  modified: string;
  language?: string;   // 默认 "markdown"
  height?: string;     // 默认 "400px"
  sideBySide?: boolean; // 默认 true
}
```

- 内部使用 `DiffEditor` from `@monaco-editor/react`
- 只读模式，禁用所有编辑功能
- 主题跟随系统（light/dark）

### `web/src/components/shared/MonacoViewer.tsx`

```typescript
interface MonacoViewerProps {
  value: string;
  language?: string;   // 默认 "markdown"
  height?: string;     // 默认 "300px"
}
```

- 内部使用 `Editor` from `@monaco-editor/react`
- 只读模式，minimap 关闭，行号显示
- 自动换行开启

## 问题修复设计

### 问题 1：Artifact Diff 功能

**文件**：`web/src/components/chat-workspace/ArtifactPane.tsx`

**改动**：
1. 删除 `lineDiff()` 函数
2. 删除 `MarkdownPreview` 组件
3. Diff 展示区替换为 `MonacoDiffViewer`：
   - `original` = `previous.markdown`
   - `modified` = `selected.markdown`
   - `language` = `"markdown"`
4. 非 Diff 模式下，内容展示替换为 `MonacoViewer`：
   - `value` = `markdown`（当前选中版本内容）
   - `language` = `"markdown"`

### 问题 2：人工确认逻辑

**涉及文件**：
- `web/src/state/chat-entries.ts`
- `web/src/components/chat-workspace/entries/GatePromptEntry.tsx`
- `web/src/components/chat-workspace/ChatEntryRenderer.tsx`
- `web/src/state/workspace-ws-store.ts`（WebSocket 消息处理）

**数据模型变更**：

`ChatEntry` 增加字段：
```typescript
resolved?: boolean;
resolution?: "confirm" | "request-change" | "terminate";
```

**GatePromptEntry 渲染逻辑**：
- `resolved = false` 或 `undefined`：显示确认/终止按钮（当前行为）
- `resolved = true`：按钮区域替换为状态标签
  - `resolution = "confirm"` → 绿色标签"已确认"
  - `resolution = "request-change"` → 橙色标签"已要求修改"
  - `resolution = "terminate"` → 红色标签"已终止"

**状态流转**：
1. 用户通过输入框发送修改意见 → 前端发送 `human_confirm { decision: "request-change", feedback: "..." }`
2. 前端 store 立即将当前 GatePromptEntry 标记为 `resolved = true, resolution = "request-change"`
3. 后端收到 request-change → 进入修改轮次 → 修改完成后推送新的 `gate_prompt` 事件
4. 前端收到新 `gate_prompt` → 追加新的 GatePromptEntry（`resolved = false`）

### 问题 3：Issue 抽屉不展示具体信息

**文件**：`web/src/components/lifecycle/LifecycleCardDrawer.tsx`

**改动**：

当 `entity.kind === "issue"` 时，在内容区展示：

1. **Issue 描述**：用 `MonacoViewer` 以 markdown 只读模式展示 `entity.description`（需要扩展 `DrawerEntity` 类型增加 `description?: string` 字段）
2. **关联产物列表**：展示 `entity.artifacts`（ProductIssueArtifact[]），每个产物显示 kind、stage、summary
3. **元信息**：创建时间、phase、status

**DrawerEntity 类型扩展**：
```typescript
export interface DrawerEntity {
  id: string;
  kind: DrawerEntityKind;
  title: string;
  status: string;
  version: number | null;
  artifactVersions?: ArtifactVersion[];
  // 新增
  description?: string;
  artifacts?: ProductIssueArtifact[];
  phase?: string;
  createdAt?: string;
}
```

**数据来源**：`ProductIssue` 已有 `description` 和 `artifacts` 字段，在 `IssueLifecycleWorkbench.tsx` 构建 DrawerEntity 时传入即可。

### 问题 4：Story Spec 抽屉展示

**文件**：`web/src/components/lifecycle/LifecycleCardDrawer.tsx`

**4.1 版本历史可切换**：
- 增加 `selectedVersionIndex` state，默认选中最新版本（index 0）
- 版本列表项增加 `onClick` + 选中高亮样式（左边框 primary 色）
- 下方预览区展示选中版本的 markdown 内容

**4.2 使用 Monaco 展示**：
- 将 `<pre>{previewMarkdown(...)}</pre>` 替换为 `MonacoViewer`
- 移除 `previewMarkdown()` 的 400 字符截断
- `value` = 选中版本的 `markdown`
- `language` = `"markdown"`

**4.3 可选：版本对比 Diff**：
- 当选中非最新版本时，显示"与最新版本对比"按钮
- 点击后切换为 `MonacoDiffViewer`（original = 选中版本，modified = 最新版本）

## 文件变更清单

| 文件 | 变更类型 | 说明 |
|------|---------|------|
| `web/package.json` | 新增依赖 | `@monaco-editor/react`, `monaco-editor` |
| `web/src/components/shared/MonacoDiffViewer.tsx` | 新建 | Diff 展示封装组件 |
| `web/src/components/shared/MonacoViewer.tsx` | 新建 | 只读查看封装组件 |
| `web/src/components/chat-workspace/ArtifactPane.tsx` | 重构 | 替换 lineDiff + MarkdownPreview 为 Monaco |
| `web/src/state/chat-entries.ts` | 修改 | ChatEntry 增加 resolved/resolution 字段 |
| `web/src/components/chat-workspace/entries/GatePromptEntry.tsx` | 修改 | 增加已处理状态渲染 |
| `web/src/components/chat-workspace/ChatEntryRenderer.tsx` | 修改 | 传递 resolved 状态 |
| `web/src/state/workspace-ws-store.ts` | 修改 | 处理 human_confirm 后标记 entry resolved |
| `web/src/components/lifecycle/LifecycleCardDrawer.tsx` | 重构 | issue 详情展示 + 版本切换 + Monaco 展示 |
| `web/src/state/lifecycle-workbench-store.ts` | 修改 | DrawerEntity 类型扩展 |
| `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx` | 修改 | 构建 DrawerEntity 时传入 description/artifacts |

## 测试策略

- 单元测试：MonacoDiffViewer / MonacoViewer 组件渲染测试
- 单元测试：GatePromptEntry resolved 状态渲染测试
- E2E 测试：使用测试场景（爬楼梯问题）验证完整流程
  - 创建 issue → 进入 workspace → 触发人工确认 → request-change → 再次确认
  - 验证 Artifact diff 展示正确
  - 验证 issue 抽屉展示描述信息
  - 验证 story_spec 抽屉版本切换和 markdown 展示
