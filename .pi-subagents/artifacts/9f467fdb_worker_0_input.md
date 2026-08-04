# Task for worker

美化图片创作页面的下拉框——**用自定义下拉组件替换原生 `<select>`**。原生 select 的下拉选项列表（option）由浏览器渲染，CSS 无法美化，所以需要自定义。

## 背景
当前 ParamsPanel/SessionList/SettingsDialog 用原生 `<select>`，闭合状态已美化（圆角/箭头），但**展开后的选项列表是浏览器原生的（丑）**。要彻底美化，必须自己实现下拉组件。

## 任务
新建一个轻量自定义下拉组件 `web/src/components/image-create/CustomSelect.tsx`，替换所有原生 `<select>`。

### CustomSelect 设计（Claymorphism 风格，参考 educational-platform）
- **触发器**：复用现有 select 的闭合样式（rounded-xl、柔和边框、右侧 ChevronDown 图标、hover/聚焦主色边框）
- **弹出列表**：
  - 浮在内容上方（absolute + z-index，最大高度可滚动 `max-h-60 overflow-y-auto`）
  - 圆角卡片（rounded-xl）+ 柔和双层阴影（`shadow-[0_4px_12px_rgba(0,0,0,0.08),0_12px_32px_rgba(0,0,0,0.12)]`）
  - 白底（`--aria-panel`），每项 padding 充足（px-3.5 py-2.5）
  - **选项样式**：默认 `--aria-ink` 文字；hover 时 `--aria-primary-soft` 背景 + 主色文字；选中项（当前值）带主色左边框或主色软背景 + 主色文字 + Check 图标
  - 选项之间用细分隔线（可选，或纯靠间距区分）
  - 字体 `text-sm font-medium`
- **交互**：
  - 点击触发器展开/收起
  - 点击选项：选中 + 收起 + 触发 onChange
  - 点击外部关闭（用 useEffect + mousedown 监听 document，或 ref 判断）
  - 键盘：Esc 关闭（基本即可，完整键盘导航可选）
  - `disabled` 状态：触发器置灰不可点
  - 动画：展开/收起用 `transition` + `opacity`/`scale`（motion-safe）
- **a11y**：`role="listbox"`、选项 `role="option"`、`aria-expanded`、`aria-selected`、`aria-label`；触发器 `role="combobox"` 或 button + label 关联

### Props 接口（和现有 ParameterSelect 对齐，便于替换）
```tsx
interface CustomSelectProps {
  label?: string;            // 可选，外部 label
  value: string;
  options: readonly string[];
  disabled?: boolean;
  onChange: (value: string) => void;
  "aria-label"?: string;
}
```

### 替换点
1. **ParamsPanel.tsx** 的 `ParameterSelect`：内部把 `<select>` 换成 `<CustomSelect>`（保留外层 label 结构）。注意 ParameterSelect 当前有 label + select，可让 CustomSelect 直接渲染 label。
2. **SessionList.tsx** 的模板/provider `<select>`（创建会话表单里的下拉）。
3. **SettingsDialog.tsx** 如有 `<select>` 也替换（看代码确认）。

### 关键约束
- **只改下拉 UI，不改逻辑**（value/options/onChange 接口不变）。
- 不加新依赖（不用 Headless UI / shadcn——纯 React + Tailwind + lucide-react）。
- 测试：现有 770 前端测试可能断言了原生 select 的行为（如 `getByRole('combobox')` 或 `getByLabelText`）。CustomSelect 要保持相同的 a11y 角色/label，让测试不破；若测试因结构变化失败，**改测试匹配新结构**（保持测试意图）。
- 用 lucide-react 的 `ChevronDown` 和 `Check` 图标。

## 门禁
- `cd web && pnpm tsc -b` 通过。
- `cd web && pnpm test` 全过。
- `cd web && pnpm build` 通过。
- commit：`style(image-create): 自定义下拉组件替换原生 select（美化选项列表）`

## 报告
写完整报告到 `/tmp/custom-select-report.md`。返回：状态/提交 hash/一行测试摘要/concerns。

## 🔴 纪律
- 批量执行；**先 commit 再写 report**。
- 重点是**下拉选项列表的视觉**（弹出后的样式），这是用户抱怨的点。
- 保持聚焦态/禁用态/hover 的精致度。
- 工作目录是 worktree 根。

## Acceptance Contract
Acceptance level: checked
Completion is not accepted from prose alone. End with a structured acceptance report.

Criteria:
- criterion-1: Implement the requested change without widening scope

Required evidence: changed-files, tests-added, commands-run, residual-risks, no-staged-files

Finish with a fenced JSON block tagged `acceptance-report` in this shape:
Use empty arrays when no items apply; array fields contain strings unless object entries are shown.
`criteriaSatisfied[].status` must be exactly one of: satisfied, not-satisfied, not-applicable.
`commandsRun[].result` must be exactly one of: passed, failed, not-run.
`manualNotes` and `notes` are optional strings; an empty string means no note and does not satisfy `manual-notes` evidence.
```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "specific proof"
    }
  ],
  "changedFiles": [
    "src/file.ts"
  ],
  "testsAddedOrUpdated": [
    "test/file.test.ts"
  ],
  "commandsRun": [
    {
      "command": "command",
      "result": "passed",
      "summary": "short result"
    }
  ],
  "validationOutput": [
    "validation output or concise summary"
  ],
  "residualRisks": [
    "none"
  ],
  "noStagedFiles": true,
  "diffSummary": "short description of the diff",
  "reviewFindings": [
    "blocker: file.ts:12 - issue found, or no blockers"
  ],
  "manualNotes": "anything else the parent should know"
}
```