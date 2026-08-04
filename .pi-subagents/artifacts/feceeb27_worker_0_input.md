# Task for worker

视觉升级图片创作 Agent 的 `/image-create` 页面，参考 educational-platform 风格（Claymorphism）。这是纯 UI/样式改造，**不改任何业务逻辑、props、状态管理、API 调用**，只改 Tailwind className 和少量 JSX 结构（为视觉效果加 wrapper/icon）。

## 设计方向：融合 Claymorphism + Aria 专业感
ui-ux-pro-max 推荐 Claymorphism（柔和 3D、圆润、双层阴影、微交互），但完全照搬（indigo 配色 + Fredoka 字体 + 玩具感）会与 Aria 现有专业开发平台冲突。采用**融合**：

### 保留（Aria 现有）
- 现有 CSS 变量（`web/src/styles.css` 的 `--aria-*` 令牌）：`--aria-bg #f6f8fa`、`--aria-panel #ffffff`、`--aria-ink #17202a`、`--aria-primary #0891b2`（青蓝）、`--aria-primary-soft #e0f7fb`、`--aria-success/warning/danger` 等。
- **不换字体**（用系统现有字体，不引入 Fredoka——太玩具化）。
- 不改其他 Aria 页面（workbench 等），只改 `web/src/components/image-create/` 和 `web/src/pages/ImageCreatePage.tsx`。

### 引入（Claymorphism 元素，让创作页更精致有创意感）
1. **卡片**：更圆润（`rounded-2xl`）、柔和双层阴影（如 `shadow-[0_2px_8px_rgba(0,0,0,0.04),0_8px_24px_rgba(0,0,0,0.06)]`）、更柔和的边框（`border-[var(--aria-line)]` 但更轻）、卡片间留白更舒展。
2. **按钮**：主按钮更鲜明圆润（`rounded-xl`、主色渐变可选 `bg-gradient-to-b from-[var(--aria-primary)] to-[#0e7490]`、柔和阴影、hover 微下沉 `hover:translate-y-px hover:shadow-sm`、active 按压）；次级按钮柔和她圆润。
3. **输入/下拉/textarea**：圆润（`rounded-lg`）、柔和聚焦环（`focus-visible:ring-2 ring-[var(--aria-primary)] ring-offset-2`）、placeholder 更友好。
4. **空状态**：更友好，用 inline SVG 图标（Lucide 风格，**不要 emoji**）+ 引导文字。Aria 已用 lucide-react（见 ChatInputBar 的 `import { ... } from "lucide-react"`）——可用 `Sparkles`、`ImagePlus`、`Wand2`、`MessageSquare` 等图标。
5. **进度/忙碌态**：更精致的脉冲动画 + 友好的进度文案（已有计时器，让它视觉更柔和）。
6. **微交互**：hover 过渡（`transition-all duration-200`）、卡片 hover 轻微上浮（`hover:-translate-y-0.5 hover:shadow-md`）、会话项 hover 高亮。
7. **层级**：用柔和的背景层次（创作区用 `--aria-panel`，外层用 `--aria-bg`，参数区可用 `--aria-panel-muted`）区分区域。
8. **生成图片卡片**：更精致的图片展示框（柔和阴影、圆角、下载按钮更突出）。
9. **页面布局**（ImageCreatePage）：左侧会话列表 + 右侧主工作区，可加一个顶部标题区（"图片创作" + 简短说明 + 图标），让页面有 hero 感。

### 关键约束（来自 ui-ux-pro-max Pre-Delivery Checklist）
- **不用 emoji 做 icon**（用 Lucide SVG）。
- 所有可点击元素 `cursor-pointer`。
- hover 过渡 150-300ms。
- 文字对比度 ≥ 4.5:1（soft 背景上用 `--aria-ink`，不要用过淡的语义色文字——之前 Task 13 已修）。
- focus 状态可见。
- 尊重 `prefers-reduced-motion`（动画用 `motion-safe:` 或 `transition` 而非强制 keyframes）。

## 组件清单（6 个 + 页面）
- `web/src/pages/ImageCreatePage.tsx`：整体布局 + 顶部标题区
- `web/src/components/image-create/SessionList.tsx`：会话列表卡片 + 创建表单
- `web/src/components/image-create/ChatPane.tsx`：聊天区 + 消息气泡 + 输入栏 + 图片展示
- `web/src/components/image-create/PromptBlock.tsx`：建议 prompt 编辑区
- `web/src/components/image-create/ParamsPanel.tsx`：参数面板 + 生成按钮 + 进度条
- `web/src/components/image-create/ReferenceImageUpload.tsx`：上传区
- `web/src/components/image-create/SettingsDialog.tsx`：设置弹窗

## 工作方式
1. **先读** `web/src/styles.css`（CSS 变量）、`web/src/pages/ImageCreatePage.tsx`、6 个组件，理解现有结构和 className。
2. 参考 ui-ux-pro-max 输出（见上）+ 融合策略，逐个组件改 className 和必要的小 JSX 结构（加图标/wrapper）。
3. **绝不改**：组件的 props 接口、store action 调用、事件处理逻辑、条件渲染逻辑、测试断言的行为。
4. 用 lucide-react 图标（确认已安装：`grep lucide web/package.json`；若没装，用 inline SVG 而非加依赖）。

## 门禁
- `cd web && pnpm tsc -b` 通过。
- `cd web && pnpm test` 全过（现有 770 测试不红——只改样式不该破坏测试；若某测试因文案/结构变化失败，**改测试匹配新结构**，但保持测试意图）。
- `git commit -m "style(image-create): 融合 Claymorphism 视觉升级（柔和阴影/圆润/微交互/Lucide 图标）"`。

## 报告
写完整报告到 `/tmp/image-create-visual-upgrade-report.md`（改了哪些组件、设计决策、验证结果）。返回：状态/提交 hash/一行测试摘要/concerns。

## 🔴 纪律
- 批量执行；**先 commit 再写 report**。
- 纯样式改造，**不动逻辑**。如果发现某处逻辑有问题，记到报告 concerns 里，不要顺手改。
- 保持 Aria 专业调性，不要过度玩具化（这是给开发者/产品用的创作工具，不是儿童 app）。

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