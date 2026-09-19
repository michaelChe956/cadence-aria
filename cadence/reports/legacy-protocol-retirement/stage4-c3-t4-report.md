# 阶段 4 C3 · T4 前端 legacy 消费面迁移（WP3 / REQ-RET-02 L1）报告

- **worktree**：`feat-b-0808-add-monorepo`；T4 提交区间 `3837ba75`（产码迁移）→ `7e64abc8`（T2，间插）→ `e404dfdc`（测试重钉+清单+e2e 退役/豁免基建）→ 本报告提交。
- **输入**：计划 v1.1 Task 4 五 Steps；T2 wire 契约（已落地 7e64abc8：`{type:"abandon_human_gate",command_id}`，空白 command_id 拒 INVALID_COMMAND_ID——前端 `newCommandId()`=crypto.randomUUID 恒非空，无风险）。

## 1. 交付摘要（对照计划 Step 1-5）

| Step | 结果 |
|---|---|
| S1 清单 | `wp3-frontend-inventory.md`：63 文件（src 61+e2e 2，HEAD 实跑口径）逐文件处置表（产码 22 条目/测试 39+e2e 2），含完成判定对照与 e2e 豁免登记 |
| S2 发送面 typed | `useWorkspaceWs`：`sendHumanConfirm` 删除，新增 `sendConfirmGate`（confirm 帧+审计）/`sendAbandonGate(commandId)`（abandon_human_gate 帧+审计+乐观 resolveGateEntry("terminate")）；operation 联合收敛（删 request_change/terminate 发送侧、增 abandon_gate）；死发送器 `sendRevertWorkItem`/`sendHumanPresentationRevision` 删除（union 变体留 T5）；`sendRequestRevision` 改返 boolean（regenerate 失败面）。`bulk-confirm-runner:127` → `{type:"confirm"}`。红→绿：`useWorkspaceWs.actions.test.tsx` 新增 abandon wire 形状/门卡乐观收敛/approve 不乐观收敛三断言（红测先行后实现） |
| S3 动作面/路由 | `cockpit-action-routing`：facade 输入切 `sendConfirm`/`sendAbandonGate`，`requestChange` 成员+`CockpitRequestChangePayload` 删除；`gateIdentityFromState` 删 `legacy:${stage}` 分支（gateId 派生收敛 turn/snapshot 两路），`selectGateProjection` 第三分支 fallback key 改 `stage:${stage}`（GatePromptEntry `stage:human_confirm` 字面量同步）。`useStageUI` human_confirm/review_decision actions 收敛 `[]`+StageAction 裁剪。`ChatInputBar`：onSendHumanDecision/humanConfirmDisabled prop 删、human_confirm 阶段输入禁用（无发送通道）、决策回调可选未提供即不渲染。`GatePromptEntry`/`CockpitInbox`：legacy 采纳返修钮+payload helpers 删（CockpitInbox :360 为清单外发现的产码消费点，一并处置）。`ChatWorkspacePageLegacy` OQ2 剥离：facade/ReviewDecisionActionBar/author 决策（含 ArtifactReviewPanel actions 插槽）/逐段 props/presentation 保存全删；compile recovery（SC compile 链 T5 保留面）经 `WorkItemPlanStagedPanel` 保留。`CodingWorkspacePage` regenerate → `sendRequestRevision`（WorkItemPlan 子会话入站即 `request_work_item_plan_revision`=SC plan-repair 通道）。`ChatCockpitPage` 三处 facade+deps 切 typed |
| S4 union/类型 | `workspace.ts` 删 `human_confirm` 分支+增 `{type:"abandon_human_gate";command_id:string}`（与 advance 对称）；`common.ts` `HumanConfirmDecision` 删除（引用清零后）；TimelineNodeType/stage 联合/labels/badges/`human_confirm_state` 等只读呈现面保留；`request_outline_revision` 等待 T5（清单标「待 T5」） |
| S5 验证 | 见 §2 |

组件只读化配套：`WorkItemPlanStagedPanel`（逐段回调可选+分支守卫，compile recovery 必选保留）、`WorkItemPlanCandidatePanel`（动作回调可选+按钮区守卫）、`WorkItemPlanArtifactPanel`/`WorkItemProjectionTabs`/`WorkItemPlanOverview`（presentation 编辑器随回调缺席隐藏——删两级默认 no-op 使守卫生效）。`CockpitOperation` 增 `abandon_gate`（request_change/terminate 留作审计历史读侧值，T5 收敛）。

## 2. 验证证据

