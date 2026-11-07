# 工作项组移除「手动」生成模式独立审查报告

## 1. 状态与审查基线

**状态：可交付（代码级审查结论，不等同于已完成 owner 真机验收或发布验收）。**

一句话理由：实现符合 oracle 的 D1／D2／D3，已确认 design 的弹窗不存在人工替代提交；原提交中的集成测试迁移缺口已由 `5a3323e2` 修订，manual 分支下线的矛盾文案也已在本次读取的工作树中纠正，当前未发现未闭合的 P0／P1／P2 产品缺陷。

- 原始审查对象：`633e0217`（契约措辞修正）、`06427ebe`（新增回归测试）、`45f2b588`（实现及三份弹窗测试改写）；逐个读取了 `git show`，没有把相邻的其它业务提交混算进本 change。
- 补充复审对象：`5a3323e2`（三份工作台测试的 fixture／入口迁移）；读取了实际 diff，不仅依据 controller 转述判断修复。
- 收口时读取的 HEAD 为 `5a3323e2`；proposal/tasks 的最后一轮文字修正在并发落盘，以下明确区分「原提交证据」「工作树修订证据」「controller 提供的运行结果」。
- 全部源码与契约读取均定位真实工作树 `/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02/`；未读取裸仓库根下的遗留 `web/`、`src/` 杂散源码树。
- 下文源码路径均相对上述工作树；旧版本行号明确附提交号，不将新增 helper 导致的行号位移误判为用例文本改写。
- 本审查唯一写入文件就是本报告；没有修改产品代码、测试或契约，没有暂存、提交、回退及任何破坏性 Git 操作。

## 2. 八条审查结论摘要

| 序号 | 结论 | 主要证据 |
|---|---|---|
| 1 | D1／D2／D3 全部符合；D2 指新授权目标不可得时禁止人工兜底，不取消既有重新开启逻辑 | `IssueLifecycleWorkbench.tsx:980-984`；`WorkItemPlanOptionsDialog.tsx:88-89,155-160,224-238,339-357,513-528` |
| 2 | 已确认 design 的生产弹窗彻底移除「手动」radio、模式切换器及 manual 提交；全产品的人工 API／旧组行为并未被删除 | 弹窗 `:31,45,88,237,339-357`；hook `:52,245,633-665` |
| 3 | 未确认 design 的现存人工路由、默认值、文案与后端确认前置保留；「未确认 design 已成功手动产组」不能成立，只存在条件性灰区 | hook `:245,653-665`；store `:130-146`；`plan_preparation.rs:73-74,142-177`；`lifecycle_aggregate_scope.inc.rs:94-98` |
| 4 | 三份原弹窗测试改写均由语义变化所必需，未发现断言弱化；后续八条工作台用例仅迁移人工路径 fixture／入口，保护用例原文不变 | 本报告 §5；保护用例原 `:590-614`、现 `:599-623` |
| 5 | 两条新增测试能够捕获指定的「恢复手动 radio」「恢复原名兜底按钮」变异；不能单独证明成功提交的 enrollment 绑定或所有交互事件 | 新测试 `:105-124,127-153`；工作台托管用例 `:382-458` |
| 6 | REQ-WIGA-01 新场景逐条可映射；禁止人工 prepare 由 UI 无入口、提交阻断、mode 归一化和 hook 消费分支共同保证 | delta `:4-30`；本报告 §7 对应表 |
| 7 | ① 根因仍在：弹窗空 query、后端默认 codex；⑤ 与 D2 叠加后若该故障复现，弹窗内不能完成任何提交，须 owner 知情确认 | 弹窗当前 `:172`（旧锚点 `:167` 已移位）；`src/web/handlers/support.rs:378-390` |
| 8 | 原三 commit 没有触碰禁改产品面或后端；原 UI 文案除删除「手动」选项外未改字面值；组件 680 行，距 1200 上限余 520 行 | 精确逐提交文件清单；源码字节比较；弹窗 `:527`；静态行数统计 |

## 3. D1／D2／D3 与模式边界

### 3.1 D1：已确认 design 的整个模式区固定自动化

