# Proposal

## Why

统一方案 C3：F-53 三层实现缺口 + V0 定案的 degraded 投递缺陷。实证：`{type:"abort"}` 输入条中止钮在门开态（无活 run）时 abort 被受理但静默零回执→会话不动=「看似无反应」；Abort 绕过 stage validation 矩阵（protocol.rs 豁免）；门态下中止钮与门级「终止此门」语义混淆；V0 证实服务端 degraded attachment 静默丢帧（try_send 满即丢零日志）×门开后引擎静默（恢复依赖下一次广播永不触发）→连接停 stale 视图至 reload。

## What Changes

1. **abort 诚实化**：无 active run 的 abort 返回稳定错误码 ProtocolError（不静默）；Abort 移出 stage 矩阵全局豁免——human_confirm/Completed 等门态拒绝（附指路文案「门级操作请用反馈/确认/终止此门」）；in-flight 修订期另设窄 escape（若裁决需要，见 Open Questions）。
2. **门态 UI 语义分层**：有人工门开着时不渲染普通 run 中止钮；错误文案指向门级动作；死代码清理（useStageUI actions/rollback sender 全仓消费者核查后删除）。
3. **degraded 投递可靠**：关键帧（stage_change/session_state/HumanGateOpened 等门开类）对 degraded 连接改等待式投递或失败时显式 `resync_required`；degraded 转移落 append-only 诊断打点（lease-diagnostics 同构通道）。

## Capabilities

### New Capabilities

- `human-gate-termination-reliability`: run 中止与门终止的语义边界、回执契约与 degraded 连接关键帧投递契约

### Modified Capabilities

（无——CG-04 abandon 语义保持；仅当裁决「in-flight 修订期 Abort 窄放行」时增补 CG-02/04 场景，见 design Open Questions）

## Non-Goals

- 不把 abort 静默升级为 abandon（语义不合并）
- 不恢复 Abort 全局豁免
- 不修改客户端 event_seq 去重（V0 证实无罪）
- 不做 WS 重排/重传协议（仅关键帧等待式投递+resync_required 信号）
