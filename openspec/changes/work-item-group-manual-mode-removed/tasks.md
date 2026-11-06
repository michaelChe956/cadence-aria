# Tasks: 工作项组不再提供手动生成模式（REQ-WIGA-01 MODIFIED）

> 契约：本 change 的 MODIFIED delta `openspec/changes/work-item-group-manual-mode-removed/specs/work-item-group-autopilot/spec.md` 的 REQ-WIGA-01（主 spec 尚未 archive/sync ⇒ 引用一律指 delta）
> 设计依据：`proposal.md`（现象 / 现状落点 / 两种粒度 / 推荐与理由 / 终裁项 D1–D3）
> ★**粒度已由 oracle 终裁（owner 已授权 oracle 决策）：D1 = ①（整个 fieldset 固定 automatic，范围限定 `automationAvailable === true` 的弹窗）／D2 = (b)（不保留任何「创建并打开 Workspace」兜底）／D3 = 保持现状（未确认 design 继续走人工路径，`prepareWorkItemPlan` 保留）**——详见 `proposal.md`。本 tasks 按**①** 展开。

## 0. 接手前必读

- [ ] 0.1 `proposal.md`（含现状落点表与「两条决定粒度的关键事实」）
- [ ] 0.2 契约 delta（本 change 的 `specs/work-item-group-autopilot/spec.md`）——重点读三个新场景：
      「工作项组生成路径不提供手动模式」「托管目标不可得时不得静默降级」「工作项组生成路径之外的既有行为逐字不变」
- [ ] 0.3 ★**D1／D2／D3 已由 oracle 终裁（owner 已授权 oracle 决策）**：D1 = ①（范围限定 `automationAvailable === true`）／D2 = (b)（不保留任何「创建并打开 Workspace」兜底）／D3 = 保持现状——**不再需要 owner 拍板**，可直接进入 §2 的实现（终裁依据见 `proposal.md`）

## 1. 红测先行（★第一步必须是失败测试★）

- [ ] 1.1 **新建** `web/src/components/lifecycle/WorkItemPlanOptionsDialog.manual-mode-removed.test.tsx`（渲染写法照 `WorkItemPlanOptionsDialog.voided.test.tsx:119-125`：`render(<WorkItemPlanOptionsDialog defaultOptions={BASE_OPTIONS} automationAvailable automationTargetScope={SCOPE} onConfirm={...} onClose={...} />)`，弹窗取 `screen.getByRole("dialog", { name: "Work Item Plan 配置" })`）：
      - **红测断言（核心）**：`expect(within(dialog).queryByRole("radio", { name: "手动" })).not.toBeInTheDocument();`
      - 配套断言：`expect(within(dialog).getByRole("radio", { name: "自动化" })).toBeChecked();`
      - 配套断言（提交形态）：`expect(within(dialog).getByRole("button", { name: "启用自动化" })).toBeInTheDocument();`
      - ★**先见红**：实现前「手动」radio 仍渲染 ⇒ 第一条断言失败；把失败输出贴进交付报告
- [ ] 1.2 **回归锚点用例**（★本条**逐字不改**，作为「非工作项组路径行为不变」的证据）：
      `web/src/components/lifecycle/IssueLifecycleWorkbench.generation.test.tsx:590-612`（未确认 design ⇒「自动化」radio 不存在、且「创建并打开 Workspace」按钮存在）
- [ ] 1.3 记录既有引用「手动」radio 的 4 处断言**基线全绿**（改写前）：
      `WorkItemPlanOptionsDialog.reenable.test.tsx:189`、`WorkItemPlanOptionsDialog.target.test.tsx:246-250`、`WorkItemPlanOptionsDialog.plan-approval.test.tsx:110-115`、`useIssueLifecycleGeneration.plan-approval.test.ts:246-270`
- [ ] 1.4 纪律：红测单独一次提交（先红后绿可追溯），再进入 §2

## 2. 实现

- [ ] 2.0 ★**粒度已终裁：D1 = ①（范围限定 `automationAvailable === true` 的弹窗），D2 = (b)，D3 = 保持现状（详见 `proposal.md`）**★
      - 按 ① ⇒ 按 2.1–2.5 执行
      - ★原「选 ②」分支已因 D1 终裁为 ① 而关闭★：若将来要 ②，须独立成一个后续 change——★前置插入一个独立工作包★：产出「本次提交是否生成 group」的新先验判据（新增 projection／新字段）并走契约评审；未完成前**不得**改弹窗
- [ ] 2.1 `web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx:333-361`：「手动」radio 不再渲染（① 下 fieldset 只保留「自动化」且呈固定托管态；提交按钮恒为「启用自动化」）
      - ★同步更新组件自陈注释 `:42-43`、`:60-61`（它们现在写着「缺省手动／人工链路不变」，与新行为矛盾）
- [ ] 2.2 `web/src/components/lifecycle/useIssueLifecycleGeneration.ts:632-665`：**保留** manual 提交分支与 `prepareWorkItemPlan` 引用——未确认 design 的提交仍走 manual（`:245` 默认值）并继续调用 `prepareWorkItemPlan`（`:653-665`；后端路由与 `web/src/api/client.ts:550` 导出一并不动，Rust 集成测试大量直接调用，删除没有对象）；唯一要求：manual 分支在 `automationAvailable === true` 的弹窗里**不可达**（弹窗已不提供「手动」选项，且 `onConfirm` 发出的 mode 已归一为 `automatic`）
- [ ] 2.3 ★**失败面（D2 已终裁为 (b)）落地**：目标投影读取失败／授权载体不受支持时（`WorkItemPlanOptionsDialog.tsx:151-155` 的 fail-closed），弹窗保持打开、提交保持禁用、目标投影读取失败时提供「重试」（`automation-target-retry`）、用户可「取消」退出，之后修复目标/载体问题再回来；不提供任何以人工推进 workspace 会话创建工作项组的替代提交（含「创建并打开 Workspace」），不发起人工 prepare 请求
      - MUST 满足 delta 场景「托管目标不可得时不得静默降级」（不静默转手动、不伪造托管状态）
