# Task for worker

优化图片创作页面 `/image-create` 的**移动端响应式界面**。当前是桌面布局（左会话列表 + 右聊天 + 右参数面板），移动端（窄屏 <lg）全部纵向堆叠体验差。要做移动端友好的响应式适配。

## 当前布局（先读 `web/src/pages/ImageCreatePage.tsx`）
- 桌面（lg+）：`lg:grid-cols-[18rem_minmax(0,1fr)]` 左 SessionList + 右主区；主区 `xl:grid-cols-[minmax(0,1fr)_22rem]` ChatPane + ParamsPanel
- 移动端（<lg）：默认单列堆叠（SessionList → ChatPane → ParamsPanel 全部纵向），会话列表占满屏、参数面板在很下面，体验差

## 移动端适配方案（< lg 断点）

### 1. 会话列表改成抽屉/折叠（核心）
- 桌面（lg+）：保持左侧固定 18rem 列表
- 移动端（<lg）：会话列表默认**隐藏**，顶部加一个「会话」按钮（Menu 图标），点击**从左侧滑出抽屉**（或底部 sheet）显示 SessionList；选中会话后自动关闭抽屉
- 抽屉实现：用一个本地 state（`useState`）控制开关 + 固定定位遮罩（`fixed inset-0 z-50`）+ 滑入动画（`transition-transform`，motion-safe）
- 可用 lucide-react 的 `Menu` / `X` 图标

### 2. 参数面板移动端折叠/置底
- 桌面（xl+）：保持右侧 22rem 侧栏
- 中屏（lg~xl）：参数面板在聊天下方
- 移动端（<lg）：参数面板可做成**可折叠的手风琴**（默认收起，点「生成参数」展开），或固定在聊天下方但默认折叠；生成按钮始终可见
- 考虑：移动端聊天区为主（占主要高度），参数面板收起节省空间

### 3. 触摸友好
- 所有可点击元素（按钮、会话项、下拉选项、tab）最小高度 ≥ 44px（`min-h-11`）
- 间距在移动端更宽松（`gap-4`、`p-4`）
- 输入框/textarea 字体 ≥ 16px（防止 iOS Safari 自动缩放：`text-base` on mobile）
- 下拉展开列表在移动端友好（CustomSelect 已有 max-h 滚动）

### 4. Header 移动端
- 标题区在移动端紧凑（图标 + 标题，说明文字可隐藏或缩短 `hidden sm:block`）
- 按钮区（设置/返回）在移动端保持可见（图标按钮）

### 5. 聊天区移动端
- 聊天区在移动端占主要视口高度（`flex-1` 或 `min-h-[60vh]`）
- 消息气泡全宽（移动端不限制宽度）
- 生成图片在移动端全宽展示

## 关键约束
- **只改样式/布局/响应式 + 必要的小状态（抽屉开关 state）**，不改业务逻辑（store action/API/数据流）。
- 桌面端（lg+）体验**不能退化**（保持现有布局）。
- 用 Tailwind 响应式断点（`lg:`、`xl:`、`sm:`），移动优先（默认移动端样式，`lg:` 恢复桌面）。
- 不加新依赖。
- 测试：现有 773 测试不破；若桌面布局测试因 className 变化失败，改测试匹配（保持意图）。
- 测试移动端可用浏览器 DevTools 模拟（375px 宽），但不要加 Playwright E2E。

## 涉及文件
- `web/src/pages/ImageCreatePage.tsx`（主布局 + 抽屉 state）
- `web/src/components/image-create/SessionList.tsx`（移动端抽屉内样式）
- `web/src/components/image-create/ParamsPanel.tsx`（移动端折叠）
- `web/src/components/image-create/ChatPane.tsx`（移动端高度/全宽）
- 可能 `web/src/components/image-create/CustomSelect.tsx`（移动端适配）

## 门禁
- `cd web && pnpm tsc -b` 通过
- `cd web && pnpm test` 全过
- `cd web && pnpm build` 通过
- commit：`style(image-create): 移动端响应式适配（会话抽屉/参数折叠/触摸友好）`

## 报告
写完整报告到 `/tmp/mobile-responsive-report.md`。返回：状态/提交 hash/一行测试摘要/concerns。

## 🔴 纪律
- 批量执行；**先 commit 再写 report**。
- **桌面端不能退化**——改完在 lg+ 断点验证布局不变。
- 移动端优先用 Tailwind 默认（无断点）+ `lg:` 恢复桌面。
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