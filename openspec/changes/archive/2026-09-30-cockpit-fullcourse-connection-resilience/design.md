# Design: cockpit-fullcourse-connection-resilience

## Context

**主契约（权威输入，不得漂移）**：`cadence/designs/2026-09-16_设计文档_3.8驾驶舱全程化与连接韧性_v1.0.md`（3.8 设计 v1.0）。本文是它的 OpenSpec 架构摘要与决策登记（D 系列），**不重新论证、不引入未拍板项**；两文冲突时以设计文档为准。

**决议权威**：`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/p38-tripartite-decision.md`（Q1-Q5/范围修正 4/纪律 4 逐条承接，对照表=设计文档附录 B）。

**现状技术事实基线**（设计文档 §1，RCA/ter/oracle 实读；实施以代码为准）：

| # | 事实 | 证据 |
|---|---|---|
| 1 | cockpit 已挂 `useWorkspaceWs`、渲染 `TimelineNodeList`+`ChatEntryList`；已有 `sendStartGeneration`/`selectProvider`/流式消息处理——P1 缺接线不缺能力 | ter §1.2 |
| 2 | Hello 仅 `session_id`+`last_seen_node_id`（`in_.rs:26-29`），cursor 未被消费（inbound 仅异步下发完整 `session_state`） | RCA:§1.2；ter §1.2 |
| 3 | `WsInMessage` 无 `deny_unknown_fields` → Hello 新增 `Option` 字段双向兼容；driver 脚本族 6 处无 role hello（不经部署通道，直连在役服务端） | oracle 实读 |
| 4 | engine/run/event 全随 socket 创建（每连接 `WorkspaceEngine::new_persistent`、socket-local `current_run`/`next_run_id`/单连接 `outbound_tx`）；连接关闭 abort forward/send task | ter §1.2 |
| 5 | socket 清理持 `current_run` 时写 `append_aborted_by_disconnect`+`transition_to_prepare_context_after_disconnect`（活动节点标 Failed，`lifecycle.rs:845-879`）——审计整流结果，非 provider cancel 证明（`abort_workspace_run` 已移除） | RCA:§2.2 |
| 6 | 任一同 endpoint 连接可发业务命令；第二连接触发 run 走 supersede `abort_active_run` 路径（`provider_run.rs:64-81`）——「driver/observer」只是前端约定 | RCA:§1.2 |
| 7 | 快照门三形态：0017=投影无终态守卫；0429=phase 停 Evaluate 未回门节点（confirm 撞 STAGE_INVALID）；0437=门开 104ms 被误标覆盖 | 设计文档 §1.3 |
| 8 | 服务端 idle 90-95s（`last_seen.elapsed()>timeout && !is_active_run()`）；前端 stale-close 60s 已热修（af4f7ccf，RCA §4 保留口径：直接因果未闭环） | RCA:§2.1/§4 |
| 9 | 类型感知默认现状（d0831dd5+d99fbb6e）：显式设置优先；未设置 plan+timeline>0→cockpit、===0 与 story/design→legacy；跨会话归属守卫已在 | 台账:L493/L495 |

**本文只做规划产物**：精确文件、命令、测试与提交步骤由 `superpowers:writing-plans` 写入 `cadence/plans/`。

## Goals / Non-Goals

**Goals:**

- 驾驶舱覆盖全程：cockpit 内完成发起（含 Provider 选择）→流式→门→落地，不切 legacy（REQ-CFC-01..04）。
- 连接与运行解耦：断连不污染终态、被动连接不盲区、恢复链可达（REQ-WCR-01..04）。
- 归因一次复现闭环 + 投影不信失配快照（REQ-WCR-05、REQ-CFC-05）。
- 0429 恢复链根治并承接 3.7 挂账 C 正向门关闭（REQ-CG-04/05 MODIFIED）。
- 存量数据零迁移 + 回退安全（REQ-WCR-06）；RCA §6 矩阵全量为 P2 done 定义（REQ-WCR-07）。

**Non-Goals（设计级边界，与 proposal 非目标互补）:**

- 不改 3.7 已验收交互面语义（快捷键/批量单门/审计三层/四码归因/ConfirmTwiceButton 归一/返回父级双链/autopilot 锚点/四层提醒/双轨开关——只做加法）。
- 不删 legacy、不执行旧协议退役、不做 role 必填收紧（阶段 4 退役门评估）、不做多仓 coding/`auto_start_coding` 显式语义（DEF-4/5/6）。
- 不改编码侧插话 fail-closed 语义（P2 lease 是连接授权不是门禁/插话语义）；不做跑动中接管。
- M1-5 arch-code 入口自检=显式 defer（文档指引 127.0.0.1:4317 直连）。
- `complete_human_gate_revision` approval 三元组缺陷留 4 号修法包 backlog（M5-2 边界，见 D13）。
- coding WS 广播不默认入 P2（条件扩展，见 D8）。

