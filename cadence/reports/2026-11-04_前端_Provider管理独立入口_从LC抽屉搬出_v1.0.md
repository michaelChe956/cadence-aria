# Provider 管理独立入口（从逻辑代码库抽屉搬出）v1.0

- 日期：2026-11-04
- 范围：`web/src/components/**`（前端信息架构 + 断言迁移）
- 触发：owner 2026-11-04 裁定——能力核验已全局化（项目级），provider 管理**不该寄居在 LC 仓抽屉里**
- 硬约束遵守：未改 `ProviderCapabilityCard.tsx` 的渲染逻辑与视觉；未碰 `src/**`；未碰 `tests/it_product/**`、`tests/it_web/**`；未碰 `openspec/**`、`spec/delta/tasks.md`；未做任何 git 操作

---

## 1. 结论摘要

provider 的探针 / 核验 / 更新 / 安装 / 版本，已从「逻辑代码库管理」抽屉**整体搬出**，改为侧栏一个平级分区
「Provider 管理」→ 点开一个**独立的 Provider 管理面板**（独立抽屉外壳，独立开关）。

- 侧栏新增 `<section aria-label="Provider 管理">`（与「代码库」「已删除」平级、同风格），内含
  `h3 = Provider` + 按钮 `data-testid="provider-management-open"`（文案「Provider 管理」），
  回调 `onOpenProviderManagement` 由上层接。
- 新增 `ProviderManagementDrawer`（标题「Provider 管理」，× / Esc / 点遮罩三路关闭）+ `ProviderManagementPanel`
  （承载**整个** `ProviderCapabilityCard`）。
- `LogicalCodebaseManagementPanel` 里删掉 `<ProviderCapabilityCard …/>`，原位置留**一行只读说明** +
  「去 Provider 管理」入口（**不含任何自造的统计数字**）。
- 两个管理浮层**互斥**：打开 Provider 管理前先收起 LC 管理抽屉（两者都是右侧同宽抽屉，叠加会互相遮挡）。

---

## 2. 新的信息架构

```
侧栏（ProjectSidebar）
├── Projects
├── 当前 Project 代码库
├── Provider                ← ★ 新增平级分区（aria-label="Provider 管理"）
│     └── [⚙ Provider 管理]  data-testid="provider-management-open"
└── 已删除的逻辑代码库

点击「Provider 管理」
   └── 独立抽屉「Provider 管理」            data-testid=provider-management-drawer（role=dialog, aria-modal）
         ├── 标题栏：Provider 管理  [×]     data-testid=provider-management-close
         └── ProviderManagementPanel         data-testid=provider-management-panel
               ├── 作用域说明：Provider 能力核验、安装与版本管理属项目级全局能力，与逻辑代码库无关。
               └── ProviderCapabilityCard（整体搬来，渲染逻辑与视觉一字未改）
                     ├── 头部：Provider 能力核验 + [一键核验全部]
                     ├── 作用域说明（全局核验口径）
                     └── 四家 provider 的行级列表（启动/写边界/恢复 三维度 + 核验/更新/安装 + 版本落后 + 核验回显）

逻辑代码库管理抽屉（LC 管理抽屉，行为不变）
├── 逻辑代码库（切换 tab + 登记成员）
├── 冷启动状态（bootstrap 卡）
├── Provider 能力核验、安装与版本管理已移至「Provider 管理」入口。   ← ★ 只读说明（无数字）
│     └── [去 Provider 管理]  data-testid="provider-management-link"（点击 → 打开独立面板）
├── 聚合初始化卡（本次顺带接上 advanceStop，见 §6）
├── 聚合索引卡 / 指针发布面板
```

### 2.1 抽屉外壳的单一约定

原 `LogicalCodebaseManagementDrawer.tsx` 的 179 行外壳实现**上提**为共享组件
`web/src/components/shared/ManagementDrawer.tsx`（遮罩 + 右侧面板 + 200ms 进出场过渡 + Esc/遮罩/× 三路关闭
+ 焦点移入归还 + Tab 圈闭 + body 滚动锁；行为内核复用既有 `shared/DialogShell` 的 `useModalBehavior`）。
两个抽屉各留一个 20~30 行的薄壳：

