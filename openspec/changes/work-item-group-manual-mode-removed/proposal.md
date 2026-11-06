# 工作项组不再提供手动生成模式（REQ-WIGA-01 MODIFIED）

> **本 change 记录 2026-11-07 owner 的业务裁定：「work item group 不允许手动生成模式，从弹窗里去掉『手动』」。**
> ★**粒度待 owner 拍板**（① 整个 fieldset 固定 automatic ／ ② 仅当本次提交将生成 group 时隐藏「手动」）——见 `## 两种粒度`。本 proposal 按**推荐 ①** 展开，并列出若改选 ② 所需的额外契约变更。

## Why

### 现象（owner 裁定原文）

```
「work item group 不允许手动生成模式，从弹窗里去掉『手动』」
```

### 现状落点（代码定位，2026-11-07 在 `bugfix-b-11-02` 工作树核对）

| 落点 | 位置 | 事实 |
|---|---|---|
| 「生成模式」fieldset | `web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx:333-361` | fieldset 内两个 radio：`:340-349`「手动」、`:350-359`「自动化」 |
| 组件自陈契约 | `web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx:42-43` | 「P1 WIGA：缺省『不自动化』；仅已确认 Design 上可选 automatic。」 |
| 渲染条件 | `web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx:60-61` | `automationAvailable`：「仅已确认 Design 显示自动化模式选择（缺省手动，人工链路不变）」 |
| `automationAvailable` 计算 | `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx:980-984` | `card.kind === "design_spec" && raw.confirmation_status === "confirmed"` |
| 弹窗唯一生产调用点 | `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx:978` | 全 `web/src` 仅此一处渲染 `<WorkItemPlanOptionsDialog>` |
| 弹窗的两个打开点 | `web/src/components/lifecycle/useIssueLifecycleGeneration.ts:280`（`handleGenerateNext`）、`:382`（`handleLaunchWorkspace`） | 两处均要求 `card.kind === "design_spec"` ⇒ 进入弹窗的实体**恒为 design_spec** |
| 提交分流（手动 vs 托管） | `web/src/components/lifecycle/useIssueLifecycleGeneration.ts:632-665` | `automation_mode === "automatic"` ⇒ `enrollConfirmedDesignAutomation`（写 enrollment）；`manual` ⇒ `prepareWorkItemPlan`（`:653`）＋ `onOpenWorkspace`（`:665`） |
| 可复用守卫先例 | `web/src/components/lifecycle/useIssueLifecycleGeneration.ts:627-630` | `card.kind !== "design_spec"` ⇒ 「当前实体不能生成 Work Item Plan」 |
| 后端不认识「生成模式」 | `src/**` 全仓检索 `automation_mode` **零命中** | 「手动／托管」的分流是**纯前端路由**；托管侧由 REQ-WIGA-03 的服务端编排器准备 plan/session |
| 「计划即组」 | `web/src/state/lifecycle-workbench-store.ts:130-146` | issue 的 `work_item_plan` 一律渲染成 `kind: "work_item_group"` 卡片（`id` = plan id，`childWorkItemIds` = 成员 WI），卡片标题就是「Work Item Group」（`web/src/components/lifecycle/LifecycleCard.tsx:231-232`） |
| 前端没有「本次是否产组」的先验判据 | `PendingWorkItemPlanLaunch`（`useIssueLifecycleGeneration.ts:36-45`）、`PrepareWorkItemPlanRequest`（`web/src/api/types/work-item-plan.ts:659-667`）、`IssueWorkItemPlanDetailDto`（`:637-657`） | 三处**均无** group／is_group／scope 字段；Work Item 数量只能由响应里的 `work_item_ids` 事后得知 |
| 「自动化」入口的既有事实面 | `web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx:151-155` | automatic 且目标投影未就绪／载体不受支持 ⇒ 提交禁用（fail-closed） |

### 两条决定粒度的关键事实

```
① 本弹窗**只有一个**生产调用点，且**只**从 design_spec 卡片打开
   ⇒ 「弹窗」与「design_spec 场景」在当前实现里是同一件事，不存在「同一弹窗服务两类场景」。
② 本弹窗的**每一次**提交产出的都是「工作项组」（前端把 plan 恒渲染为 Work Item Group 卡）
   ⇒ 前端在提交前**没有**「这次只产一个 WI、不产组」的先验判据。
```