## Decisions

### D1: 形态路由终态化 = 类型感知机制保留 + 默认表演化（Q1a 落地）

**决策**：类型感知路由**机制**保留为终态（显式设置优先/跨会话归属守卫/未知类型安全 legacy/fix r1/r2 语义全保留）；P1 落地后默认表演化——未显式设置的已知类型（story/design/plan 含未开始态）默认 cockpit；「空 plan 必须 legacy」前提删除（其安全性守卫不删）；首帧定形态（消除「先 legacy 建连收状态再切 cockpit」的 unmount+close(1000)+重连）。默认表演化是对 09-16 裁决行为面的变更，**呈报用户明示（R-1）**；C4 显式 legacy/cockpit 与回滚语义不动；legacy 页面不删（退役=阶段 4）。

**替代方案与否决理由**：①Q1(b) 翻纯 cockpit+排期 legacy 退役——与输入包 §4-2「P1 只解锁退役门评估」自相矛盾，3.8 越权消费阶段 4 决策（oracle 裁决理由 1）；且翻死默认后回滚只剩开关一层、粒度变粗（理由 2）。②保留现状默认不演化——cockpit 已具备全程能力后仍把 story/design 挡在 legacy，换脸割裂（M1-2）不闭环，P1 验收口径「全程不切 legacy」自相矛盾。

**取舍**：默认表演化验收期可能被指「暗改已验收行为」——以 R-1 明示点呈报消化，不以隐瞒消化。

### D2: wire 兼容 = 双字段容忍 + 服务端缺席归一（Q2a 落地）

**决策**：Hello 增可选 `role`（driver/observer）与 `after_event_seq` 两字段，同批同纪律：旧客户端→新服务端（缺席=None ✓）、新客户端→旧服务端（serde 忽略未知字段 ✓），双向兼容零 flag-day；driver 脚本族 6 处 hello 零强制升级（缺席语义=legacy driver 等价，与全部现存客户端真实用途一致）。**服务端入口处把缺席 role 归一为内部显式常量**，命令仲裁只认内部角色——legacy 分支不外溢到仲裁逻辑。阶段 4 退役门评估是否收紧为必填（预注册硬化路径，不预执行）。

**替代方案与否决理由**：①Q2(c) 一刀切——脚本族不经部署通道（worktree 直连在役 v16h），(c)=4+ 脚本 flag-day+部署顺序耦合，而 RCA §6 矩阵依赖这些 driver 真跑；且「同部署」只覆盖 rust-embed 内嵌前端。②Q2(b) 握手协商——单仓同部署、无长尾第三方客户端生态，能力协商解决不存在的问题，且放大权限/lease/重连组合矩阵。③ter 让步框架采纳：wire 层不让（保 (a)），实现层让（内部归一显式常量，仲裁单分支）。

### D3: P3 拆分与执行时点（Q3 落地）

**决策**：P3a（连接级归因日志+投影终态守卫）= **OpenSpec 批准后首批**（含服务端改动，按 C1 破除纪律必须批后；af4f7ccf 纯前端热修例外已发生不追溯）；**socket 误标修复随 P2 架构吸收**，不独立先行——0429 证明误标发生在两种相位（run 已业务终态被覆盖/run 存活门刚开被覆盖），正确最小守卫需要 run ownership 知识，半版热修（只删标记不动 stage 迁移）引入第三态（断连后 run 悬挂无呈现亦无恢复）；让步开口=满足「不引入第三态」判据（仅跳过覆盖动作、不动 stage 迁移/恢复语义）的最小守卫可并入先行包（默认不并，并入须实施计划先证明判据）。**P3b phase 回门=P2 族内最前**（0429 唯一根治、解锁 Q5 移交 C 挂账、用户可感知收益最大；引擎恢复链变更无纯热修资格）。

### D4: P2 架构 = WorkspaceSessionManager（session-owned run，共同前置）

**决策**：每会话一个 manager，唯一持有 `ActiveRun { token, cancellation, command_tx, state, lease_epoch }` 与单一 `WorkspaceEngine` 实例；连接=attachment（connection_id+角色+订阅），绝不拥有 run；开始/abort/命令/完成/lease 释放进入 manager 单一原子临界区（token+lease epoch 比较生效）；supersede 显式语义保留并扩展（observer/stale driver 不得触发）；engine 创建一次、运行期不随连接销毁；事件源=manager-owned journal/fan-out（不指向 socket mpsc）；内存回收点=终态+无订阅者；进程重启从 durable 重建，**in-flight provider run 不虚称跨进程连续**（可恢复 state 与不可恢复运行分开，按恢复链处置）。

