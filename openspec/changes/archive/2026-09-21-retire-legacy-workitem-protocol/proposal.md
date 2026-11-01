# Proposal: retire-legacy-workitem-protocol

## Why

阶段 4 立项三方决议（`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/stage4-tripartite-decision.md`，Q1/Q4/Q5，五项零分歧共识）将「退役+DEF-4」切为本 change（change ③，落地序 ①→③→② 的次环，前置 change ① close-provider-validation 已收官、codex/pi 验证过的 campaign 基建在场）。四项事实依据：

1. **退役门证据陈旧（Q1 裁决 a 全口径重测）**：REQ-WSC-07 判据的既有证据是阶段 2 时代出具——pi 时长唯一实测=08-31（938.04s=15.63min＞12min，唯一登记未过项），早于其后的全部栈变更；oracle 一致性核查明确「Q1『其余子项全过』为阶段 2 时代证据→重测矩阵按全口径出具不止 pi 时长」。删除面不可逆的退役必须以现行 build（决议记载 v22，起草时现行部署已至 v24，以重测执行时 build 为准）重出完整证据矩阵。用户 1c 裁决（08-31）钉死：不放宽、不改口径、不删子项；解锁路径仅两条=pi 达标 or 后续专项裁决。
2. **legacy 双轨维护成本持续发生（M3）**：现行 build 上新旧两条 workitem 路径并存——后端 `WsInMessage` 28 变体中 legacy 逐段确认族全活、`HumanConfirmDecision` 旧枚举消费面 HEAD 实读 24 个 .rs 文件（全 legacy 族符号 53 个 .rs）；前端双轨三处在案（workspace.ts union 同含 `human_confirm` 与 `human_gate_feedback`/`advance`、useStageUI 暴露 `confirm/request_change/terminate` 三动作、plan-repair/cockpit 经 `legacy:human_confirm` 路由仍发 `human_confirm`）。
3. **oracle scope 修正：删除大头在前端**：决议 §scope 接正「面 C 删除大头在前端 57 文件非后端 21」（HEAD 复核见 design 事实基线——趋势成立，绝对数以实读为准）。退役 change 必须含前端 legacy 消费面迁移 WP，不能只删后端枚举。
4. **DEF-4 契约欠账（Q4 裁决 b）**：现状行为=「advance 到 Ready 即止、StartCoding 唯一启动入口、SC_CODING_REQUIRES_ADVANCE fail-closed 守卫」由测试与代码钉住但 spec 未显式化（defer-ledger DEF-4 在案：owner 后续 change）；显式 opt-in auto 通道降为红线条件项 defer（触发=autopilot/驾驶舱真实 auto 需求）。

## What Changes

（决议四项+1 项 oracle 风险承接，不加不减）

1. **退役门全口径重测（第一个 WP，解锁闸门）**：按 REQ-WSC-07 判据原文全子项在现行 build 出具完整证据矩阵——pi 时长关键项+codex 对照（各 1 案例 Confirmed、时长 ≤12min、初评/复评 ≤2、自动返修 ≤1、服务端持久计数）+14 条 classifier golden+compiler diagnostic golden+断线重连/恢复+legacy 路径回归全绿+多仓 preflight 不静默回落。判据零改（1c：不放宽不改口径不删子项）；pi 重测超时预算前置设定（先例曾需 1800s 上限）。**重测全绿→退役解锁；仍超标→挂起等用户终裁**（星化问法已备：A=维持门不删 legacy/B=授权登记例外+修订 REQ-WSC-07 门文本放行/C=限定 N 次重测取最佳；三方不得自行放宽，分歧即挂起）。顺带核 coding_run_campaign 未测区（冗余则标记退役）。
2. **legacy 删除（后端）**：WS 协议 wire 层 legacy 逐段确认族变体与 DTO（generation-mode 决策、逐段确认消息、review_decision 双选项语法、`HumanConfirmDecision` 族、SelectRevisionPath 族等 REQ-WSC-07 点名三族及其专属 DTO）+引擎消费核心（workspace_ws_handler/decisions.rs 与 workspace_engine/decisions.rs 的 legacy 决策路由分支、draft_batch/plan_outline 逐段确认引擎面）+`flow_kind` 单路径收敛。SC 门关门决策 typed 重承载（approve 保持 `Confirm`，abandon 显式 typed 命令，不再经 `HumanConfirmDecision`）。
3. **前端 legacy 消费面迁移（oracle scope 修正，独立 WP）**：union 双轨收敛（workspace.ts）、useStageUI legacy 三动作与 review_decision 动作迁移、plan-repair/cockpit `legacy:human_confirm` 路由与发送面（useWorkspaceWs/bulk-confirm-runner）切 typed、ChatWorkspacePageLegacy 页面处置；前端测试面同步收敛。
4. **DEF-4 契约显式化（Q4 裁决 b，销账 DEF-4）**：「advance 到 Ready 即止+StartCoding 唯一启动入口+SC_CODING_REQUIRES_ADVANCE 守卫」写入 spec 结项；显式 opt-in auto 通道红线条件登记 defer（若做须满足：opt-in 持久化 run_policy/默认 off/per-attempt 单发/绝不批量/不动唯一人工门）。
5. **退役后 REQ-WSC-07 多仓 preflight 条款同步修订**（oracle 风险承接）：legacy 删除后「legacy fallback 只允许在确定性 preflight 失败且新路径尚未产生副作用时发生」条款失效——多仓 preflight 失败一律收敛为新路径 durable fatal/recoverable 终态，无 legacy 回落。