- 唯一生产装配点仍在 `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx:977-996`，`automationAvailable` 仍严格取 `design_spec && confirmation_status === "confirmed"`（`:980-984`）。
- `WorkItemPlanOptionsDialog.tsx:88` 直接令 `automatic = automationAvailable`，不再依赖表单中遗留的 `options.automation_mode`；`:339-357` 只有一个「自动化」radio，`:349-351` 为选中态加空操作 `onChange`。
- 提交边界 `:234-238` 对 automatic 明确写入 `automation_mode: "automatic"`。消费侧不是静默丢弃：`useIssueLifecycleGeneration.ts:633-650` 显式匹配 automatic，进入 `enrollConfirmedDesignAutomation`，然后刷新并关闭弹窗。
- enrollment 消费链 `useIssueLifecycleGeneration.ts:434-441,464-506` 使用实际选择的 provider 获取目标，构造 enable command 并 `putAutomationEnrollment`；不是调用人工 prepare 后只修改 UI 名称。
- 保留单项 radio 符合裁决；继续点击「自动化」不会切回 manual，既有点击动作仍有对应控件。

**结论：符合 D1。**

### 3.2 D2：目标不可得时无人工兜底

- `WorkItemPlanOptionsDialog.tsx:155-160` 对常规新授权的缺 scope、目标未 ready、legacy 不支持载体计算提交阻断；`:226-228` 同时防守表单处理函数，不只是把按钮画成禁用。
- `:382-398` 呈现目标加载失败及「重试」；`:190-195` 包含 retry tick，重试确实触发重新读取；`:441-448` 对不支持载体呈现阻断说明。
- `:524` 将阻断传给 submit disabled；`:527` 在 automatic 下只有「启用自动化」，没有「创建并打开 Workspace」分支可进入。
- `:513-520` 的「取消」绑定 `onClose`，目标错误时没有 submitting，因此仍可退出；目标读取失败本身不调用 `onClose`。
- 组件没有 prepare API 调用；其 callback 在自动化场景被归一为 automatic，hook 的 `:650` 返回发生在人工 prepare `:653` 之前。
- **既有例外不是本次新增兜底**：终止／作废 enrollment 的重新开启可用已有记录的 target，`WorkItemPlanOptionsDialog.tsx:93-107,141-160,360-371` 不要求重新取得一个新 target。这仍是托管授权重新开启，不是人工 workspace，原重新开启测试继续覆盖该边界。

**结论：符合 D2；无「把手动改名为 Workspace」绕过。**

### 3.3 D3：未确认 design 的人工链路保留

- `useIssueLifecycleGeneration.ts:7` 的 `prepareWorkItemPlan` import 保留，`:47-53` 与 `:239-248` 的默认 `automation_mode: "manual"` 保留。
- 当 `automationAvailable=false`，弹窗 `:88-89` 使 automatic=false 且不读取 target；`:339-357` 整块模式 fieldset 不渲染；`:237` 将原 options 原样交回；`:527` 仍为「创建并打开 Workspace」。
- hook `:633` 先移除仅用于前端分流的 `automation_mode`，人工分支 `:653-665` 仍调用 prepare、刷新、关闭弹窗并打开返回的 workspace，未改提交载荷。
- 产品源码 `useIssueLifecycleGeneration.ts`、`IssueLifecycleWorkbench.tsx`、`web/src/pages/ChatCockpitPage.tsx` 对比 `06427ebe^` 与本次工作树均字节一致。
- **入口层准确含义**：`LifecycleCardDrawer.tsx:115-118,144` 的「生成 Work Item」仅在 confirmed design 的抽屉出现；draft 抽屉原本没有这个按钮，不是⑤将其删除。未确认 design 的既有入口是 Work Item 阶段空面板「准备 Work Item Plan」，见原保护用例 `IssueLifecycleWorkbench.generation.test.tsx:590-614`，当前仅移位到 `:599-623`。

**结论：符合 D3。人工路由保留不等于后端允许未确认 design 成功准备 plan。**

## 4. 灰区事实的独立核实与纠正

### 4.1 已确证的投影规则

`web/src/state/lifecycle-workbench-store.ts:129-147` 取 issue 最新 plan，只要 plan 存在即推入 `columns.work_item`，固定 `kind: "work_item_group"`（`:132`）、标题「Work Item Group」（`:135`），不按 enrollment 或 automation_mode 区分。

因此，**任何已经存在或从合法人工入口产生的 plan 都会显示成组卡**；不能用「没有手动 radio」推导「系统已不存在人工组／人工 prepare API」。后端人工 prepare 端点和旧组是明确非目标。

### 4.2 不能省略的后端消费前置