| 抽屉 | 标题 | testid 前缀 |
|---|---|---|
| `LogicalCodebaseManagementDrawer` | 逻辑代码库管理 | `lc-management-*`（**不变**） |
| `ProviderManagementDrawer` | Provider 管理 | `provider-management-*`（新） |

> 既有 `LogicalCodebaseManagementDrawer.test.tsx`（9 条）原样全绿 ⇒ 外壳行为零回归。

---

## 3. 断言迁移（逐条）

### 3.1 `ProviderCapabilityCard.test.tsx`（19 条，**一条未删、未改松**，只换宿主）

| 类别 | 断言 | 处置 |
|---|---|---|
| 结构性 | 块间 1px 分割线（`divide-y` + `divide-[var(--aria-line)]`） | **保留**（宿主换成 Provider 管理面板） |
| 结构性 | 行1 `grid-cols-[1fr_auto]` + 名 `font-semibold` + 版本 `text-right`/`aria-mono` | **保留** |
| 结构性 | 行2 `flex-wrap` + 胶囊 `rounded-full` 且无 `rounded-md` | **保留** |
| 结构性 | 行3 维度容器在操作容器之前 + 操作 `ml-auto`/`justify-end` + 核验按钮 `btn-secondary` | **保留** |
| 结构性 | 块内 ≥3 个独立行容器（未退化成文字流）、块无 `rounded`/`border`、无 `grid-cols-2`、4 个 `listitem` | **保留** |
| 行为 | 只读 GET 投影（URL 逐字断言）、四家 provider 全渲染、聚合口径（已核验/未核验/拒绝） | **保留** |
| 行为 | POST `capability-revalidate`（URL/method/body 逐条）、`already_confirmed`/`revalidated` 回显、在途禁用 + 核验中、失败稳定码 + 原始 detail | **保留** |
| 行为 | 版本落后 / 更新 / 当前→最新；安装入口 + 手动命令；codegraph 不出现 | **保留** |
| 行为 | 「一键核验全部」在能力卡头部且在四家行之前 + 全局核验口径说明（5 条子串断言） | **保留** |
| 并行 worker 迁入的 6 组 | 批量逐项状态（随轮询更新）/ 失败项单项重试（不重跑整批）/ 跳过项安装入口 / 版本落后更新入口 / 批量已确认仍保留更新 / 批量并入行级列表 | **一行未删** |
| 宿主 | 渲染容器：`render(<ProviderCapabilityCard …/>)` ×15 | **换宿主** → 统一走 `renderProviderManagement()` = `ProviderManagementDrawer(open)` + `ProviderManagementPanel(projectId, logicalCodebaseId)`；**只有**「无 active LC ⇒ 不渲染任何东西」那条仍直接渲染能力卡（它断言的就是能力卡自身的 null 行为，包一层面板反而会破坏 `container.firstChild === null`） |

### 3.2 `LogicalCodebaseManagementPanel.test.tsx`（原 1 条 → 现 2 条）

| 原断言 | 处置 |
|---|---|
| 「一键核验全部」按钮在能力卡头部（上方）、在四家 provider 行之前、作用域说明 | **迁到 `ProviderManagementPanel.test.tsx`**（宿主换成 Provider 管理面板，断言内容逐条保留，含全局口径 5 条子串） |
| — | **新增**：LC 面板**不再渲染** provider 核验区（`provider-capability-card` / `provider-capability-list` / 四家行 / 「一键核验全部」按钮 / `capability-batch-scope` / `capability-batch-start` 全部为 null） |
| — | **新增**：原位置留只读说明（`lc-provider-management-summary`，断言其**不含**「已确认」等自造数字）+「去 Provider 管理」入口，点击触发 `onOpenProviderManagement` |