**排序纪律（否决四条独立并行）**：manager 是角色、close 语义、cursor/广播的共同前置——族内串行链 manager 骨架→角色+close→cursor/广播；P1 与 P2 引擎主体真并行，P2 前端尾巴（WS 客户端层）排在 P1 后（点名交集=WS 客户端层）。**P2 子项不单独交付半成品**（只做一项=制造更危险的双所有权）。

**替代方案与否决理由**：①只给 `WorkspaceRunRegistry` 加字段——engine/outbound/run id 仍随 socket 创建，重连产生第二 engine 视图，所有权仍混在两层（ter §4.1/4.2）；②四条各自独立实施再合并——merge/race 风险放大（决议 §三.4 否决）。

### D5: lease 粒度 = 会话级单活性 driver

**决策**：lease 为会话级、至多一个活性 driver 授权：由连接持有，连接关闭只撤销 lease/记录状态（不取消不改写 run）；新连接可显式接管（epoch 递增，旧 epoch 迟到写被拒）；observer 无 lease。必测边界：lease 过期、并发 driver 抢占、刷新后原 tab 迟到消息被拒、driver 重连获取可写 lease。

**替代方案与否决理由**：连接级多 driver lease——生产现实是单会话单 driver（用户裁决④同族），多写者仲裁与组合矩阵爆炸解决不存在的需求；supersede（run 级换代）与 lease（连接级授权）正交，无需多 lease 表达。

### D6: 连接关闭终态语义（close 不写 `aborted_by_disconnect`）

**决策**：连接关闭（任一形态）不写 `aborted_by_disconnect`、不把活动节点标 Failed、不触发 `transition_to_prepare_context_after_disconnect` 整流；只有 provider token 实际被取消/provider 失败/明确人类 abort 才写对应真实终态；显式 abort 与新 run supersede 的取消路径照旧；run 完成与 close 并发只产生一个真实 terminal reason。

**替代方案与否决理由**：见 D3（半版热修引入第三态）；「保留标记但改文案」——审计事实仍会把断连编码为运行终态，0429/0437 覆盖不消除。

### D7: cursor = session-scoped 单调 event_seq + 有界 journal + snapshot 基线

**决策**：不复用 node id 作 cursor（node id 不能表达同 node 内 stream chunk/stage change/permission/choice 顺序）；Hello 提交可选 `after_event_seq`；manager 保存有界事件 journal；cursor 过旧/缺口→先发带 cursor 的完整 snapshot 建基线再续事件；客户端按 seq 去重。stream chunk 策略=journal 至 run 完成 **或** snapshot 携带当前累计文本（实施二选一，验收同口径：断线重连流式呈现不丢不重，不得承诺可回放却只回放 timeline）。慢订阅者=有界 channel，溢出标记需 snapshot 降级，不反压 provider。每连接只管自己出站泵，关任一连接不影响 run/engine/其他 attachment。

### D8: 广播边界 = workspace 先行，coding 条件扩展（决议 §三.2）

**决策**：P2 范围=workspace WS 解耦+广播；coding WS 广播（M2-5/M5-1 现场）=条件扩展——P2 主体滑窗则 defer 至阶段 4 前夜并显式登记 backlog（呈报可见，不以「顺带」模糊）；P2 主体落地且余量足够则单独估时实施（按 ter §3.2 口径对 coding 面单独重估，不蚕食 P2 主体）。coding 侧 UI 启动入口已缓解盲区用户面（3.7-P4 A 链）。

**替代方案与否决理由**：「顺带根治 M2-5」——M2-5 现场在 coding WS（coding_ws_handler），RCA §5 根治设计覆盖 workspace 面；混写会让 2-3 天错误估算复活且范围不可审（oracle 范围审）。

### D9: 归因日志中性化（ter 关键修正采纳）

**决策**：归因记录=**中性连接诊断记录**，与业务 timeline terminal reason 分离——不把断连事实重新编码为「运行已中止」。服务端：每 WS 生成 connection_id，记录 receiver 退出类型（close code/reason、EOF、error）、是否 idle 触发、session、run token、drive depth、最后双向活性时间。前端：onclose code/reason/wasClean/visibilityState、最后 pong/ping 时间（进诊断面）。**过渡期**（P3a→P2 落地前）断连清理仍写 `aborted_by_disconnect`（修复随 P2），此时同 connection_id 写入其 detail 供关联——一次复现闭环归因；P2 落地后该写入随架构消失，中性记录成为唯一连接级事实源。闭环判据=一次复现唯一判定 close 起点 ∈ {server-idle, 前端 4000, 卸载 1000, 代理/TCP, 浏览器 discard}。