| 门禁 | 命令 | 结果 |
|---|---|---|
| 定向红→绿 | `npx vitest --run`（分文件多轮） | actions/plan-repair-actions/projection/facade/bulk/ChatInputBar/GatePromptEntry/p1-entries/CockpitInbox/cockpit 页面/legacy 页面/work-item-plan/review/coding plan-repair 全绿 |
| tsc | `npx tsc -b --force` | **0 error**（产码+测试） |
| 前端全量 | `npm test`（vitest --run） | **1489/1489 passed**（175 文件） |
| 构建 | `npm run build` | 成功（tsc -b+vite ✓，2.59s） |
| grep 断言 | `grep -rnE '[^_]type: "human_confirm"|sendHumanConfirm|"decision": "(request-change|terminate)"' src e2e --include="*.ts" --include="*.tsx" \| grep -v ".test."` | **exit=1 零命中**（注：计划原 pattern 会子串误中 `node_type: "human_confirm"` 只读夹具——精化 `[^_]type:` 后零命中；该夹具=服务端 timeline 节点类型呈现，清单 #51 保留只读） |
| e2e | `npm run test:e2e` | **豁免登记**（见 §3）：环境缺 chrome/chromium-1217（下载 100% 后解压被 interrupted×3）；D4/D5 已退役留档、E4 保留，其余用例与迁移面零交集；门禁以 vitest 全量+tsc+build 出具。配套：`ARIA_E2E_PORT` env 旁路落地（默认 4317 不变）——4317 被 T1 门重测部署占用（PID 1005268），旁路 4599 下 API server+vite dev 均正常拉起 |

## 3. 退役留档（断言面处置）

- **e2e D4/D5**（stage-ui.spec.ts 内注释+inventory #56）：D4 锚 legacy request-change 输入发送、D5 锚 story 流 terminate=human_confirm 帧——均随前端 legacy 决策发送删除退役；typed 面 e2e 需 SC 会话夹具（WorkItemPlan+provider run 至人工门，现无、建夹具=越界），typed 动作面由 vitest 断言族承接（abandon/confirm wire 形状=facade 路由+幂等 command_id=plan-repair typed 通道审计）。**E4 保留**（stage 驱动 unload guard 只读行为）。
- **vitest 退役**（各文件内注释留档）：`useWorkspaceWs.actions`「sends revert_work_item messages」（死发送器）；`useWorkspaceWs.presentation.actions.test.tsx` 整文件（presentation 发送面删除，store 逻辑留 T5）；p1-entries 采纳返修三测；ChatWorkspacePage.work-item-plan 逐段七测；review/actions/artifacts 的 legacy 决策断言重钉为只读呈现（review_decision 门/门卡/candidate panel/review panel 只读+按钮缺失断言）；`ChatWorkspacePage.test`「uses the stable typed gate command」退役（stable command 语义由 cockpit-action-routing.test「reuses the live turn command id」承接）。
- **gateId 重钉**：plan-repair-actions :49/:80 `legacy:human_confirm` → `"stage:human_confirm"`（投影 stage 前缀派生）；projection.test 同步。

## 4. T2 契约对齐与已知限制

- **契约一致**：`abandon_human_gate` wire 名/`command_id` 幂等键与 T2 7e64abc8 钉死形态逐字对齐；T2 白名单（SC human_confirm stage）+through-dispatch durable Terminated 已实测。前端 abandon 经 `commandId ?? newCommandId()`（UUID 恒非空）满足 INVALID_COMMAND_ID 校验。
- **story/design 双轨期限制（如实登记，REQ-RET-03 既定形态）**：T4 后 legacy 流（story/design/legacy-flow workitem）会话的**终止**钮改发 `abandon_human_gate`——T2 白名单仅在 SC 分支放行，legacy 流将收到 protocol error（明确错误优于隐式回退）；request-change 输入发送与 review_decision/逐段按钮在 Legacy 页已剥离（OQ2 只读）。approve（`confirm` 帧）legacy 分支仍接受不受影响。该限制为 T5 删除前已知窗口形态，T5 后 legacy 决策通道整体终止（在途 legacy 会话决策拒绝=已登记迁移限制）。
- **清单外发现**：CockpitInbox「采纳建议并返修」（:360）为计划锚点未列的产码 requestChange 消费点，已随清单确认一并删除。

## 5. 残留与移交（T5 输入）

- **待 T5 归属判定同批删**：`select_work_item_generation_mode`/`work_item_draft_decision`/`work_item_batch_decision`/`review_decision_response`/`select_revision_path`/`author_decision`/`request_revision`/`request_outline_revision`/`save_human_presentation_revision`/`revert_work_item` union 变体及 cockpit 侧 staged 发送器/ReviewDecisionActionBar（cockpit :1088-1097 仍装配，双轨期保留）。
- **store 面**：human_presentation begin/fail 动作、`resolveGateEntry("request-change")` 呈现值、stage 驱动门投影/闭环重置——读侧逻辑随 T5 收敛。
- **审计读侧**：`CockpitOperation` 的 request_change/terminate 历史值 T5 处置。
- **e2e 补跑**：环境就绪后 `ARIA_E2E_PORT=<port> npm run test:e2e`（inventory §e2e 豁免登记附操作）。

## 6. 提交清单

| commit | 内容 |
|---|---|
| `3837ba75` | feat(web): 产码迁移 16 文件（union/发送器/facade/路由/页面/组件，tsc 产码 0） |
| `e404dfdc` | test(web): 测试面重钉+退役留档+stage key+bulk 帧+e2e D4/D5 退役+ARIA_E2E_PORT 旁路+wp3-frontend-inventory.md |
| （本提交） | docs: 本报告+inventory e2e 豁免登记 |