- [ ] 2.4 `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx:980-984`：`automationAvailable` 口径①下**逐字不变**；仅 ② 才扩展
- [ ] 2.5 **既有断言改写（逐条，不弱化原意）**：
      - `WorkItemPlanOptionsDialog.reenable.test.tsx:189`（原「生成模式仍可切回手动」）⇒ 改为断言「手动 radio 不存在」
      - `WorkItemPlanOptionsDialog.target.test.tsx:246-250`（原「automatic GET 失败后切回 manual」）⇒ 按 2.3 的 D2 结论改写
      - `WorkItemPlanOptionsDialog.plan-approval.test.tsx:110-115`（原「切回手动：复选框恢复可用」）⇒ 按新行为改写
      - `useIssueLifecycleGeneration.plan-approval.test.ts:246-270`（原「manual 走 prepare」）⇒ D3 = 保持现状：manual 分支与 `prepareWorkItemPlan` **均保留**，断言本身不删不弱化；仅把用例前提改述为「未确认 design 的 manual 提交仍走 prepare」（该分支在 `automationAvailable === true` 时不可达，不再由弹窗「手动」radio 触发）
      - ★每条改写都要在交付报告里列出「原断言 → 新断言」
- [ ] 2.6 **后端零改动自检**：确认 `src/**` 无任何改动（`git diff --stat -- src` 空），且 `automation_mode` 在 `src/**` 仍零命中
- [ ] 2.7 **未确认 design 的弹窗（D3 = 保持现状）**：`automationAvailable=false` ⇒ fieldset 不渲染、提交仍为既有「创建并打开 Workspace」（manual 人工链路，见 2.2 的保留结论）⇒ 逐字不变；用 §1.2 的回归锚点证明

## 3. 定向验证

- [ ] 3.1 新用例：`cd web && pnpm vitest --run src/components/lifecycle/WorkItemPlanOptionsDialog.manual-mode-removed.test.tsx` ⇒ 全绿
- [ ] 3.2 弹窗与 hook 相关定向：`cd web && pnpm vitest --run src/components/lifecycle/WorkItemPlanOptionsDialog src/components/lifecycle/useIssueLifecycleGeneration src/components/lifecycle/IssueLifecycleWorkbench.generation` ⇒ 全绿
- [ ] 3.3 后端定向：本 change **零后端改动** ⇒ 无需跑 Rust 定向测试；若 §2 期间误碰 `src/**`，按 `flock /tmp/aria-cargo.lock cargo test --locked --lib <过滤名>` 补跑并说明

## 4. 全量门禁

- [ ] 4.1 前端测试：`cd web && pnpm test` ⇒ 全绿
- [ ] 4.2 类型检查：`cd web && pnpm tsc -b` ⇒ EXIT 0
- [ ] 4.3 前端 lint：`cd web && pnpm lint` ⇒ 无新增告警
- [ ] 4.4 Rust 门禁（后端零改动 ⇒ 仅确认不回归）：`flock /tmp/aria-cargo.lock cargo fmt --check` ＋ `flock /tmp/aria-cargo.lock cargo clippy --all-targets --all-features --locked -- -D warnings` ＋ `flock /tmp/aria-cargo.lock cargo test --locked --all-targets`（⛔ 禁 `-j 1`、⛔ 禁 `--test-threads=1`、⛔ 禁无过滤名的 `cargo test --locked <过滤名>`）
- [ ] 4.5 行数门禁：`MAX_PRODUCT_CODE_LINES = 1200`（扫描根 `src` / `tests` / `web/src`）⇒ 未超限
- [ ] 4.6 契约：`openspec validate work-item-group-manual-mode-removed --strict` ⇒ 通过

## 5. 独立审查

- [ ] 5.1 派 `k3-reviewer` 独立审查（重点：① 契约与实现是否一致；② 「不得静默降级」是否被违反；③ 既有断言改写是否弱化了原意；④ 后端是否真的零改动；⑤ 是否有死代码/第二入口残留）
- [ ] 5.2 逐条回应审查意见；未采纳的写明理由
- [ ] 5.3 交付报告落 `cadence/reports/`（含：owner 裁定原文、粒度终裁结果 D1/D2/D3、红测失败输出、改写断言逐条对照、门禁输出、未做项）

## 6. owner 真机验收（★这次必须执行，不留未勾★）

- [ ] 6.1 已确认 design 上打开「Work Item Plan 配置」弹窗 ⇒ ★界面上没有「手动」选项★，只有托管自动化
- [ ] 6.2 提交 ⇒ 产出确为**托管自动化**的工作项组（enrollment 已绑定 plan/session，界面呈现托管事实，驾驶舱可见托管态）
- [ ] 6.3 未确认 design 的弹窗 ⇒ 逐字不变（不呈现「生成模式」，仍有「创建并打开 Workspace」，提交仍走 manual 并调用 `prepareWorkItemPlan`）
- [ ] 6.4 目标投影不可得（422 / 载体不受支持）⇒ 界面如实告知并给出可执行下一步，★不静默转手动、不伪造托管状态★（按 D2 结论核对兜底形态）
- [ ] 6.5 托管链路的人工门 / AI choice / 最终确认仍在（REQ-WIGA-03 / REQ-WIGA-05 未被削弱）
