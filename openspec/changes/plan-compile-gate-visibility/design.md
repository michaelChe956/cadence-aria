# Design: plan-compile-gate-visibility

## Context

See `proposal.md` for the user-visible problem and scope. 本 change 承接已确认的 oracle 裁决：缺口 A 是批次确认门未被 `selectGateProjection` 投影（成功和 compile 失败转 batch-confirm 均受影响）；缺口 B 是 `WorkItemPlanCompileRecovery` 仅在 legacy 页面出现，且 SC `HumanConfirm` 协议臂未放行 `WorkItemPlanCompileRecoveryAction`。

现有前端单一事实源是 `web/src/state/workspace-cockpit-projection.ts` 的 `selectGateProjection`/`selectCockpitInbox`，动作面是 `CockpitInbox.tsx` 的 `GateInboxActions`/`GateFeedbackEditor`，动作路由集中于 `cockpit-action-routing.ts`。后端已有 recovery action 类型和 WS 入站消息（`workspace.ts` 的 `WorkItemPlanCompileRecoveryAction`），协议矩阵位于 `src/web/workspace_ws_handler/protocol.rs`。因此本 change 以现有投影、facade、协议错误和 durable 状态为边界，不新造第二套门状态机。

F-34 的事实基线已由 controller 取证定稿：`src/web/handlers/support.rs:299-306` 创建请求目前只有 title 与四个选项，缺省 provider 走服务端 `codex`/`claude_code` 默认；`src/web/provider_availability.rs:41-70` 会因可用性回退而漂移。`WorkItemPlanOptionsFormValue` 没有 provider 字段，`ChatCockpitPage.tsx:750-779` 是唯一应用 localStorage 默认的页面挂载点，`ChatWorkspacePageLegacy.tsx` 无默认应用。现有 WS `provider_select` 在 `PrepareContext` 放行且实测切换/落库正常，真正问题是创建先按服务端默认启动、页面后补发选择造成竞态。

## Goals / Non-Goals

**Goals:**

- 为成功批次确认和 compile 失败 recovery 两条路径生成稳定、去重、可诊断的 Cockpit 门投影。
- 让 Cockpit 复用既有 recovery action，形成从投影到 WS 到结果反馈的完整可操作路径。
- 让协议只在可证明的 SC recovery 状态放行 recovery action，并保持非法状态 fail-closed。
- 保持 `REQ-RET-02` typed 三命令、`F-21` phase mismatch 纪律和 `F-30` 终态守卫不变。
- 消除 provider 创建竞态：创建入口传递 provider 快照；Cockpit 与 legacy 页共享默认应用；开始生成前显示实际 provider。

**Non-Goals:**

- 不重建 legacy 逐段确认/generation-mode/review_decision；不修改 compile、batch 或 recovery 的业务状态机与 action 枚举语义。
- 不把 recovery action 当作人工门反馈或关门命令，不让 Cockpit 隐式触发 `advance` 或 coding provider。
- 不新增服务端用户账户偏好模型。本 change 继续以现有 localStorage 默认作为创建快照来源；服务端偏好可在后续 change 以兼容方式引入。
- 不连接或污染 4317。

## Decisions

### D1: 统一投影而非新增门状态机

`selectGateProjection` 扩展为识别两类现有 timeline/state 事实：`work_item_batch_confirm` 产生批次确认投影；`work_item_plan_compile_recovery` 产生 recovery 投影。投影 key 优先使用 durable gate identity 或 stable session/timeline identity，不能使用每次渲染生成的随机值。`selectCockpitInbox` 继续从该投影派生单条 inbox item，重复事件以同 key 更新而非追加。

批次投影的标题、摘要和 artifact/review 上下文沿用现有 `GateSummary` 数据源；recovery 投影展示 recovery 原因、当前状态与 action 可用性。若 projection 需要的新字段在 store snapshot 尚不存在，先从已有 timeline node、artifact payload、session status 和 protocol diagnostic 派生；无法证明状态一致时显示只读诊断，不猜测 action。

**为何这样做**：保持 inbox 与主区门卡同源，避免 Cockpit 自己维护一个会与 durable 状态漂移的状态机。备选是新增独立 `compileGateStore`，被否决：会重复身份、关闭、重连和终态逻辑。

### D2: recovery 是独立 typed 操作，不扩大三命令

Cockpit facade 增加与现有 `confirm/feedback/terminate` 同级、但语义隔离的 recovery action 发送入口（或按既有 action routing 复用同一发送器）。动作只接受现有 `continue`、`abort_and_rollback`、`human_triage` 枚举；不得转换为 `Confirm`、`HumanGateFeedback` 或 `AbandonHumanGate`。