- 前端未确认 design 的提交仍能到达 `useIssueLifecycleGeneration.ts:653` 的人工 prepare 请求。
- 后端 `src/web/handlers/lifecycle/plan_preparation.rs:73-74` 先校验 story 与 design 已确认。
- `src/web/handlers/lifecycle_aggregate_scope.inc.rs:94-98` 对非 Confirmed design 明确返回 `design_spec_not_confirmed`。
- plan 创建发生在 `plan_preparation.rs:142-177`，晚于上述校验，未确认 design 会在创建之前退出。
- 两份后端文件相对红测前基线字节一致：这是既有前置，不是⑤引入的回归。

**精确结论：条件性灰区（当前不可达）。** 当前通过「真实未确认 design → 本弹窗 manual 提交」成功新建工作项组的路径走不通；只有未来放开确认前置，或出现其它合法人工产组路径时，「plan 一律投影为组卡」才会显现出与 owner 原话「不允许手动生成模式」的张力。这里的「不可达」仅针对该新建路径，不否认已有人工组、保留的 hook manual 分支或人工 prepare API。

**严重性低于 controller 最初描述的版本；owner 读到的版本应以本条为准，controller 先前的表述已作废。** 本条仍列入需 owner 知情确认，不由审查者擅自扩大 D1 范围。

**验收提示：owner 验收⑤时，若未确认 design 下提交报 `design_spec_not_confirmed`，这是既有后端前置，不是本次改动引入的缺陷。**

### 4.3 测试诚实边界

后续修订的工作台用例在 draft fixture 下模拟 prepare 成功，可以证明 UI 入口、请求参数、回调和 plan→group 投影，但不是后端验收。`IssueLifecycleWorkbench.test-utils.ts:139-142` 负责把 design 改为 draft，人工 prepare mock（`:554-590`）仍直接返回 plan/session；它没有执行 Rust 的确认守卫，不能作为真实未确认 design 已成功产组的证据。

## 5. 既有断言改写逐条对照

以下旧行号来自 `45f2b588^`，新行号来自已审阅实现／后续修订；所有改写均说明语义必要性，不把测试通过等同于真实后端成功。

| 文件与位置 | 改前 | 改后 | 必要性与强度判断 |
|---|---|---|---|
| `WorkItemPlanOptionsDialog.reenable.test.tsx` 旧 `:188-189` → 新 `:188-197` | 「手动」radio 可用；provider 仍可更换 | 「手动」radio 不存在，「自动化」radio 可用；原 provider 可用断言保留 | D1 必需；对旧模式的允许改成禁止，不是删去约束，无弱化 |
| `WorkItemPlanOptionsDialog.target.test.tsx` 旧 `:98-139` → 新 `:99-138` | 初始 manual 不请求 target，点击自动化后请求并阻断 loading 提交 | 打开已确认 design 弹窗即请求 target，保留精确请求 URL、loading 禁用、target 身份内容及 ready 后启用 | D1 必需；删除的是不再存在的初始 manual 状态，核心 target 门禁没有放宽 |
| 同上旧 `:231-262` → 新 `:220-269` | target 失败后切 manual、人工按钮启用、点击后 onConfirm 调用一次 | 失败告警、自动化按钮禁用、无 manual radio、无人工按钮、有重试、onConfirm 零调用、target 恰一次、无 prepare | D2 必需；允许绕过改为禁止绕过，新增多个否定约束，无弱化 |
| `WorkItemPlanOptionsDialog.plan-approval.test.tsx` 旧 `:83-119` → 新 `:86-108` | 初始 manual 可编辑；切 auto 固定关闭并说明；切回 manual 恢复 | 已确认 design 打开即 auto，仍断言禁用、固定关闭、两条说明文本；新增无 manual radio、说明保留 | D1 必需；不可达的模式往返被去掉，auto 核心断言未删 |
| 同上旧 `:122-142` → 新 `:111-130`；helper 新 `:65-73` | 使用默认 `automationAvailable=true` 测试 manual 复选框可编辑 | fixture 显式 `automationAvailable=false`；原可编辑、初始勾选、无说明、点击后取消勾选断言保留 | D3 必需的 fixture 迁移，不是放宽复选框断言 |
| `useIssueLifecycleGeneration.plan-approval.test.ts:246-270` | manual 请求 prepare 一次、不 PUT enrollment、checkbox 原值、无 plan_approval／automation_mode wire 键 | 原三 commit 未修改；本次读取时完整文件仍与 `45f2b588` 一致 | D3 要求保留；不能以「manual 分支下线」为由删除或改弱；自动化用例 `:202-244,272-303` 同样未动 |
| `IssueLifecycleWorkbench.generation.test.tsx` 原 `:590-614` → 当前 `:599-623` | 未确认 design：无自动化 radio，有「创建并打开 Workspace」 | 完整 it 用例逐字相同，仅新增 helper 使行号下移 | 受保护用例原文不可改；字节提取比较结果为一致，P0=0 |

