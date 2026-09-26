# Design: human-gate-termination-reliability

> 依据：统一方案 §5-C3 + V0 结论（f53-v0-frame-order.md：帧序完整/去重无罪/真因=degraded 静默丢帧×门开后无补偿）；实证 f53-diagnosis.md（abort 豁免+零回执+UI 投放三层缺口）。

## Context

abort 无 run 静默 no-op（inbound.rs:383-389）；Abort 豁免 stage 矩阵（protocol.rs:266-276）；门开态输入条 X 仍渲染（stage 漂移放大）；degraded try_send 满即丢零日志，门开后引擎静默至下次广播。

## Decisions

### D1 abort 回执与矩阵收编

无 run→ProtocolError（abort_no_active_run 稳定码+阶段+指路）；Abort 移入矩阵：门态拒、run 态受理。in-flight 修订期（human_confirm+修订 run 活跃）窄 escape 暂不实现（修订取消已有 watchdog 600s 兜底+用户可等可终止门）——若后续需要另立 delta 增补 CG 场景（Open Question 留档）。

### D2 前端呈现分层

门开态（state.stage=human_confirm 且门卡活跃）隐藏输入条中止钮；ChatInputBar 渲染条件从 stage∈ACTIVE 改为「stage∈ACTIVE 且非门开态」；错误面指路文案按错误类（lease→接管/门态→门卡）。死代码（useStageUI actions=[abort]、rollback sender）全仓 grep 动态消费者后删除。

### D3 degraded 关键帧投递

关键帧白名单（stage_change/session_state/HumanGateOpened 族）；投递路径对 degraded 连接：try_send 失败→改为等待重试（有界）或立即发 resync_required 帧让客户端主动 REST 拉取（客户端已有全量拉取能力）。degraded 转移打点：attachment 层 degraded 进入/退出 hook → append-only jsonl（连接 id/时刻/原因），零业务影响。V0 的打点复现方案作为验收参照。

## Risks

- 等待式投递的背压：有界等待（如 2 次退避）后必发 resync_required，不无限占任务
- 矩阵收编对既有 Abort 消费者的破坏面：仓内 7+ campaign 脚本 open 即 hello、Abort 仅 run 态发（F53Diag 全景表）——理论零破坏，实施时以全景表回归

## Open Questions

- in-flight 修订期 Abort 窄 escape（用户在修订中想取消本轮修订）——暂不做（watchdog 兜底），后续按需 delta 增补