协议放行采用最小矩阵增量：在 `HumanConfirm` 仅当 `flow_kind`、workspace type、当前 timeline/compile recovery durable 事实和 action 载荷一致时放行；普通 `HumanConfirm`、`AuthorConfirm`、generate phase、已确认/终止终态和缺失/不一致 recovery 事实一律拒绝。`REQ-RET-02` 的 SC 三命令仍保持原判定；F-21 的 phase mismatch 仍禁止 confirm/feedback，且 recovery 不能借此成为相位绕行；F-30 终态守卫在 facade 与服务端双侧保留。

**为何这样做**：recovery action 已是现有 wire 能力，缺口是 SC 可达性和 cockpit 消费面，不是需要新协议。备选是恢复 legacy 页或新增 recovery 命令，前者无法解决统一 Cockpit 入口，后者扩大 wire 契约和迁移风险，均否决。

### D3: provider 在创建请求中快照，默认应用共享化

创建 plan/story/design 的请求模型增加可选但由正常用户路径填充的 provider 字段。表单 provider 选择以用户显式选择优先，否则读取现有 localStorage 默认并在提交时快照；服务端对缺省字段保留兼容默认，但不应以可用性回退静默替换已选 provider。不可用 provider 在创建或开始生成前返回明确错误/要求重选。

将 Cockpit 目前的默认应用抽为共享 hook/适配层，legacy 页同样在 provider 尚未锁定且阶段允许时调用既有 `provider_select`。hook 必须幂等：已锁定或已启动的 provider 不被默认覆盖，非 `PrepareContext` 不重发无效选择。创建页、开始生成控件或 workspace header 在启动前显示实际 provider 与可用状态。

**偏好存储取舍**：本 change 选择 localStorage 作为现行默认来源，理由是已有实现、无需新增账户/服务端 schema，且把值在创建请求中快照后即可消除竞态；缺点是跨设备和跨入口不一致，记录为后续演进而非当前 blocker。服务端偏好不是本 change 的隐式新增范围。

**为何这样做**：仅靠 cockpit 挂载后改写无法消除创建竞态；立即引入服务端偏好又扩大认证、迁移和可用性语义。备选均不如“创建请求快照 + 共享 hook + 明示实际 provider”可逆且小。

### D4: 错误、重连和终态处理

- action 发送前在 facade 检查 driver lease、stage/flow、门关闭和终态；服务端再次校验 durable 状态，任何一侧失败均无状态副作用。
- 重连/重复消息只更新稳定 key 的投影；恢复中的 action 显示处理中，服务端返回的拒绝包含既有 protocol/recovery code，不把错误降级为空门。
- compile/recovery 终态到达后投影收口；迟到 timeline/artifact 事件不能重新打开门。
- provider 创建失败或选择不可用时，按钮/启动入口保持 fail-closed，显示原因，不静默回退到环境默认。

## Risks / Trade-offs

| 风险 | 缓解 |
|---|---|
| 现有 timeline 节点与 snapshot 字段组合不足以唯一识别 recovery 门 | 使用 durable session/timeline identity；缺凭据时只读诊断并补 core fixture，不猜测身份 |
| recovery action 放行过宽，绕过 F-21/F-30 | facade + WS 协议 + 服务端 durable 状态三层守卫；it_web/it_core 覆盖合法/非法矩阵 |
| 批次门与 typed HumanConfirm 门重复展示 | projection 单一事实源与稳定 key 去重；每种状态只保留一个 inbox item |
| 重连/事件乱序导致旧门复活 | 以 durable terminal/session status 为权威，终态优先，乱序事件不能降级状态 |
| provider 仍在页面加载期间漂移 | 创建请求提交时快照 provider；开始生成前显示并校验实际 provider；已锁定会话不被 hook 覆盖 |
| localStorage 默认无法跨设备 | 明确记录为后续服务端偏好演进，不伪称跨设备一致；当前契约保证单次创建确定性 |
| provider 不可用时用户体验为硬阻断 | 返回可诊断 availability 原因与重新选择入口；禁止静默换 provider，避免不可预测执行 |
| 前后端协议增量与旧客户端交互 | 同仓原子切换；未知/不匹配 action 保持 protocol error+零副作用；无 4317 连接 |

## Migration / Rollback

无 durable schema 或生产数据迁移。前后端同仓部署时，先兼容读取缺省 provider，再由创建页面开始填充 provider；projection/action 变更可通过普通提交回滚。回滚不得删除历史事件或 durable recovery 记录，旧页面仍可按既有只读/兼容路径读取。