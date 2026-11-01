# UI 修复报告：部分交付区与 Work Item 区默认收起 + 仓卡去重

- 日期：2026-10-14
- 分支：`feat-b-0808-add-monorepo`（worktree）
- 提交：`03a253a1` `fix: 部分交付与WorkItem区默认收起`
- 范围：`web/src`（7 个文件，6 改 1 增）；未部署

## 1. owner 直批的三条改动

| # | 要求 | 实现结果 |
|---|---|---|
| 1 | 「部分交付」区 → 可折叠抽屉，默认收起 | `DeliveryStatusPanel` 受控折叠，首次进入收起；收起态保留状态徽标 + 摘要「4 个仓库 · 5 个 Work Item · 未推送 1」+「展开」按钮 |
| 2 | 「Work Item」区（4 个仓库分组面板）→ 可折叠抽屉，默认收起 | `WorkItemRepositoryGroupSection` 折叠仓库分组明细；**plan 分组卡常驻可见**，同时给出常驻展开提示与「展开 N 个仓库分组」按钮（owner 的「或」两条同时满足） |
| 3 | 仓卡列表 frontend 重复两次（5 WI 按 target 仓展开）→ 去重为 4 个唯一仓 | 交付条目按 `repository_name` 去重；多 WI 仓卡加副行「2 个 Work Item · 已推送 1 · 未推送 1」，聚合状态取最严重者（失败 > 未推送 > 已推送） |

会话内保持：展开/收起状态由 `IssueLifecycleDetail` 持有（跨阶段切换、跨 Issue 切换都不重置），按 owner 要求**不落盘**（与「折叠 Issue 队列」的跨会话 localStorage 持久化刻意区分）。

## 2. 设计决策（需 controller 向 owner 确认口径）

1. **只折叠「仓库分组」渲染**：折叠加在 `WorkItemRepositoryGroupSection`（即「4 个仓库分组面板」）。`work_item_repository_groups` 为空时的扁平兼容路径（`LifecycleContentSection`，单仓/旧响应）保持原样展开——该路径没有仓库分组面板可折，且与 Story/Design 共用同一组件；强行折叠会引入第二套折叠约定并波及无关用例。已由测试 `单仓扁平兼容路径（无仓库分组）保持既有展开形态，不引入第二套折叠约定` 锁定。
2. **plan 卡常驻**：收起时仍渲染区顶 plan 分组卡（主要交互入口），并附带折叠提示「已收起 N 个仓库分组（M 个 Work Item）· 展开查看各仓库明细」。
3. **onboarding 锚点保活**：`onboarding-anchor-plan-final-gate` 由原先只挂在分组列表/空态上，改挂在**始终渲染**的内容体容器上（收起态即提示块）。引导卡在收起态仍能定位到计划最终确认区，不会退化为「锚点缺失→底部回退」。
4. **抽屉不折叠**：「查看完整 Issue」抽屉内的交付面板不传折叠 props → 不渲染折叠控件、常驻展开（显式详情面保持原样，现有抽屉用例零改动）。
5. 折叠控件抽为共享组件 `SectionCollapseToggle`（图标 + 文案 + `aria-expanded`/`aria-label`），避免两区各写一套展开约定。

## 3. 文件清单

- 新增 `web/src/components/lifecycle/SectionCollapseToggle.tsx`
- 改 `web/src/components/lifecycle/DeliveryStatusPanel.tsx`（去重 + 聚合状态 + 折叠 + 收起摘要；新增导出 `groupDeliveryEntriesByRepository`）
- 改 `web/src/components/lifecycle/IssueLifecycleWorkbenchParts.tsx`（详情容器折叠状态 + 两处接线；`WorkItemRepositoryGroupSection` 折叠重构）
- 测试：`DeliveryStatusPanel.test.tsx`（+4）、`IssueLifecycleWorkbenchParts.test.tsx`（+4，另 3 处旧用例补展开点击）、`IssueLifecycleWorkbench.drawer.test.tsx`（2 处补展开点击）、`IssueLifecycleWorkbench.single-coding.test.tsx`（+`openSingleWorkItemDrawer` helper，4 处点击收敛）

## 4. 验证证据（本机实测）

| 检查 | 命令 | 结果 |
|---|---|---|
| 新增/相关族 | `npx vitest --run src/components/lifecycle/IssueLifecycleWorkbenchParts.test.tsx` | 15 passed / 15 |
| 交付面板 | `npx vitest --run src/components/lifecycle/DeliveryStatusPanel.test.tsx` | 8 passed / 8 |
| 单 WI 编码 | `npx vitest --run src/components/lifecycle/IssueLifecycleWorkbench.single-coding.test.tsx` | 8 passed / 8 |
| 全量 | `npx vitest --run`（web） | **213 files / 2160 tests 全绿** |
| 构建 | `npm run build`（`tsc -b && vite build`） | 成功（仅既有 chunk >500 kB 提示） |

TDD 过程：新增测试先行跑红（交付面板 4 红）→ 实现 → 转绿；再全量跑出 11 处因「默认收起」而失效的旧分组用例 → 逐处补「先展开」步骤（非放宽断言）。

## 5. controller 浏览器复验步骤

1. 打开工作台该 issue 详情页（有 4 个仓库分组 + delivery_summary 的样本）。
2. 预期（首屏，未点击）：交付区只剩徽标「部分交付」+ 摘要「4 个仓库 · 5 个 Work Item · 未推送 1」+「展开」按钮；Work Item 区只剩表头（`4 个仓库分组`）+ plan 分组卡 + 提示「已收起 4 个仓库分组（5 个 Work Item）· 展开查看各仓库明细」；页面明显变短。
3. 点交付区「展开」→ 出现 4 张仓卡（busi/api/gateway/frontend），frontend 仅 1 张且副行写「2 个 Work Item · 已推送 1 · 未推送 1」，无重复 frontend 卡。
4. 点 Work Item 区「展开」→ 出现 4 个仓库分组面板；切到 Story 或 Design 阶段再切回 Work Item，仍保持展开（会话内保持）。
5. 再点「收起」两区恢复收起态；对照首屏无回归（徽标、plan 卡、提示均在）。

## 6. 关注点

- 未做浏览器验证（controller 统一），上述 4 个验收点均为代码侧 + 单测侧证据。
- 决策 1（扁平兼容路径不折叠）与决策 3（收起态锚点落在提示块）为有意取舍，若 owner 希望扁平路径一并折叠或引导卡强制展开该区，需另行拍板。
- 折叠状态不跨会话持久化（按 owner 要求）；刷新后回到收起态属预期。