### 3.3 新增 `ProviderManagementPanel.test.tsx`（4 条）

1. Provider 管理面板**渲染**能力核验区（面板 / 能力卡 / 行级列表 / 四家 provider 行）。
2. 「一键核验全部」在能力卡头部且在四家行之前 + 全局核验口径说明（自 3.2 迁来，不弱化）。
3. 抽屉标题「Provider 管理」+ `aria-modal=true` + × 按钮触发 `onClose`。
4. Esc 触发 `onClose`。

### 3.4 新增 `IssueLifecycleWorkbench.provider-management.test.tsx`（3 条，工作台级接线）

1. 侧栏有「Provider 管理」分区与入口 → 点击打开**独立**抽屉（`aria-label=Provider 管理`），此时
   `lc-management-drawer` 不存在；面板渲染四家 provider 的行级列表。
2. 打开 LC 管理抽屉 → 抽屉内**看不到** provider 核验区（能力卡 / 行级列表 / 「一键核验全部」按钮均为 null），
   只有只读说明 + 「去 Provider 管理」入口。
3. 点 LC 抽屉里的「去 Provider 管理」→ 独立面板打开（4 行），且 LC 抽屉收起（两浮层互斥）。

### 3.5 连带修正

- `src/onboarding/anchors.test.tsx`：`ProjectSidebar` 新增必填 prop `onOpenProviderManagement` ⇒ 补 `() => {}`（+1 行）。

---

## 4. 验收证据

### 4.1 typecheck（exit 0）

```
$ cd web && pnpm -s typecheck
（无输出）
typecheck_exit=0
```

### 4.2 lint（exit 0；改动文件 0 条新增告警）

```
$ cd web && pnpm -s lint
lint_exit=0
（仓库既有 399 条 warning / 0 error；其中本次改动文件
  web/src/components/shared/ManagementDrawer.tsx、
  web/src/components/lifecycle/ProviderManagementDrawer.tsx、
  web/src/components/lifecycle/ProviderManagementPanel.tsx、
  web/src/components/lifecycle/ProviderManagementPanel.test.tsx、
  web/src/components/lifecycle/IssueLifecycleWorkbench.provider-management.test.tsx
  ⇒ 0 条告警；
  ProjectSidebar.tsx / LogicalCodebaseManagementPanel.tsx / LogicalCodebaseManagementDrawer.tsx
  / ProviderCapabilityCard.test.tsx / anchors.test.tsx ⇒ 0 条告警；
  IssueLifecycleWorkbenchView.tsx 的 3 条 unused-import 告警为改动前既有，本次未触碰那些 import）
```

### 4.3 定向测试（全绿）

```
$ cd web && pnpm exec vitest --run \
    src/components/lifecycle/ProviderCapabilityCard.test.tsx \
    src/components/lifecycle/ProviderManagementPanel.test.tsx \
    src/components/lifecycle/LogicalCodebaseManagementPanel.test.tsx \
    src/components/lifecycle/LogicalCodebaseManagementDrawer.test.tsx \
    src/components/lifecycle/IssueLifecycleWorkbench.provider-management.test.tsx \
    src/onboarding/anchors.test.tsx

 ✓ src/components/lifecycle/LogicalCodebaseManagementPanel.test.tsx (2 tests) 77ms
 ✓ src/components/lifecycle/ProviderManagementPanel.test.tsx (4 tests) 229ms
 ✓ src/components/lifecycle/ProviderCapabilityCard.test.tsx (19 tests) 479ms
 ✓ src/onboarding/anchors.test.tsx (10 tests) 141ms
 ✓ src/components/lifecycle/LogicalCodebaseManagementDrawer.test.tsx (9 tests) 1230ms
 ✓ src/components/lifecycle/IssueLifecycleWorkbench.provider-management.test.tsx (3 tests) 503ms

 Test Files  6 passed (6)
      Tests  47 passed (47)
```

### 4.4 全量测试（完整计数，未截断）

