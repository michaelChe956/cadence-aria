# WP3 前端 legacy 消费面清单（T4 / REQ-RET-02 L1）

> 生成方式：`grep -rlE "human_confirm|HumanConfirm" web/src web/e2e --include="*.ts" --include="*.tsx"`（计划 v1.1 清单口径：src+e2e）。
> 基线：worktree `feat-b-0808-add-monorepo` HEAD=`a92151c4`（T4 开工实跑，63 文件=src 61+e2e 2；计划所记 61 为 `4d097c1d` src-only 口径，实施时以实跑为准）。
> 处置 legend：**切 typed**=改发 typed 帧；**删除**=代码删除；**重钉**=测试断言随行为更新；**退役**=测试删除+理由留档；**保留只读**=呈现/服务端下发值消费，非发送面（T4 不动，T5 归属判定另定）；**待 T5**=T5 归属判定后同批删除。

## 产码（22）

| # | 文件 | legacy 面 | 处置 |
|---|---|---|---|
| 1 | api/types/common.ts | `HumanConfirmDecision` 类型定义 :256 | 删除（引用清零后） |
| 2 | api/types/workspace.ts | union `human_confirm` 分支 :266；import :3 | 切 typed：删分支+增 `abandon_human_gate`（与 advance 对称） |
| 3 | api/types/workspace.ts | TimelineNodeType `"human_confirm"` :298 | 保留只读（服务端时间线渲染值） |
| 4 | api/types/workspace.ts | `human_confirm_state` session 字段 :211 | 保留只读（服务端下发字段） |
| 5 | api/types/workspace.ts | `request_outline_revision` :247 | 待 T5（归属判定：SC 不消费即同批删） |
| 6 | hooks/useStageUI.ts | `human_confirm:{actions:[confirm,request_change,terminate]}` :56-60；`review_decision:{actions:[select_revision_path,abort]}` :44-45 | 收敛 actions:[]（headerBadge 保留只读）；StageAction 联合类型裁剪 |
| 7 | hooks/useWorkspaceWs.ts | `sendHumanConfirm` :481-500（request-change/terminate 发 `human_confirm`） | 切 typed：删；approve→`sendConfirmGate`（既有 confirm 帧+审计）；终止→新 `sendAbandonGate(commandId)`（abandon_human_gate） |
| 8 | hooks/useWorkspaceWs.ts | operation 类型 :462（request_change/terminate） | 收敛：删两成员+增 abandon_gate |
| 9 | hooks/useWorkspaceWs.ts | `sendRevertWorkItem`/`sendHumanPresentationRevision`（Legacy 页剥离后死发送面） | 删除（union 变体留 T5） |
| 10 | hooks/useWorkspaceWs.ts | hasInFlightHumanConfirm :429-430 | 保留只读（stage 值检查，非发送） |
| 11 | state/bulk-confirm-runner.ts | :127 批量 confirm 发 `{type:"human_confirm"}` | 切 typed：`{type:"confirm"}` |
| 12 | state/cockpit-action-routing.ts | facade 输入 `sendHumanConfirm` :28/:40/:47/:70（confirm/request-change/terminate） | 切 typed：`sendConfirm`+`sendAbandonGate`；requestChange 成员删除 |
| 13 | state/cockpit-action-routing.ts | `gateIdentityFromState` `legacy:${stage}` 分支 :139 | 删除（gateId 派生收敛 humanGateTurn/humanGateSnapshot 两路） |
| 14 | state/workspace-cockpit-projection.ts | selectGateProjection 第三分支 fallback key `legacy:${stage}` :169 | 重承载：fallback key 改 `stage:${stage}` |
| 15 | state/workspace-cockpit-projection.ts | 存在条件注释 :116 + :46 stage 检查 | 保留只读（投影消费 stage 值） |
| 16 | state/workspace-ws-store.ts | 重建口径 :214-215、门闭环重置 :669-674 | 保留只读（stage 驱动状态机） |
| 17 | state/workspace-ws-store-types.ts | stage 联合含 `"human_confirm"` :85 | 保留只读（服务端 stage 值） |
| 18 | state/workspace-ws-store-helpers.ts | STAGE_ORDER 含 `human_confirm` :27 | 保留只读 |
| 19 | state/workspace-stage-labels.ts | 标签 :9 | 保留只读 |
| 20 | state/workspace-chat-rebuild.ts | gate prompt 重建 H5 :530-583（legacy 节点查找/注释） | 保留只读（历史节点呈现；key 派生随 #14 变化） |
| 21 | state/plan-repair-session.ts | node_type 映射 :734 | 保留只读 |
| 22 | state/linked-workspace-amendment-store.ts | `human_confirm_state` 值检查 :110-111 | 保留只读（服务端字段） |
| 23 | components/chat-workspace/ChatInputBar.tsx | human_confirm 输入发送路径 :89-96/:118-121/:176/:182/:370（onSendHumanDecision→request-change） | 切 typed：删 onSendHumanDecision prop+发送分支；human_confirm 阶段输入禁用（无发送通道）；staged/author 回调改可选（未提供即不渲染按钮） |
| 24 | components/chat-workspace/entries/GatePromptEntry.tsx | request-change 按钮 :171-180（legacy only）+requestChangePayload 族 :290-328 | 删除（前端不再发 legacy request-change） |
| 25 | components/chat-workspace/entries/GatePromptEntry.tsx | `legacy:human_confirm` 字面量 :71 | 重钉（随 #13/#14 新 key） |
| 26 | components/chat-workspace/TimelineNodeList.tsx | case 渲染 :186 | 保留只读 |
| 27 | components/workspace/WorkspaceHeader.tsx | badge :48 | 保留只读 |
| 28 | pages/ChatWorkspacePageLegacy.tsx | gateActions facade :210-230、sendHumanConfirm 消费 :97/:216-226、review_decision :863-869+handleSelectRevisionPath :467、逐段决策 props :674-679/:776-785/:880-885、author 决策 :472-480/:736/:748、onSendHumanDecision :672/:878 | 删除（OQ2 定案：保留骨架+常规交互，workitem 决策面全剥离，WorkItem 历史只读呈现） |
| 29 | pages/ChatCockpitPage.tsx | facade 输入 :492-507、hotkey facade :629/:655/:664 | 切 typed（sendConfirm/sendAbandonGate） |
| 30 | pages/ChatCockpitPage.tsx | ChatInputBar onSendHumanDecision :1079 | 删除（prop 随 #23 移除） |
| 31 | pages/ChatCockpitPage.tsx | stage 检查 :262 | 保留只读 |
| 32 | pages/CodingWorkspacePage.tsx | plan-repair regenerate :361 发 `sendHumanConfirm("request-change")` | 切 typed：`sendRequestRevision`（WorkItemPlan 子会话→`request_work_item_plan_revision`=SC plan-repair 消费通道，T5 保留变体） |
| 33 | components/workspace/WorkItemPlanStagedPanel.tsx | 逐段决策按钮（outline/generation-mode/draft/batch） | 回调改可选+分支隐藏（legacy 页不再提供）；compile_recovery 分支保留（SC compile 链=T5 保留变体） |
| 34 | components/workspace/WorkItemPlanCandidatePanel.tsx | onAccept/onRequestRevision/onRevert 动作 | 回调改可选+按钮隐藏（legacy 页只读呈现） |
| 35 | components/workspace/WorkItemPlanArtifactPanel.tsx | onSaveHumanPresentation | 回调改可选+保存 UI 隐藏（legacy 页只读；变体待 T5） |