## 非目标（明确不做）

- 不做多仓 coding（change ② 一期/二期）、provider 批任何事项（change ① 已收官）；kimi/claude 结论不进入 REQ-WSC-07 判据（Q3 口径：判据原文仅 codex+pi）。
- **不放宽不改不删 REQ-WSC-07 任何判据子项**（1c 钉死）：重测仅出证据，不改门文本；重测仍超标的任何处置（含限次取最佳）必须用户终裁，本 change 无权自行执行。
- 不实施 auto_start_coding 通道（Q4：显式 opt-in auto 为条件 defer 项，触发=autopilot/驾驶舱真实 auto 需求；届时另行立项且须满足全部红线五条）。
- 不删 SC 单候选路径依赖的共享基座：REQ-WSC-01..06 全部契约、conversational gate（REQ-CG-01..07 语义）、compile/policy 层、amendment 链、`Confirm`（SC 门 approve 语义承载）、StartCoding 及其守卫。
- 不删历史 durable 记录（legacy session 历史/事件前缀只读保留）；不篡改任何已持久化事件。
- 不动 coding 三角色执行面、多仓 group 执行模型（REQ-COD-01..06）、provider 注册面。
- 不顺手实施 pi 2.2/2.3（change ① 已显式 defer 登记）。

## Capabilities

### New Capabilities

- `legacy-protocol-retirement`：退役轮契约——门重测全口径与判据零改红线、重测仍超标挂起等用户终裁、删除面分层（wire/引擎/前端）与残留归零、存量 durable 记录只读处置、coding_run_campaign 未测区顺带核。

### Modified Capabilities

- `work-item-plan-single-candidate`：REMOVED「新旧路径并存与可验证退役（REQ-WSC-07）」+ADDED「旧协议退役与单路径收敛（REQ-WSC-08）」——退役完成态契约+多仓 preflight 条款修订（无 legacy 回落）。
- `work-item-plan-advance`：ADDED「advance 到 Ready 即止与 coding 启动唯一入口（REQ-ADV-05）」（DEF-4 显式化+auto 通道红线条件 defer 登记）+ADDED「SC 准入唯一入口与共享初始化约束（REQ-ADV-06）」；REMOVED「SC 与 legacy 入口隔离（REQ-ADV-03）」（legacy 入口保留条款随退役失效，准入约束由 REQ-ADV-06 承接）。
- `work-item-plan-conversational-gate`：MODIFIED「单飞与预算纪律（REQ-CG-02）」SC 门消息面边界句与「门关闭决定（REQ-CG-04）」approve/abandon 映射句——关门决策 typed 重承载，不再依赖 `HumanConfirmDecision`。

## Impact

- **代码（后端）**：`src/web/workspace_ws_types/in_.rs`（legacy 变体族+DTO 删除）；`src/web/workspace_ws_handler/decisions.rs`、`src/product/workspace_engine/decisions.rs`（legacy 决策路由删除+SC 门关门决策重承载）；`src/product/workspace_engine/{draft_batch,plan_outline}` 逐段确认引擎面；`conversational_gate.rs` 关门决策签名；`flow_kind` 消费面收敛（lifecycle_store/models/workspace_engine）。
- **代码（前端）**：`web/src/api/types/workspace.ts`（union 收敛）、`web/src/hooks/useStageUI.ts`、`web/src/hooks/useWorkspaceWs.ts`、`web/src/state/{cockpit-action-routing,bulk-confirm-runner,plan-repair-session}.ts`、`web/src/pages/ChatWorkspacePageLegacy.tsx`（处置）及全部 legacy 测试面。
- **对外 wire 协议（破坏性）**：legacy 入站决策消息族删除（收到返回 protocol error 零副作用）；新增 SC 门 abandon 显式 typed 入站命令。无部署/数据迁移（历史 durable 记录只读兼容）。
- **测试**：legacy 路径回归测试族随删除面退役（重测证据矩阵留档后删除）；SC 门/advance/守卫测试保留并锚定新契约。
- **openspec**：主 specs 四 capability 同步（single-candidate/advance/conversational-gate 修订+legacy-protocol-retirement 新增）；DEF-4/DEF-6 台账销账登记。
