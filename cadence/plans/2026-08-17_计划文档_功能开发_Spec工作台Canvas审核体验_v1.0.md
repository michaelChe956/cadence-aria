# Spec 工作台 Canvas 审核体验 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Change:** `openspec/changes/spec-workbench-canvas-experience/`（契约已经 reviewer 审查修订，3 Requirement / 13 Scenario）

**Goal:** Story/Design workspace 的 AuthorConfirm 升级为 Canvas 产物审核面板（阶段驱动滑出/收起），三动作视觉分层，视觉迁移 Claymorphism 蓝紫橙规范（token 层全站、组件形态仅 spec 工作台）。

**Architecture:** 纯前端。`ChatWorkspacePage` 的 `activePanel` 互斥 Tab 改造为 author_confirm 时 chat 与右侧面板并存；面板复用 ArtifactPane 渲染能力，内嵌吸顶操作条与改动摘要折叠条；`--aria-*` 变量改值（保留变量名 + 新增 --aria-cta），组件形态经标准类收敛。零后端改动、零行为契约改动。

**Tech Stack:** React + zustand + Tailwind + vitest；ui-ux-pro-max（本地 skill，生成 design-system/MASTER.md）。

## Global Constraints

- 行为零改动：spec-design-dialog-revision 的决策语义/WS 协议/store 数据流不改；既有测试的行为断言保留，仅结构/类名断言随迁移更新。
- 按钮文案保留现状：「确认送审」「确认定稿」「发送反馈」「采纳 Review 意见」，不随意改文案。
- 克制玩具元素：无 emoji 装饰图标、无 hover scale 位移、无双层硬阴影、无衬线正文；transition 150-300ms。
- token 分层：styles.css 变量改值全站生效（其他工作台仅色系联动）；粗边框卡片/胶囊 chip/按钮体系形态类仅应用于 spec 工作台组件。
- 验证：`cd web && pnpm tsc -b && pnpm test` 全绿；`cargo check --locked` 保持绿（无后端改动，回归门禁）。
- 🔴 不触碰 `run/provider_run.rs` 及 add-monorepo 已改区域（本 change 纯前端，天然无交集）。

## 任务与契约映射

| Plan Task | 契约 tasks.md | 覆盖 Scenario |
|---|---|---|
| Task 1 | 1.1/1.2 | token 映射落地、规范持久化 |
| Task 2 | 2.1/2.2 | token 映射落地（组件形态）、克制玩具元素 |
| Task 3 | 3.1/3.1b | 产出完成自动滑出、本轮改动摘要、无摘要隐藏、吸顶操作条 |
| Task 4 | 3.2/3.3 | 反馈时收起、重连滑出、送审运行收起、采纳预填自动收起、终局确认对/发送反馈分层 |
| Task 5 | 4.1/4.2 | 全部场景的测试背书 + 冒烟 + 收尾 |

---

### Task 1: 设计规范与 token 落地

**Files:**
- Create: `design-system/MASTER.md`
- Modify: `web/src/styles.css:5-32`（:root 变量）
- Modify: `web/tailwind.config.ts`（如需扩展 borderWidth 3px / 色板映射）

**Interfaces:**
- Produces: MASTER.md 七章规范（配色/字体/边框圆角/阴影/按钮/chip/反模式）；CSS 变量 `--aria-primary: #4F46E5`、`--aria-cta: #F97316`、`--aria-cta-soft`、`--aria-bg: #f5f3ee`、其余变量值按 design §3 调整。Task 2-4 的组件类消费这些变量。

- [ ] **Step 1: 生成 MASTER.md 草稿**

```bash
cd /home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0814-repair
python3 .agents/skills/ui-ux-pro-max/scripts/search.py "developer tool SaaS professional dashboard" --design-system --persist -p "Aria" -f markdown
```

生成后按契约 design §3 的 token 映射表**定制改写** `design-system/MASTER.md`（skill 输出是通用推荐，必须以契约为准：主色 #4F46E5、CTA #F97316、底色 #f5f3ee、2-3px 深色边框、rounded-xl/2xl、胶囊 chip、反模式清单），确保七章齐全。

- [ ] **Step 2: 写失败的 token 断言测试**

新建 `web/src/styles.tokens.test.ts`（Node 环境直接读文件断言，跟随项目是否有此类测试惯例；若无则用组件冒烟代替——在 ChatInputBar.test 渲染后 getComputedStyle 断言主色）：

