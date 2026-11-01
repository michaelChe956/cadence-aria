# Proposal: cockpit-fullcourse-connection-resilience

## Why

2026-09-16 全天用户验收（3.7 四包终验）在通过主链的同时暴露三条结构性欠账，全部有实锤证据：

1. **前半程断头**：cockpit 只覆盖「生成开始之后」的门与执行流——「开始生成」入口与 Provider 配置仅存在于 legacy 页（cockpit 对 story/design 会话无生成入口，台账:L494⑤），生成过程在 cockpit 无流式可视（M1-4）；用户被迫在 legacy/cockpit 两形态间「换脸」（M1-2，09-16 用户裁决 A 原文），且验收现场出现「Provider 建错只能另改」（0437，台账:L494②③）。
2. **连接与运行纠缠**：断连污染运行终态——验收期间至少四处 `aborted_by_disconnect` 死链现场（M1-1：0408/0429/0437/用户口述一次）；RCA 定案（p4-disconnect-rca.md）：前端 stale-close 误杀已热修（af4f7ccf），但 socket 清理误标 run 终态（0429 真实业务结局 `validation_reject` 被覆盖、0437 门开 104ms 被覆盖）、服务端 idle 90-95s 独立风险、断连起点归因缺口（无 close code/connection id，「谁先关闭连接」不能诚实下结论）仍在；被动连接实时事件盲区（driver 1122 chunk vs 页面 0 行，台账:L471）。
3. **恢复链阻断**：快照门三形态——0017（引擎终态会话门投影仍可操作→confirm 撞 `INVALID_MESSAGE_FOR_STAGE`）、0429（断连中止快照门 confirm 被 `WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID` 拒：phase 停在 Evaluate 未回门节点）、0437（门刚开即被断连误标覆盖）。

三项决议权威：`p38-tripartite-decision.md`（controller+oracle+ter-task 三方共议，用户已授权共议推进）。P2 工作量按 ter 重估 84-130h（实施）/ 96-150h（含流程）专项排期，否决输入包「2-3 个工作日」估算。

## What Changes

> 主契约（权威）：`cadence/designs/2026-09-16_设计文档_3.8驾驶舱全程化与连接韧性_v1.0.md`。本 change 是其契约化落地，不重新论证。

**P1 前端补全（引擎零改动，延续 3.7-C1 安全面）**：

- **生成入口**：cockpit 对 story/design（及 plan 未开始态）补「开始生成」入口与发起流，复用既有 `useWorkspaceWs` 发送链与 legacy 全部发起语义（stage 守卫/连接状态/recoverable run/optimistic entry）；不另开生成 socket、不建第二份 provider 状态。
- **Provider 配置**：复用既有 `ProviderConfigPanel` 与 store 语义（author/reviewer、reviewer 开关、permission、rounds），仅可编辑阶段暴露；含默认 Provider 选择。
- **流式进度区**：复用三区执行流+下钻对话流投影，补「生成中」可辨识运行态（空/失败/完成边界）；只消费既有 WS 事件面，不足则该子项移入 P2 契约。
- **形态路由终态化（Q1a）**：类型感知路由机制保留为终态；默认表演化——未显式设置的已知类型（story/design/plan 含未开始态）默认 cockpit（**对 09-16 裁决行为面的变更，呈报用户明示 R-1**）；显式设置尊重/开关/回滚（C4）、跨会话归属守卫、未知类型安全 legacy 全保留；「空 plan 必须 legacy」前提删除（守卫不删）；首帧定形态（消除中途 unmount+close(1000)+重连）；legacy 页面不删（退役=阶段 4）。

**P2 连接解耦（引擎专项，正式破除 3.7-C1）**——RCA §5 根治四条：

- **session-owned run manager**：run（单一 engine 实例、run token、provider-drive、状态机、终态原因）由 session 级 manager 唯一持有；连接只是可替换订阅者；开始/取消/命令/完成进入单一原子临界区（token+lease epoch 判等）；engine 不随连接销毁；内存回收点=终态+无订阅者；进程重启从 durable 重建、不虚称 provider 跨进程连续。
- **driver/observer 角色与 lease（Q2a）**：Hello 增可选 `role` 字段，双字段容忍（serde 无 deny_unknown_fields 双向兼容，driver 脚本族 6 处 hello 发送点零破坏）；缺席 role 服务端入口归一为内部显式常量，仲裁只认内部角色；observer 不可发写消息（服务端拒绝+可诊断错误）；lease=会话级单活性 driver 授权（连接持有、关闭撤销不取消 run、显式接管 epoch 递增）；阶段 4 退役门评估收紧为必填。
- **连接关闭不写 `aborted_by_disconnect`**：只有 provider token 实际被取消/provider 失败/明确人类 abort 才写真实终态；不把活动节点标 Failed、不整流回 prepare_context；run 完成与 close 并发只产生一个真实 terminal reason。
- **cursor 重订阅+session 级广播**：session-scoped 单调 `event_seq`（Hello 可选 `after_event_seq`，同双字段容忍纪律）；有界事件 journal；cursor 过旧/缺口先发带 cursor 的 snapshot 建基线；慢订阅者有界通道溢出标记需 snapshot、不反压 provider；广播范围=workspace WS（coding WS 广播=条件扩展，P2 滑窗则 defer 并显式登记，不以「顺带」模糊）。
- **存量与回滚**：durable schema 不变、历史标记保留、卡死 session（0166/0167/0168/0199/0277/0429）处置=复活验证不回写；回退旧二进制安全（无 schema 断裂、wire 双向容忍），P2 关闸含回退读验证。