## 测试+夹具（39 src）与 e2e（2）

| # | 文件 | legacy 面 | 处置 |
|---|---|---|---|
| 36 | api/types.test.ts | union `human_confirm` 帧构造 | 重钉（abandon_human_gate 形状+human_confirm 移出联合） |
| 37 | hooks/useStageUI.test.ts | human_confirm 三动作断言 | 重钉（actions:[]） |
| 38 | hooks/useWorkspaceWs.actions.test.tsx | sendHumanConfirm 发送/审计断言 :210/:274 | 重钉（sendConfirmGate/sendAbandonGate） |
| 39 | hooks/useWorkspaceWs.plan-repair-actions.test.tsx | sendHumanConfirm confirm/request-change :74/:100；gateId `legacy:human_confirm` :49/:80 | 重钉（confirm→sendConfirmGate 审计；request-change→sendRequestRevision 帧；gateId→null 新派生） |
| 40 | hooks/useWorkspaceWs.projections.test.tsx | session_state stage | 保留只读（stage 值） |
| 41 | hooks/useWorkspaceWs.test.tsx / timeline.test.tsx | stage=human_confirm 门卡重建 | 保留只读（key 断言随 #14 重钉） |
| 42 | hooks/workspace-ws-message-handler.test.ts | context blocker 门标记 | 保留只读 |
| 43 | state/bulk-confirm-runner.test.ts | confirm 帧+审计断言 | 重钉（`{type:"confirm"}`） |
| 44 | state/cockpit-action-routing.test.ts | facade sendHumanConfirm 输入 | 重钉（sendConfirm/sendAbandonGate/terminate wire） |
| 45 | state/workspace-cockpit-projection.test.ts / workspace-ws-store.gate.test.ts 等 | gate key `legacy:human_confirm` 断言 | 重钉（`stage:human_confirm`） |
| 46 | state/chat-entries.test.ts / workspace-ws-store.*.test.ts（content/snapshot/rebuild/artifacts/test） | resolveGateEntry("request-change")/stage 值 | 保留只读（resolution 值为条目呈现，非发送） |
| 47 | state/operation-audit-projection.test.ts / workspace-observer-store.test.ts / plan-repair-session.test.ts | stage/审计投影 | 保留只读 |
| 48 | components/chat-workspace/ChatInputBar.test.tsx | human_confirm 发送按钮 | 重钉（输入禁用+无发送钮） |
| 49 | components/chat-workspace/cockpit/CockpitInbox.test.tsx / cockpit/CockpitEscalation.test.tsx | stage 夹具 | 保留只读 |
| 50 | components/chat-workspace/entries/GatePromptEntry.test.tsx | legacy facade/`legacy:human_confirm` 夹具 | 重钉（无 request-change 钮+新 key） |
| 51 | components/coding-workspace/plan-repair-test-fixtures.ts | node_type/stage/human_confirm_state 夹具 | 保留只读（服务端下发值夹具） |
| 52 | pages/ChatWorkspacePage.test-utils.tsx / CodingWorkspacePage.test-utils.ts | sendHumanConfirm mock | 重钉（mock 面随 api 收敛） |
| 53 | pages/ChatWorkspacePage.*.test.tsx（test/actions/artifacts/review/work-item-plan） | legacy 页决策面断言 | 重钉（决策面剥离后断言只读呈现）或随不可达分支删除断言 |
| 54 | pages/ChatCockpitPage.*.test.tsx（confirm/gate/inbox/plan）+ test-utils | facade/门面断言 | 重钉（typed 动作面） |
| 55 | pages/CodingWorkspacePage.plan-repair.test.tsx | regenerate 走 sendHumanConfirm 断言 | 重钉（sendRequestRevision） |
| 56 | e2e/stage-ui.spec.ts | D4（send-human-decision）/D5（确认/终止钮）锚 legacy story 决策发送 | **退役留档**：所锚 legacy 决策发送面随本 WP 删除；typed 动作面 e2e 需 SC 会话夹具（现无，建夹具=越界），typed 面由 vitest 断言族覆盖（sendAbandonGate wire 形状/facade 路由/GatePromptEntry 动作）；D1-D3/E 系不受影响 |
| 57 | e2e/disconnect-strategy.spec.ts | E4 HumanConfirm 刷新不拦截 | 保留（只读行为：stage 驱动 unload guard，不发决策帧；双轨期 story 会话仍达该 stage） |