```ts
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("aria tokens follow canvas-experience palette", () => {
  const css = readFileSync("src/styles.css", "utf8");
  expect(css).toContain("--aria-primary: #4F46E5");
  expect(css).toContain("--aria-cta: #F97316");
  expect(css).toContain("--aria-bg: #f5f3ee");
});
```

- [ ] **Step 3: 运行验证失败**

Run: `cd web && pnpm vitest run src/styles.tokens.test.ts`
Expected: FAIL（变量未更新）

- [ ] **Step 4: 更新 styles.css 变量**

`--aria-primary: #0891b2` → `#4F46E5`；`--aria-primary-soft` → 蓝紫浅底 `#E0E7FF`；新增 `--aria-cta: #F97316`、`--aria-cta-soft: #FFEDD5`；`--aria-bg: #f6f8fa` → `#f5f3ee`；`--aria-line` → `#3f3f46`（深色边框用）或新增 `--aria-border-strong: #3f3f46` 供 2-3px 边框引用（保留 --aria-line 用于细分割线）；其余 success/warning/danger 不变。tailwind.config 确认 `border-3` 可用（Tailwind 默认有 border-2/4/8，无 3——需扩展 `borderWidth: { 3: "3px" }`）。

- [ ] **Step 5: 运行验证通过 + 提交**

Run: `cd web && pnpm vitest run src/styles.tokens.test.ts && pnpm tsc -b`
Expected: PASS

```bash
git add design-system/MASTER.md web/src/styles.css web/tailwind.config.ts web/src/styles.tokens.test.ts
git commit -m "feat(web): 设计规范 MASTER.md 与 aria token 蓝紫橙迁移（spec-workbench-canvas-experience T1）"
```

---

### Task 2: 基础组件形态收敛

**Files:**
- Modify: `web/src/components/chat-workspace/ChatInputBar.tsx`（按钮类统一）
- Modify: `web/src/components/chat-workspace/TimelineNodeList.tsx`（节点激活态/chip）
- Modify: `web/src/components/chat-workspace/ArtifactPane.tsx`（卡片形态）
- Test: `web/src/components/chat-workspace/ChatInputBar.test.tsx`（类断言更新）

**Interfaces:**
- Consumes: Task 1 的变量与 tailwind border-3 扩展。
- Produces: 标准类约定——`aria-card-clay`（2-3px `var(--aria-border-strong)` 边框 + rounded-xl + 白底 + 柔和单层阴影）、`aria-chip`（rounded-full + 2px 边框 + px-3 py-1 text-xs font-bold）、`btn-primary`（bg-[var(--aria-primary)] 白字 + 2px 边框）、`btn-secondary`（白底 + 2-3px 边框 + 主色文字）。写在 styles.css @layer components。

- [ ] **Step 1: 定义标准类（styles.css @layer components）**

```css
@layer components {
  .aria-card-clay {
    @apply rounded-xl border-3 bg-white shadow-sm;
    border-color: var(--aria-border-strong);
  }
  .aria-chip {
    @apply inline-flex items-center gap-1 rounded-full border-2 px-3 py-1 text-xs font-bold;
    border-color: var(--aria-border-strong);
  }
  .btn-primary {
    @apply inline-flex items-center gap-2 rounded-xl border-2 px-4 py-2 text-sm font-semibold text-white transition-colors duration-200;
    background: var(--aria-primary);
    border-color: var(--aria-border-strong);
  }
  .btn-secondary {
    @apply inline-flex items-center gap-2 rounded-xl border-2 bg-white px-4 py-2 text-sm font-semibold transition-colors duration-200;
    color: var(--aria-primary);
    border-color: var(--aria-border-strong);
  }
}
```

- [ ] **Step 2: 迁移 ChatInputBar 按钮与关键卡片**

ChatInputBar 的「发送反馈」「确认送审」「确认定稿」「采纳 Review 意见」「开始生成」按钮统一换用 btn-primary/btn-secondary（默认高亮逻辑不变，仅换类）；ArtifactPane 容器换 aria-card-clay；版本号/阶段标签换 aria-chip；TimelineNodeList 激活节点换主色 + 粗边框。**不改任何 onClick/逻辑/文案**。

- [ ] **Step 3: 更新受影响测试断言 + 全绿**

Run: `cd web && pnpm vitest run src/components/chat-workspace && pnpm tsc -b`
Expected: PASS（类断言更新后）

