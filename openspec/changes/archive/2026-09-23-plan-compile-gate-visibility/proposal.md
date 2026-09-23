# Proposal: plan-compile-gate-visibility

## Why

当 Work Item Plan 进入 Final Compile 失败可恢复分支，或成功编译后等待整组 Work Item Draft 确认时，Cockpit 当前没有对应的门投影与可操作入口：`WorkItemBatchConfirm`（包括 compile 失败转入 `enter_work_item_batch_confirm` 的路径）处于 `AuthorConfirm` 却不创建 `humanGateTurn`，`selectGateProjection` 也不投影 `work_item_plan@author_confirm`；成功的整组确认同样不可见。`WorkItemPlanCompileRecovery` 还只存在 legacy 页面，且 SC 流的 `HumanConfirm` WS 协议臂未放行 `WorkItemPlanCompileRecoveryAction`，导致用户在 Cockpit 看不到门、也无法从 Cockpit 恢复。

这使计划会话在用户必须作出确认或恢复决定时表现为“无门、无入口”，违反 Cockpit 作为统一待处理门面和 WS 可操作通路的预期；问题已由 oracle 裁决为独立立项缺口。

同时，F-34 已获用户确认并入本 change：Workbench 创建 plan/story/design 时，创建请求不携带用户 provider，服务端先按 `codex`/`claude_code` 与可用性回退创建，页面稍后才通过 WS `provider_select` 补发默认，造成 provider 竞态与不可预知的实际执行者；legacy 页面还没有应用浏览器默认。用户需要在创建/开始生成前明确看到并控制实际 provider。

## What Changes

- 新增 Cockpit 对 `work_item_batch_confirm` 的门投影：成功路径与 compile 失败转 batch-confirm 路径均生成可辨识的待处理门，并展示整组 Draft 确认上下文。
- 新增 Cockpit 对 `work_item_plan_compile_recovery` 的门投影与动作面：恢复动作不再仅限 legacy 页面，用户可在 Cockpit 发起既有 compile recovery action。
- 复用现有 typed 动作与错误/终态守卫，使 Cockpit 的确认、反馈、终止、恢复操作不绕过 `REQ-RET-02` 三命令边界、`F-21` 相位纪律或 `F-30` 终态守卫。
- 修订 SC `HumanConfirm` 阶段的协议放行判定：仅在对应 durable 门状态与合法 phase 下放行 `WorkItemPlanCompileRecoveryAction`；非法阶段、终态、错误 flow 或不匹配门状态仍 fail-closed 并返回可诊断协议错误。
- 将 F-34 纳入正式 capability：创建请求带上用户选定/默认 provider；抽取共享默认应用机制覆盖 Cockpit 与 legacy 页面；在点「开始生成」前显示实际 provider，消除创建后补发选择的竞态窗口。
- 为 projection、Cockpit 动作面、WS 协议矩阵和 provider 创建/默认应用补齐 `vitest`、`it_web`、`it_core` 测试覆盖，锁定成功、失败恢复、重连/重放、非法动作、实际 provider 可见性与既有门回归。

## Capabilities

### New Capabilities

- `plan-compile-gate-visibility`: 计划批次确认与 Final Compile recovery 门在 Cockpit 的统一投影、可操作动作面和 SC WS 放行边界。
- `plan-provider-selection`: Workbench 创建入口的 provider 选择、默认应用、跨页面一致性与生成前可见性。

### Modified Capabilities

- `work-item-plan-single-candidate`: 增补计划编译/批次确认状态在 Cockpit 的可见性与动作入口契约；不恢复已退役的逐段确认协议。
- `work-item-plan-conversational-gate`: 明确计划 compile recovery 与人工门动作共存时的 typed 消息边界、phase/终态守卫和 fail-closed 行为。
- `legacy-protocol-retirement`: 仅修订 Cockpit 对仍有效 typed recovery action 的消费与协议放行边界；不恢复 `HumanConfirmDecision` 或其他已退役 legacy 消息。

## Scope / Non-Goals

### Scope

- Cockpit projection/store 对两类 plan compile 门的识别、去重、摘要与状态更新。
- Cockpit inbox 对批次确认和 compile recovery 的可操作入口，以及动作发送后的成功/拒绝/恢复反馈。
- SC WS protocol stage/flow 放行矩阵对 recovery action 的最小增量调整。
- Workbench plan/story/design 创建请求带上用户默认 provider；创建后由共享 hook 在 Cockpit 与 legacy 页面应用默认；开始生成前显示实际 provider。
- 与现有 typed command、durable gate snapshot/turn、重连恢复、协议错误和终态投影的集成测试，以及 provider 选择与默认应用的前后端/页面回归。

### Non-Goals

- **不改 `REQ-RET-02` 的 typed 三命令契约**：HumanConfirm SC 仍只接受 `HumanGateFeedback`、`Confirm`、`AbandonHumanGate`；AuthorConfirm SC 仍只接受 `Abort`、`AbandonHumanGate`。本 change 不把 batch/recovery 重新建模为 legacy 人工确认命令。
- **不放宽 `F-21` 相位纪律**：generate 相位的 confirm/feedback 继续被 `phase_mismatch` 封锁；只放行既有合法终止/恢复动作。任何为 recovery 设定的放行必须绑定正确 stage、flow 和 durable 状态。
- **不突破 `F-30` 终态守卫**：`confirmed`、`terminated` 等终态不产生新的门操作；迟到动作 fail-closed 且零副作用。
- 不恢复 legacy 页作为唯一入口，也不删除仍被历史存量读取所需的只读兼容。
- 不改变 compile、batch confirmation 或 recovery 的业务状态机、持久化语义、动作枚举含义和错误码语义；本 change 解决投影、可操作性与协议可达性。
- 不要求本 change 引入服务端跨设备 provider 偏好账户模型；首选复用现有 localStorage 默认，在创建请求中快照传递；服务端偏好迁移仅作为后续兼容演进，不阻塞本 change。
- 不连接或污染 4317 生产服务器。

## Impact

- **前端**：Cockpit projection、inbox/动作组件、动作路由及其测试需要识别两类门；共享 provider 默认 hook、创建表单和生成前摘要覆盖 Cockpit 与 legacy；现有 Story/Design 和 SC 对话式人工门行为保持不变。
- **后端协议**：仅调整现有 `WorkItemPlanCompileRecoveryAction` 在合法 SC 门阶段的放行矩阵与拒绝证据；创建请求新增 provider 字段并在缺省时保持服务端兼容默认，不新增 recovery wire 命令。
- **测试**：新增/修改 `vitest` projection、组件/provider 选择覆盖，`it_web` WS/handler 与创建载荷边界覆盖，`it_core` compile/batch/recovery 状态与终态守卫覆盖。
- **迁移**：无 schema、durable 数据或部署迁移；前后端同仓协议变更原子交付。

## F-34 Evidence Baseline

- `support.rs:299-306` 的创建入口当前仅接受标题与四个选项，服务端在缺少 provider 时使用 `codex`/`claude_code` 默认；`provider_availability.rs:41-70` 的可用性回退会使默认随环境漂移。
- `WorkItemPlanOptionsFormValue` 当前无 provider 字段；`ChatCockpitPage.tsx:750-779` 是默认应用的唯一页面挂载点，`ChatWorkspacePageLegacy.tsx` 未应用默认。
- 现有 WS `provider_select` 机制在 `PrepareContext` 可达且实测可切换/落库；缺口是创建请求先按服务端默认启动、页面稍后补发选择造成竞态，而非切换机制本身失效。