### 为什么这是用户可见行为变更 ⇒ 必须走 OpenSpec change

```
· 「生成模式」是用户可见的界面选项（radio + 「生成模式」legend）。
· 「手动」是当前**默认值**（`useIssueLifecycleGeneration.ts:245` 建弹窗时写死 `automation_mode: "manual"`）。
· 去掉「手动」＝ 改变用户在该弹窗里能选什么、提交后走到哪条链路（人工推进 vs 托管自动化）。
⇒ 属于契约层变更 ⇒ 走 OpenSpec change（delta 落在 REQ-WIGA-01）。
```

## 两种粒度（★待 owner 拍板★）

### ①（推荐）整个 fieldset 固定 automatic（无论 kind）

**做法**

- 「生成模式」fieldset 不再渲染「手动」radio；已确认 design 的弹窗里生成模式**恒为托管自动化**。
- 提交恒走 `enrollConfirmedDesignAutomation`（写 enrollment）⇒ 工作项组由服务端编排推进。
- 随之，弹窗内的 manual 提交分支失去唯一生产入口 ⇒ 一并下线（不做「留死代码」式的半吊子）。
- `automationAvailable` 的判定口径**不变**（design_spec + 已确认）⇒ 未确认 design 的弹窗仍不呈现「生成模式」，提交仍是既有人工路径。

**代价**

- 该弹窗内不再有「本次 plan 走人工推进」的入口。
- 既有引用「手动」radio 的 3 处断言需要**改写**（不是删断言意图）：
  `WorkItemPlanOptionsDialog.reenable.test.tsx:189`、`WorkItemPlanOptionsDialog.target.test.tsx:246-250`、`WorkItemPlanOptionsDialog.plan-approval.test.tsx:110-115`。
- hook 层既有用例 `useIssueLifecycleGeneration.plan-approval.test.ts:246-270`（「manual 走 prepare 且不带 `plan_approval`／`automation_mode`」）的前提消失 ⇒ 需改写。
- ★**失败面风险**：目标投影读取失败或授权载体不受支持时，automatic 分支提交是 fail-closed 禁用的（`WorkItemPlanOptionsDialog.tsx:151-155`）。当前用户可以「切回手动」绕过（`WorkItemPlanOptionsDialog.target.test.tsx:246-250` 就是这条断言）；去掉「手动」后**该绕过消失** ⇒ 可能把用户堵死。兜底形态见下方待拍板项 D2。

**推荐理由**

1. **与 owner 原话逐字对齐**：owner 说的是「从弹窗里去掉『手动』」。
2. **② 所需的判据在当前代码里不存在**（关键事实 ②）⇒ ② 必须**先产出一条新判据**（design 的预期 WI 数？issue 的逻辑仓成员数？服务端预检返回「本次将产 N 个 WI」？），那是**新契约**且要跨前后端 ⇒ 违背「去掉一个选项」的小改成本。
3. **切面最小且干净**：只碰前端弹窗与其提交分流；后端零改动（后端本就不知道 `automation_mode`）。
4. **不误伤人工面**：REQ-WIGA-03 / REQ-WIGA-05 的人工门、AI choice、最终确认在托管链路上同样保留 ⇒ 「去掉手动**生成模式**」≠「取消人工确认」。

### ② 仅当「本次提交将生成 group」时隐藏「手动」（复用 `card.kind !== "design_spec"` 守卫同族判据）

**做法**

- 弹窗保留 fieldset，但在「本次提交将生成 group」时**不渲染**「手动」radio（默认托管自动化）。
- 判据来源：与 `useIssueLifecycleGeneration.ts:627-630` 的 `card.kind !== "design_spec"` 同族的既有守卫口径。

**代价 / 阻塞点**

- ★**当前代码里没有这个判据**：弹窗只从 design_spec 打开（`:280`、`:382`），且其产物恒为工作项组（`lifecycle-workbench-store.ts:130-146`）⇒ 「本次提交将生成 group」在当前实现里**恒为真** ⇒ ② 在今天**退化为 ①**。
- 要让 ② 有意义，必须**先产出一条新的先验判据**（新增一条 projection 或新字段）⇒ 那是**独立的一次契约变更**，须先走 brainstorming 与契约评审，再谈弹窗改动。
- ⇒ ② 不是「① 的更细版本」，而是「① + 一条新判据的契约变更」。**若 owner 将来要 ②，建议独立成一个后续 change**，不与本 change 捆绑。