- [ ] **Step 4: 提交**

```bash
git add web/src/styles.css web/src/components/chat-workspace/
git commit -m "feat(web): 基础组件 Claymorphism 形态收敛（spec-workbench-canvas-experience T2）"
```

---

### Task 3: Canvas 面板组件与互斥 Tab 改造

**Files:**
- Create: `web/src/components/chat-workspace/ArtifactReviewPanel.tsx`
- Modify: `web/src/pages/ChatWorkspacePage.tsx:576-700`（main 布局：author_confirm 时 chat 与面板并存）
- Test: `web/src/components/chat-workspace/ArtifactReviewPanel.test.tsx`（新建）

**Interfaces:**
- Consumes: Task 2 的标准类；ArtifactPane 的渲染能力（ArtifactReviewPanel 内部复用 ArtifactPane 或提取其 markdown 渲染段——选择复用整件 ArtifactPane 最简单，传 className 撑满）。
- Produces: `ArtifactReviewPanel({ artifactVersions, artifact, sessionId, artifactContentCache, loadArtifactVersion, onCacheArtifactContent, changelogSummary?, onClose, actions: ReactNode })`；`changelogSummary` 从最近 completed revision 节点 summary 取（无则整条不渲染）。

- [ ] **Step 1: 写失败的面板测试**

```tsx
it("渲染产物全文与吸顶操作条，改动摘要条默认展开", () => {
  render(<ArtifactReviewPanel {...props} changelogSummary="新增 REQ-006" actions={<button>定稿</button>} />);
  expect(screen.getByText("新增 REQ-006")).toBeVisible();
  expect(screen.getByTestId("artifact-review-actions").className).toContain("sticky");
  expect(screen.getByRole("button", { name: "定稿" })).toBeInTheDocument();
});

it("changelogSummary 为空时整条不渲染", () => {
  render(<ArtifactReviewPanel {...props} changelogSummary={undefined} actions={null} />);
  expect(screen.queryByText("本轮改动")).not.toBeInTheDocument();
});
```

- [ ] **Step 2: 运行验证失败**

Run: `cd web && pnpm vitest run src/components/chat-workspace/ArtifactReviewPanel.test.tsx`
Expected: FAIL（组件不存在）

- [ ] **Step 3: 实现面板组件**

工具条（标题 + 版本切换复用 ArtifactPane 内建 + 收起钮 onClose）→ 改动摘要折叠条（`changelogSummary` 非空时渲染，details/summary 默认 open）→ ArtifactPane 渲染区（flex-1 min-h-0）→ 吸顶操作条（`sticky top-0 z-10` 或底部 sticky，actions 插槽）。CSS 滑出动画：面板容器 `transition-transform duration-300`，挂载态 translate-x-0 / 卸载态 translate-x-full（或 width 0 过渡，按 grid 列宽动画）。

- [ ] **Step 4: ChatWorkspacePage 并存改造**

`activePanel` Tab 语义调整：author_confirm 且 workspaceType ∈ {story, design} 时，section 内并排渲染 `ChatEntryList + ChatInputBar`（收窄，min-w-[320px]）与 `ArtifactReviewPanel`（右侧，grid 第三列或 absolute overlay）；非 author_confirm 保持现有 Tab 互斥行为；<1440px 时面板 absolute 覆盖对话区（right-0 top-0 bottom-0 w-[65%]）。改动摘要取数：`timelineNodes.filter(n => n.node_type === "revision" && n.status === "completed").at(-1)?.summary`。

- [ ] **Step 5: 运行验证通过 + 提交**

Run: `cd web && pnpm vitest run src/components/chat-workspace src/pages/ChatWorkspacePage.test.tsx && pnpm tsc -b`
Expected: PASS

```bash
git add web/src/components/chat-workspace/ArtifactReviewPanel.tsx web/src/pages/ChatWorkspacePage.tsx
git commit -m "feat(web): Canvas 产物审核面板与并存布局（spec-workbench-canvas-experience T3）"
```

---

### Task 4: 阶段驱动接线与三动作迁移

**Files:**
- Modify: `web/src/pages/ChatWorkspacePage.tsx`（面板开合逻辑 + 动作接线）
- Modify: `web/src/components/chat-workspace/ChatInputBar.tsx`（移除 author_confirm 三按钮区，保留发送反馈）
- Test: `web/src/pages/ChatWorkspacePage.actions.test.tsx`、`ChatInputBar.test.tsx`

