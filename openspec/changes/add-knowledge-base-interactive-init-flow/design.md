# 设计：知识库初始化交互式两段流程

## 1. 现状与缺口（真机事实）

2026-11-11 在 `~/aria-kb-test-workspace`（LC 聚合根 `aggregate-root`）实跑：

| 环节 | 事实 |
|---|---|
| 起草 | `POST /knowledge-base/initializations {kind:bootstrap}` ⇒ 后端分叉判定「输入未准备」⇒ 建 `user_input` operation（Pi），`prepare_user_input` 步 completed |
| 产物 | `aggregate-root/cadence/knowledge-base/user-input/` 下 9 份文件全部生成（`api-scope.md`、`base-info.md`、`configuration-scope.md`、`data-model-scope.md`、`database-ddl.sql`、`middleware-scope.md`、`page-scope.md`、`product.md`、`project-scope.md`） |
| 确认 | `GET /knowledge-base/user-input` ⇒ 9 份 `filled=true`、`pending=true`、`confirmed=false`，`confirmed_count=0/9` |
| 界面 | 卡片停在「completed / prepare_user_input / 本次使用 Pi」；详情页输入确认区只有一行计数；**无下一步入口** |

缺口 = 起草完成之后的**交互链**：打开文件、引导、确认即建库、建库后落回详情页。

## 2. 决策

### D1：起草完成 ⇒ 自动跳转 + 卡片兜底入口（两者都做）

- 卡片在「起草完成」态给出醒目的【打开输入文件并补充】按钮（永远保留，用户离开过页面也能回来）。
- 起草刚完成时**自动**进入知识库详情页（用户明确要求「初始化完成后需要打开初始化出来的 user-input 文件」）。

### D2：自动跳转的触发条件（避免打断）

只在**本次起草由用户在该 LC 的卡片上发起**且**用户仍停留在当前工作台**时触发一次：

- 触发源：`knowledgeBaseOperation.kind === "user_input" && status === "completed"`；
- 去重：以 `operation_id` 为键，同一个 operation 只跳一次（轮询重复拿到同一 operation 不重复跳）；
- 用户已离开该 LC 的工作台（切了项目/页面）⇒ 只留卡片入口，不强行抢焦点。

### D3：详情页的输入引导块

输入确认区顶部给四步指引（可执行文案，不臆造状态）：

1. 逐份打开 `user-input/` 文件核对并补充（点文件名即在右侧编辑器打开，可直接改）；
2. 改完点该份的「确认」；内容再改 ⇒ 确认自动失效（摘要比对），会提示「已修改，需重新确认」；
3. 全部确认后点【确认并初始化】；
4. 建库完成后在同页继续编辑知识库文档，**以你的修改为准**。

同时给两行事实：**确认进度**（已确认 X/N）与**未填阻塞**（还有 N 份未填 —— 未填的文件不能被确认，请先在编辑器里补充内容）。

### D4：【确认并初始化】= 批量确认 + 立即建库

- 先 `POST /user-input/confirm {all:true, items:[...]}`（items 携带每份**当前读到的 digest**，过期摘要按既有 409 口径拒绝）；
- 全部确认成功后立即 `POST /knowledge-base/initializations {kind:"bootstrap"}` —— 后端分叉此时看到「已准备且全部已确认」⇒ 建 bootstrap operation（**后端零改动**）；
- 失败可执行：确认过期 ⇒ 提示并刷新投影；建库被拒（未填/未确认）⇒ 原样展示后端中文 message。

### D5：未填文件的处理（只引导，不改后端口径）

后端口径是「9 份**全部已填写且已确认**才允许建库」（未填 = 与模板逐字相同，无法被确认）。本变更**不放宽**该口径，只做两件事：

- 面板显式列出「还有 N 份未填」并说明原因（未填不能确认）；
- 【确认并初始化】在存在未填文件时**禁用**并给出原因，避免用户点了才吃 422。

### D6：建库后的落点

【确认并初始化】触发的 bootstrap operation：

- 运行中：面板显示「知识库初始化中…」（复用既有 operation 轮询/取消能力）；
- 完成后：刷新文件树与状态投影 → 知识库文档出现在文件树里，顶部给「知识库已生成，可继续编辑」提示；页面**留在详情页**（即【查看知识库详细内容】的落点），用户可直接改文档。

### D7：用户修改优先

bootstrap 产物只是起点：详情页对知识库文档始终可写可存（既有能力），任何后续修改以用户落盘的内容为准 —— 本变更在引导文案里讲清楚，不改任何写路径。

## 3. 改动面

| 文件 | 改动 |
|---|---|
| `web/src/components/lifecycle/useKnowledgeBaseWorkbench.ts` | 起草完成 → 自动跳转（按 operation_id 去重）；暴露「打开输入文件」动作 |
| `web/src/components/lifecycle/KnowledgeBaseCard.tsx` | 起草完成态：CTA【打开输入文件并补充】+ 下一步说明行 |
| `web/src/components/lifecycle/KnowledgeBaseUserInputPanel.tsx` | 四步引导块、确认进度、未填阻塞说明、【确认并初始化】按钮（未填时禁用 + 原因） |
| `web/src/pages/KnowledgeBasePage.tsx` | 打开详情页时自动选中第一份待确认 `user-input` 文件；【确认并初始化】的编排（批量确认 → 建库 → 进度 → 完成刷新） |
| 后端 | **零改动**（起草 / 确认 / 分叉契约逐字保留） |

## 4. 风险

| 风险 | 处置 |
|---|---|
| 自动跳转打断用户 | 只在「本次由用户发起 + 用户仍在本工作台」时触发一次（D2），且卡片入口永远保留 |
| 重复建库 | 建库仍走后端单飞与 operation 唯一性；面板在运行中禁用按钮 |
| 未填文件让用户卡住 | 显式说明 + 按钮禁用（D5），不给「点了才报错」的黑盒 |
| 确认过期（用户同时改了文件） | 沿用既有 409 口径：提示并刷新投影，不静默重试 |