```
$ cd web && pnpm -s test

 Test Files  264 passed (264)
      Tests  2694 passed (2694)
   Start at  23:41:49
   Duration  33.92s (transform 10.20s, setup 51.96s, collect 77.18s, tests 149.55s, environment 142.42s, prepare 21.93s)
```

### 4.5 浏览器实测（harness 内置 Chromium，**未用 Playwright**）

- 被测前端：本 worktree 的 `web/`（vite dev server，临时配置放 `/tmp/vite-provider-verify.config.mjs`，
  与仓库 `vite.config.ts` 唯一差异是 `/api` 代理加 `changeOrigin`，让后端放行 Host）；
  后端为已在跑的 `aria web`（127.0.0.1:4317，workspace `/tmp/aria-verify`，项目 `test01` / 逻辑代码库 `agg`）。
- 真实点击（非代码推断）：
  - 点侧栏「Provider 管理」→ 独立抽屉打开，`aria-label = Provider 管理`，
    `provider-capability-list` 内 **4 个 `<li>`**（Claude Code 2.1.307 / Codex codex-cli 0.166.0 / Pi 1.3.0 / Kimi Code 2.4.1），
    每行含「已确认 / 版本落后 → … / 启动·写边界·恢复 已核验 / 核验 X / 更新 当前→最新」。
  - 打开逻辑代码库管理抽屉 → 抽屉内 `provider-capability-card` = **false**、
    `provider-capability-list` = **false**、`capability-batch-start` = **false**、
    `capability-batch-scope` = **false**；`lc-provider-management-summary` = **true**，
    `provider-management-link` 文案 = 「去 Provider 管理」。
  - 真实点击该「去 Provider 管理」链接 → `provider-management-drawer` = true、provider 行数 = 4、
    `lc-management-drawer` = false（两浮层互斥）。

**截图（正式验收证据，`cadence/reports/`）**

| 路径 | 内容 |
|---|---|
| `cadence/reports/2026-11-04_前端截图_Provider管理独立入口_v1.0.png` | ★ 主证据：侧栏「Provider 管理」入口 + 右侧独立面板（四家 provider 行级列表） |
| `cadence/reports/2026-11-04_前端截图_Provider管理独立入口_面板行级列表_v1.0.png` | Provider 管理面板整幅（标题 / 一键核验全部 / 全局核验说明 / 四家行） |
| `cadence/reports/2026-11-04_前端截图_Provider管理独立入口_LC抽屉无provider核验区_v1.0.png` | 工作台全景 + LC 抽屉（**无** provider 核验区） |
| `cadence/reports/2026-11-04_前端截图_Provider管理独立入口_LC抽屉摘要与去管理入口_v1.0.png` | LC 抽屉内的只读说明 + 「去 Provider 管理」按钮（特写） |

---

## 5. 被改文件清单（行数）

### 新建

| 文件 | 行数 |
|---|---|
| `web/src/components/shared/ManagementDrawer.tsx` | 133 |
| `web/src/components/lifecycle/ProviderManagementDrawer.tsx` | 30 |
| `web/src/components/lifecycle/ProviderManagementPanel.tsx` | 34 |
| `web/src/components/lifecycle/ProviderManagementPanel.test.tsx` | 199 |
| `web/src/components/lifecycle/IssueLifecycleWorkbench.provider-management.test.tsx` | 162 |

### 修改（+/-）

| 文件 | +/− |
|---|---|
| `web/src/components/lifecycle/IssueLifecycleWorkbenchView.tsx` | +37 / −3 |
| `web/src/components/lifecycle/ProjectSidebar.tsx` | +33 / −1 |
| `web/src/components/lifecycle/LogicalCodebaseManagementPanel.tsx` | +27 / −6 |
| `web/src/components/lifecycle/LogicalCodebaseManagementPanel.test.tsx` | +39 / −22 |
| `web/src/components/lifecycle/LogicalCodebaseManagementDrawer.tsx` | +11 / −168（外壳上提共享） |
| `web/src/components/lifecycle/ProviderCapabilityCard.test.tsx` | +56 / −110（宿主迁移，断言未减） |
| `web/src/onboarding/anchors.test.tsx` | +1 / −0 |

