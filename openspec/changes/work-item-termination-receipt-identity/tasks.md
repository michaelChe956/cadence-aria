# Tasks: 终止回执的身份限定与换代失效（REQ-WIGA-14）

## 0. 接手前必读
- `proposal.md`（现象 / 根因 / 审计判定）
- `cadence/reports/2026-11-06_诊断_终止状态跨workitem泄漏_v1.0.md`（诊断）
- `cadence/reports/2026-11-06_审计_删除workitem与自动化enrollment_设计vs实施_v1.0.md`（设计 vs 实施审计）
- 契约：本 change 的 ADDED delta `openspec/changes/work-item-termination-receipt-identity/specs/work-item-group-autopilot/spec.md` 的 REQ-WIGA-14
  ★主 specs 中尚无 REQ-WIGA-14（尚未 archive/sync）⇒ 引用一律指 delta★

## 1. 回执带身份
- [x] 1.1 `AutomationTerminationResult` 增加 `planId`（**REST 响应已含** ⇒ 零后端改动）；`client.ts:843` 解析时带上
- [x] 1.2 `coding-workspace-store.ts` 的 `setResult` 与持久化同步带上 `planId`；回执类型与存储形状更新
- [x] 1.3 **缺 `planId` 的旧回执**（历史响应 / 旧缓存 / 会话回放）⇒ 保持可读但标记为「无身份」⇒ 由展示层 fail-closed（见 2.2）

## 2. 展示守卫加身份比对
- [x] 2.1 `CodingWorkspacePage.tsx:176-181` 与 `ChatCockpitPage.tsx:854-860`：渲染 `AutomationTerminatedBanner` 前校验回执 `planId` 与**当前** plan/session 身份一致；不一致 ⇒ 不渲染
- [x] 2.2 **fail-closed**：回执缺 `planId` ⇒ 不渲染（不得据此推断当前计划已终止）；**不得**弱化既有终止文案与 `cleanup_required` 的管理员介入说明
- [x] 2.3 换代（supersede）后旧代回执不参与任何判定（不依赖 `reset()` 的 `[attemptId, issueId, projectId]` 不变这一偶然条件）

## 3. 回归测试
- [x] 3.1 `ChatCockpitPage.automation-termination.test.tsx`：同 issue 的旧回执（`planId` = 旧）+ 当前是新 plan ⇒ **横幅不渲染**
- [x] 3.2 `CockpitInboxWaitingCards.termination.test.tsx`：保留既有断言；补「回执缺 `planId` ⇒ 不渲染」
- [x] 3.3 `coding-workspace-store.test.ts`：`setResult` 带 `planId` 并可读回；旧回执（无 `planId`）标记为无身份

## 4. 真机验收（★这次必须执行，不留未勾★）

> ★**2026-11-06 状态：4.1 / 4.2 / 4.3 均【尚未执行】★**
> 现有证据等级 = ① 单元测试（4 个测试文件 / 62 用例）② controller 浏览器复核（同一会话页「自动化已终止」横幅 NOT PRESENT + 5 节点全完成 + console 无错）③ owner 真机截图（横幅时间早于本轮开始 79 秒 ⇒ 确证为上一轮遗留）
> ⇒ **真机 9.2 / 9.3 / 7.8 仍是未勾项**，已在阶段 1 测试矩阵中列为 owner 必测 ⇒ 测完回来勾。
> 矩阵落点：`cadence/notes/2026-11-06_测试矩阵_阶段1全流程_v1.0.md`
>   - **B6**（终止整个自动化 → 终止横幅可见）≈ 9.2 前半
>   - **B8**（终止态停留本会话：横幅仍在、入口已撤）≈ 9.2 后半
>   - **B7**（删除该 Work Item → 同 issue 新建 → 旧横幅不得出现）= **9.3**
>   - **B22**（门卡刷新：当前卡重建 / 重连刷新同门当前事实）≈ **7.8**
> ⇒ 矩阵已实测修正过一次导航错误（B3 第 2 步：入口在 Work Item 阶段区，不在 Design 区）。
- [ ] 4.1 **9.2**：终止整个自动化 → 同一计划重新启用 → 走通（终止横幅出现、重新启用后可继续）
- [ ] 4.2 **9.3**：终止整个自动化 → 删除该 work item → **同一 issue 新建** ⇒ ★ 新建者**不显示**旧终止横幅；且其流程正常推进（不被横幅误读为卡住）
- [ ] 4.3 **7.8**：旧卡只读历史时，终止横幅与历史记录**不串味**
- [ ] 4.4 后端护栏（可选加固）：`admission_fixture_tests.rs` 补「supersede 后新 plan 的 attempt 门控必返 None」

## 5. 验收纪律
- [x] 5.1 动态验收**退出码 0 不算通过**：须同时满足 服务未 abort + 可信 provider-start 探针 + 完整帧 + durable 进展 + 取消后能再生成
- [x] 5.2 既有断言**一条不许删/缩窄**；终止横幅的既有文案与语义逐字保持
