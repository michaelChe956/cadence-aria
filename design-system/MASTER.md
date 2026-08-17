# Aria Design System Master File（设计规范 · 唯一准绳）

> **来源：** ui-ux-pro-max 生成后按 openspec/changes/spec-workbench-canvas-experience/design.md §3 token 映射表定制。
> 本文件是 Task 2-4 组件类的视觉契约。冲突时以本文件 + 契约 §3 为准。

---

**Project:** Aria（Cadence Aria · Spec 工作台 Canvas 审核体验）
**Category:** Developer Tool / SaaS 工作台（浅色 Canvas 风格，Claymorphism 质感）

---

## 1. 配色（Color Palette）

| 角色 | 值 | CSS 变量 | 用途 |
|------|-----|----------|------|
| Primary 主色 | `#4F46E5` | `--aria-primary` | 品牌蓝紫：链接、激活态、主交互色 |
| Primary 浅底 | `#E0E7FF` | `--aria-primary-soft` | 主色浅底（选中行、高亮块） |
| CTA 强调 | `#F97316` | `--aria-cta` | 橙：激活强调（激活 chip、版本标识、review 警示、selection）、警示点缀；不作主按钮色 |
| CTA 浅底 | `#FFEDD5` | `--aria-cta-soft` | CTA 浅底 |
| 页面底色 | `#f5f3ee` | `--aria-bg` | 微暖奶油底；面板保持白色卡片 |
| 面板 | `#ffffff` | `--aria-panel` | 白色卡片/面板 |
| 正文墨色 | `#17202a` | `--aria-ink` | 正文与深色边框（2-3px 边框用 var(--aria-ink)） |
| 弱化墨色 | `#52616f` | `--aria-ink-muted` | 次要文字 |
| 细分割线 | `#dde5ec` | `--aria-line` | 1px 细分割线（保留） |
| 深色边框 | `#3f3f46` | `--aria-border-strong` | 2-3px 卡片/面板/chip 深色边框 |
| Success | `#059669` / `#dcfce7` | `--aria-success(-soft)` | 不变 |
| Warning | `#d97706` / `#fef3c7` | `--aria-warning(-soft)` | 不变 |
| Danger | `#dc2626` / `#fee2e2` | `--aria-danger(-soft)` | 不变 |

## 2. 字体（Typography）

- **字体族：** Inter / DM Sans 无衬线（现状 Inter，保持）；**禁用衬线字体做正文**。
- 正文 14-16px，行高 1.5+；标题 600 字重；代码块等宽字体。
- 对比度 ≥ 4.5:1。

## 3. 边框与圆角（Borders & Radius）

- **卡片/面板/chip：2-3px 深色实线边框**（`var(--aria-border-strong)` 或 `var(--aria-ink)`），告别 1px 浅灰细框。
- 细分割线仍用 1px `var(--aria-line)`。
- **圆角：** 卡片/面板 `rounded-xl`（16px）至 `rounded-2xl`（24px）；输入框 `rounded-xl`。

## 4. 阴影（Shadows）

- **柔和单层阴影**（如 `0 2px 8px rgba(23,32,42,0.08)`）；**禁用双层硬阴影**、禁用大扩散深投影。
- 阴影与深色边框叠加营造 Claymorphism 质感，但保持轻。

## 5. 按钮（Buttons）

- **主按钮（送审/定稿/采纳等 CTA）：** 背景 `var(--aria-primary)`（#4F46E5 蓝紫）+ 白字 + 2px 深边框 + `rounded-xl`，600 字重。橙 `var(--aria-cta)` 仅用于激活强调/警示点缀，不作主按钮色。
- **次按钮：** 白底 + 2px `var(--aria-border-strong)` 边框 + `rounded-xl`。
- **危险按钮：** `var(--aria-danger)` 系。
- hover 仅做颜色/阴影变化，transition 150-300ms；**禁 hover scale/位移**。
- 所有可点击元素 `cursor: pointer`；键盘焦点可见。

## 6. Chip（胶囊标签）

- **胶囊形：** `rounded-full` + **2px 深色边框** + 浅底（primary-soft / cta-soft / 语义 soft 色）。
- 状态标签（stage/审核状态）一律用胶囊 chip，不用方角 badge。
- 尺寸紧凑：padding 2px 10px，字号 12px。

## 7. 反模式（Anti-Patterns · 禁止）

- ❌ emoji 作装饰图标 —— 一律用 SVG 图标（Heroicons / Lucide）
- ❌ hover 时 scale / 位移变换（布局抖动）
- ❌ 双层硬阴影、大扩散投影
- ❌ 衬线字体正文
- ❌ 1px 浅灰边框的"素卡片"（新组件一律 2-3px 深边框）
- ❌ 无 transition 的瞬时状态切换 —— transition 150-300ms
- ❌ 对比度 < 4.5:1 的浅色文字
- ❌ 不可见的键盘焦点态
- ❌ 忽略 `prefers-reduced-motion`

## 交付前检查清单

- [ ] 无 emoji 装饰图标（用 SVG）
- [ ] 可点击元素 cursor-pointer、焦点可见
- [ ] hover 有 150-300ms transition，无 scale/位移
- [ ] 卡片/面板/chip 使用 2-3px 深色边框 + rounded-xl/2xl/胶囊
- [ ] 阴影为柔和单层
- [ ] 浅色模式对比度 ≥ 4.5:1
- [ ] `prefers-reduced-motion` 已尊重
- [ ] 响应式：375px / 768px / 1024px / 1440px