### 5.1 `5a3323e2` 的集成测试迁移复审

原实现没有同步工作台层仍以 confirmed design 查找人工提交按钮的用例，这是当时真实的迁移遗漏，现记录为已修复 P1（§8 的 F1）。后续提交只改三份测试，没有更改产品 UI。

| 文件与当前行号 | 迁移内容 | 断言保留情况 |
|---|---|---|
| `IssueLifecycleWorkbench.generation.test.tsx:139-205` | 用例标题改成未确认 design 的阶段面板入口；使用 draft＋空 plan fixture，经 helper 打开 | 原完整 prepare body、workspace 回调、组卡／抽屉断言保留 |
| 同文件 `:270-310` | 默认拆分选项用例改 draft＋空 plan，走阶段面板 | 原请求次数／方法、body、默认字段断言保留 |
| 同文件 `:312-353` | 勾选拆分选项用例改同一入口 | E2E／确认位、workspace 回调及 payload 断言保留 |
| 同文件 `:726-757` | provider 显式选择用例改同一入口 | author_provider／reviewer_provider 断言保留 |
| 同文件 `:759-787` | 无 provider 默认值用例改同一入口 | 两个空值选择器、prepare 请求、两个字段缺省断言保留 |
| 同文件 `:789-815` | provider_unavailable 用例改 draft fixture 与入口 | 原错误文案、弹窗保持打开、未打开 workspace 断言保留 |
| `IssueLifecycleWorkbench.test.tsx:506` | 空面板人工 prepare 用例只加 draft fixture | 所有后续断言未改 |
| `IssueLifecycleWorkbench.drawer.test.tsx:384` | 删除整组再生成人工路径用例只加 draft fixture | 所有后续断言未改 |

复审读取了完整差异，并以只读文本提取比较所有 expect 语句：generation 79→79、base workbench 65→65、drawer 120→120，内容逐项一致；首条改名用例的 expect 也一致。该比较只是辅助证据，fixture／入口的语义仍逐处人工核对。

**弱化／放宽既有断言：本次审查范围内为 0。** 人工请求测试的上下文由 confirmed 迁移到保留的 manual 路由是 D1/D3 语义变化所需；这些测试仍只证明前端路由，不能声称绕过后端 design 确认前置。

## 6. 新测试强度与变异推演

`WorkItemPlanOptionsDialog.manual-mode-removed.test.tsx` 为 155 行，当前与 `06427ebe` 完整一致。

| 新用例／断言 | 能捕获什么 | 推演结论与边界 |
|---|---|---|
| 第一条 `:105-124`：无 manual radio、auto checked、启用自动化按钮存在、人工按钮不存在、target 投影可见 | 直接复活「手动」radio；恢复默认 manual；移除 auto 控件／改错提交文案 | **把实现改回有「手动」radio，第一条会红**，`queryByRole(...手动...)` 不再为空；这是源码级变异推演，未实际改代码或运行 |
| 第二条 `:127-153`：目标 422、告警、重试可用、submit disabled、无人工按钮／manual radio、callback 零调用、fetch 无 prepare、取消可用 | 目标失败后出现原名人工兜底；放开 submit；删除重试／取消；挂载时发 prepare | **把「创建并打开 Workspace」兜底按钮加回来，第二条会红**，`:147-148` 的不存在断言失败；无论按钮是否 disabled，原名按钮都会被查询命中 |
| fetch 分派 `:59-80` | 目标／enrollment 以外请求会抛错；配合调用记录断言直接检测 prepare | 不依赖「抛错是否被组件 catch」来证明没有 prepare，最终仍检查 fetch 调用记录 |

