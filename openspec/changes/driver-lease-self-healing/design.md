# Design: driver-lease-self-healing

> 诊断依据：`.superpowers/sdd/2026-09-24_计划文档_change实施_plan-compile-gate-visibility_v1.0/`（笔误目录见 f50-lease-diagnosis.md，实际在 ears-delivery 目录）`f50-lease-diagnosis.md`；oracle 裁决 `f50-lease-oracle.md`（进行中，B 方向结论回填本节）。

## Context

lease 现状：attachment.rs:38-39 accept 即 provisional acquire；arbitration.rs:78 hello(driver) 显式持有；legacy 语义=纯 attach 即抢走（part_06b.rs:83 钉死）；observer hello 回滚恢复 attach 快照可产出僵尸持有者；holder=None 写拒；恢复仅手动重新接管。

## Decisions

### D1 自愈收窄在「悬空态」不放争抢

REQ-DLS-01 只覆盖 holder=None：这是「无人持有、写者自证是 Driver」的安全窗口；holder=Some(活跃) 仍拒（不引入连接争抢/双写风险）。幂等由仲裁层原子授予保证。

### D2 前端重试恰一次

hello 重发+原命令重放一次；二次失败回退既有手动面。防循环：STALE 后至多重放一轮，且仅对写命令（confirm/feedback/advance 等 facade 出站）。

### D3 打点为 append-only 诊断流

复用仓内 append-only JSONL 先例（usage-diagnostics.jsonl 同构）：`<session>/lease-diagnostics.jsonl`（schema_version=1）。事件枚举：acquire_provisional/hold/steal_by_attach/rollback/orphan/release。只读端点挂在既有 workspace 诊断 HTTP 面。不进仲裁状态机。

### D4 B 方向（oracle 已裁：全并入，形态二）

裁决依据 f50-lease-oracle.md：契约四层（设计裁定 §3.4 / REQ-WCR-02 / P2 计划 D5）全部写显式接管，attach 即抢是 attachment.rs:36-39 注释自认的实现妥协；前端三处 hello 全带 role，多 tab 驾驶切换不受损；服务端触点与 A/C 同文件族（lease.rs/attachment.rs/arbitration.rs），前端零改动。形态二=attach 对租约零效应 + provisional 快照/回滚整体删除（B②③ 结构性吸收）。A 成为无 hello 首写的承重墙（R2），必须与 B 同批落地；自愈放款须同步刷新 attachment.lease_epoch。R3：bind_role same_channel 反查不动，删回滚分支后复核 pending→live 迁移。三个语义反转用例逐名改写不删除（part_06b.rs:83 / part_03.rs:579 / part_03.rs:807）。

## Risks / Trade-offs

- 自愈放行窗口若与「旧持有者实际活着但网络分区」并存：心跳/活跃判定依赖既有连接活性语义，分区场景下两侧都认为自己持有——D3 打点可定案，B③ 是治本
- 前端自动重放对非幂等命令的风险：写命令本身有既有幂等键（command_id 系），重放安全
- 打点量：每次轮询 attach 都记——高频但一行式，滚动上限（如 1MiB 截断）实施时定

## Open Questions（实施首步）

- 僵尸 observer 持有者（holder=Some(observer_socket)）的发生率：D3 打点上线后以真实数据定，作为 B③ 的裁决输入