### ★需要 owner 一并拍板的事项

| 编号 | 待拍板项 | 选项 | 我的推荐 |
|---|---|---|---|
| **D1** | **粒度** | ① / ② | **①**（与原话逐字对齐；② 无判据可用） |
| **D2** | **目标投影不可得时的兜底**（automatic fail-closed 后用户被堵死） | (a) 保留「创建并打开 Workspace」兜底出口（**不**以「生成模式」radio 形态呈现）／(b) 只如实报错，无兜底 | **(a)**：不给「生成模式」选项 ≠ 取消人工推进能力；但 (a) 实质上保留了人工路径，owner 若认为「任何情况下都不许人工」则选 (b) |
| **D3** | **未确认 design 的弹窗**是否也强制托管 | (a) 保持现状（未确认 ⇒ 不呈现生成模式 ⇒ 走既有手动）／(b) 一并托管化 | **(a)**：自动化授权的前提是 design 已确认（`useIssueLifecycleGeneration.ts:451-453` 明写「自动化要求 Design Spec 已确认」），未确认时**技术上无法**托管化 ⇒ (b) 不可实现 |

> D2 / D3 不改本 change 的 delta 结构（delta 已按「不得静默降级」「不得伪造托管事实」写定），只影响前端实现细节 ⇒ 拍板结果落在 tasks 的实现工作包里。

## 不做（anti-goals）

```
✗ 不改后端（src/** 零改动）：托管化所需的服务端编排能力已由 REQ-WIGA-03 / REQ-WIGA-C5-CHAIN 提供；
  后端全仓不知「生成模式」，本次不需要新增任何 wire 字段。
✗ 不改主 spec（openspec/specs/**）：本 change 只写 delta；archive 时才同步主 spec。
✗ 不取消人工门 / AI choice / 最终确认（REQ-WIGA-03 / REQ-WIGA-05）：去掉的是「手动生成模式」，
  不是「人工确认」——托管链路同样在 AI choice 与人工门停等。
✗ 不动 story / design 的人工生成与人工确认流程。
✗ 不新增「本次提交是否生成 group」的先验判据（那是 ② 的前提，须独立立项）。
✗ 不删后端 `POST …/work-item-plans:prepare` 端点：端点契约面保留，只是该弹窗不再由前端调用它。
✗ 不动「工作项组」卡片、抽屉、编码侧（group attempt / 逐仓渲染）的既有呈现。
```

## Impact

- **契约**：`work-item-group-autopilot` —— REQ-WIGA-01 **MODIFIED**（本 change 的 delta）。
- **前端（生产代码）**：
  - `web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx`（「生成模式」fieldset 与「手动」radio；提交按钮形态）。
  - `web/src/components/lifecycle/useIssueLifecycleGeneration.ts`（弹窗提交分流；manual 分支随之下线时，其 `prepareWorkItemPlan` 引用一并清理）。
  - `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`（`automationAvailable` 口径——① 下**逐字不变**；仅 ② 才需扩展）。
- **前端（测试）**：新增 1 个弹窗用例文件；改写既有 4 处引用「手动」的断言；`IssueLifecycleWorkbench.generation.test.tsx:590-612`（未确认 design ⇒「自动化」radio 不存在、且「创建并打开 Workspace」存在）作为「逐字不变」的回归锚点**保持不动**。
- **后端**：零改动。
- **风险**：
  1. **失败面**（D2）：目标投影不可得时的人工兜底消失 ⇒ 可能把用户堵死。
  2. **既有断言**：`手动` radio 被 3 处既有断言引用，改写时 MUST NOT 弱化其原意（原本断言的是「可以切回手动」，新断言应是「手动不再存在」而非删掉）。
  3. **语义收窄**：已确认 design 的弹窗提交恒为托管 ⇒ 托管链路的失败面（provider 不可用、target 非法等）暴露面变大；本 change 不改变这些失败面本身，只改变默认路径。