**替代方案与否决理由**：按 RCA §5-1 原文把 connection_id 长期写入 `aborted_by_disconnect` detail——与 P2 目标「close 不写该标记」终态自相矛盾（ter Q3 关键修正）；过渡期衔接保留其归因意图。

### D10: P1 复用纪律（禁止第二套实现）

**决策**：生成入口复用 `ChatInputBar`+`handleStartGeneration` 语义（经 `useWorkspaceWs.sendStartGeneration`），维持 legacy 全部发起语义（stage 守卫/连接状态/recoverable run/optimistic entry）；Provider 配置复用 `ProviderConfigPanel` 与既有 store（单一状态源）；流式进度=复用三区投影+补运行态徽标与边界，不新建独立生成面板；**不另开生成 socket、不建第二份 provider 状态、不新建第二套流式状态机**；只消费既有 WS 事件面（不足则该子项移入 P2 契约重估）。

**替代方案与否决理由**：新建 cockpit 专用生成组件族/状态机——双源漂移+重建路径写两遍+违反增量不换血（3.7 设计系统纪律延续）。

### D11: P4 裁剪（Q4 落地）

**决策**：driver readback（`stage3_group_snapshot_readback` elapsedMs bug）必做且前置（2-4h，零依赖，排首批窗口或 P2 族期间，不绑定 P2 合并窗口）——挡 RCA §6 矩阵 driver 种子效率。跨会话批量=条件项：依赖 P2 两条具体机制（①命令仲裁安全：无角色时批量连接与用户 tab 同为无差别可命令连接，supersede 竞态无防护；②被动观察一致性：批量结果对其他 tab 可见性依赖 session 广播）；P2 滑窗则整体 defer 阶段 4 前夜。杂项清点表逐项「已修/已 defer+理由」；**timer 节流结项口径=「P2 重连重订阅架构使其无害化」**（节流只在 >90s 全向静默才可能成为真根因，P2 后瞬断不污染终态）——如实表述为架构吸收，非「明知风险不修」。

### D12: Q5 三分解承接

**决策**：引擎不阻塞项（B 对话侧带门完整四键/E plan-repair 返回父级等 UI-only 走查）由 controller 立即补验回填 3.7 关闸报告（非本 change 实施范围）；引擎阻塞项 C 正向门关闭（0429 现场）由本 change P3b 验收矩阵**点名承接**（REQ-CG-05 scenario「修订中止后重连 confirm 真实关闭」），无账外残留；3.7 归档+squash 照常、残留如实标注（先例=3.6 矩阵 8/9+2 限制归档）。

### D13: M5-2 四号修法包吸收边界（设计文档前必钉项）

**决策**：吸收①REQ-CG-04 人工权威升级语义显式化补句（随 MODIFIED requirement 落地，契约化 4f34ba58 既有实现零行为变更）与③phase/active_node 回门节点（approval compile 失败+修订中止两路径全量，共享同一回滚机制）；**不吸收**②`complete_human_gate_revision` 不清 approval 三元组→吸收态 Failed 缺陷（缺陷函数实在 `src/product/lifecycle_store/workspace_single_candidate.rs:323`；落吸收态 Failed 的写入点 = `compile/single_candidate.rs:662-673` 的 `record_single_candidate_compile_failure`）——与 0429 恢复阻断不同链（compile 拒后人为二次决策链的状态污染），需独立复现矩阵与吸收态语义设计，显式留 4 号修法包 backlog，不并账不吞账。

### D14: 存量数据处置与降级回滚（C1 破除代价声明补两条）

**决策**：①卡死 session 攒账（0166/0167/0168/0199/0277+0429）处置=修复落地后**复活验证**（快照门 confirm 可达则复活续链；获得明确可诊断拒绝则以可诊断终态化关闭——真实契约缺口的 compile 正确拒=产品正确行为），逐个记入 defer-ledger，不回写不修复历史 durable（事件前缀不可变），不承诺全部复活。②降级回滚=**回退安全**：durable schema 不变（只改写入行为），旧二进制读新数据无断裂；wire 双字段容忍保证新旧任意组合不炸；存量历史标记保留为历史。回退代价=恢复旧行为面（重新引入误标），无数据迁移。P2 关闸证据含一次回退读验证（旧二进制对新数据读会话状态无错）。

### D15: 验收策略（双层 + 矩阵不可裁）

