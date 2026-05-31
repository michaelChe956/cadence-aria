# CodingWorkspace 二期 P5：前端 UX 对齐（展示层）

## 文档信息

- 文档类型：计划文档
- 分支：`product-workbench-issue-lifecycle`
- 制定日期：2026-05-28
- 版本：v1.0
- 前置：P1（CodingChatEntry 模型）、P2（StageGate 组件）
- 产出：ChatEntryList 复用 + MessageGroupView 扩展 + Timeline + 角色颜色
- 设计文档：`cadence/designs/2026-05-28_技术方案_CodingWorkspace二期改造_v1.0.md` §5

---

## 一、目标

将 CodingWorkspace 的前端展示对齐 ChatWorkspace / SpecWorkspace 的 UX：

1. 复用 ChatEntryList / MessageGroupView / InlineEventRow 组件
2. 扩展消息分组规则支持 7 种角色
3. 新增 Timeline 左侧栏
4. 新增 AnalystVerdictEntry 组件
5. 角色颜色映射

---

## 二、任务清单

### 2.1 ChatEntryList 复用集成

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 1.1 | CodingWorkspacePage 替换现有事件列表为 ChatEntryList 组件 | — | 消息以气泡形式展示 |
| 1.2 | 适配 CodingChatEntry → ChatEntryList 所需的 props 格式 | — | 数据正确映射 |
| 1.3 | coding-workspace-store 的 chatEntries 数组驱动 ChatEntryList | — | 实时更新 |

### 2.2 MessageGroupView 角色扩展

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 2.1 | 扩展 `message-grouping.ts` 的 role 类型：新增 coder / tester / analyst / code_reviewer / internal_reviewer | — | 类型定义正确 |
| 2.2 | 分组规则：按 `node_id` 分组（`{stage}_{round}_{sequence}`） | — | 同一 node_id 的消息归为一组 |
| 2.3 | tool_call / tool_result 嵌套在父 assistant 气泡内（InlineEventRow） | — | 嵌套展示正确 |
| 2.4 | StageGate / AnalystVerdict 作为独立条目，不参与分组 | — | 独立展示 |

### 2.3 角色颜色映射

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 3.1 | 定义角色颜色常量：Coder(blue-600) / Tester(purple-600) / Analyst(amber-600) / CodeReviewer(green-600) / InternalReviewer(indigo-600) / User(gray-600) / System(red-500) | — | 颜色定义正确 |
| 3.2 | MessageGroupView 根据 role 应用对应颜色到气泡边框/头像 | — | 视觉区分明显 |

### 2.4 AnalystVerdictEntry 组件

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 4.1 | 新建 `AnalystVerdictEntry.tsx`：根据 verdict 类型展示不同样式 | — | 三种样式正确 |
| 4.2 | NeedsFix：橙色卡片 + 修复建议列表 | — | 列表正确展示 |
| 4.3 | NeedsHumanInput：蓝色卡片 + 问题列表 + 输入提示 | — | 引导用户输入 |
| 4.4 | NoIssue：绿色卡片 + 通过 summary | — | 简洁展示 |

### 2.5 Timeline 左侧栏

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 5.1 | 新建 `CodingTimeline.tsx`：展示阶段节点列表 | — | 所有阶段可见 |
| 5.2 | 当前阶段高亮 + 已完成阶段 checkmark | — | 状态正确反映 |
| 5.3 | 点击节点滚动到对应消息区域 | — | 滚动定位准确 |
| 5.4 | CodingWorkspacePage 布局调整：左侧 Timeline + 右侧消息区 | — | 布局合理 |

---

## 三、验收标准

1. `pnpm build` 通过
2. 手动测试：CodingWorkspace 展示消息气泡（非纯文本列表）
3. 手动测试：不同阶段的消息有不同颜色标识
4. 手动测试：tool_call 嵌套在 assistant 气泡内展示
5. 手动测试：左侧 Timeline 正确反映当前阶段，点击可跳转
6. 手动测试：AnalystVerdict 三种类型分别展示正确样式

---

## 四、不做的事

- 后端逻辑变更（纯前端展示层）
- CodeReview / InternalPrReview 的结果展示（P6）
- E2E 测试（P7）