> 同 worktree 内另有并行 worker 的改动（`AggregateInitializationCard.tsx/.test.tsx`、`ProviderCapabilityBatch.tsx/.test.tsx`）
> —— **不属于本次改动**，未计入上表。

---

## 6. 顺带完成：聚合初始化卡的 `advanceStop` 传参（Main 追加项）

`LogicalCodebaseManagementPanel` 本就持有 `bootstrapProjection` ⇒ 取其 `advance_stop` 直接传给
`AggregateInitializationCard` 的可选 prop：

```tsx
advanceStop={bootstrapProjection?.advance_stop ?? undefined}
```

- 未推进 / 无停止记录时为 `undefined` ⇒ 卡片不渲染停止原因块。
- 字段形状与卡片侧 `AggregateInitializationAdvanceStop` 一一对应
  （`step` / `reason` / `reason_code` / `detail?` / `stopped_at`），**不新造文案**（卡片复用冷启动卡片同一套
  `ADVANCE_STOP_LABELS` / `ADVANCE_STOP_GATE_LABELS`）。
- 卡片侧（`AggregateInitializationCard.tsx`）由并行 worker 实现，本任务**未改其渲染**。
- 浏览器实测已见效果：LC 抽屉的「聚合初始化」卡显示「failed / provider 的第一步（pre_check）没有完成」。

---

## 7. Concerns

1. **摘要行没有数字口径（有意为之）**：LC 抽屉里那行只读说明**不含**任何统计数字。理由：能力投影
   （`GET …/capabilities`）此刻只存在于 `ProviderCapabilityCard` 内部，LC 面板拿不到；若要显示
   「N 家已确认」，需另引一份投影/口径（=新造统计口径），按任务要求**不做**。数字口径待能力全局化落地后再定。
2. **能力面仍需一个 `lc_id`**：后端确认「端点形状不变，仍按逻辑代码库定位（承载 404 守卫）」，内容已是项目级全局。
   本面板取「当前选中 LC，没有就取项目下任意一个已存在的 LC」。**副作用**：批量作业的 `job_id` 存在
   `localStorage["aria:capability-batch:<projectId>:<lcId>"]` ⇒ 切换选中 LC 后，本面板读到的作业 key 会变
   （旧 key 仍在 localStorage 里）。若后端后续把作业面也全局化，这里可一并简化。
3. **`e2e/spec/real/s4-story.spec.ts:66-69` 会失效（未改，需 e2e 属主跟进）**：
   该 Playwright 资产在 `expandLogicalCodebasePanel()`（= 展开 LC 管理抽屉）之后断言
   `provider-capability-card` 可见 —— 能力卡已搬走 ⇒ 这条 e2e 需要改成先点侧栏「Provider 管理」再断言。
   按 `cadence/project-rules/browser-verification-only.md`（仓库既有 Playwright 资产保持原样、不删不改）
   与本次文件范围，**本任务未改 `e2e/**`**，在此显式交接。
4. **抽屉外壳上提共享组件**：`LogicalCodebaseManagementDrawer.tsx` 的外壳实现上提到
   `web/src/components/shared/ManagementDrawer.tsx`，行为内核复用既有 `shared/DialogShell` 的 `useModalBehavior`
   （Esc / 焦点圈闭与归还 / 滚动锁）。既有 9 条抽屉断言原样全绿，但**Esc 的监听位置**由「层内 React onKeyDown」
   变为「window 级 keydown + 仅当焦点在本层内才响应」（与 `WorkbenchOverlayDrawer` 同一规范）——
   语义等价且更严格（不会一次 Esc 连关多层浮层）。
5. **两个管理浮层互斥**：打开 Provider 管理会先收起 LC 管理抽屉（两者同为右侧同宽抽屉，叠加会互相遮挡）。
   如产品希望允许同时打开，需要改成左右分栏或栈叠布局。