## 完成判定对照（计划 Step 1 原文）

- 产码侧 grep（`human_confirm\|HumanConfirm`）在 T4 结束后仅剩「只读呈现保留项」（stage 字符串/服务端字段/历史节点渲染）与「待 T5 项」（request_outline_revision 等发送器+union 变体——T5 归属判定同批删）。
- 测试侧仅剩重钉后的 typed 断言与只读夹具。
- Step 5 grep 断言（产码零 `type: "human_confirm"`/`sendHumanConfirm`/`"decision": "(request-change|terminate)"`）见报告 §grep。

## e2e 执行豁免登记（T4 收口，controller 2026-09-19 授权）

**状态**：`npm run test:e2e` 本轮未出具全绿结果，原因是**环境前置缺失**而非用例失败：

1. 4317 端口被 T1 门重测部署（PID 1005268，release build+`--work-item-plan-single-candidate`）占用——已按不干扰部署原则落地 `ARIA_E2E_PORT` env 旁路（playwright.config.ts/dev-server-proxy.ts/e2e/start-api.mjs，默认值不变），API server 与 vite dev 在 4599 正常拉起。
2. e2e 浏览器不可得：config `channel:"chrome"` 指向的 /opt/google/chrome 不存在；repo playwright 1.59.1 期望 chromium-1217，`playwright install chromium` 三次执行均「下载 100% 后解压被 interrupted」（缓存目录仅余 18M 元数据，无 chrome 二进制）；缓存中 chromium-1226 二进制与 1.59.1 协议不匹配（launch 即 browser closed），1226 归属（playwright 1.6x）与 repo 版本不一致。10 分钟排障窗口未收敛，按 controller 指示登记豁免收口。

**已覆盖的证据面**：D4/D5 退役留档（#56，理由与 typed 面 vitest 承接断言族在 spec 内注释+本表）；E4 保留声明（#57，只读行为）；其余 e2e 用例与本次迁移零交集（迁移面=legacy 决策发送/页面动作面，未触及断连重连/timeline 审计/视觉/内存等面）。前端门禁以 **vitest 全量 1489/1489 绿+tsc -b 0+`npm run build` 成功**出具。

**后续**：环境具备 chrome/chromium-1217 后执行 `ARIA_E2E_PORT=<port> npm run test:e2e` 补跑（预期仅环境性失败归零，无迁移面用例）。