**P3 恢复链**：

- **P3a（OpenSpec 批准后首批）**：①连接级中性归因日志（服务端 connection_id+receiver 退出类型+最后双向活性时间；前端 onclose code/reason/wasClean/visibilityState；中性载体与业务 timeline 终态分离；过渡期同 connection_id 关联既有 `aborted_by_disconnect` detail）——一次复现闭环归因缺口；②快照门投影终态守卫（引擎终态/相位失配的门锁定不可操作，0017 形态根治）。
- **socket 误标修复随 P2 架构吸收**（不独立先行；让步判据=不引入第三态的最小守卫可并入先行包，默认不并）。
- **P3b（P2 族内最前）**：phase 回门节点（approval compile 失败/修订中止后 phase/active_node 回滚到门节点，0429 形态根治，事件前缀不可变）+ REQ-CG-04 人工权威升级语义显式化补句（4f34ba58 已实施语义的契约化，零行为变更）。M5-2 吸收边界：吸收补句+phase 回门两项；`complete_human_gate_revision` approval 三元组缺陷显式留 4 号修法包 backlog。

**P4 清尾**：driver readback elapsedMs 修复（必做且前置——挡 RCA §6 矩阵 driver 种子效率）；跨会话批量=条件项（依赖 P2 命令仲裁安全+广播一致性两机制；滑窗则 defer 阶段 4 前夜）；杂项清点表逐项「已修/已 defer+理由」（timer 节流结项口径=P2 重连重订阅架构使其无害化，非「不修」）。

**非目标（明确不做）**：3.7 已验收交互面语义零改动（只做加法）；阶段 4 边界不抢先（多仓 coding/旧协议退役门/DEF-4·5·6/legacy 删除/role 收紧必填均不执行，P1 只解锁退役门评估）；M1-5 arch-code 反代入口=显式 defer（用户侧访问配置，文档指引 127.0.0.1:4317 直连）；跑动中接管/门禁语义变更另立项；编码侧插话 fail-closed 语义不动（P2 只做连接身份与所有权）；不新增 Provider/CI/持久化评估语料。

## Capabilities

### New Capabilities

- `web-cockpit-full-course`: 对话侧驾驶舱全程化——生成入口/Provider 配置/流式进度可视/形态路由终态化（默认表演化）/快照门投影终态守卫/跨会话批量（P2 条件项）。纯前端。
- `workspace-session-connection-resilience`: workspace 会话连接韧性——session-owned run manager、driver/observer 角色与会话级 lease（wire 双字段容忍）、连接关闭不写断连终态、cursor 重订阅与 session 级广播、连接级中性归因日志、存量数据与降级回滚兼容、RCA §6 断连验收矩阵（P2 done 定义，不可裁剪）。

### Modified Capabilities

- `work-item-plan-conversational-gate`: P3b 恢复链——MODIFIED REQ-CG-04（人工权威升级语义显式化补句，契约化既有实现零行为变更）与 REQ-CG-05（approval compile 失败/修订中止后 phase/active_node 回门节点，0429 形态根治；回滚不改写事件前缀）。

## Impact

- **受影响代码**：P1——`web/src/pages/ChatCockpitPage.tsx`（+Parts）、`web/src/hooks/useWorkspaceWs.ts` 族、`web/src/state/workspace-ws-store.ts` 族、路由形态判定（`readChatCockpitMode` 族）、复用 `ChatInputBar`/`ProviderConfigPanel`/`TimelineNodeList`/`ChatEntryList`；类型感知三轮防回归既有测试随默认表演化同步迁移（不得删用例）。P2——`src/web/workspace_ws_handler/`（socket/run/decisions 族）、`src/web/workspace_ws_types/in_.rs`（Hello 可选字段）、`src/web/test_controls/socket.rs`（idle 口径如涉）、run 注册/engine 所有权/命令仲裁/事件转发；前端尾巴（`web/src` WS 客户端层 hello 新字段/重连逻辑，排序在 P1 后）。P3a——服务端连接日志（新诊断记录）+前端 onclose 诊断+投影守卫。P3b——`src/product/workspace_engine/` 恢复链族（conversational_gate/compile/single_candidate）。P4——driver 脚本（`workitem_run_campaign.mjs` 等）与杂项。
- **受影响接口/wire 契约**：workspace WS Hello 增可选 `role`/`after_event_seq` 字段（双字段容忍：旧客户端缺席=归一 legacy driver；新客户端对旧服务端 serde 忽略未知字段）；driver 脚本族 6 处 hello 发送点零强制升级；durable 语义变更=连接关闭不再写 `aborted_by_disconnect`/不再标 Failed（schema 不变，回退安全）。
- **BREAKING（行为面）**：默认表演化（未显式设置的 story/design/plan 未开始态默认 cockpit——明示点 R-1）；连接关闭不再产生 `aborted_by_disconnect` 审计节点（P2 后中性归因记录成为唯一连接级事实源）；既有测试迁移=类型感知默认族页面测试+断连清理行为测试。
- **验收**：RCA §6 四条断连矩阵全量真跑（不可裁剪）；P1 真实链（双层范式：自动化替身只做回归，人工只认真实链）；P3 三形态复现矩阵（0017/0429/0437）；存量卡死 session 复活验证；回退读验证。
- **流程纪律**：P3a/P2/P3b 均 OpenSpec 批准后实施（R-3）；归因闭环前文案不得宣称「断连主嫌已根治」（R-2，热修表述=「消除已证实的前端断连源」）。