**Interfaces:**
- Consumes: Task 3 的 ArtifactReviewPanel（actions 插槽放终局确认对+采纳按钮）。
- Produces: 面板开合状态机（派生自 stage + 用户输入焦点，无存store字段）；`onAdoptReview` 回调：预填 + 收起面板。

- [ ] **Step 1: 写失败的接线测试**

ChatWorkspacePage 测试追加（跟随现有 actions.test 模式，mock store stage）：

```tsx
it("author_confirm 阶段自动展示产物面板", () => { /* stage=author_confirm → 面板在文档中 */ });
it("stage 离开 author_confirm 面板收起", () => { /* stage=running → 面板不在 */ });
it("点击采纳 Review 意见预填并收起面板", () => { /* 点击后输入框值含引导语、面板消失 */ });
it("输入聚焦时面板收起", () => { /* focus 输入框 → 面板消失 */ });
```

- [ ] **Step 2: 运行验证失败**

Run: `cd web && pnpm vitest run src/pages/ChatWorkspacePage`
Expected: FAIL

- [ ] **Step 3: 实现开合逻辑与动作迁移**

面板可见性派生：`stage === "author_confirm" && workspaceType ∈ {story, design} && !userDismissed`（userDismissed 为组件本地 state：输入聚焦/点返回对话置 true，stage 进入 author_confirm 时重置 false）。ChatInputBar 的 author_confirm 分支移除「确认送审」「确认定稿」「采纳 Review 意见」按钮（保留输入框+发送反馈），这些按钮移入面板 actions：`确认送审 → sendAuthorDecision("accept_with_review")`、`确认定稿 → sendAuthorDecision("accept_finalize")`、采纳按钮 onClick 改为 `预填(input) + setUserDismissed(true)`。reviewerEnabled 决定主次样式（沿用现有高亮逻辑）。

- [ ] **Step 4: 运行验证通过 + 提交**

Run: `cd web && pnpm tsc -b && pnpm test`
Expected: 全绿

```bash
git add web/src/pages/ChatWorkspacePage.tsx web/src/components/chat-workspace/ChatInputBar.tsx web/src/pages/ChatWorkspacePage.actions.test.tsx web/src/components/chat-workspace/ChatInputBar.test.tsx
git commit -m "feat(web): 面板阶段驱动与三动作迁移（spec-workbench-canvas-experience T4）"
```

---

### Task 5: 测试与收尾

**Files:**
- Modify: 受布局影响的既有测试（`rg -l "activePanel|确认送审|确认定稿" web/src --glob '*.test.*'` 定位）
- Modify: `web/whats-new`（按惯例追加条目）

- [ ] **Step 1: 迁移既有测试**

逐文件更新结构断言（按钮位置迁移、面板存在性），保留行为断言（决策发送的 payload 不变）。

- [ ] **Step 2: 全量验证 + 冒烟**

```bash
cd web && pnpm tsc -b && pnpm test
cargo check --locked
```

冒烟（人工或简单渲染测试）：coding-workspace 与 image-create 页面渲染不炸（token 改值后）、文本对比度不劣化；样式人工审查：无 emoji 装饰/无 hover 放大/无双层阴影/无衬线正文。

- [ ] **Step 3: whats-new 与提交**

```bash
git add -A
git commit -m "test(web): 迁移布局相关断言并全量验证（spec-workbench-canvas-experience T5）"
```

---

## Self-Review 记录

1. **Spec 覆盖**：13 Scenario 逐一映射——滑出（T3/T4）、反馈收起（T4）、吸顶（T3）、改动摘要/无摘要（T3）、重连滑出（T4 接线天然覆盖+测试）、送审运行收起（T4）、采纳预填收起（T4）、终局确认对/发送反馈分层（T4）、token 落地/克制/持久化（T1/T2/T5）。
2. **占位符扫描**：无 TBD；每步含真实代码或精确命令。
3. **类型一致性**：ArtifactReviewPanel props（T3 定义 ↔ T4 actions 消费）；changelogSummary 取数（T3 接口 ↔ T4 实现）；userDismissed 派生命名统一。
4. **风险备注**：activePanel Tab 改造涉及 ChatWorkspacePage 大文件（~700 行），T3/T4 同文件两步顺序执行避免冲突；既有测试迁移量集中在 T5。