边界：第二条没有点击禁用 submit、重试或取消，也没有提交原生 form 事件，因此它证明当前渲染与已发生请求，不独立覆盖所有交互守卫；成功场景第一条没有点击提交，不能单凭这两条证明 mode 自动归一化或 enrollment 已绑定 plan/session。组件 `:226-238` 的守卫与归一化已静态核对，既有工作台 `generation.test.tsx:382-458` 覆盖 enable PUT、无 prepare、无 workspace 回调，target 测试 `:176-217` 覆盖点击重试成功。未把这些限制提升为新缺陷，也未虚报新测试已实跑。

## 7. 契约与实现逐条对应

delta 指 `openspec/changes/work-item-group-manual-mode-removed/specs/work-item-group-autopilot/spec.md`。

| 契约 | 实现／消费落点 | 测试／审查证据 | 结论 |
|---|---|---|---|
| `:4` 工作项组生成路径限定已确认 design | Workbench `:980-984`；弹窗 `:88` | draft 保护用例当前 `:599-623` | 一致 |
| `:8-9` 路径内不存在「未选自动化」 | 弹窗 `:88,237,349-351` | 新测试 `:114-116`；hook automatic 消费 `:633-650` | 一致；路径外默认 manual 仍保留 |
| `:13-14` 无「手动」模式选项 | 弹窗 `:339-357` | 新测试 `:110-112,143-145` | 一致 |
| `:15` 提交走 enrollment／plan-session 托管链 | hook `:434-506` enable PUT；后续既有编排器负责绑定／推进 | 工作台 `:406-453` 检查 PUT、精确 target/source；本次不声称已运行编排器 | 前端接线一致；服务端端到端绑定非本次运行验证 |
| `:16` 不改人工 workspace、不静默降级 | 弹窗 `:237`；hook `:634-650` 先返回，不落 `:653` manual 分支 | 工作台 `:412-417` 的无 prepare／无 workspace 回调 | 一致 |
| `:17` SHALL NOT 出现「创建并打开 Workspace」 | 弹窗 `:527` 仅 automatic=false 才渲染该字面值 | 新测试 `:120-122,146-148` | 一致，不是禁掉未确认 design 的原按钮 |
| `:21-24` 目标不可得如实告知、禁用、重试、可取消 | 弹窗 `:155-160,226-228,382-398,441-448,513-527` | 新测试 `:133-153`；target `:176-217` | 一致；重新开启复用旧 target 的既有行为不属于人工降级 |
| `:24` SHALL NOT 发起人工 prepare | 弹窗无 prepare 调用，handler 阻断且 mode 归一化；hook automatic 分支返回 | 新测试 `:149-152`；hook `:633-665` 消费分支 | 一致 |
| `:28-30` 路径外既有人工行为与人工门不变 | hook manual `:653-665`、入口口径、其余产品面未改 | 原保护用例不变；hook manual `:246-270` 不变 | 一致；未确认 design 的后端拒绝也属于保留的既有行为 |
| `:74-85` manual 缺省／旧记录／确认前行为仍按原约束 | 默认 manual `:52,245`；自动化确认守卫 `:451-453`；后端确认守卫未改 | 字节比较及人工路由测试 | 一致，不应误删兼容人工链路 |
| 主 spec `work-item-group-autopilot/spec.md:9-15` 旧「未选默认关闭」 | 本 change 以 MODIFIED delta 明确覆盖路径内默认，其余路径保留 | delta `:6-9` | 是显式契约变更，不是实现暗违旧主 spec；本次未 sync/archive |
| 路由主 spec `web-runtime-repository-routing/spec.md:165-167` 共用 authority、fail-closed | `src/web/handlers/automation_target.rs:84-91,115-127` 解析 carrier／角色链，前端不以人工兜底逃逸 | 原三 commit 后端零变更；target 错误与载体禁用审查 | 未引入第二 authority 或降级路径 |

其余身份持久化、集合规范化、重绑版本、服务端编排与人工最终确认条款沿用已有实现；本次没有对应后端改动，不将「没改」表述为已独立完成全部既有需求的全链验证。

## 8. P0／P1／P2 分级与闭环

### 8.1 未闭合清单

| 等级 | 当前未闭合数量 | 结论 |
|---|---|---|
| P0 | **0** | 受保护用例原文未动，无本 patch 引入的发布阻断级产品缺陷证据 |
| P1 | **0** | F1 已由后续提交修订；运行结果来源另列 |
| P2 | **0** | F2／F3 的矛盾及错误灰区口径已在本次读取的 proposal/tasks 工作树修正；须由 controller 纳入最终交付 |