**决策**：沿用 REQ-UI37-12 双层范式——人工验收只认真实链（P1：cockpit 内发起→流式→门→落地全程不切 legacy 不开命令行），自动化替身只做单测/回归。**RCA §6 四条断连矩阵全量引用为 P2 done 定义，不可因预估压力裁剪**（R-4；滑窗=整体 defer，不是裁矩阵）。P3 三形态复现矩阵（0017 终态门不可操作/0429 confirm 真实关闭/0437 业务终态不被覆盖）+归因四 close 唯一归因。口径纪律（R-2）：归因日志落地并经一次复现闭环前，任何文案不得宣称「断连主嫌已根治」——热修 af4f7ccf 表述=「消除已证实的前端断连源」。

## Risks / Trade-offs

| # | 风险 / 取舍 | 缓解 |
|---|---|---|
| R1 | **口径纪律（R-2）**：归因闭环前误宣称根治 | 文案纪律进每包验收（口径检查项）；闭环判据 D9 |
| R2 | **先行包时点（R-3）**：P3a 滑向批前 | 本 change 批准后才开工 P3a；af4f7ccf 例外不追溯不复制 |
| R3 | **P2 结构风险**：token 竞态/engine 双实例/仲裁缺口 | D4 单一原子临界区+token/epoch 判等+engine 单实例+回收点；ter §4 边界逐项测试 |
| R4 | **cursor/广播被低估** | D7 event_seq+journal+snapshot 基线+慢订阅者降级；验收同口径 |
| R5 | **P2 半成品双所有权** | D4 不交付半成品纪律；REQ-WCR-07 四条矩阵=done 定义 |
| R6 | **wire 破坏面** | D2 双字段容忍（serde 实读+缺席=driver 等价）；新字段全走可选+归一 |
| R7 | **P1 工期**（1 天口径被否决，真实 17-28h） | 薄纵切 10-15h 先行交付；全量独立验收；测试迁移不删用例 |
| R8 | **默认表演化争议（R-1）** | 明示点呈报；C4 回滚通道保留+显式 legacy 回滚演练进验收 |
| R9 | **形态切换连接重建** | D1 首帧定形态+守卫清单五条+专项回归（无中途 unmount+close(1000)） |
| R10 | **P2 滑窗（84-130h）** | 条件项纪律（D8/D11 defer+呈报）；矩阵仍不可裁（滑窗=整体 defer） |
| R11 | **3.7 语义回归** | 红线清单进每包验收；P1 验收含 3.7 交互面回归自查 |
| R12 | **Q5 挂账误判闭环**（0429 修复前 C 项不可真过） | D12 三分解：C 项点名移交 P3b 验收，无假闭环 |

## Migration Plan

1. **交付序**：首批=P3a（归因日志+终态守卫）+driver readback 前置 ∥ P1 薄纵切（明早用户可验证）→ P1 全量 → P2 族（P3b 最前→manager→角色+close→cursor/广播）→ P4（条件项+清点表）。每包独立验收（tasks.md）。
2. **无部署迁移、无数据迁移**：durable schema 不变；存量标记保留为历史；卡死 session 走复活验证（D14）。
3. **回滚策略**：P1=翻 `aria.chat.cockpit` 开关回 legacy（C4 通道原样）；P2=回退旧二进制安全（D14-②，关闸含回退读验证）；P3a 归因日志为纯增量诊断（回退无影响）。
4. **wire 迁移**：无 flag-day——可选字段+缺席归一；driver 脚本族零强制升级；前端 observer 部署随 P2 前端尾巴同次声明 `role=observer`。
5. **关闸证据**：每包两清单（自动化+人工/复现矩阵）；P2 关闸=RCA §6 四条矩阵真跑+回退读验证+存量处置记录。

## Open Questions

均为设计文档已明示、实施时各自细化的留白（不改架构、specs 或工作包划分）：

1. **P1 生成入口/配置面的具体布局**（挂点在 ③ 顶部与 ① 空态引导的视觉细节）——设计文档 §2.1 定原则（三区架构不动），布局属实施细化。
2. **stream chunk 回放策略二选一**（journal 至 run 完成 vs snapshot 带累计文本）——D7 定验收同口径，选择留 writing-plans。
3. **connection_id 诊断记录的落盘形态**（stderr 结构化行 vs 轻量文件）——D9 定字段集，载体属实施细化（须满足一次复现可查）。
4. **默认 Provider 的记忆键名**——localStorage 级，实施细化。
5. **coding WS 广播若做，其事件序号与 workspace event_seq 是否共享机制**——条件扩展项触发时单独设计（D8），不在本 change 预设。