这里的闭环结论明确包含已收到授权复审的后续修订；**若只交原三 commit 而排除 `5a3323e2` 与文字修订，则状态为「需修」**，不得套用当前综合状态。

### 8.2 F1 — P1：同步迁移依赖旧人工按钮的工作台集成测试（已修复）

- **问题**：原 `45f2b588` 令已确认 design 的弹窗默认自动化，但工作台层的旧用例继续从 confirmed design 查找「创建并打开 Workspace」，产生必然的 DOM 查询冲突。
- **证据**：patch 锚点 `WorkItemPlanOptionsDialog.tsx:86-89`；原 `45f2b588:IssueLifecycleWorkbench.generation.test.tsx:140,273,321,733,759,789` 六处查询人工按钮，默认 fixture 的 confirmed 事实见 `IssueLifecycleWorkbench.test-data.ts:232-241`；另两条工作台用例同依赖人工入口。`5a3323e2` 的实际 diff 已审。
- **影响**：原提交的集成回归测试不能按新语义通过；若只跑弹窗窄测试会漏掉调用侧迁移。
- **建议修法**：保留人工参数／回调断言，将人工路径用例迁至 D3 的 draft＋空 plan fixture，并用阶段空面板入口；已确认 design 的 enrollment 用例继续保留。
- **建议落点**：`IssueLifecycleWorkbench.generation.test.tsx:32-45,139,270,312,726,759,789`；`IssueLifecycleWorkbench.test.tsx:506`；`IssueLifecycleWorkbench.drawer.test.tsx:384`。
- **状态**：已由 `5a3323e2` 修复并验证；本审查独立核实 diff 与断言保留，运行验证由 controller 提供：修前 8 failed／63 passed，修后定向 71／71，全量 268 文件／2735 用例通过，tsc EXIT=0。**这些数字不是本审查亲自运行所得。**

### 8.3 F2 — P2：按 D3 保留 manual hook 用例，撤销「分支下线」任务（已修正文案）

- **问题**：`633e0217` 一方面保留 manual 分支，另一方面仍把 hook manual 用例称为「前提消失／manual 分支下线后改写」，会指导后续实施者删除合法回归保障。
- **证据**：原提交 patch 锚点 `tasks.md:34` 写「保留」，同版 `tasks.md:42` 却写「manual 分支下线后改写」；`proposal.md:57` 与 `:65` 同样冲突。合法人工分支确实仍在 hook `:653-665`，对应测试 `:246-270` 仍有作用。
- **影响**：契约／任务误导可能造成 D3 人工分支的测试被删或弱化；不是新的运行时产品故障。
- **建议修法**：统一为「D3 保留 manual prepare，断言不删不弱化；必要时只补充用例前提说明」，不要删除 import、分支或 prepare 负载断言。
- **建议落点**：`openspec/changes/work-item-group-manual-mode-removed/proposal.md:65`、`tasks.md:42`；hook 测试 `:246-270` 仅需准确说明覆盖边界，不得伪称它实际运行了后端。
- **状态**：原问题保留审查记录；本次读取的工作树两处已明确写 D3 保留及断言不删不弱化，静态关闭。该文字修订为**并发在途**，本审查未提交，controller 需保证最终交付包含它。

### 8.4 F3 — P2：撤回「未确认 design 已能成功手动产组」的无条件结论（已修正文案）

- **问题**：`633e0217` 新增灰区说明把 plan→group 投影直接推导为真实未确认 design 可成功人工产组，遗漏后端确认守卫。
- **证据**：原提交 patch 锚点 `proposal.md:98-100`；实际 `plan_preparation.rs:73-74` 调用确认校验，`lifecycle_aggregate_scope.inc.rs:94-98` 在创建 plan 之前拒绝未确认 design。
- **影响**：owner 可能被错误告知当前存在一个可操作的手动绕过，或把既有 `design_spec_not_confirmed` 当作⑤回归，造成错误验收与范围决策。
- **建议修法**：分别陈述前端路由、后端阻断、现存 plan 投影三层事实，灰区改成「条件性、当前该路径不可达」，保留 owner 知情确认。
- **建议落点**：`proposal.md` 灰区章节（当前 `:98-108`）、controller 交付口径及本报告 §4。
- **状态**：本次读取的工作树已改为条件性灰区并明确废止前述版本；controller 已书面认可该定性，静态关闭。该文字修订为**并发在途**，没有据此声称已产生新提交。

## 9. 需 owner 知情确认与未决问题

### 9.1 条件性灰区：严重性低于原描述

确认范围是「已确认 design 的本弹窗」而不是全系统清除所有 manual API／旧人工组。store 的 plan→group 规则不区分托管；但真实未确认 design 在创建前被拒绝，因此不能宣称当前该弹窗的 draft 提交已成功产组。**owner 应以 §4 的修正版为准，controller 先前的无条件表述作废。** 若未来放开确认前置或新增人工产组入口，应重新评审 owner 原话与路径定义的覆盖范围。

### 9.2 ① 根因未改与 D2 无兜底叠加

- `WorkItemPlanOptionsDialog.tsx:172` 仍调用 `getAutomationTarget(projectId, issueId, {})`；`:190-195` 的依赖只含 scope／retry，没有表单 provider。换 provider 后仍会读同一个空 query，点重试也不会把所选 provider 带上。
- `web/src/api/client.ts:571-596` 仅在 query 有键时附 provider 参数；空 query 无参数。
- `src/web/handlers/automation_target.rs:47-59,90-91,120-127` 用缺省配置解析并做真实角色链预检；`src/web/handlers/support.rs:378-390` 的 author 默认仍是 `"codex"`，reviewer 默认是 `"claude_code"`。因此该 GET 不是完全与 provider 无关的纯身份读取。
- 真正启用时 hook `:434-441` 会传实际选项，但如果弹窗早期默认 provider 的预检失败，submit 在 `:155-160,524` 已被禁用，无法走到那一步。
- **⑤ 落地后若①在别的机器复现，用户在弹窗里没有出路——精确指不能完成任何提交；仍可点「重试」或「取消」退出，不是无法关闭弹窗。** 需要在弹窗外修复 provider／载体配置再返回；本审查不替 owner 决定恢复人工兜底或修①。
- ⑧ 的预检错误详情丢失也未改变：弹窗 `:178-184` 只保留 message，`:388` 只展示该 message；与提交错误 `:245-246,492-507` 的逐角色信息不是同一错误入口。owner「先不管」来自派工裁定，不冒充源码能证明 owner 决策。
- 以上是既有根因与已批准 D2 叠加的残余风险，不作为本 patch 新增 bug 再报 P1；具体机器是否触发没有本次运行证据。

### 9.3 未决问题

1. owner 是否接受未来一旦放开 design 确认前置，仍需重新审查 manual API／plan→group 的条件性灰区？当前不扩大⑤范围。
2. owner 是否在知晓「空 query 默认 provider 失败 → 无法提交且只有重试／取消」后继续维持①不改、⑧先不管、D2 无人工兜底？本报告只要求知情，不重开已授权 oracle 的 D1/D2/D3。
3. controller 的最终交付需包含 `5a3323e2` 及并发在途的文案修订，并按项目规则补 owner 页面验收／发布资源验收；本次只读审查没有执行这些工作，不能把报告状态当作发布签字。

## 10. 越界、一致性与行数

- 原三 commit 精确并集为 8 个文件：3 份 change 文档、1 份新测试、3 份既有弹窗测试、1 份弹窗产品文件；`src/**`、主 spec、`useIssueLifecycleGeneration.ts`、`IssueLifecycleWorkbench.tsx`、`ChatCockpitPage.tsx` 均不在变更清单。
- 已以只读字节对比确认上述三份禁改产品文件相对红测前完整不变；后端 `automation_mode` 检索零命中。保留 manual 默认及类型不是死代码：未确认 design 的回调路径仍消费它，旧人工端点也是明确非目标。
- 弹窗只删除「手动」radio 对应 UI 文字，其余按钮／label 的字面值未改；「启用自动化」「创建并打开 Workspace」仍共用原条件表达式 `:527`，变化在 automatic 的判定，不是改文案。
- `5a3323e2` 只改三份工作台测试的 fixture／入口／一条测试标题／helper；未更改 UI 文案。所有引用这些文件的修订状态均来自**并发在途**工作随后形成的明确提交，不将当时未提交状态误归本审查所有。
- `WorkItemPlanOptionsDialog.tsx` 实际 680 行，门禁 1200，余量 **520 行**；新测试 155 行。后续工作台三测试为 923／837／1143 行，余量分别 277／363／57 行。
- 没有运行全仓行数扫描，只确认本次已读取／统计文件的静态行数；未把它冒充全仓 guard 执行结果。
- Monaco 并发文件、排期 notes 及其他 worker 改动没有作为⑤产品正确性的证据，也未写入、回退或暂存。

## 11. 诚实边界与证据来源

### 11.1 本审查亲自完成的代码级确证

- 逐个读取原三 commit 和 `5a3323e2` 的 diff；读取已修改文件上下文与调用／消费侧。
- 读取 delta、proposal、tasks、REQ-WIGA-01 主 spec 及 routing authority 条款，逐条映射。
- 使用 CodeGraph 追踪 onConfirm→hook→enrollment／prepare、provider 缺省解析、plan 投影及后端确认校验。
- 只读检索 automation_mode 写入点，核实禁改面与后端零命中。
- 通过只读文本／字节比较确认保护用例原文、相关产品文件不变，以及后续测试 expect 语句没有被删除或放宽；静态统计相关文件行数。
- 唯一写入为本报告，无产品改动、无 git add、无 commit、无 cargo／npm／pnpm 命令、无构建／测试／格式化／服务变更。

### 11.2 推演而非实测

- 「恢复手动 radio 会使第一条红」「恢复原名兜底按钮会使第二条红」是依据实际查询与断言做的变异推演；未修改实现、未运行变异测试。
- 「若默认 codex 角色链预检在其它机器失败，则该弹窗无法完成提交」是已核实代码链的条件推论；没有在该机器复现，也未估计发生概率。
- 「未确认 design 的成功人工产组当前不可达」是后端守卫与创建顺序的静态证据，不是 curl／浏览器实测；不否认保留的合法人工 API 或历史组。

### 11.3 外部提供的验证结果

controller 通知：原相关用例 8 failed／63 passed；`5a3323e2` 修后定向 71／71；全量 268 文件／2735 用例通过；`pnpm tsc -b` EXIT=0。本审查没有重跑，也没有读取相应完整日志，不把这些数字算作独立运行证据。它们与本次独立检查到的 fixture／入口迁移一致。

本报告不声称已完成新增测试的真实红→绿复现、全量 lint、OpenSpec strict validate、服务端编排 plan/session 的真实绑定、owner 真机操作或发布资源指纹核验；这些由 controller 的验证与交付链提供。

## 12. 最终结论与 concerns

- **当前综合代码审查：可交付；未闭合 P0=0、P1=0、P2=0。** 原三 commit 单独交付仍有集成测试迁移与文案问题，不能遗漏已复审的修订。
- **两项必须向 owner 明示**：条件性灰区（当前该未确认 design 产组路径不可达，旧表述作废）；① 空 query／默认 codex 未改，D2 下故障复现会导致弹窗内无法提交。
- **非本次新缺陷**：draft 抽屉原本无「生成 Work Item」；真实 draft prepare 被 `design_spec_not_confirmed` 拒绝；旧人工 prepare API 与现存人工组仍保留。
- **交付 concern**：文字修订为并发在途，controller 应保证落入最终提交集；测试运行证据由 controller 提供，owner 真机与发布验收不由本报告替代。
- **操作边界**：本审查未改其它文件，未运行 cargo／npm／pnpm，未暂存或提交；报告由 controller 接收并决定纳入交付。

## 补充：6.4 真机未验说明（2026-11-08）

★**6.4（目标投影不可得 ⇒ 界面如实告知并给出可执行下一步）本轮真机未验**（本地实例无该场景）⇒ 与 `openspec/changes/archive/2026-11-07-work-item-group-manual-mode-removed/tasks.md` 的保留未勾一致。本报告此前的覆盖结论仅限单测层面。

★**编号限定语（2026-11-08 补充）**：上句的「6.4」= ★**⑤ 的 6.4**★（`work-item-group-manual-mode-removed` tasks 6.4：目标投影不可得 ⇒ 界面如实告知并给出可执行下一步）★；验收卡 `cadence/reports/2026-11-07_验收卡_bug1与八条问题复验_v1.0.md` **§B 的 6.4**（bug1「被取消的 provider run 节点显示中止」）是**另一处** ⇒ ★**两处编号不同源、互不冲突**，见该卡 §B 6.4 / §B.1 与 §D 5.2 / 5.4★。
